//! Syntax highlighter for PowerShell (.ps1), Windows Command Prompt (.cmd / .bat) and
//! Lua scripts. Produces formatted `egui::text::LayoutJob` with token styling matched
//! to studio tokens.

use egui::{Color32, FontId, TextFormat};
use egui::text::LayoutJob;
use crate::gui::theme::{self, Tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptLanguage {
    PowerShell,
    Batch,
    Lua,
}

impl ScriptLanguage {
    /// Interpreter badge for the modal header. The names stay Latin in every catalog (they are
    /// product/interpreter names), but the label still comes from the caller's `Strings`.
    pub fn badge_name(&self, loc: &crate::i18n::Strings) -> &'static str {
        match self {
            Self::PowerShell => loc.script_badge_powershell,
            Self::Batch => loc.script_badge_batch,
            Self::Lua => loc.script_badge_lua,
        }
    }
}

const PWSH_KEYWORDS: &[&str] = &[
    "begin", "break", "catch", "class", "continue", "data", "do", "dynamicparam",
    "else", "elseif", "end", "exit", "filter", "finally", "for", "foreach", "from",
    "function", "if", "in", "inlinescript", "parallel", "param", "process", "return",
    "sequence", "switch", "throw", "trap", "try", "until", "using", "var", "while",
    "workflow",
];

const PWSH_OPERATORS: &[&str] = &[
    "-eq", "-ne", "-gt", "-ge", "-lt", "-le", "-like", "-notlike",
    "-match", "-notmatch", "-replace", "-split", "-contains", "-notcontains",
    "-in", "-notin", "-is", "-isnot", "-and", "-or", "-xor", "-not", "-band",
    "-bor", "-bxor", "-bnot", "-shl", "-shr",
];

const PWSH_CMDLETS: &[&str] = &[
    "write-host", "write-output", "write-warning", "write-error", "write-verbose", "write-debug",
    "get-process", "start-process", "stop-process", "wait-process",
    "get-service", "start-service", "stop-service", "restart-service",
    "get-item", "set-item", "new-item", "remove-item", "copy-item", "move-item", "rename-item",
    "get-childitem", "test-path", "resolve-path", "split-path", "join-path",
    "get-content", "set-content", "add-content", "clear-content",
    "select-object", "where-object", "foreach-object", "sort-object", "measure-object",
    "out-file", "out-null", "out-string", "export-csv", "import-csv",
    "invoke-webrequest", "invoke-restmethod", "invoke-expression", "invoke-command",
    "start-sleep", "get-date", "set-location", "get-location", "push-location", "pop-location",
    "get-command", "get-help", "get-member", "get-variable", "set-variable",
];

const PWSH_ALIASES: &[&str] = &[
    "ls", "dir", "cd", "cat", "ps", "kill", "echo", "cp", "mv", "rm", "rmdir",
    "curl", "wget", "man", "sleep", "clear", "cls",
];

const CMD_KEYWORDS: &[&str] = &[
    "echo", "off", "on", "set", "setlocal", "endlocal", "enabledelayedexpansion",
    "disabledelayedexpansion", "if", "else", "exist", "defined", "errorlevel",
    "for", "in", "do", "goto", "call", "start", "exit", "pause", "rem", "cls",
    "cd", "chdir", "md", "mkdir", "rd", "rmdir", "dir", "del", "erase", "copy",
    "xcopy", "robocopy", "move", "ren", "rename", "type", "find", "findstr",
    "tasklist", "taskkill", "net", "sc", "reg", "ping", "powershell", "cmd",
    "timeout", "shutdown", "choice", "shift", "pushd", "popd", "title", "color",
];

