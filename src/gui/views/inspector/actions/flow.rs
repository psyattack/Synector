//! `flow` action field editors.

use super::*;

pub(crate) fn render_wait_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let state = ctx.state;
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let default_picker_target = ctx.default_picker_target;
    let Action::Wait { mode, seconds, condition, timeout_sec, timeout_policy } = action else { return; };
    widgets::hud(ui, loc.inspector_wait_mode, t.muted);
    let mode_options = [
        (crate::models::WaitMode::Timer, Cell::new(loc.inspector_wait_timer, "")),
        (crate::models::WaitMode::UntilCondition, Cell::new(loc.inspector_wait_until_condition, "")),
    ];
    widgets::cells(ui, mode, &mode_options);

    match mode {
        crate::models::WaitMode::Timer => {
            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_wait_seconds, t.muted);
            widgets::duration_secs_f64(ui, loc, ("wait_seconds", idx), seconds, 0.05..=86400.0);
        }
        crate::models::WaitMode::UntilCondition => {
            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_wait_condition_type, t.muted);

            #[derive(Clone, Copy, PartialEq, Eq)]
            enum WaitCondKind {
                WindowOpened,
                WindowClosed,
                ProcessStarted,
                ProcessExited,
                Hotkey,
                File,
                RuleStarted,
                RuleStopped,
            }

            let mut current_kind = match condition {
                Some(crate::models::WaitCondition::WindowOpened { .. }) => WaitCondKind::WindowOpened,
                Some(crate::models::WaitCondition::WindowClosed { .. }) => WaitCondKind::WindowClosed,
                Some(crate::models::WaitCondition::ProcessStarted { .. }) => WaitCondKind::ProcessStarted,
                Some(crate::models::WaitCondition::ProcessExited { .. }) => WaitCondKind::ProcessExited,
                Some(crate::models::WaitCondition::Hotkey { .. }) => WaitCondKind::Hotkey,
                Some(crate::models::WaitCondition::File { .. }) => WaitCondKind::File,
                Some(crate::models::WaitCondition::RuleStarted { .. }) => WaitCondKind::RuleStarted,
                Some(crate::models::WaitCondition::RuleStopped { .. }) => WaitCondKind::RuleStopped,
                None => WaitCondKind::WindowOpened,
            };

            let combo_id = ui.scope_id().with(idx).with("wait_cond_combo");
            let kind_label = match current_kind {
                WaitCondKind::WindowOpened => loc.inspector_wait_window_opened,
                WaitCondKind::WindowClosed => loc.inspector_wait_window_closed,
                WaitCondKind::ProcessStarted => loc.inspector_wait_process_started,
                WaitCondKind::ProcessExited => loc.inspector_wait_process_exited,
                WaitCondKind::Hotkey => loc.inspector_wait_hotkey,
                WaitCondKind::File => loc.inspector_wait_file,
                WaitCondKind::RuleStarted => loc.inspector_wait_rule_started,
                WaitCondKind::RuleStopped => loc.inspector_wait_rule_stopped,
            };

            let mut changed_kind = false;
            egui::ComboBox::from_id_salt(combo_id)
                .selected_text(kind_label)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut current_kind, WaitCondKind::WindowOpened, loc.inspector_wait_window_opened).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::WindowClosed, loc.inspector_wait_window_closed).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::ProcessStarted, loc.inspector_wait_process_started).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::ProcessExited, loc.inspector_wait_process_exited).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::Hotkey, loc.inspector_wait_hotkey).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::File, loc.inspector_wait_file).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::RuleStarted, loc.inspector_wait_rule_started).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::RuleStopped, loc.inspector_wait_rule_stopped).clicked() { changed_kind = true; }
                });

            // A missing condition is left untouched until the user picks a kind; an
            // until_condition wait without one is rejected by the shared validator on save.
            if changed_kind {
                *condition = Some(match current_kind {
                    WaitCondKind::WindowOpened => crate::models::WaitCondition::WindowOpened { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::WindowClosed => crate::models::WaitCondition::WindowClosed { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::ProcessStarted => crate::models::WaitCondition::ProcessStarted { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::ProcessExited => crate::models::WaitCondition::ProcessExited { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::Hotkey => crate::models::WaitCondition::Hotkey { key_combination: "Ctrl+Shift+W".into() },
                    WaitCondKind::File => crate::models::WaitCondition::File { path: String::new(), condition: crate::models::WaitFileCondition::Exists },
                    WaitCondKind::RuleStarted => crate::models::WaitCondition::RuleStarted { rule_id: String::new() },
                    WaitCondKind::RuleStopped => crate::models::WaitCondition::RuleStopped { rule_id: String::new() },
                });
            }

            ui.add_space(4.0);
            if let Some(cond) = condition {
                match cond {
                    crate::models::WaitCondition::WindowOpened { process_name, match_mode }
                    | crate::models::WaitCondition::WindowClosed { process_name, match_mode }
                    | crate::models::WaitCondition::ProcessStarted { process_name, match_mode }
                    | crate::models::WaitCondition::ProcessExited { process_name, match_mode } => {
                        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                        if render_process_name_row(ui, loc, process_name, loc.inspector_hint_wait_window_process) {
                            resp.pick_target = Some(default_picker_target);
                        }

                        ui.add_space(4.0);
                        widgets::hud(ui, loc.inspector_field_match_mode, t.muted);
                        let match_options = [
                            (crate::models::ProcessMatchMode::Exact, Cell::new(loc.inspector_match_mode_exact, "")),
                            (crate::models::ProcessMatchMode::Wildcard, Cell::new(loc.inspector_match_mode_wildcard, "")),
                            (crate::models::ProcessMatchMode::Regex, Cell::new(loc.inspector_match_mode_regex, "")),
                        ];
                        widgets::cells(ui, match_mode, &match_options);
                    }
                    crate::models::WaitCondition::Hotkey { key_combination } => {
                        widgets::hud(ui, loc.inspector_trigger_hotkey, t.muted);
                        ui.add(
                            egui::TextEdit::singleline(key_combination)
                                .hint_text(loc.inspector_hint_wait_hotkey)
                                .desired_width(ui.available_width())
                                .min_size(vec2(0.0, theme::control_height(ui)))
                                .align(egui::Align2::LEFT_CENTER)
                                .margin(vec2(10.0, 8.0)),
                        );
                    }
                    crate::models::WaitCondition::File { path, condition: file_cond } => {
                        widgets::hud(ui, loc.inspector_field_watch_path, t.muted);
                        ui.horizontal(|ui| {
                            ui.set_width(ui.available_width());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let btn_w = widgets::button_width(ui, loc.inspector_btn_choose_file, 0.0);
                                if widgets::button(ui, loc.inspector_btn_choose_file, Kind::Secondary, btn_w).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                                        *path = f.to_string_lossy().to_string();
                                    }
                                }
                                ui.add(
                                    egui::TextEdit::singleline(path)
                                        .hint_text(loc.inspector_hint_wait_file_path)
                                        .desired_width(ui.available_width())
                                        .min_size(vec2(0.0, theme::control_height(ui)))
                                        .align(egui::Align2::LEFT_CENTER)
                                        .margin(vec2(10.0, 8.0)),
                                );
                            });
                        });

                        ui.add_space(4.0);
                        let file_opts = [
                            (crate::models::WaitFileCondition::Exists, Cell::new(loc.inspector_wait_file_exists, "")),
                            (crate::models::WaitFileCondition::Deleted, Cell::new(loc.inspector_wait_file_deleted, "")),
                        ];
                        widgets::cells(ui, file_cond, &file_opts);
                    }
                    crate::models::WaitCondition::RuleStarted { rule_id }
                    | crate::models::WaitCondition::RuleStopped { rule_id } => {
                        // A wait cannot observe its own pipeline start or stop:
                        // "started" is already true and "stopped" cannot happen
                        // before the pipeline itself ends.
                        let editing_id = state
                            .editing_rule
                            .as_ref()
                            .map(|rule| rule.id.as_str())
                            .unwrap_or_default();
                        let choices: Vec<RuleChoice> = rule_choices(state)
                            .into_iter()
                            .filter(|choice| choice.id != editing_id)
                            .collect();
                        render_rule_choice_combo(ui, &choices, rule_id, ("wait_rule_target", idx), loc, t);
                    }
                }
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                widgets::hud(ui, loc.inspector_wait_timeout, t.muted);
                widgets::duration_secs_f64(ui, loc, ("wait_timeout", idx), timeout_sec, 0.0..=3600.0);
                ui.label(egui::RichText::new(loc.inspector_wait_timeout_zero_hint).size(11.0).color(t.muted));
            });

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_wait_timeout_policy, t.muted);
            let policy_options = [
                (crate::models::WaitTimeoutPolicy::Abort, Cell::new(loc.inspector_wait_policy_abort, "")),
                (crate::models::WaitTimeoutPolicy::Continue, Cell::new(loc.inspector_wait_policy_continue, "")),
            ];
            widgets::cells(ui, timeout_policy, &policy_options);
        }
    }
}

