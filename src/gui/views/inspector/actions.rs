//! Per-action field editors, dispatched from [`render_action_fields`].

use super::*;

mod files;
mod flow;
mod media;
mod network;
mod overlay;
mod process;
mod r#macro;
mod system;

pub(crate) use files::*;
pub(crate) use flow::*;
pub(crate) use media::*;
pub(crate) use network::*;
pub(crate) use overlay::*;
pub(crate) use process::*;
pub(crate) use r#macro::*;
pub(crate) use system::*;

/// One action's lifetime selector, shared by `start_process` and `overlay` steps.
///
/// `UntilTriggerProcessExit` is only offered when the rule fires on process start; choosing it on
/// another trigger would produce a resource nothing could ever release.
pub(crate) fn render_resource_lifetime(
    ui: &mut Ui,
    t: &Tokens,
    loc: &crate::i18n::Strings,
    idx: usize,
    lifetime: &mut crate::models::ResourceLifetime,
    can_follow_trigger: bool,
) {
    use crate::models::ResourceLifetime;

    if *lifetime == ResourceLifetime::UntilTriggerProcessExit && !can_follow_trigger {
        *lifetime = ResourceLifetime::Detach;
    }
    widgets::hud(ui, loc.inspector_lifetime_label, t.muted);
    let id = ui.scope_id().with(idx).with("resource_lifetime_combo");
    egui::ComboBox::from_id_salt(id)
        .selected_text(lifetime_label(*lifetime, loc))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(
                lifetime,
                ResourceLifetime::Detach,
                loc.inspector_lifetime_detach,
            )
            .on_hover_text(loc.inspector_lifetime_detach_tip);
            ui.selectable_value(
                lifetime,
                ResourceLifetime::UntilRunEnd,
                loc.inspector_lifetime_until_run_end,
            )
            .on_hover_text(loc.inspector_lifetime_until_run_end_tip);
            let host = ui.add_enabled(
                can_follow_trigger,
                egui::Button::new(loc.inspector_lifetime_until_trigger_exit).selected(
                    *lifetime == ResourceLifetime::UntilTriggerProcessExit,
                ),
            );
            if host.clicked() {
                *lifetime = ResourceLifetime::UntilTriggerProcessExit;
            }
            if can_follow_trigger {
                host.on_hover_text(loc.inspector_lifetime_until_trigger_exit_tip);
            } else {
                host.on_disabled_hover_text(loc.inspector_lifetime_trigger_exit_disabled);
            }
        });
}

fn lifetime_label(
    lifetime: crate::models::ResourceLifetime,
    loc: &crate::i18n::Strings,
) -> &'static str {
    use crate::models::ResourceLifetime;
    match lifetime {
        ResourceLifetime::Detach => loc.inspector_lifetime_detach,
        ResourceLifetime::UntilRunEnd => loc.inspector_lifetime_until_run_end,
        ResourceLifetime::UntilTriggerProcessExit => loc.inspector_lifetime_until_trigger_exit,
    }
}

/// Format cell for the archive editor: the label is localized and the tooltip is empty.
pub(crate) fn archive_format_cell(format: crate::models::ArchiveFormat, loc: &crate::i18n::Strings) -> Cell<'static> {
    Cell::new(archive_format_label(format, loc), "")
}

/// Lays one preset selector out in the space it may occupy. `ComboBox::width` is only a minimum:
/// its selected text extends the button, so a long preset name would widen the whole step card.
/// The selector therefore lives in an exact-width slot and truncates its text there; the slot
/// width is the row width minus whatever the call site reserves on the right.
pub(crate) fn preset_selector_slot(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(width, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        add,
    );
}

/// The right-hand "open" pill of a studio banner. Returns its left edge, so the banner title can
/// be truncated before the pill instead of painting over it.
pub(crate) fn paint_studio_banner_pill(
    painter: &egui::Painter,
    text: &str,
    right: f32,
    center_y: f32,
    color: Color32,
) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::new(11.0, theme::mono_bold()), color);
    let left = right - galley.size().x;
    painter.galley(pos2(left, center_y - galley.size().y * 0.5), galley, color);
    left
}

/// The middle title of a studio banner, truncated at `max_right`: it carries the preset name,
/// which can be longer than the banner and must not paint past the card or over the pill.
pub(crate) fn paint_studio_banner_title(
    painter: &egui::Painter,
    text: &str,
    left: f32,
    center_y: f32,
    max_right: f32,
    color: Color32,
) {
    let format = egui::TextFormat {
        font_id: egui::FontId::new(13.0, theme::semibold()),
        color,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), format);
    job.wrap = egui::text::TextWrapping::truncate_at_width((max_right - left).max(1.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(left, center_y - galley.size().y * 0.5), galley, color);
}

