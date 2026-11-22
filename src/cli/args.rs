//! Argument vocabulary and invocation parsing for the rule-authoring CLI.
//!
//! One command flag selects exactly one command, modifier flags refine it, and two commands in one
//! invocation are a usage error. `command_spellings` exposes the canonical spellings so the CLI
//! parser and the process-wide argv table in `app::args` cannot drift apart.

use crate::arg_spellings as flags;

pub const USAGE: &str = r#"Synector - rule authoring from the command line

  --rules-validate <file|->   Parse and validate a JSON rule file; no changes are made
  --rules-apply <file|->      Create or update rules from a JSON rule file (upsert by id)
  --rules-list [--json]       List configured rules and their triggers
  --rules-export [<file>]     Print all rules as JSON, optionally to a file
  --rules-share [<id>]        Print a compact SYN1: share code for one rule or all rules
  --rules-import-share <code|file|->  Import automation(s) from a SYN1: share code
  --rules-delete <id>         Delete one rule by id
  --rules-sync <file|->       Make the configured rules match the file exactly (upsert and delete missing)
  --rules-enable <id>         Enable one rule
  --rules-disable <id>        Disable one rule
  --rules-reset <id>          Reset a rule's execution counter and re-enable it
  --rules-example             Print an example rule file
  --rules-schema              Print the draft-07 JSON Schema for one rule object
  --rules-reference           Print a human-readable reference of every trigger, action and condition
  --rules-template <t[:a]>    Print a ready-to-edit rule skeleton for a trigger and action
  --json                      Machine-readable output for validate/apply/sync/list/export/
                              share/import-share/delete/enable/disable/reset
  --dry-run                   With --rules-apply or --rules-sync: report changes without writing
  --enable-imported           With --rules-import-share: enable the imported rules (default: disabled)
  --help                      Print this help

A rule file is one rule object, an array of rules, or {"rules": [...]}. Every rule
needs an "id" (the upsert key) and a "name". Rules are applied to the running daemon
over IPC; without a daemon the same upsert is written to rules.json and takes effect
on the next daemon start. Runtime counters (execution_count, last_executed_at) live in
runtime.json and are preserved when an existing rule is updated.

Share codes are compact SYN1: strings that also carry the macro presets and overlays
the rules reference. Imported automations are disabled by default; pass
--enable-imported to activate them immediately. Codes are imported from the argument
itself, from a file, or from stdin with "-".

Use --rules-schema for machine-readable discovery (AI agents, IDE, validators) and
--rules-reference for a compact human-readable catalogue.
"#;

/// Flags that modify a command instead of selecting one.
const MODIFIER_FLAGS: &[&str] = &[flags::JSON, flags::DRY_RUN, flags::ENABLE_IMPORTED];

/// One rule-authoring command. The command is resolved before anything runs, so the priority that
/// used to be implicit in the ordered `if` chain is gone: two command flags in one invocation are
/// a usage error instead of a silent preference ([U13-002]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    Help,
    Example,
    Schema,
    Reference,
    Template,
    Validate,
    Apply,
    Sync,
    Delete,
    Enable,
    Disable,
    Reset,
    List,
    Export,
    Share,
    ImportShare,
}

/// How a command consumes the argument after its flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandArgument {
    /// A plain switch: the next argument is not part of the command.
    None,
    /// The value is required. `-` means stdin for the commands that read input.
    Required,
    /// The value is optional; without it the command uses its default selector.
    Optional,
}

impl Command {
    fn from_flag(flag: &str) -> Option<Self> {
        Some(match flag {
            flags::HELP | flags::HELP_SHORT => Self::Help,
            flags::RULES_EXAMPLE => Self::Example,
            flags::RULES_SCHEMA => Self::Schema,
            flags::RULES_REFERENCE => Self::Reference,
            flags::RULES_TEMPLATE => Self::Template,
            flags::RULES_VALIDATE => Self::Validate,
            flags::RULES_APPLY => Self::Apply,
            flags::RULES_SYNC => Self::Sync,
            flags::RULES_DELETE => Self::Delete,
            flags::RULES_ENABLE => Self::Enable,
            flags::RULES_DISABLE => Self::Disable,
            flags::RULES_RESET => Self::Reset,
            flags::RULES_LIST => Self::List,
            flags::RULES_EXPORT => Self::Export,
            flags::RULES_SHARE => Self::Share,
            flags::RULES_IMPORT_SHARE => Self::ImportShare,
            _ => return None,
        })
    }

    fn argument(self) -> CommandArgument {
        match self {
            Self::Help | Self::Example | Self::Schema | Self::Reference | Self::List => {
                CommandArgument::None
            }
            Self::Template | Self::Export | Self::Share => CommandArgument::Optional,
            Self::Validate
            | Self::Apply
            | Self::Sync
            | Self::Delete
            | Self::Enable
            | Self::Disable
            | Self::Reset
            | Self::ImportShare => CommandArgument::Required,
        }
    }

