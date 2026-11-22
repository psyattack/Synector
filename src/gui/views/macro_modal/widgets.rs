use egui::{vec2, Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui};
use crate::gui::components::widgets::{self, Cell, Kind};
use crate::gui::theme::{self, Tokens};
use crate::models::MouseButton;

#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, MESSAGEBOX_STYLE};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_RETURN, VK_SPACE};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Debug::MessageBeep;

/// Borderless ghost icon button for delete / clone in timeline rows.
/// Completely eliminates the clunky box outlines on every step.
pub(super) fn ghost_icon_button(ui: &mut Ui, glyph: widgets::Glyph, tip: &str, is_danger: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let size = 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered();

    let p = ui.painter();
    if hovered {
        let bg_hover = if is_danger {
            t.bad.gamma_multiply(0.18)
        } else {
            t.accent_ink.gamma_multiply(0.12)
        };
        p.rect_filled(rect, CornerRadius::same(2), bg_hover);
    }

    let glyph_color = if hovered {
        if is_danger { t.bad } else { t.accent_ink }
    } else {
        t.faint
    };

    let m = rect.center();
    match glyph {
        widgets::Glyph::Close => {
            let s = 3.5;
            p.line_segment([m + vec2(-s, -s), m + vec2(s, s)], Stroke::new(1.3, glyph_color));
            p.line_segment([m + vec2(-s, s), m + vec2(s, -s)], Stroke::new(1.3, glyph_color));
        }
        widgets::Glyph::Clone => {
            let back = Rect::from_center_size(m + vec2(-1.8, -1.8), vec2(6.5, 6.5));
            let front = Rect::from_center_size(m + vec2(1.8, 1.8), vec2(6.5, 6.5));
            p.rect_stroke(back, CornerRadius::ZERO, Stroke::new(1.0, glyph_color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            p.rect_filled(front, CornerRadius::ZERO, if hovered { t.bg } else { t.sunken });
            p.rect_stroke(front, CornerRadius::ZERO, Stroke::new(1.0, glyph_color), egui::StrokeKind::Inside);
        }
        _ => {}
    }

    resp.on_hover_text(tip)
}

pub(super) fn render_chip(ui: &mut Ui, label: &str, fg: Color32, bg: Color32) {
    let chip_h = 19.0;
    let text_w = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::new(9.5, theme::mono_bold()), fg).size().x;
    let (rect, _) = ui.allocate_exact_size(vec2(text_w + 12.0, chip_h), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, fg.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(9.5, theme::mono_bold()),
        fg,
    );
}

pub(super) fn render_clickable_chip(ui: &mut Ui, label: &str, fg: Color32, bg: Color32, tooltip: &str) -> bool {
    let chip_h = 19.0;
    let text_w = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::new(9.5, theme::mono_bold()), fg).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(text_w + 12.0, chip_h), Sense::click());
    let hovered = resp.hovered();

    let border_color = if hovered { fg } else { fg.gamma_multiply(0.7) };
    let bg_color = if hovered { bg.blend(fg.gamma_multiply(0.20)) } else { bg };

    ui.painter().rect_filled(rect, CornerRadius::same(2), bg_color);
    ui.painter().rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(9.5, theme::mono_bold()),
        fg,
    );
    if !tooltip.is_empty() {
        resp.clone().on_hover_text(tooltip);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

pub(super) fn render_button_cells(ui: &mut Ui, button: &mut MouseButton, loc: &crate::i18n::Strings) {
    let btn_options = [
        (MouseButton::Left, Cell::new(loc.macro_studio_btn_lmb, loc.macro_studio_btn_lmb_tip)),
        (MouseButton::Right, Cell::new(loc.macro_studio_btn_rmb, loc.macro_studio_btn_rmb_tip)),
        (MouseButton::Middle, Cell::new(loc.macro_studio_btn_mmb, loc.macro_studio_btn_mmb_tip)),
        (MouseButton::XButton1, Cell::new(loc.macro_studio_btn_m4, loc.macro_studio_btn_m4_tip)),
        (MouseButton::XButton2, Cell::new(loc.macro_studio_btn_m5, loc.macro_studio_btn_m5_tip)),
    ];
    render_compact_cells(ui, button, &btn_options);
}

pub(super) fn render_compact_cells<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, Cell<'_>)]) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let mut rects = Vec::with_capacity(options.len());
    let h = 20.0;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (opt, cell) in options {
            let on = *value == *opt;
            let galley = ui.painter().layout_no_wrap(cell.label.to_string(), egui::FontId::new(10.0, theme::mono_bold()), t.ink);
            let w = galley.size().x + 14.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            let hovered = resp.hovered();

            let (fill, ink) = match (on, hovered) {
                (true, _) => (t.accent_ink, t.bg),
                (false, true) => (t.sunken, t.ink_strong),
                (false, false) => (Color32::TRANSPARENT, t.muted),
            };

            ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
            let text_rect = rect.shrink2(vec2(6.0, 0.0));
            widgets::paint_left(ui, text_rect, galley, ink);
            let resp = if !cell.tip.trim().is_empty() {
                resp.on_hover_text(cell.tip)
            } else {
                resp
            };
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                *value = *opt;
                changed = true;
            }
            rects.push(rect);
        }
    });

    if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
        let union = first.union(*last);
        ui.painter().rect_stroke(union, CornerRadius::same(2), t.hairline(), egui::StrokeKind::Inside);
    }

    changed
}

pub(super) fn render_compact_coord_picker(
    ui: &mut Ui,
    picker_id: Id,
    x: &mut Option<i32>,
    y: &mut Option<i32>,
    loc: &crate::i18n::Strings,
) {
    #[cfg(windows)]
    {
        let is_picking: bool = ui.data_mut(|d| d.get_temp(picker_id).unwrap_or(false));
        if is_picking {
            ui.ctx().request_repaint();
            let mut pt = windows::Win32::Foundation::POINT::default();
            let _ = unsafe { GetCursorPos(&mut pt) };

            let hit_key = unsafe {
                GetAsyncKeyState(VK_SPACE.0 as i32) < 0 || GetAsyncKeyState(VK_RETURN.0 as i32) < 0
            };
            let cancel = unsafe {
                GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0
            };

            if cancel {
                ui.data_mut(|d| d.insert_temp(picker_id, false));
            } else if hit_key {
                *x = Some(pt.x);
                *y = Some(pt.y);
                ui.data_mut(|d| d.insert_temp(picker_id, false));
                unsafe {
                    let _ = MessageBeep(MESSAGEBOX_STYLE(0));
                }
            } else {
                let label = loc.macro_studio_space_capture_prefix.replacen("{}", &pt.x.to_string(), 1).replacen("{}", &pt.y.to_string(), 1);
                if widgets::compact_button(ui, &label, Kind::Primary, 18.0)
                    .on_hover_text(loc.macro_studio_space_capture_tip)
                    .clicked()
                {
                    *x = Some(pt.x);
                    *y = Some(pt.y);
                    ui.data_mut(|d| d.insert_temp(picker_id, false));
                }
            }
        } else {
            if widgets::compact_button(ui, loc.macro_studio_pick_btn, Kind::Secondary, 18.0)
                .on_hover_text(loc.macro_studio_pick_btn_tip)
                .clicked()
            {
                ui.data_mut(|d| d.insert_temp(picker_id, true));
            }
        }
    }
}