pub(crate) fn render_check_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let state = ctx.state;
    let t = ctx.t;
    let idx = ctx.idx;
    let default_picker_target = ctx.default_picker_target;
    let Action::Check { condition } = action else { return; };
    widgets::hud(ui, loc.catalog_title_check, t.muted);
    ui.add(
        egui::Label::new(
            egui::RichText::new(loc.inspector_check_desc)
                .size(11.5)
                .color(t.muted),
        ).wrap(),
    );
    ui.add_space(4.0);

    let mut kind = ConditionKind::from_condition(condition);
    let combo_id = ui.scope_id().with(idx).with("check_cond_kind");
    let mut changed_kind = false;
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(kind.label(loc))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for k in [
                ConditionKind::Process,
                ConditionKind::Window,
                ConditionKind::Time,
                ConditionKind::Event,
                ConditionKind::Resource,
                ConditionKind::FileWatcher,
                ConditionKind::Rule,
            ] {
                if ui.selectable_value(&mut kind, k, k.label(loc)).clicked() {
                    changed_kind = true;
                }
            }
        });

    if changed_kind {
        *condition = match kind {
            ConditionKind::Process => RuleCondition::default_process(),
            ConditionKind::Window => RuleCondition::WindowFocused { process_name: String::new(), match_mode: ProcessMatchMode::Exact, must_be_focused: true },
            ConditionKind::Time => RuleCondition::TimeRange { start_time: "09:00".into(), end_time: "18:00".into(), days_of_week: vec![1, 2, 3, 4, 5] },
            ConditionKind::Event => RuleCondition::Event { event_type: SystemEventType::SessionLock, idle_duration_sec: 300 },
            ConditionKind::Resource => RuleCondition::ResourceThreshold { process_name: String::new(), cpu_percent: Some(80.0), ram_mb: None },
            ConditionKind::FileWatcher => RuleCondition::FileWatcher { path: String::new(), filter_extension: None, must_exist: true },
            ConditionKind::Rule => RuleCondition::RuleRunning { rule_id: String::new(), must_be_running: true },
        };
    }

    ui.add_space(4.0);
    render_single_condition_editor(ui, loc, condition, idx, || {
        resp.pick_target = Some(default_picker_target);
    }, t, state);
}

