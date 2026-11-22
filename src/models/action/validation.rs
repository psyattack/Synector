use crate::models::{duration_from_secs, Action, WaitMode};

impl Action {
    /// Value checks for one action, shared by `--rules-apply` and share import. Nested
    /// if/else pipelines are walked by the callers.
    pub fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        match self {
            Action::Wait {
                mode,
                condition,
                seconds,
                timeout_sec,
                ..
            } => {
                if *mode == WaitMode::UntilCondition {
                    match condition {
                        Some(condition) => {
                            errors.extend(condition.validation_errors().into_iter().map(str::to_string));
                        }
                        None => errors.push("an until_condition wait needs a condition".to_string()),
                    }
                }
                if let Err(err) = duration_from_secs(*seconds) {
                    errors.push(format!("wait seconds: {err}"));
                }
                if let Err(err) = duration_from_secs(*timeout_sec) {
                    errors.push(format!("wait timeout_sec: {err}"));
                }
            }
            Action::Check { condition } => {
                errors.extend(condition.validation_errors().into_iter().map(str::to_string));
            }
            _ => {}
        }
        errors
    }
}
