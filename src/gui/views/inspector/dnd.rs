//! Index-based drag-and-drop engine pieces shared by the action pipelines.

use super::*;
use crate::gui::components::dnd;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DndDragSession {
    pub(crate) source_idx: usize,
    pub(crate) target_idx: usize,
    pub(crate) card_width: f32,
}

/// Action to render as the floating drag preview.
///
/// `source_idx` was captured in an earlier frame; an undo/redo can replace the draft with a
/// shorter snapshot while the session is still alive, so the index is resolved with `get`.
/// A session that no longer fits is dropped instead of panicking on the stale index.
pub(crate) fn dnd_preview_action<'a>(
    actions: &'a [Action],
    session: &mut Option<DndDragSession>,
) -> Option<&'a Action> {
    let source_idx = session.as_ref()?.source_idx;
    let action = actions.get(source_idx);
    if action.is_none() {
        *session = None;
    }
    action
}

/// Inspector pipeline drop line: 22 px, 0.18 s, rail inset 24/8 and a haloed 12 px bead. The
/// corner follows the skin, so a rounded generation gets a capsule instead of a rectangle.
fn drop_line_style(t: &Tokens) -> dnd::DropLineStyle {
    dnd::DropLineStyle {
        height: 22.0,
        anim_secs: 0.18,
        corner: t.chip_corner(),
        fill_alpha: 0.08,
        line_width: 1.8,
        line_left: 24.0,
        line_right: 8.0,
        bead: Some(dnd::DropBead {
            left: 12.0,
            radius: 3.5,
            halo_radius: Some(5.5),
        }),
    }
}

/// Floating preview chrome: an elevated card with a 1.5 px accent stroke.
fn preview_style(t: &Tokens) -> dnd::FloatingPreviewStyle {
    dnd::FloatingPreviewStyle {
        corner: t.inset_corner(),
        stroke_width: 1.5,
        shadow: true,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_drop_slot(
    ui: &mut Ui,
    _loc: &crate::i18n::Strings,
    slot_idx: usize,
    active_dst: Option<usize>,
    active_src: Option<usize>,
    pipeline_id: &str,
    prefix: &str,
    t: &Tokens,
) {
    let is_targeted = if let (Some(dst), Some(src)) = (active_dst, active_src) {
        dst == slot_idx && slot_idx != src && slot_idx != src + 1
    } else {
        false
    };

    let anim_id = ui.scope_id().with(("slot_anim", pipeline_id, slot_idx));
    let Some((rect, anim)) = dnd::drop_line(ui, anim_id, is_targeted, &drop_line_style(t), t) else {
        return;
    };

    // Step number label at the right of the drop line
    let step_num = if let Some(src) = active_src {
        if slot_idx > src {
            slot_idx
        } else {
            slot_idx + 1
        }
    } else {
        slot_idx + 1
    };
    let label = format!("{} {:02}", prefix, step_num);
    let text_color = t.accent_ink.gamma_multiply(anim);
    let galley = widgets::caps(ui, &label, 9.0, theme::mono_bold(), 1.0, text_color);
    let text_pos = pos2(rect.right() - galley.size().x - 12.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley_with_override_text_color(text_pos, galley, text_color);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_floating_drag_preview(
    ui: &Ui,
    loc: &crate::i18n::Strings,
    action: &Action,
    session: DndDragSession,
    pointer_pos: Option<Pos2>,
    prefix: &str,
    pipeline_id: &str,
    t: &Tokens,
) {
    let Some(pointer) = pointer_pos else { return };

    let card_w = session.card_width.clamp(280.0, 520.0);
    let card_h = 56.0;
    let preview_pos = pos2(
        pointer.x - card_w * 0.5,
        pointer.y - card_h * 0.5,
    );
    let preview_rect = Rect::from_min_size(preview_pos, vec2(card_w, card_h));

    dnd::floating_preview(
        ui,
        Id::unique(("dnd_float_preview", pipeline_id)),
        preview_rect,
        &preview_style(t),
        t,
        |painter| {
            // Top accent stripe
            painter.hline(preview_rect.x_range(), preview_rect.top() + 1.0, Stroke::new(2.0, t.accent_ink));

            // Grip dots on the left of floating card
            let grip_m = pos2(preview_rect.left() + 14.0, preview_rect.center().y);
            for x in [-2.8, 2.8] {
                for y in [-4.0, 0.0, 4.0] {
                    painter.circle_filled(grip_m + vec2(x, y), 1.2, t.accent_ink);
                }
            }

            // Step indicator: "STEP 01"
            let step_txt = format!("{} {:02}", prefix, session.source_idx + 1);
            let step_galley = widgets::caps(ui, &step_txt, 11.0, theme::mono_bold(), 1.0, t.accent_ink);
            let step_w = step_galley.size().x;
            painter.galley_with_override_text_color(
                pos2(preview_rect.left() + 26.0, preview_rect.top() + 8.0),
                step_galley,
                t.accent_ink,
            );

            // Type badge
            let type_badge = action_type_badge(action, loc);
            let badge_galley = widgets::caps(ui, type_badge, 10.0, theme::mono(), 1.0, t.ink_strong);
            let badge_rect = Rect::from_min_size(
                pos2(preview_rect.left() + 26.0 + step_w + 8.0, preview_rect.top() + 7.0),
                vec2(badge_galley.size().x + 10.0, 18.0),
            );
            painter.rect_filled(badge_rect, CornerRadius::ZERO, t.sunken);
            painter.rect_stroke(badge_rect, CornerRadius::ZERO, t.hairline(), egui::StrokeKind::Inside);
            painter.galley_with_override_text_color(
                badge_rect.center() - badge_galley.size() * 0.5,
                badge_galley,
                t.ink_strong,
            );

            // Live destination indicator on far right: "➜ STEP 02"
            let dest_num = if session.target_idx > session.source_idx {
                session.target_idx
            } else {
                session.target_idx + 1
            };
            let dest_txt = format!("➜ {} {:02}", prefix, dest_num);
            let dest_galley = widgets::caps(ui, &dest_txt, 10.0, theme::mono_bold(), 1.0, t.accent_ink);
            painter.galley_with_override_text_color(
                pos2(preview_rect.right() - dest_galley.size().x - 12.0, preview_rect.top() + 9.0),
                dest_galley,
                t.accent_ink,
            );

            // Action summary line
            let summary = action_summary_preview(action, loc);
            let summary_galley = ui.painter().layout_no_wrap(
                summary,
                egui::FontId::new(11.0, theme::mono()),
                t.muted,
            );
            painter.galley_with_override_text_color(
                pos2(preview_rect.left() + 26.0, preview_rect.top() + 32.0),
                summary_galley,
                t.muted,
            );
        },
    );
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drag_preview_drops_a_stale_source_index() {
        let actions = vec![
            Action::Notification { title: "a".into(), message: String::new() },
            Action::Notification { title: "b".into(), message: String::new() },
        ];

        // An undo can replace the draft with a shorter snapshot while the drag session is
        // still alive in egui's temporary data. The stale index must drop the session, not
        // index past the end of the vector.
        let mut stale = Some(DndDragSession { source_idx: 5, target_idx: 5, card_width: 300.0 });
        assert!(dnd_preview_action(&actions, &mut stale).is_none());
        assert!(stale.is_none(), "the stale session is dropped");

        let mut valid = Some(DndDragSession { source_idx: 1, target_idx: 0, card_width: 300.0 });
        assert!(dnd_preview_action(&actions, &mut valid).is_some());
        assert!(valid.is_some(), "a fitting session survives");
    }
}
