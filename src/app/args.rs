//! Process-argument vocabulary shared by the runtime dispatch and the rule-authoring CLI.
//!
//! [`ARGS`] lists every spelling the process accepts, exactly once, with the surface that claims
//! it and its arity. The parsers stay separate: the CLI parser rejects conflicting commands and
//! treats `-` as stdin, while the runtime validator tolerates positionals and only refuses a
//! known flag as a value. What they no longer do is own a private list of spellings, so a flag
//! cannot exist on one surface and silently be unknown on the other.
//!
//! `--help`/`-h` is the one documented double claim: `cli::run` runs before the runtime dispatch
//! and prints the rule-authoring usage, so the runtime entry (and `RUNTIME_USAGE`'s `[--help]`)
//! stays valid for validation while being unreachable in normal dispatch.

use std::env;

/// The command-line surface that claims a spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    /// Rule-authoring command or modifier, parsed by `cli::parse_invocation`.
    RuleCli,
    /// Runtime dispatch, parsed by [`validate_runtime_arguments`] and [`LaunchContext`].
    Runtime,
    /// Standalone overlay companion, parsed by [`LaunchContext::overlay`].
    Overlay,
}

/// Whether a spelling consumes the next argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arity {
    /// A flag without a value.
    Switch,
    /// A flag that takes the next argument as its value.
    Value,
}

/// The stable identity of one process argument, independent of the spelling that selected it.
///
/// Aliases share one identity (`-d`/`--daemon`/`--service`/`-s`, `-r`/`--run-rule`, `-o`/
/// `--overlay-window`, `-h`/`--help`), so dispatch asks for the argument and never for a
/// particular spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Argument {
    // Runtime dispatch.
    Gui,
    Daemon,
    Status,
    NoElevate,
    SpawnedByGui,
    RunRule,
    OverlayWindow,
    GuiExe,
    Help,
    // Standalone overlay fast path.
    Calibration,
    Duration,
    OverrideText,
    OverlayData,
    // Rule-authoring CLI.
    RulesExample,
    RulesSchema,
    RulesReference,
    RulesTemplate,
    RulesValidate,
    RulesApply,
    RulesSync,
    RulesDelete,
    RulesEnable,
    RulesDisable,
    RulesReset,
    RulesList,
    RulesExport,
    RulesShare,
    RulesImportShare,
    Json,
    DryRun,
    EnableImported,
}

/// One process spelling and who owns it.
pub(crate) struct ArgSpec {
    pub(crate) spelling: &'static str,
    pub(crate) argument: Argument,
    /// Surfaces that accept this spelling. Only the help spellings have two.
    pub(crate) claims: &'static [Surface],
    pub(crate) arity: Arity,
}