/// Byte cursor shared by every highlighter. Walking the UTF-8 bytes with a cursor
/// keeps token boundaries as byte offsets, so spans are appended as borrowed slices
/// instead of materializing the whole script as `Vec<(usize, char)>` every frame.
struct Scanner<'a> {
    text: &'a str,
    byte: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, byte: 0 }
    }

    fn byte(&self) -> usize {
        self.byte
    }

    fn peek(&self) -> Option<char> {
        self.text[self.byte..].chars().next()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.text[self.byte..].chars().nth(n)
    }

    fn prev(&self) -> Option<char> {
        self.text[..self.byte].chars().next_back()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.byte += ch.len_utf8();
        Some(ch)
    }

    /// Advances while `pred` holds and returns the byte offset after the last char.
    fn bump_while(&mut self, mut pred: impl FnMut(char) -> bool) -> usize {
        while let Some(ch) = self.peek() {
            if !pred(ch) {
                break;
            }
            self.bump();
        }
        self.byte
    }
}

/// A number literal starts with a digit, or with a dot followed by a digit.
fn starts_number(scanner: &Scanner<'_>) -> bool {
    match scanner.peek() {
        Some(ch) if ch.is_ascii_digit() => true,
        Some('.') => matches!(scanner.peek_n(1), Some(ch) if ch.is_ascii_digit()),
        _ => false,
    }
}

/// Scans a numeric literal shared by PowerShell and Lua: optional `0x` hex (with
/// optional `_` separators for PowerShell), at most one decimal point flanked by
/// digits, and an exponent only directly after a digit. Ranges like `1..10` and a
/// dangling `1e` therefore stay split instead of becoming one number token.
fn scan_number(scanner: &mut Scanner<'_>, allow_separators: bool) -> usize {
    if scanner.peek() == Some('0') && matches!(scanner.peek_n(1), Some('x' | 'X')) {
        scanner.bump();
        scanner.bump();
        scanner.bump_while(|ch| ch.is_ascii_hexdigit() || (allow_separators && ch == '_'));
        return scanner.byte();
    }

    scanner.bump_while(|ch| ch.is_ascii_digit() || (allow_separators && ch == '_'));

    // At most one decimal point, and only when a digit follows it.
    if scanner.peek() == Some('.') && matches!(scanner.peek_n(1), Some(ch) if ch.is_ascii_digit()) {
        scanner.bump();
        scanner.bump_while(|ch| ch.is_ascii_digit() || (allow_separators && ch == '_'));
    }

    // An exponent is only valid after a digit (`1e` stays `1` plus identifier `e`).
    let mantissa_ends_in_digit = scanner.prev().is_some_and(|ch| ch.is_ascii_digit());
    if mantissa_ends_in_digit && matches!(scanner.peek(), Some('e' | 'E')) {
        let (has_sign, digit_after) = match scanner.peek_n(1) {
            Some('+' | '-') => (true, scanner.peek_n(2)),
            other => (false, other),
        };
        if matches!(digit_after, Some(ch) if ch.is_ascii_digit()) {
            scanner.bump();
            if has_sign {
                scanner.bump();
            }
            scanner.bump_while(|ch| ch.is_ascii_digit());
        }
    }

    scanner.byte()
}

fn append_span(
    job: &mut LayoutJob,
    text: &str,
    color: Color32,
    bold: bool,
    font_size: f32,
) {
    let font_family = if bold {
        theme::mono_bold()
    } else {
        theme::mono()
    };
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::new(font_size, font_family),
            color,
            ..Default::default()
        },
    );
}

