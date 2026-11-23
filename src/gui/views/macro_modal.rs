//! Macro Studio modal: the shell (capture pump, close gates and enabling the recorder's global
//! studio hotkeys) plus the extracted toolbar, timeline, footer, preset-library dialog and row
//! widgets.

use eframe::emath::vec2;
use egui::RichText;
use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::widgets::{self as gui_widgets, Kind};
use crate::gui::macro_recorder::{egui_key_to_string, run_macro_test_safe, CaptureChannel, MacroRecorder};
use crate::gui::state::GuiState;
use crate::gui::theme::{self, Tokens};
use crate::models::Action;

mod footer;
mod presets;
mod timeline;
mod toolbar;
mod widgets;

pub use presets::render_macro_presets_modal;

pub fn render_macro_modal(ctx: &egui::Context, state: &mut GuiState) {
    let is_open = state.macro_editor.is_some();

    let recorder = MacroRecorder::global();
    let is_recording = recorder.is_active();
    if is_open {
        crate::gui::macro_recorder::set_studio_active(true);
        if is_recording || crate::gui::macro_recorder::is_macro_test_running() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    if is_recording {
        // Direct capture of keyboard events in GUI event loop (ensures immediate capture if modal has focus)
        if recorder.capture(CaptureChannel::Keyboard).load(std::sync::atomic::Ordering::SeqCst) {
            let key_events: Vec<(String, bool)> = ctx.input(|i| {
                let mut list = Vec::new();
                for ev in &i.raw.events {
                    if let egui::Event::Key { key, pressed, repeat, .. } = ev {
                        let k_str = egui_key_to_string(*key);
                        if !repeat && !k_str.is_empty() && *key != egui::Key::F9 && *key != egui::Key::Escape && *key != egui::Key::F10 {
                            list.push((k_str.to_string(), *pressed));
                        }
                    }
                }
                list
            });

            for (k, pressed) in key_events {
                recorder.record_key(&k, pressed);
            }
        }
    }

    // Stop running test on Escape key
    if is_open && crate::gui::macro_recorder::is_macro_test_running() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        crate::gui::macro_recorder::stop_macro_test();
    }

    // Target action resolution
    let loc = state.loc();
    let editor_state = state.macro_editor.clone().or_else(|| state.last_macro_editor.clone());
    let Some(edit_info) = editor_state else { return; };

    let editing_rule = state.editing_rule.as_mut();
    let Some(rule) = editing_rule else { return; };

    let can_toggle_infinite = matches!(rule.trigger, Some(crate::models::Trigger::GlobalHotkey { .. }));

    let action_list = if edit_info.is_exit_pipeline {
        &mut rule.on_exit_actions
    } else {
        &mut rule.actions
    };

    let Some(action) = action_list.get_mut(edit_info.step_idx) else { return; };

    if !matches!(action, Action::Macro { .. }) {
        *action = Action::Macro {
            preset_id: None,
            operation: crate::models::MacroOperation::Run,
            events: Vec::new(),
            repeat: 1,
            speed: 1.0,
            infinite: false,
        };
    }

    let Action::Macro { events, repeat, speed, infinite, .. } = action else { return; };

    // Real-time live draining: any newly captured events stream directly into the list
    let new_events = recorder.drain_events();
    if !new_events.is_empty() {
        events.extend(new_events);
    }
    if let Some(bg_events) = crate::gui::macro_recorder::take_studio_drained_events() {
        events.extend(bg_events);
    }

    // Sync with studio payload for background hotkeys
    crate::gui::macro_recorder::update_studio_payload(events.clone(), *speed);

    let rule_name = rule.name.trim().to_string();
    let t = Tokens::get(ctx);
    let mut group_full_presses = state.group_full_presses;

    let modal_res = Modal::new("macro-modal", is_open)
        .width_fraction(0.88)
        .height_fraction(0.84)
        .min_size(vec2(920.0, 560.0))
        .max_size(vec2(1180.0, 800.0))
        .header_height(46.0)
        .toolbar_height(48.0)
        .footer_height(46.0)
        .header_sunken(true)
        .footer_sunken(true)
        .close_on_escape(!is_recording && !crate::gui::macro_recorder::is_macro_test_running())
        .close_on_scrim(!is_recording)
        .show(ctx, &t, |frame| {
            // --- HEADER ---
            frame.header(|ui, close_req| {
                render_modal_header(
                    ui,
                    close_req,
                    &t,
                    ModalHeader {
                        glyph: Some(egui_phosphor::regular::GAME_CONTROLLER),
                        glyph_size: 18.0,
                        close_tip: loc.macro_studio_close_tip,
                        left_pad: 16.0,
                        glyph_pad: 8.0,
                        right_pad: 14.0,
                        item_spacing_x: None,
                    },
                    |ui| {
                        ui.label(
                            RichText::new(loc.macro_studio_title)
                                .font(egui::FontId::new(14.0, theme::semibold()))
                                .color(t.ink_strong),
                        );
                    },
                    |ui| {
                        if gui_widgets::glyph_button_quiet(ui, gui_widgets::Glyph::Minimize, loc.macro_studio_minimize_tip).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }

                        // Presets button
                        ui.add_space(8.0);
                        let presets_kind = if state.show_macro_presets_modal { Kind::Primary } else { Kind::Secondary };
                        if gui_widgets::button(ui, loc.macro_studio_presets_btn, presets_kind, 0.0).clicked() {
                            state.show_macro_presets_modal = !state.show_macro_presets_modal;
                            if state.show_macro_presets_modal && state.macro_preset_save_name.trim().is_empty() && !rule_name.is_empty() {
                                state.macro_preset_save_name = rule_name.clone();
                            }
                        }

                        // Test run button (F10) - toggle start / stop on safe background runner
                        let is_testing = crate::gui::macro_recorder::is_macro_test_running();
                        let test_lbl = if is_testing {
                            loc.macro_studio_btn_stop_test
                        } else {
                            loc.macro_studio_btn_test_run
                        };
                        let test_kind = if is_testing { Kind::Danger } else { Kind::Secondary };
                        let btn_w = gui_widgets::button_width(ui, test_lbl, 0.0);
                        let test_clicked = gui_widgets::button(ui, test_lbl, test_kind, btn_w)
                            .on_hover_text(if is_testing {
                                loc.macro_studio_btn_stop_test_tip
                            } else {
                                loc.macro_studio_btn_test_run_tip
                            })
                            .clicked();

                        #[cfg(windows)]
                        let f10_hit = false;

                        if test_clicked || f10_hit {
                            if is_testing {
                                crate::gui::macro_recorder::stop_macro_test();
                            } else {
                                run_macro_test_safe(events.clone(), *speed);
                            }
                        }
                    },
                );
            });

            // --- TOOLBAR: CLEAN, REORGANIZED WITH DISTINCT FUNCTIONAL ZONES ---
            frame.toolbar(|ui, _close_req| {
                toolbar::render_toolbar(ui, &recorder, is_recording, events, loc, &t);
            });

            // --- BODY: UNIFIED TIMELINE TABLE WITH SYMMETRIC MARGINS & ROCK-SOLID DRAG & DROP ---
            frame.body(|ui| {
                timeline::render_timeline(ui, events, group_full_presses, loc, &t);
            });

            // --- FOOTER ---
            frame.footer(|ui, _close_req| {
                footer::render_footer(ui, events, repeat, speed, infinite, can_toggle_infinite, &mut group_full_presses, loc, &t);
            });
        });

    state.group_full_presses = group_full_presses;

    if let Some(res) = modal_res {
        if res.close_requested {
            if recorder.is_active() {
                recorder.stop_recording();
                let more = recorder.drain_events();
                let editor_state = state.macro_editor.clone().or_else(|| state.last_macro_editor.clone());
                if let Some(edit_info) = editor_state {
                    if let Some(rule) = state.editing_rule.as_mut() {
                        let action_list = if edit_info.is_exit_pipeline {
                            &mut rule.on_exit_actions
                        } else {
                            &mut rule.actions
                        };
                        if let Some(Action::Macro { events, .. }) = action_list.get_mut(edit_info.step_idx) {
                            events.extend(more);
                        }
                    }
                }
            }
            crate::gui::macro_recorder::stop_macro_test();
            state.close_macro_editor();
        }
    } else if !is_open {
        crate::gui::macro_recorder::set_studio_active(false);
        state.last_macro_editor = None;
    }
}
