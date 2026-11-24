use tokio::sync::oneshot;

use crate::daemon::rule_call;
use crate::daemon::watcher::ProcessEvent;
use crate::ipc::{IpcRequest, IpcResponse};
use crate::models::{Action, ConditionOperator, ProcessMatchMode, RuleCondition, RuleConditionItem, ScheduleType, Trigger};

use super::executions::next_execution_id;
use super::pipeline::{run_action_pipeline, PipelineRun};
use super::projector::StateProjector;

use super::*;
use chrono::{Datelike, Timelike};

#[tokio::test]
async fn test_bound_overlay_companion_lifecycle() {
    let mut config = AppConfig::default();
    let overlay_id = "companion_overlay_hud".to_string();
    let rule = Rule {
        id: "game_with_overlay".into(),
        name: "Game with Overlay HUD".into(),
        description: "".into(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Process {
            process_name: "mygame.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: false,
        }),
        actions: vec![
            Action::Overlay {
                overlay_id: overlay_id.clone(),
                custom_overlay: None,
                operation: crate::models::OverlayOperation::Show,
                override_text: None,
                duration_ms: None,
                close_on_exit: true,
            },
        ],
        ..Default::default()
    };
    config.rules.push(rule.clone());

    let mut engine = DaemonEngine::new(config);

    // 1. Process starts -> triggers rule execution
    engine.handle_process_event(ProcessEvent {
        pid: 4321,
        process_name: "mygame.exe".into(),
        is_started: true,
    }).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine.drain_action_events();

    // Overlay is registered as bound companion
    assert_eq!(engine.executions.bound_overlays().len(), 1);
    assert_eq!(engine.executions.bound_overlays()[0].overlay_id, overlay_id);
    assert_eq!(engine.executions.bound_overlays()[0].primary_pid, 4321);

    // State reflects companion supervisor execution
    let state = engine.get_current_state();
    assert!(state.active_rule_executions.contains_key("game_with_overlay"));

    // 2. Process exits -> auto-closes bound overlay
    engine.handle_process_event(ProcessEvent {
        pid: 4321,
        process_name: "mygame.exe".into(),
        is_started: false,
    }).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine.drain_action_events();

    assert_eq!(engine.executions.bound_overlays().len(), 0, "Bound overlay must be removed after primary process exit");
    assert!(engine.logs.iter().any(|l| l.message.contains("Auto-closing bound overlay")));
}

#[test]
fn test_log_limit_500() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    assert_eq!(engine.logs.len(), 0);

    for i in 1..=600 {
        engine.log("INFO", "TEST", &format!("Log message {}", i));
    }

    assert_eq!(engine.logs.len(), 500);
    assert_eq!(engine.logs[0].message, "Log message 101");
    assert_eq!(engine.logs[499].message, "Log message 600");
}

#[test]
fn test_sensitive_action_log_keeps_details_in_app_log() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    engine
        .action_tx
        .send(ActionCompletionEvent::SensitiveLog {
            level: "ERROR".to_string(),
            category: "ACTION".to_string(),
            message: "Action failed in 'Rule': script output with SECRET".to_string(),
            file_message: "Action failed in 'Rule' at step 2 (details omitted from the file log)"
                .to_string(),
        })
        .expect("engine event channel is open");
    engine.drain_action_events();

    let entry = engine.logs.last().expect("action log entry");
    assert_eq!(entry.level, "ERROR");
    assert_eq!(entry.category, "ACTION");
    assert!(
        entry.message.contains("SECRET"),
        "the in-app log must keep the full action error"
    );
}

#[test]
fn test_log_deduplication_aggregation() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    assert_eq!(engine.logs.len(), 0);

    engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
    assert_eq!(engine.logs.len(), 1);
    assert_eq!(engine.logs[0].message, "Rule 'Test' conditions were not met");

    // Consecutive repeat increments to (x2)
    engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
    assert_eq!(engine.logs.len(), 1);
    assert_eq!(engine.logs[0].message, "Rule 'Test' conditions were not met (x2)");

    // Consecutive repeat increments to (x3)
    engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
    assert_eq!(engine.logs.len(), 1);
    assert_eq!(engine.logs[0].message, "Rule 'Test' conditions were not met (x3)");

    // Different message creates a new log line
    engine.log("INFO", "RULE", "Different message");
    assert_eq!(engine.logs.len(), 2);
    assert_eq!(engine.logs[1].message, "Different message");

    // Repeat of original message now creates a new entry
    engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
    assert_eq!(engine.logs.len(), 3);
    assert_eq!(engine.logs[2].message, "Rule 'Test' conditions were not met");
}

#[test]
fn test_matches_process() {
    assert!(DaemonEngine::matches_process("notepad.exe", "notepad.exe", ProcessMatchMode::Exact));
    assert!(DaemonEngine::matches_process("NOTEPAD.EXE", "notepad.exe", ProcessMatchMode::Exact));
    assert!(!DaemonEngine::matches_process("notepad.exe", "calc.exe", ProcessMatchMode::Exact));

    assert!(DaemonEngine::matches_process("*game*", "my_game_client.exe", ProcessMatchMode::Wildcard));
    assert!(DaemonEngine::matches_process("*.exe", "anything.exe", ProcessMatchMode::Wildcard));
    assert!(DaemonEngine::matches_process("steam*", "steamwebhelper.exe", ProcessMatchMode::Wildcard));
    assert!(!DaemonEngine::matches_process("*.exe", "anything.dll", ProcessMatchMode::Wildcard));

    assert!(DaemonEngine::matches_process("chrome", "google-chrome-stable", ProcessMatchMode::Regex));
    assert!(!DaemonEngine::matches_process("firefox", "google-chrome-stable", ProcessMatchMode::Regex));
    assert!(DaemonEngine::matches_process(r"^notepad\.exe$", "notepad.exe", ProcessMatchMode::Regex));
    assert!(DaemonEngine::matches_process(r"^NOTEPAD\.EXE$", "notepad.exe", ProcessMatchMode::Regex));
    assert!(!DaemonEngine::matches_process(r"^notepad\.exe$", "my_notepad.exe", ProcessMatchMode::Regex));
    assert!(DaemonEngine::matches_process(r"(chrome|edge)\.exe", "edge.exe", ProcessMatchMode::Regex));
    assert!(DaemonEngine::matches_process(r"game_\d+\.exe", "game_42.exe", ProcessMatchMode::Regex));
    assert!(!DaemonEngine::matches_process(r"game_\d+\.exe", "game_abc.exe", ProcessMatchMode::Regex));
    // Graceful handling of invalid regex pattern without panicking (falls back to contains)
    assert!(DaemonEngine::matches_process("[invalid", "test_[invalid_proc", ProcessMatchMode::Regex));
}

#[test]
fn test_device_filter_matches() {
    let device = r"\\?\USB#VID_046D&PID_C52B#12345678#{guid}";
    assert!(DaemonEngine::device_filter_matches(None, Some(device)));
    assert!(DaemonEngine::device_filter_matches(Some(""), Some(device)));
    assert!(DaemonEngine::device_filter_matches(Some("vid_046d"), Some(device)));
    assert!(DaemonEngine::device_filter_matches(Some("USB"), Some(device)));
    assert!(!DaemonEngine::device_filter_matches(Some("PCI"), Some(device)));
    assert!(!DaemonEngine::device_filter_matches(Some("USB"), None));
    assert!(DaemonEngine::device_filter_matches(Some("*"), None));
}

#[test]
fn test_rule_with_companion_mode_and_exit_actions_serde() {
    let json = r#"{
        "id": "rule_1",
        "name": "Game with Companion",
        "is_enabled": true,
        "trigger": {
            "$trigger_type": "process",
            "process_name": "game.exe",
            "is_started": true,
            "match_mode": "exact"
        },
        "actions": [
            {
                "$action_type": "start_process",
                "file_path": "companion.exe",
                "close_on_exit": true,
                "run_hidden": true
            }
        ],
        "on_exit_actions": [
            {
                "$action_type": "notification",
                "title": "Game Over",
                "message": "Game exited"
            }
        ]
    }"#;

    let rule: Rule = serde_json::from_str(json).expect("deserialize rule");
    assert_eq!(rule.name, "Game with Companion");
    assert_eq!(rule.actions.len(), 1);
    if let Action::StartProcess { close_on_exit, run_hidden, .. } = &rule.actions[0] {
        assert!(*close_on_exit);
        assert!(*run_hidden);
    } else {
        panic!("Expected StartProcess");
    }

    assert_eq!(rule.on_exit_actions.len(), 1);
    if let Action::Notification { title, .. } = &rule.on_exit_actions[0] {
        assert_eq!(title, "Game Over");
    } else {
        panic!("Expected Notification");
    }
}

#[test]
fn test_process_audio_rule_serde() {
    use crate::models::{AudioTarget, AudioVolumeAction};

    let json = r#"{
        "id": "audio_rule_1",
        "name": "Game Audio Intro Muter",
        "is_enabled": true,
        "actions": [
            {
                "$action_type": "audio_control",
                "target": "process",
                "process_name": "game.exe",
                "action_type": "mute",
                "volume_level": 100
            },
            {
                "$action_type": "wait",
                "mode": "timer",
                "seconds": 5.0
            },
            {
                "$action_type": "audio_control",
                "target": "process",
                "process_name": "{ProcessName}",
                "action_type": "unmute",
                "volume_level": 100
            }
        ]
    }"#;

    let rule: Rule = serde_json::from_str(json).expect("deserialize process audio rule");
    assert_eq!(rule.name, "Game Audio Intro Muter");
    assert_eq!(rule.actions.len(), 3);

    if let Action::AudioControl { target, process_name, action_type, .. } = &rule.actions[0] {
        assert_eq!(*target, AudioTarget::Process);
        assert_eq!(process_name.as_deref(), Some("game.exe"));
        assert_eq!(*action_type, AudioVolumeAction::Mute);
    } else {
        panic!("Expected AudioControl Process Mute");
    }

    if let Action::Wait { seconds, .. } = &rule.actions[1] {
        assert_eq!(*seconds, 5.0);
    } else {
        panic!("Expected Wait");
    }

    if let Action::AudioControl { target, process_name, action_type, .. } = &rule.actions[2] {
        assert_eq!(*target, AudioTarget::Process);
        assert_eq!(process_name.as_deref(), Some("{ProcessName}"));
        assert_eq!(*action_type, AudioVolumeAction::Unmute);
    } else {
        panic!("Expected AudioControl Process Unmute");
    }

    let serialized = serde_json::to_string(&rule).expect("serialize rule");
    assert!(serialized.contains("audio_control"));
    assert!(serialized.contains("game.exe"));
}

#[test]
fn test_microphone_rule_serde() {
    use crate::models::{AudioTarget, AudioVolumeAction};

    let json = r#"{
        "id": "mic_rule_1",
        "name": "Mic Muter",
        "is_enabled": true,
        "actions": [
            {
                "$action_type": "audio_control",
                "target": "microphone",
                "action_type": "toggle_mute",
                "volume_level": 80
            }
        ]
    }"#;

    let rule: Rule = serde_json::from_str(json).expect("deserialize mic rule");
    assert_eq!(rule.name, "Mic Muter");
    assert_eq!(rule.actions.len(), 1);

    if let Action::AudioControl { target, action_type, volume_level, .. } = &rule.actions[0] {
        assert_eq!(*target, AudioTarget::Microphone);
        assert_eq!(*action_type, AudioVolumeAction::ToggleMute);
        assert_eq!(*volume_level, 80);
    } else {
        panic!("Expected Action::AudioControl Microphone");
    }

    let serialized = serde_json::to_string(&rule).expect("serialize mic rule");
    assert!(serialized.contains("audio_control"));
    assert!(serialized.contains("toggle_mute"));
}







#[tokio::test]
async fn test_schedule_trigger_interval() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "sched_rule".into(),
        name: "Interval Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 2,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![Action::Notification {
            title: "Timer".into(),
            message: "Tick".into(),
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    // First tick sets initial timestamp
    engine.tick_schedule().await;
    assert_eq!(engine.logs.len(), 0);

    // Artificially advance timer past the 2 second interval. The scheduler keeps its own
    // attempt stamp next to the accepted-trigger stamp, so both have to move back.
    let past = Instant::now() - std::time::Duration::from_secs(3);
    engine.rule_last_triggered.insert("sched_rule".into(), past);
    engine.scheduler.last_attempt.insert("sched_rule".into(), past);

    // Next tick triggers rule execution
    engine.tick_schedule().await;
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("triggered")));
}