/// Every spelling the process accepts, listed once.
pub(crate) const ARGS: &[ArgSpec] = &[
    // Runtime dispatch: daemon/GUI start, elevation, status, direct rule run and the daemon
    // handshake. `--service`/`-s` are the Windows-service spelling of daemon mode.
    ArgSpec {
        spelling: crate::arg_spellings::GUI,
        argument: Argument::Gui,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::DAEMON,
        argument: Argument::Daemon,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::DAEMON_SHORT,
        argument: Argument::Daemon,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::SERVICE,
        argument: Argument::Daemon,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::SERVICE_SHORT,
        argument: Argument::Daemon,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::STATUS,
        argument: Argument::Status,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::NO_ELEVATE,
        argument: Argument::NoElevate,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::SPAWNED_BY_GUI,
        argument: Argument::SpawnedByGui,
        claims: &[Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RUN_RULE,
        argument: Argument::RunRule,
        claims: &[Surface::Runtime],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RUN_RULE_SHORT,
        argument: Argument::RunRule,
        claims: &[Surface::Runtime],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::OVERLAY_WINDOW,
        argument: Argument::OverlayWindow,
        claims: &[Surface::Runtime],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::OVERLAY_WINDOW_SHORT,
        argument: Argument::OverlayWindow,
        claims: &[Surface::Runtime],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::GUI_EXE,
        argument: Argument::GuiExe,
        claims: &[Surface::Runtime],
        arity: Arity::Value,
    },
    // The one documented double claim: the CLI help still wins dispatch.
    ArgSpec {
        spelling: crate::arg_spellings::HELP,
        argument: Argument::Help,
        claims: &[Surface::RuleCli, Surface::Runtime],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::HELP_SHORT,
        argument: Argument::Help,
        claims: &[Surface::RuleCli, Surface::Runtime],
        arity: Arity::Switch,
    },
    // Standalone overlay companions. They are not valid once the fast path has returned, so they
    // are absent from the runtime surface on purpose.
    ArgSpec {
        spelling: crate::arg_spellings::CALIBRATION,
        argument: Argument::Calibration,
        claims: &[Surface::Overlay],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::DURATION,
        argument: Argument::Duration,
        claims: &[Surface::Overlay],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::OVERRIDE_TEXT,
        argument: Argument::OverrideText,
        claims: &[Surface::Overlay],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::OVERLAY_DATA,
        argument: Argument::OverlayData,
        claims: &[Surface::Overlay],
        arity: Arity::Value,
    },
    // Rule-authoring commands.
    ArgSpec {
        spelling: crate::arg_spellings::RULES_EXAMPLE,
        argument: Argument::RulesExample,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_SCHEMA,
        argument: Argument::RulesSchema,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_REFERENCE,
        argument: Argument::RulesReference,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_TEMPLATE,
        argument: Argument::RulesTemplate,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_VALIDATE,
        argument: Argument::RulesValidate,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_APPLY,
        argument: Argument::RulesApply,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_SYNC,
        argument: Argument::RulesSync,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_DELETE,
        argument: Argument::RulesDelete,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_ENABLE,
        argument: Argument::RulesEnable,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_DISABLE,
        argument: Argument::RulesDisable,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_RESET,
        argument: Argument::RulesReset,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_LIST,
        argument: Argument::RulesList,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_EXPORT,
        argument: Argument::RulesExport,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_SHARE,
        argument: Argument::RulesShare,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    ArgSpec {
        spelling: crate::arg_spellings::RULES_IMPORT_SHARE,
        argument: Argument::RulesImportShare,
        claims: &[Surface::RuleCli],
        arity: Arity::Value,
    },
    // Rule-authoring modifiers.
    ArgSpec {
        spelling: crate::arg_spellings::JSON,
        argument: Argument::Json,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::DRY_RUN,
        argument: Argument::DryRun,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
    ArgSpec {
        spelling: crate::arg_spellings::ENABLE_IMPORTED,
        argument: Argument::EnableImported,
        claims: &[Surface::RuleCli],
        arity: Arity::Switch,
    },
];

/// The vocabulary-table entry for a spelling, if the process accepts it at all.
pub(crate) fn argument_spec(spelling: &str) -> Option<&'static ArgSpec> {
    ARGS.iter().find(|spec| spec.spelling == spelling)
}

/// The first argv position holding a spelling of `argument`.
fn find_argument(argv: &[String], argument: Argument) -> Option<usize> {
    argv.iter()
        .position(|token| argument_spec(token).is_some_and(|spec| spec.argument == argument))
}

/// The argument following the first spelling of `argument`, when one is present.
fn argument_value(argv: &[String], argument: Argument) -> Option<&String> {
    let position = find_argument(argv, argument)?;
    argv.get(position + 1)
}

fn has_argument(argv: &[String], argument: Argument) -> bool {
    find_argument(argv, argument).is_some()
}

/// Arguments of one process launch, resolved once.
///
/// `argv` never contains the executable path: `env::args()` is collected once and element 0 is
/// dropped here, so the strict CLI parser and the runtime validator both consume only real
/// arguments. Arguments are re-derived nowhere else in the dispatch.
#[derive(Debug)]
pub(crate) struct LaunchContext {
    argv: Vec<String>,
    no_elevate: bool,
}

impl LaunchContext {
    /// Collects `env::args()` once.
    pub(crate) fn from_env() -> Self {
        Self::from_process_args(env::args().collect())
    }

    /// Builds the context from a full `env::args()`-shaped vector.
    ///
    /// Element 0 is the executable path and is dropped here; every parser below therefore sees
    /// post-`argv[0]` arguments only (the strict CLI parser would reject the path as an
    /// unexpected argument).
    pub(crate) fn from_process_args(args: Vec<String>) -> Self {
        let argv: Vec<String> = args.into_iter().skip(1).collect();
        let no_elevate = has_argument(&argv, Argument::NoElevate);
        Self { argv, no_elevate }
    }

    /// The arguments after the executable path.
    pub(crate) fn argv(&self) -> &[String] {
        &self.argv
    }

    /// The user's `--no-elevate` opt-out, computed once for the whole process tree.
    pub(crate) fn no_elevate(&self) -> bool {
        self.no_elevate
    }

    /// Whether the daemon was requested directly (`--daemon`/`-d`/`--service`/`-s`).
    pub(crate) fn is_daemon(&self) -> bool {
        has_argument(&self.argv, Argument::Daemon)
    }

    /// Whether a spelling of `argument` is present.
    pub(crate) fn contains(&self, argument: Argument) -> bool {
        has_argument(&self.argv, argument)
    }

    /// The standalone overlay invocation, when this process is an overlay window.
    ///
    /// The id defaults to an empty string and the duration is only kept when it parses as `u32`,
    /// which is exactly what the overlay writers in `daemon/overlay_manager.rs` rely on.
    pub(crate) fn overlay(&self) -> Option<OverlayInvocation> {
        let position = find_argument(&self.argv, Argument::OverlayWindow)?;
        Some(OverlayInvocation {
            overlay_id: self.argv.get(position + 1).cloned().unwrap_or_default(),
            calibration: self.contains(Argument::Calibration),
            duration_ms: argument_value(&self.argv, Argument::Duration)
                .and_then(|value| value.parse::<u32>().ok()),
            override_text: argument_value(&self.argv, Argument::OverrideText).cloned(),
            overlay_data: argument_value(&self.argv, Argument::OverlayData).cloned(),
        })
    }
}

/// One standalone overlay window request, parsed from the shared argv.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OverlayInvocation {
    pub(crate) overlay_id: String,
    pub(crate) calibration: bool,
    pub(crate) duration_ms: Option<u32>,
    pub(crate) override_text: Option<String>,
    pub(crate) overlay_data: Option<String>,
}

/// Whether a runtime flag carries a value in the next argument or is a plain switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeFlag {
    /// A flag without a value.
    Switch,
    /// A flag that takes the next argument as its value.
    Value,
}

