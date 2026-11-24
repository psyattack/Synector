//! Analytics dialog: the daemon's rolling activity and system telemetry rendered as a dashboard.
//!
//! The layout is one scroll: the activity log is its first section and the dashboard follows it.
//! Every number comes from the daemon: the snapshot carries run sources, outcomes, hourly buckets
//! and the live CPU/RAM series; this view only formats them. Charts are hand-painted in
//! `components::charts` on the design tokens, so no plotting dependency and no hardcoded colours.

use egui::{
    Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Ui, pos2, vec2,
};
use crate::gui::components::charts::{self, BarDatum, ChartSeries, DonutSegment};
use crate::gui::components::modal::{Modal, ModalHeader, render_modal_header};
use crate::gui::components::widgets;
use crate::gui::state::{AnalyticsLogFilter, GuiState};
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::{AnalyticsSnapshot, LogItem, Rule, RunSource};

/// Height of the KPI cards. The extra lines need room: the caption sits below the value with a
/// visible gap instead of touching its descenders.
const KPI_H: f32 = 72.0;
/// Height of the system chart card.
const SYSTEM_H: f32 = 204.0;
/// Height of the sources/activity row.
const MID_H: f32 = 206.0;
/// Height of the top-rules/attention row.
const LOWER_H: f32 = 236.0;
/// Gap between cards and rows.
const GAP: f32 = 12.0;
/// Below this body width the dashboard stacks its two-column rows.
const COLUMN_MIN_W: f32 = 820.0;
/// Height of the activity-log block at the top of the analytics page. It is the first section of
/// the common scroll, not a separate dock.
const LOG_SECTION_H: f32 = 238.0;
/// The page inset above the first block and below the last one; both are identical.
const PAGE_INSET: f32 = 16.0;
/// Side inset of the page inside the dialog body, left and right.
const SIDE_INSET: f32 = 20.0;
/// Samples the live CPU/RAM chart shows: the collector keeps about two minutes, the card title
/// promises the last 60 seconds and the sample cadence is one per second.
const LIVE_CHART_SAMPLES: usize = 60;
/// Corner radius of the 3 px progress marks (track and filled bar): half their height, which is a
/// property of the mark rather than a chrome radius.
const MARK_CORNER: u8 = 1;

/// One KPI card of the top strip.
struct KpiCard<'a> {
    label: &'a str,
    value: String,
    caption: String,
    value_color: Color32,
    /// Optional sparkline values, drawn in the card's top-right corner.
    spark: Option<&'a [f32]>,
}

/// One line of the top-rules ranking.
struct TopRuleRow<'a> {
    label: &'a str,
    executions: u32,
    max: u32,
    rule: Option<&'a Rule>,
}

/// The analytics dialog. The daemon snapshot is polled by `GuiState::poll_analytics` while this
/// dialog is open; closing it stops the polling.
pub fn render_analytics_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let loc = state.loc();
    let mut clicked_rule_id: Option<String> = None;

    let Some(response) = Modal::new("analytics-modal", state.show_analytics_modal)
        .width_fraction(0.88)
        .height_fraction(0.86)
        .min_size(vec2(680.0, 460.0))
        .max_size(vec2(1180.0, 840.0))
        .header_height(56.0)
        .show(ctx, &t, |frame| {
            frame.header(|ui, close_requested| {
                render_modal_header(
                    ui,
                    close_requested,
                    &t,
                    ModalHeader {
                        glyph: Some(egui_phosphor::regular::CHART_DONUT),
                        glyph_size: 16.0,
                        close_tip: loc.modal_tip_dismiss,
                        left_pad: 20.0,
                        glyph_pad: 10.0,
                        right_pad: 20.0,
                        item_spacing_x: Some(10.0),
                    },
                    |ui| {
                        widgets::hud(ui, loc.analytics_title, t.ink_strong);
                    },
                    |_ui| {},
                );
            });

            frame.body(|ui| {
                // One page: the activity log is the first section of the same scroll as the
                // metrics, and the inset above the first block equals the one below the last.
                let inner = Rect::from_min_max(
                    ui.max_rect().min + vec2(SIDE_INSET, PAGE_INSET),
                    ui.max_rect().max - vec2(SIDE_INSET, PAGE_INSET),
                );
                if inner.width() < 60.0 || inner.height() < 160.0 {
                    return;
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                    ui.set_width(inner.width());
                    ui.set_height(inner.height());
                    egui::ScrollArea::vertical()
                        .id_salt("analytics-scroll")
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.set_width(inner.width());
                            ui.spacing_mut().item_spacing.y = GAP;

                            // Logs first, in their own order: level filters and the stretched
                            // text search, then the stream itself. The block fills its slot, so
                            // the gap to the metrics is exactly the page's item spacing. The log
                            // buffer and the two filter fields are borrowed separately, so the
                            // stream is never cloned.
                            let logs = state
                                .daemon_state
                                .as_ref()
                                .map(|daemon| daemon.logs.as_slice())
                                .unwrap_or(&[]);
                            ui.allocate_ui_with_layout(
                                vec2(ui.available_width(), LOG_SECTION_H),
                                Layout::top_down(Align::LEFT),
                                |ui| {
                                    render_log_panel(
                                        ui,
                                        logs,
                                        &mut state.analytics_log_filter,
                                        &mut state.analytics_log_level,
                                        &t,
                                        loc,
                                    );
                                },
                            );

                            clicked_rule_id = render_dashboard(ui, state, &t, loc);
                        });
                });
            });
        })
    else {
        return;
    };

    if response.close_requested {
        state.close_analytics_modal();
    }

    // A click during the 160 ms close animation must not commit: the dialog is already on its way
    // out for the user (the catalog modal guards its commits the same way).
    if let Some(rule_id) = clicked_rule_id.filter(|_| state.show_analytics_modal) {
        state.close_analytics_modal();
        if let Some(rule) = state
            .daemon_state
            .as_ref()
            .and_then(|daemon| daemon.config.rules.iter().find(|rule| rule.id == rule_id))
            .cloned()
        {
            state.edit_rule(rule);
        }
    }
}

