//! Changelog & Release Notes Modal:
//! Centered studio canvas for displaying GitHub release notes
//! with HTML & Markdown parsing, clickable links, and one-click update installation.

use egui::{
    CornerRadius, RichText, Ui, vec2,
};
use crate::gui::components::modal::Modal;
use crate::gui::components::widgets::{self, Glyph, Kind};
use crate::gui::state::GuiState;
use crate::gui::theme::{self, Tokens};

pub fn render_changelog_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.show_changelog_modal && state.available_update.is_some();

    let Some(info) = state.available_update.clone() else { return };
    let loc = state.loc();

    let mut update_clicked = false;

    let modal_res = Modal::new("changelog-modal", is_open)
        .width_fraction(0.76)
        .height_fraction(0.78)
        .min_size(vec2(580.0, 380.0))
        .max_size(vec2(920.0, 720.0))
        .header_height(56.0)
        .body_sunken(true)
        .show(ctx, &t, |frame| {
            // --- 1. HEADER ZONE ---
            frame.header(|ui, close_req| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add_space(20.0);

                    ui.vertical(|ui| {
                        let top_space = ((ui.available_height() - 35.0).max(0.0) / 2.0).round();
                        ui.add_space(top_space);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        widgets::hud(ui, loc.changelog_eyebrow, t.muted);
                        ui.label(
                            RichText::new(&info.name)
                                .family(theme::semibold())
                                .size(16.0)
                                .color(t.ink_strong),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(20.0);
                        if widgets::glyph_button(ui, Glyph::Close, loc.changelog_btn_close).clicked() {
                            *close_req = true;
                        }

                        ui.add_space(8.0);

                        // Update Now Button
                        let is_busy = state.is_downloading_update;
                        let update_btn_label = if is_busy {
                            format!("{} {:.0}%", loc.changelog_btn_update, (state.update_download_progress * 100.0).clamp(0.0, 100.0))
                        } else {
                            loc.changelog_btn_update.to_string()
                        };

                        ui.add_enabled_ui(!is_busy, |ui| {
                            if widgets::button(ui, &update_btn_label, Kind::Primary, 0.0).clicked() {
                                update_clicked = true;
                            }
                        });

                        if !info.html_url.is_empty() {
                            ui.add_space(8.0);
                            let open_label = format!("{} {}", egui_phosphor::regular::ARROW_SQUARE_OUT, loc.changelog_open_github);
                            if widgets::button(ui, &open_label, Kind::Secondary, 0.0).clicked() {
                                ui.ctx().open_url(egui::OpenUrl::same_tab(&info.html_url));
                            }
                        }
                    });
                });
            });

            // --- 2. BODY ZONE (Rich HTML & Markdown) ---
            frame.body(|ui| {
                let body_rect = ui.max_rect();
                let inner_body = body_rect.shrink2(vec2(22.0, 16.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(inner_body), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("changelog_scroll_area")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 10.0;

                            let body_text = info.body.trim_start_matches('\u{feff}').trim();
                            if body_text.is_empty() {
                                widgets::hud(ui, loc.changelog_no_notes, t.muted);
                            } else {
                                render_rich_changelog(ui, body_text, &t);
                            }
                        });
                });
            });
        });

    if let Some(res) = modal_res {
        if res.close_requested {
            state.show_changelog_modal = false;
        }
    }

    if update_clicked {
        state.show_changelog_modal = false;
        state.start_update(info);
    }
}

