//! Expanded Script Canvas Modal:
//! Centered studio canvas for editing PowerShell and CMD scripts
//! with syntax highlighting, line metrics and smooth animations.

use egui::{pos2, vec2, Stroke};
use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::script_highlighter::create_script_layouter;
use crate::gui::components::widgets;
use crate::gui::state::GuiState;
use crate::gui::theme::{self, Tokens};
use crate::models::Action;

/// What the dialog's own history shortcuts ask for.
#[derive(Clone, Copy)]
enum ScriptHistoryAction {
    Undo,
    Redo,
}

/// Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z for the dialog's own text history.
///
/// The script dialog keeps its history the way the studios do, and it owns these keys while it is
/// open: the inspector stops handling the draft's shortcuts in that state, so the keys are still
/// unconsumed here. They are consumed before the text field renders, so egui's own per-field undo
/// cannot run on top of this history. Redo is matched first, because `consume_key` ignores an
/// extra Shift and `Ctrl+Z` would otherwise swallow `Ctrl+Shift+Z`.
fn read_script_history_shortcuts(ctx: &egui::Context) -> Option<ScriptHistoryAction> {
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
            || i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z)
            || i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
            || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
        {
            Some(ScriptHistoryAction::Redo)
        } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
            || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
        {
            Some(ScriptHistoryAction::Undo)
        } else {
            None
        }
    })
}