#[tokio::test]
async fn test_schedule_trigger_daily_with_seconds() {
    let mut config = AppConfig::default();
    let now = chrono::Local::now();
    let time_str = format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());

    config.rules.push(Rule {
        id: "daily_rule".into(),
        name: "Daily Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Daily,
            interval_sec: 0,
            daily_time: Some(time_str),
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![Action::Notification {
            title: "Daily".into(),
            message: "Alarm".into(),
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    let tick_now = chrono::Local::now();
    let tick_time_str = format!("{:02}:{:02}:{:02}", tick_now.hour(), tick_now.minute(), tick_now.second());
    if let Some(r) = engine.config.rules.first_mut() {
        if let Some(Trigger::Schedule { daily_time, .. }) = &mut r.trigger {
            *daily_time = Some(tick_time_str);
        }
    }
    engine.tick_schedule().await;
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Daily Rule")));
}

#[tokio::test]
async fn test_schedule_trigger_oneshot_with_seconds() {
    let mut config = AppConfig::default();
    let past = chrono::Local::now() - chrono::Duration::seconds(5);
    let dt_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        past.year(), past.month(), past.day(), past.hour(), past.minute(), past.second()
    );

    config.rules.push(Rule {
        id: "oneshot_rule".into(),
        name: "OneShot Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::OneShot,
            interval_sec: 0,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: Some(dt_str),
        }),
        actions: vec![Action::Notification {
            title: "OneShot".into(),
            message: "Fired".into(),
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    engine.tick_schedule().await;
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("OneShot Rule")));
    assert!(!engine.config.rules[0].is_enabled); // OneShot automatically deactivates after execution
}

#[tokio::test]
async fn test_schedule_trigger_interval_with_start_datetime() {
    let mut config = AppConfig::default();
    // Set future start datetime (1 hour in future)
    let future = chrono::Local::now() + chrono::Duration::hours(1);
    let future_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        future.year(), future.month(), future.day(), future.hour(), future.minute(), future.second()
    );

    config.rules.push(Rule {
        id: "future_interval_rule".into(),
        name: "Future Interval Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 10,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: Some(future_str),
        }),
        actions: vec![Action::Notification {
            title: "Timer".into(),
            message: "Tick".into(),
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    engine.tick_schedule().await;
    // Should not have run because future time has not arrived
    assert!(!engine.logs.iter().any(|l| l.message.contains("Future Interval Rule")));

    // Now set past start datetime (5 seconds ago)
    let past = chrono::Local::now() - chrono::Duration::seconds(5);
    let past_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        past.year(), past.month(), past.day(), past.hour(), past.minute(), past.second()
    );
    if let Some(Trigger::Schedule { target_datetime, .. }) = &mut engine.config.rules[0].trigger {
        *target_datetime = Some(past_str);
    }

    engine.tick_schedule().await;
    // Should have run immediately on first execution after start time
    assert!(engine.logs.iter().any(|l| l.message.contains("Future Interval Rule")));
}

#[tokio::test]
async fn test_execute_shutdown_rules() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "shutdown_rule".into(),
        name: "Shutdown Cleanup".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::AppShutdown,
            interval_sec: 0,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![Action::Notification {
            title: "Bye".into(),
            message: "Closing".into(),
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    engine.execute_shutdown_rules().await;
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Shutdown Cleanup")));
}

#[tokio::test]
async fn test_single_instance_process_lifecycle() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "si_rule".into(),
        name: "Single Instance App".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Process {
            process_name: "testapp.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: true,
        }),
        actions: vec![Action::Notification {
            title: "Started".into(),
            message: "App started".into(),
        }],
        on_exit_actions: vec![Action::Notification {
            title: "Exited".into(),
            message: "All instances exited".into(),
        }],
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);

    // 1. First process starts (PID 1000) -> triggers rule
    engine.handle_process_event(ProcessEvent {
        pid: 1000,
        process_name: "testapp.exe".into(),
        is_started: true,
    }).await;

    let start_logs: Vec<_> = engine.logs.iter()
        .filter(|l| l.category == "RULE" && l.message.contains("triggered by testapp.exe") && !l.message.contains("exit actions"))
        .collect();
    assert_eq!(start_logs.len(), 1, "First instance must trigger the rule");

    // 2. Second process of same exe starts (PID 1001) -> must be SKIPPED
    engine.handle_process_event(ProcessEvent {
        pid: 1001,
        process_name: "testapp.exe".into(),
        is_started: true,
    }).await;

    let start_logs_after_second: Vec<_> = engine.logs.iter()
        .filter(|l| l.category == "RULE" && l.message.contains("triggered by testapp.exe") && !l.message.contains("exit actions"))
        .collect();
    assert_eq!(start_logs_after_second.len(), 1, "Second instance must NOT trigger the rule");
    assert!(engine.logs.iter().any(|l| l.message.contains("skipped (Single Instance)")));

    // 3. First process exits (PID 1000), but second process (PID 1001) is still active
    // -> on_exit_actions must NOT run yet
    engine.handle_process_event(ProcessEvent {
        pid: 1000,
        process_name: "testapp.exe".into(),
        is_started: false,
    }).await;

    let exit_logs: Vec<_> = engine.logs.iter()
        .filter(|l| l.category == "RULE" && l.message.contains("exit actions triggered"))
        .collect();
    assert_eq!(exit_logs.len(), 0, "Exit actions must NOT trigger while another instance is running");

    // 4. Second process exits (PID 1001) -> all instances closed -> on_exit_actions MUST trigger!
    engine.handle_process_event(ProcessEvent {
        pid: 1001,
        process_name: "testapp.exe".into(),
        is_started: false,
    }).await;

    let exit_logs_after_all: Vec<_> = engine.logs.iter()
        .filter(|l| l.category == "RULE" && l.message.contains("exit actions triggered"))
        .collect();
    assert_eq!(exit_logs_after_all.len(), 1, "Exit actions MUST trigger when last instance closes");

    // 5. Subsequent new launch (PID 2000) -> should trigger again as new first instance!
    engine.handle_process_event(ProcessEvent {
        pid: 2000,
        process_name: "testapp.exe".into(),
        is_started: true,
    }).await;

    let final_start_logs: Vec<_> = engine.logs.iter()
        .filter(|l| l.category == "RULE" && l.message.contains("triggered by testapp.exe") && !l.message.contains("exit actions"))
        .collect();
    assert_eq!(final_start_logs.len(), 2, "Subsequent launch after all instances closed must trigger again");
}

#[tokio::test]
async fn test_process_termination_only_rule() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "exit_only_rule".into(),
        name: "Exit Only".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Process {
            process_name: "exitapp.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: true,
        }),
        actions: Vec::new(),
        on_exit_actions: vec![Action::Notification {
            title: "Exited".into(),
            message: "Closed".into(),
        }],
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);

    // Process starts: actions is empty, so no rule trigger log
    engine.handle_process_event(ProcessEvent {
        pid: 500,
        process_name: "exitapp.exe".into(),
        is_started: true,
    }).await;

    assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("triggered by exitapp.exe")));

    // Process terminates: exit actions trigger!
    engine.handle_process_event(ProcessEvent {
        pid: 500,
        process_name: "exitapp.exe".into(),
        is_started: false,
    }).await;

    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("exit actions triggered by exitapp.exe")));
}

#[tokio::test]
async fn test_rule_delay_does_not_block_engine() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "delayed_rule".into(),
        name: "Delayed Automation".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.5,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
            Action::Notification {
                title: "Test".into(),
                message: "Completed".into(),
            },
        ],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    let start = Instant::now();

    // 1. trigger MUST return immediately without blocking on the 0.5s delay
    let handle = engine.trigger(&engine.config.rules[0].clone(), ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    let elapsed_on_return = start.elapsed();
    assert!(
        elapsed_on_return < std::time::Duration::from_millis(150),
        "trigger took too long to return: {:?}",
        elapsed_on_return
    );

    // 2. The trigger log was generated immediately
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Delayed Automation")));

    // 3. Engine is free to perform other operations immediately (e.g. handle requests, state query)
    let resp = engine.handle_request(IpcRequest::Ping).await;
    assert!(matches!(resp, IpcResponse::Pong));

    // 4. Awaiting the task handle waits for the actual background delay to complete
    assert!(handle.is_some());
    let _ = handle.unwrap().await;
    assert!(start.elapsed() >= std::time::Duration::from_millis(450));
}

