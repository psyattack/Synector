use mlua::{Error as LuaError, HookTriggers, Lua, Value, VmState};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::command::CommandCancellation;

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
/// Limits for the output map filled through `syn.set`.
const MAX_OUTPUT_VARS: usize = 64;
const MAX_OUTPUT_KEY_BYTES: usize = 256;
const MAX_OUTPUT_VALUE_BYTES: usize = 16 * 1024;
/// `syn.run_cmd` deadline and captured output caps. The stdout cap is the shared runner's
/// per-stream cap, so the `[output truncated]` marker keeps its exact meaning.
const RUN_CMD_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CMD_OUTPUT_BYTES: usize = crate::command::MAX_CAPTURED_OUTPUT;
const MAX_CMD_STDERR_BYTES: usize = 2 * 1024;

pub struct LuaRunner;

impl LuaRunner {
    pub fn execute(
        script: &str,
        context: &HashMap<String, String>,
        output_vars: Arc<Mutex<HashMap<String, String>>>,
        cancel: Option<Arc<CommandCancellation>>,
    ) -> Result<String, String> {
        if cancel
            .as_deref()
            .is_some_and(CommandCancellation::is_cancelled)
        {
            return Err("the script was stopped".to_string());
        }
        let lua = Lua::new();
        Self::install_limits(&lua, cancel.clone())?;

        // 1. Create `syn` global table
        let syn_table = lua.create_table().map_err(|e| e.to_string())?;

        // 2. Populate syn.context
        let ctx_table = lua.create_table().map_err(|e| e.to_string())?;
        for (k, v) in context {
            ctx_table.set(k.as_str(), v.as_str()).map_err(|e| e.to_string())?;
        }
        syn_table.set("context", ctx_table).map_err(|e| e.to_string())?;

        // 3. syn.get(key)
        let ctx_clone = context.clone();
        let get_fn = lua.create_function(move |_, key: String| {
            Ok(ctx_clone.get(&key).cloned().unwrap_or_default())
        }).map_err(|e| e.to_string())?;
        syn_table.set("get", get_fn).map_err(|e| e.to_string())?;

        // 4. syn.set(key, value)
        let out_vars_clone = Arc::clone(&output_vars);
        let set_fn = lua.create_function(move |_, (key, val): (String, String)| {
            if key.len() > MAX_OUTPUT_KEY_BYTES {
                return Err(LuaError::runtime(format!(
                    "syn.set: key exceeds the {MAX_OUTPUT_KEY_BYTES} byte limit"
                )));
            }
            if val.len() > MAX_OUTPUT_VALUE_BYTES {
                return Err(LuaError::runtime(format!(
                    "syn.set: value for '{key}' exceeds the {MAX_OUTPUT_VALUE_BYTES} byte limit"
                )));
            }
            if let Ok(mut map) = out_vars_clone.lock() {
                if !map.contains_key(&key) && map.len() >= MAX_OUTPUT_VARS {
                    return Err(LuaError::runtime(format!(
                        "syn.set: more than {MAX_OUTPUT_VARS} output variables"
                    )));
                }
                map.insert(key, val);
            }
            Ok(())
        }).map_err(|e| e.to_string())?;
        syn_table.set("set", set_fn).map_err(|e| e.to_string())?;

        // 5. syn.log(msg)
        let log_fn = lua.create_function(|_, msg: String| {
            println!("[LUA] {}", msg);
            Ok(())
        }).map_err(|e| e.to_string())?;
        syn_table.set("log", log_fn).map_err(|e| e.to_string())?;

        // 6. syn.run_cmd(cmd)
        let cmd_cancel = cancel.clone();
        let cmd_fn = lua.create_function(move |_, cmd: String| {
            Ok(run_shell_command(&cmd, RUN_CMD_TIMEOUT, cmd_cancel.clone()))
        }).map_err(|e| e.to_string())?;
        syn_table.set("run_cmd", cmd_fn).map_err(|e| e.to_string())?;

        // 7. syn.overlay_show(id)
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
        syn_table.set("overlay_show", ov_show).map_err(|e| e.to_string())?;

        // 8. syn.overlay_hide(id)
        let ov_hide = lua.create_function(|_, id: String| {
            let config = crate::config::Library::shared();
            let target_id = config.find_overlay(&id).map(|o| o.id.as_str()).unwrap_or(&id);
            crate::daemon::overlay_manager::hide_overlay(target_id);
            Ok(())
        }).map_err(|e| e.to_string())?;
        syn_table.set("overlay_hide", ov_hide).map_err(|e| e.to_string())?;

        // 9. syn.overlay_toggle(id)
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
        syn_table.set("overlay_toggle", ov_toggle).map_err(|e| e.to_string())?;

        // 10. syn.overlay_set_text(id, text)
        let ov_set_text = lua.create_function(|_, (id, text): (String, String)| {
            crate::daemon::overlay_manager::update_overlay_text(&id, &text);
            Ok(())
        }).map_err(|e| e.to_string())?;
        syn_table.set("overlay_set_text", ov_set_text).map_err(|e| e.to_string())?;

        lua.globals().set("syn", syn_table).map_err(|e| e.to_string())?;

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
    /// for the whole script, including callbacks such as `syn.run_cmd`.
    fn install_limits(
        lua: &Lua,
        cancel: Option<Arc<CommandCancellation>>,
    ) -> Result<(), String> {
        lua.set_memory_limit(MEMORY_LIMIT_BYTES)
            .map_err(|e| format!("Lua memory limit is unavailable: {e}"))?;

        let executed = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let hook_executed = Arc::clone(&executed);
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(INSTRUCTION_HOOK_INTERVAL),
            move |_lua, _debug| {
                if cancel
                    .as_deref()
                    .is_some_and(CommandCancellation::is_cancelled)
                {
                    return Err(LuaError::runtime("the script was stopped"));
                }
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

/// Runs a shell command through the shared bounded runner with a wall-clock deadline, a capped
/// capture of stdout/stderr, the documented `Error: ...` string for every failure mode, and the
/// process-tree kill on timeout or stop. Blocking on purpose: `syn.run_cmd` is synchronous in the
/// Lua VM.
fn run_shell_command(
    cmd: &str,
    timeout: Duration,
    cancel: Option<Arc<CommandCancellation>>,
) -> String {
    let mut command = Command::new("cmd");
    command.args(["/C", cmd]);
    let outcome = match crate::command::run_command_bounded_tracked(
        command,
        timeout,
        cancel.as_deref(),
    ) {
        Ok(outcome) => outcome,
        Err(error) => return format!("Error: {}", error.io_error()),
    };

    if outcome.cancelled {
        return "Error: command stopped".to_string();
    }
    if outcome.timed_out {
        return format!("Error: command timed out after {}s", timeout.as_secs_f32());
    }

    let stdout = capped_string(outcome.stdout_bytes(), MAX_CMD_OUTPUT_BYTES);
    let stderr = capped_string(outcome.stderr_bytes(), MAX_CMD_STDERR_BYTES);
    let stderr = stderr.trim();

    let Some(status) = outcome.status else {
        return "Error: command terminated abnormally".to_string();
    };

    if status.success() {
        if outcome.stdout_truncated {
            return format!("{stdout}\n[output truncated]");
        }
        return stdout;
    }

    let mut message = match status.code() {
        Some(code) => format!("Error: exit code {code}"),
        None => "Error: command terminated abnormally".to_string(),
    };
    if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(stderr);
    }
    message
}

/// Lossy-decodes at most `cap` raw bytes, matching the old hand-rolled reader (the stderr cap is
/// smaller than the shared capture cap, so the byte boundary stays the same).
fn capped_string(bytes: &[u8], cap: usize) -> String {
    let keep = bytes.len().min(cap);
    String::from_utf8_lossy(&bytes[..keep]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(script: &str) -> Result<String, String> {
        let ctx = HashMap::new();
        let outputs = Arc::new(Mutex::new(HashMap::new()));
        LuaRunner::execute(script, &ctx, outputs, None)
    }

    #[test]
    fn test_lua_runner_basic_math_and_globals() {
        let mut ctx = HashMap::new();
        ctx.insert("ProcessName".into(), "game.exe".into());
        let outputs = Arc::new(Mutex::new(HashMap::new()));

        let script = r#"
            local name = syn.get("ProcessName")
            syn.set("Result", "Hello " .. name)
            syn.log("Running in lua for " .. name)
            return 42
        "#;

        let res = LuaRunner::execute(script, &ctx, Arc::clone(&outputs), None)
            .expect("Lua execution should succeed");
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
        let err = run(&format!("syn.set('k', string.rep('v', {}))", MAX_OUTPUT_VALUE_BYTES + 1))
            .expect_err("an oversized output variable must be rejected");
        assert!(err.contains("syn.set"), "unexpected abort reason: {err}");
    }

    #[test]
    fn test_lua_runner_caps_output_variable_count() {
        let err = run(&format!(
            "for i = 1, {} do syn.set('k' .. i, 'v') end",
            MAX_OUTPUT_VARS + 1
        ))
        .expect_err("too many output variables must be rejected");
        assert!(err.contains("syn.set"), "unexpected abort reason: {err}");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_reports_nonzero_exit() {
        let out = run_shell_command("exit /b 3", Duration::from_secs(5), None);
        assert_eq!(out, "Error: exit code 3");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_includes_stderr() {
        let out = run_shell_command("echo boom 1>&2 & exit /b 2", Duration::from_secs(5), None);
        assert!(out.starts_with("Error: exit code 2"), "{out}");
        assert!(out.contains("boom"), "{out}");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_times_out() {
        let started = Instant::now();
        let out = run_shell_command(
            "ping -n 20 127.0.0.1 >nul",
            Duration::from_millis(300),
            None,
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the command must be killed at the deadline"
        );
        assert!(out.starts_with("Error: command timed out"), "{out}");
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_marks_truncated_stdout() {
        let out = run_shell_command(
            "for /L %i in (1,1,7000) do @echo xxxxxxxxxx",
            Duration::from_secs(10),
            None,
        );
        assert!(
            out.ends_with("\n[output truncated]"),
            "stdout over the {MAX_CMD_OUTPUT_BYTES} byte cap must carry the marker"
        );
        assert!(out.len() >= MAX_CMD_OUTPUT_BYTES, "{}", out.len());
    }

    #[cfg(windows)]
    #[test]
    fn test_run_shell_command_caps_error_stderr() {
        let out = run_shell_command(
            "(for /L %i in (1,1,300) do @echo eeeeeeeee 1>&2) & exit /b 1",
            Duration::from_secs(10),
            None,
        );
        assert!(out.starts_with("Error: exit code 1: "), "{out}");
        assert!(
            out.len() < MAX_CMD_STDERR_BYTES + 100,
            "the stderr suffix must keep its {MAX_CMD_STDERR_BYTES} byte cap: {}",
            out.len()
        );
    }

    #[test]
    fn test_lua_runner_run_cmd_exposes_error_string() {
        let res = run("return syn.run_cmd('exit /b 7')").expect("run_cmd itself must not raise");
        assert_eq!(res, "Error: exit code 7");
    }
}
