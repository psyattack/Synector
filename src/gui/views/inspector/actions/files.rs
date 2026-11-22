//! `files` action field editors.

use super::*;

pub(crate) fn render_execute_script_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::ExecuteScript { interpreter, script, run_in_window } = action else { return; };
    ui.horizontal(|ui| {
        widgets::hud(ui, loc.inspector_script_interpreter, t.muted);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::glyph_button(ui, Glyph::Expand, loc.inspector_script_expand_tip).clicked() {
                resp.expand_script = Some(match interpreter {
                    crate::models::ScriptType::PowerShell => ScriptLanguage::PowerShell,
                    crate::models::ScriptType::Batch => ScriptLanguage::Batch,
                    crate::models::ScriptType::Lua => ScriptLanguage::Lua,
                });
            }
        });
    });

    let script_options = [
        (crate::models::ScriptType::PowerShell, Cell::new(loc.inspector_script_type_powershell, loc.inspector_script_type_powershell_tip)),
        (crate::models::ScriptType::Batch, Cell::new(loc.inspector_script_type_cmd, loc.inspector_script_type_cmd_tip)),
        (crate::models::ScriptType::Lua, Cell::new(loc.inspector_script_type_lua, loc.inspector_script_type_lua_tip)),
    ];
    widgets::cells(ui, interpreter, &script_options);

    ui.add_space(4.0);
    let lang = match interpreter {
        crate::models::ScriptType::PowerShell => ScriptLanguage::PowerShell,
        crate::models::ScriptType::Batch => ScriptLanguage::Batch,
        crate::models::ScriptType::Lua => ScriptLanguage::Lua,
    };
    let mut layouter = create_script_layouter(lang, *t, 12.0);
    ui.add(
        egui::TextEdit::multiline(script)
            .hint_text(loc.inspector_script_placeholder)
            .font(egui::FontId::new(12.0, theme::mono()))
            .desired_width(ui.available_width())
            .desired_rows(4)
            .margin(vec2(8.0, 6.0))
            .layouter(&mut layouter),
    );

    if *interpreter != crate::models::ScriptType::Lua {
        widgets::checkbox(ui, run_in_window, loc.inspector_checkbox_visible_console);
    }
}