    /// What the value after the flag is, for usage errors ([U13-014]).
    fn value_label(self) -> &'static str {
        match self {
            Self::Template => "a trigger[:action] spec",
            Self::Validate | Self::Apply | Self::Sync => "a file path or -",
            Self::Delete | Self::Enable | Self::Disable | Self::Reset => "a rule id",
            Self::Export => "an output file path",
            Self::Share => "a rule id or name",
            Self::ImportShare => "a share code, file path or -",
            Self::Help | Self::Example | Self::Schema | Self::Reference | Self::List => {
                "no argument"
            }
        }
    }
}

pub(super) fn is_known_flag(arg: &str) -> bool {
    Command::from_flag(arg).is_some() || MODIFIER_FLAGS.contains(&arg)
}

/// A validated invocation: exactly one command and its optional value.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Invocation {
    pub(super) command: Command,
    pub(super) value: Option<String>,
}

/// True when argv contains a `--rules*` spelling that matches no command while no known flag is
/// present. `main` used to fall through to self-elevation and a GUI start for these ([U13-001]).
pub(super) fn has_unresolved_rule_command(args: &[String]) -> bool {
    !args.iter().any(|arg| is_known_flag(arg))
        && args.iter().any(|arg| arg.starts_with("--rules"))
}

/// Records which command was requested. Only the flag and its path/id argument are logged, never
/// rule JSON, script bodies or other content read from the file.
pub(super) fn log_invocation(args: &[String]) -> String {
    let Some(flag) = args.iter().find(|arg| Command::from_flag(arg).is_some()) else {
        return "unknown command".to_string();
    };
    let Some(command) = Command::from_flag(flag) else {
        return "unknown command".to_string();
    };
    let label = if command == Command::ImportShare {
        // Never log the code itself: it contains the user's automation definitions.
        flag.clone()
    } else {
        match command_argument_value(args, flag, command) {
            Some(value) if !value.is_empty() => format!("{flag} {value}"),
            _ => flag.clone(),
        }
    };
    crate::logging::info("CLI", &format!("Invoked {label}"));
    label
}

/// Resolves exactly one command and validates every other argument. Conflicting or repeated
/// commands, a missing value and arguments that are neither that command's value nor a known flag
/// are usage errors ([U13-001], [U13-002]).
pub(super) fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    let mut selected: Option<(Command, &str)> = None;
    for arg in args {
        let Some(command) = Command::from_flag(arg) else {
            continue;
        };
        match selected {
            Some((existing, existing_flag)) if existing == command => {
                return Err(format!(
                    "{existing_flag} and {arg} select the same command; give it once"
                ));
            }
            Some((_, existing_flag)) => {
                return Err(format!("conflicting commands: {existing_flag} and {arg}"));
            }
            None => selected = Some((command, arg.as_str())),
        }
    }
    let Some((command, flag)) = selected else {
        return Err("no command given".to_string());
    };
    let flag_position = args
        .iter()
        .position(|arg| arg == flag)
        .expect("the selected command flag is present");

    let value = match command.argument() {
        CommandArgument::None => None,
        CommandArgument::Required => Some(required_value(args, flag_position, flag, command)?),
        CommandArgument::Optional => command_argument_value(args, flag, command),
    };
    let value_position = value
        .as_ref()
        .filter(|_| command.argument() != CommandArgument::None)
        .map(|_| flag_position + 1);

    // Everything else must be a modifier. A typo like `--dry-runn` used to be ignored, so the
    // command ran for real; rejecting it is the point of [U13-001].
    for (position, argument) in args.iter().enumerate() {
        if position == flag_position || Some(position) == value_position {
            continue;
        }
        if MODIFIER_FLAGS.contains(&argument.as_str()) {
            continue;
        }
        return Err(format!("unexpected argument \"{argument}\""));
    }

    Ok(Invocation { command, value })
}

/// The value attached to `flag`, if the next argument can be a value. `-` is stdin.
fn command_argument_value(args: &[String], flag: &str, command: Command) -> Option<String> {
    if command.argument() == CommandArgument::None {
        return None;
    }
    let position = args.iter().position(|arg| arg == flag)?;
    args.get(position + 1)
        .filter(|value| value.as_str() == "-" || !value.starts_with('-'))
        .cloned()
}

fn required_value(
    args: &[String],
    flag_position: usize,
    flag: &str,
    command: Command,
) -> Result<String, String> {
    let label = command.value_label();
    match args.get(flag_position + 1) {
        Some(value) if value == "-" || !value.starts_with('-') => Ok(value.clone()),
        Some(value) => Err(format!("{flag} requires {label}, not \"{value}\"")),
        None => Err(format!("{flag} requires {label}")),
    }
}

pub(super) fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

