pub mod components;
pub mod macro_recorder;
pub mod overlay_window;
pub mod state;
pub mod theme;
pub mod views;

use eframe::App;
use egui::{Color32, CornerRadius, Stroke, Ui, vec2};
use components::widgets;
use state::{ActiveView, GuiState};
use theme::{Tokens, apply_theme};
use views::analytics_view::render_analytics_view;
use views::catalogs::action::render_action_catalog_modal;
use views::catalogs::condition::render_condition_catalog_modal;
use views::catalogs::element::render_element_catalog_modal;
use views::catalogs::trigger::render_trigger_catalog_modal;
use views::changelog_modal::render_changelog_modal;
use views::guide_modal::render_guide_modal;
use views::header_bar::render_header_bar;
use views::inspector::render_inspector_drawer;
use views::macro_modal::{render_macro_modal, render_macro_presets_modal};
use views::overlay_modal::{render_overlay_modal, render_overlay_presets_modal};
use views::process_picker_modal::render_process_picker_modal;
use views::rules_view::render_rules_view;
use views::script_modal::render_script_editor_canvas;
use views::settings_view::render_settings_view;
use crate::config::{Settings, WindowState};

/// Window geometry is streamed by every frame of a drag/resize; writing it that often would
/// cost an IPC round-trip plus fsync per frame. The latest state is still tracked every frame
/// and persisted at this rate, and `on_exit`/`Drop` always write the exact final geometry.
const WINDOW_STATE_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether this frame must report a confirmed daemon outage once. Kept as a pure decision so the
/// "log once, keep the window" contract is testable without a GUI.
fn should_report_daemon_outage(
    has_connected_daemon: bool,
    is_daemon_connected: bool,
    already_reported: bool,
) -> bool {
    has_connected_daemon && !is_daemon_connected && !already_reported
}

pub struct SynectorApp {
    state: GuiState,
    fonts_ready: bool,
    current_theme: Option<crate::config::AppTheme>,
    current_skin: Option<crate::config::UiSkin>,
    current_accent: Option<[u8; 3]>,
    /// Latest geometry observed in this frame (used for change detection and exit persistence).
    last_window_state: Option<WindowState>,
    /// Geometry that is already on disk; compared against to decide what needs a write.
    persisted_window_state: Option<WindowState>,
    last_window_persist: std::time::Instant,
    startup_focused_frames: u8,
    /// Set after the shutdown flush ran, so `Drop` does not repeat it after `on_exit`.
    exit_flushed: bool,
    /// Set after a confirmed daemon outage was reported, so it is logged once per outage.
    daemon_outage_reported: bool,
}

