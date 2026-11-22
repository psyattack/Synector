use egui::{pos2, vec2, CornerRadius, RichText, Sense, Stroke, Ui};
use crate::gui::components::dnd;
use crate::gui::components::widgets;
use crate::gui::theme::{self, Tokens};
use crate::models::{MacroEvent, MouseButton, MouseCoordMode};
use super::widgets::{
    ghost_icon_button, render_button_cells, render_chip, render_clickable_chip,
    render_compact_coord_picker,
};

#[derive(Clone, Debug, PartialEq)]
enum TimelineItem {
    Single(usize),
    MoveGroup {
        start_idx: usize,
        count: usize,
    },
    MoveGroupExpandedHead {
        start_idx: usize,
        count: usize,
    },
    FullKeyPress {
        start_idx: usize,
        count: usize,
    },
    FullMouseClick {
        start_idx: usize,
        count: usize,
    },
}

fn partition_timeline_items(
    events: &[MacroEvent],
    group_presses: bool,
    expanded_groups: &std::collections::HashSet<usize>,
) -> Vec<TimelineItem> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < events.len() {
        if matches!(events[i], MacroEvent::MouseMove { .. }) {
            let start = i;
            let mut count = 1;
            while i + 1 < events.len() && matches!(events[i + 1], MacroEvent::MouseMove { .. }) {
                count += 1;
                i += 1;
            }
            if count >= 2 {
                if expanded_groups.contains(&start) {
                    items.push(TimelineItem::MoveGroupExpandedHead { start_idx: start, count });
                    for sub in (start + 1)..(start + count) {
                        items.push(TimelineItem::Single(sub));
                    }
                } else {
                    items.push(TimelineItem::MoveGroup { start_idx: start, count });
                }
            } else {
                items.push(TimelineItem::Single(start));
            }
            i += 1;
        } else if group_presses {
            // Check for KeyDown + [Delay] + KeyUp with matching key
            if let MacroEvent::KeyDown { key: ref k_down } = events[i] {
                if i + 2 < events.len() {
                    if let (MacroEvent::Delay { .. }, MacroEvent::KeyUp { key: ref k_up }) = (&events[i + 1], &events[i + 2]) {
                        if k_down == k_up {
                            items.push(TimelineItem::FullKeyPress { start_idx: i, count: 3 });
                            i += 3;
                            continue;
                        }
                    }
                }
                if i + 1 < events.len() {
                    if let MacroEvent::KeyUp { key: ref k_up } = &events[i + 1] {
                        if k_down == k_up {
                            items.push(TimelineItem::FullKeyPress { start_idx: i, count: 2 });
                            i += 2;
                            continue;
                        }
                    }
                }
                items.push(TimelineItem::Single(i));
                i += 1;
            } else if let MacroEvent::MouseDown { button: ref b_down } = events[i] {
                // Check for MouseDown + [Delay] + MouseUp with matching button
                if i + 2 < events.len() {
                    if let (MacroEvent::Delay { .. }, MacroEvent::MouseUp { button: ref b_up }) = (&events[i + 1], &events[i + 2]) {
                        if b_down == b_up {
                            items.push(TimelineItem::FullMouseClick { start_idx: i, count: 3 });
                            i += 3;
                            continue;
                        }
                    }
                }
                if i + 1 < events.len() {
                    if let MacroEvent::MouseUp { button: ref b_up } = &events[i + 1] {
                        if b_down == b_up {
                            items.push(TimelineItem::FullMouseClick { start_idx: i, count: 2 });
                            i += 2;
                            continue;
                        }
                    }
                }
                items.push(TimelineItem::Single(i));
                i += 1;
            } else {
                items.push(TimelineItem::Single(i));
                i += 1;
            }
        } else {
            items.push(TimelineItem::Single(i));
            i += 1;
        }
    }
    items
}

