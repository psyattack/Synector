//! Process Picker Modal: Studio modal for browsing and selecting an active Windows process
//! with segmented hairline filter cells, instant search, multi-column sorting,
//! Win32 icons, and 1-click select binding for triggers and actions.
//!
//! The modal is split into the header (category cells + search), the filter/sort query and the
//! virtualised table body so every piece can be reasoned about (and tested) on its own.

use egui::{
    Color32, CornerRadius, Margin, Rect, Sense, Stroke,
    pos2, vec2,
};
use egui_extras::{Column, TableBody, TableBuilder};
use crate::gui::components::icons::ProcessIconCache;
use crate::gui::components::modal::{Modal, ModalFrame};
use crate::gui::components::widgets::{self, Cell, Glyph, Kind};
use crate::gui::state::{GuiState, ProcessCategoryFilter, ProcessSortColumn};
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::ProcessSnapshotItem;

pub fn render_process_picker_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.picker_target.is_some();
    let loc = state.loc();
    let mut chosen_process: Option<String> = None;

    let modal_res = Modal::new("process-picker-modal", is_open)
        .width_fraction(0.85)
        .height_fraction(0.82)
        .min_size(vec2(720.0, 480.0))
        .max_size(vec2(960.0, 680.0))
        .header_height(46.0)
        .header_sunken(true)
        .show(ctx, &t, |frame| {
            render_picker_header(frame, state, &t, loc);
            frame.body(|ui| render_picker_body(ui, state, &t, loc, &mut chosen_process));
        });

    // A pick only commits while the picker is still open: the fully clickable table is painted
    // through the close animation, so an unguarded click re-applied the selection (U04-002).
    if !crate::gui::views::action_catalog_modal::catalog_commit_allowed(is_open) {
        if modal_res.is_none() {
            state.last_picker_target = None;
        }
        return;
    }

    if let Some(name) = chosen_process {
        state.apply_selected_process(&name);
    } else if let Some(res) = modal_res {
        if res.close_requested {
            state.close_process_picker();
        }
    } else if !is_open {
        state.last_picker_target = None;
    }
}

/// Header slice: segmented category cells on the left, close button + search on the right.
fn render_picker_header(
    frame: &mut ModalFrame<'_>,
    state: &mut GuiState,
    t: &Tokens,
    loc: &'static Strings,
) {
    frame.header(|ui, close_req| {
        egui::Frame::new()
            .inner_margin(Margin { left: 14, right: 12, top: 7, bottom: 7 })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;

                    render_category_cells(ui, state, t, loc);

                    // Right side: Close button, Search Input
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::glyph_button(ui, Glyph::Close, loc.inspector_btn_close_tip).clicked() {
                            *close_req = true;
                        }

                        ui.add_space(8.0);

                        ui.add(
                            egui::TextEdit::singleline(&mut state.process_search_query)
                                .hint_text(loc.hunter_search_placeholder)
                                .desired_width(240.0)
                                .min_size(vec2(0.0, theme::CONTROL_H))
                                .align(egui::Align2::LEFT_CENTER)
                                .margin(vec2(10.0, 8.0)),
                        );
                    });
                });
            });
    });
}

/// The three segmented filter cells with their live counts.
fn render_category_cells(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    t: &Tokens,
    loc: &'static Strings,
) {
    let total = state.cached_processes.len();
    let windowed_count = state.cached_processes.iter().filter(|p| !p.window_title.is_empty()).count();
    let bg_count = total.saturating_sub(windowed_count);

    let options = [
        (
            ProcessCategoryFilter::All,
            Cell {
                label: loc.hunter_cell_all,
                tip: loc.hunter_cell_all_tip,
                count: Some((total, None)),
            },
        ),
        (
            ProcessCategoryFilter::WindowedOnly,
            Cell {
                label: loc.hunter_cell_windowed,
                tip: loc.hunter_cell_windowed_tip,
                count: Some((windowed_count, Some(t.accent_ink))),
            },
        ),
        (
            ProcessCategoryFilter::BackgroundOnly,
            Cell {
                label: loc.hunter_cell_services,
                tip: loc.hunter_cell_services_tip,
                count: Some((bg_count, None)),
            },
        ),
    ];

    widgets::cells(ui, &mut state.process_category_filter, &options);
}