#[tokio::test]
async fn test_concurrent_delays_do_not_block_each_other() {
    let config = AppConfig::default();
    let rule_long = Rule {
        id: "long_rule".into(),
        name: "Long Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 0.4,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    };
    let rule_short = Rule {
        id: "short_rule".into(),
        name: "Short Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 0.1,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(config);
    let start = Instant::now();

    // Launch long rule first, then short rule immediately after
    let handle_long = engine.trigger(&rule_long, ExecutionOrigin::SystemEvent, "Test Concurrent", 0, HashMap::new()).await.unwrap();
    let handle_short = engine.trigger(&rule_short, ExecutionOrigin::SystemEvent, "Test Concurrent", 0, HashMap::new()).await.unwrap();

    // The short rule must complete before the long rule, without being blocked by it
    handle_short.await.unwrap();
    let short_duration = start.elapsed();
    assert!(short_duration < std::time::Duration::from_millis(300), "Short rule was blocked: {:?}", short_duration);

    handle_long.await.unwrap();
    let total_duration = start.elapsed();
    assert!(total_duration >= std::time::Duration::from_millis(380), "Long rule finished too early: {:?}", total_duration);
}

#[tokio::test]
async fn test_rule_execution_tracking_concurrent_and_step() {
    let config = AppConfig::default();
    let rule = Rule {
        id: "tracked_rule".into(),
        name: "Tracked Multi-Step Rule".into(),
        description: "Testing execution tracking".into(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.2,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.2,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
        ],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(config);

    // Initially no active executions
    let state0 = engine.get_current_state();
    assert!(!state0.active_rule_executions.contains_key("tracked_rule"));

    // Launch instance 1
    let h1 = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await.unwrap();
    // Give tokio runtime a brief tick to process channel event
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let state1 = engine.get_current_state();
    assert!(state1.active_rule_executions.contains_key("tracked_rule"));
    let exec1 = &state1.active_rule_executions["tracked_rule"];
    assert_eq!(exec1.active_count, 1);
    assert_eq!(exec1.current_step, 0);
    assert_eq!(exec1.total_steps, 2);

    // Launch instance 2 concurrently
    let h2 = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let state2 = engine.get_current_state();
    let exec2 = &state2.active_rule_executions["tracked_rule"];
    assert_eq!(exec2.active_count, 2, "Must report 2 concurrent active executions");

    // Wait for both to finish
    let _ = h1.await;
    let _ = h2.await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let state_done = engine.get_current_state();
    assert!(!state_done.active_rule_executions.contains_key("tracked_rule"));
}

#[tokio::test]
async fn test_stop_rule_execution_aborts_immediately() {
    let config = AppConfig::default();
    let rule = Rule {
        id: "long_running_rule".into(),
        name: "Long Running Rule".into(),
        description: "".into(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 5.0,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
        ],
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(config);
    let h = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    assert!(engine.is_rule_executing("long_running_rule"));

    // Stop execution
    engine.stop_rule_execution("long_running_rule");
    assert!(!engine.is_rule_executing("long_running_rule"));

    // Join handle should return error (cancelled)
    let res = h.await;
    assert!(res.is_err(), "Aborted task must return join error");
}

#[tokio::test]
async fn test_rule_activity_registry_tracks_executions() {
    use crate::daemon::rule_activity;

    let rule_id = "rule_activity_registry_rule";
    let rule = Rule {
        id: rule_id.into(),
        name: "Activity Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 5.0,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
        ],
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(AppConfig::default());
    assert!(!rule_activity::is_running(rule_id));

    let handle = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await.unwrap();
    assert!(
        rule_activity::is_running(rule_id),
        "a started pipeline must be observable as running"
    );
    assert!(DaemonEngine::evaluate_condition(&RuleCondition::RuleRunning {
        rule_id: rule_id.into(),
        must_be_running: true,
    }));

    engine.stop_rule_execution(rule_id);
    let _ = handle.await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(
        !rule_activity::is_running(rule_id),
        "an aborted pipeline must drop out of the registry"
    );
}

#[test]
fn test_rule_running_condition_evaluation() {
    use crate::daemon::rule_activity;

    let target = "rule_activity_test_condition_target";
    let running = RuleCondition::RuleRunning {
        rule_id: target.into(),
        must_be_running: true,
    };
    let stopped = RuleCondition::RuleRunning {
        rule_id: target.into(),
        must_be_running: false,
    };

    assert!(!DaemonEngine::evaluate_condition(&running));
    assert!(DaemonEngine::evaluate_condition(&stopped));

    rule_activity::mark_started(target);
    assert!(DaemonEngine::evaluate_condition(&running));
    assert!(!DaemonEngine::evaluate_condition(&stopped));

    rule_activity::mark_finished(target);
    assert!(!DaemonEngine::evaluate_condition(&running));
    assert!(DaemonEngine::evaluate_condition(&stopped));

    // An empty target is invalid and never satisfies the condition, in either direction.
    for must_be_running in [true, false] {
        assert!(!DaemonEngine::evaluate_condition(&RuleCondition::RuleRunning {
            rule_id: String::new(),
            must_be_running,
        }));
    }
}

#[tokio::test]
async fn test_overlay_rule_execution_tracking_and_stop() {
    let _overlay_guard = lock_overlay_state().await;
    crate::daemon::overlay_manager::init();
    let overlay_id = "test_overlay_tracking_id".to_string();
    let rule = Rule {
        id: "overlay_rule".into(),
        name: "Overlay Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::PlaySound { file_path: "".into(), play_beep: false, beep_frequency: None, beep_duration_ms: None },
            Action::Overlay {
                overlay_id: overlay_id.clone(),
                custom_overlay: None,
                operation: crate::models::OverlayOperation::Toggle,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            },
        ],
        on_exit_actions: Vec::new(),
        execution_count: 0,
        custom_icon_path: None,
        auto_process_icon: false,
        ..Default::default()
    };

    let config = AppConfig {
        rules: vec![rule],
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(config);

    // Initially overlay is not active
    let state0 = engine.get_current_state();
    assert!(!state0.active_rule_executions.contains_key("overlay_rule"));
    assert!(!engine.is_rule_executing("overlay_rule"));

    // Simulate overlay becoming active in overlay_manager
    crate::daemon::overlay_manager::show_overlay(
        &crate::models::OverlayDefinition {
            id: overlay_id.clone(),
            ..Default::default()
        },
        false,
        None,
    );

    // Allow background thread to process Show command
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Overlay is active in overlay_manager
    let active_ovs = crate::daemon::overlay_manager::get_active_overlay_ids();
    assert!(active_ovs.contains(&overlay_id));

    // Rule MUST report as executing, pointing to step 1 (the Action::Overlay step)
    assert!(engine.is_rule_executing("overlay_rule"));
    let state1 = engine.get_current_state();
    assert!(state1.active_rule_executions.contains_key("overlay_rule"));
    let exec = &state1.active_rule_executions["overlay_rule"];
    assert_eq!(exec.current_step, 1);
    assert_eq!(exec.total_steps, 2);
    assert!(state1.active_overlay_ids.contains(&overlay_id));

    // Stopping the rule MUST hide the overlay
    engine.stop_rule_execution("overlay_rule");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let active_ovs_after = crate::daemon::overlay_manager::get_active_overlay_ids();
    assert!(!active_ovs_after.contains(&overlay_id));
    assert!(!engine.is_rule_executing("overlay_rule"));

    let state_after = engine.get_current_state();
    assert!(!state_after.active_rule_executions.contains_key("overlay_rule"));
}

#[test]
fn test_rule_references_active_overlay_scans_if_else_branches() {
    let nested_overlay_id = "nested_overlay_hud";
    let rule = Rule {
        id: "nested_overlay_rule".into(),
        name: "Nested Overlay Rule".into(),
        actions: vec![Action::IfElse {
            conditions: Vec::new(),
            then_actions: vec![Action::Overlay {
                overlay_id: nested_overlay_id.into(),
                custom_overlay: None,
                operation: crate::models::OverlayOperation::Show,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            }],
            else_if_branches: Vec::new(),
            else_actions: Vec::new(),
        }],
        ..Default::default()
    };
    assert!(
        !rule.actions.iter().any(|action| matches!(action, Action::Overlay { .. })),
        "the overlay must live only inside the IfElse branch for this test to prove nesting"
    );

    let mut active = HashSet::new();
    active.insert(nested_overlay_id.to_string());
    assert!(
        DaemonEngine::rule_references_active_overlay(&rule, &active),
        "an overlay active inside an IfElse branch must report the rule as executing"
    );

    let mut other_active = HashSet::new();
    other_active.insert("some_other_overlay".to_string());
    assert!(
        !DaemonEngine::rule_references_active_overlay(&rule, &other_active),
        "an unrelated active overlay id must not report this rule as executing"
    );
    assert!(!DaemonEngine::rule_references_active_overlay(&rule, &HashSet::new()));
}

#[test]
fn test_condition_evaluation_logic() {
    let temp_dir = std::env::temp_dir();
    let existing_path = temp_dir.join("synector_test_cond.tmp");
    let _ = std::fs::write(&existing_path, "test");
    let non_existing_path = temp_dir.join("synector_non_existing_file_12345.tmp");

    let cond_file_exists = RuleCondition::FileWatcher {
        path: existing_path.to_string_lossy().to_string(),
        filter_extension: None,
        must_exist: true,
    };
    let cond_file_not_exists = RuleCondition::FileWatcher {
        path: non_existing_path.to_string_lossy().to_string(),
        filter_extension: None,
        must_exist: true,
    };

    assert!(DaemonEngine::evaluate_condition(&cond_file_exists));
    assert!(!DaemonEngine::evaluate_condition(&cond_file_not_exists));

    let cond_file_must_not_exist = RuleCondition::FileWatcher {
        path: non_existing_path.to_string_lossy().to_string(),
        filter_extension: None,
        must_exist: false,
    };
    assert!(DaemonEngine::evaluate_condition(&cond_file_must_not_exist));

    // Test operators: (1 AND 2) OR (3 AND 4)
    // Case 1: C0 (true) AND C1 (true) -> true
    let all_ok = vec![
        RuleConditionItem::new(cond_file_exists.clone()),
        RuleConditionItem::with_operator(ConditionOperator::And, cond_file_must_not_exist.clone()),
    ];
    assert!(DaemonEngine::evaluate_rule_conditions(&all_ok));

    // Case 2: C0 (true) AND C1 (false) -> false
    let one_fail = vec![
        RuleConditionItem::new(cond_file_exists.clone()),
        RuleConditionItem::with_operator(ConditionOperator::And, cond_file_not_exists.clone()),
    ];
    assert!(!DaemonEngine::evaluate_rule_conditions(&one_fail));

    // Case 3: C0 (true) AND C1 (false) OR C2 (true) -> true (first group fails, second group succeeds)
    let or_branch = vec![
        RuleConditionItem::new(cond_file_exists.clone()),
        RuleConditionItem::with_operator(ConditionOperator::And, cond_file_not_exists.clone()),
        RuleConditionItem::with_operator(ConditionOperator::Or, cond_file_must_not_exist.clone()),
    ];
    assert!(DaemonEngine::evaluate_rule_conditions(&or_branch));

    // Case 4: (1 and 2) or (3 and 4)
    let two_by_two = vec![
        RuleConditionItem::new(cond_file_exists.clone()),
        RuleConditionItem::with_operator(ConditionOperator::And, cond_file_not_exists.clone()), // Group 1 fails
        RuleConditionItem::with_operator(ConditionOperator::Or, cond_file_exists.clone()),
        RuleConditionItem::with_operator(ConditionOperator::And, cond_file_must_not_exist.clone()), // Group 2 passes
    ];
    assert!(DaemonEngine::evaluate_rule_conditions(&two_by_two));

    let _ = std::fs::remove_file(&existing_path);
}

#[tokio::test]
async fn test_preconditions_gate_rule_execution() {
    let temp_dir = std::env::temp_dir();
    let non_existing_path = temp_dir.join("synector_never_exists_abc.tmp");

    let config = AppConfig::default();
    let rule = Rule {
        id: "gated_rule".into(),
        name: "Gated Rule".into(),
        description: "".into(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        conditions: vec![
            RuleConditionItem::new(RuleCondition::FileWatcher {
                path: non_existing_path.to_string_lossy().to_string(),
                filter_extension: None,
                must_exist: true,
            }),
        ],
        actions: vec![
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.05,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
        ],
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(config);

    // When triggered by a real trigger (e.g. process or schedule), conditions fail -> returns None
    let handle = engine.trigger(&rule, ExecutionOrigin::SystemEvent, "AUTO_TRIGGER", 0, HashMap::new()).await;
    assert!(handle.is_none(), "Rule execution must be skipped when preconditions are not met");

    // Preconditions gate rule execution consistently, including in manual/test runs
    let manual_handle = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(manual_handle.is_none(), "Manual test must also respect preconditions");

    let test_run_handle = engine.emulate_rule_trigger(&rule).await;
    assert!(test_run_handle.is_none(), "Test run emulation must skip when conditions fail");

    // When conditions are met (e.g. creating the file):
    let _ = std::fs::write(&non_existing_path, b"ok");
    let satisfied_handle = engine.emulate_rule_trigger(&rule).await;
    assert!(satisfied_handle.is_some(), "Test run emulation must execute when conditions are met");
    let _ = satisfied_handle.unwrap().await;
    let _ = std::fs::remove_file(&non_existing_path);
}

#[tokio::test]
async fn test_emulate_rule_trigger_interval() {
    let config = AppConfig::default();
    let rule = Rule {
        id: "interval_test_rule".into(),
        name: "Interval Emulation Rule".into(),
        description: "".into(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 1,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.05,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
        ],
        ..Default::default()
    };

    let mut engine = DaemonEngine::new(config);
    let handle = engine.emulate_rule_trigger(&rule).await;
    assert!(handle.is_some(), "Interval rule emulation must spawn a task");
    assert!(engine.is_rule_executing(&rule.id), "Rule must be tracked as executing");
    assert!(engine.executions.has_recurring_test(&rule.id));

    let state = engine.get_current_state();
    assert!(state.active_test_rule_ids.contains(&rule.id));
    assert!(state.active_rule_executions.contains_key(&rule.id));

    // Stopping execution cleanly terminates the loop
    engine.stop_rule_execution(&rule.id);
    assert!(!engine.is_rule_executing(&rule.id), "Rule must not be executing after stop");
    assert!(!engine.executions.has_recurring_test(&rule.id));

    let state_after = engine.get_current_state();
    assert!(!state_after.active_test_rule_ids.contains(&rule.id));
    assert!(!state_after.active_rule_executions.contains_key(&rule.id));
}

#[tokio::test]
async fn test_schedule_trigger_monthly_catch_up() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "monthly_rule".into(),
        name: "Monthly Catchup Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Monthly,
            interval_sec: 0,
            daily_time: Some("00:00:00".into()),
            days_of_week: Vec::new(),
            day_of_month: Some(1),
            target_datetime: None,
        }),
        actions: vec![Action::Notification {
            title: "Month".into(),
            message: "Run".into(),
        }],
        last_executed_at: None,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    engine.tick_schedule().await;
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Monthly Catchup Rule")));

    let cur_ym = chrono::Local::now().format("%Y-%m-01 12:00:00").to_string();
    engine.config.rules[0].last_executed_at = Some(cur_ym);
    engine.logs.clear();

    engine.tick_schedule().await;
    assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Monthly Catchup Rule")));
}

#[tokio::test]
async fn test_schedule_interval_persisted_timestamp() {
    let mut config = AppConfig::default();
    let last_ts = (chrono::Local::now() - chrono::Duration::seconds(20)).format("%Y-%m-%d %H:%M:%S").to_string();
    config.rules.push(Rule {
        id: "persisted_interval_rule".into(),
        name: "Persisted Interval Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 100,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![Action::Notification {
            title: "Timer".into(),
            message: "Tick".into(),
        }],
        last_executed_at: Some(last_ts),
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    engine.tick_schedule().await;
    assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Persisted Interval Rule")));
    assert!(engine.rule_last_triggered.contains_key("persisted_interval_rule"));
}

#[test]
fn test_matches_process_extension_normalization() {
    // Exact mode: automatically normalizes with/without .exe
    assert!(DaemonEngine::matches_process("notepad", "notepad.exe", ProcessMatchMode::Exact));
    assert!(DaemonEngine::matches_process("notepad.exe", "notepad", ProcessMatchMode::Exact));
    assert!(DaemonEngine::matches_process("notepad", "notepad", ProcessMatchMode::Exact));
    assert!(DaemonEngine::matches_process("NOTEPAD", "notepad.exe", ProcessMatchMode::Exact));
    assert!(DaemonEngine::matches_process("notepad.exe", "NOTEPAD.EXE", ProcessMatchMode::Exact));
    assert!(!DaemonEngine::matches_process("notepad", "calc.exe", ProcessMatchMode::Exact));
    assert!(!DaemonEngine::matches_process("notepad.exe", "calc", ProcessMatchMode::Exact));

    // Wildcard mode: supports with/without .exe
    assert!(DaemonEngine::matches_process("note*", "notepad.exe", ProcessMatchMode::Wildcard));
    assert!(DaemonEngine::matches_process("note*", "notepad", ProcessMatchMode::Wildcard));
    assert!(DaemonEngine::matches_process("*pad", "notepad.exe", ProcessMatchMode::Wildcard));
    assert!(DaemonEngine::matches_process("notepad", "notepad.exe", ProcessMatchMode::Wildcard));

    // Regex mode: supports matching process name without .exe
    assert!(DaemonEngine::matches_process(r"^notepad$", "notepad.exe", ProcessMatchMode::Regex));
    assert!(DaemonEngine::matches_process(r"^notepad\.exe$", "notepad.exe", ProcessMatchMode::Regex));
    assert!(!DaemonEngine::matches_process(r"^calc$", "notepad.exe", ProcessMatchMode::Regex));
}

#[tokio::test]
async fn test_stop_rule_execution_terminates_and_clears_bound_processes() {
    let mut config = AppConfig::default();
    let rule = Rule {
        id: "companion_rule".into(),
        name: "Companion Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::StartProcess {
                file_path: "satellite_tool.exe".into(),
                arguments: String::new(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: false,
                run_hidden: false,
                close_on_exit: true,
            },
        ],
        ..Default::default()
    };
    config.rules.push(rule);

    let mut engine = DaemonEngine::new(config);
    // Simulate active bound companion process registered for this rule
    engine.executions.add_bound_process(BoundProcess {
        rule_id: "companion_rule".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 1234,
        target_executable: "satellite_tool.exe".into(),
    });

    assert!(engine.is_rule_executing("companion_rule"), "Rule must report executing while bound processes are active");

    // Stop rule execution
    engine.stop_rule_execution("companion_rule");

    assert_eq!(engine.executions.bound_processes().len(), 0, "Bound processes must be cleared upon stopping rule");
    assert!(!engine.is_rule_executing("companion_rule"), "is_rule_executing must be false after stopping rule");
}

#[tokio::test]
async fn test_manual_run_clears_and_ignores_bound_processes() {
    let mut config = AppConfig::default();
    let rule = Rule {
        id: "manual_companion_rule".into(),
        name: "Manual Companion Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::StartProcess {
                file_path: "cmd.exe".into(),
                arguments: "/c exit 0".into(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: false,
                run_hidden: false,
                close_on_exit: true,
            },
        ],
        ..Default::default()
    };
    config.rules.push(rule.clone());

    let mut engine = DaemonEngine::new(config);
    // Pre-populate stale bound process from earlier
    engine.executions.add_bound_process(BoundProcess {
        rule_id: "manual_companion_rule".into(),
        primary_process_name: "old_game.exe".into(),
        primary_pid: 9999,
        target_executable: "satellite_tool.exe".into(),
    });

    // Trigger manual run (pid == 0 / "Manual Run")
    let h = engine.trigger(&rule, ExecutionOrigin::Manual, "Manual Run", 0, HashMap::new()).await;
    if let Some(handle) = h {
        let _ = handle.await;
    }
    engine.drain_action_events();

    assert_eq!(engine.executions.bound_processes().len(), 0, "Manual run must clear leftover bound processes and avoid registering new ones without host PID");
    assert!(!engine.is_rule_executing("manual_companion_rule"), "Manual run must not hang in executing state");
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("after manual execution of rule 'manual_companion_rule'")), "Must log force terminating bound satellite after manual run");
}