pub(super) fn render_timeline(
    ui: &mut Ui,
    events: &mut Vec<MacroEvent>,
    group_full_presses: bool,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    egui::ScrollArea::vertical()
        .id_salt("macro_timeline_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 4.0;

            if events.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(90.0);
                    ui.label(
                        RichText::new(egui_phosphor::regular::MOUSE_SIMPLE)
                            .font(egui::FontId::new(42.0, theme::phosphor()))
                            .color(t.faint),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(loc.macro_studio_empty_text)
                        .font(egui::FontId::new(13.0, theme::medium()))
                        .color(t.muted),
                    );
                });
                return;
            }

            let expand_id = ui.scope_id().with("macro_expanded_move_groups");
            let mut expanded_groups: std::collections::HashSet<usize> = ui.data(|d| d.get_temp(expand_id).unwrap_or_default());

            let items = partition_timeline_items(events, group_full_presses, &expanded_groups);

            // Robust Drag & Drop tracking (using continuous vertical Y-centers pattern)
            let dnd_id = ui.scope_id().with("macro_timeline_dnd_src");
            let active_src: Option<usize> = ui.data(|d| d.get_temp(dnd_id));
            let pointer_pos = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()));
            let pointer_released = ui.input(|i| i.pointer.any_released());

            // Cancel drag on Esc key
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.data_mut(|d| d.remove_temp::<usize>(dnd_id));
            }

            if active_src.is_some() {
                ui.ctx().request_repaint();

                // 1. Mouse wheel scrolling while dragging
                dnd::wheel_scroll(ui);

                // 2. Edge auto-scrolling when pointer is near top or bottom of scroll area
                if let Some(pointer) = pointer_pos {
                    dnd::edge_autoscroll(ui, pointer, 70.0);
                }
            }

            let mut drag_started_idx = None;
            let mut to_remove: Option<TimelineItem> = None;
            let mut to_duplicate: Option<TimelineItem> = None;
            let mut row_centers = Vec::with_capacity(items.len());
            let mut row_rects = Vec::with_capacity(items.len());

            // Enclose rows in a padded frame so borders are never clipped against the modal edges
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(16, 6))
                .show(ui, |ui| {
                    for (item_idx, item) in items.iter().enumerate() {
                        let is_being_dragged = active_src == Some(item_idx);

                        // Allocate row with symmetric width matching available frame width (compact 36px height)
                        let row_w = ui.available_width();
                        let (row_rect, row_resp) = ui.allocate_exact_size(vec2(row_w, 36.0), Sense::click_and_drag());
                        row_centers.push(row_rect.center().y);
                        row_rects.push(row_rect);

                        // Drag started if row or grip was dragged
                        if row_resp.drag_started() {
                            drag_started_idx = Some(item_idx);
                        }

                        // Card background & border stroke (INSIDE so it never bleeds or gets clipped)
                        let bg_color = if is_being_dragged {
                            t.accent_ink.gamma_multiply(0.2)
                        } else {
                            match item {
                                TimelineItem::Single(idx) if matches!(events[*idx], MacroEvent::Delay { .. }) => t.bg.gamma_multiply(0.4),
                                _ => t.sunken,
                            }
                        };

                        let border_color = if is_being_dragged {
                            t.accent_ink
                        } else {
                            t.hairline().color
                        };

                        ui.painter().rect_filled(row_rect, CornerRadius::same(2), bg_color);
                        ui.painter().rect_stroke(row_rect, CornerRadius::same(2), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

                        ui.scope_builder(egui::UiBuilder::new().max_rect(row_rect), |ui| {
                            ui.spacing_mut().interact_size.y = 19.0;
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.horizontal_centered(|ui| {
                                 ui.add_space(8.0);

                                // Drag handle on far left
                                let grip = widgets::drag_handle(ui, loc.macro_studio_drag_handle_tip);
                                if grip.drag_started() {
                                    drag_started_idx = Some(item_idx);
                                }

                                match item {
                                    TimelineItem::Single(ev_idx) => {
                                        // Step index
                                        widgets::hud(ui, &format!("{:02}", ev_idx + 1), t.faint);
                                        ui.add_space(4.0);

                                        // Align subsequent moves in an expanded MoveGroup with the MoveGroupExpandedHead chip
                                        let is_subsequent_group_move = matches!(events[*ev_idx], MacroEvent::MouseMove { .. })
                                            && *ev_idx > 0
                                            && matches!(events[*ev_idx - 1], MacroEvent::MouseMove { .. });
                                        if is_subsequent_group_move {
                                            ui.add_space(18.0);
                                        }

                                        // Render clear, focused inline row by type
                                        render_timeline_row_inline(ui, &mut events[*ev_idx], *ev_idx, loc, t);

                                        // Action buttons on the right (sleek ghost icon buttons, NO ugly square borders)
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_space(8.0);
                                            if ghost_icon_button(ui, widgets::Glyph::Close, loc.macro_studio_tip_delete_step, true).clicked() {
                                                to_remove = Some(item.clone());
                                            }
                                            if ghost_icon_button(ui, widgets::Glyph::Clone, loc.macro_studio_tip_duplicate_step, false).clicked() {
                                                to_duplicate = Some(item.clone());
                                            }
                                        });
                                    }
                                    TimelineItem::MoveGroup { start_idx, count } => {
                                        // Step index range
                                        widgets::hud(ui, &format!("{:02}-{:02}", start_idx + 1, start_idx + count), t.faint);
                                        ui.add_space(4.0);

                                        // Expand toggle button
                                        let toggle_resp = ui.add(
                                            egui::Button::new(RichText::new("▶").font(egui::FontId::new(10.0, theme::mono_bold())).color(t.accent_ink))
                                                .frame(false)
                                                .min_size(vec2(16.0, 18.0)),
                                        ).on_hover_text(loc.macro_studio_tip_expand_moves);
                                        if toggle_resp.clicked() {
                                            expanded_groups.insert(*start_idx);
                                        }
                                        ui.add_space(2.0);

                                        // Stacked Badge
                                        let moves_title = format!("{} ({})", loc.macro_studio_badge_moves, count);
                                        render_chip(ui, &moves_title, t.accent_ink, t.bg);
                                        ui.add_space(4.0);

                                        // Trajectory summary
                                        let p_start = match &events[*start_idx] { MacroEvent::MouseMove { x, y, .. } => (*x, *y), _ => (0, 0) };
                                        let p_end = match &events[*start_idx + *count - 1] { MacroEvent::MouseMove { x, y, .. } => (*x, *y), _ => (0, 0) };
                                        let summary = format!("A: ({}, {}) → B: ({}, {}) • {} {}", p_start.0, p_start.1, p_end.0, p_end.1, count, loc.macro_studio_pts_suffix);
                                        widgets::hud(ui, &summary, t.ink_strong);

                                        ui.add_space(8.0);

                                        // Batch Smooth & Organic controls for the whole movement sequence
                                        let all_smooth = events[*start_idx..*start_idx + *count].iter().all(|e| match e {
                                            MacroEvent::MouseMove { smooth, .. } => *smooth,
                                            _ => false,
                                        });
                                        let mut group_smooth = all_smooth;
                                        let smooth_resp = ui.checkbox(&mut group_smooth, loc.macro_studio_checkbox_smooth);
                                        if smooth_resp.changed() {
                                            for ev in &mut events[*start_idx..*start_idx + *count] {
                                                if let MacroEvent::MouseMove { smooth, .. } = ev {
                                                    *smooth = group_smooth;
                                                }
                                            }
                                        }

                                        if group_smooth {
                                            let mut group_dur = match &events[*start_idx] {
                                                MacroEvent::MouseMove { duration_ms, .. } => duration_ms.unwrap_or(200),
                                                _ => 200,
                                            };
                                            let group_field = widgets::DurationField::millis(ui, loc, ("macro_group_duration", *start_idx), 10.0..=5000.0)
                                                .default_unit(widgets::TimeUnit::Milliseconds)
                                                .prefix("⏱ ");
                                            if group_field.show_millis_u32(ui, &mut group_dur).changed() {
                                                for ev in &mut events[*start_idx..*start_idx + *count] {
                                                    if let MacroEvent::MouseMove { duration_ms, .. } = ev {
                                                        *duration_ms = Some(group_dur);
                                                    }
                                                }
                                            }


                                        }

                                        // Action buttons on the right
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_space(8.0);
                                            if ghost_icon_button(ui, widgets::Glyph::Close, loc.macro_studio_tip_delete_group, true).clicked() {
                                                to_remove = Some(item.clone());
                                            }
                                            if ghost_icon_button(ui, widgets::Glyph::Clone, loc.macro_studio_tip_duplicate_group, false).clicked() {
                                                to_duplicate = Some(item.clone());
                                            }
                                        });
                                    }
                                    TimelineItem::MoveGroupExpandedHead { start_idx, count: _ } => {
                                        // Step index for the first coordinate in the sequence
                                        widgets::hud(ui, &format!("{:02}", start_idx + 1), t.faint);
                                        ui.add_space(4.0);

                                        // Collapse toggle button
                                        let toggle_resp = ui.add(
                                            egui::Button::new(RichText::new("▼").font(egui::FontId::new(10.0, theme::mono_bold())).color(t.accent_ink))
                                                .frame(false)
                                                .min_size(vec2(16.0, 18.0)),
                                        ).on_hover_text(loc.macro_studio_tip_collapse_moves);
                                        if toggle_resp.clicked() {
                                            expanded_groups.remove(start_idx);
                                        }
                                        ui.add_space(2.0);

                                        // Render as a regular single move step
                                        render_timeline_row_inline(ui, &mut events[*start_idx], *start_idx, loc, t);

                                        // Action buttons on the right: duplicate this step, delete this step
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_space(8.0);
                                            if ghost_icon_button(ui, widgets::Glyph::Close, loc.macro_studio_tip_delete_step, true).clicked() {
                                                to_remove = Some(item.clone());
                                            }
                                            if ghost_icon_button(ui, widgets::Glyph::Clone, loc.macro_studio_tip_duplicate_step, false).clicked() {
                                                to_duplicate = Some(item.clone());
                                            }
                                        });
                                    }
                                    TimelineItem::FullKeyPress { start_idx, count } => {
                                        // Step index range
                                        widgets::hud(ui, &format!("{:02}-{:02}", start_idx + 1, start_idx + count), t.faint);
                                        ui.add_space(4.0);

                                        render_chip(ui, loc.macro_studio_badge_key_press, t.accent_ink, t.bg);
                                        ui.add_space(4.0);

                                        let mut key_val = match &events[*start_idx] {
                                            MacroEvent::KeyDown { key } => key.clone(),
                                            _ => String::new(),
                                        };
                                        widgets::hud(ui, loc.macro_studio_label_key, t.muted);
                                        let edit_resp = ui.add(
                                            egui::TextEdit::singleline(&mut key_val)
                                                .desired_width(110.0)
                                                .min_size(vec2(0.0, 20.0))
                                                .font(egui::FontId::new(11.0, theme::mono())),
                                        );
                                        if edit_resp.changed() {
                                            if let MacroEvent::KeyDown { key } = &mut events[*start_idx] {
                                                *key = key_val.clone();
                                            }
                                            let up_idx = start_idx + count - 1;
                                            if let MacroEvent::KeyUp { key } = &mut events[up_idx] {
                                                *key = key_val;
                                            }
                                        }

                                        if *count == 3 {
                                            if let MacroEvent::Delay { ms, .. } = &mut events[start_idx + 1] {
                                                ui.add_space(4.0);
                                                widgets::hud(ui, loc.macro_studio_label_delay, t.muted);
                                                widgets::DurationField::millis(ui, loc, ("macro_press_delay", *start_idx), 0.0..=60_000.0)
                                                    .default_unit(widgets::TimeUnit::Milliseconds)
                                                    .prefix("⏱ ")
                                                    .show_millis_u32(ui, ms);
                                            }
                                        }

                                        // Action buttons on the right
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_space(8.0);
                                            if ghost_icon_button(ui, widgets::Glyph::Close, loc.macro_studio_tip_delete_press, true).clicked() {
                                                to_remove = Some(item.clone());
                                            }
                                            if ghost_icon_button(ui, widgets::Glyph::Clone, loc.macro_studio_tip_duplicate_press, false).clicked() {
                                                to_duplicate = Some(item.clone());
                                            }
                                        });
                                    }
                                    TimelineItem::FullMouseClick { start_idx, count } => {
                                        // Step index range
                                        widgets::hud(ui, &format!("{:02}-{:02}", start_idx + 1, start_idx + count), t.faint);
                                        ui.add_space(4.0);

                                        render_chip(ui, loc.macro_studio_badge_click, t.accent_ink, t.bg);
                                        ui.add_space(4.0);

                                        let mut btn_val = match events[*start_idx] {
                                            MacroEvent::MouseDown { button } => button,
                                            _ => MouseButton::Left,
                                        };
                                        widgets::hud(ui, loc.macro_studio_label_button, t.muted);
                                        let prev_btn = btn_val;
                                        render_button_cells(ui, &mut btn_val, loc);
                                        if btn_val != prev_btn {
                                            if let MacroEvent::MouseDown { button } = &mut events[*start_idx] {
                                                *button = btn_val;
                                            }
                                            let up_idx = start_idx + count - 1;
                                            if let MacroEvent::MouseUp { button } = &mut events[up_idx] {
                                                *button = btn_val;
                                            }
                                        }

                                        if *count == 3 {
                                            if let MacroEvent::Delay { ms, .. } = &mut events[start_idx + 1] {
                                                ui.add_space(4.0);
                                                widgets::hud(ui, loc.macro_studio_label_delay, t.muted);
                                                widgets::DurationField::millis(ui, loc, ("macro_click_delay", *start_idx), 0.0..=60_000.0)
                                                    .default_unit(widgets::TimeUnit::Milliseconds)
                                                    .prefix("⏱ ")
                                                    .show_millis_u32(ui, ms);
                                            }
                                        }

                                        // Action buttons on the right
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_space(8.0);
                                            if ghost_icon_button(ui, widgets::Glyph::Close, loc.macro_studio_tip_delete_click, true).clicked() {
                                                to_remove = Some(item.clone());
                                            }
                                            if ghost_icon_button(ui, widgets::Glyph::Clone, loc.macro_studio_tip_duplicate_click, false).clicked() {
                                                to_duplicate = Some(item.clone());
                                            }
                                        });
                                    }
                                }
                            });
                        });
                    }
                });

            // Calculate target drop index across continuous vertical space
            let mut target_idx = None;
            if let (Some(_src), Some(ptr)) = (active_src, pointer_pos) {
                if !row_centers.is_empty() {
                    let t_idx = dnd::slot_from_pointer(&row_centers, ptr.y);
                    target_idx = Some(t_idx);

                    // Draw glowing insertion line with endpoints
                    if t_idx < row_centers.len() {
                        let rect = row_rects[t_idx];
                        let y = rect.top() - 2.0;
                        ui.painter().hline(rect.x_range(), y, Stroke::new(2.5, t.accent_ink));
                        ui.painter().circle_filled(pos2(rect.left(), y), 3.0, t.accent_ink);
                        ui.painter().circle_filled(pos2(rect.right(), y), 3.0, t.accent_ink);
                    } else if let Some(last_rect) = row_rects.last() {
                        let y = last_rect.bottom() + 2.0;
                        ui.painter().hline(last_rect.x_range(), y, Stroke::new(2.5, t.accent_ink));
                        ui.painter().circle_filled(pos2(last_rect.left(), y), 3.0, t.accent_ink);
                        ui.painter().circle_filled(pos2(last_rect.right(), y), 3.0, t.accent_ink);
                    }
                }
            }

            // Commit reordering on release
            if pointer_released {
                if let (Some(src_item_idx), Some(dst_item_idx)) = (active_src, target_idx) {
                    if src_item_idx != dst_item_idx && src_item_idx < items.len() && dst_item_idx <= items.len() {
                        let (src_start, src_len) = match items[src_item_idx] {
                            TimelineItem::Single(idx) => (idx, 1),
                            TimelineItem::MoveGroup { start_idx, count }
                            | TimelineItem::FullKeyPress { start_idx, count }
                            | TimelineItem::FullMouseClick { start_idx, count } => (start_idx, count),
                            TimelineItem::MoveGroupExpandedHead { start_idx, .. } => (start_idx, 1),
                        };
                        let mut dst_event_idx = if dst_item_idx < items.len() {
                            match items[dst_item_idx] {
                                TimelineItem::Single(idx) => idx,
                                TimelineItem::MoveGroup { start_idx, .. }
                                | TimelineItem::FullKeyPress { start_idx, .. }
                                | TimelineItem::FullMouseClick { start_idx, .. } => start_idx,
                                TimelineItem::MoveGroupExpandedHead { start_idx, .. } => start_idx,
                            }
                        } else {
                            events.len()
                        };

                        if src_start + src_len <= events.len() {
                            let moving_slice: Vec<_> = events.drain(src_start..src_start + src_len).collect();
                            if dst_event_idx > src_start {
                                dst_event_idx = dst_event_idx.saturating_sub(src_len);
                            }
                            let ins_pos = dst_event_idx.min(events.len());
                            for (offset, ev) in moving_slice.into_iter().enumerate() {
                                events.insert(ins_pos + offset, ev);
                            }
                        }
                    }
                }
                ui.data_mut(|d| d.remove_temp::<usize>(dnd_id));
            } else if let Some(new_src) = drag_started_idx {
                ui.data_mut(|d| d.insert_temp(dnd_id, new_src));
            }

            if let Some(item) = to_remove {
                match item {
                    TimelineItem::Single(idx) => {
                        if idx < events.len() {
                            events.remove(idx);
                            let mut updated = std::collections::HashSet::new();
                            for g in expanded_groups.drain() {
                                if g > idx {
                                    updated.insert(g - 1);
                                } else if g == idx {
                                    if idx < events.len() && matches!(events[idx], MacroEvent::MouseMove { .. }) {
                                        updated.insert(idx);
                                    }
                                } else {
                                    updated.insert(g);
                                }
                            }
                            expanded_groups = updated;
                        }
                    }
                    TimelineItem::MoveGroup { start_idx, count }
                    | TimelineItem::FullKeyPress { start_idx, count }
                    | TimelineItem::FullMouseClick { start_idx, count } => {
                        if start_idx + count <= events.len() {
                            events.drain(start_idx..start_idx + count);
                            expanded_groups.remove(&start_idx);
                            let mut updated = std::collections::HashSet::new();
                            for g in expanded_groups.drain() {
                                if g > start_idx {
                                    updated.insert(g.saturating_sub(count));
                                } else {
                                    updated.insert(g);
                                }
                            }
                            expanded_groups = updated;
                        }
                    }
                    TimelineItem::MoveGroupExpandedHead { start_idx, .. } => {
                        if start_idx < events.len() {
                            events.remove(start_idx);
                            let mut updated = std::collections::HashSet::new();
                            for g in expanded_groups.drain() {
                                if g > start_idx {
                                    updated.insert(g - 1);
                                } else if g == start_idx {
                                    if start_idx < events.len() && matches!(events[start_idx], MacroEvent::MouseMove { .. }) {
                                        updated.insert(start_idx);
                                    }
                                } else {
                                    updated.insert(g);
                                }
                            }
                            expanded_groups = updated;
                        }
                    }
                }
            } else if let Some(item) = to_duplicate {
                match item {
                    TimelineItem::Single(idx) => {
                        if idx < events.len() {
                            let clone_ev = events[idx].clone();
                            events.insert(idx + 1, clone_ev);
                            let mut updated = std::collections::HashSet::new();
                            for g in expanded_groups.drain() {
                                if g > idx {
                                    updated.insert(g + 1);
                                } else {
                                    updated.insert(g);
                                }
                            }
                            expanded_groups = updated;
                        }
                    }
                    TimelineItem::MoveGroup { start_idx, count }
                    | TimelineItem::FullKeyPress { start_idx, count }
                    | TimelineItem::FullMouseClick { start_idx, count } => {
                        if start_idx + count <= events.len() {
                            let slice: Vec<_> = events[start_idx..start_idx + count].to_vec();
                            for (offset, ev) in slice.into_iter().enumerate() {
                                events.insert(start_idx + count + offset, ev);
                            }
                        }
                    }
                    TimelineItem::MoveGroupExpandedHead { start_idx, .. } => {
                        if start_idx < events.len() {
                            let clone_ev = events[start_idx].clone();
                            events.insert(start_idx + 1, clone_ev);
                            let mut updated = std::collections::HashSet::new();
                            for g in expanded_groups.drain() {
                                if g > start_idx {
                                    updated.insert(g + 1);
                                } else {
                                    updated.insert(g);
                                }
                            }
                            expanded_groups = updated;
                        }
                    }
                }
            }

            ui.data_mut(|d| d.insert_temp(expand_id, expanded_groups));

            ui.add_space(8.0);
        });
}

