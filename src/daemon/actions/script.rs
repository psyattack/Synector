use std::collections::HashMap;
use std::process::Command;

use super::support::command::{
    apply_no_window, powershell_command, run_command_bounded_async, SCRIPT_TIMEOUT,
};
use super::ActionRunner;
use crate::models::Action;

/// Context tokens that scripts may see. Keys starting with `__` are engine internals
/// (pipeline execution ids and similar) and are not exported to Lua or PowerShell.
fn script_env(context: &HashMap<String, String>) -> impl Iterator<Item = (&String, &String)> {
    context.iter().filter(|(key, _)| !key.starts_with("__"))
}

pub(super) async fn run_script_internal(
    interpreter: &crate::models::ScriptType,
    script: &str,
    run_in_window: bool,
    context: &HashMap<String, String>,
) -> Result<Option<String>, String> {
    match interpreter {
        crate::models::ScriptType::Lua => {
            let script_code = ActionRunner::interpolate_tokens(script, context);
            let outputs = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
            let lua_ctx: HashMap<String, String> = script_env(context)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            // The VM has its own instruction/time budgets; running it on a blocking
            // thread keeps a long script from occupying a Tokio worker.
            let res = tokio::task::spawn_blocking(move || {
                crate::daemon::lua_runner::LuaRunner::execute(&script_code, &lua_ctx, outputs)
            })
            .await
            .map_err(|e| format!("Lua task failed: {e}"))??;
            Ok(if res.is_empty() { None } else { Some(res) })
        }
        crate::models::ScriptType::PowerShell => {
            let script_code = ActionRunner::interpolate_tokens(script, context);
            if run_in_window {
                let mut cmd = powershell_command(&script_code);
                for (k, v) in script_env(context) {
                    cmd.env(format!("PH_{}", k.to_uppercase()), v);
                }
                let _ = cmd.spawn().map_err(|e| format!("PowerShell spawn error: {e}"))?;
                Ok(None)
            } else {
                let mut cmd = powershell_command(&script_code);
                for (k, v) in script_env(context) {
                    cmd.env(format!("PH_{}", k.to_uppercase()), v);
                }
                let out = run_command_bounded_async(cmd, SCRIPT_TIMEOUT).await?;
                if out.timed_out {
                    return Err(format!(
                        "PowerShell script timed out after {}s",
                        SCRIPT_TIMEOUT.as_secs()
                    ));
                }
                if !out.succeeded() {
                    let message = if out.stderr.is_empty() { &out.stdout } else { &out.stderr };
                    return Err(format!("PowerShell failed: {message}"));
                }
                Ok(if out.stdout.is_empty() { None } else { Some(out.stdout) })
            }
        }
        crate::models::ScriptType::Batch => {
            let cmd_text = ActionRunner::interpolate_tokens(script, context);
            if run_in_window {
                let mut cmd = Command::new("cmd");
                cmd.args(["/C", &cmd_text]);
                let _ = cmd.spawn().map_err(|e| format!("Cmd spawn error: {e}"))?;
                Ok(None)
            } else {
                let mut cmd = Command::new("cmd");
                cmd.args(["/C", &cmd_text]);
                apply_no_window(&mut cmd);
                let out = run_command_bounded_async(cmd, SCRIPT_TIMEOUT).await?;
                if out.timed_out {
                    return Err(format!(
                        "Cmd script timed out after {}s",
                        SCRIPT_TIMEOUT.as_secs()
                    ));
                }
                if !out.succeeded() {
                    let message = if out.stderr.is_empty() { &out.stdout } else { &out.stderr };
                    return Err(format!("Cmd failed: {message}"));
                }
                Ok(if out.stdout.is_empty() { None } else { Some(out.stdout) })
            }
        }
    }
}

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::ExecuteScript { interpreter, script, run_in_window } => {
            run_script_internal(interpreter, script, *run_in_window, context).await?;
            Ok(())
        }
        _ => unreachable!("script handler received a non-script action"),
    }
}