impl SynectorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let mut state = GuiState::new();
        state.sync_with_daemon();
        let config = Settings::read();
        let accent = theme::parse_accent(config.accent.as_deref());
        theme::apply_theme(&cc.egui_ctx, config.theme, config.ui_skin, accent);
        let last_window_state = config.window;
        Self {
            state,
            fonts_ready: false,
            current_theme: Some(config.theme),
            current_skin: Some(config.ui_skin),
            current_accent: accent,
            persisted_window_state: last_window_state.clone(),
            last_window_state,
            // Persist the first change immediately instead of waiting out the first window.
            last_window_persist: std::time::Instant::now() - WINDOW_STATE_SAVE_INTERVAL,
            startup_focused_frames: 0,
            exit_flushed: false,
            daemon_outage_reported: false,
        }
    }

    fn render_footer(&mut self, ui: &mut Ui, t: &Tokens) {
        let loc = self.state.loc();
        // One footer row height in both skins: the modern control row is taller than the footer
        // panel and used to shift the status line down by a pixel or two.
        ui.spacing_mut().interact_size.y = theme::CONTROL_H;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            if let Some(ds) = &self.state.daemon_state {
                let active = ds.active_rules_count;
                let total = ds.total_rules_count;
                widgets::hud(ui, &format!("{}/{} {}", active, total, loc.footer_rules_active), t.faint);
            }

            // Right-aligned status
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let autorun = self.state.daemon_state.as_ref().map(|s| s.autorun_enabled).unwrap_or(false);
                widgets::hud(
                    ui,
                    if autorun { loc.footer_autorun_on } else { loc.footer_autorun_off },
                    if autorun { t.good } else { t.muted },
                );

                widgets::rule(ui, 14.0);
                widgets::hud(ui, &format!("v{}", env!("CARGO_PKG_VERSION")), t.faint);

                if self.state.is_downloading_update {
                    widgets::rule(ui, 14.0);
                    let pct = (self.state.update_download_progress * 100.0).clamp(0.0, 100.0);
                    let progress_text = loc.footer_downloading_progress.replace("{:.0}", &format!("{:.0}", pct));
                    widgets::hud(ui, &progress_text, t.accent_ink);
                } else if let Some(info) = self.state.available_update.clone() {
                    widgets::rule(ui, 14.0);

                    if widgets::compact_button(ui, loc.footer_btn_update, widgets::Kind::Secondary, 22.0)
                        .on_hover_text(loc.footer_btn_update_tip)
                        .clicked()
                    {
                        self.state.start_update(info.clone());
                    }

                    if widgets::compact_button(ui, loc.footer_btn_info, widgets::Kind::Secondary, 22.0)
                        .on_hover_text(loc.footer_btn_info_tip)
                        .clicked()
                    {
                        self.state.show_changelog_modal = true;
                    }

                    let update_label = loc.footer_update_available.replace("{}", &info.version);
                    widgets::hud(ui, &update_label, t.good);
                }
            });
        });
    }

    fn render_toast(&self, ui: &mut Ui, t: &Tokens) {
        if let Some((msg, time)) = &self.state.toast {
            let elapsed = time.elapsed().as_secs_f32();
            if elapsed < 3.0 {
                // Smooth frame repainting during fade in/out
                ui.ctx().request_repaint();

                let alpha = if elapsed < 0.25 {
                    (elapsed / 0.25).min(1.0)
                } else if elapsed > 2.5 {
                    ((3.0 - elapsed) / 0.5).max(0.0)
                } else {
                    1.0
                };

                let fill = Color32::from_black_alpha((alpha * 240.0) as u8);
                let stroke = Stroke::new(1.0, Color32::from_white_alpha((alpha * 120.0) as u8));
                let text_color = Color32::from_white_alpha((alpha * 255.0) as u8);

                egui::Area::new(egui::Id::unique("app-toast"))
                    .order(egui::Order::Tooltip)
                    .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -t.metrics().footer_h - 24.0))
                    .interactable(false)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::new()
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(CornerRadius::ZERO)
                            .inner_margin(egui::Margin::symmetric(24, 10))
                            .show(ui, |ui| {
                                let galley = widgets::caps(
                                    ui,
                                    msg,
                                    theme::HUD_SIZE,
                                    theme::mono_bold(),
                                    theme::HUD_SPACING,
                                    text_color,
                                );
                                let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                                widgets::paint_centred(ui, rect, galley, text_color);
                            });
                    });
            }
        }
    }
}

