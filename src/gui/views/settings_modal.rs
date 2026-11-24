//! Preferences and System Settings dialog:
//! One scrolling body of untitled setting blocks in a fixed order, rendered in the shared modal
//! base. A search field spanning the body sits above the blocks and filters the rows by the text
//! they show.

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::widgets::{self, Kind};
use crate::gui::state::GuiState;
use crate::gui::theme::{self, Metrics, Palette, Tokens};
use crate::i18n::Strings;

/// One row of the settings page. The search field matches the text [`SettingsRow::text`] returns
/// and the page renders that same text, so a row can never be searchable while hidden: a new
/// variant forces both matches below to be filled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsRow {
    Autorun,
    TrayIcon,
    HideTrayIcon,
    MaskDaemon,
    DiskLogging,
    ConfigFolder,
    Language,
    Theme,
    Skin,
    Accent,
    UpdatesOnStartup,
    AutoInstallUpdates,
    InstalledVersion,
}

impl SettingsRow {
    /// Every row exactly once, for the page-order and catalog-text guards in the tests.
    #[cfg(test)]
    const ALL: [Self; 13] = [
        Self::Autorun,
        Self::TrayIcon,
        Self::HideTrayIcon,
        Self::MaskDaemon,
        Self::DiskLogging,
        Self::ConfigFolder,
        Self::Language,
        Self::Theme,
        Self::Skin,
        Self::Accent,
        Self::UpdatesOnStartup,
        Self::AutoInstallUpdates,
        Self::InstalledVersion,
    ];

    /// Title and description the row shows, both from the catalog. The installed-version row
    /// formats its own description, so the running version is searchable too.
    fn text(self, loc: &Strings) -> (&'static str, String) {
        match self {
            Self::Autorun => (loc.settings_autorun_title, loc.settings_autorun_desc.to_string()),
            Self::TrayIcon => (
                loc.settings_tray_icon_title,
                loc.settings_tray_icon_desc.to_string(),
            ),
            Self::HideTrayIcon => (
                loc.settings_hide_tray_icon_title,
                loc.settings_hide_tray_icon_desc.to_string(),
            ),
            Self::MaskDaemon => (
                loc.settings_mask_daemon_title,
                loc.settings_mask_daemon_desc.to_string(),
            ),
            Self::DiskLogging => (
                loc.settings_logging_title,
                loc.settings_logging_desc.to_string(),
            ),
            Self::ConfigFolder => (
                loc.settings_config_folder_title,
                loc.settings_config_folder_desc.to_string(),
            ),
            Self::Language => (loc.settings_language_title, loc.settings_language_desc.to_string()),
            Self::Theme => (loc.settings_theme_title, loc.settings_theme_desc.to_string()),
            Self::Skin => (loc.settings_skin_title, loc.settings_skin_desc.to_string()),
            Self::Accent => (loc.settings_accent_title, loc.settings_accent_desc.to_string()),
            Self::UpdatesOnStartup => (
                loc.settings_updates_startup_title,
                loc.settings_updates_startup_desc.to_string(),
            ),
            Self::AutoInstallUpdates => (
                loc.settings_updates_auto_install_title,
                loc.settings_updates_auto_install_desc.to_string(),
            ),
            Self::InstalledVersion => (
                loc.settings_updates_check_title,
                updates_version_subtitle(loc),
            ),
        }
    }
}

/// The page's blocks, in order. A block is one card of rows; a search that empties every row of
/// a block removes the whole card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsSection {
    General,
    Appearance,
    Updates,
}

/// The page order the renderer walks. The search field walks the same table.
const SETTINGS_SECTIONS: [SettingsSection; 3] = [
    SettingsSection::General,
    SettingsSection::Appearance,
    SettingsSection::Updates,
];