fn highlight_powershell(text: &str, t: &Tokens, font_size: f32, job: &mut LayoutJob) {
    let mut scanner = Scanner::new(text);

    while let Some(ch) = scanner.peek() {
        let start = scanner.byte();

        // 1. Block comment <# ... #>
        if ch == '<' && scanner.peek_n(1) == Some('#') {
            scanner.bump();
            scanner.bump();
            while let Some(cur) = scanner.peek() {
                if cur == '#' && scanner.peek_n(1) == Some('>') {
                    scanner.bump();
                    scanner.bump();
                    break;
                }
                scanner.bump();
            }
            append_span(job, &text[start..scanner.byte()], t.muted.gamma_multiply(0.7), false, font_size);
            continue;
        }

        // 2. Line comment # ...
        if ch == '#' {
            scanner.bump_while(|cur| cur != '\n');
            append_span(job, &text[start..scanner.byte()], t.muted.gamma_multiply(0.7), false, font_size);
            continue;
        }

        // 3. String literal "..." or '...'
        if ch == '"' || ch == '\'' {
            let quote = ch;
            scanner.bump();
            while let Some(cur) = scanner.peek() {
                if cur == '`' && quote == '"' && scanner.peek_n(1).is_some() {
                    // Backtick escape inside double quotes
                    scanner.bump();
                    scanner.bump();
                    continue;
                }
                if cur == quote {
                    scanner.bump();
                    if scanner.peek() == Some(quote) {
                        scanner.bump();
                        continue;
                    }
                    break;
                }
                scanner.bump();
            }
            append_span(job, &text[start..scanner.byte()], t.warn, false, font_size);
            continue;
        }

        // 4. Variables $var, $_, $env:VAR, ${...}
        if ch == '$' {
            scanner.bump();
            if scanner.peek() == Some('{') {
                scanner.bump_while(|cur| cur != '}');
                if scanner.peek() == Some('}') {
                    scanner.bump();
                }
            } else {
                scanner.bump_while(|cur| {
                    cur.is_alphanumeric() || cur == '_' || cur == ':' || cur == '?'
                });
            }
            append_span(job, &text[start..scanner.byte()], t.accent_ink, false, font_size);
            continue;
        }

        // 5. Parameter or comparison operator -Param, -eq, etc.
        if ch == '-'
            && matches!(scanner.peek_n(1), Some(cur) if cur.is_alphabetic() || cur == '-')
        {
            scanner.bump();
            scanner.bump_while(|cur| {
                cur.is_alphanumeric() || cur == '_' || cur == '-'
            });
            let word = &text[start..scanner.byte()];
            let lower = word.to_ascii_lowercase();
            if PWSH_OPERATORS.contains(&lower.as_str()) {
                append_span(job, word, t.ink_strong, true, font_size);
            } else {
                let color = if t.dark { Color32::from_rgb(120, 215, 255) } else { Color32::from_rgb(20, 120, 200) };
                append_span(job, word, color, false, font_size);
            }
            continue;
        }

        // 6. Range operator `..`; it must win over a leading-dot number so `1..10`
        // stays `1`, `..`, `10` instead of swallowing the second dot.
        if ch == '.' && scanner.peek_n(1) == Some('.') {
            scanner.bump();
            scanner.bump();
            append_span(job, &text[start..scanner.byte()], t.faint, false, font_size);
            continue;
        }

        // 7. Number literals
        if starts_number(&scanner) {
            scan_number(&mut scanner, true);
            let num_color = if t.dark { Color32::from_rgb(190, 155, 255) } else { Color32::from_rgb(120, 75, 200) };
            append_span(job, &text[start..scanner.byte()], num_color, false, font_size);
            continue;
        }

        // 8. Identifiers / keywords / cmdlets
        if ch.is_alphabetic() || ch == '_' {
            scanner.bump_while(|cur| cur.is_alphanumeric() || cur == '_' || cur == '-');
            let word = &text[start..scanner.byte()];
            let lower = word.to_ascii_lowercase();

            if PWSH_KEYWORDS.contains(&lower.as_str()) {
                append_span(job, word, t.ink_strong, true, font_size);
            } else if PWSH_CMDLETS.contains(&lower.as_str()) || PWSH_ALIASES.contains(&lower.as_str()) || (word.contains('-') && word.split('-').count() == 2) {
                append_span(job, word, t.good, false, font_size);
            } else {
                append_span(job, word, t.ink, false, font_size);
            }
            continue;
        }

        // 9. Delimiters & operators
        if "{}()[]|,;=<>+*/%".contains(ch) {
            scanner.bump();
            append_span(job, &text[start..scanner.byte()], t.faint, false, font_size);
            continue;
        }

        // 10. Whitespace and other characters
        scanner.bump();
        append_span(job, &text[start..scanner.byte()], t.ink, false, font_size);
    }
}

