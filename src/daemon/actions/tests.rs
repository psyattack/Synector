use super::*;

use super::audio::{device_name_matches, endpoint_registry_guid};
use super::macro_action::{macro_cancellations, resolve_macro_plan, run_macro};
use crate::win_args::{join_windows_arguments, split_windows_arguments};
use crate::command::{read_capped, run_command_bounded};
use super::webhook::{build_webhook_request, webhook_client};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(windows)]
use super::input::{mouse_control, send_key_event, ACTIVE_HELD_KEYS, ACTIVE_HELD_MOUSE_BUTTONS};

// Compile-time guard against a future change that would let `cargo test` type into the
// focused window or move the cursor while the developer is working.
const _: () = assert!(!INPUT_INJECTION_ENABLED, "unit tests must not inject input into the desktop");

/// The macro cancellation registry is process-global, so tests that inspect it must not
/// overlap with a test that transiently registers a running macro.
static MACRO_REGISTRY_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn test_active_held_inputs_and_release() {
    // Physical injection is compiled out under `cfg(test)` (see INPUT_INJECTION_ENABLED),
    // so these calls only exercise the anti-stuck tracking registry and its drain path.
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_W;
        // Simulate key down tracking
        send_key_event(VK_W, false);
        assert!(ACTIVE_HELD_KEYS.lock().unwrap().contains(&VK_W.0));

        // Track mouse button
        if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
            lock.insert(crate::models::MouseButton::Left);
        }
        assert!(ACTIVE_HELD_MOUSE_BUTTONS.lock().unwrap().contains(&crate::models::MouseButton::Left));

        // Release all simulated inputs immediately
        release_all_simulated_inputs();

        // Verify both sets are completely cleared
        assert!(ACTIVE_HELD_KEYS.lock().unwrap().is_empty());
        assert!(ACTIVE_HELD_MOUSE_BUTTONS.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_relative_mouse_control_no_panic() {
    #[cfg(windows)]
    {
        // Cursor movement is suppressed under `cfg(test)`, so this exercises the
        // relative coordinate path and its error handling without touching the desktop.
        let res = mouse_control(
            crate::models::MouseActionType::Move,
            crate::models::MouseButton::Left,
            crate::models::MouseCoordMode::Relative,
            Some(50),
            Some(-30),
            false,
            Some(0),
            false,
            None,
            None,
            None,
            false,
        )
        .await;
        assert!(res.is_ok());
        // A move never leaves a button in the held-input registry.
        assert!(ACTIVE_HELD_MOUSE_BUTTONS.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_if_else_branches_execution() {
    let temp_dir = std::env::temp_dir();
    let existing_path = temp_dir.join("ph_action_test_exists.tmp");
    std::fs::write(&existing_path, b"1").unwrap();
    let non_existing_path = temp_dir.join("ph_action_test_never_exists.tmp");

    let action = crate::models::Action::IfElse {
        conditions: vec![crate::models::RuleConditionItem::new(crate::models::RuleCondition::FileWatcher {
            path: non_existing_path.to_string_lossy().to_string(),
            filter_extension: None,
            must_exist: true,
        })],
        then_actions: vec![crate::models::Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 0.001,
            condition: None,
            timeout_sec: 1.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }],
        else_if_branches: vec![crate::models::ElseIfBranch {
            conditions: vec![crate::models::RuleConditionItem::new(crate::models::RuleCondition::FileWatcher {
                path: existing_path.to_string_lossy().to_string(),
                filter_extension: None,
                must_exist: true,
            })],
            actions: vec![crate::models::Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.001,
                condition: None,
                timeout_sec: 1.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            }],
        }],
        else_actions: vec![],
    };

    let context = std::collections::HashMap::new();
    let res = ActionRunner::execute(&action, &context).await;
    assert!(matches!(res, Ok(ActionOutcome::Continue(_))));

    let _ = std::fs::remove_file(&existing_path);
}

#[tokio::test]
async fn test_check_action_met_and_unmet() {
    let temp_dir = std::env::temp_dir();
    let existing_path = temp_dir.join("synector_check_test.tmp");
    std::fs::write(&existing_path, b"data").unwrap();

    let check_pass = Action::Check {
        condition: crate::models::RuleCondition::FileWatcher {
            path: existing_path.to_string_lossy().to_string(),
            filter_extension: None,
            must_exist: true,
        },
    };
    let ctx = HashMap::new();
    let res_pass = ActionRunner::execute(&check_pass, &ctx).await;
    assert_eq!(res_pass, Ok(ActionOutcome::Continue(None)));

    let check_fail = Action::Check {
        condition: crate::models::RuleCondition::FileWatcher {
            path: temp_dir.join("non_existent_file.tmp").to_string_lossy().to_string(),
            filter_extension: None,
            must_exist: true,
        },
    };
    let res_fail = ActionRunner::execute(&check_fail, &ctx).await;
    assert!(matches!(res_fail, Ok(ActionOutcome::AbortPipeline { .. })));

    let _ = std::fs::remove_file(&existing_path);
}

#[tokio::test]
async fn test_wait_action_timer_and_timeout() {
    let wait_timer = Action::Wait {
        mode: crate::models::WaitMode::Timer,
        seconds: 0.005,
        condition: None,
        timeout_sec: 1.0,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    let ctx = HashMap::new();
    let res = ActionRunner::execute(&wait_timer, &ctx).await;
    assert_eq!(res, Ok(ActionOutcome::Continue(None)));

    // Test timeout abort
    let wait_timeout_abort = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::File {
            path: "C:\\non_existent_path_never_created.tmp".into(),
            condition: crate::models::WaitFileCondition::Exists,
        }),
        timeout_sec: 0.05,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    let res_abort = ActionRunner::execute(&wait_timeout_abort, &ctx).await;
    assert!(matches!(res_abort, Ok(ActionOutcome::AbortPipeline { .. })));

    // Test timeout continue
    let wait_timeout_continue = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::File {
            path: "C:\\non_existent_path_never_created.tmp".into(),
            condition: crate::models::WaitFileCondition::Exists,
        }),
        timeout_sec: 0.05,
        timeout_policy: crate::models::WaitTimeoutPolicy::Continue,
    };
    let res_cont = ActionRunner::execute(&wait_timeout_continue, &ctx).await;
    assert_eq!(res_cont, Ok(ActionOutcome::Continue(None)));
}

