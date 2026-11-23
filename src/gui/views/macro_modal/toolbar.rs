use egui::{vec2, Color32, Sense, Stroke, Ui};
use crate::gui::components::widgets::{self, Kind};
use crate::gui::macro_recorder::{CaptureChannel, MacroRecorder};
use crate::gui::theme::{self, Tokens};
use crate::models::{MacroEvent, MouseButton, MouseCoordMode};

pub(super) fn render_toolbar(
    ui: &mut Ui,
    recorder: &MacroRecorder,
    is_recording: bool,
    events: &mut Vec<MacroEvent>,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    ui.horizontal_centered(|ui| {
        ui.add_space(14.0);

        // ZONE 1: RECORD BUTTON & CAPTURE TOGGLE CHIPS
        if is_recording {
            let rec_label = loc.macro_studio_btn_stop_record;
            let btn_w = widgets::button_width(ui, rec_label, 0.0);
            if widgets::button(ui, rec_label, Kind::Danger, btn_w).clicked() {
                recorder.stop_recording();
                let more = recorder.drain_events();
                events.extend(more);
                // If user stopped via GUI button with mouse, cleanly strip the trailing stop click
                if let Some(MacroEvent::MouseUp { button: MouseButton::Left }) = events.last() {
                    events.pop();
                    if let Some(MacroEvent::MouseDown { button: MouseButton::Left }) = events.last() {
                        events.pop();
                    }
                    if let Some(MacroEvent::Delay { .. }) = events.last() {
                        events.pop();
                    }
                }
            }
        } else {
            let rec_label = loc.macro_studio_btn_record;
            let btn_w = widgets::button_width(ui, rec_label, 0.0);
            if widgets::button(ui, rec_label, Kind::Primary, btn_w).clicked() {
                recorder.start();
            }
        }

        ui.add_space(8.0);

        // Filter toggle chips (compact, stylish DAW-like pills)
        render_filter_chip(ui, loc.macro_studio_chip_keys, recorder.capture(CaptureChannel::Keyboard), loc.macro_studio_chip_keys_tip, t);
        ui.add_space(2.0);
        render_filter_chip(ui, loc.macro_studio_chip_clicks, recorder.capture(CaptureChannel::MouseClicks), loc.macro_studio_chip_clicks_tip, t);
        ui.add_space(2.0);
        render_filter_chip(ui, loc.macro_studio_chip_moves, recorder.capture(CaptureChannel::MouseMoves), loc.macro_studio_chip_moves_tip, t);
        ui.add_space(2.0);
        render_filter_chip(ui, loc.macro_studio_chip_wheel, recorder.capture(CaptureChannel::MouseWheel), loc.macro_studio_chip_wheel_tip, t);
        ui.add_space(2.0);
        render_filter_chip(ui, loc.macro_studio_chip_delays, recorder.capture(CaptureChannel::Delays), loc.macro_studio_chip_delays_tip, t);

        ui.add_space(10.0);
        widgets::rule(ui, 20.0);
        ui.add_space(10.0);

        // ZONE 2: ADD STEP ACTIONS
        if widgets::compact_button(ui, loc.macro_studio_btn_add_key, Kind::Secondary, 24.0).clicked() {
            events.push(MacroEvent::KeyDown { key: "Enter".to_string() });
            events.push(MacroEvent::Delay { ms: 40, organic: false, jitter_percent: 30 });
            events.push(MacroEvent::KeyUp { key: "Enter".to_string() });
        }

        if widgets::compact_button(ui, loc.macro_studio_btn_add_mouse, Kind::Secondary, 24.0).clicked() {
            events.push(MacroEvent::MouseDown { button: MouseButton::Left });
            events.push(MacroEvent::Delay { ms: 40, organic: false, jitter_percent: 30 });
            events.push(MacroEvent::MouseUp { button: MouseButton::Left });
        }

        if widgets::compact_button(ui, loc.macro_studio_btn_add_move, Kind::Secondary, 24.0).clicked() {
            events.push(MacroEvent::MouseMove {
                x: 960,
                y: 540,
                coord_mode: MouseCoordMode::Absolute,
                smooth: true,
                duration_ms: Some(250),
                organic: false,
            });
        }

        if widgets::compact_button(ui, loc.macro_studio_btn_add_delay, Kind::Secondary, 24.0).clicked() {
            events.push(MacroEvent::Delay { ms: 100, organic: false, jitter_percent: 30 });
        }

        if widgets::compact_button(ui, loc.macro_studio_btn_add_text, Kind::Secondary, 24.0).clicked() {
            events.push(MacroEvent::Text { text: "Hello".to_string(), delay_ms: 20, organic: false });
        }

        if widgets::compact_button(ui, loc.macro_studio_btn_add_wheel, Kind::Secondary, 24.0).clicked() {
            events.push(MacroEvent::MouseWheel { delta: 120 });
        }

        // ZONE 3: UTILITIES (RIGHT-ALIGNED)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(14.0);

            if !events.is_empty() {
                let clr_lbl = loc.macro_studio_btn_clear_all;
                let btn_w = widgets::button_width(ui, clr_lbl, 0.0);
                if widgets::button(ui, clr_lbl, Kind::Secondary, btn_w).clicked() {
                    events.clear();
                }
            }
        });
    });
}

fn render_filter_chip(
    ui: &mut Ui,
    label: &str,
    state: &std::sync::atomic::AtomicBool,
    tip: &str,
    t: &Tokens,
) {
    let mut is_active = state.load(std::sync::atomic::Ordering::SeqCst);
    let chip_h = 24.0;
    let text_w = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::new(10.5, theme::semibold()), Color32::WHITE).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + 14.0, chip_h), Sense::click());

    if resp.clicked() {
        is_active = !is_active;
        state.store(is_active, std::sync::atomic::Ordering::SeqCst);
    }

    let p = ui.painter();
    let (bg, border, text_color) = if is_active {
        (t.accent_ink.gamma_multiply(0.18), t.accent_ink, t.accent_ink)
    } else {
        (t.sunken, t.hairline().color, t.muted)
    };

    p.rect_filled(rect, t.inset_corner(), bg);
    p.rect_stroke(rect, t.inset_corner(), Stroke::new(1.0, border), egui::StrokeKind::Inside);
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(10.5, theme::semibold()),
        text_color,
    );

    resp.on_hover_text(tip);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_trailing_gui_stop_click() {
        let mut events = vec![
            MacroEvent::KeyDown { key: "A".to_string() },
            MacroEvent::KeyUp { key: "A".to_string() },
            MacroEvent::Delay { ms: 500, organic: false, jitter_percent: 30 },
            MacroEvent::MouseDown { button: MouseButton::Left },
            MacroEvent::MouseUp { button: MouseButton::Left },
        ];

        // Simulate logic on GUI Stop button click
        if let Some(MacroEvent::MouseUp { button: MouseButton::Left }) = events.last() {
            events.pop();
            if let Some(MacroEvent::MouseDown { button: MouseButton::Left }) = events.last() {
                events.pop();
            }
            if let Some(MacroEvent::Delay { .. }) = events.last() {
                events.pop();
            }
        }

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], MacroEvent::KeyDown { .. }));
        assert!(matches!(events[1], MacroEvent::KeyUp { .. }));
    }
}