/// Body slice: empty state, otherwise the virtualised process table.
fn render_picker_body(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    t: &Tokens,
    loc: &'static Strings,
    chosen_process: &mut Option<String>,
) {
    egui::Frame::new()
        .inner_margin(Margin { left: 14, right: 14, top: 4, bottom: 8 })
        .show(ui, |ui| {
            let order = filter_and_sort_processes(state);
            if order.is_empty() {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    widgets::hud(ui, loc.hunter_empty_title, t.muted);
                });
            } else {
                let table_h = ui.available_height().max(100.0);
                render_process_table(ui, state, t, loc, &order, table_h, chosen_process);
            }
        });
}

fn render_process_table(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    t: &Tokens,
    loc: &'static Strings,
    order: &[usize],
    table_h: f32,
    chosen_process: &mut Option<String>,
) {
    let table = TableBuilder::new(ui)
        .striped(false)
        .resizable(false)
        .min_scrolled_height(table_h)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(300.0).clip(true))
        .column(Column::exact(65.0))
        .column(Column::remainder().at_least(200.0).clip(true))
        .column(Column::exact(125.0))
        .header(32.0, |mut header| {
            header.col(|ui| {
                render_sortable_header(ui, state, ProcessSortColumn::Name, loc.hunter_col_process_name, t);
            });
            header.col(|ui| {
                render_sortable_header(ui, state, ProcessSortColumn::Pid, loc.hunter_col_pid, t);
            });
            header.col(|ui| {
                render_sortable_header(ui, state, ProcessSortColumn::WindowTitle, loc.hunter_col_window_title, t);
            });
            header.col(|ui| {
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
                let galley = widgets::caps(ui, loc.hunter_col_quick_actions, theme::HUD_SIZE, theme::mono_bold(), theme::HUD_SPACING, t.faint);
                widgets::paint_centred(ui, rect, galley, t.faint);
            });
        });

    let processes = &state.cached_processes;
    let icons = &mut state.icon_cache;
    table.body(|body| {
        render_process_rows(body, processes, order, icons, t, loc, chosen_process);
    });
}

/// Virtualised body: `body.rows` lays out only the rows inside the viewport (U04-004).
fn render_process_rows(
    body: TableBody<'_>,
    processes: &[ProcessSnapshotItem],
    order: &[usize],
    icons: &mut ProcessIconCache,
    t: &Tokens,
    loc: &'static Strings,
    chosen_process: &mut Option<String>,
) {
    body.rows(36.0, order.len(), |mut row| {
        let proc = &processes[order[row.index()]];
        let mut row_double_clicked = false;

        row.col(|ui| render_process_name_cell(ui, icons, proc, t, &mut row_double_clicked));
        row.col(|ui| render_process_pid_cell(ui, proc, t, &mut row_double_clicked));
        row.col(|ui| render_process_window_cell(ui, proc, t, &mut row_double_clicked));
        row.col(|ui| {
            render_process_select_cell(ui, loc, row_double_clicked, chosen_process, &proc.name);
        });
    });
}