#[tokio::test]
async fn test_wait_and_check_rule_activity() {
    use crate::daemon::rule_activity;

    let target = "rule_activity_test_wait_target";
    let ctx = HashMap::new();

    // "Rule stopped" is satisfied while the target has no running pipeline.
    let wait_stopped = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::RuleStopped {
            rule_id: target.into(),
        }),
        timeout_sec: 1.0,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    assert_eq!(
        ActionRunner::execute(&wait_stopped, &ctx).await,
        Ok(ActionOutcome::Continue(None))
    );

    // "Rule started" does not pass while the target is idle and honours the timeout policy.
    let wait_started_idle = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::RuleStarted {
            rule_id: target.into(),
        }),
        timeout_sec: 0.05,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    assert!(matches!(
        ActionRunner::execute(&wait_started_idle, &ctx).await,
        Ok(ActionOutcome::AbortPipeline { .. })
    ));

    // A running target satisfies both the wait and the inline check.
    rule_activity::mark_started(target);
    let wait_started_running = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::RuleStarted {
            rule_id: target.into(),
        }),
        timeout_sec: 1.0,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    assert_eq!(
        ActionRunner::execute(&wait_started_running, &ctx).await,
        Ok(ActionOutcome::Continue(None))
    );

    let check_running = Action::Check {
        condition: crate::models::RuleCondition::RuleRunning {
            rule_id: target.into(),
            must_be_running: true,
        },
    };
    assert_eq!(
        ActionRunner::execute(&check_running, &ctx).await,
        Ok(ActionOutcome::Continue(None))
    );

    let check_stopped = Action::Check {
        condition: crate::models::RuleCondition::RuleRunning {
            rule_id: target.into(),
            must_be_running: false,
        },
    };
    assert!(matches!(
        ActionRunner::execute(&check_stopped, &ctx).await,
        Ok(ActionOutcome::AbortPipeline { .. })
    ));

    // A start that happens after the wait began is still observed even when the pipeline has
    // already finished by the next poll.
    rule_activity::mark_finished(target);
    let starter = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        rule_activity::mark_started(target);
        rule_activity::mark_finished(target);
    });
    let wait_started_after = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::RuleStarted {
            rule_id: target.into(),
        }),
        timeout_sec: 2.0,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    assert_eq!(
        ActionRunner::execute(&wait_started_after, &ctx).await,
        Ok(ActionOutcome::Continue(None))
    );
    starter.await.unwrap();

    // An empty target is invalid: it never satisfies either wait and runs into the timeout.
    let wait_empty = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::RuleStopped {
            rule_id: String::new(),
        }),
        timeout_sec: 0.05,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    assert!(matches!(
        ActionRunner::execute(&wait_empty, &ctx).await,
        Ok(ActionOutcome::AbortPipeline { .. })
    ));
}