#[tokio::test]
async fn test_schedule_trigger_daily_catch_up() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "daily_catchup_rule".into(),
        name: "Daily Catchup Rule".into(),
        description: String::new(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Daily,
            interval_sec: 0,
            daily_time: Some("00:00:00".into()),
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![Action::Notification {
            title: "Daily Catchup".into(),
            message: "Run".into(),
        }],
        last_executed_at: None,
        ..Default::default()
    });

    let mut engine = DaemonEngine::new(config);
    engine.tick_schedule().await;
    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Daily Catchup Rule")), "Rule should catch up when started after scheduled daily time");

    // Set last_executed_at to today so it won't re-run
    let today = chrono::Local::now().format("%Y-%m-%d 10:00:00").to_string();
    engine.config.rules[0].last_executed_at = Some(today);
    engine.logs.clear();

    engine.tick_schedule().await;
    assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Daily Catchup Rule")), "Rule should not re-run if already executed today");
}

#[tokio::test]
async fn test_pipeline_check_step_halts_subsequent_actions() {
    let mut config = AppConfig::default();
    let rule = Rule {
        id: "check_halt_rule".into(),
        name: "Check Halt Rule".into(),
        description: "".into(),
        is_enabled: true,
        cooldown_seconds: 0.0,
        trigger: None,
        actions: vec![
            Action::Check {
                condition: RuleCondition::FileWatcher {
                    path: "C:\\definitely_non_existent_file_xyz_123.tmp".into(),
                    filter_extension: None,
                    must_exist: true,
                },
            },
            Action::Notification {
                title: "ShouldNotRun".into(),
                message: "Never".into(),
            },
        ],
        ..Default::default()
    };
    config.rules.push(rule.clone());

    let mut engine = DaemonEngine::new(config);
    let h = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    if let Some(handle) = h {
        let _ = handle.await;
    }
    engine.drain_action_events();

    assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("gracefully halted")), "Check failure must gracefully halt pipeline");
    assert!(!engine.logs.iter().any(|l| l.message.contains("ShouldNotRun")), "Subsequent actions must be skipped");
}

#[test]
fn test_runtime_counter_flush_is_coalesced() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "runtime_rule".into(),
        name: "Runtime Rule".into(),
        ..Default::default()
    });
    let mut engine = DaemonEngine::new(config);
    engine.config.rules[0].execution_count = 3;

    engine.mark_runtime_dirty();
    engine.persist_runtime_if_due();
    assert!(
        engine.quota.dirty,
        "the first flush must wait for the coalescing window"
    );

    engine.quota.last_save = Instant::now() - DaemonEngine::RUNTIME_SAVE_INTERVAL - std::time::Duration::from_millis(1);
    engine.persist_runtime_if_due();
    assert!(!engine.quota.dirty, "a flushed runtime must be clean again");
    assert_eq!(engine.config.runtime_state().rules["runtime_rule"].execution_count, 3);

    // A forced flush (quota pause, reset, shutdown) ignores the window.
    engine.mark_runtime_dirty();
    engine.persist_runtime_now();
    assert!(!engine.quota.dirty);
}

#[tokio::test]
async fn test_sync_rules_replaces_set_preserves_counters_and_reloads_once() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "keep".into(),
        name: "Keep".into(),
        execution_count: 4,
        last_executed_at: Some("2026-01-01 00:00:00".into()),
        ..Default::default()
    });
    config.rules.push(Rule {
        id: "drop".into(),
        name: "Drop".into(),
        ..Default::default()
    });
    let mut engine = DaemonEngine::new(config);

    let synced = vec![
        Rule {
            id: "keep".into(),
            name: "Keep renamed".into(),
            ..Default::default()
        },
        Rule {
            id: "new".into(),
            name: "New".into(),
            // Sync files define rules, not execution history: these must be dropped.
            execution_count: 9,
            last_executed_at: Some("2026-02-02 00:00:00".into()),
            quota_window_start: Some("2026-02-02 00:00:00".into()),
            ..Default::default()
        },
    ];
    let response = engine.handle_request(IpcRequest::SyncRules(synced)).await;
    assert!(matches!(response, IpcResponse::Ok));

    let ids: Vec<&str> = engine.config.rules.iter().map(|rule| rule.id.as_str()).collect();
    assert_eq!(ids, vec!["keep", "new"]);
    assert_eq!(engine.config.rules[0].name, "Keep renamed");
    assert_eq!(engine.config.rules[0].execution_count, 4);
    assert_eq!(
        engine.config.rules[0].last_executed_at.as_deref(),
        Some("2026-01-01 00:00:00")
    );
    assert_eq!(engine.config.rules[1].execution_count, 0);
    assert!(engine.config.rules[1].last_executed_at.is_none());
    assert!(engine.config.rules[1].quota_window_start.is_none());
}

#[tokio::test]
async fn test_import_rules_starts_new_rules_with_a_clean_runtime() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    let imported = vec![Rule {
        id: "imported".into(),
        name: "Imported".into(),
        trigger: Some(Trigger::Manual),
        // Counters in a rule file are not execution history.
        execution_count: 7,
        last_executed_at: Some("2026-03-03 00:00:00".into()),
        quota_window_start: Some("2026-03-03 00:00:00".into()),
        ..Default::default()
    }];

    let response = engine.handle_request(IpcRequest::ImportRules(imported)).await;
    assert!(matches!(response, IpcResponse::Ok));
    let rule = &engine.config.rules[0];
    assert_eq!(rule.id, "imported");
    assert_eq!(rule.execution_count, 0);
    assert!(rule.last_executed_at.is_none());
    assert!(rule.quota_window_start.is_none());
}