impl SettingsSection {
    /// Name of the block. The cards themselves stay untitled; the name is the block's identity and
    /// a search alias, so a query that names the block keeps all of its rows.
    fn heading(self, loc: &Strings) -> &'static str {
        match self {
            Self::General => loc.settings_section_general,
            Self::Appearance => loc.settings_section_appearance,
            Self::Updates => loc.settings_section_updates,
        }
    }

    fn rows(self) -> &'static [SettingsRow] {
        match self {
            Self::General => &[
                SettingsRow::Autorun,
                SettingsRow::TrayIcon,
                SettingsRow::HideTrayIcon,
                SettingsRow::MaskDaemon,
                SettingsRow::DiskLogging,
                SettingsRow::ConfigFolder,
            ],
            Self::Appearance => &[
                SettingsRow::Language,
                SettingsRow::Theme,
                SettingsRow::Skin,
                SettingsRow::Accent,
            ],
            Self::Updates => &[
                SettingsRow::UpdatesOnStartup,
                SettingsRow::AutoInstallUpdates,
                SettingsRow::InstalledVersion,
            ],
        }
    }
}

/// Query of the settings search field, matched case-insensitively against the text a row shows.
struct SettingsFilter {
    needle: String,
}

impl SettingsFilter {
    fn new(query: &str) -> Self {
        Self {
            needle: query.trim().to_lowercase(),
        }
    }

    fn is_active(&self) -> bool {
        !self.needle.is_empty()
    }

    /// An empty query keeps everything; otherwise the text must contain the query.
    fn matches(&self, text: &str) -> bool {
        !self.is_active() || text.to_lowercase().contains(&self.needle)
    }

    fn matches_row(&self, row: SettingsRow, loc: &Strings) -> bool {
        let (title, description) = row.text(loc);
        self.matches(title) || self.matches(&description)
    }
}

/// The settings dialog, opened from the header gear glyph. The settings page keeps its own
/// search; the modal supplies the card, the header, the scrim and Escape/backdrop dismissal.
pub fn render_settings_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let loc = state.loc();

    let Some(response) = Modal::new("settings-modal", state.show_settings_modal)
        .width_fraction(0.72)
        .height_fraction(0.82)
        .min_size(vec2(620.0, 420.0))
        .max_size(vec2(980.0, 760.0))
        .header_height(56.0)
        .show(ctx, &t, |frame| {
            frame.header(|ui, close_requested| {
                render_modal_header(
                    ui,
                    close_requested,
                    &t,
                    ModalHeader {
                        glyph: Some(egui_phosphor::regular::GEAR_SIX),
                        glyph_size: 16.0,
                        close_tip: loc.modal_tip_dismiss,
                        left_pad: 20.0,
                        glyph_pad: 10.0,
                        right_pad: 20.0,
                        item_spacing_x: Some(10.0),
                    },
                    |ui| {
                        widgets::hud(ui, loc.settings_title, t.ink_strong);
                    },
                    |_ui| {},
                );
            });

            frame.body(|ui| {
                let rect = ui.max_rect().shrink2(vec2(20.0, 16.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.set_width(rect.width());
                    ui.set_height(rect.height());
                    render_settings_body(ui, state);
                });
            });
        })
    else {
        return;
    };

    if response.close_requested {
        state.close_settings_modal();
    }
}

