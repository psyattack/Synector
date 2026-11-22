//! Preferences and System Settings:
//! Modern Master-Detail layout with clean sidebar categories, toggle switches,
//! theme palette cards, live tray icon preview, and direct configuration folder access.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use crate::gui::components::widgets::{self, Kind};
use crate::gui::state::{GuiState, SettingsTab};
use crate::gui::theme::{self, Tokens};

pub fn render_settings_view(ui: &mut Ui, state: &mut GuiState) {
    let t = Tokens::get(ui.ctx());
    let loc = state.loc();

    let (mut cfg, current_autorun) = match &state.daemon_state {
        Some(ds) => (ds.config.settings(), ds.autorun_enabled),
        None => {
            ui.add_space(32.0);
            widgets::hud(ui, loc.settings_daemon_offline, t.bad);
            return;
        }
    };

    // A pending write keeps its draft in egui's temporary data: a failure re-renders the edit so
    // an unreachable daemon cannot silently revert it, and a confirmation drops the draft.
    if let Some(draft) = ui.data(|d| d.get_temp::<PendingSettingsDraft>(pending_settings_id())) {
        if state.settings_write_confirmed(draft.revision) {
            ui.data_mut(|d| d.remove_temp::<PendingSettingsDraft>(pending_settings_id()));
        } else {
            cfg = draft.settings;
        }
    }

    let mut changes = SettingsChanges::default();

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll_area")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let available_w = ui.available_width();
            let is_wide = available_w >= 620.0;

            if is_wide {
                let start_top = ui.cursor().top();
                let initial_avail_h = ui.available_height();

                let mut sidebar_rect = Rect::NOTHING;
                let mut content_rect = Rect::NOTHING;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;

                    // Left Column: Navigation Sidebar
                    let sidebar_w = 190.0;
                    let resp_side = ui.allocate_ui_with_layout(
                        vec2(sidebar_w, ui.available_height()),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            render_sidebar(ui, state, loc, &t, sidebar_w);
                        },
                    );
                    sidebar_rect = resp_side.response.rect;

                    // Space reserved for the vertical divider
                    ui.add_space(1.0);

                    // Right Column: Active Content Panel
                    let content_w = ui.available_width();
                    let resp_content = ui.allocate_ui_with_layout(
                        vec2(content_w, ui.available_height()),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 12.0;
                            render_content_panel(ui, state, &mut cfg, &mut changes, current_autorun, &t, loc);
                        },
                    );
                    content_rect = resp_content.response.rect;
                });

                // Vertical divider from header edge to footer edge (exact 1px filled rect, DPI-independent)
                let div_x = sidebar_rect.right() + 12.0;
                let top_y = theme::HEADER_H;
                let bottom_y = (ui.ctx().content_rect().bottom() - theme::FOOTER_H)
                    .max(content_rect.bottom())
                    .max(start_top + initial_avail_h);

                if bottom_y > top_y {
                    let v_divider = Rect::from_min_max(
                        pos2(div_x, top_y),
                        pos2(div_x + 1.0, bottom_y),
                    );
                    ui.painter()
                        .with_clip_rect(ui.ctx().content_rect())
                        .rect_filled(v_divider, CornerRadius::ZERO, t.hairline().color);
                }
            } else {
                // Narrow / Compact Responsive Fallback: Top segmented tabs + panel
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    let tab_options = [
                        (SettingsTab::General, widgets::Cell::new(loc.settings_tab_general, "")),
                        (SettingsTab::Appearance, widgets::Cell::new(loc.settings_tab_appearance, "")),
                        (SettingsTab::Updates, widgets::Cell::new(loc.settings_tab_updates, "")),
                    ];
                    widgets::cells(ui, &mut state.settings_tab, &tab_options);
                    widgets::hrule(ui);
                    render_content_panel(ui, state, &mut cfg, &mut changes, current_autorun, &t, loc);
                });
            }
        });

    if changes.config {
        // The write runs on the IPC worker; the draft survives until the daemon confirms the
        // exact revision, so a failed or slow write never loses the edit or claims success.
        let revision = state.request_save_settings(cfg.clone(), changes.mask);
        ui.data_mut(|d| {
            d.insert_temp(
                pending_settings_id(),
                PendingSettingsDraft {
                    revision,
                    settings: cfg,
                },
            );
        });
    }
}