pub(crate) fn render_if_else_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let state = ctx.state;
    let idx = ctx.idx;
    let loc = ctx.state.loc();
    let t = ctx.t;
    let depth = ctx.depth;
    let Action::IfElse { conditions, then_actions, else_if_branches, else_actions } = action else { return; };
    if depth > 0 {
        ui.label(egui::RichText::new(loc.inspector_ifelse_nested_not_supported).color(t.muted));
    } else {
        // An empty condition list is rendered as-is; the "add condition" button below is
        // the user action that fills it, so opening the rule does not mutate the draft.
        // ── 1. PRIMARY IF BLOCK ──
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_then_badge, t.good);
        });

        let mut remove_if_cond_idx = None;
        let mut change_if_cond_idx = None;
        let mut clone_if_cond_idx = None;
        let total_if_conds = conditions.len();
        for (c_idx, c_item) in conditions.iter_mut().enumerate() {
            if c_idx > 0 {
                render_condition_operator_connector(ui, &mut c_item.operator, loc, t);
            }
            let can_remove = total_if_conds > 1;
            let (remove_clicked, change_clicked, clone_clicked) = render_condition_card(
                ui,
                c_idx,
                c_item,
                loc,
                t,
                true,
                can_remove,
                || {
                    resp.pick_target = Some(crate::gui::state::ProcessPickerTarget::ActionCondition {
                        action_idx: idx,
                        cond_idx: c_idx,
                    });
                },
                state,
            );
            if remove_clicked && can_remove {
                remove_if_cond_idx = Some(c_idx);
            }
            if change_clicked {
                change_if_cond_idx = Some(c_idx);
            }
            if clone_clicked {
                clone_if_cond_idx = Some(c_idx);
            }
        }
        if let Some(c_idx) = clone_if_cond_idx {
            if c_idx < conditions.len() {
                let cloned = conditions[c_idx].clone();
                conditions.insert(c_idx + 1, cloned);
            }
        }
        if let Some(c_idx) = remove_if_cond_idx {
            if c_idx < conditions.len() {
                conditions.remove(c_idx);
            }
        }
        if let Some(c_idx) = change_if_cond_idx {
            resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ChangeActionIfElse {
                action_idx: idx,
                cond_idx: c_idx,
            });
        }

        if render_add_condition_button(ui, loc, t) {
            resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ActionIfElse {
                action_idx: idx,
            });
        }

        ui.add_space(6.0);

        // THEN branch actions
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_then_execute, t.good);
        });

        let then_branch_salt = ifelse_branch_salt(IfElseBranchSalt::Then);
        render_subactions_branch_pipeline(
            ui,
            state,
            then_actions,
            idx,
            &then_branch_salt,
            loc.inspector_ifelse_then_badge,
            BranchPickerKind::Then,
            loc.inspector_ifelse_add_then,
            t,
            depth + 1,
            resp,
            Some(crate::gui::state::IfElseCatalogTarget::Then(idx)),
        );

        // ── 2. ELSE IF BRANCHES ──
        let mut remove_branch_idx = None;
        for (b_idx, branch) in else_if_branches.iter_mut().enumerate() {
            ui.add_space(8.0);
            widgets::hrule(ui);

            ui.horizontal(|ui| {
                let b_lbl = format!("{} {:02}", loc.inspector_ifelse_elseif_badge, b_idx + 1);
                widgets::hud(ui, &b_lbl, t.warn);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::glyph_button(ui, Glyph::Close, loc.inspector_ifelse_remove_branch_tip).clicked() {
                        remove_branch_idx = Some(b_idx);
                    }
                });
            });

            let mut remove_elif_cond_idx = None;
            let mut change_elif_cond_idx = None;
            let mut clone_elif_cond_idx = None;
            let total_elif_conds = branch.conditions.len();
            for (c_idx, c_item) in branch.conditions.iter_mut().enumerate() {
                if c_idx > 0 {
                    render_condition_operator_connector(ui, &mut c_item.operator, loc, t);
                }
                let can_remove = total_elif_conds > 1;
                let (remove_clicked, change_clicked, clone_clicked) = render_condition_card(
                    ui,
                    c_idx,
                    c_item,
                    loc,
                    t,
                    true,
                    can_remove,
                    || {
                        resp.pick_target = Some(crate::gui::state::ProcessPickerTarget::ActionElseIfCondition {
                            action_idx: idx,
                            branch_idx: b_idx,
                            cond_idx: c_idx,
                        });
                    },
                    state,
                );
                if remove_clicked && can_remove {
                    remove_elif_cond_idx = Some(c_idx);
                }
                if change_clicked {
                    change_elif_cond_idx = Some(c_idx);
                }
                if clone_clicked {
                    clone_elif_cond_idx = Some(c_idx);
                }
            }
            if let Some(c_idx) = clone_elif_cond_idx {
                if c_idx < branch.conditions.len() {
                    let cloned = branch.conditions[c_idx].clone();
                    branch.conditions.insert(c_idx + 1, cloned);
                }
            }
            if let Some(c_idx) = remove_elif_cond_idx {
                if c_idx < branch.conditions.len() {
                    branch.conditions.remove(c_idx);
                }
            }
            if let Some(c_idx) = change_elif_cond_idx {
                resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ChangeActionElseIf {
                    action_idx: idx,
                    branch_idx: b_idx,
                    cond_idx: c_idx,
                });
            }

            if render_add_condition_button(ui, loc, t) {
                resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ActionElseIf {
                    action_idx: idx,
                    branch_idx: b_idx,
                });
            }

            ui.add_space(6.0);

            // Branch actions header
            let branch_salt = ifelse_branch_salt(IfElseBranchSalt::ElseIf(b_idx));
            let branch_lbl = loc.inspector_ifelse_elif_badge.replace("{:02}", &format!("{:02}", b_idx + 1));
            ui.horizontal(|ui| {
                widgets::hud(ui, loc.inspector_ifelse_then_execute, t.warn);
            });

            render_subactions_branch_pipeline(
                ui,
                state,
                &mut branch.actions,
                idx,
                &branch_salt,
                &branch_lbl,
                BranchPickerKind::ElseIf(b_idx),
                loc.inspector_ifelse_add_then,
                t,
                depth + 1,
                resp,
                Some(crate::gui::state::IfElseCatalogTarget::ElseIf {
                    parent_idx: idx,
                    branch_idx: b_idx,
                }),
            );
        }
        if let Some(b_idx) = remove_branch_idx {
            if b_idx < else_if_branches.len() {
                else_if_branches.remove(b_idx);
            }
        }

        // Full-width button to add an Else If branch with separator above and below
        ui.add_space(8.0);
        widgets::hrule(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_elseif_badge, t.warn);
        });
        ui.add_space(4.0);
        if render_full_width_action_button(ui, loc.inspector_ifelse_add_elseif, "", t) {
            else_if_branches.push(crate::models::ElseIfBranch::new());
        }
        ui.add_space(6.0);
        widgets::hrule(ui);
        ui.add_space(8.0);

        // ── 3. ELSE BRANCH ──
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_else_badge, t.bad);
        });

        let else_branch_salt = ifelse_branch_salt(IfElseBranchSalt::Else);
        render_subactions_branch_pipeline(
            ui,
            state,
            else_actions,
            idx,
            &else_branch_salt,
            loc.inspector_ifelse_else_badge,
            BranchPickerKind::Else,
            loc.inspector_ifelse_add_else,
            t,
            depth + 1,
            resp,
            Some(crate::gui::state::IfElseCatalogTarget::Else(idx)),
        );
    }
}
