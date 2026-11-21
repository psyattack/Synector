use mlua::{Error as LuaError, HookTriggers, Lua, Value, VmState};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Instruction budget for one script. The hook runs every
/// [`INSTRUCTION_HOOK_INTERVAL`] VM instructions, so an endless loop aborts
/// promptly instead of pinning a daemon worker forever.
const MAX_INSTRUCTIONS: u64 = 5_000_000;
const INSTRUCTION_HOOK_INTERVAL: u32 = 1_000;
/// Wall-clock backstop for work that is not pure VM instructions.
const EXECUTION_TIME_LIMIT: Duration = Duration::from_secs(3);
/// Per-script Lua heap cap; a runaway allocation raises a Lua memory error.
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// Largest value a script may return (it is retained by the pipeline).
const MAX_RETURNED_STRING_BYTES: usize = 64 * 1024;
/// Limits for the output map filled through `ph.set`.
const MAX_OUTPUT_VARS: usize = 64;
const MAX_OUTPUT_KEY_BYTES: usize = 256;
const MAX_OUTPUT_VALUE_BYTES: usize = 16 * 1024;
/// `ph.run_cmd` deadline and captured output cap.
const RUN_CMD_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CMD_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_CMD_STDERR_BYTES: usize = 2 * 1024;

pub struct LuaRunner;