#[tokio::test]
async fn test_import_share_bundles_deps_disables_rules_and_skips_duplicates() {
    let mut config = AppConfig::default();
    let existing = Rule {
        id: "keep".into(),
        name: "Keep".into(),
        trigger: Some(Trigger::Manual),
        ..Default::default()
    };
    config.rules.push(existing.clone());
    let mut engine = DaemonEngine::new(config);

    let mut overlay = crate::models::OverlayDefinition::default_new_overlay();
    overlay.id = "share_ov".into();
    overlay.name = "Shared overlay".into();
    let preset = crate::models::MacroPreset {
        id: "share_preset".into(),
        name: "Shared preset".into(),
        events: vec![crate::models::MacroEvent::KeyDown { key: "A".into() }],
        repeat: 1,
        speed: 1.0,
        infinite: false,
    };
    let mut duplicate = existing.clone();
    duplicate.id = "renamed_duplicate".into();
    let fresh = Rule {
        id: "fresh".into(),
        name: "Fresh".into(),
        trigger: Some(Trigger::Manual),
        actions: vec![
            Action::Macro {
                preset_id: Some("share_preset".into()),
                operation: crate::models::MacroOperation::Run,
                events: vec![],
                repeat: 1,
                speed: 1.0,
                infinite: false,
            },
            Action::Overlay {
                overlay_id: "share_ov".into(),
                custom_overlay: None,
                operation: crate::models::OverlayOperation::Show,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            },
        ],
        ..Default::default()
    };
    let payload = crate::sharing::SharePayload {
        version: crate::sharing::SHARE_VERSION,
        app_version: "test".into(),
        created_at: String::new(),
        rules: vec![duplicate, fresh],
        overlays: vec![overlay],
        macro_presets: vec![preset],
    };

    let response = engine
        .handle_request(IpcRequest::ImportShare {
            payload: Box::new(payload.clone()),
            enable: false,
        })
        .await;
    match response {
        IpcResponse::ShareImportResult(summary) => {
            assert_eq!(summary.imported, 1);
            assert_eq!(summary.skipped_duplicates, 1);
            assert_eq!(summary.overlays, 1);
            assert_eq!(summary.macro_presets, 1);
            assert!(!summary.enabled);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let imported = engine
        .config
        .rules
        .iter()
        .find(|rule| rule.id == "fresh")
        .expect("fresh rule must be imported");
    assert!(!imported.is_enabled, "share imports are disabled by default");
    assert_eq!(imported.execution_count, 0);
    assert!(engine.config.overlays.iter().any(|ov| ov.id == "share_ov"));
    assert!(engine
        .config
        .macro_presets
        .iter()
        .any(|preset| preset.id == "share_preset"));

    // Importing the same code again imports nothing and skips both rules as duplicates.
    let response = engine
        .handle_request(IpcRequest::ImportShare {
            payload: Box::new(payload),
            enable: false,
        })
        .await;
    match response {
        IpcResponse::ShareImportResult(summary) => {
            assert_eq!(summary.imported, 0);
            assert_eq!(summary.skipped_duplicates, 2);
            assert_eq!(summary.overlays, 0);
            assert_eq!(summary.macro_presets, 0);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn test_periodic_quota_resets_without_pausing_the_rule() {
    let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "periodic_quota".into(),
        name: "Periodic quota".into(),
        max_executions: Some(2),
        quota_period: Some(crate::models::QuotaPeriod::Day),
        execution_count: 2,
        last_executed_at: Some(yesterday),
        trigger: Some(Trigger::Manual),
        actions: vec![Action::Notification {
            title: "Tick".into(),
            message: "Tick".into(),
        }],
        ..Default::default()
    });
    let mut engine = DaemonEngine::new(config);

    // Yesterday's spent daily counter does not block today's first run.
    let first = engine.trigger(&engine.config.rules[0].clone(), ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(first.is_some());
    assert_eq!(engine.config.rules[0].execution_count, 1);
    engine.drain_action_events();

    let second = engine.trigger(&engine.config.rules[0].clone(), ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(second.is_some());
    assert_eq!(engine.config.rules[0].execution_count, 2);
    assert!(engine.config.rules[0].is_enabled, "a periodic quota must not pause the rule");
    assert!(engine
        .logs
        .iter()
        .any(|log| log.message.contains("reached its quota (2/2)")));
    engine.drain_action_events();

    // Once the period is spent the rule stays enabled but is skipped.
    let blocked = engine.trigger(&engine.config.rules[0].clone(), ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(blocked.is_none());
    assert!(engine.config.rules[0].is_enabled);
    assert_eq!(engine.config.rules[0].execution_count, 2);

    // A user-invoked run reports the exhausted quota instead of failing silently.
    let emulated = engine.emulate_rule_trigger(&engine.config.rules[0].clone()).await;
    assert!(emulated.is_none());
}

#[tokio::test]
async fn test_lifetime_quota_still_pauses_the_rule() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "lifetime_quota".into(),
        name: "Lifetime quota".into(),
        max_executions: Some(1),
        execution_count: 1,
        last_executed_at: Some("2020-01-01 00:00:00".into()),
        trigger: Some(Trigger::Manual),
        actions: vec![Action::Notification {
            title: "Tick".into(),
            message: "Tick".into(),
        }],
        ..Default::default()
    });
    let mut engine = DaemonEngine::new(config);

    let blocked = engine.trigger(&engine.config.rules[0].clone(), ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(blocked.is_none());
    assert!(!engine.config.rules[0].is_enabled);
    assert_eq!(engine.config.rules[0].execution_count, 1);
    assert!(engine
        .logs
        .iter()
        .any(|log| log.message.contains("completed quota (1/1)")));
}

#[tokio::test]
async fn test_save_rule_resets_counters_only_when_quota_is_redefined() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "quota_edit".into(),
        name: "Quota edit".into(),
        max_executions: Some(5),
        execution_count: 4,
        last_executed_at: Some("2026-11-19 10:00:00".into()),
        quota_window_start: Some("2026-11-19 09:00:00".into()),
        ..Default::default()
    });
    let mut engine = DaemonEngine::new(config);

    // Unrelated edits keep the daemon-owned counters.
    let mut draft = engine.config.rules[0].clone();
    draft.name = "Renamed".into();
    engine.handle_request(IpcRequest::SaveRule(draft)).await;
    assert_eq!(engine.config.rules[0].execution_count, 4);
    assert!(engine.config.rules[0].quota_window_start.is_some());

    // Changing the quota period starts a fresh counting window.
    let mut draft = engine.config.rules[0].clone();
    draft.quota_period = Some(crate::models::QuotaPeriod::Day);
    engine.handle_request(IpcRequest::SaveRule(draft)).await;
    assert_eq!(engine.config.rules[0].execution_count, 0);
    assert!(engine.config.rules[0].quota_window_start.is_none());
    assert!(
        engine.config.rules[0].last_executed_at.is_some(),
        "schedule deduplication relies on the last-run stamp"
    );
}

// -----------------------------------------------------------------------
// Rule-to-rule control (`Action::RuleControl`)
// -----------------------------------------------------------------------

fn callable_rule(id: &str, name: &str, actions: Vec<Action>) -> Rule {
    Rule {
        id: id.into(),
        name: name.into(),
        trigger: None,
        actions,
        ..Default::default()
    }
}

fn timer_wait(seconds: f64) -> Action {
    Action::Wait {
        mode: crate::models::WaitMode::Timer,
        seconds,
        condition: None,
        timeout_sec: 30.0,
        timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
    }
}

fn started_receiver(start: RuleCallStart) -> oneshot::Receiver<()> {
    match start {
        RuleCallStart::Started(completion) => completion,
        RuleCallStart::Skipped => panic!("the rule should have started"),
    }
}

fn execution_of(engine: &DaemonEngine, rule_id: &str) -> u64 {
    engine
        .executions
        .active_executions
        .iter()
        .find(|(_, execution)| execution.rule_id == rule_id)
        .map(|(id, _)| *id)
        .expect("the rule should have an active execution")
}

#[tokio::test]
async fn test_rule_call_starts_pipeline_and_signals_completion() {
    let mut config = AppConfig::default();
    config.rules.push(callable_rule("target", "Target", vec![timer_wait(0.1)]));
    let mut engine = DaemonEngine::new(config);

    let completion = started_receiver(
        engine.start_called_rule("target", 0).await.expect("call accepted"),
    );
    engine.drain_action_events();

    assert!(engine.is_rule_executing("target"), "the called pipeline is tracked as active");
    assert_eq!(
        engine.config.rules[0].execution_count, 1,
        "a rule call consumes the target's execution quota"
    );
    assert!(
        engine.logs.iter().any(|log| log.message.contains("triggered by Rule Call")),
        "the call is logged as a trigger"
    );

    completion.await.expect("completion signal");
    engine.drain_action_events();
    assert!(!engine.is_rule_executing("target"));
}

#[tokio::test]
async fn test_rule_call_exposes_the_direct_caller() {
    let mut config = AppConfig::default();
    config.rules.push(callable_rule("caller", "Caller", Vec::new()));
    config.rules.push(callable_rule("target", "Target", vec![timer_wait(0.05)]));
    let mut engine = DaemonEngine::new(config);

    // The caller's chain is empty (it was triggered, not called): the direct caller must
    // still reach the callee's context.
    engine.executions.record_started(
        7,
        ActiveExecutionItem {
            rule_id: "caller".into(),
            step_index: 0,
            total_steps: 1,
            is_exit: false,
            started_at: Instant::now(),
            call_chain: Vec::new(),
        },
    );
    let _completion = started_receiver(engine.start_called_rule("target", 7).await.unwrap());

    assert!(
        engine
            .logs
            .iter()
            .any(|log| log.message.contains("triggered by Rule Call (from 'Caller')")),
        "the callee context names the calling rule: {:?}",
        engine.logs.iter().map(|log| log.message.as_str()).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_rule_call_rejects_cycles_and_self_calls() {
    let mut config = AppConfig::default();
    config.rules.push(callable_rule("a", "A", vec![timer_wait(5.0)]));
    config.rules.push(callable_rule("b", "B", vec![timer_wait(5.0)]));
    let mut engine = DaemonEngine::new(config);

    let _completion = started_receiver(engine.start_called_rule("a", 0).await.unwrap());
    engine.drain_action_events();
    let a_execution = execution_of(&engine, "a");

    let err = engine.start_called_rule("a", a_execution).await.unwrap_err();
    assert!(err.contains("calls itself"), "{err}");

    // A pipeline whose chain already contains the target is rejected before spawning.
    engine.executions.record_started(
        999,
        ActiveExecutionItem {
            rule_id: "a".into(),
            step_index: 0,
            total_steps: 1,
            is_exit: false,
            started_at: Instant::now(),
            call_chain: vec!["b".into(), "a".into()],
        },
    );
    let err = engine.start_called_rule("b", 999).await.unwrap_err();
    assert!(err.contains("cycle"), "{err}");

    engine.stop_rule_by_call("a").unwrap();
}

#[tokio::test]
async fn test_rule_call_skips_disabled_targets_and_depth_overflow() {
    let mut config = AppConfig::default();
    config.rules.push(callable_rule("deep", "Deep", vec![timer_wait(5.0)]));
    let mut disabled = callable_rule("off", "Off", vec![timer_wait(5.0)]);
    disabled.is_enabled = false;
    config.rules.push(disabled);
    let mut engine = DaemonEngine::new(config);

    assert!(matches!(
        engine.start_called_rule("off", 0).await.unwrap(),
        RuleCallStart::Skipped
    ));

    engine.executions.record_started(
        1000,
        ActiveExecutionItem {
            rule_id: "chain".into(),
            step_index: 0,
            total_steps: 1,
            is_exit: false,
            started_at: Instant::now(),
            call_chain: (0..rule_call::MAX_RULE_CALL_DEPTH)
                .map(|index| format!("r{index}"))
                .collect(),
        },
    );
    let err = engine.start_called_rule("deep", 1000).await.unwrap_err();
    assert!(err.contains("depth limit"), "{err}");
}

#[tokio::test]
async fn test_rule_call_respects_cooldown_and_conditions() {
    let mut config = AppConfig::default();
    let mut cooled = callable_rule("cool", "Cool", vec![timer_wait(0.05)]);
    cooled.cooldown_seconds = 60.0;
    config.rules.push(cooled);
    let mut conditional = callable_rule("cond", "Cond", vec![timer_wait(0.05)]);
    conditional.conditions = vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
        process_name: "synector-not-a-real-process.exe".into(),
        match_mode: ProcessMatchMode::Exact,
        must_be_running: true,
    })];
    config.rules.push(conditional);
    let mut engine = DaemonEngine::new(config);

    let _first = started_receiver(engine.start_called_rule("cool", 0).await.unwrap());
    assert!(
        matches!(
            engine.start_called_rule("cool", 0).await.unwrap(),
            RuleCallStart::Skipped
        ),
        "the second call inside the cooldown window is skipped"
    );
    assert!(engine.logs.iter().any(|log| log.message.contains("cooldown is still active")));

    assert!(matches!(
        engine.start_called_rule("cond", 0).await.unwrap(),
        RuleCallStart::Skipped
    ));
    assert!(engine.logs.iter().any(|log| log.message.contains("conditions are not met")));
}

#[tokio::test]
async fn test_rule_stop_aborts_without_disabling() {
    let mut config = AppConfig::default();
    config.rules.push(callable_rule("victim", "Victim", vec![timer_wait(30.0)]));
    let mut engine = DaemonEngine::new(config);

    let completion = started_receiver(engine.start_called_rule("victim", 0).await.unwrap());
    engine.drain_action_events();
    assert!(engine.is_rule_executing("victim"));

    engine.stop_rule_by_call("victim").expect("stop accepted");
    completion.await.expect("a stopped pipeline still reports completion");
    assert!(!engine.is_rule_executing("victim"));
    assert!(
        engine.config.rules[0].is_enabled,
        "a rule-to-rule stop must not disable the target"
    );
    assert!(engine.logs.iter().any(|log| log.message.contains("stopped by a rule call")));

    // Stopping an unknown rule fails loudly instead of pretending success.
    assert!(engine.stop_rule_by_call("missing").is_err());
}

#[tokio::test]
async fn test_rule_call_reports_missing_target() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    let err = engine.start_called_rule("missing", 0).await.unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    let err = engine.start_called_rule("   ", 0).await.unwrap_err();
    assert!(err.contains("empty rule_id"), "{err}");
}

// -----------------------------------------------------------------------
// Regression tests for the engine audit findings (U08 / U11 / U14)
// -----------------------------------------------------------------------

fn notify_rule(id: &str, name: &str) -> Rule {
    Rule {
        id: id.into(),
        name: name.into(),
        is_enabled: true,
        actions: vec![Action::Notification {
            title: name.into(),
            message: "tick".into(),
        }],
        ..Default::default()
    }
}

/// Serializes tests that inspect the global overlay-manager map: the shutdown handler hides
/// every overlay, which would otherwise race another test's `show_overlay`.
static OVERLAY_STATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn lock_overlay_state() -> tokio::sync::MutexGuard<'static, ()> {
    OVERLAY_STATE_LOCK.lock().await
}

#[tokio::test]
async fn test_daily_schedule_with_cooldown_still_fires() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        cooldown_seconds: 3600.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Daily,
            interval_sec: 0,
            daily_time: Some("00:00:00".into()),
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        ..notify_rule("daily_cooldown_rule", "Daily Cooldown Rule")
    });
    let mut engine = DaemonEngine::new(config);

    engine.tick_schedule().await;
    assert!(
        engine.logs.iter().any(|log| log.message.contains("triggered by Daily Schedule")),
        "a daily rule with a cooldown must fire, not pre-record its trigger: {:?}",
        engine.logs.iter().map(|log| log.message.as_str()).collect::<Vec<_>>()
    );
    assert!(engine.rule_last_triggered.contains_key("daily_cooldown_rule"));

    // The accepted run recorded `last_executed_at`, so the same day does not fire twice.
    engine.logs.clear();
    engine.tick_schedule().await;
    assert!(!engine.logs.iter().any(|log| log.message.contains("triggered by Daily Schedule")));
}

