//! `process` action field editors.

use super::*;

pub(crate) fn render_start_process_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let state = ctx.state;
    let t = ctx.t;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let Action::StartProcess { file_path, arguments, working_directory, run_as_admin, only_if_not_running, run_hidden, close_on_exit, } = action else { return; };
    widgets::hud(ui, loc.inspector_field_executable_path, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
            if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter(loc.inspector_dialog_filter_executables, &["exe", "bat", "cmd", "lnk", "ps1"])
                    .pick_file()
                {
                    *file_path = file.to_string_lossy().to_string();
                }
            }
            ui.add(
                egui::TextEdit::singleline(file_path)
                    .hint_text(loc.inspector_field_executable_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::control_height(ui)))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });

    widgets::hud(ui, loc.inspector_field_arguments, t.muted);
    ui.add(
        egui::TextEdit::singleline(arguments)
            .hint_text(loc.inspector_field_arguments_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );

    widgets::hud(ui, loc.inspector_field_working_directory, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_w = widgets::button_width(ui, loc.inspector_btn_folder, 0.0);
            if widgets::button(ui, loc.inspector_btn_folder, Kind::Secondary, btn_w).clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    *working_directory = Some(folder.to_string_lossy().to_string());
                }
            }
            let mut wd_str = working_directory.clone().unwrap_or_default();
            if ui.add(
                egui::TextEdit::singleline(&mut wd_str)
                    .hint_text(loc.inspector_field_working_directory_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::control_height(ui)))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            ).changed() {
                *working_directory = if wd_str.trim().is_empty() { None } else { Some(wd_str.clone()) };
            }
        });
    });

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(run_hidden, loc.inspector_checkbox_run_hidden);
        ui.checkbox(only_if_not_running, loc.inspector_checkbox_only_if_not_running);
        ui.checkbox(run_as_admin, loc.inspector_checkbox_run_as_admin);
        if !is_exit_pipeline {
            let is_process_trigger = state.editing_rule.as_ref().is_some_and(|r| matches!(r.trigger, Some(Trigger::Process { .. })));
            if !is_process_trigger {
                *close_on_exit = false;
            }
            let resp = ui.add_enabled(
                is_process_trigger,
                egui::Checkbox::new(close_on_exit, loc.inspector_checkbox_close_on_exit),
            );
            if !is_process_trigger {
                resp.on_disabled_hover_text(loc.inspector_companion_disabled_tip);
            } else {
                resp.on_hover_text(loc.inspector_checkbox_close_on_exit_tip);
            }
        }
    });
}

pub(crate) fn render_kill_process_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let default_picker_target = ctx.default_picker_target;
    let Action::KillProcess { process_name, kill_tree, force } = action else { return; };
    widgets::hud(ui, loc.inspector_field_terminate_process_name, t.muted);
    if render_process_name_row(ui, loc, process_name, loc.inspector_field_terminate_hint) {
        resp.pick_target = Some(default_picker_target);
    }

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(kill_tree, loc.inspector_checkbox_kill_tree);
        ui.checkbox(force, loc.inspector_checkbox_force_terminate);
    });
}

pub(crate) fn render_process_tuning_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let default_picker_target = ctx.default_picker_target;
    let Action::ProcessTuning { target_process, priority, affinity_mask: _, trim_memory: _, } = action else { return; };
    widgets::hud(ui, loc.inspector_field_target_process, t.muted);
    if render_optional_process_name_row(ui, loc, target_process, loc.inspector_hint_tuning_process) {
        resp.pick_target = Some(default_picker_target);
    }

    widgets::hud(ui, loc.inspector_field_priority, t.muted);
    let combo_id = ui.scope_id().with(idx).with("pri_combo");
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(priority_label(priority, loc))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(priority, None, loc.inspector_priority_unchanged);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::Idle), loc.inspector_priority_idle);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::BelowNormal), loc.inspector_priority_below_normal);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::Normal), loc.inspector_priority_normal);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::AboveNormal), loc.inspector_priority_above_normal);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::High), loc.inspector_priority_high);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::Realtime), loc.inspector_priority_realtime);
        });
}
