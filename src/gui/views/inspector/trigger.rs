//! §02 trigger section: selector header, variants and system-event presentation.

use super::*;

/// The trigger section of the rule editor: header, run controls and every trigger variant.
pub(crate) fn render_trigger_section(ui: &mut Ui, state: &mut GuiState, rule: &mut Rule, t: &Tokens) {
    let loc = state.loc();
    let is_manual = rule.trigger.as_ref().is_none_or(|t| matches!(t, Trigger::Manual));

    // 2. Trigger Section
    widgets::eyebrow(ui, "02", loc.inspector_eyebrow_trigger);

    let mut switch_trigger_to_hunter = false;

    egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_max_width(ui.available_width());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                let (glyph, title, glyph_color) = match &rule.trigger {
                    Some(Trigger::Process { .. }) => (egui_phosphor::regular::CPU, loc.inspector_trigger_process, t.good),
                    Some(Trigger::WindowFocus { .. }) => (egui_phosphor::regular::APP_WINDOW, loc.inspector_trigger_window, t.accent_ink),
                    Some(Trigger::Schedule { schedule_type, .. }) => match schedule_type {
                        ScheduleType::AppStartup => (egui_phosphor::regular::ROCKET_LAUNCH, loc.inspector_schedule_app_start, t.good),
                        ScheduleType::AppShutdown => (egui_phosphor::regular::POWER, loc.inspector_schedule_app_close, t.warn),
                        ScheduleType::Interval => (egui_phosphor::regular::TIMER, loc.inspector_sched_tab_interval, t.accent_ink),
                        ScheduleType::Daily => (egui_phosphor::regular::CALENDAR, loc.inspector_sched_tab_daily, t.good),
                        ScheduleType::OneShot => (egui_phosphor::regular::CALENDAR_CHECK, loc.inspector_sched_tab_specific, t.muted),
                        ScheduleType::Monthly => (egui_phosphor::regular::CALENDAR_BLANK, loc.inspector_sched_tab_monthly, t.accent_ink),
                    },
                    Some(Trigger::SystemEvent { event_type, .. }) => {
                        let meta = system_event_meta(*event_type, loc, t);
                        (meta.glyph, meta.label, meta.tint)
                    },
                    Some(Trigger::ResourceThreshold { .. }) => (egui_phosphor::regular::CHART_LINE, loc.inspector_trigger_resource, t.warn),
                    Some(Trigger::GlobalHotkey { .. }) => (egui_phosphor::regular::KEYBOARD, loc.inspector_trigger_hotkey, t.accent_ink),
                    Some(Trigger::FileWatcher { .. }) => (egui_phosphor::regular::FILE, loc.inspector_trigger_file, t.accent_ink),
                    Some(Trigger::Webhook { .. }) => (egui_phosphor::regular::GLOBE, loc.inspector_trigger_webhook, t.good),
                    Some(Trigger::Manual) | None => (egui_phosphor::regular::PLAY_CIRCLE, loc.inspector_manual_title, t.accent_ink),
                };

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(glyph)
                            .family(theme::phosphor())
                            .size(16.0)
                            .color(glyph_color),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(title)
                            .family(theme::semibold())
                            .size(12.5)
                            .color(t.ink),
                    );

                    let exec_opt = state.get_rule_execution_state(rule);
                    let is_running = exec_opt.is_some();
                    let is_test_running = state.active_test_runs.contains_key(&rule.id);
                    let is_stoppable = if is_manual { is_running } else { is_test_running };
                    let run_label = if is_stoppable { loc.rules_btn_stop } else { loc.rules_btn_test_run };
                    let run_kind = if is_stoppable { Kind::Danger } else { Kind::Secondary };

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, loc.inspector_btn_change_trigger, Kind::Secondary, 0.0).clicked() {
                            state.open_trigger_catalog();
                        }
                        if widgets::button(ui, run_label, run_kind, 0.0).clicked() {
                            if is_stoppable {
                                state.stop_rule_execution(&rule.id);
                            } else {
                                state.test_run_rule(&rule.id);
                            }
                        }
                    });
                });

                widgets::hrule(ui);

                if is_manual {
                    render_trigger_info_frame(ui, egui_phosphor::regular::INFO, loc.inspector_manual_desc, t.accent_ink, t);
                } else {
                    match &mut rule.trigger {
                    Some(Trigger::Process { process_name, match_mode, single_instance }) => {
                                widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                                if render_process_name_row(ui, loc, process_name, loc.inspector_field_target_process_hint) {
                                    switch_trigger_to_hunter = true;
                                }

                                ui.add_space(6.0);
                                widgets::hud(ui, loc.inspector_field_match_mode, t.muted);
                                let match_options = [
                                    (ProcessMatchMode::Exact, Cell::new(loc.inspector_match_tab_exact, "")),
                                    (ProcessMatchMode::Wildcard, Cell::new(loc.inspector_match_tab_wildcard, "")),
                                    (ProcessMatchMode::Regex, Cell::new(loc.inspector_match_tab_regex, "")),
                                ];
                                widgets::cells(ui, match_mode, &match_options);

                                ui.add_space(6.0);
                                widgets::checkbox(ui, single_instance, loc.inspector_checkbox_single_instance);
                    }

                    Some(Trigger::WindowFocus { process_name, on_focus_gained, on_focus_lost }) => {
                        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                        if render_process_name_row(ui, loc, process_name, loc.inspector_field_target_process_hint) {
                            switch_trigger_to_hunter = true;
                        }

                        ui.add_space(4.0);
                        widgets::hud(ui, loc.inspector_field_sys_event_type, t.muted);
                        widgets::checkbox_group(ui, |ui| {
                            widgets::checkbox_row(ui, |ui| {
                                if ui.checkbox(on_focus_gained, loc.inspector_focus_gained).changed()
                                    && !*on_focus_gained && !*on_focus_lost
                                {
                                    *on_focus_lost = true;
                                }
                                if ui.checkbox(on_focus_lost, loc.inspector_focus_lost).changed()
                                    && !*on_focus_lost && !*on_focus_gained
                                {
                                    *on_focus_gained = true;
                                }
                            });
                        });
                    }

                    Some(Trigger::Schedule { schedule_type, interval_sec, daily_time, days_of_week, day_of_month, target_datetime }) => {
                        match schedule_type {
                            ScheduleType::AppStartup | ScheduleType::AppShutdown => {
                                let mut is_startup = *schedule_type == ScheduleType::AppStartup;
                                let options = [
                                    (true, Cell::new(loc.inspector_schedule_app_start, "")),
                                    (false, Cell::new(loc.inspector_schedule_app_close, "")),
                                ];
                                if widgets::cells(ui, &mut is_startup, &options) {
                                    *schedule_type = if is_startup { ScheduleType::AppStartup } else { ScheduleType::AppShutdown };
                                }
                                ui.add_space(4.0);

                                if *schedule_type == ScheduleType::AppStartup {
                                    render_trigger_info_frame(
                                        ui,
                                        egui_phosphor::regular::ROCKET_LAUNCH,
                                        loc.inspector_schedule_app_start_desc,
                                        t.good,
                                        t,
                                    );
                                } else {
                                    render_trigger_info_frame(
                                        ui,
                                        egui_phosphor::regular::POWER,
                                        loc.inspector_schedule_app_close_desc,
                                        t.warn,
                                        t,
                                    );
                                }
                            }
                            _ => {
                                render_schedule_trigger(
                                    ui,
                                    loc,
                                    *schedule_type,
                                    interval_sec,
                                    daily_time,
                                    days_of_week,
                                    day_of_month,
                                    target_datetime,
                                    t,
                                );
                            }
                        }
                    }

                    Some(Trigger::SystemEvent { event_type, .. }) => {
                        render_system_event_pair_toggle(ui, event_type, loc, t);
                        let meta = system_event_meta(*event_type, loc, t);
                        render_trigger_info_frame(ui, meta.glyph, meta.description, meta.tint, t);
                    }
                    Some(Trigger::ResourceThreshold { process_name, cpu_percent, ram_mb, duration_sec }) => {
                        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                        if render_process_name_row(ui, loc, process_name, loc.inspector_hint_process_executable) {
                            switch_trigger_to_hunter = true;
                        }

                        ui.add_space(6.0);
                        widgets::checkbox_row(ui, |ui| {
                            let mut has_cpu = cpu_percent.is_some();
                            if ui.checkbox(&mut has_cpu, loc.inspector_cond_res_cpu).changed() {
                                *cpu_percent = if has_cpu { Some(80.0) } else { None };
                            }
                            if let Some(cpu) = cpu_percent {
                                ui.add(egui::DragValue::new(cpu).range(1.0..=100.0).suffix("%"));
                            }

                            ui.add_space(10.0);
                            let mut has_mem = ram_mb.is_some();
                            if ui.checkbox(&mut has_mem, loc.inspector_cond_res_ram).changed() {
                                *ram_mb = if has_mem { Some(1024) } else { None };
                            }
                            if let Some(mem) = ram_mb {
                                ui.add(egui::DragValue::new(mem).range(10..=65536).suffix(" MB"));
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            widgets::hud(ui, loc.inspector_cond_res_duration, t.muted);
                            widgets::duration_secs_u64(ui, loc, "trigger_res_duration", duration_sec, 1.0..=3600.0);
                        });
                    }

                    Some(Trigger::GlobalHotkey { key_combination, hold_mode, repeat_while_held }) => {
                        render_hotkey_trigger_fields(ui, key_combination, hold_mode, repeat_while_held, loc, t);
                    }
                    Some(Trigger::FileWatcher { folder_path, filter_extension, on_created, on_modified, on_deleted }) => {
                        widgets::hud(ui, loc.inspector_field_watch_path, t.muted);
                        ui.horizontal(|ui| {
                            ui.set_width(ui.available_width());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::button(ui, loc.inspector_btn_choose_file, Kind::Secondary, 0.0).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                                        *folder_path = f.to_string_lossy().to_string();
                                    }
                                }
                                if widgets::button(ui, loc.inspector_btn_choose_folder, Kind::Secondary, 0.0).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().pick_folder() {
                                        *folder_path = f.to_string_lossy().to_string();
                                    }
                                }
                                ui.add(
                                    egui::TextEdit::singleline(folder_path)
                                        .hint_text(loc.inspector_hint_watch_path)
                                        .desired_width(ui.available_width())
                                        .min_size(vec2(0.0, theme::CONTROL_H))
                                        .align(egui::Align2::LEFT_CENTER)
                                        .margin(vec2(10.0, 8.0)),
                                );
                            });
                        });

                        ui.add_space(4.0);
                        widgets::hud(ui, loc.inspector_field_filter_ext, t.muted);
                        let mut ext = filter_extension.clone().unwrap_or_default();
                        let max_w = ui.available_width().min(240.0);
                        if ui.add(
                            egui::TextEdit::singleline(&mut ext)
                                .hint_text(loc.inspector_hint_filter_ext)
                                .desired_width(max_w)
                                .min_size(vec2(0.0, theme::CONTROL_H))
                                .align(egui::Align2::LEFT_CENTER)
                                .margin(vec2(10.0, 8.0)),
                        ).changed() {
                            *filter_extension = if ext.trim().is_empty() { None } else { Some(ext.trim().to_string()) };
                        }

                        ui.add_space(4.0);
                        widgets::checkbox_group(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.checkbox(on_created, loc.inspector_file_event_created);
                                ui.checkbox(on_modified, loc.inspector_file_event_modified);
                                ui.checkbox(on_deleted, loc.inspector_file_event_deleted);
                            });
                        });
                    }

                    Some(Trigger::Webhook { endpoint_path }) => {
                        widgets::hud(ui, loc.inspector_field_webhook_endpoint, t.muted);
                        ui.add(
                            egui::TextEdit::singleline(endpoint_path)
                                .hint_text(loc.inspector_hint_webhook_endpoint)
                                .desired_width(ui.available_width())
                                .min_size(vec2(0.0, theme::CONTROL_H))
                                .align(egui::Align2::LEFT_CENTER)
                                .margin(vec2(10.0, 8.0)),
                        );

                        let endpoint_display = if endpoint_path.trim().is_empty() {
                            rule.id.as_str()
                        } else {
                            endpoint_path.as_str()
                        };
                        let full_url = format!("http://127.0.0.1:48123/api/v1/trigger/{}", endpoint_display.trim_start_matches('/'));

                        ui.add_space(6.0);
                        let url_label = egui::RichText::new(&full_url)
                            .font(egui::FontId::new(12.0, theme::mono()))
                            .color(t.accent_ink)
                            .underline();
                        let resp = ui.add(egui::Label::new(url_label).sense(egui::Sense::click()))
                            .on_hover_text(loc.inspector_webhook_copy_url)
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            crate::sharing::copy_text_to_clipboard(ui.ctx(), &full_url);
                            state.set_toast(loc.toast_url_copied);
                        }
                    }

                    Some(Trigger::Manual) | None => {}
                }
            }
        });
    });

    if switch_trigger_to_hunter {
        state.open_process_picker(crate::gui::state::ProcessPickerTarget::Trigger);
    }
}