fn render_process_name_cell(
    ui: &mut egui::Ui,
    icons: &mut ProcessIconCache,
    proc: &ProcessSnapshotItem,
    t: &Tokens,
    row_double_clicked: &mut bool,
) {
    let (row_rect, row_resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    if row_resp.double_clicked() {
        *row_double_clicked = true;
    }

    let p = ui.painter();
    if row_resp.hovered() {
        p.rect_filled(row_rect, CornerRadius::ZERO, t.sunken);
    }
    p.hline(row_rect.x_range(), row_rect.bottom() - 0.5, t.hairline());

    // Draw real Win32 process icon
    let icon_rect = Rect::from_min_size(pos2(row_rect.left() + 8.0, row_rect.center().y - 8.0), vec2(16.0, 16.0));
    if let Some(tex) = icons.get_icon(ui.ctx(), proc.pid, &proc.name) {
        p.image(tex.id(), icon_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        p.rect_stroke(icon_rect.shrink(2.0), CornerRadius::ZERO, Stroke::new(1.0, t.line_strong), egui::StrokeKind::Inside);
        p.circle_filled(icon_rect.center(), 2.0, t.muted);
    }

    let galley = widgets::caps(ui, &proc.name, theme::HUD_SIZE + 1.0, theme::mono_bold(), theme::HUD_SPACING, t.ink_strong);
    let text_pos = pos2(row_rect.left() + 32.0, row_rect.center().y - galley.size().y / 2.0);
    p.galley(text_pos, galley, t.ink_strong);
}

fn render_process_pid_cell(
    ui: &mut egui::Ui,
    proc: &ProcessSnapshotItem,
    t: &Tokens,
    row_double_clicked: &mut bool,
) {
    let (cell_rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    if resp.double_clicked() {
        *row_double_clicked = true;
    }
    ui.painter().hline(cell_rect.x_range(), cell_rect.bottom() - 0.5, t.hairline());
    let galley = widgets::hud_galley(ui, &format!("{:05}", proc.pid), t.muted);
    widgets::paint_left(ui, cell_rect, galley, t.muted);
}

fn render_process_window_cell(
    ui: &mut egui::Ui,
    proc: &ProcessSnapshotItem,
    t: &Tokens,
    row_double_clicked: &mut bool,
) {
    let (cell_rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    if resp.double_clicked() {
        *row_double_clicked = true;
    }
    ui.painter().hline(cell_rect.x_range(), cell_rect.bottom() - 0.5, t.hairline());

    let title = if proc.window_title.is_empty() { "—" } else { &proc.window_title };
    let color = if proc.window_title.is_empty() { t.faint } else { t.muted };
    let format = egui::TextFormat {
        font_id: egui::FontId::new(theme::HUD_SIZE, theme::mono()),
        color,
        extra_letter_spacing: theme::HUD_SPACING,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::single_section(title.to_uppercase(), format);
    job.wrap = egui::text::TextWrapping::truncate_at_width(cell_rect.width());
    let galley = ui.painter().layout_job(job);
    let elided = galley.elided;
    widgets::paint_left(ui, cell_rect, galley, color);
    if elided {
        resp.on_hover_text(title);
    }
}

fn render_process_select_cell(
    ui: &mut egui::Ui,
    loc: &'static Strings,
    row_double_clicked: bool,
    chosen_process: &mut Option<String>,
    name: &str,
) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() - theme::CONTROL_H).max(0.0) / 2.0);
        if widgets::button(ui, loc.hunter_btn_select, Kind::Primary, 0.0).clicked()
            || row_double_clicked
        {
            *chosen_process = Some(name.to_string());
        }
    });
}

/// Indices into `state.cached_processes` matching the active category filter and search query,
/// ordered by the active sort column. Indices (not clones) keep the per-frame cost off the
/// snapshot payloads.
fn filter_and_sort_processes(state: &GuiState) -> Vec<usize> {
    let needle = state.process_search_query.trim().to_lowercase();

    let mut order: Vec<usize> = state
        .cached_processes
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            match state.process_category_filter {
                ProcessCategoryFilter::All => true,
                ProcessCategoryFilter::WindowedOnly => !p.window_title.is_empty(),
                ProcessCategoryFilter::BackgroundOnly => p.window_title.is_empty(),
            }
        })
        .filter(|(_, p)| {
            if needle.is_empty() {
                true
            } else {
                p.name.to_lowercase().contains(&needle)
                    || p.pid.to_string().contains(&needle)
                    || p.window_title.to_lowercase().contains(&needle)
            }
        })
        .map(|(idx, _)| idx)
        .collect();

    let processes = &state.cached_processes;
    let asc = state.process_sort_ascending;
    match state.process_sort_column {
        ProcessSortColumn::Name => order.sort_by(|&a, &b| {
            let cmp = processes[a].name.to_lowercase().cmp(&processes[b].name.to_lowercase());
            if asc { cmp } else { cmp.reverse() }
        }),
        ProcessSortColumn::Pid => order.sort_by(|&a, &b| {
            let cmp = processes[a].pid.cmp(&processes[b].pid);
            if asc { cmp } else { cmp.reverse() }
        }),
        ProcessSortColumn::WindowTitle => order.sort_by(|&a, &b| {
            let cmp = processes[a].window_title.to_lowercase().cmp(&processes[b].window_title.to_lowercase());
            if asc { cmp } else { cmp.reverse() }
        }),
    }

    order
}