/// Paints the dashboard over a borrowed state and returns the rule a top-rules row was clicked
/// for; committing that click is the caller's job, after the render borrows end.
fn render_dashboard(
    ui: &mut Ui,
    state: &GuiState,
    t: &Tokens,
    loc: &Strings,
) -> Option<String> {
    ui.spacing_mut().item_spacing.y = GAP;
    // The last snapshot stays on screen during an outage; say so instead of presenting stale
    // numbers as live. A GUI that has never reached the daemon already has its own offline
    // signal and briefly carries `false` before the first sync, so only a confirmed outage is
    // reported here.
    if state.has_connected_daemon && !state.is_daemon_connected {
        widgets::hud(ui, loc.settings_daemon_offline, t.bad);
    }
    let snapshot = state.analytics.as_ref();
    let daemon = state.daemon_state.as_ref();

    render_kpi_row(ui, state, snapshot, t, loc);
    render_system_card(ui, snapshot, t, loc);

    let width = ui.available_width();
    if width >= COLUMN_MIN_W {
        two_columns(
            ui,
            0.46,
            MID_H,
            |left| {
                render_sources_card(left, snapshot, t, loc);
            },
            |right| {
                render_activity_card(right, snapshot, t, loc);
            },
        );
        two_columns_collect(
            ui,
            0.62,
            LOWER_H,
            |left| render_top_rules_card(left, snapshot, daemon, t, loc),
            |right| {
                render_attention_card(right, daemon, t, loc);
                None
            },
        )
    } else {
        render_sources_card(ui, snapshot, t, loc);
        render_activity_card(ui, snapshot, t, loc);
        let clicked = render_top_rules_card(ui, snapshot, daemon, t, loc);
        render_attention_card(ui, daemon, t, loc);
        clicked
    }
}

fn render_kpi_row(
    ui: &mut Ui,
    state: &GuiState,
    snapshot: Option<&AnalyticsSnapshot>,
    t: &Tokens,
    loc: &Strings,
) {
    let lifetime: u64 = state
        .daemon_state
        .as_ref()
        .map(|daemon| {
            daemon
                .config
                .rules
                .iter()
                .map(|rule| rule.execution_count)
                .sum()
        })
        .unwrap_or(0);
    let session = snapshot.map_or(0, |snapshot| snapshot.session_executions);
    let active = state
        .daemon_state
        .as_ref()
        .map_or(0, |daemon| daemon.active_rule_executions.len());
    let overlays = state
        .daemon_state
        .as_ref()
        .map_or(0, |daemon| daemon.active_overlay_ids.len());
    let uptime = state.daemon_state.as_ref().map_or(0, |daemon| daemon.uptime_secs);
    let outcomes = snapshot.map(|snapshot| snapshot.outcomes);

    let success = outcomes
        .and_then(|outcomes| outcomes.success_rate())
        .map(|rate| format!("{:.1}%", rate * 100.0))
        .unwrap_or_else(|| "—".to_string());
    let success_color = match outcomes.and_then(|outcomes| outcomes.success_rate()) {
        Some(rate) if rate >= 0.99 => t.good,
        Some(rate) if rate >= 0.95 => t.warn,
        Some(_) => t.bad,
        None => t.muted,
    };
    let outcomes_caption = match outcomes {
        Some(outcomes) if outcomes.total() > 0 => format!(
            "{} {} · {} {} · {} {}",
            outcomes.succeeded,
            loc.analytics_outcome_ok,
            outcomes.failed,
            loc.analytics_outcome_failed,
            outcomes.aborted,
            loc.analytics_outcome_aborted
        ),
        _ => loc.analytics_collecting.to_string(),
    };
    let uptime_caption = format!(
        "{} {}",
        loc.analytics_kpi_uptime,
        format_uptime(uptime, loc)
    );
    let overlays_caption = loc
        .analytics_overlays
        .replace("{}", &overlays.to_string());
    // The session card carries the hourly run sparkline; the other cards stay numeric.
    let hourly: Vec<f32> = snapshot
        .map(|snapshot| {
            snapshot
                .hours
                .iter()
                .map(|hour| hour.executions as f32)
                .collect()
        })
        .unwrap_or_default();
    let hourly_spark = (hourly.len() >= 2).then_some(hourly.as_slice());

    let labels = [
        KpiCard {
            label: loc.analytics_kpi_total_executions,
            value: format_count(lifetime),
            caption: loc.analytics_caption_rule_counters.to_string(),
            value_color: t.accent_ink,
            spark: None,
        },
        KpiCard {
            label: loc.analytics_kpi_session,
            value: format_count(session),
            caption: uptime_caption,
            value_color: t.ink_strong,
            spark: hourly_spark,
        },
        KpiCard {
            label: loc.analytics_kpi_active_rules,
            value: format_count(active as u64),
            caption: overlays_caption,
            value_color: t.good,
            spark: None,
        },
        KpiCard {
            label: loc.analytics_kpi_success,
            value: success,
            caption: outcomes_caption,
            value_color: success_color,
            spark: None,
        },
    ];

    let width = ui.available_width();
    let columns = if width >= 720.0 { 4 } else { 2 };
    let card_w = ((width - GAP * (columns as f32 - 1.0)) / columns as f32).floor();
    for chunk in labels.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for card in chunk {
                render_kpi_card(ui, vec2(card_w, KPI_H), card, t);
            }
        });
    }
}

fn render_kpi_card(ui: &mut Ui, size: egui::Vec2, card: &KpiCard<'_>, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, t.card_corner(), t.sunken);
        painter.rect_stroke(rect, t.card_corner(), t.hairline(), egui::StrokeKind::Inside);
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 9.0),
            Align2::LEFT_TOP,
            card.label,
            FontId::new(9.0, theme::mono()),
            t.muted,
        );
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 26.0),
            Align2::LEFT_TOP,
            &card.value,
            FontId::new(19.0, theme::mono_bold()),
            card.value_color,
        );
        painter.text(
            pos2(rect.left() + 12.0, rect.bottom() - 6.0),
            Align2::LEFT_BOTTOM,
            &card.caption,
            FontId::new(9.0, theme::mono()),
            t.faint,
        );
    }
    if let Some(values) = card.spark {
        let spark_rect = Rect::from_min_size(
            pos2(rect.right() - 92.0, rect.top() + 8.0),
            vec2(80.0, 26.0),
        );
        let mut spark_ui = ui.new_child(egui::UiBuilder::new().max_rect(spark_rect));
        charts::sparkline(&mut spark_ui, spark_rect.size(), values, card.value_color);
    }
}

/// The last [`LIVE_CHART_SAMPLES`] samples of one live series: the collector keeps about two
/// minutes, so the one-minute chart window is always filled after a short run.
fn last_samples(values: &[f32]) -> &[f32] {
    &values[values.len().saturating_sub(LIVE_CHART_SAMPLES)..]
}