pub(crate) fn render_trigger_info_frame(ui: &mut Ui, glyph: &'static str, desc: &str, tint: Color32, t: &Tokens) {
    egui::Frame::new()
        .fill(t.bg)
        .stroke(t.hairline())
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(
                    egui::RichText::new(glyph)
                        .family(theme::phosphor())
                        .size(18.0)
                        .color(tint),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(desc)
                            .size(11.5)
                            .color(t.ink_strong),
                    ).wrap(),
                );
            });
        });
}

/// Glyph, short label, long description and accent tint of one system event.
#[derive(Clone, Copy)]
pub(crate) struct SystemEventMeta {
    pub(crate) glyph: &'static str,
    pub(crate) label: &'static str,
    pub(crate) description: &'static str,
    pub(crate) tint: Color32,
}

/// Single source of truth for the system-event presentation: the trigger header, the
/// trigger info card and the condition card all read this table.
pub(crate) fn system_event_meta(event_type: SystemEventType, loc: &crate::i18n::Strings, t: &Tokens) -> SystemEventMeta {
    match event_type {
        SystemEventType::SessionLock => SystemEventMeta {
            glyph: egui_phosphor::regular::LOCK,
            label: loc.inspector_sys_event_lock,
            description: loc.inspector_sys_event_desc_lock,
            tint: t.accent_ink,
        },
        SystemEventType::SessionUnlock => SystemEventMeta {
            glyph: egui_phosphor::regular::LOCK_OPEN,
            label: loc.inspector_sys_event_unlock,
            description: loc.inspector_sys_event_desc_unlock,
            tint: t.accent_ink,
        },
        SystemEventType::SystemSleep => SystemEventMeta {
            glyph: egui_phosphor::regular::MOON,
            label: loc.inspector_sys_event_sleep,
            description: loc.inspector_sys_event_desc_sleep,
            tint: t.accent_ink,
        },
        SystemEventType::SystemResume => SystemEventMeta {
            glyph: egui_phosphor::regular::SUN,
            label: loc.inspector_sys_event_resume,
            description: loc.inspector_sys_event_desc_resume,
            tint: t.good,
        },
        SystemEventType::PowerSourceAc => SystemEventMeta {
            glyph: egui_phosphor::regular::PLUG,
            label: loc.inspector_sys_event_ac,
            description: loc.inspector_sys_event_desc_ac,
            tint: t.good,
        },
        SystemEventType::PowerSourceBattery => SystemEventMeta {
            glyph: egui_phosphor::regular::BATTERY_CHARGING,
            label: loc.inspector_sys_event_battery,
            description: loc.inspector_sys_event_desc_battery,
            tint: t.warn,
        },
        SystemEventType::UserIdle => SystemEventMeta {
            glyph: egui_phosphor::regular::HOURGLASS,
            label: loc.inspector_sys_event_idle,
            description: loc.inspector_sys_event_desc_idle,
            tint: t.muted,
        },
        SystemEventType::UserActive => SystemEventMeta {
            glyph: egui_phosphor::regular::ACTIVITY,
            label: loc.inspector_sys_event_active,
            description: loc.inspector_sys_event_desc_active,
            tint: t.good,
        },
        SystemEventType::DeviceConnected => SystemEventMeta {
            glyph: egui_phosphor::regular::HARD_DRIVES,
            label: loc.inspector_sys_event_dev_connected,
            description: loc.inspector_sys_event_desc_dev_connected,
            tint: t.good,
        },
        SystemEventType::DeviceDisconnected => SystemEventMeta {
            glyph: egui_phosphor::regular::PLUG,
            label: loc.inspector_sys_event_dev_disconnected,
            description: loc.inspector_sys_event_desc_dev_disconnected,
            tint: t.warn,
        },
        SystemEventType::MonitorOn => SystemEventMeta {
            glyph: egui_phosphor::regular::DESKTOP,
            label: loc.inspector_sys_event_mon_on,
            description: loc.inspector_sys_event_desc_mon_on,
            tint: t.good,
        },
        SystemEventType::MonitorOff => SystemEventMeta {
            glyph: egui_phosphor::regular::MOON_STARS,
            label: loc.inspector_sys_event_mon_off,
            description: loc.inspector_sys_event_desc_mon_off,
            tint: t.muted,
        },
        SystemEventType::NetworkConnected => SystemEventMeta {
            glyph: egui_phosphor::regular::WIFI_HIGH,
            label: loc.inspector_sys_event_net_connected,
            description: loc.inspector_sys_event_desc_net_connected,
            tint: t.good,
        },
        SystemEventType::NetworkDisconnected => SystemEventMeta {
            glyph: egui_phosphor::regular::WIFI_SLASH,
            label: loc.inspector_sys_event_net_disconnected,
            description: loc.inspector_sys_event_desc_net_disconnected,
            tint: t.bad,
        },
    }
}