/// The settings page itself, without the dialog chrome. Tests render this body directly.
pub(crate) fn render_settings_body(ui: &mut Ui, state: &mut GuiState) {
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

    // The field spans the page and sits above the scroll area, so it stays reachable while a
    // long page is scrolled.
    widgets::catalog_search_box(
        ui,
        &mut state.settings_search_query,
        loc.settings_search_placeholder,
    );
    ui.add_space(14.0);

    let filter = SettingsFilter::new(&state.settings_search_query);
    let mut ctx = RowContext {
        state: &mut *state,
        cfg: &mut cfg,
        changes: &mut changes,
        current_autorun,
        t: &t,
        loc,
    };

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll_area")
        .auto_shrink([false, false])
        // The page scrolls with the wheel and keyboard; the bar itself is kept out of the design
        // like on the other views.
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 16.0;
            let mut any_shown = false;
            for section in SETTINGS_SECTIONS {
                render_section(ui, section, &mut ctx, &filter, &mut any_shown);
            }
            if !any_shown {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    widgets::hud(ui, loc.settings_search_empty, t.muted);
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

/// Rows of one block the filter keeps. The block's name stays searchable even though the card
/// itself is untitled: a query that names the block keeps all of its rows. The dialog and the
/// header search both use this, so their row sets cannot drift apart.
fn matching_rows(
    section: SettingsSection,
    filter: &SettingsFilter,
    loc: &Strings,
) -> Vec<SettingsRow> {
    if filter.matches(section.heading(loc)) {
        return section.rows().to_vec();
    }
    section
        .rows()
        .iter()
        .copied()
        .filter(|row| filter.matches_row(*row, loc))
        .collect()
}

/// Rows whose own text matches `query`, for the header global search. It applies the same filter
/// as the settings dialog, including the block-name alias, and returns the row titles and
/// descriptions as previews.
pub(crate) fn search_settings(query: &str, loc: &Strings) -> Vec<(&'static str, String)> {
    let filter = SettingsFilter::new(query);
    let mut rows = Vec::new();
    for section in SETTINGS_SECTIONS {
        for row in matching_rows(section, &filter, loc) {
            let (title, description) = row.text(loc);
            rows.push((title, description));
        }
    }
    rows
}

/// Key of the unsaved settings draft in egui's temporary data.
fn pending_settings_id() -> egui::Id {
    egui::Id::unique("settings_unsaved_draft")
}

/// Key of the expanded accent editor state in egui's temporary data.
fn accent_editor_id() -> egui::Id {
    egui::Id::unique("settings-accent-editor")
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

/// Everything a setting row reads or writes this frame. Passing one context instead of six
/// arguments keeps the row dispatcher and its helpers readable.
struct RowContext<'a> {
    state: &'a mut GuiState,
    cfg: &'a mut crate::config::Settings,
    changes: &'a mut SettingsChanges,
    current_autorun: bool,
    t: &'a Tokens,
    loc: &'a Strings,
}

/// Renders one block: the rows the filter kept, in page order, inside their card. A block with
/// no surviving row paints nothing at all, so a search cannot leave an empty card behind.
fn render_section(
    ui: &mut Ui,
    section: SettingsSection,
    ctx: &mut RowContext<'_>,
    filter: &SettingsFilter,
    any_shown: &mut bool,
) {
    let t = ctx.t;
    let rows = matching_rows(section, filter, ctx.loc);
    if rows.is_empty() {
        return;
    }
    *any_shown = true;

    egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(t.card_corner())
        .inner_margin(egui::Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;

            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    widgets::hrule(ui);
                }
                let (title, description) = row.text(ctx.loc);
                render_setting_row(ui, title, &description, t, |ui| {
                    render_row_control(ui, *row, ctx);
                });
                if *row == SettingsRow::Accent {
                    render_accent_editor(
                        ui,
                        ctx.state,
                        ctx.cfg,
                        &mut ctx.changes.config,
                        t,
                        ctx.loc,
                    );
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

/// The control a row puts on the right of its line.
fn render_row_control(ui: &mut Ui, row: SettingsRow, ctx: &mut RowContext<'_>) {
    match row {
        SettingsRow::Autorun => {
            let mut autorun_val = ctx.current_autorun;
            if widgets::toggle_switch(ui, &mut autorun_val).changed() {
                ctx.state.toggle_autorun(autorun_val);
            }
        }
        SettingsRow::TrayIcon => {
            render_tray_icon_control(ui, ctx.state, ctx.cfg, ctx.changes, ctx.t, ctx.loc)
        }
        SettingsRow::HideTrayIcon => {
            if widgets::toggle_switch(ui, &mut ctx.cfg.hide_tray_icon).changed() {
                ctx.changes.config = true;
            }
        }
        // The daemon applies masking deployment only at start-up, and `ShutdownDaemon` also posts
        // WM_CLOSE to this GUI window (dropping unsaved drafts). The view therefore persists the
        // preference through the normal commit path and tells the user a restart is required; it
        // never stops the daemon from the frame thread.
        SettingsRow::MaskDaemon => {
            if widgets::toggle_switch(ui, &mut ctx.cfg.mask_daemon_process).changed() {
                ctx.changes.config = true;
                ctx.changes.mask = true;
            }
        }
        // Disabling takes effect in this process immediately and in the daemon when it saves the
        // section; the log files already written are left alone, and turning the switch back on
        // resumes appending to the same day's file.
        SettingsRow::DiskLogging => {
            if widgets::toggle_switch(ui, &mut ctx.cfg.log_to_disk).changed() {
                ctx.changes.config = true;
                crate::logging::set_disk_logging_enabled(ctx.cfg.log_to_disk);
            }
        }
        SettingsRow::ConfigFolder => {
            let open = widgets::button(ui, ctx.loc.settings_btn_open_folder, Kind::Secondary, 0.0);
            if open.clicked() {
                let dir = crate::config::AppConfig::config_dir();
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            }
        }
        SettingsRow::Language => {
            render_language_control(ui, ctx.state, ctx.cfg, &mut ctx.changes.config, ctx.loc)
        }
        SettingsRow::Theme => {
            render_theme_control(ui, ctx.state, ctx.cfg, &mut ctx.changes.config, ctx.t, ctx.loc)
        }
        SettingsRow::Skin => {
            render_skin_control(ui, ctx.state, ctx.cfg, &mut ctx.changes.config, ctx.t, ctx.loc)
        }
        SettingsRow::Accent => {
            render_accent_control(ui, ctx.state, ctx.cfg, &mut ctx.changes.config, ctx.t, ctx.loc)
        }
        SettingsRow::UpdatesOnStartup => {
            if widgets::toggle_switch(ui, &mut ctx.cfg.check_updates_on_startup).changed() {
                ctx.changes.config = true;
            }
        }
        SettingsRow::AutoInstallUpdates => {
            if widgets::toggle_switch(ui, &mut ctx.cfg.auto_install_updates).changed() {
                ctx.changes.config = true;
            }
        }
        SettingsRow::InstalledVersion => render_update_check_control(ui, ctx.state, ctx.loc),
    }
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

/// Tray-icon row: a live thumbnail of the effective icon, the editable path and the reset button.
fn render_tray_icon_control(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changes: &mut SettingsChanges,
    t: &Tokens,
    loc: &Strings,
) {
    let edit_id = egui::Id::unique("settings_tray_icon_edit");
    let input_id = egui::Id::unique("settings_tray_icon_input");
    let mut edit = ui.data(|d| d.get_temp::<TrayIconPathEdit>(edit_id)).unwrap_or_default();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Live preview button matching the exact size of the input line next to it.
        let control_h = theme::control_height(ui);
        let preview_size = vec2(control_h, control_h);
        let (p_rect, p_resp) = ui.allocate_exact_size(preview_size, Sense::click());
        let p_hovered = p_resp.hovered();
        let p_stroke = if p_hovered { Stroke::new(1.0, t.line_strong) } else { t.hairline() };
        ui.painter().rect(p_rect, t.card_corner(), t.bg, p_stroke, egui::StrokeKind::Inside);

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
                .min_size(vec2(input_w, control_h))
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
                t.card_corner(),
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
}

/// Interface language row.
fn render_language_control(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    loc: &Strings,
) {
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
}

/// Colour theme row as palette preview swatches: each tile is a miniature window — a thin accent
/// strip over a content well — so the palette previews itself instead of carrying a caption. The
/// name stays as the tooltip and as the accessibility label.
fn render_theme_control(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    t: &Tokens,
    loc: &Strings,
) {
    let themes = [
        (crate::config::AppTheme::Dark, loc.settings_theme_dark),
        (crate::config::AppTheme::Light, loc.settings_theme_light),
    ];

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        for (thm, label) in themes {
            // The preview comes from the live palette, so a Tokens change cannot
            // desynchronize the swatches from the theme the user actually gets.
            let palette = Tokens::of(thm, t.metrics().skin);
            let is_selected = state.theme == thm;
            let (rect, resp) =
                ui.allocate_exact_size(vec2(96.0, t.metrics().control_h), Sense::click());
            if resp.clicked() && !is_selected {
                state.set_theme(thm);
                cfg.theme = thm;
                *changed = true;
            }

            let well = paint_preview_tile(
                ui,
                rect,
                t,
                palette.palette(),
                t.metrics(),
                is_selected,
                resp.hovered(),
            );
            // Two faint lines read as content rows inside the miniature window.
            for row in 0..2 {
                let y = well.top() + 7.0 + row as f32 * 7.0;
                if y + 1.0 < well.bottom() {
                    ui.painter().hline(
                        egui::Rangef::new(well.left() + 5.0, well.right() - 5.0),
                        y,
                        Stroke::new(1.0, palette.line),
                    );
                }
            }

            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, is_selected, label)
            });
            resp.on_hover_text(label)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    });
}

/// Skin generation row: preview tiles, mirroring the palette swatches above, so a third or
/// fourth generation can be added as one more tile.
fn render_skin_control(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    t: &Tokens,
    loc: &Strings,
) {
    let skins = [
        (crate::config::UiSkin::Modern, loc.settings_skin_modern),
        (crate::config::UiSkin::Classic, loc.settings_skin_classic),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (skin, label) in skins {
            let metrics = Metrics::of(skin);
            let palette = Tokens::of(state.theme, skin);
            let is_selected = state.ui_skin == skin;
            let (rect, resp) =
                ui.allocate_exact_size(vec2(96.0, t.metrics().control_h), Sense::click());
            if resp.clicked() && !is_selected {
                state.set_ui_skin(skin);
                cfg.ui_skin = skin;
                *changed = true;
            }

            // The miniature shows the generation's own geometry: strip, content well and a
            // control whose corner radius is the skin's.
            let well = paint_preview_tile(
                ui,
                rect,
                t,
                palette.palette(),
                metrics,
                is_selected,
                resp.hovered(),
            );
            let painter = ui.painter();
            let control_h = (well.height() - 10.0).clamp(6.0, 10.0);
            let control = Rect::from_min_size(
                well.min + vec2(5.0, 5.0),
                vec2((well.width() * 0.5).max(18.0), control_h),
            );
            painter.rect_filled(
                control,
                egui::CornerRadius::same(metrics.control_radius.min(8)),
                palette.accent_ink.gamma_multiply(0.7),
            );
            let dot = Rect::from_center_size(
                pos2(control.right() + 8.0, control.center().y),
                vec2(8.0, 8.0),
            );
            if dot.right() < well.right() && dot.bottom() < well.bottom() {
                painter.rect_filled(
                    dot,
                    egui::CornerRadius::same(metrics.control_radius.min(4)),
                    palette.sunken,
                );
            }

            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, is_selected, label)
            });
            resp.on_hover_text(label)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    });
}

