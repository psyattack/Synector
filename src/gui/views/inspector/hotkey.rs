//! Hotkey name mapping, combination formatting and the recording field.

use super::*;

pub(crate) fn egui_key_to_hotkey_name(key: egui::Key) -> Option<&'static str> {
    match key {
        egui::Key::A => Some("A"),
        egui::Key::B => Some("B"),
        egui::Key::C => Some("C"),
        egui::Key::D => Some("D"),
        egui::Key::E => Some("E"),
        egui::Key::F => Some("F"),
        egui::Key::G => Some("G"),
        egui::Key::H => Some("H"),
        egui::Key::I => Some("I"),
        egui::Key::J => Some("J"),
        egui::Key::K => Some("K"),
        egui::Key::L => Some("L"),
        egui::Key::M => Some("M"),
        egui::Key::N => Some("N"),
        egui::Key::O => Some("O"),
        egui::Key::P => Some("P"),
        egui::Key::Q => Some("Q"),
        egui::Key::R => Some("R"),
        egui::Key::S => Some("S"),
        egui::Key::T => Some("T"),
        egui::Key::U => Some("U"),
        egui::Key::V => Some("V"),
        egui::Key::W => Some("W"),
        egui::Key::X => Some("X"),
        egui::Key::Y => Some("Y"),
        egui::Key::Z => Some("Z"),
        egui::Key::Num0 => Some("0"),
        egui::Key::Num1 => Some("1"),
        egui::Key::Num2 => Some("2"),
        egui::Key::Num3 => Some("3"),
        egui::Key::Num4 => Some("4"),
        egui::Key::Num5 => Some("5"),
        egui::Key::Num6 => Some("6"),
        egui::Key::Num7 => Some("7"),
        egui::Key::Num8 => Some("8"),
        egui::Key::Num9 => Some("9"),
        egui::Key::F1 => Some("F1"),
        egui::Key::F2 => Some("F2"),
        egui::Key::F3 => Some("F3"),
        egui::Key::F4 => Some("F4"),
        egui::Key::F5 => Some("F5"),
        egui::Key::F6 => Some("F6"),
        egui::Key::F7 => Some("F7"),
        egui::Key::F8 => Some("F8"),
        egui::Key::F9 => Some("F9"),
        egui::Key::F10 => Some("F10"),
        egui::Key::F11 => Some("F11"),
        egui::Key::F12 => Some("F12"),
        egui::Key::Enter => Some("Enter"),
        egui::Key::Tab => Some("Tab"),
        egui::Key::Space => Some("Space"),
        egui::Key::Backspace => Some("Backspace"),
        egui::Key::Delete => Some("Delete"),
        egui::Key::Insert => Some("Insert"),
        egui::Key::Home => Some("Home"),
        egui::Key::End => Some("End"),
        egui::Key::PageUp => Some("PageUp"),
        egui::Key::PageDown => Some("PageDown"),
        egui::Key::ArrowUp => Some("Up"),
        egui::Key::ArrowDown => Some("Down"),
        egui::Key::ArrowLeft => Some("Left"),
        egui::Key::ArrowRight => Some("Right"),
        _ => None,
    }
}

pub(crate) fn format_hotkey_combination(key: egui::Key, modifiers: egui::Modifiers) -> Option<String> {
    let key_name = egui_key_to_hotkey_name(key)?;
    let mut parts = Vec::new();
    if modifiers.ctrl || modifiers.command {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if modifiers.mac_cmd {
        parts.push("Win");
    }
    parts.push(key_name);
    Some(parts.join("+"))
}

/// Recording field and hold-mode toggles of the global-hotkey trigger.
pub(crate) fn render_hotkey_trigger_fields(
    ui: &mut Ui,
    key_combination: &mut String,
    hold_mode: &mut bool,
    repeat_while_held: &mut bool,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    widgets::hud(ui, loc.inspector_field_hotkey_combo, t.muted);
    let rec_id = ui.scope_id().with("rec_global_hotkey");
            let mut is_recording: bool = ui.data_mut(|d| d.get_temp(rec_id).unwrap_or(false));

            if is_recording {
                ui.ctx().input(|i| {
                    for event in &i.raw.events {
                        match event {
                            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                                if *key == egui::Key::Escape {
                                    is_recording = false;
                                    break;
                                }
                                if let Some(combo) = format_hotkey_combination(*key, *modifiers) {
                                    *key_combination = combo;
                                    is_recording = false;
                                    break;
                                }
                            }
                            egui::Event::PointerButton { button, pressed: true, modifiers, .. } => {
                                let btn_str = match button {
                                    egui::PointerButton::Primary => "LButton",
                                    egui::PointerButton::Secondary => "RButton",
                                    egui::PointerButton::Middle => "MButton",
                                    egui::PointerButton::Extra1 => "M4",
                                    egui::PointerButton::Extra2 => "M5",
                                };
                                let mut parts = Vec::new();
                                if modifiers.ctrl || modifiers.command { parts.push("Ctrl"); }
                                if modifiers.alt { parts.push("Alt"); }
                                if modifiers.shift { parts.push("Shift"); }
                                parts.push(btn_str);
                                *key_combination = parts.join("+");
                                is_recording = false;
                                break;
                            }
                            _ => {}
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(rec_id, is_recording));
                ui.ctx().request_repaint();
            }

            ui.horizontal(|ui| {
                ui.set_width(ui.available_width());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let rec_label = if is_recording { loc.inspector_hotkey_listening } else { loc.inspector_hotkey_record };
                    let rec_kind = if is_recording { Kind::Primary } else { Kind::Secondary };
                    if widgets::button(ui, rec_label, rec_kind, 0.0)
                        .on_hover_text(loc.inspector_hotkey_record_tip)
                        .clicked()
                    {
                        is_recording = !is_recording;
                        ui.data_mut(|d| d.insert_temp(rec_id, is_recording));
                    }
                    ui.add(
                        egui::TextEdit::singleline(key_combination)
                            .hint_text(loc.inspector_hint_hotkey_combo)
                            .desired_width(ui.available_width())
                            .min_size(vec2(0.0, theme::CONTROL_H))
                            .align(egui::Align2::LEFT_CENTER)
                            .margin(vec2(10.0, 8.0)),
                    );
                });
            });

            ui.add_space(4.0);
            widgets::checkbox_group(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(hold_mode, loc.inspector_hotkey_hold_mode)
                        .on_hover_text(loc.inspector_hotkey_hold_mode_tip);
                    if *hold_mode {
                        ui.checkbox(repeat_while_held, loc.inspector_hotkey_repeat_while_held)
                            .on_hover_text(loc.inspector_hotkey_repeat_while_held_tip);
                    } else {
                        ui.checkbox(repeat_while_held, loc.inspector_hotkey_repeat_in_loop)
                            .on_hover_text(loc.inspector_hotkey_repeat_in_loop_tip);
                    }
                });
            });
}