/// Mutually exclusive system-event pairs offered as one two-cell toggle.
pub(crate) const SYSTEM_EVENT_PAIRS: [(SystemEventType, SystemEventType); 7] = [
    (SystemEventType::SessionLock, SystemEventType::SessionUnlock),
    (SystemEventType::SystemSleep, SystemEventType::SystemResume),
    (SystemEventType::PowerSourceAc, SystemEventType::PowerSourceBattery),
    (SystemEventType::UserIdle, SystemEventType::UserActive),
    (SystemEventType::DeviceConnected, SystemEventType::DeviceDisconnected),
    (SystemEventType::MonitorOn, SystemEventType::MonitorOff),
    (SystemEventType::NetworkConnected, SystemEventType::NetworkDisconnected),
];

/// Two-cell toggle for the pair an event belongs to. Unpaired events render nothing.
pub(crate) fn render_system_event_pair_toggle(
    ui: &mut Ui,
    event_type: &mut SystemEventType,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) -> bool {
    let Some((first, second)) = SYSTEM_EVENT_PAIRS
        .iter()
        .copied()
        .find(|(first, second)| *event_type == *first || *event_type == *second)
    else {
        return false;
    };

    let mut is_first = *event_type == first;
    let options = [
        (true, Cell::new(system_event_meta(first, loc, t).label, "")),
        (false, Cell::new(system_event_meta(second, loc, t).label, "")),
    ];
    let changed = widgets::cells(ui, &mut is_first, &options);
    if changed {
        *event_type = if is_first { first } else { second };
    }
    ui.add_space(4.0);
    changed
}
