//! Single home for the Windows command-line quoting rules and the no-window spawn flag.
//!
//! The process entry (`app::args`, rule shortcuts, elevation), the updater's PowerShell scripts
//! and the daemon action helpers all build Windows command lines and spawn helper processes with
//! the same conventions. The copies had already drifted apart in shape, so one quoting rule lives
//! here and every caller delegates: a fix reaches every launch path instead of only the copy that
//! happened to be edited.

use std::process::Command;

/// Splits a `start_process` argument string with the Windows command-line rules
/// (`CommandLineToArgvW`): whitespace separates arguments, double quotes group and
/// escape, and backslashes before a quote follow the documented pairing rules.
pub(crate) fn split_windows_arguments(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut started = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' => {
                if in_quotes {
                    current.push(chars[i]);
                } else if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
                i += 1;
            }
            '"' => {
                started = true;
                if in_quotes && chars.get(i + 1) == Some(&'"') {
                    // A pair of quotes inside a quoted argument is one literal quote.
                    current.push('"');
                    i += 2;
                } else {
                    in_quotes = !in_quotes;
                    i += 1;
                }
            }
            '\\' => {
                let start = i;
                while i < chars.len() && chars[i] == '\\' {
                    i += 1;
                }
                let count = i - start;
                if chars.get(i) == Some(&'"') {
                    current.push_str(&"\\".repeat(count / 2));
                    started = true;
                    if count % 2 == 1 {
                        // The odd backslash escapes the quote into the argument.
                        current.push('"');
                        i += 1;
                    }
                    // An even count leaves the quote for the delimiter branch above.
                } else {
                    current.push_str(&"\\".repeat(count));
                    started = true;
                }
            }
            ch => {
                current.push(ch);
                started = true;
                i += 1;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
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

/// Joins `arguments` into one command-line string, quoting each element.
pub(crate) fn join_windows_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escapes a value for a PowerShell single-quoted literal: doubling the only special
/// character keeps arbitrary user content as data instead of code.
pub(crate) fn ps_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

/// Applies the `CREATE_NO_WINDOW` creation flag to `cmd`; a no-op off Windows.
pub(crate) fn apply_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}