/// Accent colour row: a few presets plus the shared HSVA editor for anything custom.
fn render_accent_control(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    t: &Tokens,
    loc: &Strings,
) {
    let accent = crate::gui::theme::parse_accent(state.accent.as_deref());
    let editor_id = accent_editor_id();

    const PRESETS: [(&str, [u8; 3]); 6] = [
        ("#2d81ff", [0x2d, 0x81, 0xff]),
        ("#8b7bd8", [0x8b, 0x7b, 0xd8]),
        ("#4fb3a0", [0x4f, 0xb3, 0xa0]),
        ("#d9a04a", [0xd9, 0xa0, 0x4a]),
        ("#e06a86", [0xe0, 0x6a, 0x86]),
        ("#9aa0aa", [0x9a, 0xa0, 0xaa]),
    ];
    // The presets and the two buttons share one wrapped row: in a narrow settings panel the
    // buttons drop to a second line instead of being clipped.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (hex, rgb) in PRESETS {
            let selected = accent == Some(rgb);
            let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            ui.painter().circle_filled(
                rect.center(),
                9.0,
                Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
            );
            if selected {
                ui.painter().circle_stroke(rect.center(), 11.0, Stroke::new(2.0, t.ink_strong));
            } else if resp.hovered() {
                ui.painter().circle_stroke(rect.center(), 11.0, t.hairline());
            }
            if resp.clicked() && !selected {
                state.set_accent(Some(hex.to_string()));
                cfg.accent = Some(hex.to_string());
                *changed = true;
            }
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, hex)
            });
            resp.on_hover_text(hex);
        }

        let button_h = t.metrics().control_h;
        if widgets::compact_button(ui, loc.settings_accent_custom, Kind::Secondary, button_h)
            .clicked()
        {
            let open = ui.data(|d| d.get_temp::<bool>(editor_id)).unwrap_or(false);
            ui.data_mut(|d| d.insert_temp(editor_id, !open));
        }

        if accent.is_some()
            && widgets::compact_button(ui, loc.settings_btn_reset, Kind::Secondary, button_h)
                .clicked()
        {
            // Clearing the override brings back the palette accent. The editor closes with
            // it, otherwise the next frame would write the old custom colour straight back.
            state.set_accent(None);
            cfg.accent = None;
            ui.data_mut(|d| d.insert_temp(editor_id, false));
            *changed = true;
        }
    });
}