/// Key of the unsaved settings draft in egui's temporary data.
fn pending_settings_id() -> egui::Id {
    egui::Id::unique("settings_unsaved_draft")
}

/// Which settings edits a frame produced: `config` asks for a write at the end of the frame,
/// `mask` additionally asks for the restart notice (masking is applied only when the daemon
/// starts).
#[derive(Default)]
struct SettingsChanges {
    config: bool,
    mask: bool,
}

/// One settings write awaiting its result. The revision lets a confirmation drop exactly the
/// draft it was written from, even if the user edited again while the write was in flight.
#[derive(Clone, Default)]
struct PendingSettingsDraft {
    revision: u64,
    settings: crate::config::Settings,
}

fn render_sidebar(ui: &mut Ui, state: &mut GuiState, loc: &crate::i18n::Strings, t: &Tokens, w: f32) {
    let tabs = [
        (SettingsTab::General, "01", loc.settings_tab_general),
        (SettingsTab::Appearance, "02", loc.settings_tab_appearance),
        (SettingsTab::Updates, "03", loc.settings_tab_updates),
    ];

    for (tab, idx, label) in tabs {
        let is_active = state.settings_tab == tab;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
        if resp.clicked() {
            state.settings_tab = tab;
        }
        let enabled = ui.is_enabled();
        let hovered = resp.hovered() && enabled;

        let bg_fill = if is_active {
            t.sunken
        } else if hovered {
            t.sunken.gamma_multiply(0.6)
        } else {
            Color32::TRANSPARENT
        };

        ui.painter().rect_filled(rect, CornerRadius::ZERO, bg_fill);

        if is_active {
            let bar_rect = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
            ui.painter().rect_filled(bar_rect, CornerRadius::ZERO, t.accent);
        }

        let text_y = rect.center().y;
        let idx_color = if is_active { t.accent_ink } else { t.faint };
        let idx_galley = widgets::caps(ui, idx, 10.0, theme::mono(), 1.0, idx_color);
        let sep_galley = widgets::caps(ui, " / ", 10.0, theme::mono(), 1.0, t.faint);
        let label_color = if is_active {
            t.ink_strong
        } else if hovered {
            t.ink
        } else {
            t.muted
        };
        let label_galley = widgets::caps(ui, label, 11.0, theme::mono_bold(), theme::BUTTON_SPACING, label_color);

        let mut x = rect.left() + 14.0;
        widgets::paint_left(ui, Rect::from_min_size(pos2(x, text_y - idx_galley.size().y / 2.0), idx_galley.size()), idx_galley.clone(), idx_color);
        x += idx_galley.size().x;
        widgets::paint_left(ui, Rect::from_min_size(pos2(x, text_y - sep_galley.size().y / 2.0), sep_galley.size()), sep_galley.clone(), t.faint);
        x += sep_galley.size().x;
        widgets::paint_left(ui, Rect::from_min_size(pos2(x, text_y - label_galley.size().y / 2.0), label_galley.size()), label_galley, label_color);

        if enabled {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
}

fn render_content_panel(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changes: &mut SettingsChanges,
    current_autorun: bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;

            match state.settings_tab {
                SettingsTab::General => {
                    render_general_content(ui, state, cfg, changes, current_autorun, t, loc);
                }
                SettingsTab::Appearance => {
                    render_appearance_content(ui, state, cfg, &mut changes.config, t, loc);
                }
                SettingsTab::Updates => {
                    render_updates_content(ui, state, cfg, &mut changes.config, t, loc);
                }
            }
        });
}

