use egui::{pos2, Rect, Sense, Stroke, Ui, vec2};
use crate::gui::components::chips::{paint_tag_pill, tag_pill_width, TagPillStyle};
use crate::gui::theme::{self, Tokens};

pub(super) fn measure_tags_width(ui: &egui::Ui, tags: &[crate::models::RuleTag]) -> f32 {
    let mut total_w = 0.0;
    let more_badge_w = 22.0;
    for (i, tag) in tags.iter().enumerate() {
        if i >= 3 {
            total_w += more_badge_w + 3.0;
            break;
        }
        total_w += tag_pill_width(ui, tag, &TagPillStyle::CARD) + 3.0;
    }
    total_w
}

pub(super) fn render_card_tags_at(
    ui: &mut Ui,
    tags: &[crate::models::RuleTag],
    start_x: f32,
    top_y: f32,
    max_w: f32,
    rule_id: &str,
    t: &Tokens,
) {
    let style = &TagPillStyle::CARD;

    let mut curr_x = start_x;
    let pill_h = style.height;
    let total_tags = tags.len();
    let mut shown_count = 0;

    for (i, tag) in tags.iter().enumerate() {
        let tag_color = theme::tag_color(tag, t.accent_ink);
        let pill_w = tag_pill_width(ui, tag, style);
        let more_badge_w = 22.0;

        let has_more = i + 1 < total_tags;
        let needed_w = if has_more { pill_w + 3.0 + more_badge_w } else { pill_w };

        if curr_x - start_x + needed_w > max_w && shown_count > 0 {
            break;
        }

        let pill_rect = Rect::from_min_size(pos2(curr_x, top_y), vec2(pill_w, pill_h));
        let _resp = ui.interact(pill_rect, ui.scope_id().with(("card_tag", rule_id, i)), Sense::hover());

        paint_tag_pill(ui, pill_rect, tag, style, tag_color, tag_color, tag_color, false);

        // Tautological tooltip removed: tag.name is already fully legible on the pill

        curr_x += pill_w + 3.0;
        shown_count += 1;
    }

    let hidden_count = total_tags.saturating_sub(shown_count);
    if hidden_count > 0 {
        let more_text = format!("+{}", hidden_count);
        let more_galley = ui.painter().layout_no_wrap(more_text, egui::FontId::new(8.5, theme::mono()), t.muted);
        let more_w = more_galley.size().x + 8.0;
        let more_rect = Rect::from_min_size(pos2(curr_x, top_y), vec2(more_w, pill_h));
        let more_resp = ui.interact(more_rect, ui.scope_id().with(("card_tag_more", rule_id)), Sense::hover());

        ui.painter().rect(
            more_rect,
            t.chip_corner(),
            t.sunken,
            Stroke::new(0.8, t.line),
            egui::StrokeKind::Inside,
        );
        let more_pos = pos2(more_rect.center().x - more_galley.size().x / 2.0, more_rect.center().y - more_galley.size().y / 2.0);
        ui.painter().galley(more_pos, more_galley, t.muted);

        let hidden_names: Vec<&str> = tags.iter().skip(shown_count).map(|t| t.name.as_str()).collect();
        more_resp.on_hover_text(hidden_names.join(", "));
    }
}