/// The daemon's own CPU, memory and disk use in one quiet line under the card title: the chart
/// above shows the machine, this line shows what the supervisor itself costs.
fn render_daemon_usage_line(
    ui: &mut Ui,
    snapshot: Option<&AnalyticsSnapshot>,
    t: &Tokens,
    loc: &Strings,
) {
    let Some(usage) = snapshot.map(|snapshot| snapshot.daemon) else {
        return;
    };
    let ram = format!("{:.1}", usage.ram_bytes as f64 / (1024.0 * 1024.0));
    let cpu = usage
        .cpu_pct
        .map(|value| loc.analytics_daemon_cpu.replace("{}", &format!("{value:.1}")))
        .unwrap_or_else(|| "—".to_string());
    let disk = loc
        .analytics_daemon_disk
        .replace("{}", &format!("{:.0}", usage.disk_bps() as f64 / 1024.0));
    let line = crate::gui::components::text::fill(loc.analytics_daemon_usage, &[&ram, &cpu, &disk]);
    widgets::hud(ui, &line, t.faint);
}

fn render_system_card(
    ui: &mut Ui,
    snapshot: Option<&AnalyticsSnapshot>,
    t: &Tokens,
    loc: &Strings,
) {
    let width = ui.available_width();
    card(ui, width, SYSTEM_H, t, |ui| {
        let (cpu, ram) = snapshot
            .map(|snapshot| (snapshot.cpu.last().copied(), snapshot.ram.last().copied()))
            .unwrap_or((None, None));
        let cpu_text = format!(
            "{} {}",
            loc.analytics_cpu,
            cpu.map(|value| format!("{value:.0}%")).unwrap_or_else(|| "—".to_string())
        );
        let ram_text = format!(
            "{} {}",
            loc.analytics_ram,
            ram.map(|value| format!("{value:.0}%")).unwrap_or_else(|| "—".to_string())
        );
        let right: Vec<(&str, Color32)> = [
            Some((cpu_text.as_str(), t.accent_ink)),
            Some((ram_text.as_str(), t.series(3))),
        ]
        .into_iter()
        .flatten()
        .collect();
        card_title(ui, loc.analytics_card_system, &right, t);
        render_daemon_usage_line(ui, snapshot, t, loc);
        let (cpu_len, ram_len) = snapshot
            .map(|snapshot| (snapshot.cpu.len(), snapshot.ram.len()))
            .unwrap_or((0, 0));
        let chart_h = (ui.available_height() - 2.0).max(60.0);
        if cpu_len < 2 && ram_len < 2 {
            empty_state(ui, chart_h, loc.analytics_collecting, t);
            return;
        }
        let cpu = snapshot
            .map(|snapshot| last_samples(&snapshot.cpu))
            .unwrap_or(&[]);
        let ram = snapshot
            .map(|snapshot| last_samples(&snapshot.ram))
            .unwrap_or(&[]);
        let series = [
            ChartSeries {
                label: loc.analytics_cpu,
                values: cpu,
                color: t.accent_ink,
                area: true,
            },
            ChartSeries {
                label: loc.analytics_ram,
                values: ram,
                color: t.series(3),
                area: false,
            },
        ];
        charts::line_chart(ui, vec2(ui.available_width(), chart_h), &series, 100.0, "%");
    });
}

fn render_sources_card(
    ui: &mut Ui,
    snapshot: Option<&AnalyticsSnapshot>,
    t: &Tokens,
    loc: &Strings,
) {
    let width = ui.available_width();
    card(ui, width, MID_H, t, |ui| {
        widgets::hud(ui, loc.analytics_card_sources, t.muted);
        let Some(snapshot) = snapshot else {
            empty_state(ui, ui.available_height(), loc.analytics_collecting, t);
            return;
        };
        let total: u32 = snapshot.sources.iter().map(|entry| entry.executions).sum();
        if total == 0 {
            empty_state(ui, ui.available_height(), loc.analytics_no_runs, t);
            return;
        }
        let segments: Vec<DonutSegment<'_>> = snapshot
            .sources
            .iter()
            .map(|entry| DonutSegment {
                value: entry.executions as f32,
                color: source_color(entry.source, t),
                label: source_label(entry.source, loc),
            })
            .collect();
        let diameter = (ui.available_height() - 4.0).min(ui.available_width() * 0.45).max(96.0);
        ui.horizontal(|ui| {
            charts::donut(
                ui,
                diameter,
                &segments,
                &format_count(total as u64),
                loc.analytics_runs,
            );
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                for entry in &snapshot.sources {
                    let share = entry.executions as f32 / total as f32 * 100.0;
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot.center(), 3.0, source_color(entry.source, t));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(source_label(entry.source, loc))
                                .family(theme::medium())
                                .size(10.0)
                                .color(t.ink),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} · {:.0}%",
                                    format_count(entry.executions as u64),
                                    share
                                ))
                                .family(theme::mono())
                                .size(9.5)
                                .color(t.faint),
                            );
                        });
                    });
                }
            });
        });
    });
}

fn render_activity_card(
    ui: &mut Ui,
    snapshot: Option<&AnalyticsSnapshot>,
    t: &Tokens,
    loc: &Strings,
) {
    let width = ui.available_width();
    card(ui, width, MID_H, t, |ui| {
        let window = snapshot.map_or(0, |snapshot| snapshot.window_executions);
        let count = format_count(window as u64);
        card_title(
            ui,
            loc.analytics_card_activity,
            &[(&count, t.ink_strong)],
            t,
        );
        let Some(snapshot) = snapshot else {
            empty_state(ui, ui.available_height(), loc.analytics_collecting, t);
            return;
        };
        if snapshot.window_executions == 0 {
            empty_state(ui, ui.available_height(), loc.analytics_no_runs, t);
            return;
        }
        let labels: Vec<String> = snapshot
            .hours
            .iter()
            .map(|hour| charts::hour_label(hour.hour_start_unix))
            .collect();
        let bars: Vec<BarDatum<'_>> = snapshot
            .hours
            .iter()
            .enumerate()
            .map(|(index, hour)| BarDatum {
                value: hour.executions,
                errors: hour.failures_total(),
                label: labels.get(index).map(String::as_str).unwrap_or(""),
            })
            .collect();
        let chart_h = (ui.available_height() - 2.0).max(70.0);
        charts::bar_chart(
            ui,
            vec2(ui.available_width(), chart_h),
            &bars,
            t.accent_ink,
            t.bad,
            |bar| {
                let runs = loc
                    .analytics_tooltip_runs
                    .replace("{}", &bar.value.to_string());
                if bar.errors > 0 {
                    format!(
                        "{}  ·  {}  ·  {}",
                        bar.label,
                        runs,
                        loc.analytics_tooltip_errors
                            .replace("{}", &bar.errors.to_string())
                    )
                } else {
                    format!("{}  ·  {}", bar.label, runs)
                }
            },
        );
    });
}