fn highlight_batch(text: &str, t: &Tokens, font_size: f32, job: &mut LayoutJob) {
    let mut scanner = Scanner::new(text);
    let mut at_line_start = true;

    while let Some(ch) = scanner.peek() {
        let start = scanner.byte();

        // 1. Comments starting with :: or REM at line start
        if at_line_start {
            if ch == ':' && scanner.peek_n(1) == Some(':') {
                scanner.bump_while(|cur| cur != '\n');
                append_span(job, &text[start..scanner.byte()], t.muted.gamma_multiply(0.7), false, font_size);
                at_line_start = true;
                if scanner.peek() == Some('\n') {
                    append_span(job, "\n", t.ink, false, font_size);
                    scanner.bump();
                }
                continue;
            }
            if matches!(ch, 'R' | 'r')
                && scanner.peek_n(1).is_some_and(|cur| cur.eq_ignore_ascii_case(&'E'))
                && scanner.peek_n(2).is_some_and(|cur| cur.eq_ignore_ascii_case(&'M'))
                && (scanner.peek_n(3).is_none() || scanner.peek_n(3).is_some_and(char::is_whitespace))
            {
                scanner.bump_while(|cur| cur != '\n');
                append_span(job, &text[start..scanner.byte()], t.muted.gamma_multiply(0.7), false, font_size);
                at_line_start = true;
                if scanner.peek() == Some('\n') {
                    append_span(job, "\n", t.ink, false, font_size);
                    scanner.bump();
                }
                continue;
            }
        }

        // 2. Labels :LABEL
        if at_line_start && ch == ':' && scanner.peek_n(1).is_some_and(char::is_alphanumeric) {
            scanner.bump_while(|cur| !cur.is_whitespace());
            append_span(job, &text[start..scanner.byte()], t.good, true, font_size);
            at_line_start = false;
            continue;
        }

        if !ch.is_whitespace() {
            at_line_start = false;
        }
        if ch == '\n' {
            at_line_start = true;
            append_span(job, "\n", t.ink, false, font_size);
            scanner.bump();
            continue;
        }

        // 3. String literals "..."
        if ch == '"' {
            scanner.bump();
            scanner.bump_while(|cur| cur != '"' && cur != '\n');
            if scanner.peek() == Some('"') {
                scanner.bump();
            }
            append_span(job, &text[start..scanner.byte()], t.warn, false, font_size);
            continue;
        }

        // 4. Variables %VAR%, %~dp0, !VAR!
        if ch == '%' {
            scanner.bump();
            if scanner.peek() == Some('%') {
                scanner.bump();
                scanner.bump_while(|cur| cur.is_alphanumeric());
            } else if scanner.peek() == Some('~') {
                scanner.bump_while(|cur| !cur.is_whitespace() && cur != '%');
            } else {
                scanner.bump_while(|cur| cur != '%' && cur != '\n' && !cur.is_whitespace());
                if scanner.peek() == Some('%') {
                    scanner.bump();
                }
            }
            append_span(job, &text[start..scanner.byte()], t.accent_ink, false, font_size);
            continue;
        }
        if ch == '!' {
            scanner.bump();
            scanner.bump_while(|cur| cur != '!' && cur != '\n');
            if scanner.peek() == Some('!') {
                scanner.bump();
            }
            append_span(job, &text[start..scanner.byte()], t.accent_ink, false, font_size);
            continue;
        }

        // 5. Switches /s /q /y
        if ch == '/' && scanner.peek_n(1).is_some_and(char::is_alphanumeric) {
            scanner.bump();
            scanner.bump_while(|cur| cur.is_alphanumeric());
            let color = if t.dark { Color32::from_rgb(120, 215, 255) } else { Color32::from_rgb(20, 120, 200) };
            append_span(job, &text[start..scanner.byte()], color, false, font_size);
            continue;
        }

        // 6. Numbers
        if ch.is_ascii_digit() {
            scanner.bump_while(|cur| cur.is_ascii_digit());
            let num_color = if t.dark { Color32::from_rgb(190, 155, 255) } else { Color32::from_rgb(120, 75, 200) };
            append_span(job, &text[start..scanner.byte()], num_color, false, font_size);
            continue;
        }

        // 7. Words / commands
        if ch.is_alphabetic() || ch == '_' {
            scanner.bump_while(|cur| cur.is_alphanumeric() || cur == '_');
            let word = &text[start..scanner.byte()];
            let lower = word.to_ascii_lowercase();
            if CMD_KEYWORDS.contains(&lower.as_str()) {
                append_span(job, word, t.ink_strong, true, font_size);
            } else {
                append_span(job, word, t.ink, false, font_size);
            }
            continue;
        }

        // 8. Operators
        if "&|><^=()[]".contains(ch) {
            scanner.bump();
            append_span(job, &text[start..scanner.byte()], t.faint, false, font_size);
            continue;
        }

        scanner.bump();
        append_span(job, &text[start..scanner.byte()], t.ink, false, font_size);
    }
}

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function",
    "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
    "true", "until", "while",
];