pub(crate) fn archive_format_ext(format: crate::models::ArchiveFormat) -> &'static str {
    use crate::models::ArchiveFormat;
    match format {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::SevenZip => "7z",
        ArchiveFormat::TarGz => "tar.gz",
        ArchiveFormat::Tar => "tar",
        ArchiveFormat::TarXz => "tar.xz",
        ArchiveFormat::Rar => "rar",
    }
}

/// Order used by the format cells; every variant appears exactly once.
pub(crate) const ARCHIVE_FORMAT_ORDER: [crate::models::ArchiveFormat; 6] = [
    crate::models::ArchiveFormat::Zip,
    crate::models::ArchiveFormat::SevenZip,
    crate::models::ArchiveFormat::TarGz,
    crate::models::ArchiveFormat::Tar,
    crate::models::ArchiveFormat::TarXz,
    crate::models::ArchiveFormat::Rar,
];

/// Shared inputs of the per-action field editors.
pub(crate) struct ActionFieldsCtx<'a> {
    pub(crate) state: &'a GuiState,
    pub(crate) t: &'a Tokens,
    pub(crate) idx: usize,
    pub(crate) default_picker_target: crate::gui::state::ProcessPickerTarget,
    pub(crate) depth: usize,
}

#[derive(Default)]
pub(crate) struct ActionFieldsResponse {
    pub expand_script: Option<ScriptLanguage>,
    pub expand_macro: bool,
    pub expand_overlay: Option<String>,
    pub expand_overlay_action: Option<usize>,
    pub switch_to_hunter: bool,
    pub open_catalog_nested: Option<crate::gui::state::IfElseCatalogTarget>,
    pub open_condition_catalog: Option<crate::gui::state::ConditionCatalogTarget>,
    pub pick_target: Option<crate::gui::state::ProcessPickerTarget>,
    pub tested_overlay_ids: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_action_parameter_fields(
    ui: &mut Ui,
    state: &GuiState,
    action: &mut Action,
    idx: usize,
    default_picker_target: crate::gui::state::ProcessPickerTarget,
    t: &Tokens,
    depth: usize,
) -> ActionFieldsResponse {
    let ctx = ActionFieldsCtx {
        state,
        t,
        idx,
        default_picker_target,
        depth,
    };
    render_action_fields(ui, &ctx, action)
}

/// Short dispatcher: one call per `Action` variant. Each variant keeps its own field
/// function so the layout, widget ids and defaults stay readable and reviewable.
pub(crate) fn render_action_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    action: &mut Action,
) -> ActionFieldsResponse {
    let mut resp = ActionFieldsResponse::default();
    match action {
        Action::StartProcess { .. } => render_start_process_fields(ui, ctx, &mut resp, action),
        Action::KillProcess { .. } => render_kill_process_fields(ui, ctx, &mut resp, action),
        Action::Notification { .. } => render_notification_fields(ui, ctx, &mut resp, action),
        Action::SystemPower { .. } => render_system_power_fields(ui, ctx, &mut resp, action),
        Action::Wait { .. } => render_wait_fields(ui, ctx, &mut resp, action),
        Action::Check { .. } => render_check_fields(ui, ctx, &mut resp, action),
        Action::HttpWebhook { .. } => render_http_webhook_fields(ui, ctx, &mut resp, action),
        Action::Window { .. } => render_window_fields(ui, ctx, &mut resp, action),
        Action::ProcessTuning { .. } => render_process_tuning_fields(ui, ctx, &mut resp, action),
        Action::PowerPlan { .. } => render_power_plan_fields(ui, ctx, &mut resp, action),
        Action::PlaySound { .. } => render_play_sound_fields(ui, ctx, &mut resp, action),
        Action::Firewall { .. } => render_firewall_fields(ui, ctx, &mut resp, action),
        Action::Display { .. } => render_display_fields(ui, ctx, &mut resp, action),
        Action::AudioControl { .. } => render_audio_control_fields(ui, ctx, &mut resp, action),
        Action::AudioFormat { .. } => render_audio_format_fields(ui, ctx, &mut resp, action),
        Action::DefaultDevice { .. } => render_default_device_fields(ui, ctx, &mut resp, action),
        Action::ExecuteScript { .. } => render_execute_script_fields(ui, ctx, &mut resp, action),
        Action::Macro { .. } => render_macro_fields(ui, ctx, &mut resp, action),
        Action::IfElse { .. } => render_if_else_fields(ui, ctx, &mut resp, action),
        Action::TrimWorkingSet { .. } => render_trim_working_set_fields(ui, ctx, &mut resp, action),
        Action::DiscordRpcSet { .. } => render_discord_rpc_set_fields(ui, ctx, &mut resp, action),
        Action::DiscordRpcClear => render_discord_rpc_clear_fields(ui, ctx, &mut resp, action),
        Action::Archive { .. } => render_archive_fields(ui, ctx, &mut resp, action),
        Action::FileSystem { .. } => render_file_system_fields(ui, ctx, &mut resp, action),
        Action::Overlay { .. } => render_overlay_fields(ui, ctx, &mut resp, action),
        Action::RuleControl { .. } => render_rule_control_fields(ui, ctx, &mut resp, action),
    }
    resp
}