pub(crate) fn render_archive_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::Archive { operation, format, source_path, destination_path, overwrite, .. } = action else { return; };
    let is_pack = matches!(operation, crate::models::ArchiveOperation::Pack);
    let ext = archive_format_ext(*format);

    widgets::hud(ui, loc.inspector_archive_operation, t.muted);
    let op_options = [
        (crate::models::ArchiveOperation::Pack, Cell::new(loc.inspector_archive_op_pack, "")),
        (crate::models::ArchiveOperation::Unpack, Cell::new(loc.inspector_archive_op_unpack, "")),
    ];
    widgets::cells(ui, operation, &op_options);
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_archive_format, t.muted);
    let fmt_options = ARCHIVE_FORMAT_ORDER.map(|f| (f, archive_format_cell(f, loc)));
    widgets::cells(ui, format, &fmt_options);
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_archive_source, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if is_pack {
                if widgets::button(ui, loc.inspector_cond_file_btn_folder, Kind::Secondary, 0.0).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        *source_path = folder.to_string_lossy().to_string();
                    }
                }
                if widgets::button(ui, loc.inspector_cond_file_btn_file, Kind::Secondary, 0.0).clicked() {
                    if let Some(file) = rfd::FileDialog::new().pick_file() {
                        *source_path = file.to_string_lossy().to_string();
                    }
                }
            } else {
                let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter(loc.inspector_dialog_filter_archive, &["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2"])
                        .add_filter(loc.inspector_dialog_filter_all, &["*"])
                        .pick_file()
                    {
                        *source_path = file.to_string_lossy().to_string();
                    }
                }
            }
            let src_hint = if is_pack {
                loc.inspector_hint_archive_source_pack
            } else {
                loc.inspector_hint_archive_source_unpack
            };
            ui.add(
                egui::TextEdit::singleline(source_path)
                    .hint_text(src_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_archive_destination, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
            if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                if is_pack {
                    let filter_label = loc.inspector_dialog_filter_archive_typed.replace("{}", &ext.to_uppercase());
                    let default_name = format!("archive.{}", ext);
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter(&filter_label, &[ext])
                        .set_file_name(&default_name)
                        .save_file()
                    {
                        let mut p_str = p.to_string_lossy().to_string();
                        let suffix = format!(".{}", ext);
                        if !p_str.to_lowercase().ends_with(&suffix) {
                            p_str.push_str(&suffix);
                        }
                        *destination_path = p_str;
                    }
                } else if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    *destination_path = folder.to_string_lossy().to_string();
                }
            }
            let dst_hint = if is_pack {
                loc.inspector_hint_archive_dest_pack
            } else {
                loc.inspector_hint_archive_dest_unpack
            };
            ui.add(
                egui::TextEdit::singleline(destination_path)
                    .hint_text(dst_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    ui.add_space(4.0);

    widgets::checkbox(ui, overwrite, loc.inspector_archive_overwrite);
}

pub(crate) fn render_file_system_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let Action::FileSystem { operation, path, target_path, content, create_parents, overwrite, } = action else { return; };
    widgets::hud(ui, loc.inspector_fs_operation, t.muted);
    let combo_id = ui.scope_id().with(idx).with("fs_op_combo");
    let op_text = match operation {
        crate::models::FileSystemOperation::CreateFile => loc.inspector_fs_op_create_file,
        crate::models::FileSystemOperation::CreateDirectory => loc.inspector_fs_op_create_dir,
        crate::models::FileSystemOperation::Rename => loc.inspector_fs_op_rename,
        crate::models::FileSystemOperation::Copy => loc.inspector_fs_op_copy,
        crate::models::FileSystemOperation::Delete => loc.inspector_fs_op_delete,
        crate::models::FileSystemOperation::WriteText => loc.inspector_fs_op_write_text,
        crate::models::FileSystemOperation::AppendText => loc.inspector_fs_op_append_text,
    };
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(op_text)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(operation, crate::models::FileSystemOperation::CreateFile, loc.inspector_fs_op_create_file);
            ui.selectable_value(operation, crate::models::FileSystemOperation::CreateDirectory, loc.inspector_fs_op_create_dir);
            ui.selectable_value(operation, crate::models::FileSystemOperation::Rename, loc.inspector_fs_op_rename);
            ui.selectable_value(operation, crate::models::FileSystemOperation::Copy, loc.inspector_fs_op_copy);
            ui.selectable_value(operation, crate::models::FileSystemOperation::Delete, loc.inspector_fs_op_delete);
            ui.selectable_value(operation, crate::models::FileSystemOperation::WriteText, loc.inspector_fs_op_write_text);
            ui.selectable_value(operation, crate::models::FileSystemOperation::AppendText, loc.inspector_fs_op_append_text);
        });
    ui.add_space(4.0);

    let fs_op = *operation;
    widgets::hud(ui, loc.inspector_fs_path, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match fs_op {
                crate::models::FileSystemOperation::CreateDirectory => {
                    let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                    if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            *path = p.to_string_lossy().to_string();
                        }
                    }
                }
                crate::models::FileSystemOperation::CreateFile => {
                    let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                    if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                        if let Some(p) = rfd::FileDialog::new().save_file() {
                            *path = p.to_string_lossy().to_string();
                        }
                    }
                }
                _ => {
                    if widgets::button(ui, loc.inspector_cond_file_btn_folder, Kind::Secondary, 0.0).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            *path = folder.to_string_lossy().to_string();
                        }
                    }
                    if widgets::button(ui, loc.inspector_cond_file_btn_file, Kind::Secondary, 0.0).clicked() {
                        if let Some(file) = rfd::FileDialog::new().pick_file() {
                            *path = file.to_string_lossy().to_string();
                        }
                    }
                }
            }
            let p_hint = match fs_op {
                crate::models::FileSystemOperation::CreateDirectory => loc.inspector_hint_fs_new_folder,
                crate::models::FileSystemOperation::CreateFile => loc.inspector_hint_fs_new_file,
                _ => loc.inspector_hint_fs_path,
            };
            ui.add(
                egui::TextEdit::singleline(path)
                    .hint_text(p_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    ui.add_space(4.0);

    if matches!(operation, crate::models::FileSystemOperation::Rename | crate::models::FileSystemOperation::Copy) {
        widgets::hud(ui, loc.inspector_fs_target_path, t.muted);
        let mut tgt = target_path.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, loc.inspector_cond_file_btn_folder, Kind::Secondary, 0.0).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        *target_path = Some(folder.to_string_lossy().to_string());
                    }
                }
                if widgets::button(ui, loc.inspector_cond_file_btn_file, Kind::Secondary, 0.0).clicked() {
                    if let Some(file) = rfd::FileDialog::new().pick_file() {
                        *target_path = Some(file.to_string_lossy().to_string());
                    }
                }
                if ui.add(
                    egui::TextEdit::singleline(&mut tgt)
                        .hint_text(loc.inspector_hint_fs_target_path)
                        .desired_width(ui.available_width())
                        .min_size(vec2(0.0, theme::CONTROL_H))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(10.0, 8.0)),
                ).changed() {
                    *target_path = if tgt.trim().is_empty() { None } else { Some(tgt.clone()) };
                }
            });
        });
        ui.add_space(4.0);
    }

    if matches!(operation, crate::models::FileSystemOperation::WriteText | crate::models::FileSystemOperation::AppendText) {
        widgets::hud(ui, loc.inspector_fs_content, t.muted);
        let mut text_body = content.clone().unwrap_or_default();
        if ui.add(
            egui::TextEdit::multiline(&mut text_body)
                .hint_text(loc.inspector_hint_fs_content)
                .desired_width(ui.available_width())
                .desired_rows(4)
                .margin(vec2(10.0, 8.0)),
        ).changed() {
            *content = Some(text_body);
        }
        ui.add_space(4.0);
    }

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(create_parents, loc.inspector_fs_create_parents);
        ui.checkbox(overwrite, loc.inspector_fs_overwrite);
    });
}