const LUA_GLOBALS: &[&str] = &[
    "ph", "log", "run_cmd", "get", "set", "print", "assert", "error", "ipairs",
    "pairs", "pcall", "xpcall", "select", "tonumber", "tostring", "type", "string",
    "table", "math", "io", "os", "coroutine", "package", "utf8", "debug",
];

fn highlight_lua(text: &str, t: &Tokens, font_size: f32, job: &mut LayoutJob) {
    let mut scanner = Scanner::new(text);

    while let Some(ch) = scanner.peek() {
        let start = scanner.byte();

        // 1. Block comment --[[ ... ]] or line comment -- ...
        if ch == '-' && scanner.peek_n(1) == Some('-') {
            scanner.bump();
            scanner.bump();
            if scanner.peek() == Some('[') && scanner.peek_n(1) == Some('[') {
                scanner.bump();
                scanner.bump();
                while let Some(cur) = scanner.peek() {
                    if cur == ']' && scanner.peek_n(1) == Some(']') {
                        scanner.bump();
                        scanner.bump();
                        break;
                    }
                    scanner.bump();
                }
            } else {
                scanner.bump_while(|cur| cur != '\n');
            }
            append_span(job, &text[start..scanner.byte()], t.muted.gamma_multiply(0.7), false, font_size);
            continue;
        }

        // 2. Multiline raw string literal [[ ... ]]
        if ch == '[' && scanner.peek_n(1) == Some('[') {
            scanner.bump();
            scanner.bump();
            while let Some(cur) = scanner.peek() {
                if cur == ']' && scanner.peek_n(1) == Some(']') {
                    scanner.bump();
                    scanner.bump();
                    break;
                }
                scanner.bump();
            }
            let str_color = if t.dark { Color32::from_rgb(165, 214, 167) } else { Color32::from_rgb(46, 125, 50) };
            append_span(job, &text[start..scanner.byte()], str_color, false, font_size);
            continue;
        }

        // 3. String literal "..." or '...'
        if ch == '"' || ch == '\'' {
            let quote = ch;
            scanner.bump();
            while let Some(cur) = scanner.peek() {
                if cur == '\\' && scanner.peek_n(1).is_some() {
                    scanner.bump();
                    scanner.bump();
                    continue;
                }
                if cur == quote {
                    scanner.bump();
                    break;
                }
                if cur == '\n' {
                    break;
                }
                scanner.bump();
            }
            let str_color = if t.dark { Color32::from_rgb(165, 214, 167) } else { Color32::from_rgb(46, 125, 50) };
            append_span(job, &text[start..scanner.byte()], str_color, false, font_size);
            continue;
        }

        // 4. Concatenation operator `..`; it must win over a leading-dot number so
        // `1..2` stays `1`, `..`, `2` instead of swallowing the second dot.
        if ch == '.' && scanner.peek_n(1) == Some('.') {
            scanner.bump();
            scanner.bump();
            append_span(job, &text[start..scanner.byte()], t.muted, false, font_size);
            continue;
        }

        // 5. Numbers (hex 0x... or decimal / float)
        if starts_number(&scanner) {
            scan_number(&mut scanner, false);
            let num_color = if t.dark { Color32::from_rgb(255, 183, 77) } else { Color32::from_rgb(230, 81, 0) };
            append_span(job, &text[start..scanner.byte()], num_color, false, font_size);
            continue;
        }

        // 6. Identifiers (keywords, globals, or normal identifiers)
        if ch.is_alphabetic() || ch == '_' {
            scanner.bump_while(|cur| cur.is_alphanumeric() || cur == '_');
            let word = &text[start..scanner.byte()];
            if LUA_KEYWORDS.contains(&word) {
                append_span(job, word, t.accent_ink, true, font_size);
            } else if LUA_GLOBALS.contains(&word) {
                let glob_color = if t.dark { Color32::from_rgb(129, 212, 250) } else { Color32::from_rgb(2, 119, 189) };
                append_span(job, word, glob_color, false, font_size);
            } else {
                append_span(job, word, t.ink, false, font_size);
            }
            continue;
        }

        // 7. Operators & punctuation
        if "+-*/%^#=~<>;:,.{}()[]".contains(ch) {
            scanner.bump();
            append_span(job, &text[start..scanner.byte()], t.muted, false, font_size);
            continue;
        }

        scanner.bump();
        append_span(job, &text[start..scanner.byte()], t.ink, false, font_size);
    }
}