#[tokio::test]
async fn test_interval_schedule_with_cooldown_fires_after_the_first_interval() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        cooldown_seconds: 60.0,
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 1,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        ..notify_rule("interval_cooldown_rule", "Interval Cooldown Rule")
    });
    let mut engine = DaemonEngine::new(config);

    // The first tick starts the interval clock without running.
    engine.tick_schedule().await;
    assert!(!engine.logs.iter().any(|log| log.message.contains("triggered by Schedule Interval")));

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    engine.tick_schedule().await;
    assert!(
        engine.logs.iter().any(|log| log.message.contains("triggered by Schedule Interval 1s")),
        "the interval run must not be rejected by the trigger stamp it just wrote: {:?}",
        engine.logs.iter().map(|log| log.message.as_str()).collect::<Vec<_>>()
    );
}

fn test_process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .expect("the test executable has a file name")
}

#[tokio::test]
async fn test_resource_threshold_cpu_only_fires() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        trigger: Some(Trigger::ResourceThreshold {
            process_name: test_process_name(),
            cpu_percent: Some(0.0),
            ram_mb: None,
            duration_sec: 0,
        }),
        ..notify_rule("cpu_only_rule", "CPU Only Rule")
    });
    let mut engine = DaemonEngine::new(config);

    engine.tick_resource_threshold().await;
    assert!(
        engine.rule_last_triggered.contains_key("cpu_only_rule"),
        "a CPU-only resource rule must fire"
    );
}

#[tokio::test]
async fn test_resource_threshold_requires_every_configured_metric() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        trigger: Some(Trigger::ResourceThreshold {
            process_name: test_process_name(),
            cpu_percent: Some(0.0),
            ram_mb: Some(u64::MAX),
            duration_sec: 0,
        }),
        ..notify_rule("both_metrics_rule", "Both Metrics Rule")
    });
    let mut engine = DaemonEngine::new(config);

    engine.tick_resource_threshold().await;
    assert!(
        !engine.rule_last_triggered.contains_key("both_metrics_rule"),
        "a rule with both limits must not fire on CPU alone"
    );
}

#[tokio::test]
async fn test_emulate_rule_trigger_aborts_without_disabling() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        actions: vec![timer_wait(5.0)],
        ..notify_rule("emulated_rule", "Emulated Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let rule = engine.config.rules[0].clone();

    let handle = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(handle.is_some(), "the rule should be running before the emulation");
    let _ = engine.emulate_rule_trigger(&rule).await;
    engine.drain_action_events();

    assert!(
        engine.config.rules[0].is_enabled,
        "a GUI test/run must abort the previous execution, not pause the rule"
    );
    engine.abort_rule_executions("emulated_rule");
}

/// U08-002: all three spawn paths draw execution ids from one process-wide counter, so
/// concurrent executions can never share an id. Separate counters would collide (each
/// starts low) and a later allocation would not be greater than every issued id.
#[tokio::test]
async fn test_execution_ids_are_unique_across_the_three_spawn_paths() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        actions: vec![timer_wait(30.0)],
        ..notify_rule("triggered_id_rule", "Triggered ID Rule")
    });
    config.rules.push(Rule {
        actions: vec![Action::Notification {
            title: "exit".into(),
            message: "x".into(),
        }],
        on_exit_actions: vec![timer_wait(30.0)],
        ..notify_rule("exit_id_rule", "Exit ID Rule")
    });
    config.rules.push(Rule {
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 60,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![timer_wait(30.0)],
        ..notify_rule("recurring_id_rule", "Recurring ID Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let triggered_rule = engine.config.rules[0].clone();
    let exit_rule = engine.config.rules[1].clone();
    let recurring_rule = engine.config.rules[2].clone();

    let triggered_handle = engine
        .trigger(&triggered_rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new())
        .await
        .expect("the triggered spawn path must start");
    let exit_handle = engine
        .try_execute_exit_actions(&exit_rule, "MANUAL_TEST", 0)
        .await
        .expect("the exit spawn path must start");
    let recurring_handle = engine
        .emulate_rule_trigger(&recurring_rule)
        .await
        .expect("the recurring spawn path must start");
    engine.drain_action_events();

    let ids = [
        execution_of(&engine, "triggered_id_rule"),
        execution_of(&engine, "exit_id_rule"),
        execution_of(&engine, "recurring_id_rule"),
    ];
    let mut unique = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        3,
        "the three spawn paths must not collide on an execution id: {ids:?}"
    );
    let observed_max = *ids.iter().max().expect("three ids were collected");
    assert!(
        next_execution_id() > observed_max,
        "a later allocation must be greater than every issued id; separate counters would \
         hand out low numbers again ({ids:?})"
    );

    engine.abort_rule_executions("triggered_id_rule");
    engine.abort_rule_executions("exit_id_rule");
    engine.abort_rule_executions("recurring_id_rule");
    let _ = triggered_handle.await;
    let _ = exit_handle.await;
    let _ = recurring_handle.await;
}

/// U08-003: the daemon's hold-mode hotkey loop routes `HotkeyEvent::HoldStop` through
/// `abort_rule_executions` (`src/daemon.rs`), so releasing the hotkey ends the run without
/// pausing the rule. The explicit user stop is the path that persists a pause; the
/// contrast pins the difference.
#[tokio::test]
async fn test_holdstop_abort_keeps_the_rule_enabled() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        actions: vec![timer_wait(30.0)],
        ..notify_rule("holdstop_rule", "HoldStop Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let rule = engine.config.rules[0].clone();

    let handle = engine
        .trigger(&rule, ExecutionOrigin::Hotkey, "Global Hotkey (Hold)", 0, HashMap::new())
        .await
        .expect("the held rule must start");
    engine.drain_action_events();
    assert!(engine.is_rule_executing("holdstop_rule"));

    engine.abort_rule_executions("holdstop_rule");
    engine.drain_action_events();
    assert!(
        engine.config.rules[0].is_enabled,
        "releasing a hold hotkey must abort the run, not pause the rule"
    );
    assert!(
        !engine.is_rule_executing("holdstop_rule"),
        "the held execution must be aborted"
    );
    assert!(handle.await.is_err(), "the aborted task must be cancelled");

    // The user-facing Stop remains the pause: it disables and persists the rule.
    let handle = engine
        .trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new())
        .await
        .expect("the rule is still enabled after the HoldStop abort");
    engine.stop_rule_execution("holdstop_rule");
    assert!(
        !engine.config.rules[0].is_enabled,
        "only the user-facing stop persists a pause"
    );
    assert!(handle.await.is_err(), "the stopped task must be cancelled");
}

#[tokio::test]
async fn test_shutdown_daemon_defers_to_the_coordinated_shutdown() {
    let _overlay_guard = lock_overlay_state().await;
    let mut engine = DaemonEngine::new(AppConfig::default());
    let response = engine.handle_request(IpcRequest::ShutdownDaemon).await;
    assert!(
        matches!(response, IpcResponse::Ok),
        "the handler must answer before the daemon exits"
    );
    // The engine is still alive: the exit and the final flush belong to the daemon main
    // thread, which the handler only asks to shut down.
    assert!(matches!(engine.handle_request(IpcRequest::Ping).await, IpcResponse::Pong));
}

#[test]
fn test_test_step_error_is_redacted_in_the_file_log() {
    crate::logging::init();
    let mut engine = DaemonEngine::new(AppConfig::default());
    let secret = "SYNECTOR_TEST_STEP_SECRET_OUTPUT";
    let response = engine.record_test_step(Err(format!("script failed: {secret}")));

    assert!(matches!(response, IpcResponse::TestStepResult(Err(_))));
    assert!(
        engine.logs.iter().any(|log| log.category == "TEST" && log.message.contains(secret)),
        "the in-app log keeps the actionable error"
    );
    let contents = std::fs::read_to_string(crate::logging::current_log_path()).unwrap_or_default();
    assert!(
        !contents.contains(secret),
        "script output must not reach the daily file log"
    );
    assert!(
        contents.contains("Step test failed (details omitted from the file log)"),
        "the file log keeps a redacted record of the failure"
    );
}

/// U08-011: the engine must never run a `TestStep` action itself. Only
/// `daemon::ipc_server::dispatch` does, before it takes the engine lock, so a wait or a
/// script cannot stall the scheduler. The inline arm that executed the action under the
/// lock is gone; this pins the refusal.
#[tokio::test]
async fn test_handle_request_does_not_execute_test_step_under_the_lock() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        engine.handle_request(IpcRequest::TestStep {
            action: timer_wait(30.0),
            context: HashMap::new(),
        }),
    )
    .await
    .expect("handle_request must not run the step test under the engine lock");
    match response {
        IpcResponse::Error(message) => assert!(
            message.contains("dispatched"),
            "the refusal must point at the pre-lock dispatcher: {message}"
        ),
        other => panic!("the engine must refuse to execute a TestStep directly: {other:?}"),
    }
    assert!(
        !engine.logs.iter().any(|log| log.category == "TEST"),
        "no step-test outcome may be recorded: the action never ran"
    );
}

#[tokio::test]
async fn test_rule_writes_report_a_failed_persist() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    // The daemon's write path is switched to fail; the response is what the GUI and CLI read,
    // so it must not claim success.
    engine.forced_persist_failure = true;
    let rule = notify_rule("unpersistable_rule", "Unpersistable Rule");

    let response = engine.handle_request(IpcRequest::SaveRule(rule.clone())).await;
    assert!(matches!(response, IpcResponse::Error(_)), "SaveRule must not answer Ok when the write failed");

    let response = engine
        .handle_request(IpcRequest::ToggleRule { id: rule.id.clone(), enabled: false })
        .await;
    assert!(matches!(response, IpcResponse::Error(_)), "ToggleRule must not answer Ok when the write failed");

    let response = engine.handle_request(IpcRequest::SyncRules(vec![rule.clone()])).await;
    assert!(matches!(response, IpcResponse::Error(_)), "SyncRules must not answer Ok when the write failed");

    let response = engine.handle_request(IpcRequest::DeleteRule(rule.id)).await;
    assert!(matches!(response, IpcResponse::Error(_)), "DeleteRule must not answer Ok when the write failed");
}

#[tokio::test]
async fn test_webhook_run_is_not_treated_as_manual() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        trigger: Some(Trigger::Webhook { endpoint_path: "/hook".into() }),
        ..notify_rule("webhook_run_rule", "Webhook Run Rule")
    });
    let mut engine = DaemonEngine::new(config);
    engine.executions.add_bound_process(BoundProcess {
        rule_id: "webhook_run_rule".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 4242,
        target_executable: "synector-test-nonexistent-satellite.exe".into(),
    });

    let rule = engine.config.rules[0].clone();
    let handle = engine.trigger(&rule, ExecutionOrigin::Webhook, "Webhook (/hook)", 0, HashMap::new()).await;
    assert!(handle.is_some());
    assert_eq!(
        engine.executions.bound_processes().len(),
        1,
        "a webhook run has no pid but is not a manual run: it must keep companion bindings"
    );
    engine.abort_rule_executions("webhook_run_rule");
}

#[tokio::test]
async fn test_cooldown_survives_a_restart_from_last_executed_at() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        cooldown_seconds: 3600.0,
        last_executed_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        ..notify_rule("persisted_cooldown_rule", "Persisted Cooldown Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let mut rule = engine.config.rules[0].clone();
    assert!(!engine.rule_last_triggered.contains_key(&rule.id), "the memory map starts empty");

    let blocked = engine.trigger(&rule, ExecutionOrigin::Schedule, "Schedule Interval 60s", 0, HashMap::new()).await;
    assert!(blocked.is_none(), "the persisted last run must still gate the cooldown after a restart");

    // User-invoked runs bypass the cooldown.
    let manual = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await;
    assert!(manual.is_some());
    if let Some(handle) = manual {
        let _ = handle.await;
    }
    engine.drain_action_events();

    // An old persisted timestamp no longer blocks.
    rule.last_executed_at = Some("2020-01-01 00:00:00".into());
    engine.rule_last_triggered.remove(&rule.id);
    let started = engine.trigger(&rule, ExecutionOrigin::Schedule, "Schedule Interval 60s", 0, HashMap::new()).await;
    assert!(started.is_some(), "a stale last run must not block forever");
    engine.abort_rule_executions(&rule.id);
}