fn render_setting_row<F: FnOnce(&mut Ui)>(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    t: &Tokens,
    add_control: F,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let available_w = ui.available_width();
        let control_w_estimate = 260.0;
        let text_w = (available_w - control_w_estimate).max(140.0);

        ui.allocate_ui_with_layout(
            vec2(text_w, 36.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(
                    egui::RichText::new(title)
                        .font(egui::FontId::new(13.0, theme::medium()))
                        .color(t.ink_strong),
                ));
                ui.add(egui::Label::new(
                    egui::RichText::new(subtitle)
                        .font(egui::FontId::new(11.0, egui::FontFamily::Proportional))
                        .color(t.muted),
                ));
            },
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add_control(ui);
        });
    });
}

/// Editable text and validation state of the tray-icon path field, kept in egui's temporary
/// data so typing is not persisted until the field loses focus.
#[derive(Clone, Default)]
struct TrayIconPathEdit {
    text: String,
    invalid: bool,
}

/// Why a typed tray-icon path cannot be stored.
#[derive(Debug, PartialEq, Eq)]
enum TrayIconPathError {
    /// The file does not exist (the daemon would silently fall back to the default icon).
    Missing,
    /// The extension is not one the daemon's icon loader understands.
    Unsupported,
}

/// Validates a typed tray-icon path with the same rules the daemon's loader applies: the file
/// must exist and be an `.ico`, `.exe` or `.dll`. Returns the normalized value to persist,
/// `None` for an empty field, or the reason it was rejected.
fn validate_tray_icon_path(raw: &str) -> Result<Option<String>, TrayIconPathError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = std::path::Path::new(trimmed);
    // The daemon expands `%VAR%` segments before loading, so such a path cannot be checked
    // for existence here; only its extension is validated.
    if !trimmed.contains('%') && !path.is_file() {
        return Err(TrayIconPathError::Missing);
    }
    let supported = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "ico" | "exe" | "dll"));
    if !supported {
        return Err(TrayIconPathError::Unsupported);
    }
    Ok(Some(trimmed.to_string()))
}