pub fn highlight_script(
    text: &str,
    lang: ScriptLanguage,
    t: &Tokens,
    font_size: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    match lang {
        ScriptLanguage::PowerShell => highlight_powershell(text, t, font_size, &mut job),
        ScriptLanguage::Batch => highlight_batch(text, t, font_size, &mut job),
        ScriptLanguage::Lua => highlight_lua(text, t, font_size, &mut job),
    }
    job
}

/// Identity of a tokenized job in the highlight cache.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct HighlightKey {
    lang: ScriptLanguage,
    font_size_bits: u32,
    tokens: Tokens,
    text_hash: u64,
    text_len: usize,
}

#[derive(Clone)]
struct CachedLayout {
    key: HighlightKey,
    last_used: u64,
    job: LayoutJob,
}

/// Size of the job cache: enough for the script editors visible at once (the inline
/// script field and the expanded modal) without growing without bound.
const HIGHLIGHT_CACHE_SIZE: usize = 4;

#[derive(Clone, Default)]
struct HighlightCache {
    entries: Vec<CachedLayout>,
    clock: u64,
}

impl HighlightCache {
    fn get(&mut self, key: &HighlightKey) -> Option<LayoutJob> {
        self.clock += 1;
        let clock = self.clock;
        self.entries
            .iter_mut()
            .find(|entry| entry.key == *key)
            .map(|entry| {
                entry.last_used = clock;
                entry.job.clone()
            })
    }

    fn insert(&mut self, key: HighlightKey, job: LayoutJob) {
        self.clock += 1;
        if self.entries.len() >= HIGHLIGHT_CACHE_SIZE {
            if let Some((index, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
            {
                self.entries.swap_remove(index);
            }
        }
        self.entries.push(CachedLayout {
            key,
            last_used: self.clock,
            job,
        });
    }
}

/// Returns a tokenized job for `text`, reusing the cached one when the language,
/// font size, palette and text are unchanged. Only the wrap width is applied per
/// frame by [`create_script_layouter`].
fn cached_highlight(
    ctx: &egui::Context,
    text: &str,
    lang: ScriptLanguage,
    t: &Tokens,
    font_size: f32,
) -> LayoutJob {
    use std::hash::{Hash as _, Hasher as _};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let key = HighlightKey {
        lang,
        font_size_bits: font_size.to_bits(),
        tokens: *t,
        text_hash: hasher.finish(),
        text_len: text.len(),
    };
    let cache_id = egui::Id::unique("synector_script_highlight_cache");

    if let Some(job) = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HighlightCache>(cache_id)
            .get(&key)
    }) {
        return job;
    }