impl LuaRunner {
    pub fn execute(
        script: &str,
        context: &HashMap<String, String>,
        output_vars: Arc<Mutex<HashMap<String, String>>>,
    ) -> Result<String, String> {
        let lua = Lua::new();
        Self::install_limits(&lua)?;

        // 1. Create `ph` global table
        let ph_table = lua.create_table().map_err(|e| e.to_string())?;

        // 2. Populate ph.context
        let ctx_table = lua.create_table().map_err(|e| e.to_string())?;
        for (k, v) in context {
            ctx_table.set(k.as_str(), v.as_str()).map_err(|e| e.to_string())?;
        }
        ph_table.set("context", ctx_table).map_err(|e| e.to_string())?;

        // 3. ph.get(key)
        let ctx_clone = context.clone();
        let get_fn = lua.create_function(move |_, key: String| {
            Ok(ctx_clone.get(&key).cloned().unwrap_or_default())
        }).map_err(|e| e.to_string())?;
        ph_table.set("get", get_fn).map_err(|e| e.to_string())?;

        // 4. ph.set(key, value)
        let out_vars_clone = Arc::clone(&output_vars);
        let set_fn = lua.create_function(move |_, (key, val): (String, String)| {
            if key.len() > MAX_OUTPUT_KEY_BYTES {
                return Err(LuaError::runtime(format!(
                    "ph.set: key exceeds the {MAX_OUTPUT_KEY_BYTES} byte limit"
                )));
            }
            if val.len() > MAX_OUTPUT_VALUE_BYTES {
                return Err(LuaError::runtime(format!(
                    "ph.set: value for '{key}' exceeds the {MAX_OUTPUT_VALUE_BYTES} byte limit"
                )));
            }
            if let Ok(mut map) = out_vars_clone.lock() {
                if !map.contains_key(&key) && map.len() >= MAX_OUTPUT_VARS {
                    return Err(LuaError::runtime(format!(
                        "ph.set: more than {MAX_OUTPUT_VARS} output variables"
                    )));
                }
                map.insert(key, val);
            }
            Ok(())
        }).map_err(|e| e.to_string())?;
        ph_table.set("set", set_fn).map_err(|e| e.to_string())?;

        // 5. ph.log(msg)
        let log_fn = lua.create_function(|_, msg: String| {
            println!("[LUA] {}", msg);
            Ok(())
        }).map_err(|e| e.to_string())?;
        ph_table.set("log", log_fn).map_err(|e| e.to_string())?;

        // 6. ph.run_cmd(cmd)
        let cmd_fn = lua.create_function(|_, cmd: String| {
            Ok(run_shell_command(&cmd, RUN_CMD_TIMEOUT))
        }).map_err(|e| e.to_string())?;
        ph_table.set("run_cmd", cmd_fn).map_err(|e| e.to_string())?;

        // 7. ph.overlay_show(id)
        let ov_show = lua.create_function(|_, id: String| {
            let config = crate::config::Library::shared();
            let overlay = config.find_overlay(&id).cloned().unwrap_or_else(|| match id.as_str() {
                "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                _ => crate::models::OverlayDefinition::default_new_overlay(),
            });
            crate::daemon::overlay_manager::show_overlay(&overlay, false, None);
            Ok(())
        }).map_err(|e| e.to_string())?;
        ph_table.set("overlay_show", ov_show).map_err(|e| e.to_string())?;

        // 8. ph.overlay_hide(id)
        let ov_hide = lua.create_function(|_, id: String| {
            let config = crate::config::Library::shared();
            let target_id = config.find_overlay(&id).map(|o| o.id.as_str()).unwrap_or(&id);
            crate::daemon::overlay_manager::hide_overlay(target_id);
            Ok(())
        }).map_err(|e| e.to_string())?;
        ph_table.set("overlay_hide", ov_hide).map_err(|e| e.to_string())?;

        // 9. ph.overlay_toggle(id)
        let ov_toggle = lua.create_function(|_, id: String| {
            let config = crate::config::Library::shared();
            let overlay = config.find_overlay(&id).cloned().unwrap_or_else(|| match id.as_str() {
                "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
                "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
                "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
                _ => crate::models::OverlayDefinition::default_new_overlay(),
            });
            crate::daemon::overlay_manager::toggle_overlay(&overlay, None);
            Ok(())
        }).map_err(|e| e.to_string())?;
        ph_table.set("overlay_toggle", ov_toggle).map_err(|e| e.to_string())?;

        // 10. ph.overlay_set_text(id, text)
        let ov_set_text = lua.create_function(|_, (id, text): (String, String)| {
            crate::daemon::overlay_manager::update_overlay_text(&id, &text);
            Ok(())
        }).map_err(|e| e.to_string())?;
        ph_table.set("overlay_set_text", ov_set_text).map_err(|e| e.to_string())?;

        lua.globals().set("ph", ph_table).map_err(|e| e.to_string())?;

        // Execute chunk and capture return value if any
        let result: Value = lua
            .load(script)
            .eval()
            .map_err(|e| match e {
                LuaError::MemoryError(_) => format!(
                    "Lua runtime error: script exceeded the {MEMORY_LIMIT_BYTES} byte memory limit"
                ),
                other => format!("Lua runtime error: {other}"),
            })?;
        match result {
            Value::String(s) => {
                let text = s
                    .to_str()
                    .map_err(|e| format!("Lua runtime error: returned string is not valid UTF-8 ({e})"))?
                    .to_string();
                if text.len() > MAX_RETURNED_STRING_BYTES {
                    return Err(format!(
                        "Lua runtime error: returned string exceeds the {MAX_RETURNED_STRING_BYTES} byte limit"
                    ));
                }
                Ok(text)
            }
            Value::Integer(i) => Ok(i.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Boolean(b) => Ok(b.to_string()),
            _ => Ok(String::new()),
        }
    }

    /// Installs the instruction/time hook and the Lua heap cap. They stay active
    /// for the whole script, including callbacks such as `ph.run_cmd`.
    fn install_limits(lua: &Lua) -> Result<(), String> {
        lua.set_memory_limit(MEMORY_LIMIT_BYTES)
            .map_err(|e| format!("Lua memory limit is unavailable: {e}"))?;

        let executed = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let hook_executed = Arc::clone(&executed);
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(INSTRUCTION_HOOK_INTERVAL),
            move |_lua, _debug| {
                let executed = hook_executed
                    .fetch_add(u64::from(INSTRUCTION_HOOK_INTERVAL), Ordering::Relaxed)
                    + u64::from(INSTRUCTION_HOOK_INTERVAL);
                if executed >= MAX_INSTRUCTIONS {
                    return Err(LuaError::runtime(format!(
                        "script exceeded the {MAX_INSTRUCTIONS} instruction budget"
                    )));
                }
                if started.elapsed() > EXECUTION_TIME_LIMIT {
                    return Err(LuaError::runtime(format!(
                        "script exceeded the {} second execution budget",
                        EXECUTION_TIME_LIMIT.as_secs()
                    )));
                }
                Ok(VmState::Continue)
            },
        )
        .map_err(|e| format!("Lua instruction limit is unavailable: {e}"))
    }
}

enum WaitOutcome {
    Exited(ExitStatus),
    TimedOut,
    Failed(std::io::Error),
}

/// Kills `cmd` and any process it started. Killing only the shell would leave a
/// long-running grandchild holding the stdout/stderr pipes open, so the reader
/// threads below would never see EOF.
fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs a shell command on a helper process with a wall-clock deadline, a capped
/// capture of stdout/stderr, and the documented `Error: ...` string for every
/// failure mode. Blocking on purpose: `ph.run_cmd` is synchronous in the Lua VM.
fn run_shell_command(cmd: &str, timeout: Duration) -> String {
    let mut child = match Command::new("cmd")
        .args(["/C", cmd])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => return format!("Error: {err}"),
    };

    let stdout_reader = child
        .stdout
        .take()
        .map(|stream| std::thread::spawn(move || read_capped(stream, MAX_CMD_OUTPUT_BYTES)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stream| std::thread::spawn(move || read_capped(stream, MAX_CMD_STDERR_BYTES)));

    let deadline = Instant::now() + timeout;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break WaitOutcome::Exited(status),
            Ok(None) => {}
            Err(err) => {
                kill_process_tree(&mut child);
                break WaitOutcome::Failed(err);
            }
        }
        if Instant::now() >= deadline {
            kill_process_tree(&mut child);
            break WaitOutcome::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let (stdout, stdout_truncated) = join_reader(stdout_reader);
    let (stderr, _) = join_reader(stderr_reader);

    let status = match outcome {
        WaitOutcome::Exited(status) => status,
        WaitOutcome::TimedOut => {
            return format!("Error: command timed out after {}s", timeout.as_secs_f32());
        }
        WaitOutcome::Failed(err) => return format!("Error: {err}"),
    };

    if status.success() {
        if stdout_truncated {
            return format!("{stdout}\n[output truncated]");
        }
        return stdout;
    }

    let mut message = match status.code() {
        Some(code) => format!("Error: exit code {code}"),
        None => "Error: command terminated abnormally".to_string(),
    };
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(stderr);
    }
    message
}