fn render_general_content(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changes: &mut SettingsChanges,
    current_autorun: bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    // 1. Windows Startup Toggle
    render_setting_row(ui, loc.settings_autorun_title, loc.settings_autorun_desc, t, |ui| {
        let mut autorun_val = current_autorun;
        if widgets::toggle_switch(ui, &mut autorun_val).changed() {
            state.toggle_autorun(autorun_val);
        }
    });

    widgets::hrule(ui);

    // 2. Custom Tray Icon with live thumbnail preview
    render_setting_row(ui, loc.settings_tray_icon_title, loc.settings_tray_icon_desc, t, |ui| {
        let edit_id = egui::Id::unique("settings_tray_icon_edit");
        let input_id = egui::Id::unique("settings_tray_icon_input");
        let mut edit = ui.data(|d| d.get_temp::<TrayIconPathEdit>(edit_id)).unwrap_or_default();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // Live preview button matching the exact size of the input line (theme::CONTROL_H x theme::CONTROL_H)
            let preview_size = vec2(theme::CONTROL_H, theme::CONTROL_H);
            let (p_rect, p_resp) = ui.allocate_exact_size(preview_size, Sense::click());
            let p_hovered = p_resp.hovered();
            let p_stroke = if p_hovered { Stroke::new(1.0, t.line_strong) } else { t.hairline() };
            ui.painter().rect(p_rect, CornerRadius::ZERO, t.bg, p_stroke, egui::StrokeKind::Inside);

            let mut icon_rendered = false;
            if let Some(ref path) = cfg.tray_icon_path {
                if let Some(tex) = state.icon_cache.get_icon_from_path(ui.ctx(), path) {
                    let img_rect = Rect::from_center_size(p_rect.center(), vec2(22.0, 22.0));
                    ui.painter().image(
                        tex.id(),
                        img_rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    icon_rendered = true;
                }
            }

            // By default, display the official application logo icon (not a simple glyph)
            if !icon_rendered {
                if let Some(app_tex) = state.icon_cache.get_app_icon(ui.ctx()) {
                    let img_rect = Rect::from_center_size(p_rect.center(), vec2(22.0, 22.0));
                    ui.painter().image(
                        app_tex.id(),
                        img_rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    icon_rendered = true;
                }
            }

            if !icon_rendered {
                let icon_glyph = egui_phosphor::regular::APP_WINDOW;
                let galley = egui::WidgetText::from(
                    egui::RichText::new(icon_glyph)
                        .font(egui::FontId::new(16.0, theme::phosphor()))
                        .color(t.muted),
                ).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::FontSelection::Default);
                widgets::paint_centred(ui, p_rect, galley, t.muted);
            }

            // Clicking the preview box opens the file picker
            if p_resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter(loc.settings_tray_icon_filter, &["ico", "exe", "dll"])
                    .pick_file()
                {
                    let picked = file.to_string_lossy().to_string();
                    cfg.tray_icon_path = Some(picked.clone());
                    edit.text = picked;
                    edit.invalid = false;
                    changes.config = true;
                }
            }

            let has_reset = cfg.tray_icon_path.is_some();
            let reset_w = if has_reset {
                widgets::button_width(ui, loc.settings_btn_reset, 0.0) + 8.0
            } else {
                0.0
            };
            let input_w = (ui.available_width() - reset_w - 8.0).clamp(60.0, 240.0);

            // The typed path is committed on focus loss / Enter instead of on every keystroke:
            // each keystroke used to persist settings.json and reload the tray icon. The draft
            // lives in egui's temporary data so the field does not jump back while being typed.
            let focused = ui.memory(|m| m.has_focus(input_id));
            let stored = cfg.tray_icon_path.clone().unwrap_or_default();
            let mut path = if focused || edit.invalid { edit.text.clone() } else { stored.clone() };
            let response = ui.add(
                egui::TextEdit::singleline(&mut path)
                    .id(input_id)
                    .hint_text(loc.settings_tray_icon_placeholder)
                    .desired_width(input_w)
                    .min_size(vec2(input_w, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(8.0, 6.0)),
            );
            if response.changed() {
                edit.text = path.clone();
                edit.invalid = false;
            }
            if response.lost_focus() {
                match validate_tray_icon_path(&path) {
                    Ok(next) if next != cfg.tray_icon_path => {
                        cfg.tray_icon_path = next;
                        edit.invalid = false;
                        changes.config = true;
                    }
                    Ok(_) => edit.invalid = false,
                    Err(_) => {
                        // Keep the rejected text in the field (outlined in red) so the user can
                        // correct it, and report why it was not stored.
                        edit.text = path.clone();
                        edit.invalid = true;
                        crate::logging::warn("GUI", "Rejected an invalid tray icon path");
                        state.set_toast(loc.settings_tray_icon_invalid);
                    }
                }
            }
            if !focused && !edit.invalid {
                edit.text = stored;
            }
            if edit.invalid {
                ui.painter().rect_stroke(
                    response.rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, t.bad),
                    egui::StrokeKind::Inside,
                );
            }

            if has_reset && widgets::button(ui, loc.settings_btn_reset, Kind::Secondary, 0.0).clicked() {
                cfg.tray_icon_path = None;
                edit.text.clear();
                edit.invalid = false;
                changes.config = true;
            }
        });

        ui.data_mut(|d| d.insert_temp(edit_id, edit));
    });

    widgets::hrule(ui);

    // 3. Hide Tray Icon Toggle
    render_setting_row(ui, loc.settings_hide_tray_icon_title, loc.settings_hide_tray_icon_desc, t, |ui| {
        if widgets::toggle_switch(ui, &mut cfg.hide_tray_icon).changed() {
            changes.config = true;
        }
    });

    widgets::hrule(ui);

    // 4. Mask Daemon as System Service Toggle
    //
    // The daemon applies masking deployment only at start-up, and `ShutdownDaemon` also posts
    // WM_CLOSE to this GUI window (dropping unsaved drafts). The view therefore persists the
    // preference through the normal commit path and tells the user a restart is required; it
    // never stops the daemon from the frame thread.
    render_setting_row(ui, loc.settings_mask_daemon_title, loc.settings_mask_daemon_desc, t, |ui| {
        if widgets::toggle_switch(ui, &mut cfg.mask_daemon_process).changed() {
            changes.config = true;
            changes.mask = true;
        }
    });

    widgets::hrule(ui);

    // 5. Open Configuration and Storage Folder
    render_setting_row(ui, loc.settings_config_folder_title, loc.settings_config_folder_desc, t, |ui| {
        if widgets::button(ui, loc.settings_btn_open_folder, Kind::Secondary, 0.0).clicked() {
            let dir = crate::config::AppConfig::config_dir();
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        }
    });
}