fn render_top_rules_card(
    ui: &mut Ui,
    snapshot: Option<&AnalyticsSnapshot>,
    daemon: Option<&crate::ipc::DaemonState>,
    t: &Tokens,
    loc: &Strings,
) -> Option<String> {
    let width = ui.available_width();
    let mut clicked: Option<String> = None;
    card(ui, width, LOWER_H, t, |ui| {
        widgets::hud(ui, loc.analytics_card_top_rules, t.muted);
        let Some(snapshot) = snapshot else {
            empty_state(ui, ui.available_height(), loc.analytics_collecting, t);
            return;
        };
        if snapshot.top_rules.is_empty() {
            empty_state(ui, ui.available_height(), loc.analytics_no_runs, t);
            return;
        }
        let max = snapshot
            .top_rules
            .iter()
            .map(|entry| entry.executions)
            .max()
            .unwrap_or(1)
            .max(1);
        let rules = daemon.map(|daemon| daemon.config.rules.as_slice()).unwrap_or(&[]);
        let row_w = ui.available_width();
        ui.add_space(2.0);
        for entry in &snapshot.top_rules {
            let rule = rules.iter().find(|rule| rule.id == entry.rule_id);
            let label = rule.map(|rule| rule.name.as_str()).unwrap_or(entry.rule_id.as_str());
            let (rect, response) =
                ui.allocate_exact_size(vec2(row_w, 23.0), Sense::click());
            let row = TopRuleRow {
                label,
                executions: entry.executions,
                max,
                rule,
            };
            paint_top_rule_row(ui, rect, &row, t, loc);
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() && rule.is_some() {
                clicked = Some(entry.rule_id.clone());
            }
        }
    });
    clicked
}

fn paint_top_rule_row(ui: &Ui, rect: Rect, row: &TopRuleRow<'_>, t: &Tokens, loc: &Strings) {
    let painter = ui.painter();
    if ui.rect_contains_pointer(rect) {
        painter.rect_filled(rect, t.inset_corner(), t.hover_fill());
    }
    let center_y = rect.center().y;

    // No source dot here: the row counts runs from every source, so colouring it with the
    // configured trigger would claim a cause the number does not have. The bar carries the rank.
    let name_rect = Rect::from_min_max(
        pos2(rect.left() + 5.0, rect.top()),
        pos2(rect.left() + rect.width() * 0.46, rect.bottom()),
    );
    let ink = if row.rule.is_some_and(|rule| !rule.is_enabled) {
        t.muted
    } else {
        t.ink_strong
    };
    painter.with_clip_rect(name_rect).text(
        pos2(name_rect.left(), center_y),
        Align2::LEFT_CENTER,
        row.label,
        FontId::new(11.0, theme::medium()),
        ink,
    );

    let bar_left = rect.left() + rect.width() * 0.49;
    let bar_width = (rect.width() * 0.28).max(30.0);
    let track = Rect::from_min_size(pos2(bar_left, center_y - 1.5), vec2(bar_width, 3.0));
    painter.rect_filled(track, egui::CornerRadius::same(MARK_CORNER), t.line);
    let filled = (row.executions as f32 / row.max as f32).clamp(0.0, 1.0);
    if filled > 0.0 {
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(bar_width * filled, 3.0)),
            egui::CornerRadius::same(MARK_CORNER),
            t.accent_ink,
        );
    }

    painter.text(
        pos2(rect.left() + rect.width() * 0.88, center_y),
        Align2::RIGHT_CENTER,
        format_count(row.executions as u64),
        FontId::new(10.5, theme::mono_bold()),
        ink,
    );
    let last = row
        .rule
        .map(|rule| format_last_run(rule.last_executed_at.as_deref(), loc))
        .unwrap_or_default();
    painter.text(
        pos2(rect.right(), center_y),
        Align2::RIGHT_CENTER,
        last,
        FontId::new(9.0, theme::mono()),
        t.faint,
    );
}