#[tokio::test]
async fn test_sync_rules_aborts_executions_of_removed_rules() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        actions: vec![timer_wait(5.0)],
        ..notify_rule("vanishing_rule", "Vanishing Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let rule = engine.config.rules[0].clone();
    let handle = engine.trigger(&rule, ExecutionOrigin::Test, "MANUAL_TEST", 0, HashMap::new()).await.expect("rule started");
    engine.drain_action_events();
    assert!(engine.is_rule_executing("vanishing_rule"));
    engine.executions.add_bound_process(BoundProcess {
        rule_id: "vanishing_rule".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 1,
        target_executable: "synector-test-nonexistent-satellite.exe".into(),
    });

    let response = engine.handle_request(IpcRequest::SyncRules(Vec::new())).await;
    assert!(matches!(response, IpcResponse::Ok));

    assert!(
        engine.executions.active_executions.is_empty(),
        "the removed rule's pipeline must be aborted by the declarative sync"
    );
    assert!(
        engine.executions.bound_processes().is_empty(),
        "its bound satellites must be torn down with the rule"
    );
    assert!(handle.await.is_err(), "the aborted task must be cancelled");
}

#[tokio::test]
async fn test_recurring_execution_stops_when_its_rule_disappears() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 1,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        ..notify_rule("recurring_live_rule", "Recurring Live Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let rule = engine.config.rules[0].clone();
    let handle = engine.emulate_rule_trigger(&rule).await.expect("recurring run");
    engine.drain_action_events();
    assert!(engine.executions.has_recurring_test("recurring_live_rule"));

    // Remove the rule from the live set the recurring loop re-reads every cycle.
    engine.config.rules.clear();
    engine.refresh_live_rules();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    engine.drain_action_events();

    assert!(
        !engine.executions.has_recurring_test("recurring_live_rule"),
        "a recurring loop must stop once its rule no longer exists"
    );
    let _ = handle.await;
}

#[tokio::test]
async fn test_oneshot_schedule_stays_armed_when_the_run_is_rejected() {
    let past = chrono::Local::now() - chrono::Duration::seconds(5);
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::OneShot,
            interval_sec: 0,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: Some(past.format("%Y-%m-%d %H:%M:%S").to_string()),
        }),
        conditions: vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
            process_name: "synector-test-absent-process.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        })],
        ..notify_rule("conditional_oneshot_rule", "Conditional OneShot Rule")
    });
    let mut engine = DaemonEngine::new(config);

    engine.tick_schedule().await;
    assert!(
        engine.config.rules[0].is_enabled,
        "a one-shot whose conditions were not met must stay armed"
    );

    engine.config.rules[0].conditions.clear();
    engine.tick_schedule().await;
    assert!(
        !engine.config.rules[0].is_enabled,
        "a one-shot disables itself only after a run actually started"
    );
}

// -----------------------------------------------------------------------
// Offline `--run-rule` parity (U17-008)
// -----------------------------------------------------------------------

fn lua_action(script: &str) -> Action {
    Action::ExecuteScript {
        interpreter: crate::models::ScriptType::Lua,
        script: script.to_string(),
        run_in_window: false,
    }
}

#[tokio::test]
async fn test_offline_runner_produces_the_same_step_tokens_as_the_engine_pipeline() {
    let rule = Rule {
        id: "token_parity".into(),
        name: "Token parity".into(),
        trigger: Some(Trigger::Manual),
        actions: vec![
            lua_action("return 'alpha'"),
            lua_action("return '{PrevOutput}-beta'"),
            lua_action("return '{Step1.Output}/{Step2.Output}'"),
        ],
        stop_on_error: true,
        ..Default::default()
    };

    let offline = run_rule_offline(&rule, RuleRunPolicy::MANUAL).await;
    assert!(offline.skipped.is_none(), "unexpected skip: {:?}", offline.skipped);
    assert!(offline.failures.is_empty(), "no step may fail: {:?}", offline.failures);
    assert!(!offline.halted);
    assert_eq!(offline.context.get("Step1.Output").map(String::as_str), Some("alpha"));
    assert_eq!(offline.context.get("Step2.Output").map(String::as_str), Some("alpha-beta"));
    assert_eq!(
        offline.context.get("Step3.Output").map(String::as_str),
        Some("alpha/alpha-beta")
    );
    assert_eq!(
        offline.context.get("PrevOutput").map(String::as_str),
        Some("alpha/alpha-beta")
    );

    // The daemon pipeline runs the same loop over the same standard context.
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut engine_ctx = DaemonEngine::pipeline_context("Manual Run", 0, HashMap::new());
    let run = PipelineRun {
        actions: &rule.actions,
        execution_id: 42,
        rule_id: &rule.id,
        rule_name: &rule.name,
        context_name: "Manual Run",
        pid: 0,
        is_exit: false,
        stop_on_error: true,
        bind_companions: true,
        label: "Pipeline",
        error_label: "Action",
        warn_on_error_halt: true,
        finished: None,
    };
    let halted = run_action_pipeline(run, &mut engine_ctx, &tx).await;
    assert!(!halted);
    for key in [
        "PrevOutput",
        "Step1.Output",
        "Step2.Output",
        "Step3.Output",
        "ProcessName",
        "ProcessId",
    ] {
        assert_eq!(
            offline.context.get(key),
            engine_ctx.get(key),
            "token {key} diverged between the offline runner and the engine"
        );
    }
}

#[tokio::test]
async fn test_offline_runner_matches_daemon_quota_and_disabled_behaviour() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "offline_quota".into(),
        name: "Offline quota".into(),
        // A spent lifetime quota and a disabled flag: both tiers must treat the manual run
        // the same way (quota refuses, `is_enabled` does not gate a manual run).
        max_executions: Some(1),
        execution_count: 1,
        is_enabled: false,
        trigger: Some(Trigger::Manual),
        actions: vec![lua_action("return 'ran'")],
        ..Default::default()
    });
    let rule = config.rules[0].clone();

    let offline = run_rule_offline(&rule, RuleRunPolicy::MANUAL).await;
    assert!(
        offline
            .skipped
            .as_deref()
            .is_some_and(|reason| reason.contains("completed quota (1/1)")),
        "the offline run must refuse the spent quota: {:?}",
        offline.skipped
    );
    assert!(offline.context.is_empty());

    let mut engine = DaemonEngine::new(config);
    assert!(
        engine.emulate_rule_trigger(&rule).await.is_none(),
        "the daemon manual run must refuse the spent quota too"
    );
    assert!(engine
        .logs
        .iter()
        .any(|log| log.message.contains("completed quota (1/1)")));

    // With the quota gate disabled the pipeline still runs through the same loop.
    let ungated = run_rule_offline(
        &rule,
        RuleRunPolicy {
            enforce_quota: false,
            enforce_conditions: true,
        },
    )
    .await;
    assert!(ungated.skipped.is_none());
    assert_eq!(
        ungated.context.get("Step1.Output").map(String::as_str),
        Some("ran")
    );

    // A disabled rule without limits runs on both manual paths.
    let disabled = Rule {
        max_executions: None,
        execution_count: 0,
        last_executed_at: None,
        ..rule.clone()
    };
    let offline = run_rule_offline(&disabled, RuleRunPolicy::MANUAL).await;
    assert!(offline.skipped.is_none(), "a manual run may test a disabled rule");
    let mut engine = DaemonEngine::new(AppConfig::default());
    let handle = engine
        .emulate_rule_trigger(&disabled)
        .await
        .expect("the daemon manual run must start a disabled rule");
    let _ = handle.await;

    // Preconditions are enforced identically on both sides.
    let conditional = Rule {
        conditions: vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
            process_name: "synector-test-absent-process.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        })],
        ..disabled
    };
    let offline = run_rule_offline(&conditional, RuleRunPolicy::MANUAL).await;
    assert!(
        offline
            .skipped
            .as_deref()
            .is_some_and(|reason| reason.contains("conditions were not met")),
        "the offline run must honour preconditions: {:?}",
        offline.skipped
    );
    let mut engine = DaemonEngine::new(AppConfig::default());
    assert!(engine.emulate_rule_trigger(&conditional).await.is_none());
}

#[tokio::test]
async fn test_offline_runner_honours_abort_pipeline() {
    let rule = Rule {
        id: "offline_abort".into(),
        name: "Offline abort".into(),
        trigger: Some(Trigger::Manual),
        actions: vec![
            lua_action("return 'before'"),
            Action::Check {
                condition: RuleCondition::ProcessRunning {
                    process_name: "synector-test-absent-process.exe".into(),
                    match_mode: ProcessMatchMode::Exact,
                    must_be_running: true,
                },
            },
            lua_action("return 'after'"),
        ],
        ..Default::default()
    };

    let result = run_rule_offline(&rule, RuleRunPolicy::MANUAL).await;
    assert!(result.halted, "a failed inline check must halt the pipeline");
    assert_eq!(
        result.abort.as_ref().map(|(step, _)| *step),
        Some(2),
        "the abort must be reported at the check step"
    );
    assert_eq!(
        result.context.get("Step1.Output").map(String::as_str),
        Some("before")
    );
    assert!(
        !result.context.contains_key("Step3.Output"),
        "no step after the abort may run"
    );
}

#[tokio::test]
async fn test_offline_runner_stops_on_error_only_when_requested() {
    let actions = vec![lua_action("error('offline boom')"), lua_action("return 'after'")];
    let stop = Rule {
        id: "offline_stop".into(),
        name: "Offline stop".into(),
        actions: actions.clone(),
        stop_on_error: true,
        ..Default::default()
    };

    let result = run_rule_offline(&stop, RuleRunPolicy::MANUAL).await;
    assert!(result.halted);
    assert_eq!(result.failures.len(), 1);
    assert!(!result.context.contains_key("Step2.Output"));

    let continue_on_error = Rule {
        stop_on_error: false,
        ..stop
    };
    let result = run_rule_offline(&continue_on_error, RuleRunPolicy::MANUAL).await;
    assert!(!result.halted, "without stop_on_error the pipeline continues");
    assert_eq!(result.failures.len(), 1);
    assert_eq!(
        result.context.get("Step2.Output").map(String::as_str),
        Some("after")
    );
}

// -----------------------------------------------------------------------
// IPC rule validation (U13-006)
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_save_rule_rejects_invalid_rule_and_keeps_the_stored_set() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        id: "stored".into(),
        name: "Stored".into(),
        actions: vec![Action::Notification {
            title: "keep".into(),
            message: "me".into(),
        }],
        ..Default::default()
    });
    let mut engine = DaemonEngine::new(config);

    // The documented invalid combination: an infinite macro without a global-hotkey trigger.
    let invalid = Rule {
        id: "stored".into(),
        name: "Replaced".into(),
        actions: vec![Action::Macro {
            preset_id: None,
            operation: crate::models::MacroOperation::Run,
            events: Vec::new(),
            repeat: 1,
            speed: 1.0,
            infinite: true,
        }],
        ..Default::default()
    };
    let response = engine
        .handle_request(IpcRequest::SaveRule(invalid))
        .await;
    match response {
        IpcResponse::Error(message) => assert!(
            message.contains("infinite macro"),
            "the validation message must explain the rejection: {message}"
        ),
        other => panic!("an invalid rule must be rejected: {other:?}"),
    }
    assert_eq!(engine.config.rules.len(), 1);
    assert_eq!(engine.config.rules[0].name, "Stored");
    assert!(matches!(
        engine.config.rules[0].actions.as_slice(),
        [Action::Notification { .. }]
    ));
}

#[tokio::test]
async fn test_sync_rules_rejects_invalid_set_and_keeps_the_stored_set() {
    let mut config = AppConfig::default();
    config.rules.push(notify_rule("stored", "Stored"));
    let mut engine = DaemonEngine::new(config);

    let synced = vec![
        Rule {
            id: "duplicate".into(),
            name: "First".into(),
            trigger: Some(Trigger::Manual),
            ..Default::default()
        },
        Rule {
            id: "duplicate".into(),
            name: "Second".into(),
            trigger: Some(Trigger::Manual),
            ..Default::default()
        },
    ];
    let response = engine
        .handle_request(IpcRequest::SyncRules(synced))
        .await;
    match response {
        IpcResponse::Error(message) => assert!(
            message.contains("duplicate rule id"),
            "the validation message must explain the rejection: {message}"
        ),
        other => panic!("an invalid rule set must be rejected: {other:?}"),
    }
    let ids: Vec<&str> = engine.config.rules.iter().map(|rule| rule.id.as_str()).collect();
    assert_eq!(ids, vec!["stored"], "the stored rules must stay untouched");
    assert_eq!(engine.config.rules[0].name, "Stored");
}

fn tracked_execution(rule_id: &str, call_chain: Vec<String>) -> ActiveExecutionItem {
    ActiveExecutionItem {
        rule_id: rule_id.to_string(),
        step_index: 0,
        total_steps: 2,
        is_exit: false,
        started_at: Instant::now(),
        call_chain,
    }
}