fn join_reader(
    handle: Option<std::thread::JoinHandle<(String, bool)>>,
) -> (String, bool) {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_else(|| (String::new(), false))
}

fn read_capped(mut stream: impl Read, cap: usize) -> (String, bool) {
    let mut captured = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if captured.len() < cap {
                    let keep = (cap - captured.len()).min(n);
                    captured.extend_from_slice(&buf[..keep]);
                    if keep < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&captured).into_owned(), truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(script: &str) -> Result<String, String> {
        let ctx = HashMap::new();
        let outputs = Arc::new(Mutex::new(HashMap::new()));
        LuaRunner::execute(script, &ctx, outputs)
    }

    #[test]
    fn test_lua_runner_basic_math_and_globals() {
        let mut ctx = HashMap::new();
        ctx.insert("ProcessName".into(), "game.exe".into());
        let outputs = Arc::new(Mutex::new(HashMap::new()));

        let script = r#"
            local name = ph.get("ProcessName")
            ph.set("Result", "Hello " .. name)
            ph.log("Running in lua for " .. name)
            return 42
        "#;

        let res = LuaRunner::execute(script, &ctx, Arc::clone(&outputs)).expect("Lua execution should succeed");
        assert_eq!(res, "42");

        let vars = outputs.lock().unwrap();
        assert_eq!(vars.get("Result").unwrap(), "Hello game.exe");
    }

    #[test]
    fn test_lua_runner_aborts_infinite_loop() {
        let started = Instant::now();
        let err = run("while true do end").expect_err("an endless script must abort");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the instruction budget must abort quickly, took {:?}",
            started.elapsed()
        );
        assert!(
            err.contains("instruction budget") || err.contains("execution budget"),
            "unexpected abort reason: {err}"
        );
    }

    #[test]
    fn test_lua_runner_limits_memory() {
        let err = run("return string.rep('x', 128 * 1024 * 1024)")
            .expect_err("a huge allocation must be rejected");
        assert!(
            err.to_lowercase().contains("memory"),
            "unexpected abort reason: {err}"
        );
    }

    #[test]
    fn test_lua_runner_caps_returned_string() {
        let err = run(&format!("return string.rep('x', {})", MAX_RETURNED_STRING_BYTES + 1))
            .expect_err("an oversized return value must be rejected");
        assert!(
            err.contains(&format!("{MAX_RETURNED_STRING_BYTES} byte limit")),
            "unexpected abort reason: {err}"
        );
    }

    #[test]
    fn test_lua_runner_caps_output_variable_value() {
        let err = run(&format!("ph.set('k', string.rep('v', {}))", MAX_OUTPUT_VALUE_BYTES + 1))
            .expect_err("an oversized output variable must be rejected");
        assert!(err.contains("ph.set"), "unexpected abort reason: {err}");
    }

    #[test]
    fn test_lua_runner_caps_output_variable_count() {
        let err = run(&format!(
            "for i = 1, {} do ph.set('k' .. i, 'v') end",
            MAX_OUTPUT_VARS + 1
        ))
        .expect_err("too many output variables must be rejected");
        assert!(err.contains("ph.set"), "unexpected abort reason: {err}");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_reports_nonzero_exit() {
        let out = run_shell_command("exit /b 3", Duration::from_secs(5));
        assert_eq!(out, "Error: exit code 3");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_includes_stderr() {
        let out = run_shell_command("echo boom 1>&2 & exit /b 2", Duration::from_secs(5));
        assert!(out.starts_with("Error: exit code 2"), "{out}");
        assert!(out.contains("boom"), "{out}");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_times_out() {
        let started = Instant::now();
        let out = run_shell_command("ping -n 20 127.0.0.1 >nul", Duration::from_millis(300));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the command must be killed at the deadline"
        );
        assert!(out.starts_with("Error: command timed out"), "{out}");
    }

    #[test]
    fn test_lua_runner_run_cmd_exposes_error_string() {
        let res = run("return ph.run_cmd('exit /b 7')").expect("run_cmd itself must not raise");
        assert_eq!(res, "Error: exit code 7");
    }
}