impl RuntimeFlag {
    /// Classifies one runtime flag spelling. `None` means the argument is not a runtime flag.
    ///
    /// This is the surface that reaches the daemon/GUI dispatch, so it is exactly the `Runtime`
    /// claim in [`ARGS`]. The standalone overlay fast path and the CLI rule commands parse and
    /// consume their own arguments before this point, so overlay-only companions
    /// (`--calibration`, `--duration`, `--override-text`, `--overlay-data`) are deliberately
    /// absent: they are not valid once the fast path has returned.
    fn from_spelling(flag: &str) -> Option<Self> {
        let spec = argument_spec(flag)?;
        if !spec.claims.contains(&Surface::Runtime) {
            return None;
        }
        Some(match spec.arity {
            Arity::Switch => Self::Switch,
            Arity::Value => Self::Value,
        })
    }
}

/// A process argument that cannot be honored once the runtime dispatch is reached.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuntimeArgumentError {
    /// A `-`-prefixed argument matches no runtime flag.
    UnknownFlag(String),
    /// A value-taking flag is followed by nothing, an empty value or another flag.
    MissingValue(String),
}

impl std::fmt::Display for RuntimeArgumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(formatter, "unknown argument \"{flag}\""),
            Self::MissingValue(flag) => write!(formatter, "{flag} requires a non-empty value"),
        }
    }
}

/// Usage line for a malformed runtime invocation; the rule-authoring reference is `--help`.
pub(crate) const RUNTIME_USAGE: &str = "Usage: Synector.exe [--gui | --daemon] [--status] [--run-rule <rule id>] [--no-elevate] [--help]";