fn render_sortable_header(
    ui: &mut egui::Ui,
    state: &mut GuiState,
    col: ProcessSortColumn,
    label: &str,
    t: &Tokens,
) {
    let is_active = state.process_sort_column == col;
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());

    if resp.clicked() {
        if is_active {
            state.process_sort_ascending = !state.process_sort_ascending;
        } else {
            state.process_sort_column = col;
            state.process_sort_ascending = true;
        }
    }

    let color = if resp.hovered() {
        t.ink_strong
    } else if is_active {
        t.accent_ink
    } else {
        t.faint
    };

    let title = if is_active {
        if state.process_sort_ascending {
            format!("{} ▲", label)
        } else {
            format!("{} ▼", label)
        }
    } else {
        label.to_string()
    };

    let galley = widgets::caps(ui, &title, theme::HUD_SIZE, theme::mono_bold(), theme::HUD_SPACING, color);
    widgets::paint_left(ui, rect, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(pid: u32, name: &str, window_title: &str) -> ProcessSnapshotItem {
        ProcessSnapshotItem {
            pid,
            name: name.to_string(),
            window_title: window_title.to_string(),
            memory_bytes: 0,
            cpu_percent: 0.0,
        }
    }

    fn picker_state(processes: Vec<ProcessSnapshotItem>) -> GuiState {
        let mut state = GuiState::new();
        state.cached_processes = processes;
        state
    }

    fn names<'a>(state: &'a GuiState, order: &[usize]) -> Vec<&'a str> {
        order.iter().map(|&idx| state.cached_processes[idx].name.as_str()).collect()
    }

    #[test]
    fn test_filter_and_sort_processes_uses_live_snapshot_and_indices() {
        let mut state = picker_state(vec![
            snapshot(30, "svchost.exe", ""),
            snapshot(412, "chrome.exe", "New Tab - Chrome"),
            snapshot(7, "notepad.exe", "Untitled - Notepad"),
        ]);

        // Name ascending is the default order.
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["chrome.exe", "notepad.exe", "svchost.exe"]);

        // Descending flips the same comparator.
        state.process_sort_ascending = false;
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["svchost.exe", "notepad.exe", "chrome.exe"]);

        // PID column sorts numerically.
        state.process_sort_column = ProcessSortColumn::Pid;
        state.process_sort_ascending = true;
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["notepad.exe", "svchost.exe", "chrome.exe"]);

        // Window title sorts case-insensitively; background rows (empty title) come first.
        state.process_sort_column = ProcessSortColumn::WindowTitle;
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["svchost.exe", "chrome.exe", "notepad.exe"]);
    }

    #[test]
    fn test_filter_and_sort_processes_honours_category_and_query() {
        let mut state = picker_state(vec![
            snapshot(1, "svchost.exe", ""),
            snapshot(2, "chrome.exe", "New Tab - Chrome"),
            snapshot(3, "notepad.exe", "Untitled - Notepad"),
        ]);

        state.process_category_filter = ProcessCategoryFilter::WindowedOnly;
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["chrome.exe", "notepad.exe"]);

        state.process_category_filter = ProcessCategoryFilter::BackgroundOnly;
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["svchost.exe"]);

        state.process_category_filter = ProcessCategoryFilter::All;
        state.process_search_query = "CHROME".to_string();
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["chrome.exe"]);

        state.process_search_query = "3".to_string();
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["notepad.exe"]);

        state.process_search_query = "untitled".to_string();
        assert_eq!(names(&state, &filter_and_sort_processes(&state)), ["notepad.exe"]);

        state.process_search_query = "no-such-process".to_string();
        assert!(filter_and_sort_processes(&state).is_empty());
    }
}
