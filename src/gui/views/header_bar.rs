use egui::{CornerRadius, Rect, Ui, pos2, vec2};
use crate::gui::components::widgets;
use crate::gui::state::{ActiveView, GuiState};
use crate::gui::theme::{self, Tokens};

pub fn render_header_bar(ui: &mut Ui, state: &mut GuiState) {
    let t = Tokens::get(ui.ctx());
    let h = t.metrics().header_h;
    let full_rect = ui.max_rect();

    let loc = state.loc();

    // 1. Custom window title bar drag & double-click maximize
    widgets::title_bar_drag(ui);

    // 2. Left: Brand Wordmark
    let left_rect = Rect::from_min_size(full_rect.min, vec2(260.0, h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(16.0);
            widgets::wordmark(ui, "SYN", "ECTOR", 16.0, h);
        });
    });

    // 3. Right: Window Caption Buttons (no sync/settings icons, no extra dividers)
    let right_w = 140.0;
    let right_rect = Rect::from_min_size(pos2(full_rect.right() - right_w, full_rect.top()), vec2(right_w, h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            widgets::window_buttons(ui, h, loc);
        });
    });

    // 4. Center: Navigation Tabs (strictly centered in the middle of the window)
    let tabs = [
        (ActiveView::Rules, loc.tab_automations),
        (ActiveView::Analytics, loc.tab_analytics),
        (ActiveView::Settings, loc.tab_settings),
    ];

    let galleys: Vec<_> = tabs
        .iter()
        .map(|&(view, label)| {
            let galley = widgets::hud_galley(ui, label, t.muted);
            let w = galley.size().x + 24.0;
            (view, galley, w)
        })
        .collect();

    let total_tabs_w: f32 = galleys.iter().map(|(_, _, w)| *w).sum();
    let center_x = full_rect.center().x;
    let mut current_x = (center_x - total_tabs_w / 2.0).round();
    let modern = t.metrics().skin == crate::config::UiSkin::Modern;

    // One sliding pill marks the active tab in the modern generation. Its rect is animated in
    // egui's temporary data, so the tab switch reads as movement instead of a jump.
    let mut pill_target = None;
    for (view, galley, w) in &galleys {
        let tab_rect = Rect::from_min_size(pos2(current_x, full_rect.top()), vec2(*w, h));
        current_x += *w;
        if *view == state.active_view {
            pill_target = Some(Rect::from_center_size(tab_rect.center(), galley.size() + vec2(24.0, 10.0)));
        }
    }
    if modern {
        if let Some(target) = pill_target {
            let pill = theme::motion::slide_rect(ui.ctx(), egui::Id::unique("header-tab-pill"), target);
            ui.painter().rect_filled(
                pill,
                CornerRadius::same((pill.height() * 0.5) as u8),
                t.raised(),
            );
        }
    }

    current_x = (center_x - total_tabs_w / 2.0).round();
    for (view, galley, w) in &galleys {
        let view = *view;
        let galley = galley.clone();
        let tab_rect = Rect::from_min_size(pos2(current_x, full_rect.top()), vec2(*w, h));
        current_x += *w;

        let resp = ui.interact(tab_rect, ui.scope_id().with(view as usize), egui::Sense::click());
        if resp.clicked() {
            if view != state.active_view {
                if let Some(rule) = state.editing_rule.take() {
                    state.builder_draft = Some(rule);
                }
                state.selected_process = None;
                if view == ActiveView::Rules && (state.is_creating_new_rule || state.selected_rule_id.is_some()) {
                    if let Some(draft) = state.builder_draft.take() {
                        state.editing_rule = Some(draft);
                    }
                }
            }
            state.active_view = view;
        }
        let on = state.active_view == view || resp.hovered();
        if on && !modern {
            let span = Rect::from_center_size(tab_rect.center(), galley.size() + vec2(14.0, 8.0));
            ui.painter().rect_filled(span, CornerRadius::ZERO, t.invert_bg);
        }
        let active_ink = if modern { t.ink_strong } else { t.invert_fg };
        widgets::paint_centred(ui, tab_rect, galley, if on { active_ink } else { t.muted });
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
}