/// Validates the process arguments that reach the daemon/GUI dispatch.
///
/// The standalone overlay window and the CLI rule commands consume their own arguments and return
/// before this check, so only the runtime surface may appear here. [`LaunchContext::argv`]
/// already excludes the executable path; every later argument starting with `-` must be a known
/// runtime flag, and a value-taking flag must be followed by a non-empty value that is not itself
/// a known flag. A value such as a rule id may still start with a single `-`, so
/// `--run-rule -custom-id` keeps working. Positional arguments are not flags and are left to the
/// dispatch.
///
/// [U13-001]: without this check a bare unknown flag fell through update-file cleanup, UAC
/// self-elevation and a daemon/GUI start, so a typo launched the elevated GUI instead of failing.
pub(crate) fn validate_runtime_arguments(argv: &[String]) -> Result<(), RuntimeArgumentError> {
    let mut position = 0;
    while let Some(argument) = argv.get(position) {
        if !argument.starts_with('-') {
            position += 1;
            continue;
        }
        match RuntimeFlag::from_spelling(argument) {
            Some(RuntimeFlag::Switch) => position += 1,
            Some(RuntimeFlag::Value) => match argv.get(position + 1) {
                Some(value)
                    if !value.trim().is_empty() && RuntimeFlag::from_spelling(value).is_none() =>
                {
                    position += 2;
                }
                _ => return Err(RuntimeArgumentError::MissingValue(argument.clone())),
            },
            None => return Err(RuntimeArgumentError::UnknownFlag(argument.clone())),
        }
    }
    Ok(())
}

/// Parsed `--run-rule` / `-r` request from the process arguments.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunRuleArgument {
    /// The flag is not present; normal start-up continues.
    Absent,
    /// A non-empty rule id to run.
    RuleId(String),
    /// The flag is present but its value is missing or empty; this is a usage error.
    MissingValue,
}

pub(crate) fn parse_run_rule_argument(argv: &[String]) -> RunRuleArgument {
    let Some(position) = find_argument(argv, Argument::RunRule) else {
        return RunRuleArgument::Absent;
    };
    match argv.get(position + 1) {
        Some(rule_id) if !rule_id.trim().is_empty() => RunRuleArgument::RuleId(rule_id.clone()),
        _ => RunRuleArgument::MissingValue,
    }
}

/// Quotes one argument so that an elevated re-exec (or a shortcut launch) reconstructs it exactly.
///
/// This is the `CommandLineToArgvW`/MSVC CRT convention: backslashes are only special in front of
/// a double quote, so they are doubled before quotes and before the closing quote; embedded quotes
/// are preceded by `2n + 1` backslashes. Empty arguments are preserved as `""`.
pub(crate) fn quote_windows_argument(argument: &str) -> String {
    if argument.is_empty() {
        return "\"\"".to_string();
    }
    if !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }

    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