fn render_attention_card(
    ui: &mut Ui,
    daemon: Option<&crate::ipc::DaemonState>,
    t: &Tokens,
    loc: &Strings,
) {
    let width = ui.available_width();
    card(ui, width, LOWER_H, t, |ui| {
        widgets::hud(ui, loc.analytics_card_attention, t.muted);
        let Some(daemon) = daemon else {
            empty_state(ui, ui.available_height(), loc.settings_daemon_offline, t);
            return;
        };
        let now = chrono::Local::now();
        let mut issues: Vec<(&str, &'static str, Color32)> = Vec::new();
        for rule in &daemon.config.rules {
            if !rule.is_enabled {
                continue;
            }
            let used = rule.quota_used(now);
            if rule.max_executions.is_some_and(|limit| used >= limit) {
                issues.push((rule.name.as_str(), loc.analytics_attention_quota, t.bad));
            } else if rule.execution_count == 0 {
                issues.push((rule.name.as_str(), loc.analytics_attention_never_ran, t.warn));
            }
        }
        if issues.is_empty() {
            ui.add_space(6.0);
            widgets::hud(ui, loc.analytics_attention_clear, t.good);
            return;
        }
        let shown = issues.len().min(5);
        let row_w = ui.available_width();
        for (name, reason, color) in issues.iter().take(shown) {
            let (rect, _) = ui.allocate_exact_size(vec2(row_w, 24.0), Sense::hover());
            let painter = ui.painter();
            painter.circle_filled(
                pos2(rect.left() + 5.0, rect.center().y),
                3.0,
                *color,
            );
            let name_rect = Rect::from_min_max(
                pos2(rect.left() + 15.0, rect.top()),
                pos2(rect.left() + rect.width() * 0.62, rect.bottom()),
            );
            painter.with_clip_rect(name_rect).text(
                pos2(name_rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::new(11.0, theme::medium()),
                t.ink,
            );
            painter.with_clip_rect(rect).text(
                pos2(rect.right(), rect.center().y),
                Align2::RIGHT_CENTER,
                *reason,
                FontId::new(9.0, theme::mono()),
                *color,
            );
        }
        if issues.len() > shown {
            widgets::hud(
                ui,
                &loc
                    .analytics_attention_more
                    .replace("{}", &(issues.len() - shown).to_string()),
                t.faint,
            );
        }
    });
}

/// Entries of the activity stream the log section and the header search examine: one cap keeps
/// the search's count equal to what the section is able to show.
pub(crate) const LOG_SCAN_LIMIT: usize = 500;

/// Case-insensitive match of one log entry against an already lowercased query. The log section
/// and the header search share it, so their results and counts cannot drift apart.
pub(crate) fn log_matches_query(log: &LogItem, needle: &str) -> bool {
    needle.is_empty()
        || log.message.to_lowercase().contains(needle)
        || log.category.to_lowercase().contains(needle)
        || log.level.to_lowercase().contains(needle)
}

/// The activity-log section. The stream and the two filter fields are borrowed separately so the
/// daemon's log buffer is never cloned for a frame.
fn render_log_panel(
    ui: &mut Ui,
    logs: &[LogItem],
    query: &mut String,
    level: &mut AnalyticsLogFilter,
    t: &Tokens,
    loc: &Strings,
) {
    let total = logs.len().min(LOG_SCAN_LIMIT);
    let error_count = logs
        .iter()
        .take(LOG_SCAN_LIMIT)
        .filter(|log| log.level == "ERROR")
        .count();

    let render_filters = |ui: &mut Ui, filter: &mut AnalyticsLogFilter| {
        let options = [
            (
                AnalyticsLogFilter::All,
                widgets::Cell {
                    label: loc.logs_cell_all,
                    tip: loc.logs_cell_all_tip,
                    count: Some((total, None)),
                },
            ),
            (
                AnalyticsLogFilter::Rules,
                widgets::Cell::new(loc.logs_cell_rules, loc.logs_cell_rules_tip),
            ),
            (
                AnalyticsLogFilter::Processes,
                widgets::Cell::new(loc.logs_cell_processes, loc.logs_cell_processes_tip),
            ),
            (
                AnalyticsLogFilter::Daemon,
                widgets::Cell::new(loc.logs_cell_daemon, loc.logs_cell_daemon_tip),
            ),
            (
                AnalyticsLogFilter::Errors,
                widgets::Cell {
                    label: loc.logs_cell_errors,
                    tip: loc.logs_cell_errors_tip,
                    count: Some((error_count, (error_count > 0).then_some(t.bad))),
                },
            ),
        ];
        widgets::cells(ui, filter, &options);
    };
    // The filter/search line is separated from the stream below it by the same page spacing as
    // the cards have between each other.
    ui.spacing_mut().item_spacing.y = GAP;
    // The level filters and the text search share one line: the field starts right after the
    // last filter and takes every remaining pixel, so the log header has one continuous bar.
    ui.horizontal(|ui| {
        render_filters(ui, level);
        ui.add_space(6.0);
        let search_w = ui.available_width().max(140.0);
        // All search fields share the same background-free outlined chrome.
        widgets::catalog_search_box_sized(
            ui,
            query,
            loc.logs_search_placeholder,
            search_w,
        );
    });
    let needle = query.trim().to_lowercase();
    let selected = *level;
    let filtered: Vec<&LogItem> = logs
        .iter()
        .take(LOG_SCAN_LIMIT)
        .filter(|log| match selected {
            AnalyticsLogFilter::All => true,
            AnalyticsLogFilter::Rules => log.category.eq_ignore_ascii_case("RULE"),
            AnalyticsLogFilter::Processes => log.category.eq_ignore_ascii_case("PROCESS"),
            AnalyticsLogFilter::Daemon => {
                !log.category.eq_ignore_ascii_case("RULE")
                    && !log.category.eq_ignore_ascii_case("PROCESS")
                    && !log.level.eq_ignore_ascii_case("ERROR")
            }
            AnalyticsLogFilter::Errors => log.level.eq_ignore_ascii_case("ERROR"),
        })
        .filter(|log| log_matches_query(log, &needle))
        .collect();

    // Fill the block exactly: the next section is separated only by the page's item spacing.
    let well_h = ui.available_height().max(60.0);
    theme::well(t)
        // The vertical inset matches the cards and the spacing between sections (12 px), so the
        // stream's bottom breathing room equals the gap it has above the next block.
        .inner_margin(egui::Margin::symmetric(12, 12))
        .show(ui, |ui| {
            // The well's own inner margins are part of the space available here; sizing the
            // content to the full remaining height made the frame overflow its slice and cover
            // the bottom breathing room of the dialog.
            let content_h = (well_h - 24.0).max(20.0);
            ui.set_width(ui.available_width());
            ui.set_height(content_h);
            if filtered.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space((content_h / 2.0 - 22.0).max(10.0));
                    widgets::hud(ui, loc.logs_empty_title, t.muted);
                    ui.add_space(4.0);
                    widgets::hud(ui, loc.logs_empty_subtitle, t.faint);
                });
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("analytics-log-scroll")
                .max_height(content_h)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for log in filtered.iter().copied() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            widgets::hud(ui, &log.timestamp, t.faint);
                            let level_color = match log.level.as_str() {
                                "ERROR" => t.bad,
                                "WARN" => t.warn,
                                "TRIGGER" => t.accent_ink,
                                _ => t.good,
                            };
                            widgets::badge(ui, &log.level, level_color);
                            let category_color = match log.category.as_str() {
                                "PROCESS" => t.accent_ink,
                                "RULE" => t.warn,
                                _ => t.muted,
                            };
                            widgets::badge(ui, &log.category, category_color);
                            let message_color = if log.level == "ERROR" { t.bad } else { t.ink };
                            // A long message truncates at the right edge of the stream instead of
                            // widening its row: one unbounded line used to stretch the whole page
                            // width, and every card below it then laid itself out to that width.
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&log.message)
                                        .family(theme::mono())
                                        .size(11.5)
                                        .color(message_color),
                                )
                                .truncate(),
                            );
                        });
                    }
                });
        });
}

/// One dashboard card: the recessed surface every section shares.
fn card<R>(
    ui: &mut Ui,
    width: f32,
    height: f32,
    t: &Tokens,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let result = egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(t.card_corner())
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width((width - 30.0).max(40.0));
            ui.set_height((height - 26.0).max(0.0));
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        });
    result.inner
}