#[tokio::test]
async fn test_execution_tracker_keeps_pipelines_and_abort_handles_in_step() {
    let mut tracker = ExecutionTracker::new();
    let first = tokio::spawn(std::future::pending::<()>());
    tracker.record_started(1, tracked_execution("rule_a", vec!["caller".into()]));
    tracker.register_abort_handle(1, "rule_a".into(), first.abort_handle());

    assert!(tracker.is_rule_executing("rule_a"));
    assert_eq!(tracker.call_chain(1), vec!["caller".to_string()]);
    assert!(tracker.is_running_rule(1, "rule_a"));
    assert_eq!(tracker.running_rule_id(1), Some("rule_a"));
    tracker.record_step(1, 1, 2, true);
    let item = tracker.active_executions().next().expect("one tracked execution");
    assert_eq!((item.step_index, item.total_steps, item.is_exit), (1, 2, true));

    // A finished pipeline leaves neither its entry nor its abort handle behind.
    tracker.record_finished(1, "rule_a");
    assert_eq!(tracker.active_executions().count(), 0);
    assert!(!tracker.is_rule_executing("rule_a"));

    // A recurring loop is removed together with the execution it was started as.
    let recurring = tokio::spawn(std::future::pending::<()>());
    tracker.record_started(2, tracked_execution("rule_b", Vec::new()));
    tracker.register_abort_handle(2, "rule_b".into(), recurring.abort_handle());
    tracker.register_recurring_test("rule_b".into(), recurring.abort_handle(), 2);
    assert!(tracker.has_recurring_test("rule_b"));
    assert_eq!(
        tracker.recurring_rule_ids().map(String::as_str).collect::<Vec<_>>(),
        vec!["rule_b"]
    );
    assert!(tracker.abort_recurring_test("rule_b"));
    assert!(!tracker.has_recurring_test("rule_b"));
    assert_eq!(tracker.active_executions().count(), 0);
    assert!(!tracker.abort_recurring_test("rule_b"), "aborting twice must be a no-op");

    // `abort_rule` aborts the live pipelines and drops the rule's stale entries, even when
    // the completion event has not been drained yet.
    let pipeline = tokio::spawn(std::future::pending::<()>());
    tracker.record_started(3, tracked_execution("rule_c", Vec::new()));
    tracker.register_abort_handle(3, "rule_c".into(), pipeline.abort_handle());
    tracker.record_started(4, tracked_execution("rule_c", Vec::new()));
    assert_eq!(tracker.abort_rule("rule_c"), 1);
    assert_eq!(tracker.active_executions().count(), 0);
    assert_eq!(tracker.abort_rule("rule_c"), 0);
}

#[test]
fn test_execution_tracker_releases_bound_companions() {
    let mut tracker = ExecutionTracker::new();
    tracker.add_bound_overlay(BoundOverlay {
        rule_id: "rule_a".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 1,
        overlay_id: "overlay_one".into(),
    });
    tracker.add_bound_overlay(BoundOverlay {
        rule_id: "rule_a".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 2,
        overlay_id: "overlay_two".into(),
    });
    tracker.add_bound_overlay(BoundOverlay {
        rule_id: "rule_b".into(),
        primary_process_name: "other.exe".into(),
        primary_pid: 1,
        overlay_id: "overlay_three".into(),
    });
    assert_eq!(tracker.bound_overlays().len(), 3);

    // Both overlays of PID 1 go, the other host's overlay stays.
    let closed = tracker.take_bound_overlays_where(|overlay| overlay.primary_pid == 1);
    assert_eq!(closed.len(), 2);
    assert_eq!(tracker.bound_overlays().len(), 1);
    assert_eq!(tracker.bound_overlays()[0].overlay_id, "overlay_two");

    // Rule teardown removes every remaining overlay of that rule.
    assert_eq!(tracker.take_rule_bound_overlays("rule_a").len(), 1);
    assert!(tracker.bound_overlays().is_empty());

    // A taken identity also drops a same-(rule, overlay) entry that did not match the
    // predicate itself, matching the pre-split filter + retain pair.
    tracker.add_bound_overlay(BoundOverlay {
        rule_id: "rule_c".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 9,
        overlay_id: "shared".into(),
    });
    tracker.add_bound_overlay(BoundOverlay {
        rule_id: "rule_c".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 1,
        overlay_id: "shared".into(),
    });
    assert_eq!(tracker.take_bound_overlays_where(|overlay| overlay.primary_pid == 1).len(), 1);
    assert!(tracker.bound_overlays().is_empty());

    tracker.add_bound_overlay(BoundOverlay {
        rule_id: "rule_d".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 1,
        overlay_id: "kill_me".into(),
    });
    tracker.remove_rule_bound_overlays("rule_d");
    assert!(tracker.bound_overlays().is_empty());
}

#[tokio::test]
async fn test_state_projector_aggregates_status_telemetry_and_logs() {
    let mut config = AppConfig::default();
    config.rules.push(notify_rule("running", "Running"));
    config.rules.push(notify_rule("companion", "Companion"));
    config.rules.push(notify_rule("overlay_rule", "Overlay Rule"));
    let mut engine = DaemonEngine::new(config);
    engine.analytics.record_run(
        crate::models::RunSource::Schedule,
        "running",
        chrono::Local::now(),
    );
    engine.log("INFO", "TEST", "projector probe");
    engine.executions.record_started(1, tracked_execution("running", Vec::new()));
    engine.executions.add_bound_process(BoundProcess {
        rule_id: "companion".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 7,
        target_executable: "satellite.exe".into(),
    });
    engine.executions.add_bound_overlay(BoundOverlay {
        rule_id: "overlay_rule".into(),
        primary_process_name: "game.exe".into(),
        primary_pid: 8,
        overlay_id: "overlay_one".into(),
    });
    engine.begin_update_download();
    engine.set_update_progress(0.25);
    engine.set_update_status("Downloading".to_string());
    engine.request_open_update_modal();

    let state = StateProjector::new(&engine).project();

    assert_eq!(state.total_rules_count, 3);
    assert_eq!(state.active_rules_count, 3);
    // The analytics payload is no longer part of the state snapshot; it is a narrow query.
    assert_eq!(engine.analytics_snapshot().session_executions, 1);
    assert_eq!(engine.analytics_snapshot().top_rules[0].rule_id, "running");
    assert!(state.logs.iter().any(|log| log.message == "projector probe"));
    assert_eq!(state.active_rule_executions["running"].active_count, 1);
    assert_eq!(state.active_rule_executions["running"].total_steps, 2);
    assert!(
        state.active_rule_executions.contains_key("companion"),
        "a bound companion keeps its rule visible as running"
    );
    assert!(
        state.active_rule_executions.contains_key("overlay_rule"),
        "a bound overlay keeps its rule visible as running"
    );
    assert_eq!(state.update_progress, Some(0.25));
    assert_eq!(state.update_status.as_deref(), Some("Downloading"));
    assert!(state.pending_open_update_modal);
    assert_eq!(state.config.rules.len(), 3);
}

#[tokio::test]
async fn test_get_analytics_answers_with_the_collector_snapshot() {
    let mut engine = DaemonEngine::new(AppConfig::default());
    let now = chrono::Local::now();
    engine.analytics.record_run(
        crate::models::RunSource::Manual,
        "rule_x",
        now,
    );
    engine.analytics.record_success(now);

    match engine.handle_request(IpcRequest::GetAnalytics).await {
        IpcResponse::Analytics(snapshot) => {
            assert_eq!(snapshot.session_executions, 1);
            assert_eq!(snapshot.window_executions, 1);
            assert_eq!(snapshot.outcomes.succeeded, 1);
            assert_eq!(snapshot.top_rules.len(), 1);
            assert_eq!(snapshot.top_rules[0].rule_id, "rule_x");
            assert_eq!(snapshot.sources.len(), 1);
        }
        other => panic!("wrong response variant: {other:?}"),
    }

    // The ordinary state snapshot no longer carries analytics: the dialog's narrow query is the
    // only reader of the collector.
    let state_json = serde_json::to_string(&engine.get_current_state()).unwrap();
    assert!(
        !state_json.contains("\"analytics\""),
        "analytics must not ride on every state poll"
    );
}

/// A run stopped before it finishes is reported exactly once as aborted, by its execution guard:
/// the stop path itself no longer counts handles, so a stop and a finished run cannot double count.
#[tokio::test]
async fn test_a_stopped_run_is_counted_once_as_aborted() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        actions: vec![timer_wait(30.0)],
        ..notify_rule("stopped_analytics_rule", "Stopped Analytics Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let rule = engine.config.rules[0].clone();

    let handle = engine
        .trigger(&rule, ExecutionOrigin::Process, "Proc", 0, HashMap::new())
        .await;
    assert!(handle.is_some(), "the run starts");
    engine.drain_action_events();

    engine.abort_rule_executions("stopped_analytics_rule");
    // The aborted task drops its guard on the next scheduler pass and reports through the
    // completion channel.
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    engine.drain_action_events();

    let snapshot = engine.analytics_snapshot();
    assert_eq!(snapshot.window_executions, 1, "the run was counted when it started");
    assert_eq!(snapshot.outcomes.aborted, 1, "a stopped run is exactly one abort");
    assert_eq!(snapshot.outcomes.succeeded, 0);
    assert_eq!(snapshot.outcomes.failed, 0);
}

/// Stopping a recurring test loop while it sleeps between cycles has nothing to abort: no phantom
/// aborted outcome may be recorded for a cycle that never started.
#[tokio::test]
async fn test_stopping_an_idle_recurring_loop_records_no_abort() {
    let mut config = AppConfig::default();
    config.rules.push(Rule {
        trigger: Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 60,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
        actions: vec![timer_wait(0.01)],
        ..notify_rule("recurring_analytics_rule", "Recurring Analytics Rule")
    });
    let mut engine = DaemonEngine::new(config);
    let rule = engine.config.rules[0].clone();

    let handle = engine.emulate_rule_trigger(&rule).await;
    assert!(handle.is_some(), "the recurring loop starts");
    // Wait for the first cycle's outcome; the loop is sleeping until the next interval after it.
    let mut waited_ms = 0;
    while engine.analytics_snapshot().outcomes.total() == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        engine.drain_action_events();
        waited_ms += 20;
        assert!(waited_ms < 5_000, "the first cycle must report its outcome");
    }

    engine.abort_rule_executions("recurring_analytics_rule");
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    engine.drain_action_events();

    let snapshot = engine.analytics_snapshot();
    assert_eq!(snapshot.window_executions, 1, "exactly one cycle ran");
    assert_eq!(snapshot.outcomes.aborted, 0, "an idle loop has nothing to abort");
    assert_eq!(snapshot.outcomes.succeeded, 1, "the finished cycle counts as a success");
    assert_eq!(
        snapshot
            .sources
            .iter()
            .map(|entry| entry.source)
            .collect::<Vec<_>>(),
        vec![crate::models::RunSource::Manual],
        "a run-now emulation is a manual run, whatever trigger the rule carries"
    );
}

#[tokio::test]
async fn test_get_update_progress_answers_narrowly_and_matches_the_state_snapshot() {
    let mut engine = DaemonEngine::new(AppConfig::default());

    let idle = engine.handle_request(IpcRequest::GetUpdateProgress).await;
    assert!(
        matches!(idle, IpcResponse::UpdateProgress { progress: None, status: None }),
        "an idle daemon reports no progress: {idle:?}"
    );

    // A terminal updater status without an active download is still reported.
    engine.set_update_status("Update failed".to_string());
    match engine.handle_request(IpcRequest::GetUpdateProgress).await {
        IpcResponse::UpdateProgress { progress, status } => {
            assert_eq!(progress, None);
            assert_eq!(status.as_deref(), Some("Update failed"));
        }
        other => panic!("wrong response variant: {other:?}"),
    }

    engine.begin_update_download();
    engine.set_update_progress(0.5);
    engine.set_update_status("50%".to_string());
    match engine.handle_request(IpcRequest::GetUpdateProgress).await {
        IpcResponse::UpdateProgress { progress, status } => {
            assert_eq!(progress, Some(0.5));
            assert_eq!(status.as_deref(), Some("50%"));
        }
        other => panic!("wrong response variant: {other:?}"),
    }

    // The full snapshot projects the same pair, so the overlay and the GUI cannot disagree.
    let state = engine.get_current_state();
    assert_eq!(state.update_progress, Some(0.5));
    assert_eq!(state.update_status.as_deref(), Some("50%"));
}