fn render_appearance_content(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    // 1. Interface Language
    render_setting_row(ui, loc.settings_language_title, loc.settings_language_desc, t, |ui| {
        let mut lang_idx = match state.language {
            crate::i18n::Language::English => 0usize,
            crate::i18n::Language::Russian => 1usize,
        };
        // A language is always named in its own language (endonym), so the option labels stay
        // from `Language::label()`; only the descriptions follow the UI language.
        let lang_options = [
            (
                0usize,
                widgets::Cell::new(
                    crate::i18n::Language::English.label(),
                    loc.settings_language_en_desc,
                ),
            ),
            (
                1usize,
                widgets::Cell::new(
                    crate::i18n::Language::Russian.label(),
                    loc.settings_language_ru_desc,
                ),
            ),
        ];

        if widgets::cells(ui, &mut lang_idx, &lang_options) {
            let new_lang = if lang_idx == 1 {
                crate::i18n::Language::Russian
            } else {
                crate::i18n::Language::English
            };
            state.set_language(new_lang);
            cfg.language = new_lang;
            *changed = true;
        }
    });

    widgets::hrule(ui);

    // 2. Color Theme with visual palette cards
    render_setting_row(ui, loc.settings_theme_title, loc.settings_theme_desc, t, |ui| {
        // Preview swatches come from the live palette, so a Tokens change cannot desynchronize
        // the cards from the theme the user actually gets.
        let themes = [
            (crate::config::AppTheme::Dark, loc.settings_theme_dark),
            (crate::config::AppTheme::Light, loc.settings_theme_light),
            (crate::config::AppTheme::Chocolate, loc.settings_theme_chocolate),
        ];

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            for (thm, label) in themes {
                let palette = Tokens::of(thm);
                let is_selected = state.theme == thm;
                let card_size = vec2(90.0, theme::CONTROL_H);
                let (rect, resp) = ui.allocate_exact_size(card_size, Sense::click());
                if resp.clicked() && !is_selected {
                    state.set_theme(thm);
                    crate::gui::theme::apply_theme(ui.ctx(), thm);
                    cfg.theme = thm;
                    *changed = true;
                }

                let hovered = resp.hovered();
                let border_stroke = if is_selected {
                    Stroke::new(2.0, t.accent)
                } else if hovered {
                    Stroke::new(1.0, t.line_strong)
                } else {
                    t.hairline()
                };

                ui.painter().rect(rect, CornerRadius::ZERO, palette.bg, border_stroke, egui::StrokeKind::Inside);

                // Accent dot + theme name
                let dot_center = pos2(rect.left() + 14.0, rect.center().y);
                ui.painter().circle_filled(dot_center, 4.0, palette.accent);

                let text_pos = pos2(rect.left() + 24.0, rect.center().y);
                let galley = widgets::caps(ui, label, 10.0, theme::mono_bold(), 1.0, palette.ink);
                widgets::paint_left(ui, Rect::from_min_size(pos2(text_pos.x, text_pos.y - galley.size().y / 2.0), galley.size()), galley, palette.ink);

                resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
        });
    });
}