/// Two cards side by side with a fixed row height, or stacked when the dialog is narrow.
fn two_columns(
    ui: &mut Ui,
    left_fraction: f32,
    height: f32,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) {
    two_columns_collect(ui, left_fraction, height, left, right);
}

/// [`two_columns`] with both halves free to return a value; the caller keeps the left one (the
/// clicked top rule) and ignores the right one.
fn two_columns_collect<T>(
    ui: &mut Ui,
    left_fraction: f32,
    height: f32,
    left: impl FnOnce(&mut Ui) -> T,
    right: impl FnOnce(&mut Ui) -> T,
) -> T {
    let width = ui.available_width();
    let left_w = ((width - GAP) * left_fraction).floor();
    let right_w = width - GAP - left_w;
    let (row, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let left_rect = Rect::from_min_size(row.min, vec2(left_w, height));
    let right_rect = Rect::from_min_size(pos2(row.left() + left_w + GAP, row.top()), vec2(right_w, height));
    let left_value = ui
        .scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
            ui.set_width(left_w);
            ui.set_height(height);
            left(ui)
        })
        .inner;
    let _ = ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        ui.set_width(right_w);
        ui.set_height(height);
        right(ui)
    });
    left_value
}

/// The title row every dashboard card shares: the title on the left and optional right-side
/// stats on the same baseline. It is painted directly instead of inside a `ui.horizontal`, which
/// reserves a control row and pushed the titles of the two cards with right-side stats below the
/// titles of the other cards.
fn card_title(ui: &mut Ui, title: &str, right: &[(&str, Color32)], t: &Tokens) {
    let title_galley = widgets::hud_galley(ui, title, t.muted);
    let height = title_galley.size().y;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    widgets::paint_left(ui, rect, title_galley, t.muted);

    let galleys: Vec<_> = right
        .iter()
        .map(|(text, color)| (*color, widgets::hud_galley(ui, text, *color)))
        .collect();
    let painter = ui.painter();
    let mut x = rect.right();
    for (color, galley) in galleys.into_iter().rev() {
        x -= galley.size().x;
        painter.galley_with_override_text_color(pos2(x, rect.top()), galley, color);
        x -= 10.0;
    }
}

fn empty_state(ui: &mut Ui, height: f32, text: &str, t: &Tokens) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height.max(24.0)),
        Layout::top_down(Align::Center),
        |ui| {
            ui.add_space((height * 0.35).max(6.0));
            widgets::hud(ui, text, t.faint);
        },
    );
}

fn source_label(source: RunSource, loc: &Strings) -> &'static str {
    match source {
        RunSource::Process => loc.analytics_source_process,
        RunSource::Window => loc.analytics_source_window,
        RunSource::Resource => loc.analytics_source_resource,
        RunSource::Hotkey => loc.analytics_source_hotkey,
        RunSource::Schedule => loc.analytics_source_schedule,
        RunSource::System => loc.analytics_source_system,
        RunSource::File => loc.analytics_source_file,
        RunSource::Webhook => loc.analytics_source_webhook,
        RunSource::Manual => loc.analytics_source_manual,
        RunSource::RuleCall => loc.analytics_source_rule_call,
    }
}