/// Renders changelog content supporting HTML tags (<p>, <br>, <pre>, <h1>-<h6>, <b>, <i>, <code>, <a>, <ul>, <li>)
/// and Markdown syntax (#, ##, bullets, `code`, **bold**, links).
fn render_rich_changelog(ui: &mut Ui, text: &str, t: &Tokens) {
    let clean = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");

    let mut in_code_block = false;
    let mut code_block_buf = String::new();

    for raw_line in clean.lines() {
        let line = raw_line.trim_end();

        // 1. HTML <pre> tags are handled independently of the Markdown fence: a lone opening
        //    tag starts a block, a lone closing tag ends it, and `<pre>…</pre>` on one line
        //    keeps its content without leaving the block open for every following line. (The
        //    old combined toggle consumed the line and dropped inline content.)
        match classify_pre_line(line) {
            PreLine::Open(content) => {
                in_code_block = true;
                if !content.trim().is_empty() {
                    code_block_buf.push_str(content);
                    code_block_buf.push('\n');
                }
                continue;
            }
            PreLine::Close(rest) => {
                if in_code_block {
                    render_code_block(ui, &code_block_buf, t);
                    code_block_buf.clear();
                    in_code_block = false;
                }
                if !rest.trim().is_empty() {
                    render_plain_changelog_line(ui, rest, t);
                }
                continue;
            }
            PreLine::Inline { before, code, after } => {
                if !before.trim().is_empty() {
                    render_plain_changelog_line(ui, before, t);
                }
                if in_code_block {
                    code_block_buf.push_str(code);
                    code_block_buf.push('\n');
                } else {
                    render_code_block(ui, code, t);
                }
                if !after.trim().is_empty() {
                    render_plain_changelog_line(ui, after, t);
                }
                continue;
            }
            PreLine::None => {}
        }

        // 2. Markdown code fence
        if line.starts_with("```") {
            if in_code_block {
                render_code_block(ui, &code_block_buf, t);
                code_block_buf.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_block_buf.push_str(line);
            code_block_buf.push('\n');
            continue;
        }

        // 3. Horizontal divider
        if line == "---" || line == "***" || line == "___" || line.contains("<hr>") || line.contains("<hr/>") {
            widgets::hrule(ui);
            continue;
        }

        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }

        // 4. Headings (Markdown: #, ##, ### or HTML: <h1>, <h2>, <h3>, <h4>)
        if let Some(title) = trimmed.strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("<h4>"))
            .or_else(|| trimmed.strip_prefix("<h3>"))
        {
            let clean_title = strip_tags(title);
            ui.add_space(4.0);
            ui.label(
                RichText::new(clean_title)
                    .family(theme::semibold())
                    .size(14.0)
                    .color(t.ink_strong),
            );
            continue;
        }

        if let Some(title) = trimmed.strip_prefix("## ")
            .or_else(|| trimmed.strip_prefix("<h2>"))
        {
            let clean_title = strip_tags(title);
            ui.add_space(6.0);
            ui.label(
                RichText::new(clean_title)
                    .family(theme::semibold())
                    .size(16.0)
                    .color(t.ink_strong),
            );
            continue;
        }

        if let Some(title) = trimmed.strip_prefix("# ")
            .or_else(|| trimmed.strip_prefix("<h1>"))
        {
            let clean_title = strip_tags(title);
            ui.add_space(8.0);
            ui.label(
                RichText::new(clean_title)
                    .family(theme::display())
                    .size(18.0)
                    .color(t.ink_strong),
            );
            continue;
        }

        // 5. Bullet list items
        if let Some(item_text) = trimmed.strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("• "))
            .or_else(|| trimmed.strip_prefix("<li>"))
        {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("•").family(theme::mono_bold()).color(t.accent_ink));
                render_inline_markup(ui, item_text, t);
            });
            continue;
        }

        // 6. Standard paragraph / line
        render_plain_changelog_line(ui, trimmed, t);
    }

    if in_code_block && !code_block_buf.is_empty() {
        render_code_block(ui, &code_block_buf, t);
    }
}

/// One line of changelog text with inline HTML/Markdown markup.
fn render_plain_changelog_line(ui: &mut Ui, line: &str, t: &Tokens) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        render_inline_markup(ui, line.trim(), t);
    });
}

/// Where a `<pre>` tag sits on a changelog line.
#[derive(Debug, PartialEq, Eq)]
enum PreLine<'a> {
    /// No `<pre>`/`</pre>` on this line.
    None,
    /// An opening tag without a closing one; the rest of the line starts the code block.
    Open(&'a str),
    /// A closing tag without a matching opening one; `rest` follows the tag.
    Close(&'a str),
    /// An inline `<pre>…</pre>` pair with the text before and after it.
    Inline { before: &'a str, code: &'a str, after: &'a str },
}

/// Classifies the `<pre>` tags on one changelog line. Open and close tags are independent, so a
/// lone closing tag cannot start a code block and an inline pair keeps the content between the
/// tags instead of consuming the whole line.
fn classify_pre_line(line: &str) -> PreLine<'_> {
    let Some(open) = line.find("<pre>") else {
        return match line.find("</pre>") {
            Some(close) => PreLine::Close(&line[close + "</pre>".len()..]),
            None => PreLine::None,
        };
    };
    let after_open = open + "<pre>".len();
    match line[after_open..].find("</pre>") {
        Some(close) => PreLine::Inline {
            before: &line[..open],
            code: &line[after_open..after_open + close],
            after: &line[after_open + close + "</pre>".len()..],
        },
        None => PreLine::Open(&line[after_open..]),
    }
}

/// Renders a framed monospace code block.
fn render_code_block(ui: &mut Ui, code: &str, t: &Tokens) {
    egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(code.trim_end())
                    .family(theme::mono())
                    .size(12.0)
                    .color(t.ink),
            );
        });
}