pub fn render_script_editor_canvas(ctx: &egui::Context, state: &mut GuiState) {
    let is_open = state.expanded_script_editor.is_some();
    let active_editor = state
        .expanded_script_editor
        .as_ref()
        .or(state.last_script_editor.as_ref())
        .cloned();

    let Some(mut modal) = active_editor else { return };
    let t = Tokens::get(ctx);

    // Retrieve script string from state.editing_rule
    let mut script_buffer = String::new();
    let mut found = false;
    if let Some(rule) = &state.editing_rule {
        if let Some(Action::ExecuteScript { script, .. }) = rule.actions.get(modal.step_idx) {
            script_buffer = script.clone();
            found = true;
        }
    }

    if !found && is_open {
        state.close_script_editor();
        return;
    }

    // History shortcuts first: the dialog's text history is its own, so Ctrl+Z / Ctrl+Y step
    // through the script's edits instead of the rule draft's snapshot history.
    if is_open {
        if let Some(action) = read_script_history_shortcuts(ctx) {
            // Commit the typing gesture in progress first, so the undo lands on the state the
            // gesture started from instead of skipping it.
            modal.commit_pending_undo(&script_buffer);
            let restored = match action {
                ScriptHistoryAction::Undo => modal.undo(&script_buffer),
                ScriptHistoryAction::Redo => modal.redo(&script_buffer),
            };
            if let Some(text) = restored {
                script_buffer = text;
                if let Some(rule) = &mut state.editing_rule {
                    if let Some(Action::ExecuteScript { script, .. }) = rule.actions.get_mut(modal.step_idx) {
                        *script = script_buffer.clone();
                    }
                }
            }
        }
    }

    // The script as this pass found it, before the text field edits it: the first changed pass of
    // a gesture keeps this as the pre-gesture snapshot.
    let text_before_edit = script_buffer.clone();
    let mut text_changed = false;

    let modal_res = Modal::new("script-canvas-modal", is_open)
        .width_fraction(0.84)
        .height_fraction(0.82)
        .min_size(vec2(620.0, 420.0))
        .max_size(vec2(1150.0, 840.0))
        .header_height(52.0)
        .body_sunken(true)
        .show(ctx, &t, |frame| {
            let loc = state.loc();

            // 1. Header slice
            frame.header(|ui, close_req| {
                render_modal_header(
                    ui,
                    close_req,
                    &t,
                    ModalHeader {
                        glyph: None,
                        glyph_size: 16.0,
                        close_tip: loc.modal_tip_close,
                        left_pad: 20.0,
                        glyph_pad: 0.0,
                        right_pad: 20.0,
                        item_spacing_x: None,
                    },
                    |ui| {
                        widgets::hud(ui, &format!("{} {:02}", loc.inspector_step_prefix, modal.step_idx + 1), t.accent_ink);
                        ui.add_space(8.0);
                        widgets::badge(ui, modal.language.badge_name(loc), t.accent_ink);
                    },
                    |_| {},
                );
            });

            // 2. Editor body slice
            frame.body(|ui| {
                let body_rect = ui.max_rect();
                let inner_body = body_rect.shrink2(vec2(20.0, 16.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(inner_body), |ui| {
                    let mut layouter = create_script_layouter(modal.language, t, 13.0);

                    egui::ScrollArea::vertical()
                        .id_salt("expanded_canvas_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let text_resp = ui.add(
                                egui::TextEdit::multiline(&mut script_buffer)
                                    .desired_width(ui.available_width())
                                    .desired_rows(24)
                                    .font(egui::FontId::new(13.0, theme::mono()))
                                    .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT).stroke(Stroke::NONE))
                                    .margin(vec2(0.0, 0.0))
                                    .layouter(&mut layouter),
                            );
                            if text_resp.changed() {
                                text_changed = true;
                            }
                        });
                });

                // Floating semi-transparent metrics label in bottom-right corner of editor canvas
                {
                    let line_count = script_buffer.lines().count().max(1);
                    let char_count = script_buffer.chars().count();
                    let metrics_text = format!("{} {}  •  {} {}", line_count, loc.modal_lines, char_count, loc.modal_chars);
                    let text_color = t.faint.gamma_multiply(0.5);
                    let galley = widgets::caps(ui, &metrics_text, 9.5, theme::mono(), 9.5 * 0.10, text_color);
                    let text_pos = pos2(
                        inner_body.right() - 8.0 - galley.size().x,
                        inner_body.bottom() - 4.0 - galley.size().y,
                    );
                    ui.painter().galley_with_override_text_color(text_pos, galley, text_color);
                }
            });
        });

    if text_changed {
        // One undo entry per typing gesture: the first changed pass records the text the gesture
        // started from, and the first pass without a change commits it.
        if modal.pending_undo.is_none() {
            modal.pending_undo = Some(text_before_edit);
        }
        if let Some(rule) = &mut state.editing_rule {
            if let Some(Action::ExecuteScript { ref mut script, .. }) = rule.actions.get_mut(modal.step_idx) {
                *script = script_buffer;
            }
        }
    } else {
        modal.commit_pending_undo(&script_buffer);
    }

    if let Some(res) = modal_res {
        if res.close_requested {
            state.close_script_editor();
        } else if is_open {
            // Keep the dialog's own history across passes: the view is rebuilt from this state
            // every frame, exactly like the studios' state.
            state.expanded_script_editor = Some(modal);
        }
    } else if !is_open {
        state.last_script_editor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Rule, ScriptType};

    /// A draft whose first step is a PowerShell script.
    fn script_draft(script: &str) -> Rule {
        Rule {
            id: "rule_script_modal".into(),
            name: "Script".into(),
            actions: vec![Action::ExecuteScript {
                interpreter: ScriptType::PowerShell,
                script: script.to_string(),
                run_in_window: false,
            }],
            ..Default::default()
        }
    }

    /// The draft's first script, as the rule inspector would read it.
    fn draft_script(state: &GuiState) -> &str {
        match state.editing_rule.as_ref().and_then(|rule| rule.actions.first()) {
            Some(Action::ExecuteScript { script, .. }) => script,
            _ => panic!("the test draft starts with one script step"),
        }
    }

    /// `(undo entries, redo entries)` of the dialog's own history.
    fn dialog_history(state: &GuiState) -> (usize, usize) {
        let modal = state.expanded_script_editor.as_ref().expect("the dialog stays open");
        (modal.undo_stack.len(), modal.redo_stack.len())
    }

    /// One GUI pass with the drawer and the dialog, in the order the frame runs them.
    fn gui_pass(ctx: &egui::Context, state: &mut GuiState, time: f64, events: Vec<egui::Event>) {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                crate::gui::views::inspector::render_inspector_drawer(ui.ctx(), state);
                render_script_editor_canvas(ui.ctx(), state);
            },
        );
        out.textures_delta.clear();
    }

    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            })
            .collect()
    }

    fn history_key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }]
    }

    fn type_text(text: &str) -> Vec<egui::Event> {
        vec![egui::Event::Text(text.to_string())]
    }

    /// A passing frame of the dialog, with the script editor focused.
    fn open_dialog(ctx: &egui::Context, state: &mut GuiState) {
        for time in [0.0, 0.4, 0.8] {
            gui_pass(ctx, state, time, Vec::new());
        }
        gui_pass(ctx, state, 1.0, click(pos2(640.0, 450.0)));
    }

    /// The script dialog keeps its own text history, like the studios keep theirs: Ctrl+Z and
    /// Ctrl+Y walk that history or nothing at all — the rule draft's snapshot history must not
    /// move while the dialog is open, or a single undo would revert unrelated rule edits too.
    #[test]
    fn test_script_dialog_owns_ctrl_z_and_ctrl_y_while_open() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.editing_rule = Some(script_draft("Write-Host 'before'"));
        state.open_script_editor(0, crate::gui::components::script_highlighter::ScriptLanguage::PowerShell);
        open_dialog(&ctx, &mut state);

        // Typing, then Ctrl+Z before the gesture was committed: the flush makes the undo land on
        // the text from before the gesture.
        gui_pass(&ctx, &mut state, 1.1, type_text("tail"));
        let typed = draft_script(&state).to_string();
        assert!(typed.contains("tail"), "the dialog typed into the draft: {typed}");
        gui_pass(&ctx, &mut state, 1.2, history_key(egui::Key::Z, egui::Modifiers::CTRL));
        assert_eq!(
            draft_script(&state),
            "Write-Host 'before'",
            "Ctrl+Z with the dialog on top steps back through the dialog's own history"
        );
        assert_eq!(dialog_history(&state), (0, 1));
        assert!(
            state.undo_stack.is_empty(),
            "the rule draft's history is not touched while the dialog owns the keys"
        );

        // Ctrl+Y brings the typed text back without adding a new undo entry.
        gui_pass(&ctx, &mut state, 1.3, history_key(egui::Key::Y, egui::Modifiers::CTRL));
        assert_eq!(draft_script(&state), typed);
        assert_eq!(dialog_history(&state), (1, 0));
        gui_pass(&ctx, &mut state, 1.4, Vec::new());
        assert_eq!(dialog_history(&state), (1, 0), "an idle pass commits nothing new");

        // Ctrl+Shift+Z is the documented second redo shortcut; it must redo, not undo.
        gui_pass(&ctx, &mut state, 1.5, history_key(egui::Key::Z, egui::Modifiers::CTRL));
        assert_eq!(draft_script(&state), "Write-Host 'before'");
        gui_pass(&ctx, &mut state, 1.6, history_key(egui::Key::Z, egui::Modifiers::CTRL | egui::Modifiers::SHIFT));
        assert_eq!(draft_script(&state), typed, "Ctrl+Shift+Z redoes the dialog's text");
        assert_eq!(dialog_history(&state), (1, 0));
    }

    /// One undo entry per typing gesture, like the studios' one-per-property-edit rule.
    #[test]
    fn test_script_dialog_commits_one_entry_per_typing_gesture() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.editing_rule = Some(script_draft("Write-Host 'before'"));
        state.open_script_editor(0, crate::gui::components::script_highlighter::ScriptLanguage::PowerShell);
        open_dialog(&ctx, &mut state);

        // Three changed passes are one gesture: no entry until the typing stops.
        for (pass, text) in [(1.1, "a"), (1.2, "b"), (1.3, "c")] {
            gui_pass(&ctx, &mut state, pass, type_text(text));
            assert_eq!(dialog_history(&state), (0, 0), "no entry while the gesture is moving");
        }
        gui_pass(&ctx, &mut state, 1.4, Vec::new());
        assert_eq!(dialog_history(&state), (1, 0), "the stopped gesture is one undo entry");

        // The committed entry is the text from before the gesture, so one undo clears it.
        let typed = draft_script(&state).to_string();
        gui_pass(&ctx, &mut state, 1.5, history_key(egui::Key::Z, egui::Modifiers::CTRL));
        assert_eq!(draft_script(&state), "Write-Host 'before'");
        gui_pass(&ctx, &mut state, 1.6, history_key(egui::Key::Y, egui::Modifiers::CTRL));
        assert_eq!(draft_script(&state), typed);
        assert!(state.undo_stack.is_empty(), "the draft history never moved");
    }
}