/// Stable series colour of one run source, so the donut, the legend and the top-rules dots agree.
fn source_color(source: RunSource, t: &Tokens) -> Color32 {
    let index = RunSource::ALL
        .iter()
        .position(|candidate| *candidate == source)
        .unwrap_or(0);
    t.series(index)
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

fn format_uptime(uptime_secs: u64, loc: &Strings) -> String {
    let days = uptime_secs / 86400;
    let hours = (uptime_secs % 86400) / 3600;
    let mins = (uptime_secs % 3600) / 60;
    let secs = uptime_secs % 60;
    let (day_unit, hour_unit, min_unit, sec_unit) = (
        loc.analytics_unit_days,
        loc.analytics_unit_hours,
        loc.analytics_unit_minutes,
        loc.analytics_unit_seconds,
    );
    if days > 0 {
        format!("{days}{day_unit} {hours:02}{hour_unit} {mins:02}{min_unit}")
    } else if hours > 0 {
        format!("{hours:02}{hour_unit} {mins:02}{min_unit} {secs:02}{sec_unit}")
    } else {
        format!("{mins:02}{min_unit} {secs:02}{sec_unit}")
    }
}

fn format_last_run(timestamp: Option<&str>, loc: &Strings) -> String {
    let Some(stamp) = timestamp else {
        return loc.analytics_rule_never.to_string();
    };
    let Ok(time) = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S") else {
        return stamp.to_string();
    };
    let seconds = chrono::Local::now()
        .naive_local()
        .signed_duration_since(time)
        .num_seconds()
        .max(0);
    if seconds < 60 {
        loc.analytics_ago_now.to_string()
    } else if seconds < 3600 {
        loc.analytics_ago_minutes
            .replace("{}", &(seconds / 60).to_string())
    } else if seconds < 86400 {
        loc.analytics_ago_hours
            .replace("{}", &(seconds / 3600).to_string())
    } else {
        loc.analytics_ago_days
            .replace("{}", &(seconds / 86400).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnalyticsHour, RuleRuns, RunOutcomes, SourceRuns};
    use std::collections::{HashMap, HashSet};

    fn analytics_state() -> GuiState {
        let mut config = crate::config::AppConfig::default();
        config.rules.push(Rule {
            id: "rule_one".into(),
            name: "Macro Rule".into(),
            execution_count: 7,
            ..Default::default()
        });

        let now = chrono::Local::now().timestamp();
        let hours: Vec<AnalyticsHour> = (0..24)
            .map(|index| AnalyticsHour {
                hour_start_unix: now - (23 - index) * 3600,
                executions: (index % 3) as u32,
                failures: 0,
                aborted: 0,
            })
            .collect();

        let mut state = GuiState::new();
        state.daemon_state = Some(crate::ipc::DaemonState {
            autorun_enabled: true,
            uptime_secs: 3700,
            daemon_memory_bytes: 24 * 1024 * 1024,
            active_rules_count: 1,
            total_rules_count: 1,
            config,
            logs: vec![],
            active_rule_executions: HashMap::new(),
            active_overlay_ids: HashSet::new(),
            active_test_rule_ids: HashSet::new(),
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });
        state.analytics = Some(AnalyticsSnapshot {
            session_executions: 12,
            window_executions: 7,
            outcomes: RunOutcomes {
                succeeded: 6,
                failed: 1,
                aborted: 0,
            },
            sources: vec![SourceRuns {
                source: RunSource::Schedule,
                executions: 7,
            }],
            hours,
            top_rules: vec![RuleRuns {
                rule_id: "rule_one".into(),
                executions: 7,
            }],
            cpu: vec![12.0, 18.0, 22.0],
            ram: vec![40.0, 42.0, 44.0],
            daemon: crate::models::DaemonUsage {
                cpu_pct: Some(0.4),
                ram_bytes: 24 * 1024 * 1024,
                disk_read_bps: 8 * 1024,
                disk_write_bps: 4 * 1024,
            },
        });
        state
    }

    /// The dialog paints its log section and dashboard in one frame; this also
    /// exercises the chart painters and the modal slices with a real snapshot.
    #[test]
    fn test_analytics_modal_renders_dashboard_and_log_panel() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = analytics_state();
        state.show_analytics_modal = true;
        let loc = crate::i18n::get(crate::i18n::Language::English);
        // Modal content lives in an Area, and egui drains area layers on the following pass, so
        // the dialog is rendered twice and the second frame is inspected.
        let mut output = None;
        for time in [0.0, 0.1] {
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1180.0, 760.0))),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    render_analytics_modal(ui.ctx(), &mut state);
                },
            );
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let output = output.expect("two frames were rendered");

        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        for needle in [
            loc.analytics_title,
            loc.analytics_kpi_total_executions,
            loc.analytics_kpi_session,
            loc.analytics_card_sources,
            loc.analytics_card_top_rules,
            loc.logs_empty_title,
        ] {
            assert!(
                texts.iter().any(|text| text == needle),
                "{needle:?} was not painted: {texts:?}"
            );
        }

        // The daemon line carries its own CPU, memory and disk use, taken from the snapshot and
        // assembled from the catalog rather than a view literal. `hud` renders it upper-cased.
        let cpu_part = loc.analytics_daemon_cpu.replace("{}", "0.4").to_uppercase();
        let disk_part = loc.analytics_daemon_disk.replace("{}", "12").to_uppercase();
        assert!(
            texts.iter().any(|text| text.contains("DAEMON 24.0")
                && text.contains(&cpu_part)
                && text.contains(&disk_part)),
            "the daemon usage line was not painted: {texts:?}"
        );
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
            _ => {}
        }
    }

    /// The caption under a KPI value must clear the value's descenders; they used to touch at the
    /// old card height.
    #[test]
    fn kpi_caption_has_a_gap_below_the_value() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(
            &ctx,
            crate::config::AppTheme::Dark,
            crate::config::UiSkin::Modern,
            None,
        );
        let t = Tokens::get(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 120.0))),
                ..Default::default()
            },
            |ui| {
                let card = KpiCard {
                    label: "LABEL",
                    value: "7".to_string(),
                    caption: "CAPTION".to_string(),
                    value_color: t.ink_strong,
                    spark: None,
                };
                render_kpi_card(ui, vec2(240.0, KPI_H), &card, &t);
            },
        );
        output.textures_delta.clear();

        let mut value_bottom = None;
        let mut caption_top = None;
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                match text.galley.text() {
                    "7" => value_bottom = Some(text.pos.y + text.galley.size().y),
                    "CAPTION" => caption_top = Some(text.pos.y),
                    _ => {}
                }
            }
        }
        let value_bottom = value_bottom.expect("the value is painted");
        let caption_top = caption_top.expect("the caption is painted");
        assert!(
            caption_top - value_bottom >= 2.0,
            "the caption starts at {caption_top}, the value ends at {value_bottom}"
        );
    }

    fn painted_title_top(
        ctx: &egui::Context,
        title: &str,
        mut render: impl FnMut(&mut Ui),
    ) -> f32 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 400.0))),
                ..Default::default()
            },
            |ui| render(ui),
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == title => Some(text.pos.y),
                _ => None,
            })
            .unwrap_or_else(|| panic!("title {title:?} was not painted"))
    }

    /// Every dashboard card paints its title at the same inset. The two cards with right-side
    /// stats used to sit a few pixels lower because their title lived in a `ui.horizontal`.
    #[test]
    fn card_titles_share_one_top_inset() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let t = Tokens::of(crate::config::AppTheme::Dark, crate::config::UiSkin::Modern);
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let state = analytics_state();
        let snapshot = state.analytics.as_ref();
        let daemon = state.daemon_state.as_ref();

        let tops = [
            painted_title_top(&ctx, loc.analytics_card_system, |ui| {
                render_system_card(ui, snapshot, &t, loc)
            }),
            painted_title_top(&ctx, loc.analytics_card_sources, |ui| {
                render_sources_card(ui, snapshot, &t, loc)
            }),
            painted_title_top(&ctx, loc.analytics_card_activity, |ui| {
                render_activity_card(ui, snapshot, &t, loc)
            }),
            painted_title_top(&ctx, loc.analytics_card_top_rules, |ui| {
                let _ = render_top_rules_card(ui, snapshot, daemon, &t, loc);
            }),
            painted_title_top(&ctx, loc.analytics_card_attention, |ui| {
                render_attention_card(ui, daemon, &t, loc)
            }),
        ];
        let min = tops.iter().copied().fold(f32::INFINITY, f32::min);
        let max = tops.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max - min <= 1.0,
            "card titles do not share one top inset: {tops:?}"
        );
        assert!(max <= 14.0, "the title inset grew beyond the card margin: {tops:?}");
    }

    #[test]
    fn counts_are_grouped_with_thin_spaces() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1 000");
        assert_eq!(format_count(12_480), "12 480");
        assert_eq!(format_count(1_234_567), "1 234 567");
    }

    #[test]
    fn the_live_chart_shows_the_last_minute() {
        let samples: Vec<f32> = (0..120).map(|value| value as f32).collect();
        let shown = last_samples(&samples);
        assert_eq!(shown.len(), LIVE_CHART_SAMPLES);
        assert_eq!(shown.last().copied(), Some(119.0));
        assert_eq!(last_samples(&samples[..30]).len(), 30, "a short series is shown whole");
        assert!(last_samples(&[]).is_empty());
    }

    #[test]
    fn uptime_formats_by_the_largest_unit() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let (day, hour, minute, second) = (
            en.analytics_unit_days,
            en.analytics_unit_hours,
            en.analytics_unit_minutes,
            en.analytics_unit_seconds,
        );
        assert_eq!(format_uptime(59, en), format!("00{minute} 59{second}"));
        assert_eq!(
            format_uptime(3_661, en),
            format!("01{hour} 01{minute} 01{second}")
        );
        assert_eq!(format_uptime(90_061, en), format!("1{day} 01{hour} 01{minute}"));
    }

    #[test]
    fn last_run_falls_back_to_never_and_relative_units() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        assert_eq!(
            format_last_run(None, en),
            en.analytics_rule_never
        );
        let now = chrono::Local::now();
        let recent = (now - chrono::Duration::seconds(30))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        assert_eq!(format_last_run(Some(&recent), en), en.analytics_ago_now);
        let minutes = (now - chrono::Duration::minutes(5))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        assert_eq!(
            format_last_run(Some(&minutes), en),
            en.analytics_ago_minutes.replace("{}", "5")
        );
        let malformed = "not-a-timestamp";
        assert_eq!(format_last_run(Some(malformed), en), malformed);
    }

    /// Every source has its own label and colour, and every series colour is opaque: a donut slice
    /// can never read as an unexplained or washed-out colour block.
    #[test]
    fn every_run_source_has_a_label_and_a_distinct_colour() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);
        for t in [Tokens::DARK, Tokens::LIGHT] {
            let mut seen = std::collections::HashSet::new();
            for source in RunSource::ALL {
                assert!(!source_label(source, en).is_empty());
                assert!(!source_label(source, ru).is_empty());
                let color = source_color(source, &t);
                assert_eq!(color.a(), 255, "a series colour must be opaque");
                assert!(
                    seen.insert(color.to_array()),
                    "two sources share one series colour"
                );
            }
        }
    }

    /// A message far wider than the dialog: the stream has to truncate it rather than lay out to
    /// the full width of the text.
    const LONG_LOG_MESSAGE: &str =
        "A logged event whose message is far wider than the analytics dialog, which is exactly \
         what used to push the dashboard blocks past the right edge of the dialog";

    /// [`analytics_state`] filled with the densest realistic content: long log messages, long rule
    /// names, every run source and a full top-rules ranking.
    fn dense_analytics_state(language: crate::i18n::Language) -> GuiState {
        let mut state = analytics_state();
        state.language = language;
        let daemon = state
            .daemon_state
            .as_mut()
            .expect("the fixture carries a daemon state");
        daemon.config.rules.clear();
        for index in 0..8 {
            daemon.config.rules.push(Rule {
                id: format!("rule_{index}"),
                name: format!(
                    "Rule with an extremely long descriptive name #{index} that a real rule may carry"
                ),
                execution_count: 0,
                max_executions: Some(3),
                ..Default::default()
            });
        }
        daemon.logs = (0..24)
            .map(|index| crate::models::LogItem {
                timestamp: format!("2026-01-01 12:{index:02}:00"),
                level: if index % 4 == 0 { "ERROR" } else { "INFO" }.to_string(),
                category: "RULE".to_string(),
                message: LONG_LOG_MESSAGE.to_string(),
            })
            .collect();
        let snapshot = state.analytics.as_mut().expect("the fixture carries a snapshot");
        snapshot.sources = RunSource::ALL
            .iter()
            .enumerate()
            .map(|(index, source)| SourceRuns {
                source: *source,
                executions: (index as u32 + 1) * 111,
            })
            .collect();
        snapshot.top_rules = (0..8)
            .map(|index| RuleRuns {
                rule_id: format!("rule_{index}"),
                executions: 9_000 - index as u32,
            })
            .collect();
        state
    }

    /// Renders the dialog until the open transition has settled and returns the last frame plus
    /// the card rect painted for the dialog.
    fn settled_analytics_frame(
        ctx: &egui::Context,
        state: &mut GuiState,
        screen: Rect,
    ) -> (egui::FullOutput, Rect) {
        let mut output = None;
        for time in [0.0, 0.1, 0.3, 0.5, 0.8] {
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    render_analytics_modal(ui.ctx(), state);
                },
            );
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let output = output.expect("the dialog rendered");
        let card = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == Tokens::DARK.bg => Some(rect.rect),
                _ => None,
            })
            .max_by(|left, right| {
                (left.width() * left.height()).total_cmp(&(right.width() * right.height()))
            })
            .expect("the dialog card is painted");
        (output, card)
    }

    /// A long log line truncates at the right edge of the stream instead of widening the page.
    /// One unbounded message used to stretch the page's layout width, and every card below it
    /// then laid itself out to that width and painted over the dialog's right inset.
    #[test]
    fn long_log_messages_never_widen_the_page() {
        for (screen, language) in [
            // The smallest window the app allows, where the dashboard rows stack.
            (vec2(880.0, 580.0), crate::i18n::Language::Russian),
            (vec2(1280.0, 900.0), crate::i18n::Language::English),
        ] {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            let mut state = dense_analytics_state(language);
            state.show_analytics_modal = true;
            let (output, card) = settled_analytics_frame(
                &ctx,
                &mut state,
                Rect::from_min_size(pos2(0.0, 0.0), screen),
            );
            let page_right = card.right() - SIDE_INSET;

            let mut offenders: Vec<String> = Vec::new();
            let mut truncated_rows = 0;
            for clipped in &output.shapes {
                // The dialog surface, its shadow and its dividers are clipped wider than the card
                // itself; every other shape belongs to the page.
                if clipped.clip_rect.right() > card.right() + 0.5 {
                    continue;
                }
                let visible = clipped
                    .shape
                    .visual_bounding_rect()
                    .intersect(clipped.clip_rect);
                if !visible.is_positive() {
                    continue;
                }
                if visible.right() > page_right + 0.5 {
                    offenders.push(format!("{visible:?}"));
                }
                if let egui::Shape::Text(text) = &clipped.shape {
                    if text.galley.text() == LONG_LOG_MESSAGE && text.galley.elided {
                        truncated_rows += 1;
                    }
                }
            }
            assert!(
                offenders.is_empty(),
                "{language:?} at {screen:?}: {} shapes cross the page's right inset (the page ends \
                 at {page_right}, the card at {}): {:?}",
                offenders.len(),
                card.right(),
                &offenders[..offenders.len().min(6)]
            );
            assert!(
                truncated_rows > 0,
                "{language:?} at {screen:?}: no log line was truncated, the stream widened instead"
            );
        }
    }
}