/// Parses inline HTML tags (<a>, <b>, <strong>, <i>, <code>) and Markdown (**bold**, `code`, links).
fn render_inline_markup(ui: &mut Ui, line: &str, t: &Tokens) {
    let mut rest = line;

    while !rest.is_empty() {
        // Find next tag or markup token
        if let Some(link_start) = rest.find("<a ") {
            if link_start > 0 {
                render_simple_text(ui, &rest[..link_start], t);
            }
            if let Some(link_end) = rest[link_start..].find("</a>") {
                let tag_slice = &rest[link_start..link_start + link_end + 4];
                render_html_link(ui, tag_slice, t);
                rest = &rest[link_start + link_end + 4..];
                continue;
            } else {
                render_simple_text(ui, &rest[link_start..], t);
                break;
            }
        }

        // Markdown link [text](url)
        if let Some(bracket_start) = rest.find('[') {
            if let Some(bracket_end) = rest[bracket_start..].find(']') {
                let text_inside = &rest[bracket_start + 1..bracket_start + bracket_end];
                let after_bracket = &rest[bracket_start + bracket_end + 1..];
                if after_bracket.starts_with('(') {
                    if let Some(paren_end) = after_bracket.find(')') {
                        let url_inside = &after_bracket[1..paren_end];
                        if bracket_start > 0 {
                            render_simple_text(ui, &rest[..bracket_start], t);
                        }
                        if ui.link(RichText::new(text_inside).color(t.accent_ink)).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::same_tab(url_inside));
                        }
                        rest = &after_bracket[paren_end + 1..];
                        continue;
                    }
                }
            }
        }

        // Raw URLs (https://... or http://...)
        if let Some(http_pos) = rest.find("http://").or_else(|| rest.find("https://")) {
            if http_pos > 0 {
                render_simple_text(ui, &rest[..http_pos], t);
            }
            let url_rest = &rest[http_pos..];
            let end_pos = url_rest
                .find(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == ')' || c == ']' || c == '"')
                .unwrap_or(url_rest.len());
            let url = &url_rest[..end_pos];
            if ui.link(RichText::new(url).color(t.accent_ink)).clicked() {
                ui.ctx().open_url(egui::OpenUrl::same_tab(url));
            }
            rest = &url_rest[end_pos..];
            continue;
        }

        // Bold or strong tags
        if let Some(bold_start) = rest.find("<b>").or_else(|| rest.find("<strong>")) {
            let is_b = rest[bold_start..].starts_with("<b>");
            let tag_len = if is_b { 3 } else { 8 };
            let close_tag = if is_b { "</b>" } else { "</strong>" };

            if bold_start > 0 {
                render_simple_text(ui, &rest[..bold_start], t);
            }
            if let Some(close_pos) = rest[bold_start + tag_len..].find(close_tag) {
                let bold_text = &rest[bold_start + tag_len..bold_start + tag_len + close_pos];
                ui.label(RichText::new(bold_text).family(theme::semibold()).color(t.ink_strong));
                rest = &rest[bold_start + tag_len + close_pos + close_tag.len()..];
                continue;
            }
        }

        // Markdown **bold**
        if let Some(bold_start) = rest.find("**") {
            if let Some(bold_end) = rest[bold_start + 2..].find("**") {
                if bold_start > 0 {
                    render_simple_text(ui, &rest[..bold_start], t);
                }
                let bold_text = &rest[bold_start + 2..bold_start + 2 + bold_end];
                ui.label(RichText::new(bold_text).family(theme::semibold()).color(t.ink_strong));
                rest = &rest[bold_start + 2 + bold_end + 2..];
                continue;
            }
        }

        // Monospace `code` or <code>
        if let Some(code_start) = rest.find("<code>").or_else(|| rest.find('`')) {
            let is_tag = rest[code_start..].starts_with("<code>");
            let open_len = if is_tag { 6 } else { 1 };
            let close_token = if is_tag { "</code>" } else { "`" };

            if code_start > 0 {
                render_simple_text(ui, &rest[..code_start], t);
            }
            if let Some(close_pos) = rest[code_start + open_len..].find(close_token) {
                let code_text = &rest[code_start + open_len..code_start + open_len + close_pos];
                ui.label(
                    RichText::new(code_text)
                        .family(theme::mono())
                        .size(11.5)
                        .color(t.accent_ink)
                        .background_color(t.sunken),
                );
                rest = &rest[code_start + open_len + close_pos + close_token.len()..];
                continue;
            }
        }

        // Fallback for remaining text
        render_simple_text(ui, rest, t);
        break;
    }
}

