//! Schedule trigger: type tabs, presets, weekday and date-time pickers.

use super::*;

/// Toggle one weekday in a days-of-week list. An empty list means "every day", so the
/// first toggle expands it to all seven days before removing the clicked one.
pub(crate) fn toggle_day_of_week(days_of_week: &mut Vec<u8>, day: u8) {
    if days_of_week.is_empty() {
        *days_of_week = (1..=7).collect();
        days_of_week.retain(|&d| d != day);
    } else if days_of_week.contains(&day) {
        days_of_week.retain(|&d| d != day);
    } else {
        days_of_week.push(day);
        days_of_week.sort_unstable();
    }
}

/// Weekday selector shared by the schedule trigger and the time-range condition.
pub(crate) fn render_days_of_week_selector(ui: &mut Ui, loc: &crate::i18n::Strings, days_of_week: &mut Vec<u8>) {
    let dow_names = [
        (1, loc.inspector_dow_mon),
        (2, loc.inspector_dow_tue),
        (3, loc.inspector_dow_wed),
        (4, loc.inspector_dow_thu),
        (5, loc.inspector_dow_fri),
        (6, loc.inspector_dow_sat),
        (7, loc.inspector_dow_sun),
    ];

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (val, label) in dow_names {
            let is_on = days_of_week.is_empty() || days_of_week.contains(&val);
            if widgets::button(ui, label, if is_on { Kind::Primary } else { Kind::Secondary }, 0.0).clicked() {
                toggle_day_of_week(days_of_week, val);
            }
        }
        let all_active = days_of_week.is_empty() || days_of_week.len() == 7;
        if widgets::button(ui, loc.inspector_btn_all_days, if all_active { Kind::Primary } else { Kind::Secondary }, 0.0).clicked() {
            days_of_week.clear();
        }
    });
}

pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

pub(crate) fn render_datetime_picker(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    target_datetime: &mut Option<String>,
    t: &Tokens,
    id_salt: &str,
) {
    let now = chrono::Local::now();
    // An unset timestamp is rendered as tomorrow noon but is written into the draft only
    // once the user actually picks a day or edits a field; opening a rule must not dirty it.
    let fallback_day = now.date_naive().succ_opt().unwrap_or(now.date_naive());

    let (mut sel_year, mut sel_month, mut sel_day, mut sel_hour, mut sel_min, mut sel_sec) = {
        let dt_str = target_datetime.as_deref().unwrap_or("");
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S") {
            (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second())
        } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&format!("{}:00", dt_str), "%Y-%m-%d %H:%M:%S") {
            (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 0)
        } else {
            (fallback_day.year(), fallback_day.month(), fallback_day.day(), 12, 0, 0)
        }
    };

    let view_month_id = ui.scope_id().with(id_salt).with("cal_view_month");
    let view_year_id = ui.scope_id().with(id_salt).with("cal_view_year");
    let mut view_month: u32 = ui.data_mut(|d| d.get_temp(view_month_id).unwrap_or(sel_month));
    let mut view_year: i32 = ui.data_mut(|d| d.get_temp(view_year_id).unwrap_or(sel_year));

    widgets::hud(ui, loc.inspector_mini_calendar, t.muted);
    ui.horizontal(|ui| {
        if widgets::button(ui, "◀", Kind::Secondary, 24.0).clicked() {
            if view_month == 1 {
                view_month = 12;
                view_year -= 1;
            } else {
                view_month -= 1;
            }
            ui.data_mut(|d| {
                d.insert_temp(view_month_id, view_month);
                d.insert_temp(view_year_id, view_year);
            });
        }

        let month_name = if (1..=12).contains(&view_month) {
            loc.inspector_cal_month_names[(view_month - 1) as usize]
        } else {
            ""
        };
        ui.label(egui::RichText::new(format!("{} {}", month_name, view_year)).strong());

        if widgets::button(ui, "▶", Kind::Secondary, 24.0).clicked() {
            if view_month == 12 {
                view_month = 1;
                view_year += 1;
            } else {
                view_month += 1;
            }
            ui.data_mut(|d| {
                d.insert_temp(view_month_id, view_month);
                d.insert_temp(view_year_id, view_year);
            });
        }
    });

    if let Some(first_day) = chrono::NaiveDate::from_ymd_opt(view_year, view_month, 1) {
        let start_col = first_day.weekday().number_from_monday() as usize;
        let days_in_cur_month = days_in_month(view_year, view_month);

        let two_digit_w = (widgets::caps(ui, "00", theme::HUD_SIZE, theme::mono_bold(), theme::BUTTON_SPACING, t.ink).size().x + 28.0).max(44.0);

        egui::Grid::new(ui.scope_id().with(id_salt).with("mini_cal_grid"))
            .spacing(vec2(3.0, 3.0))
            .show(ui, |ui| {
                for d in loc.inspector_cal_day_names {
                    let (rect, _) = ui.allocate_exact_size(vec2(two_digit_w, 18.0), Sense::hover());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        d,
                        egui::FontId::new(11.0, theme::mono()),
                        t.muted,
                    );
                }
                ui.end_row();

                let mut col = 1;
                for _ in 1..start_col {
                    ui.allocate_exact_size(vec2(two_digit_w, theme::CONTROL_H), Sense::hover());
                    col += 1;
                }

                for day in 1..=days_in_cur_month {
                    let is_selected = sel_year == view_year && sel_month == view_month && sel_day == day;
                    let day_str = format!("{}", day);
                    let btn = widgets::button(
                        ui,
                        &day_str,
                        if is_selected { Kind::Primary } else { Kind::Secondary },
                        two_digit_w,
                    );
                    if btn.clicked() {
                        sel_year = view_year;
                        sel_month = view_month;
                        sel_day = day;
                        *target_datetime = Some(format!(
                            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                            sel_year, sel_month, sel_day, sel_hour, sel_min, sel_sec
                        ));
                    }
                    col += 1;
                    if col > 7 {
                        ui.end_row();
                        col = 1;
                    }
                }
                if col != 1 {
                    ui.end_row();
                }
            });
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        widgets::hud(ui, loc.inspector_execution_time, t.muted);
        let h_ch = ui.add(
            egui::DragValue::new(&mut sel_hour)
                .range(0..=23)
                .speed(0.1)
                .suffix(" h"),
        ).changed();
        ui.label(":");
        let m_ch = ui.add(
            egui::DragValue::new(&mut sel_min)
                .range(0..=59)
                .speed(0.1)
                .suffix(" m"),
        ).changed();
        ui.label(":");
        let s_ch = ui.add(
            egui::DragValue::new(&mut sel_sec)
                .range(0..=59)
                .speed(0.1)
                .suffix(" s"),
        ).changed();

        if h_ch || m_ch || s_ch {
            *target_datetime = Some(format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                sel_year, sel_month, sel_day, sel_hour, sel_min, sel_sec
            ));
        }
    });
}