pub(crate) fn build_elevation_arguments(args: &[String]) -> String {
    args.iter()
        .map(|argument| quote_windows_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain argument vector: `LaunchContext::argv` shape, no executable path.
    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|value| (*value).to_string()).collect()
    }

    /// Builds a context from a full `env::args()`-shaped vector (element 0 is the executable).
    fn launch_context(raw: &[&str]) -> LaunchContext {
        LaunchContext::from_process_args(args(raw))
    }

    /// The runtime parser consumes `LaunchContext::argv`, which has no executable path.
    fn runtime_argv(raw: &[&str]) -> Vec<String> {
        launch_context(raw).argv().to_vec()
    }

    fn argv_strings(argv: &[String]) -> Vec<&str> {
        argv.iter().map(String::as_str).collect()
    }

    #[test]
    fn test_parse_run_rule_argument() {
        assert_eq!(
            parse_run_rule_argument(&runtime_argv(&["Synector.exe"])),
            RunRuleArgument::Absent
        );
        assert_eq!(
            parse_run_rule_argument(&runtime_argv(&["Synector.exe", "--run-rule", "rule_1"])),
            RunRuleArgument::RuleId("rule_1".into())
        );
        assert_eq!(
            parse_run_rule_argument(&runtime_argv(&["Synector.exe", "-r", "rule_2"])),
            RunRuleArgument::RuleId("rule_2".into())
        );
        assert_eq!(
            parse_run_rule_argument(&runtime_argv(&["Synector.exe", "--run-rule"])),
            RunRuleArgument::MissingValue
        );
        assert_eq!(
            parse_run_rule_argument(&runtime_argv(&["Synector.exe", "--run-rule", ""])),
            RunRuleArgument::MissingValue
        );
        assert_eq!(
            parse_run_rule_argument(&runtime_argv(&["Synector.exe", "--run-rule", "   "])),
            RunRuleArgument::MissingValue
        );
    }

    #[test]
    fn test_launch_context_drops_the_executable_path() {
        let launch = launch_context(&["Synector.exe", "--rules-list", "--json"]);
        assert_eq!(argv_strings(launch.argv()), ["--rules-list", "--json"]);
        assert!(launch_context(&["Synector.exe"]).argv().is_empty());
    }

    #[test]
    fn test_production_argv_shape_reaches_both_parsers() {
        // env::args() starts with the executable path, which the strict CLI parser rejects as an
        // unexpected argument. The shared slice is what both parsers receive.
        let cli_launch = launch_context(&[
            "C:\\Program Files\\Synector\\Synector.exe",
            "--rules-list",
            "--json",
        ]);
        assert_eq!(argv_strings(cli_launch.argv()), ["--rules-list", "--json"]);
        assert!(!cli_launch
            .argv()
            .iter()
            .any(|argument| argument.ends_with("Synector.exe")));

        let runtime_launch = launch_context(&[
            "C:\\Program Files\\Synector\\Synector.exe",
            "--gui",
            "--no-elevate",
        ]);
        assert_eq!(
            validate_runtime_arguments(runtime_launch.argv()),
            Ok(())
        );
        assert!(runtime_launch.no_elevate());
        assert!(!runtime_launch.is_daemon());
    }

    #[test]
    fn test_validate_runtime_arguments_accepts_known_runtime_flags() {
        let valid = [
            args(&[]),
            args(&["--gui"]),
            args(&["--daemon"]),
            args(&["-d"]),
            args(&["--service"]),
            args(&["-s"]),
            args(&["--status"]),
            args(&["--no-elevate"]),
            args(&["--spawned-by-gui"]),
            args(&["--help"]),
            args(&["-h"]),
            args(&["--run-rule", "rule_1"]),
            args(&["-r", "rule_1"]),
            // A rule id may legally start with a dash; only a known flag is refused as a value.
            args(&["--run-rule", "-custom-rule-id"]),
            args(&["--overlay-window", "overlay_1"]),
            args(&["-o", "overlay_1"]),
            args(&["--gui-exe", "C:\\Program Files\\Synector\\Synector.exe"]),
            args(&["--gui", "--no-elevate", "--spawned-by-gui"]),
        ];
        for case in valid {
            assert_eq!(
                validate_runtime_arguments(&case),
                Ok(()),
                "{case:?} must be accepted"
            );
        }
    }

    #[test]
    fn test_validate_runtime_arguments_rejects_unknown_flags() {
        assert_eq!(
            validate_runtime_arguments(&args(&["--bogus"])),
            Err(RuntimeArgumentError::UnknownFlag("--bogus".into()))
        );
        assert_eq!(
            validate_runtime_arguments(&args(&["--gui", "--dry-runn"])),
            Err(RuntimeArgumentError::UnknownFlag("--dry-runn".into())),
            "an unknown flag next to a known one must still be rejected"
        );
        assert_eq!(
            validate_runtime_arguments(&args(&["-x"])),
            Err(RuntimeArgumentError::UnknownFlag("-x".into()))
        );
        // Overlay-only companions are not valid once the overlay fast path has returned.
        assert_eq!(
            validate_runtime_arguments(&args(&["--calibration"])),
            Err(RuntimeArgumentError::UnknownFlag("--calibration".into()))
        );
        // CLI modifiers are consumed by `cli::run` before this check; on their own they are not
        // runtime flags either.
        for modifier in ["--json", "--dry-run", "--enable-imported", "--rules-list"] {
            assert_eq!(
                validate_runtime_arguments(&args(&[modifier])),
                Err(RuntimeArgumentError::UnknownFlag(modifier.into())),
                "{modifier} is a rule-authoring spelling, not a runtime flag"
            );
        }
    }

    #[test]
    fn test_validate_runtime_arguments_rejects_missing_values() {
        for flag in ["--run-rule", "-r", "--gui-exe", "--overlay-window", "-o"] {
            assert_eq!(
                validate_runtime_arguments(&args(&[flag])),
                Err(RuntimeArgumentError::MissingValue(flag.into())),
                "{flag} without a value"
            );
            assert_eq!(
                validate_runtime_arguments(&args(&[flag, ""])),
                Err(RuntimeArgumentError::MissingValue(flag.into())),
                "{flag} with an empty value"
            );
        }
        assert_eq!(
            validate_runtime_arguments(&args(&["--run-rule", "   "])),
            Err(RuntimeArgumentError::MissingValue("--run-rule".into()))
        );
        // A following known flag is not a value: `--run-rule --gui` must not try to run a rule
        // literally named `--gui`.
        assert_eq!(
            validate_runtime_arguments(&args(&["--run-rule", "--gui"])),
            Err(RuntimeArgumentError::MissingValue("--run-rule".into()))
        );
        // A following flag is not a value: `--gui-exe --daemon` must not treat `--daemon` as a
        // path and then open the GUI.
        assert_eq!(
            validate_runtime_arguments(&args(&["--gui-exe", "--daemon"])),
            Err(RuntimeArgumentError::MissingValue("--gui-exe".into()))
        );
    }

    #[test]
    fn test_runtime_argument_error_messages() {
        assert_eq!(
            RuntimeArgumentError::UnknownFlag("--bogus".into()).to_string(),
            "unknown argument \"--bogus\""
        );
        assert_eq!(
            RuntimeArgumentError::MissingValue("--run-rule".into()).to_string(),
            "--run-rule requires a non-empty value"
        );
    }

    #[test]
    fn test_launch_context_resolves_daemon_and_elevation_flags() {
        for spelling in ["--daemon", "-d", "--service", "-s"] {
            let launch = launch_context(&["Synector.exe", spelling]);
            assert!(launch.is_daemon(), "{spelling} must select daemon mode");
            assert!(!launch.no_elevate());
            assert!(!launch.contains(Argument::Status));
        }
        let launch = launch_context(&["Synector.exe", "--daemon", "--no-elevate", "--status"]);
        assert!(launch.is_daemon());
        assert!(launch.no_elevate());
        assert!(launch.contains(Argument::Status));
        assert!(!launch.contains(Argument::Gui));
    }

    #[test]
    fn test_overlay_invocation_is_parsed_from_the_same_argv() {
        let launch = launch_context(&[
            "Synector.exe",
            "--overlay-window",
            "overlay_1",
            "--calibration",
            "--duration",
            "1500",
            "--override-text",
            "hello",
            "--overlay-data",
            "payload",
        ]);
        assert_eq!(
            launch.overlay(),
            Some(OverlayInvocation {
                overlay_id: "overlay_1".to_string(),
                calibration: true,
                duration_ms: Some(1500),
                override_text: Some("hello".to_string()),
                overlay_data: Some("payload".to_string()),
            })
        );

        // The short alias is the same argument.
        let launch = launch_context(&["Synector.exe", "-o", "overlay_2"]);
        assert_eq!(
            launch.overlay().map(|overlay| overlay.overlay_id),
            Some("overlay_2".to_string())
        );

        // A missing id defaults to empty, and a non-numeric duration is ignored (OK-only parse).
        let launch = launch_context(&["Synector.exe", "-o"]);
        assert_eq!(
            launch.overlay(),
            Some(OverlayInvocation {
                overlay_id: String::new(),
                calibration: false,
                duration_ms: None,
                override_text: None,
                overlay_data: None,
            })
        );
        let launch = launch_context(&[
            "Synector.exe",
            "--overlay-window",
            "overlay_3",
            "--duration",
            "soon",
        ]);
        assert_eq!(launch.overlay().and_then(|overlay| overlay.duration_ms), None);

        // Anything without the overlay-window spelling stays on the runtime path.
        assert_eq!(launch_context(&["Synector.exe", "--gui"]).overlay(), None);
    }

    #[test]
    fn test_no_spelling_is_claimed_by_two_surfaces() {
        let mut seen = std::collections::HashSet::new();
        for spec in ARGS {
            assert!(
                seen.insert(spec.spelling),
                "{} is listed more than once",
                spec.spelling
            );
            if spec.argument == Argument::Help {
                assert!(spec.claims.contains(&Surface::RuleCli));
                assert!(spec.claims.contains(&Surface::Runtime));
                assert_eq!(
                    spec.claims.len(),
                    2,
                    "{} is the documented double claim and nothing else",
                    spec.spelling
                );
            } else {
                assert_eq!(
                    spec.claims.len(),
                    1,
                    "{} is claimed by more than one surface",
                    spec.spelling
                );
            }
        }
    }

    #[test]
    fn test_runtime_classification_is_driven_by_the_table() {
        for spec in ARGS {
            let expected = if spec.claims.contains(&Surface::Runtime) {
                Some(match spec.arity {
                    Arity::Switch => RuntimeFlag::Switch,
                    Arity::Value => RuntimeFlag::Value,
                })
            } else {
                None
            };
            assert_eq!(
                RuntimeFlag::from_spelling(spec.spelling),
                expected,
                "{} must be classified exactly like its table row",
                spec.spelling
            );
        }
        assert_eq!(RuntimeFlag::from_spelling("--bogus"), None);
        assert_eq!(RuntimeFlag::from_spelling("--calibration"), None);
        assert_eq!(RuntimeFlag::from_spelling("--json"), None);
    }

    /// `RUNTIME_USAGE` is user-visible text and may not change. The internal cross-process flags
    /// and the short aliases are accepted without being advertised, so they are listed here once:
    /// a new runtime spelling must either be advertised or be added deliberately.
    const UNDOCUMENTED_RUNTIME_SPELLINGS: &[&str] = &[
        "-d",
        "-s",
        "-r",
        "-o",
        "-h",
        "--service",
        "--spawned-by-gui",
        "--gui-exe",
        "--overlay-window",
    ];

    /// The spellings `RUNTIME_USAGE` advertises, as whole tokens (`[--gui` → `--gui`).
    fn runtime_usage_spellings() -> Vec<&'static str> {
        RUNTIME_USAGE
            .split_whitespace()
            .map(|token| {
                token.trim_matches(|character| character == '[' || character == ']')
            })
            .filter(|spelling| spelling.starts_with("--"))
            .collect()
    }

    #[test]
    fn test_runtime_usage_matches_the_vocabulary_table() {
        assert_eq!(
            RUNTIME_USAGE,
            "Usage: Synector.exe [--gui | --daemon] [--status] [--run-rule <rule id>] [--no-elevate] [--help]"
        );
        let advertised = runtime_usage_spellings();
        assert_eq!(
            advertised,
            [
                "--gui",
                "--daemon",
                "--status",
                "--run-rule",
                "--no-elevate",
                "--help"
            ]
        );
        for spec in ARGS
            .iter()
            .filter(|spec| spec.claims.contains(&Surface::Runtime))
        {
            assert!(
                advertised.contains(&spec.spelling)
                    || UNDOCUMENTED_RUNTIME_SPELLINGS.contains(&spec.spelling),
                "runtime spelling {} is neither advertised in RUNTIME_USAGE nor marked internal",
                spec.spelling
            );
        }
        // The internal list is real: every entry is an accepted runtime spelling that the frozen
        // usage text does not advertise.
        for spelling in UNDOCUMENTED_RUNTIME_SPELLINGS {
            let spec = argument_spec(spelling)
                .unwrap_or_else(|| panic!("{spelling} is missing from the vocabulary table"));
            assert!(spec.claims.contains(&Surface::Runtime));
            assert!(!advertised.contains(spelling));
        }
        // Every advertised spelling is a real runtime spelling.
        for spelling in advertised {
            assert!(
                argument_spec(spelling)
                    .is_some_and(|spec| spec.claims.contains(&Surface::Runtime)),
                "{spelling} appears in RUNTIME_USAGE but is not a runtime spelling"
            );
        }
    }

    #[test]
    fn test_cli_parser_and_vocabulary_table_agree() {
        let cli_spellings = crate::cli::command_spellings();
        for spelling in &cli_spellings {
            let spec = argument_spec(spelling)
                .unwrap_or_else(|| panic!("{spelling} is missing from the vocabulary table"));
            assert!(
                spec.claims.contains(&Surface::RuleCli),
                "{spelling} must be claimed by the rule-authoring CLI"
            );
            assert!(
                crate::cli::USAGE.contains(spelling),
                "{spelling} must appear in the CLI USAGE text"
            );
        }
        for spec in ARGS
            .iter()
            .filter(|spec| spec.claims.contains(&Surface::RuleCli))
        {
            // `-h` is a short alias of `--help`; the CLI parser accepts it but the frozen USAGE
            // text advertises only the long form.
            if spec.spelling == "-h" {
                continue;
            }
            assert!(
                cli_spellings.contains(&spec.spelling),
                "{} is in the vocabulary table but the CLI parser does not accept it",
                spec.spelling
            );
        }
    }

    /// The daemon tier decides "am I the daemon?" with the same shared spellings the table lists,
    /// so `--service`/`-s` cannot work in one place and be unknown in the other.
    #[test]
    fn test_daemon_mode_spellings_are_the_runtime_daemon_flags() {
        assert_eq!(
            crate::arg_spellings::DAEMON_MODE,
            &[
                crate::arg_spellings::DAEMON,
                crate::arg_spellings::DAEMON_SHORT,
                crate::arg_spellings::SERVICE,
                crate::arg_spellings::SERVICE_SHORT,
            ]
        );
        for spelling in crate::arg_spellings::DAEMON_MODE {
            let spec = argument_spec(spelling).expect("daemon spelling is in the vocabulary table");
            assert_eq!(spec.argument, Argument::Daemon, "{spelling}");
            assert!(spec.claims.contains(&Surface::Runtime), "{spelling}");
            assert_eq!(spec.arity, Arity::Switch, "{spelling}");
        }
    }

    /// The overlay writer and the overlay fast path share the spellings: an argv built from the
    /// shared constants parses back into the invocation the daemon meant to send.
    #[test]
    fn test_overlay_writer_spellings_parse_back_to_an_overlay_invocation() {
        let mut argv = vec!["Synector.exe".to_string()];
        argv.extend(args(&[
            crate::arg_spellings::OVERLAY_WINDOW,
            "overlay_1",
            crate::arg_spellings::CALIBRATION,
            crate::arg_spellings::DURATION,
            "250",
            crate::arg_spellings::OVERRIDE_TEXT,
            "hello",
            crate::arg_spellings::OVERLAY_DATA,
            "payload",
        ]));
        let launch = LaunchContext::from_process_args(argv);
        assert_eq!(
            launch.overlay(),
            Some(OverlayInvocation {
                overlay_id: "overlay_1".to_string(),
                calibration: true,
                duration_ms: Some(250),
                override_text: Some("hello".to_string()),
                overlay_data: Some("payload".to_string()),
            })
        );
    }

    #[test]
    fn test_quote_windows_argument_round_trips_through_command_line_to_argv() {
        let cases = [
            "",
            "plain",
            "with space",
            "tab\tinside",
            "leading and trailing ",
            "trailing\\",
            "C:\\Program Files\\Synector\\",
            "embedded\"quote",
            "quote\" and space",
            "backslashes\\\\before\"quote",
            "'apostrophe'; Start-Process calc",
            "$\"nested\" $(quote)",
        ];
        for case in cases {
            let command_line = quote_windows_argument(case);
            assert_eq!(
                parse_arguments(&command_line),
                vec![case.to_string()],
                "argument {case:?} must round-trip through {command_line:?}"
            );
        }
    }

    #[test]
    fn test_build_elevation_arguments_keeps_empty_arguments() {
        let command_line = build_elevation_arguments(&args(&["--gui", "", "a b"]));
        assert_eq!(command_line, "--gui \"\" \"a b\"");
        assert_eq!(parse_arguments(&command_line), args(&["--gui", "", "a b"]));
    }

    /// Parses the arguments after the program name, which is how ShellExecuteExW and shortcut
    /// launchers deliver our parameters. `CommandLineToArgvW` parses the first token differently,
    /// so the helper must not stand in for argv[0].
    fn parse_arguments(command_line: &str) -> Vec<String> {
        let mut parsed = parse_command_line_to_argv(&format!("Synector.exe {command_line}"));
        if !parsed.is_empty() {
            parsed.remove(0);
        }
        parsed
    }

    /// Parses a command line with the same Win32 parser that reconstructs the elevated argv.
    fn parse_command_line_to_argv(command_line: &str) -> Vec<String> {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::UI::Shell::CommandLineToArgvW;

        let command_line = HSTRING::from(command_line);
        unsafe {
            let mut count = 0i32;
            let pointer = CommandLineToArgvW(PCWSTR(command_line.as_ptr()), &mut count);
            if pointer.is_null() || count <= 0 {
                return Vec::new();
            }
            let arguments = std::slice::from_raw_parts(pointer, count as usize)
                .iter()
                .map(|argument| {
                    let mut length = 0usize;
                    while *argument.0.add(length) != 0 {
                        length += 1;
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(argument.0, length))
                })
                .collect();
            let _ = LocalFree(Some(HLOCAL(pointer as *mut _)));
            arguments
        }
    }
}