/// Duration shown in the Macro Studio editor for a `MouseMove` step. The engine uses 250 ms for
/// smooth moves and 16 ms for instant moves when the step stores no duration, so the editor
/// shows that effective default instead of a value the replay would ignore.
fn effective_move_duration_ms(smooth: bool, duration_ms: Option<u32>) -> u32 {
    duration_ms.unwrap_or(if smooth { 250 } else { 16 })
}

fn render_timeline_row_inline(
    ui: &mut Ui,
    event: &mut MacroEvent,
    idx: usize,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    // Normalize KeyPress and MouseClick events to physical down events
    if let MacroEvent::KeyPress { key } = event {
        *event = MacroEvent::KeyDown { key: key.clone() };
    }
    if let MacroEvent::MouseClick { button, .. } = event {
        *event = MacroEvent::MouseDown { button: *button };
    }

    let picker_id = ui.scope_id().with(idx).with("row_coord_picker");
    ui.spacing_mut().interact_size.y = 19.0;
    ui.spacing_mut().item_spacing.y = 0.0;

    let mut toggle_to = None;

    match event {
        // --- KEY DOWN ---
        MacroEvent::KeyDown { key } => {
            let tip = loc.macro_studio_tip_toggle_to_release;
            let chip_txt = loc.macro_studio_chip_key_down;
            if render_clickable_chip(ui, chip_txt, t.accent_ink, t.bg, tip) {
                toggle_to = Some(MacroEvent::KeyUp { key: key.clone() });
            }
            widgets::hud(ui, loc.macro_studio_label_down, t.muted);
            ui.add(egui::TextEdit::singleline(key).desired_width(110.0).min_size(vec2(0.0, 20.0)).font(egui::FontId::new(11.0, theme::mono())));
        }

        // --- KEY UP ---
        MacroEvent::KeyUp { key } => {
            let tip = loc.macro_studio_tip_toggle_to_press;
            let chip_txt = loc.macro_studio_chip_key_up;
            if render_clickable_chip(ui, chip_txt, t.muted, t.bg, tip) {
                toggle_to = Some(MacroEvent::KeyDown { key: key.clone() });
            }
            widgets::hud(ui, loc.macro_studio_label_up, t.muted);
            ui.add(egui::TextEdit::singleline(key).desired_width(110.0).min_size(vec2(0.0, 20.0)).font(egui::FontId::new(11.0, theme::mono())));
        }

        // --- MOUSE DOWN ---
        MacroEvent::MouseDown { button } => {
            let tip = loc.macro_studio_tip_toggle_to_mouse_release;
            let chip_txt = loc.macro_studio_chip_mouse_down;
            if render_clickable_chip(ui, chip_txt, t.accent_ink, t.bg, tip) {
                toggle_to = Some(MacroEvent::MouseUp { button: *button });
            }
            widgets::hud(ui, loc.macro_studio_label_down, t.muted);
            render_button_cells(ui, button, loc);
        }

        // --- MOUSE UP ---
        MacroEvent::MouseUp { button } => {
            let tip = loc.macro_studio_tip_toggle_to_mouse_press;
            let chip_txt = loc.macro_studio_chip_mouse_up;
            if render_clickable_chip(ui, chip_txt, t.muted, t.bg, tip) {
                toggle_to = Some(MacroEvent::MouseDown { button: *button });
            }
            widgets::hud(ui, loc.macro_studio_label_up, t.muted);
            render_button_cells(ui, button, loc);
        }

        // Fallbacks for any other click/press variants
        MacroEvent::KeyPress { key } => {
            toggle_to = Some(MacroEvent::KeyDown { key: key.clone() });
        }
        MacroEvent::MouseClick { button, .. } => {
            toggle_to = Some(MacroEvent::MouseDown { button: *button });
        }

        // --- MOUSE MOVE & GAME CAMERA TURN ---
        MacroEvent::MouseMove { x, y, coord_mode, smooth, duration_ms, organic: _ } => {
            let is_rel = *coord_mode == MouseCoordMode::Relative;
            let chip_title = if is_rel {
                loc.macro_studio_chip_turn
            } else {
                loc.macro_studio_chip_move
            };
            let chip_tip = if is_rel {
                loc.macro_studio_tip_rel_turn
            } else {
                loc.macro_studio_tip_abs_move
            };

            if render_clickable_chip(ui, chip_title, t.accent_ink, t.bg, chip_tip) {
                *coord_mode = if is_rel {
                    MouseCoordMode::Absolute
                } else {
                    MouseCoordMode::Relative
                };
            }

            if is_rel {
                ui.add(egui::DragValue::new(x).prefix("ΔX: ").speed(2.0));
                ui.add(egui::DragValue::new(y).prefix("ΔY: ").speed(2.0));
            } else {
                ui.add(egui::DragValue::new(x).prefix("X: "));
                ui.add(egui::DragValue::new(y).prefix("Y: "));

                let mut opt_x = Some(*x);
                let mut opt_y = Some(*y);
                render_compact_coord_picker(ui, picker_id, &mut opt_x, &mut opt_y, loc);
                if let (Some(nx), Some(ny)) = (opt_x, opt_y) {
                    *x = nx;
                    *y = ny;
                }
            }

            ui.checkbox(smooth, loc.macro_studio_checkbox_smooth);

            // The duration is applied in both modes — glide time for a smooth move, a pause
            // after an instant jump — so it stays editable instead of leaving an invisible
            // multi-second pause behind an unchecked Smooth box.
            let mut dur = effective_move_duration_ms(*smooth, *duration_ms);
            let duration_field = widgets::DurationField::millis(ui, loc, ("macro_move_duration", idx), 10.0..=5000.0)
                .default_unit(widgets::TimeUnit::Milliseconds)
                .prefix("⏱ ");
            if duration_field.show_millis_u32(ui, &mut dur).changed() {
                *duration_ms = Some(dur);
            }
        }

        // --- PAUSE / DELAY (clean input without redundant preset buttons) ---
        MacroEvent::Delay { ms, .. } => {
            render_chip(ui, loc.macro_studio_chip_delay, t.faint, t.bg);
            widgets::hud(ui, loc.macro_studio_label_delay, t.muted);
            widgets::DurationField::millis(ui, loc, ("macro_delay", idx), 0.0..=60_000.0)
                .default_unit(widgets::TimeUnit::Milliseconds)
                .prefix("⏱ ")
                .show_millis_u32(ui, ms);
        }

        // --- TEXT TYPING (with built-in char delay and organic jitter) ---
        MacroEvent::Text { text, delay_ms, .. } => {
            render_chip(ui, loc.macro_studio_chip_text, t.accent_ink, t.bg);
            widgets::hud(ui, loc.macro_studio_label_text, t.muted);
            ui.add(egui::TextEdit::singleline(text).desired_width(170.0).min_size(vec2(0.0, 20.0)).font(egui::FontId::new(11.0, theme::mono())));
            ui.add_space(4.0);
            widgets::hud(ui, loc.macro_studio_label_delay, t.muted);
            widgets::DurationField::millis(ui, loc, ("macro_text_delay", idx), 0.0..=5000.0)
                .default_unit(widgets::TimeUnit::Milliseconds)
                .prefix("⏱ ")
                .show_millis_u32(ui, delay_ms);
        }

        // --- SCROLL WHEEL ---
        MacroEvent::MouseWheel { delta } => {
            render_chip(ui, loc.macro_studio_chip_wheel_badge, t.muted, t.bg);
            widgets::hud(ui, loc.macro_studio_label_wheel, t.muted);
            ui.add(egui::DragValue::new(delta).range(-5000..=5000));
        }
    }

    if let Some(new_ev) = toggle_to {
        *event = new_ev;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_timeline_items_grouping() {
        let events = vec![
            MacroEvent::KeyDown { key: "Enter".to_string() },
            MacroEvent::Delay { ms: 40, organic: false, jitter_percent: 30 },
            MacroEvent::KeyUp { key: "Enter".to_string() },
            MacroEvent::MouseDown { button: MouseButton::Left },
            MacroEvent::Delay { ms: 40, organic: false, jitter_percent: 30 },
            MacroEvent::MouseUp { button: MouseButton::Left },
            MacroEvent::MouseMove { x: 10, y: 10, coord_mode: MouseCoordMode::Absolute, smooth: false, duration_ms: None, organic: false },
            MacroEvent::MouseMove { x: 20, y: 20, coord_mode: MouseCoordMode::Absolute, smooth: false, duration_ms: None, organic: false },
        ];

        // Without group_presses: keys/mouse stay Single, moves group into MoveGroup
        let unpartitioned = partition_timeline_items(&events, false, &std::collections::HashSet::new());
        assert_eq!(unpartitioned.len(), 7);
        assert_eq!(unpartitioned[0], TimelineItem::Single(0));
        assert_eq!(unpartitioned[1], TimelineItem::Single(1));
        assert_eq!(unpartitioned[2], TimelineItem::Single(2));
        assert_eq!(unpartitioned[3], TimelineItem::Single(3));
        assert_eq!(unpartitioned[4], TimelineItem::Single(4));
        assert_eq!(unpartitioned[5], TimelineItem::Single(5));
        assert_eq!(unpartitioned[6], TimelineItem::MoveGroup { start_idx: 6, count: 2 });

        // With group_presses: KeyDown+Delay+KeyUp -> FullKeyPress, MouseDown+Delay+MouseUp -> FullMouseClick
        let partitioned = partition_timeline_items(&events, true, &std::collections::HashSet::new());
        assert_eq!(partitioned.len(), 3);
        assert_eq!(partitioned[0], TimelineItem::FullKeyPress { start_idx: 0, count: 3 });
        assert_eq!(partitioned[1], TimelineItem::FullMouseClick { start_idx: 3, count: 3 });
        assert_eq!(partitioned[2], TimelineItem::MoveGroup { start_idx: 6, count: 2 });

        // With expanded move group: MoveGroupExpandedHead + Single
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(6);
        let partitioned_exp = partition_timeline_items(&events, true, &expanded);
        assert_eq!(partitioned_exp.len(), 4);
        assert_eq!(partitioned_exp[0], TimelineItem::FullKeyPress { start_idx: 0, count: 3 });
        assert_eq!(partitioned_exp[1], TimelineItem::FullMouseClick { start_idx: 3, count: 3 });
        assert_eq!(partitioned_exp[2], TimelineItem::MoveGroupExpandedHead { start_idx: 6, count: 2 });
        assert_eq!(partitioned_exp[3], TimelineItem::Single(7));
    }

    /// U03-010: a duration stored for a smooth move stays visible and editable after Smooth is
    /// unchecked, because the engine still applies it as the pause after an instant move.
    #[test]
    fn test_move_duration_is_editable_in_both_move_modes() {
        assert_eq!(effective_move_duration_ms(true, None), 250);
        assert_eq!(effective_move_duration_ms(false, None), 16);
        assert_eq!(effective_move_duration_ms(false, Some(5000)), 5000);
        assert_eq!(effective_move_duration_ms(true, Some(5000)), 5000);
    }

    #[test]
    fn test_dnd_reordering_math() {
        // Test move down: from index 0 to target 2
        let mut items = vec!["A", "B", "C", "D"];
        let src = 0;
        let dst = 2;
        if dst > src + 1 {
            let item = items.remove(src);
            items.insert(dst - 1, item);
        }
        assert_eq!(items, vec!["B", "A", "C", "D"]);

        // Test move up: from index 3 to target 1
        let mut items2 = vec!["A", "B", "C", "D"];
        let src2 = 3;
        let dst2 = 1;
        if dst2 < src2 {
            let item = items2.remove(src2);
            items2.insert(dst2, item);
        }
        assert_eq!(items2, vec!["A", "D", "B", "C"]);
    }
}