#[tokio::test]
async fn test_archive_pack_unpack_zip() {
    let temp_dir = std::env::temp_dir().join(format!("ph_test_arch_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let src_dir = temp_dir.join("src");
    let extract_dir = temp_dir.join("extracted");
    let zip_file = temp_dir.join("bundle.zip");

    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("hello.txt"), b"Hello Synector Archive").unwrap();

    let pack_action = crate::models::Action::Archive {
        operation: crate::models::ArchiveOperation::Pack,
        format: crate::models::ArchiveFormat::Zip,
        source_path: src_dir.to_string_lossy().to_string(),
        destination_path: zip_file.to_string_lossy().to_string(),
        compression_level: None,
        overwrite: true,
    };
    let ctx = std::collections::HashMap::new();
    let pack_res = ActionRunner::execute(&pack_action, &ctx).await;
    assert!(pack_res.is_ok(), "Pack failed: {:?}", pack_res);
    assert!(zip_file.exists());

    let unpack_action = crate::models::Action::Archive {
        operation: crate::models::ArchiveOperation::Unpack,
        format: crate::models::ArchiveFormat::Zip,
        source_path: zip_file.to_string_lossy().to_string(),
        destination_path: extract_dir.to_string_lossy().to_string(),
        compression_level: None,
        overwrite: true,
    };
    let unpack_res = ActionRunner::execute(&unpack_action, &ctx).await;
    assert!(unpack_res.is_ok(), "Unpack failed: {:?}", unpack_res);
    assert!(extract_dir.join("hello.txt").exists());
    let data = std::fs::read_to_string(extract_dir.join("hello.txt")).unwrap();
    assert_eq!(data, "Hello Synector Archive");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_file_system_crud_actions() {
    let temp_dir = std::env::temp_dir().join(format!("ph_test_fs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let ctx = std::collections::HashMap::new();

    let dir_path = temp_dir.join("folder");
    let act_mkdir = crate::models::Action::FileSystem {
        operation: crate::models::FileSystemOperation::CreateDirectory,
        path: dir_path.to_string_lossy().to_string(),
        target_path: None,
        content: None,
        create_parents: true,
        overwrite: true,
    };
    assert!(ActionRunner::execute(&act_mkdir, &ctx).await.is_ok());
    assert!(dir_path.exists());

    let file_path = dir_path.join("data.txt");
    let act_write = crate::models::Action::FileSystem {
        operation: crate::models::FileSystemOperation::WriteText,
        path: file_path.to_string_lossy().to_string(),
        target_path: None,
        content: Some("First Line\n".to_string()),
        create_parents: true,
        overwrite: true,
    };
    assert!(ActionRunner::execute(&act_write, &ctx).await.is_ok());

    let act_append = crate::models::Action::FileSystem {
        operation: crate::models::FileSystemOperation::AppendText,
        path: file_path.to_string_lossy().to_string(),
        target_path: None,
        content: Some("Second Line".to_string()),
        create_parents: true,
        overwrite: true,
    };
    assert!(ActionRunner::execute(&act_append, &ctx).await.is_ok());
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "First Line\nSecond Line");

    // Verify overwrite=false does not overwrite existing file
    let act_write_no_overwrite = crate::models::Action::FileSystem {
        operation: crate::models::FileSystemOperation::WriteText,
        path: file_path.to_string_lossy().to_string(),
        target_path: None,
        content: Some("Should not overwrite".to_string()),
        create_parents: true,
        overwrite: false,
    };
    assert!(ActionRunner::execute(&act_write_no_overwrite, &ctx).await.is_ok());
    let content_after = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content_after, "First Line\nSecond Line");

    let copy_target = dir_path.join("data_copy.txt");
    let act_copy = crate::models::Action::FileSystem {
        operation: crate::models::FileSystemOperation::Copy,
        path: file_path.to_string_lossy().to_string(),
        target_path: Some(copy_target.to_string_lossy().to_string()),
        content: None,
        create_parents: true,
        overwrite: true,
    };
    assert!(ActionRunner::execute(&act_copy, &ctx).await.is_ok());
    assert!(copy_target.exists());

    let act_del = crate::models::Action::FileSystem {
        operation: crate::models::FileSystemOperation::Delete,
        path: copy_target.to_string_lossy().to_string(),
        target_path: None,
        content: None,
        create_parents: false,
        overwrite: true,
    };
    assert!(ActionRunner::execute(&act_del, &ctx).await.is_ok());
    assert!(!copy_target.exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_trim_working_set_execution() {
    let current_pid = std::process::id();
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("ProcessId".to_string(), current_pid.to_string());

    // 1. Full empty working set
    let act_empty = crate::models::Action::TrimWorkingSet {
        target_process: None,
        soft_limit_mb: None,
        skip_if_focused: false,
    };
    let res_empty = ActionRunner::execute(&act_empty, &ctx).await;
    assert!(res_empty.is_ok(), "Empty working set should succeed on current process: {:?}", res_empty);

    // 2. Soft working set limit
    let act_soft = crate::models::Action::TrimWorkingSet {
        target_process: None,
        soft_limit_mb: Some(128),
        skip_if_focused: false,
    };
    let res_soft = ActionRunner::execute(&act_soft, &ctx).await;
    assert!(res_soft.is_ok(), "Soft limit working set should succeed on current process: {:?}", res_soft);

    // 3. Skip if focused flag
    let act_skip_focused = crate::models::Action::TrimWorkingSet {
        target_process: None,
        soft_limit_mb: Some(64),
        skip_if_focused: true,
    };
    let res_skip = ActionRunner::execute(&act_skip_focused, &ctx).await;
    assert!(res_skip.is_ok(), "Skip if focused should succeed: {:?}", res_skip);

    // 4. Non-existent process returns error
    let act_non_existent = crate::models::Action::TrimWorkingSet {
        target_process: Some("non_existent_process_12345xyz.exe".to_string()),
        soft_limit_mb: None,
        skip_if_focused: false,
    };
    let empty_ctx = std::collections::HashMap::new();
    let res_err = ActionRunner::execute(&act_non_existent, &empty_ctx).await;
    assert!(res_err.is_err(), "Non-existent process must return error");
}

#[test]
fn test_get_foreground_window_pid_no_panic() {
    let fg_pid = get_foreground_window_pid();
    if let Some(pid) = fg_pid {
        assert!(pid > 0);
    }
}

#[tokio::test]
async fn test_macro_cancellation_by_id_and_all() {
    let _guard = MACRO_REGISTRY_TEST_LOCK.lock().await;
    let cancel_token_1 = Arc::new(AtomicBool::new(false));
    let cancel_token_2 = Arc::new(AtomicBool::new(false));

    {
        let mut lock = macro_cancellations().lock().unwrap();
        lock.entry("test_macro_alpha".to_string())
            .or_default()
            .push(cancel_token_1.clone());
        lock.entry("test_macro_beta".to_string())
            .or_default()
            .push(cancel_token_2.clone());
    }

    assert!(!cancel_token_1.load(Ordering::SeqCst));
    assert!(!cancel_token_2.load(Ordering::SeqCst));
    assert!(is_macro_running("test_macro_alpha"));
    assert!(is_macro_running("test_macro_beta"));
    assert!(has_any_active_macro());

    stop_macro_by_id_or_name("test_macro_alpha");
    assert!(cancel_token_1.load(Ordering::SeqCst));
    assert!(!cancel_token_2.load(Ordering::SeqCst));
    assert!(!is_macro_running("test_macro_alpha"));
    assert!(is_macro_running("test_macro_beta"));

    stop_all_active_macros();
    assert!(cancel_token_2.load(Ordering::SeqCst));
    assert!(!is_macro_running("test_macro_beta"));
    assert!(!has_any_active_macro());
}

#[test]
fn test_webhook_request_keeps_injection_as_data_and_sends_headers() {
    let client = webhook_client().expect("webhook client");
    let payload = r#"{"note":"'; Remove-Item C:\\ -Recurse -Force; '"}"#;
    let headers = vec![(
        "Authorization".to_string(),
        "Bearer '; calc; '".to_string(),
    )];
    let request = build_webhook_request(
        client,
        "http://127.0.0.1:9/hook",
        "POST",
        &headers,
        Some(payload),
    )
    .expect("valid request");
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().as_str(), "http://127.0.0.1:9/hook");
    assert_eq!(
        request.headers().get("authorization").unwrap().to_str().unwrap(),
        "Bearer '; calc; '"
    );
    assert_eq!(
        request.headers().get("content-type").unwrap().to_str().unwrap(),
        "application/json"
    );
    assert_eq!(
        request.body().and_then(|body| body.as_bytes()),
        Some(payload.as_bytes())
    );

    // Quotes and spaces in a URL stay URL data (percent-encoded by the URL parser);
    // they are never pasted into a PowerShell command line any more.
    let quoted = build_webhook_request(
        client,
        "http://127.0.0.1:9/' ; calc ; '",
        "GET",
        &[],
        None,
    )
    .expect("URL text stays data");
    assert!(quoted.url().as_str().starts_with("http://127.0.0.1:9/"));
    assert!(quoted.url().as_str().contains("calc"));
    assert!(build_webhook_request(client, "not a url", "GET", &[], None).is_err());
    // Header names and methods are validated instead of being concatenated.
    assert!(build_webhook_request(
        client,
        "http://127.0.0.1:9/hook",
        "POST",
        &[("Bad Header".to_string(), "x".to_string())],
        None
    )
    .is_err());
    assert!(
        build_webhook_request(client, "http://127.0.0.1:9/hook", "P OST", &[], None).is_err()
    );
}

#[tokio::test]
async fn test_wait_rejects_invalid_durations_without_panic() {
    let ctx = HashMap::new();
    for seconds in [-1.0f64, f64::NAN, 1e300] {
        let wait = Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds,
            condition: None,
            timeout_sec: 1.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        };
        assert!(
            ActionRunner::execute(&wait, &ctx).await.is_err(),
            "seconds={seconds} must be rejected instead of panicking"
        );
    }

    let wait_timeout = Action::Wait {
        mode: crate::models::WaitMode::UntilCondition,
        seconds: 0.0,
        condition: Some(crate::models::WaitCondition::File {
            path: "C:\\synector_never_exists.tmp".into(),
            condition: crate::models::WaitFileCondition::Exists,
        }),
        timeout_sec: -1.0,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    };
    assert!(ActionRunner::execute(&wait_timeout, &ctx).await.is_err());
}

#[test]
fn test_split_windows_arguments_is_quote_aware() {
    assert_eq!(
        split_windows_arguments(r#"-m "C:\Program Files\app.exe" "a b" plain"#),
        vec!["-m", r"C:\Program Files\app.exe", "a b", "plain"]
    );
    assert_eq!(
        split_windows_arguments(r#""say \"hi\"""#),
        vec![r#"say "hi""#]
    );
    assert_eq!(split_windows_arguments(r#"a""b"#), vec!["ab"]);
    assert_eq!(
        split_windows_arguments(r#""C:\path with space\\""#),
        vec!["C:\\path with space\\"]
    );
    assert_eq!(split_windows_arguments("\"\""), vec![""]);
    assert_eq!(split_windows_arguments("\"a b\""), vec!["a b"]);
    assert!(split_windows_arguments("   ").is_empty());

    let arguments = vec![
        "one".to_string(),
        "two words".to_string(),
        "quote\"inside".to_string(),
        "trailing\\".to_string(),
        String::new(),
    ];
    let joined = join_windows_arguments(&arguments);
    assert_eq!(split_windows_arguments(&joined), arguments);
}

#[tokio::test]
async fn test_if_else_propagates_nested_script_output() {
    let action = Action::IfElse {
        conditions: vec![],
        then_actions: vec![Action::ExecuteScript {
            interpreter: crate::models::ScriptType::Lua,
            script: "return 'nested-output'".to_string(),
            run_in_window: false,
        }],
        else_if_branches: vec![],
        else_actions: vec![],
    };
    let res = ActionRunner::execute(&action, &HashMap::new()).await;
    assert_eq!(
        res,
        Ok(ActionOutcome::Continue(Some("nested-output".to_string())))
    );
}

#[tokio::test]
async fn test_powershell_script_sees_syn_prefixed_context() {
    let mut context = HashMap::new();
    context.insert("ProcessName".to_string(), "probe.exe".to_string());
    let out = super::script::run_script_internal(
        &crate::models::ScriptType::PowerShell,
        "Write-Output $env:SYN_PROCESSNAME",
        false,
        &context,
    )
    .await
    .expect("the script should run");
    assert_eq!(out.as_deref(), Some("probe.exe"));
}

#[test]
fn test_macro_plan_resolution_prefers_preset_defaults() {
    let inline_event = crate::models::MacroEvent::Delay {
        ms: 5,
        organic: false,
        jitter_percent: 0,
    };
    let preset = crate::models::MacroPreset {
        id: "preset_actions_test".into(),
        name: "Preset".into(),
        events: vec![crate::models::MacroEvent::Delay {
            ms: 99,
            organic: false,
            jitter_percent: 0,
        }],
        repeat: 3,
        speed: 2.0,
        infinite: true,
    };

    let (events, repeat, speed, infinite) = resolve_macro_plan(
        std::slice::from_ref(&inline_event),
        1,
        1.0,
        false,
        Some(&preset),
    );
    assert_eq!(events, vec![inline_event.clone()]);
    assert_eq!(repeat, 3);
    assert!((speed - 2.0).abs() < f32::EPSILON);
    assert!(infinite);

    let (events, repeat, speed, infinite) = resolve_macro_plan(&[], 2, 0.5, false, Some(&preset));
    assert_eq!(events, preset.events);
    assert_eq!(repeat, 2);
    assert!((speed - 0.5).abs() < f32::EPSILON);
    assert!(infinite);

    let (events, repeat, speed, infinite) = resolve_macro_plan(
        std::slice::from_ref(&inline_event),
        1,
        1.0,
        false,
        None,
    );
    assert_eq!(events, vec![inline_event]);
    assert_eq!(repeat, 1);
    assert!((speed - 1.0).abs() < f32::EPSILON);
    assert!(!infinite);
}

#[tokio::test]
async fn test_run_macro_finite_cleans_up_registration() {
    let _guard = MACRO_REGISTRY_TEST_LOCK.lock().await;
    let key = "test_run_macro_helper_key";
    let res = run_macro(vec![], 2, 1.0, false, Some(key.to_string()), &HashMap::new()).await;
    assert!(res.is_ok());
    assert!(!is_macro_running(key));
}

#[test]
fn test_read_capped_bounds_output() {
    let data = vec![b'x'; 1000];
    assert_eq!(read_capped(std::io::Cursor::new(data.clone()), 100).len(), 100);
    assert_eq!(read_capped(std::io::Cursor::new(data), 2000).len(), 1000);
}

#[test]
fn test_bounded_command_runner_maps_status_and_timeout() {
    let mut ok_cmd = Command::new("cmd");
    ok_cmd.args(["/C", "exit 0"]);
    let ok = run_command_bounded(ok_cmd, std::time::Duration::from_secs(10)).expect("exit 0");
    assert!(ok.succeeded(), "exit code {:?}", ok.exit_code());

    let mut failing_cmd = Command::new("cmd");
    failing_cmd.args(["/C", "exit 7"]);
    let failing = run_command_bounded(failing_cmd, std::time::Duration::from_secs(10))
        .expect("exit 7");
    assert!(!failing.succeeded());
    assert_eq!(failing.exit_code(), Some(7));

    let mut slow_cmd = Command::new("cmd");
    slow_cmd.args(["/C", "ping -n 6 127.0.0.1 >nul"]);
    let started = std::time::Instant::now();
    let slow =
        run_command_bounded(slow_cmd, std::time::Duration::from_millis(200)).expect("timeout");
    assert!(slow.timed_out);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn test_audio_endpoint_name_helpers() {
    assert_eq!(
        endpoint_registry_guid("{0.0.1.00000000}.{abc-def}"),
        Some("{abc-def}")
    );
    assert_eq!(endpoint_registry_guid("no-dots"), None);

    let names = vec![
        "Speakers (Realtek(R) Audio)".to_string(),
        "Speakers".to_string(),
        "Realtek(R) Audio".to_string(),
    ];
    assert!(device_name_matches("speakers", &names));
    assert!(device_name_matches("REALTEK", &names));
    assert!(device_name_matches("Speakers (Realtek", &names));
    assert!(!device_name_matches("Microphone", &names));
    assert!(!device_name_matches("   ", &names));
}

#[tokio::test]
async fn test_process_tuning_current_process() {
    let mut ctx = HashMap::new();
    ctx.insert("ProcessId".to_string(), std::process::id().to_string());
    let action = crate::models::Action::ProcessTuning {
        target_process: None,
        priority: Some(crate::models::ProcessPriorityLevel::Normal),
        affinity_mask: None,
        trim_memory: false,
    };
    let res = ActionRunner::execute(&action, &ctx).await;
    assert!(res.is_ok(), "tuning own process should succeed: {res:?}");
}