impl App for SynectorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            self.fonts_ready = true;
            ctx.request_repaint();
            return;
        }
        let accent = theme::parse_accent(self.state.accent.as_deref());
        if self.current_theme != Some(self.state.theme)
            || self.current_skin != Some(self.state.ui_skin)
            || self.current_accent != accent
        {
            apply_theme(&ctx, self.state.theme, self.state.ui_skin, accent);
            self.current_theme = Some(self.state.theme);
            self.current_skin = Some(self.state.ui_skin);
            self.current_accent = accent;
        }

        // Sync with daemon state
        self.state.sync_with_daemon();
        // Background share-code compression finishes on a worker thread; collect it here.
        self.state.poll_share_events(&ctx);

        // A confirmed daemon outage must never destroy unsaved drafts. Keep the window open with
        // the offline state (reported by the settings view and the outage toast) and log the
        // failure once; the sync layer stored the failure detail when the outage was confirmed.
        if should_report_daemon_outage(
            self.state.has_connected_daemon,
            self.state.is_daemon_connected,
            self.daemon_outage_reported,
        ) {
            self.daemon_outage_reported = true;
            crate::logging::error(
                "GUI",
                &format!(
                    "Daemon connection lost after {} failed polls; keeping the window open with the offline state so unsaved drafts survive (the failure detail was logged by the state sync)",
                    self.state.failed_daemon_syncs
                ),
            );
        } else if self.state.is_daemon_connected {
            // A recovered daemon must be able to report a later outage again.
            self.daemon_outage_reported = false;
        }
        self.state.poll_update_events();
        if self.state.is_any_rule_executing() {
            ctx.request_repaint();
        }

        if self.startup_focused_frames < 3 {
            self.startup_focused_frames += 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            #[cfg(windows)]
            {
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
                    use windows::core::w;
                    if let Ok(hwnd) = FindWindowW(None, w!("Synector")) {
                        if !hwnd.0.is_null() {
                            crate::single_instance::force_foreground_window(hwnd);
                        }
                    }
                }
            }
        }

        // Handle GUI window close while update is downloading:
        // Background daemon handles the download seamlessly and will restart with --daemon when GUI is closed.
        if ctx.input(|i| i.viewport().close_requested()) && self.state.is_downloading_update {
            self.state.gui_closed_during_update = true;
            crate::updater::set_gui_closed_during_update(true);
        }

        if self.state.is_downloading_update {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let tokens = Tokens::get(&ctx);

        // A running palette cross-fade re-derives the native visuals each frame; it is a no-op
        // otherwise.
        theme::tick_palette_fade(&ctx);

        // The backdrop sits under every panel. The classic skin paints the flat page colour;
        // the modern skin paints a quiet wash that its translucent surfaces show through.
        theme::paint_backdrop(&ctx, &tokens);

        // 1. Top Header Bar (Title Bar & Navigation)
        egui::Panel::top("header")
            .exact_size(tokens.metrics().header_h)
            .resizable(false)
            .frame(egui::Frame::new().fill(tokens.surface()))
            .show(ui, |ui| {
                render_header_bar(ui, &mut self.state);
            });

        // 2. Bottom Footer Bar (Telemetry, Memory, Autorun)
        egui::Panel::bottom("footer")
            .exact_size(tokens.metrics().footer_h)
            .resizable(false)
            // The panel's built-in separator reserves an outer-margin row and lands below the
            // border, which shifts the footer's visible top edge between skins. The hairline is
            // painted on the border instead, in the skin's own hairline colour.
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(tokens.surface()).inner_margin(egui::Margin::symmetric(20, 0)))
            .show(ui, |ui| {
                self.render_footer(ui, &tokens);
            });

        // 3. Central Stage (Wide, comfortable, elegant)
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(tokens.surface()).inner_margin(egui::Margin { left: 24, right: 24, top: 18, bottom: 8 }))
            .show(ui, |ui| {
                match self.state.active_view {
                    ActiveView::Rules => render_rules_view(ui, &mut self.state),
                    ActiveView::Analytics => render_analytics_view(ui, &mut self.state),
                    ActiveView::Settings => render_settings_view(ui, &mut self.state),
                }

                // Toast notification
                self.render_toast(ui, &tokens);
            });

        // 4. Sliding Inspector Drawer (slides in from right with cubic easing)
        render_inspector_drawer(&ctx, &mut self.state);

        // 4.1. Trigger Catalog Modal (library for selecting rule trigger)
        render_trigger_catalog_modal(&ctx, &mut self.state);

        // 4.2. Pre-condition Catalog Modal (library for selecting pre-conditions)
        render_condition_catalog_modal(&ctx, &mut self.state);

        // 5. Action Catalog Modal (library for selecting rule actions)
        render_action_catalog_modal(&ctx, &mut self.state);

        // 7. Centered Expanded Script Canvas (modal popup for pwsh / cmd editing)
        render_script_editor_canvas(&ctx, &mut self.state);

        // 8. Centered Interactive Macro Studio Modal (record / edit keystrokes & mouse)
        render_macro_modal(&ctx, &mut self.state);

        // 8.0.1. Macro Presets Modal (save / load macro templates)
        render_macro_presets_modal(&ctx, &mut self.state);

        // 8.1. Centered Interactive Overlay Studio Modal (visual WYSIWYG overlay designer)
        render_overlay_modal(&ctx, &mut self.state);

        // 8.1.1. Overlay Element Catalog Modal (library for selecting overlay elements & widgets)
        render_element_catalog_modal(&ctx, &mut self.state);

        // 8.1.2. Overlay Presets Modal (save / load overlay templates)
        render_overlay_presets_modal(&ctx, &mut self.state);

        // 8.2. Process Picker Modal (Hunter inside studio modal for selecting a running process)
        render_process_picker_modal(&ctx, &mut self.state);

        // 9. Centered Changelog & Release Notes Modal
        render_changelog_modal(&ctx, &mut self.state);

        // 10. Centered Editor Guide & Documentation Modal
        render_guide_modal(&ctx, &mut self.state);

        // One footer separator for both skins: painted after the panels so the central panel
        // cannot cover the boundary row, and in the background layer so the inspector drawer and
        // other Areas stay above it.
        let footer_y = ctx.content_rect().bottom() - tokens.metrics().footer_h;
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::unique("footer-separator"),
        ))
        .hline(ctx.content_rect().x_range(), footer_y + 0.5, tokens.hairline());

        // 7. Borderless Window Edges (8-direction border resize & hairline stroke)
        widgets::window_edges(&ctx);

        // Track window size and position for persistence across restarts
        let vp = ctx.input(|i| i.viewport().clone());
        let is_max = vp.maximized.unwrap_or(false);
        let mut new_state = None;

        if is_max {
            if let Some(ws) = &self.last_window_state {
                let mut updated = ws.clone();
                updated.is_maximized = true;
                new_state = Some(updated);
            }
        } else if let Some(rect) = vp.inner_rect {
            if rect.width() >= 400.0 && rect.height() >= 300.0 {
                new_state = Some(WindowState {
                    x: rect.min.x,
                    y: rect.min.y,
                    width: rect.width(),
                    height: rect.height(),
                    is_maximized: false,
                });
            }
        }

        if let Some(new_ws) = new_state {
            let changed = match &self.persisted_window_state {
                Some(old) => {
                    (old.x - new_ws.x).abs() > 2.0
                        || (old.y - new_ws.y).abs() > 2.0
                        || (old.width - new_ws.width).abs() > 2.0
                        || (old.height - new_ws.height).abs() > 2.0
                        || old.is_maximized != new_ws.is_maximized
                }
                None => true,
            };
            if changed && self.last_window_persist.elapsed() >= WINDOW_STATE_SAVE_INTERVAL {
                persist_window_state(&new_ws);
                self.last_window_persist = std::time::Instant::now();
                self.persisted_window_state = Some(new_ws.clone());
            }
            // Always keep the latest geometry so on_exit/Drop can persist the final rect.
            self.last_window_state = Some(new_ws);
        }

        // Repaint periodically for telemetry
        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_on_exit();
    }
}