/// Renders plain text with stripped residual HTML tags.
fn render_simple_text(ui: &mut Ui, s: &str, t: &Tokens) {
    let clean = strip_tags(s);
    if !clean.is_empty() {
        ui.label(RichText::new(clean).color(t.ink).size(13.0));
    }
}

/// Parses `<a href="...">text</a>` and renders as a clickable link.
fn render_html_link(ui: &mut Ui, html: &str, t: &Tokens) {
    let href = if let Some(h_start) = html.find("href=\"") {
        let after = &html[h_start + 6..];
        after.find('"').map(|h_end| &after[..h_end])
    } else if let Some(h_start) = html.find("href='") {
        let after = &html[h_start + 6..];
        after.find('\'').map(|h_end| &after[..h_end])
    } else {
        None
    };

    let inner = if let Some(tag_close) = html.find('>') {
        if let Some(end_tag) = html.rfind("</a>") {
            if tag_close < end_tag {
                &html[tag_close + 1..end_tag]
            } else {
                html
            }
        } else {
            html
        }
    } else {
        html
    };

    let link_text = strip_tags(inner);
    if let Some(url) = href {
        if ui.link(RichText::new(link_text).color(t.accent_ink)).clicked() {
            ui.ctx().open_url(egui::OpenUrl::same_tab(url));
        }
    } else {
        ui.label(RichText::new(link_text).color(t.accent_ink));
    }
}

/// Helper to strip basic HTML tags from a string.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;

    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_tags() {
        assert_eq!(strip_tags("<p>Hello <b>World</b>!</p>"), "Hello World!");
        assert_eq!(strip_tags("<a href=\"https://github.com\">Click here</a>"), "Click here");
        assert_eq!(strip_tags("Plain text with no tags"), "Plain text with no tags");
        assert_eq!(strip_tags("<code>let x = 10;</code>"), "let x = 10;");
    }

    #[test]
    fn test_strip_bom_and_heading_prefix() {
        let text_with_bom = "\u{feff}### Added\n- Feature 1";
        let clean = text_with_bom.trim_start_matches('\u{feff}').trim();
        let first_line = clean.lines().next().unwrap();
        assert_eq!(first_line, "### Added");
        assert_eq!(first_line.strip_prefix("### "), Some("Added"));
    }

    /// U03-012: `<pre>` and `</pre>` are classified independently. An inline pair keeps its
    /// content (it used to be consumed and left the block open for every later line), and a
    /// stray closing tag no longer opens a block.
    #[test]
    fn test_classify_pre_line_keeps_inline_content() {
        assert_eq!(classify_pre_line("plain text"), PreLine::None);
        assert_eq!(
            classify_pre_line("  <pre>cargo build --release</pre>"),
            PreLine::Inline {
                before: "  ",
                code: "cargo build --release",
                after: "",
            }
        );
        assert_eq!(
            classify_pre_line("<pre></pre>"),
            PreLine::Inline { before: "", code: "", after: "" }
        );
        assert_eq!(classify_pre_line("<pre><b>code</b>"), PreLine::Open("<b>code</b>"));
        assert_eq!(classify_pre_line("text before</pre>"), PreLine::Close(""));
        assert_eq!(classify_pre_line("</pre> trailing text"), PreLine::Close(" trailing text"));
    }
}