/// Test-only: the command spellings and modifier flags this parser accepts.
///
/// `app::args` gates that the process-wide vocabulary table and this parser agree. The canonical
/// long spellings are listed once here; `-h` is accepted as an alias of `--help` but is not
/// advertised in `USAGE`, so it stays out of the returned list and is asserted directly instead.
#[cfg(test)]
pub(crate) fn command_spellings() -> Vec<&'static str> {
    let commands: &[(&'static str, Command)] = &[
        ("--help", Command::Help),
        ("--rules-example", Command::Example),
        ("--rules-schema", Command::Schema),
        ("--rules-reference", Command::Reference),
        ("--rules-template", Command::Template),
        ("--rules-validate", Command::Validate),
        ("--rules-apply", Command::Apply),
        ("--rules-sync", Command::Sync),
        ("--rules-delete", Command::Delete),
        ("--rules-enable", Command::Enable),
        ("--rules-disable", Command::Disable),
        ("--rules-reset", Command::Reset),
        ("--rules-list", Command::List),
        ("--rules-export", Command::Export),
        ("--rules-share", Command::Share),
        ("--rules-import-share", Command::ImportShare),
    ];
    for (spelling, command) in commands {
        assert_eq!(
            Command::from_flag(spelling),
            Some(*command),
            "{spelling} must select {command:?}"
        );
    }
    assert_eq!(Command::from_flag("-h"), Some(Command::Help));
    commands
        .iter()
        .map(|(spelling, _)| *spelling)
        .chain(MODIFIER_FLAGS.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::resolve_invocation;

    fn cli_args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn test_unknown_argument_is_rejected_with_usage_exit_code() {
        // U13-001: `--dry-runn` next to a real command used to be ignored and the apply ran.
        let unknown = cli_args(&["--rules-apply", "rules.json", "--dry-runn"]);
        let message = parse_invocation(&unknown).expect_err("unknown flag");
        assert!(
            message.contains("unexpected argument \"--dry-runn\""),
            "{message}"
        );
        assert_eq!(resolve_invocation(&unknown, false).expect_err("usage exit code"), 2);

        // A misspelled command with no known flag must be claimed by the CLI and fail closed
        // instead of falling through to elevation and a GUI start.
        let misspelled = cli_args(&["--rules-appply", "rules.json"]);
        assert!(has_unresolved_rule_command(&misspelled));
        assert_eq!(
            parse_invocation(&misspelled).expect_err("no command"),
            "no command given"
        );
        assert!(resolve_invocation(&misspelled, false).is_err());

        assert!(!has_unresolved_rule_command(&cli_args(&[
            "--rules-apply",
            "rules.json"
        ])));
        assert!(!has_unresolved_rule_command(&cli_args(&["--json"])));
        assert!(!has_unresolved_rule_command(&cli_args(&[])));
    }

    #[test]
    fn test_conflicting_and_repeated_commands_are_rejected() {
        // U13-002: the ordered if-chain silently preferred the first command it matched.
        let conflicting = cli_args(&["--rules-validate", "a.json", "--rules-apply", "b.json"]);
        let message = parse_invocation(&conflicting).expect_err("conflict");
        assert!(message.contains("conflicting commands"), "{message}");
        assert!(message.contains("--rules-validate"), "{message}");
        assert!(message.contains("--rules-apply"), "{message}");

        let repeated = cli_args(&["--rules-apply", "a.json", "--rules-apply", "b.json"]);
        let message = parse_invocation(&repeated).expect_err("repeated command");
        assert!(message.contains("select the same command"), "{message}");

        assert!(parse_invocation(&cli_args(&["--help", "--rules-list"])).is_err());

        let invocation =
            parse_invocation(&cli_args(&["--rules-apply", "rules.json", "--json", "--dry-run"]))
                .expect("valid invocation");
        assert_eq!(invocation.command, Command::Apply);
        assert_eq!(invocation.value.as_deref(), Some("rules.json"));
    }

    #[test]
    fn test_command_values_are_validated_per_command() {
        // U13-014: the error names the value the flag actually takes.
        let message = parse_invocation(&cli_args(&["--rules-delete"])).expect_err("missing id");
        assert!(
            message.contains("--rules-delete requires a rule id"),
            "{message}"
        );

        let message =
            parse_invocation(&cli_args(&["--rules-delete", "--json"])).expect_err("flag as id");
        assert!(
            message.contains("--rules-delete requires a rule id"),
            "{message}"
        );
        assert!(message.contains("--json"), "{message}");

        let message = parse_invocation(&cli_args(&["--rules-import-share", "--json"]))
            .expect_err("missing share argument");
        assert!(message.contains("a share code, file path or -"), "{message}");

        let invocation =
            parse_invocation(&cli_args(&["--rules-export", "rules.json"])).expect("export path");
        assert_eq!(invocation.value.as_deref(), Some("rules.json"));
        let invocation = parse_invocation(&cli_args(&["--rules-export"])).expect("export stdout");
        assert_eq!(invocation.value, None);
        let invocation =
            parse_invocation(&cli_args(&["--rules-list", "--json"])).expect("list has no value");
        assert_eq!(invocation.command, Command::List);
        assert_eq!(invocation.value, None);
    }
}