/// The expanded accent editor, shown under the accent row while its CUSTOM button is on.
fn render_accent_editor(
    ui: &mut Ui,
    state: &mut GuiState,
    cfg: &mut crate::config::Settings,
    changed: &mut bool,
    t: &Tokens,
    loc: &Strings,
) {
    if !ui.data(|d| d.get_temp::<bool>(accent_editor_id())).unwrap_or(false) {
        return;
    }
    let accent = crate::gui::theme::parse_accent(state.accent.as_deref());
    let mut rgba = accent
        .map(|[r, g, b]| [r, g, b, 255])
        .unwrap_or([0x2d, 0x81, 0xff, 255]);
    let edited =
        crate::gui::components::color_picker::inline_color_editor(ui, &mut rgba, false, t, loc);
    let rgb = [rgba[0], rgba[1], rgba[2]];
    // Only a real interaction commits: the editor writes its own state back every frame, so
    // opening CUSTOM must not pin the palette accent as a custom colour.
    if edited && accent != Some(rgb) {
        let hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        state.set_accent(Some(hex.clone()));
        cfg.accent = Some(hex);
        *changed = true;
    }
    ui.add_space(6.0);
}

/// Installed-version row: the manual check button; the description carries the running version.
fn render_update_check_control(ui: &mut Ui, state: &mut GuiState, loc: &Strings) {
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
}