/// Installed-version subtitle of the updates row. The catalog supplies the product name and
/// `{}`; the build version comes from `CARGO_PKG_VERSION`.
fn updates_version_subtitle(loc: &crate::i18n::Strings) -> String {
    loc.settings_updates_version
        .replace("{}", env!("CARGO_PKG_VERSION"))
}

fn render_updates_content(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    t: &Tokens,
    loc: &crate::i18n::Strings,
) {
    // 1. Check on Startup Toggle
    render_setting_row(ui, loc.settings_updates_startup_title, loc.settings_updates_startup_desc, t, |ui| {
        if widgets::toggle_switch(ui, &mut cfg.check_updates_on_startup).changed() {
            *changed = true;
        }
    });

    widgets::hrule(ui);

    // 2. Automatic Updates Toggle
    render_setting_row(ui, loc.settings_updates_auto_install_title, loc.settings_updates_auto_install_desc, t, |ui| {
        if widgets::toggle_switch(ui, &mut cfg.auto_install_updates).changed() {
            *changed = true;
        }
    });

    widgets::hrule(ui);

    // 3. Installed Version & Manual Check
    let version_subtitle = updates_version_subtitle(loc);
    render_setting_row(ui, loc.settings_updates_check_title, &version_subtitle, t, |ui| {
        let btn_label = if state.is_checking_update {
            loc.settings_btn_checking_updates
        } else {
            loc.settings_btn_check_updates
        };

        let can_click = !state.is_checking_update && !state.is_downloading_update;
        ui.add_enabled_ui(can_click, |ui| {
            if widgets::button(ui, btn_label, Kind::Secondary, 0.0).clicked() {
                state.check_for_updates(true);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_tabs_rendering() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = GuiState::new();
        state.daemon_state = Some(crate::ipc::DaemonState {
            autorun_enabled: true,
            uptime_secs: 100,
            daemon_memory_bytes: 1000,
            active_rules_count: 0,
            total_rules_count: 0,
            config: crate::config::AppConfig::default(),
            logs: vec![],
            active_rule_executions: std::collections::HashMap::new(),
            active_overlay_ids: std::collections::HashSet::new(),
            active_test_rule_ids: std::collections::HashSet::new(),
            analytics: Default::default(),
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });

        // Test rendering across all 3 tabs
        let all_tabs = [
            SettingsTab::General,
            SettingsTab::Appearance,
            SettingsTab::Updates,
        ];

        for tab in all_tabs {
            state.settings_tab = tab;
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(900.0);
                    render_settings_view(ui, &mut state);
                });
            });
            out.textures_delta.clear();
        }

        // Test responsive compact layout (< 620px width)
        let mut out_compact = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(500.0);
                render_settings_view(ui, &mut state);
            });
        });
        out_compact.textures_delta.clear();

        assert_eq!(state.settings_tab, SettingsTab::Updates);

        // Verify General tab renders full card height without being clipped
        state.settings_tab = SettingsTab::General;
        let mut general_height = 0.0;
        let mut out = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(900.0);
                let initial_top = ui.cursor().top();
                render_settings_view(ui, &mut state);
                general_height = ui.cursor().top() - initial_top;
            });
        });
        out.textures_delta.clear();
        assert!(general_height > 250.0, "Settings content card should not be clipped; was {general_height}");
    }


    /// U03-004: the updates-row subtitle is the catalog template with the real product name
    /// ("Synector", not "Process Handler"), and the language-picker descriptions come from
    /// `Strings` so they follow the UI language.
    #[test]
    fn test_updates_version_subtitle_uses_catalog_product_name() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);
        let version = env!("CARGO_PKG_VERSION");

        for loc in [en, ru] {
            let subtitle = updates_version_subtitle(loc);
            assert!(subtitle.starts_with("Synector v"), "was {subtitle:?}");
            assert!(subtitle.ends_with(version), "was {subtitle:?}");
            assert!(!subtitle.contains("Process Handler"), "was {subtitle:?}");
            assert!(!subtitle.contains("{}"), "the placeholder must be replaced");
        }

        assert_eq!(en.settings_language_en_desc, "English interface");
        assert_eq!(ru.settings_language_en_desc, "Английский интерфейс");
        assert_eq!(ru.settings_language_ru_desc, "Русский интерфейс");
        assert_ne!(en.settings_language_en_desc, ru.settings_language_en_desc);
        assert_ne!(en.settings_language_ru_desc, ru.settings_language_ru_desc);
    }


    /// U03-007: typing is not persisted; only an existing icon file with a supported extension
    /// is accepted on commit.
    #[test]
    fn test_validate_tray_icon_path_accepts_only_existing_icon_files() {
        let dir = std::env::temp_dir().join(format!("synector-tray-icon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let icon = dir.join("icon.ico");
        std::fs::write(&icon, b"stub").expect("write icon");
        let upper = dir.join("ICON.ICO");
        std::fs::write(&upper, b"stub").expect("write upper-case icon");
        let text = dir.join("icon.txt");
        std::fs::write(&text, b"stub").expect("write text file");

        assert_eq!(validate_tray_icon_path(""), Ok(None));
        assert_eq!(validate_tray_icon_path("   "), Ok(None));
        assert_eq!(
            validate_tray_icon_path(&format!("  {}  ", icon.display())),
            Ok(Some(icon.display().to_string()))
        );
        assert!(
            matches!(validate_tray_icon_path(&upper.display().to_string()), Ok(Some(_))),
            "extensions are case-insensitive like the icon loader"
        );
        assert_eq!(
            validate_tray_icon_path(&text.display().to_string()),
            Err(TrayIconPathError::Unsupported)
        );
        assert_eq!(
            validate_tray_icon_path("%USERPROFILE%\\icon.ico"),
            Ok(Some("%USERPROFILE%\\icon.ico".to_string())),
            "the daemon expands %VAR% paths, so they are stored without an existence check"
        );
        assert_eq!(
            validate_tray_icon_path(&dir.join("missing.ico").display().to_string()),
            Err(TrayIconPathError::Missing)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// I18N-GAP: the tray-icon file dialog filter is a catalog value, not a view literal.
    #[test]
    fn test_tray_icon_dialog_filter_comes_from_the_catalog() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);
        assert_eq!(en.settings_tray_icon_filter, concat!("Icons & ", "Executables"));
        assert_ne!(en.settings_tray_icon_filter, ru.settings_tray_icon_filter);
        let source = include_str!("settings_view.rs");
        assert!(
            !source.contains(concat!("\"Icons & ", "Executables\"")),
            "the native dialog filter literal is back in the view"
        );
    }

    /// U03-NEW-01: the mask row and its commit path must never request a daemon shutdown. That
    /// request also posts WM_CLOSE to this GUI window and would drop unsaved drafts; masking is
    /// applied on the next daemon start instead. The needle is built with `concat!` so the scan
    /// cannot match this test, and the scan stops at `#[cfg(test)]` and skips comment lines
    /// (the row's explanatory comment names the request).
    #[test]
    fn test_mask_setting_never_requests_a_daemon_shutdown() {
        let source = include_str!("settings_view.rs");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .unwrap_or(source);
        let production = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let shutdown_request = concat!("Shutdown", "Daemon");
        assert!(
            !production.contains(shutdown_request),
            "the settings view must not stop the daemon: masking applies on the next start and \
             {shutdown_request} also closes the GUI, dropping unsaved drafts"
        );
    }
}