pub(crate) fn render_rule_control_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let state = ctx.state;
    let t = ctx.t;
    let idx = ctx.idx;
    let Action::RuleControl { rule_id, operation, wait } = action else { return; };
    widgets::hud(ui, loc.inspector_rule_operation, t.muted);
    let operation_options = [
        (
            crate::models::RuleControlOperation::Call,
            widgets::Cell::new(loc.inspector_rule_op_call, loc.inspector_rule_op_call_desc),
        ),
        (
            crate::models::RuleControlOperation::Stop,
            widgets::Cell::new(loc.inspector_rule_op_stop, loc.inspector_rule_op_stop_desc),
        ),
    ];
    widgets::cells(ui, operation, &operation_options);

    ui.add_space(4.0);
    render_rule_target_picker(ui, state, rule_id, idx, t);

    if *operation == crate::models::RuleControlOperation::Call {
        ui.add_space(4.0);
        widgets::checkbox_group(ui, |ui| {
            ui.checkbox(wait, loc.inspector_rule_wait)
                .on_hover_text(loc.inspector_rule_wait_tip);
        });
    } else {
        // A stopped rule is never awaited: the mode switch must not leave a stale flag.
        *wait = false;
    }
}

pub(crate) fn render_full_width_action_button(
    ui: &mut Ui,
    label: &str,
    tip: &str,
    t: &Tokens,
) -> bool {
    widgets::full_width_action_button(ui, label, tip, t)
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::gui::state::{GuiState, ProcessPickerTarget};
    use crate::models::RuleControlOperation;


    #[test]
    fn test_rule_control_action_editor_renders() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);

        // The picker lists rules from the daemon snapshot or, when there is none, from disk.
        let target = Rule {
            id: "rule_target".into(),
            name: "Target Rule".into(),
            ..Default::default()
        };
        crate::config::RulesFile::save_rules_to(
            &crate::config::AppConfig::config_dir(),
            std::slice::from_ref(&target),
        )
        .expect("write the isolated test rules file");

        let state = GuiState::new();
        let mut action = Action::RuleControl {
            rule_id: String::new(),
            operation: RuleControlOperation::Call,
            wait: true,
        };

        for operation in [RuleControlOperation::Call, RuleControlOperation::Stop] {
            if let Action::RuleControl { operation: current, .. } = &mut action {
                *current = operation;
            }
            for _ in 0..3 {
                let mut out = ctx.run_ui(Default::default(), |ctx| {
                    let t = Tokens::get(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = render_action_parameter_fields(
                            ui,
                            &state,
                            &mut action,
                            0,
                            ProcessPickerTarget::Action(0),
                            &t,
                            0,
                        );
                    });
                });
                out.textures_delta.clear();
            }
        }

        match action {
            Action::RuleControl { rule_id, wait, .. } => {
                assert_eq!(rule_id, "", "rendering must not auto-select a target rule");
                assert!(!wait, "the wait flag is cleared when the operation is stop");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_archive_format_cells_cover_every_variant() {
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let formats: Vec<crate::models::ArchiveFormat> = ARCHIVE_FORMAT_ORDER.to_vec();
        assert_eq!(formats.len(), 6, "one cell per ArchiveFormat variant");
        let distinct: std::collections::HashSet<_> = ARCHIVE_FORMAT_ORDER.iter().collect();
        assert_eq!(distinct.len(), formats.len(), "no variant is listed twice");
        assert!(formats.contains(&crate::models::ArchiveFormat::TarXz));

        for format in formats {
            let cell = archive_format_cell(format, loc);
            assert!(!cell.label.is_empty(), "{format:?} has no label");
            assert!(!archive_format_ext(format).is_empty(), "{format:?} has no extension");
        }
    }
}