/// Paints the chrome every appearance preview tile shares: the selection border, the page
/// colour, the accent strip and the content well. `metrics` belongs to the generation the tile
/// previews, so a skin tile shows its own geometry. Returns the well rect, where the caller
/// draws its miniature body.
fn paint_preview_tile(
    ui: &Ui,
    rect: Rect,
    t: &Tokens,
    palette: Palette,
    metrics: Metrics,
    selected: bool,
    hovered: bool,
) -> Rect {
    let border = if selected {
        Stroke::new(2.0, t.accent)
    } else if hovered {
        Stroke::new(t.metrics().hairline_w, t.line_strong)
    } else {
        t.hairline()
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same(metrics.card_radius),
        palette.bg,
        border,
        egui::StrokeKind::Inside,
    );

    let inset = 6.0;
    let strip = Rect::from_min_size(
        rect.min + vec2(inset, inset),
        vec2(rect.width() - 2.0 * inset, 6.0),
    );
    painter.rect_filled(
        strip,
        egui::CornerRadius::same(metrics.inset_radius),
        palette.accent,
    );

    let well = Rect::from_min_max(
        pos2(rect.left() + inset, strip.bottom() + 4.0),
        pos2(rect.right() - inset, rect.bottom() - inset),
    );
    painter.rect(
        well,
        egui::CornerRadius::same(metrics.inset_radius),
        palette.sunken,
        Stroke::new(1.0, palette.line),
        egui::StrokeKind::Inside,
    );
    well
}