impl Drop for SynectorApp {
    fn drop(&mut self) {
        self.flush_on_exit();
    }
}

impl SynectorApp {
    /// Shutdown flush, bounded by the IPC client's exit budget. eframe calls `on_exit` and then
    /// drops the app, so the flag keeps the flush to one pass. Window geometry (settings) is
    /// persisted first, so a hung daemon cannot starve it behind the preview-hide round-trips;
    /// `persist_window_state` still falls back to a direct section write when the daemon does not
    /// answer. Without the budget a hung daemon could hold the process for the 5 s per-call bound
    /// on every preview and again for the geometry.
    fn flush_on_exit(&mut self) {
        if self.exit_flushed {
            return;
        }
        self.exit_flushed = true;
        crate::ipc::IpcClient::begin_shutdown_budget();
        if let Some(ws) = &self.last_window_state {
            persist_window_state(ws);
        }
        self.state.cleanup_on_exit();
    }
}

fn persist_window_state(ws: &WindowState) {
    if let Ok(crate::ipc::IpcResponse::Ok) = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::UpdateWindowState(ws.clone())) {
        return;
    }
    Settings::save_window_state(ws);
}

pub fn run_gui() -> eframe::Result<()> {
    let _instance_guard = match crate::single_instance::SingleInstanceGuard::acquire_gui() {
        Some(guard) => guard,
        None => {
            crate::single_instance::focus_existing_gui_window();
            return Ok(());
        }
    };

    let config = Settings::read();
    let saved_state = config.window;

    let mut viewport = egui::ViewportBuilder::default()
        .with_min_inner_size([880.0, 580.0])
        .with_title("Synector")
        .with_decorations(false);

    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app.png")) {
        viewport = viewport.with_icon(icon);
    }

    if let Some(ws) = &saved_state {
        viewport = viewport
            .with_inner_size([ws.width, ws.height])
            .with_position([ws.x, ws.y]);
        if ws.is_maximized {
            viewport = viewport.with_maximized(true);
        }
    } else {
        viewport = viewport.with_inner_size([1180.0, 760.0]);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Synector",
        native_options,
        Box::new(|cc| Ok(Box::new(SynectorApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_outage_is_reported_once_and_the_window_is_kept() {
        assert!(
            !should_report_daemon_outage(false, false, false),
            "a GUI that never reached the daemon has no outage to report"
        );
        assert!(
            should_report_daemon_outage(true, false, false),
            "a confirmed outage of a previously connected daemon is reported"
        );
        assert!(
            !should_report_daemon_outage(true, false, true),
            "one log line per outage, not one per frame"
        );
        assert!(
            !should_report_daemon_outage(true, true, false),
            "a live daemon is not an outage"
        );
    }
}