/// Interval presets as one table: the label, the value it sets and its tooltip.
/// Keeping them together means a label change can no longer silently drop the tip.
pub(crate) fn interval_presets(loc: &crate::i18n::Strings) -> [(&'static str, u64, &'static str); 7] {
    [
        ("1S", 1, loc.inspector_preset_tip_1s),
        ("1M", 60, loc.inspector_preset_tip_1m),
        ("1H", 3600, loc.inspector_preset_tip_1h),
        ("1D", 86400, loc.inspector_preset_tip_1d),
        ("1W", 604800, loc.inspector_preset_tip_1w),
        ("1MO", 2592000, loc.inspector_preset_tip_1mo),
        ("1Y", 31536000, loc.inspector_preset_tip_1y),
    ]
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_schedule_trigger(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    schedule_type: ScheduleType,
    interval_sec: &mut u64,
    daily_time: &mut Option<String>,
    days_of_week: &mut Vec<u8>,
    day_of_month: &mut Option<u8>,
    target_datetime: &mut Option<String>,
    t: &Tokens,
) {
    match schedule_type {
        ScheduleType::Interval => {
            widgets::hud(ui, loc.inspector_preset_intervals, t.muted);

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (label, secs, tip) in interval_presets(loc) {
                    let on = *interval_sec == secs;
                    let btn = widgets::button(ui, label, if on { Kind::Primary } else { Kind::Secondary }, 0.0)
                        .on_hover_text(tip);
                    if btn.clicked() {
                        *interval_sec = secs;
                    }
                }
            });

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_custom_interval, t.muted);

            let s = *interval_sec;
            let mut hours = s / 3600;
            let mut mins = (s % 3600) / 60;
            let mut secs = s % 60;

            ui.horizontal_wrapped(|ui| {
                let h_ch = ui.add(
                    egui::DragValue::new(&mut hours)
                        .range(0..=u64::MAX / 3600)
                        .speed(0.2)
                        .suffix(" h"),
                ).changed();
                ui.label(":");
                let m_ch = ui.add(
                    egui::DragValue::new(&mut mins)
                        .range(0..=59)
                        .speed(0.2)
                        .suffix(" m"),
                ).changed();
                ui.label(":");
                let s_ch = ui.add(
                    egui::DragValue::new(&mut secs)
                        .range(0..=59)
                        .speed(0.2)
                        .suffix(" s"),
                ).changed();

                if h_ch || m_ch || s_ch {
                    let total = hours
                        .saturating_mul(3600)
                        .saturating_add(mins.saturating_mul(60))
                        .saturating_add(secs);
                    *interval_sec = if total == 0 { 1 } else { total };
                }
            });

            ui.add_space(8.0);
            widgets::hrule(ui);
            let mut has_start_time = target_datetime.is_some();
            if ui.checkbox(&mut has_start_time, loc.inspector_checkbox_first_execution).changed() {
                if has_start_time {
                    let now = chrono::Local::now();
                    *target_datetime = Some(format!(
                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                        now.year(), now.month(), now.day(), now.hour(), now.minute(), now.second()
                    ));
                } else {
                    *target_datetime = None;
                }
            }

            if has_start_time {
                ui.add_space(4.0);
                render_datetime_picker(ui, loc, target_datetime, t, "interval_cal");
            }
        }

        ScheduleType::Daily => {
            widgets::hud(ui, loc.inspector_active_days_of_week, t.muted);
            render_days_of_week_selector(ui, loc, days_of_week);

            ui.add_space(4.0);

            let cur_str = daily_time.as_deref().unwrap_or("12:00:00");
            let (mut hour, mut min, mut sec) = {
                let parts: Vec<&str> = cur_str.split(':').collect();
                let h = parts.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(12);
                let m = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                let s = parts.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                (h, m, s)
            };

            widgets::hud(ui, loc.inspector_daily_execution_time, t.muted);
            ui.horizontal_wrapped(|ui| {
                let h_ch = ui.add(
                    egui::DragValue::new(&mut hour)
                        .range(0..=23)
                        .speed(0.1)
                        .suffix(" h"),
                ).changed();
                ui.label(":");
                let m_ch = ui.add(
                    egui::DragValue::new(&mut min)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" m"),
                ).changed();
                ui.label(":");
                let s_ch = ui.add(
                    egui::DragValue::new(&mut sec)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" s"),
                ).changed();

                if h_ch || m_ch || s_ch {
                    *daily_time = Some(format!("{:02}:{:02}:{:02}", hour, min, sec));
                }
            });
        }

        ScheduleType::Monthly => {
            widgets::hud(ui, loc.inspector_sched_day_of_month, t.muted);
            // Unset renders as day 1; the value is stored only when the user changes it.
            let mut cur_day = day_of_month.unwrap_or(1);
            if ui.add(
                egui::DragValue::new(&mut cur_day)
                    .range(1..=31)
                    .speed(0.1)
                    .suffix(" d"),
            ).changed() {
                *day_of_month = Some(cur_day.clamp(1, 31));
            }

            ui.add_space(8.0);

            let cur_str = daily_time.as_deref().unwrap_or("12:00:00");
            let (mut hour, mut min, mut sec) = {
                let parts: Vec<&str> = cur_str.split(':').collect();
                let h = parts.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(12);
                let m = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                let s = parts.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                (h, m, s)
            };

            widgets::hud(ui, loc.inspector_daily_execution_time, t.muted);
            ui.horizontal_wrapped(|ui| {
                let h_ch = ui.add(
                    egui::DragValue::new(&mut hour)
                        .range(0..=23)
                        .speed(0.1)
                        .suffix(" h"),
                ).changed();
                ui.label(":");
                let m_ch = ui.add(
                    egui::DragValue::new(&mut min)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" m"),
                ).changed();
                ui.label(":");
                let s_ch = ui.add(
                    egui::DragValue::new(&mut sec)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" s"),
                ).changed();

                if h_ch || m_ch || s_ch {
                    *daily_time = Some(format!("{:02}:{:02}:{:02}", hour, min, sec));
                }
            });
        }

        ScheduleType::OneShot => {
            render_datetime_picker(ui, loc, target_datetime, t, "oneshot_cal");
        }

        _ => {}
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interval_preset_tips_travel_with_the_entry() {
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let presets = interval_presets(loc);
        assert_eq!(presets.len(), 7);
        for (label, seconds, tip) in presets {
            assert!(!label.is_empty());
            assert!(seconds > 0);
            assert!(!tip.is_empty(), "preset {label} lost its tooltip");
        }
        assert_eq!(presets[1], ("1M", 60, loc.inspector_preset_tip_1m));
    }

    #[test]
    fn test_day_of_week_toggle_expands_all_days_first() {
        let mut days: Vec<u8> = Vec::new();
        toggle_day_of_week(&mut days, 3);
        assert_eq!(days, vec![1, 2, 4, 5, 6, 7]);
        toggle_day_of_week(&mut days, 3);
        assert_eq!(days, vec![1, 2, 3, 4, 5, 6, 7]);
        toggle_day_of_week(&mut days, 3);
        assert_eq!(days, vec![1, 2, 4, 5, 6, 7]);
    }
}