/// Installed-version subtitle of the updates row. The catalog supplies the product name and
/// `{}`; the build version comes from `CARGO_PKG_VERSION`.
fn updates_version_subtitle(loc: &Strings) -> String {
    loc.settings_updates_version
        .replace("{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GUI state with a daemon snapshot, so the page renders instead of the offline notice.
    fn settings_state() -> GuiState {
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
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });
        state
    }

    /// Opening the custom-accent editor must not pin the palette accent as a custom colour: the
    /// editor writes its own state back every frame, so only a real interaction may commit.
    #[test]
    fn test_opening_the_accent_editor_does_not_commit_a_custom_colour() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = settings_state();
        state.accent = None;
        ctx.data_mut(|data| data.insert_temp(accent_editor_id(), true));

        let _ = page_texts(&ctx, &mut state, vec2(1000.0, 3000.0));

        assert!(
            state.accent.is_none(),
            "opening CUSTOM must not commit the palette accent as a custom colour"
        );
    }

    /// The settings page is a dialog now: the shared modal chrome paints the title and the body
    /// rows inside it, instead of a full-window page. Areas are drained on the following pass, so
    /// the dialog is rendered twice and the second frame is inspected.
    #[test]
    fn test_settings_modal_renders_title_and_rows() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = settings_state();
        state.show_settings_modal = true;
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let mut output = None;
        for time in [0.0, 0.1] {
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1180.0, 760.0))),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    render_settings_modal(ui.ctx(), &mut state);
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
        assert!(
            text_index(&texts, loc.settings_title).is_some(),
            "the dialog title must be painted: {texts:?}"
        );
        assert!(
            text_index(&texts, loc.settings_autorun_title).is_some(),
            "the settings body must be painted inside the dialog"
        );
    }

    /// Every text run the page painted, in paint order, with its top edge.
    fn page_texts(
        ctx: &egui::Context,
        state: &mut GuiState,
        size: egui::Vec2,
    ) -> Vec<(String, f32)> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| render_settings_body(ui, state));
        });
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        texts
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<(String, f32)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.galley.text().to_string(), text.pos.y)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
            _ => {}
        }
    }

    fn text_index(texts: &[(String, f32)], needle: &str) -> Option<usize> {
        texts.iter().position(|(text, _)| text == needle)
    }

    fn text_top(texts: &[(String, f32)], needle: &str) -> Option<f32> {
        texts
            .iter()
            .find(|(text, _)| text == needle)
            .map(|(_, top)| *top)
    }

    /// SETTINGS-BLOCKS: the page is one column of untitled blocks in a fixed order — the sections
    /// are no longer tabs, so every block renders on the same page, and no block repeats its name
    /// inside the card.
    #[test]
    fn test_settings_blocks_render_as_one_untitled_page() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = settings_state();
        let loc = crate::i18n::get(crate::i18n::Language::English);

        let texts = page_texts(&ctx, &mut state, vec2(1000.0, 3000.0));

        for row in SettingsRow::ALL {
            let (title, _) = row.text(loc);
            assert!(
                text_index(&texts, title).is_some(),
                "row {row:?} ({title:?}) was not painted"
            );
        }
        for section in SETTINGS_SECTIONS {
            let heading = section.heading(loc).to_uppercase();
            assert!(
                text_index(&texts, &heading).is_none(),
                "block heading {heading:?} must not be painted inside its card"
            );
        }

        // The blocks stack top to bottom: the first row of each block is the top edge of that
        // block, and every row of the block sits between that edge and the next block's.
        let tops: Vec<f32> = SETTINGS_SECTIONS
            .iter()
            .map(|section| {
                let first = section.rows()[0];
                text_top(&texts, first.text(loc).0)
                    .unwrap_or_else(|| panic!("block {section:?} was not painted"))
            })
            .collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "blocks must follow the page order: {tops:?}"
        );
        for (index, section) in SETTINGS_SECTIONS.iter().enumerate() {
            let bottom = tops.get(index + 1).copied().unwrap_or(f32::INFINITY);
            for row in section.rows() {
                let row_top = text_top(&texts, row.text(loc).0)
                    .unwrap_or_else(|| panic!("row {row:?} was not painted"));
                assert!(
                    row_top >= tops[index] && row_top < bottom,
                    "row {row:?} is outside block {section:?}"
                );
            }
        }
    }

    /// SETTINGS-SEARCH: the field spanning the page keeps only the rows whose own text matches,
    /// and a query naming a block keeps that whole block.
    #[test]
    fn test_settings_search_filters_the_blocks() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = settings_state();
        let loc = crate::i18n::get(crate::i18n::Language::English);

        // A substring of one description keeps exactly that row.
        state.settings_search_query = "system login".to_string();
        let texts = page_texts(&ctx, &mut state, vec2(1000.0, 3000.0));
        assert!(text_index(&texts, loc.settings_autorun_title).is_some());
        assert!(
            text_index(&texts, loc.settings_theme_title).is_none(),
            "a row outside the query must not be painted"
        );
        assert!(text_index(&texts, loc.settings_updates_startup_title).is_none());

        // A block name still finds every row of that block, even though the card shows no title.
        state.settings_search_query = loc.settings_section_appearance.to_string();
        let texts = page_texts(&ctx, &mut state, vec2(1000.0, 3000.0));
        assert!(text_index(&texts, loc.settings_theme_title).is_some());
        assert!(text_index(&texts, loc.settings_skin_title).is_some());
        assert!(text_index(&texts, loc.settings_autorun_title).is_none());
        assert!(text_index(&texts, loc.settings_updates_startup_title).is_none());

        // Nothing matches: the page reports it instead of showing empty blocks.
        state.settings_search_query = "no such setting".to_string();
        let texts = page_texts(&ctx, &mut state, vec2(1000.0, 3000.0));
        assert!(text_index(&texts, loc.settings_search_empty).is_some());
        for row in SettingsRow::ALL {
            let (title, _) = row.text(loc);
            assert!(
                text_index(&texts, title).is_none(),
                "row {row:?} must not survive a query that matches nothing"
            );
        }
    }

    /// The header search and the dialog share one row filter: a block name keeps the whole block
    /// in both languages, a substring keeps one row, and a query without a match keeps nothing.
    #[test]
    fn test_search_settings_matches_rows_and_block_aliases() {
        for language in [
            crate::i18n::Language::English,
            crate::i18n::Language::Russian,
        ] {
            let loc = crate::i18n::get(language);

            let by_block = search_settings(loc.settings_section_appearance, loc);
            assert_eq!(
                by_block.len(),
                SettingsSection::Appearance.rows().len(),
                "a block name keeps every row of that block"
            );
            assert!(
                by_block
                    .iter()
                    .any(|(title, _)| *title == loc.settings_theme_title),
                "the appearance block includes its theme row"
            );

            let by_row = search_settings(&loc.settings_autorun_title.to_lowercase(), loc);
            assert!(
                by_row.iter().any(|(title, _)| *title == loc.settings_autorun_title),
                "a row title matches by substring"
            );
            assert!(
                !by_row.iter().any(|(title, _)| *title == loc.settings_theme_title),
                "a row of another block must not match"
            );

            assert!(
                search_settings("no such setting", loc).is_empty(),
                "a query without a match keeps nothing"
            );
        }
    }

    /// The row table is the single source of truth for the search and the page: every row sits in
    /// exactly one block and carries catalog text in both languages.
    #[test]
    fn test_every_row_sits_in_one_block_with_catalog_text() {
        let rows: Vec<SettingsRow> = SETTINGS_SECTIONS
            .iter()
            .flat_map(|section| section.rows().iter().copied())
            .collect();
        assert_eq!(rows.len(), SettingsRow::ALL.len());
        for row in SettingsRow::ALL {
            assert_eq!(
                rows.iter().filter(|candidate| **candidate == row).count(),
                1,
                "row {row:?} must appear in exactly one block"
            );
        }

        for loc in [
            crate::i18n::get(crate::i18n::Language::English),
            crate::i18n::get(crate::i18n::Language::Russian),
        ] {
            for row in SettingsRow::ALL {
                let (title, description) = row.text(loc);
                assert!(!title.is_empty(), "row {row:?} has no title");
                assert!(!description.is_empty(), "row {row:?} has no description");
            }
            for section in SETTINGS_SECTIONS {
                assert!(!section.heading(loc).is_empty());
            }
        }
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
        let source = include_str!("settings_modal.rs");
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
        let source = include_str!("settings_modal.rs");
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