    let job = highlight_script(text, lang, t, font_size);
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HighlightCache>(cache_id)
            .insert(key, job.clone());
    });
    job
}

pub fn create_script_layouter(
    lang: ScriptLanguage,
    t: Tokens,
    font_size: f32,
) -> impl FnMut(&egui::Ui, &dyn egui::text_edit::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &dyn egui::text_edit::TextBuffer, wrap_width: f32| {
        let mut job = cached_highlight(ui.ctx(), text.as_str(), lang, &t, font_size);
        job.wrap.max_width = wrap_width;
        ui.painter().layout_job(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_badge_name_comes_from_the_catalog() {
        // Interpreter names are product names: EN and RU intentionally keep the same Latin text,
        // so the check pins the per-language catalog wiring instead of an EN/RU difference.
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);
        for loc in [en, ru] {
            assert_eq!(ScriptLanguage::PowerShell.badge_name(loc), loc.script_badge_powershell);
            assert_eq!(ScriptLanguage::Batch.badge_name(loc), loc.script_badge_batch);
            assert_eq!(ScriptLanguage::Lua.badge_name(loc), loc.script_badge_lua);
            assert!(!ScriptLanguage::Batch.badge_name(loc).is_empty());
        }
        assert_eq!(
            ScriptLanguage::PowerShell.badge_name(en),
            ScriptLanguage::PowerShell.badge_name(ru),
            "interpreter names stay Latin in both catalogs"
        );
        assert_ne!(ScriptLanguage::PowerShell.badge_name(en), ScriptLanguage::Batch.badge_name(en));
        assert_ne!(ScriptLanguage::Batch.badge_name(en), ScriptLanguage::Lua.badge_name(en));
    }

    fn section_texts(job: &LayoutJob) -> Vec<&str> {
        job.sections
            .iter()
            .map(|section| {
                &job.text[section.byte_range.start.0..section.byte_range.end.0]
            })
            .collect()
    }

    fn section_color(job: &LayoutJob, text: &str) -> Option<Color32> {
        job.sections
            .iter()
            .find(|section| {
                &job.text[section.byte_range.start.0..section.byte_range.end.0] == text
            })
            .map(|section| section.format.color)
    }

    #[test]
    fn test_powershell_highlighter_preserves_text() {
        let t = Tokens::DARK;
        let script = "# Comment\n<# Block #>\n$proc = \"notepad.exe\"\nif ($true) {\n    Start-Process -FilePath $proc -Force\n}";
        let job = highlight_script(script, ScriptLanguage::PowerShell, &t, 12.0);
        assert!(!job.sections.is_empty());
        assert_eq!(job.text, script);
    }

    #[test]
    fn test_batch_highlighter_preserves_text() {
        let t = Tokens::DARK;
        let script = "@echo off\n:: comment\nrem comment\nset TARGET=app.exe\nif exist %TARGET% (\n    start /b \"\" \"%TARGET%\"\n)";
        let job = highlight_script(script, ScriptLanguage::Batch, &t, 12.0);
        assert!(!job.sections.is_empty());
        assert_eq!(job.text, script);
    }

    #[test]
    fn test_lua_highlighter_preserves_text() {
        let t = Tokens::DARK;
        let script = "-- Lua script\n--[[ block ]]\nlocal target = \"notepad.exe\"\nph.log(\"info\", \"Starting: \" .. target)\nif ph.get(\"status\") == nil then\n    ph.set(\"status\", 1)\nend";
        let job = highlight_script(script, ScriptLanguage::Lua, &t, 12.0);
        assert!(!job.sections.is_empty());
        assert_eq!(job.text, script);
    }

    #[test]
    fn test_scanner_keeps_byte_offsets_on_unicode_scripts() {
        let t = Tokens::DARK;
        let script = "-- комментарий\nlocal s = \"привет\"\nlocal n = 1.5";
        let job = highlight_script(script, ScriptLanguage::Lua, &t, 12.0);
        assert_eq!(job.text, script);
        assert_eq!(section_texts(&job).concat(), script);
        // Every section must start and end on UTF-8 boundaries.
        for section in &job.sections {
            assert!(job.text.is_char_boundary(section.byte_range.start.0));
            assert!(job.text.is_char_boundary(section.byte_range.end.0));
        }
    }

    #[test]
    fn test_number_scanner_splits_ranges_and_rejects_dangling_exponents() {
        let t = Tokens::DARK;
        let ps_number = Color32::from_rgb(190, 155, 255);

        let powershell = highlight_script(
            "$range = 1..10; $exp = 1e; $float = 1.5e-3; $hex = 0x1F",
            ScriptLanguage::PowerShell,
            &t,
            12.0,
        );
        let texts = section_texts(&powershell);
        assert!(texts.contains(&"1"));
        assert!(texts.contains(&".."));
        assert!(texts.contains(&"10"));
        assert!(!texts.contains(&"1..10"));
        assert!(!texts.contains(&"1e"));
        assert!(texts.contains(&"1.5e-3"));
        assert!(texts.contains(&"0x1F"));
        assert_eq!(section_color(&powershell, "1"), Some(ps_number));
        assert_eq!(section_color(&powershell, "10"), Some(ps_number));
        assert_ne!(section_color(&powershell, "e"), Some(ps_number));

        let lua = highlight_script(
            "local range = 1..2\nlocal exp = 1e\nlocal float = 1.5e-3",
            ScriptLanguage::Lua,
            &t,
            12.0,
        );
        let texts = section_texts(&lua);
        assert!(texts.contains(&"1"));
        assert!(texts.contains(&".."));
        assert!(texts.contains(&"2"));
        assert!(!texts.contains(&"1..2"));
        assert!(!texts.contains(&"1e"));
        assert!(texts.contains(&"1.5e-3"));
        let lua_number = Color32::from_rgb(255, 183, 77);
        assert_eq!(section_color(&lua, "1"), Some(lua_number));
        assert_eq!(section_color(&lua, "2"), Some(lua_number));
    }

    #[test]
    fn test_highlight_cache_is_bounded_and_evicts_least_recently_used() {
        let mut cache = HighlightCache::default();
        let key = |text_hash: u64| HighlightKey {
            lang: ScriptLanguage::Lua,
            font_size_bits: 12.0f32.to_bits(),
            tokens: Tokens::DARK,
            text_hash,
            text_len: 1,
        };
        let job = |text: &str| {
            let mut job = LayoutJob::default();
            job.append(text, 0.0, TextFormat::default());
            job
        };

        for text_hash in 0..HIGHLIGHT_CACHE_SIZE as u64 {
            cache.insert(key(text_hash), job("x"));
        }
        assert_eq!(cache.entries.len(), HIGHLIGHT_CACHE_SIZE);
        assert!(cache.get(&key(1)).is_some());

        cache.insert(key(99), job("y"));
        assert_eq!(cache.entries.len(), HIGHLIGHT_CACHE_SIZE);
        assert!(
            cache.get(&key(0)).is_none(),
            "the least recently used entry should be evicted"
        );
        assert!(cache.get(&key(1)).is_some());
    }

    #[test]
    fn test_cached_highlight_keys_by_text_theme_and_font_size() {
        let ctx = egui::Context::default();
        let script = "local n = 1.5";
        let first = cached_highlight(&ctx, script, ScriptLanguage::Lua, &Tokens::DARK, 12.0);
        let second = cached_highlight(&ctx, script, ScriptLanguage::Lua, &Tokens::DARK, 12.0);
        assert_eq!(first, second);
        assert_ne!(
            first,
            cached_highlight(&ctx, "local n = 2.5", ScriptLanguage::Lua, &Tokens::DARK, 12.0)
        );
        assert_ne!(
            first,
            cached_highlight(&ctx, script, ScriptLanguage::Lua, &Tokens::LIGHT, 12.0)
        );
        assert_ne!(
            first,
            cached_highlight(&ctx, script, ScriptLanguage::Lua, &Tokens::DARK, 14.0)
        );
    }
}
