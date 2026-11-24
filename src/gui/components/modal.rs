use eframe::emath::vec2;
use egui::{
    pos2, Color32, CornerRadius, Id, Order, Rect, Sense, Stroke, Ui, Vec2,
};
use crate::gui::components::widgets::{self, Glyph};
use crate::gui::theme::{self, Tokens};

/// Response returned from `Modal::show`.
pub struct ModalResponse {
    /// True if the user pressed Esc, clicked the scrim, or an internal close button was triggered.
    pub close_requested: bool,
    #[cfg(test)]
    pub anim: f32,
}

/// A structured frame passed to the modal's contents closure, providing isolated `Ui` scopes
/// for the header, toolbar, body, and footer slices.
pub struct ModalFrame<'a> {
    pub ui: &'a mut Ui,
    pub header_rect: Option<Rect>,
    pub toolbar_rect: Option<Rect>,
    pub body_rect: Rect,
    pub footer_rect: Option<Rect>,
    pub close_requested: &'a mut bool,
}

impl<'a> ModalFrame<'a> {
    /// Render content in the header slice (if `header_height` was specified).
    /// Provides `&mut Ui` scoped to the header rect and `&mut bool` to trigger close.
    pub fn header<R>(&mut self, add_contents: impl FnOnce(&mut Ui, &mut bool) -> R) -> Option<R> {
        let rect = self.header_rect?;
        let close_requested = &mut *self.close_requested;
        Some(scoped_slice(&mut *self.ui, rect, |ui| {
            add_contents(ui, close_requested)
        }))
    }

    /// Render content in the secondary toolbar slice below the header (if `toolbar_height` was specified).
    /// Provides `&mut Ui` scoped to the toolbar rect and `&mut bool` to trigger close.
    pub fn toolbar<R>(&mut self, add_contents: impl FnOnce(&mut Ui, &mut bool) -> R) -> Option<R> {
        let rect = self.toolbar_rect?;
        let close_requested = &mut *self.close_requested;
        Some(scoped_slice(&mut *self.ui, rect, |ui| {
            add_contents(ui, close_requested)
        }))
    }

    /// Render content in the main body area.
    pub fn body<R>(&mut self, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
        let rect = self.body_rect;
        scoped_slice(&mut *self.ui, rect, add_contents)
    }

    /// Render content in the footer slice (if `footer_height` was specified).
    /// Provides `&mut Ui` scoped to the footer rect and `&mut bool` to trigger close.
    pub fn footer<R>(&mut self, add_contents: impl FnOnce(&mut Ui, &mut bool) -> R) -> Option<R> {
        let rect = self.footer_rect?;
        let close_requested = &mut *self.close_requested;
        Some(scoped_slice(&mut *self.ui, rect, |ui| {
            add_contents(ui, close_requested)
        }))
    }
}

/// Builds the isolated `Ui` for one modal slice: exact rect, matching size and clip.
/// Shared by header, toolbar, body and footer so sizing and clipping stay in one place.
fn scoped_slice<R>(ui: &mut Ui, rect: Rect, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_width(rect.width());
        ui.set_height(rect.height());
        ui.set_clip_rect(rect);
        add_contents(ui)
    })
    .inner
}

/// Chrome and spacing of one [`render_modal_header`] call.
///
/// The paddings stay per call site because the overlay main header uses 18/6/16, the
/// overlay/macro presets dialogs 16/8/14 and the guide 20/8/16; the widget sequence itself is
/// shared. The title and any extra actions are closures, so a two-line title (guide, changelog)
/// or a header with controls (studio, script canvas) keeps its exact position.
pub struct ModalHeader<'a> {
    /// Optional phosphor glyph shown before the title in the accent ink.
    pub glyph: Option<&'a str>,
    /// Glyph size in points: 16 for the standard dialogs, 18 for the studio and guide headers.
    pub glyph_size: f32,
    pub close_tip: &'a str,
    pub left_pad: f32,
    pub glyph_pad: f32,
    pub right_pad: f32,
    /// Horizontal item spacing applied before the left padding; `None` keeps the `Ui` default.
    pub item_spacing_x: Option<f32>,
}

/// The repeated modal header: an optional glyph, the dialog title, and a right-to-left close glyph.
///
/// `title` renders in the left part of the header (the standard call sites pass `widgets::hud`);
/// the studio headers also append their action buttons there, so the leading cluster stays pinned
/// to the left edge; `trailing` renders after the close glyph inside the same right-to-left layout,
/// so a header with trailing actions keeps its order. Callers pass this straight into
/// `ModalFrame::header`.
pub fn render_modal_header(
    ui: &mut Ui,
    close_requested: &mut bool,
    t: &Tokens,
    header: ModalHeader<'_>,
    title: impl FnOnce(&mut Ui),
    trailing: impl FnOnce(&mut Ui),
) {
    ui.horizontal_centered(|ui| {
        if let Some(spacing_x) = header.item_spacing_x {
            ui.spacing_mut().item_spacing.x = spacing_x;
        }
        ui.add_space(header.left_pad);

        if let Some(glyph) = header.glyph {
            ui.label(
                egui::RichText::new(glyph)
                    .font(egui::FontId::new(header.glyph_size, theme::phosphor()))
                    .color(t.accent_ink),
            );
            ui.add_space(header.glyph_pad);
        }

        title(ui);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(header.right_pad);
            if widgets::glyph_button_quiet(ui, Glyph::Close, header.close_tip).clicked() {
                *close_requested = true;
            }
            trailing(ui);
        });
    });
}

/// Unified, high-performance base modal dialog builder.
///
/// Features:
/// - **Modern Cinematic animation**: graceful rise (`+24px -> 0px`), depth zoom (`0.92 -> 1.00`),
///   and smooth card opacity fade (`0.0 -> 1.0`) preventing harsh dark pop-in.
/// - **Asymmetric timing**: the active skin's motion metrics open slower than they close.
/// - **Consistent theming**: automatic depth shadow, background fills, border strokes, hairline dividers,
///   and optional signature top accent stripe.
/// - **Nested dialogs**: a modal opened over another modal dims the card below it instead of
///   stacking under it; its own scrim and popups stay ordered around its card.
/// - **Safe keyboard and backdrop interaction**: Escape key & backdrop click handling with optional inhibition.
pub struct Modal {
    id_str: &'static str,
    is_open: bool,
    width_fraction: f32,
    height_fraction: f32,
    min_size: Vec2,
    max_size: Vec2,
    close_on_escape: bool,
    close_on_scrim: bool,
    scrim_max_alpha: u8,
    header_height: Option<f32>,
    toolbar_height: Option<f32>,
    footer_height: Option<f32>,
    header_sunken: bool,
    body_sunken: bool,
    footer_sunken: bool,
}

impl Modal {
    pub fn new(id_str: &'static str, is_open: bool) -> Self {
        Self {
            id_str,
            is_open,
            width_fraction: 0.80,
            height_fraction: 0.80,
            min_size: vec2(580.0, 380.0),
            max_size: vec2(1180.0, 840.0),
            close_on_escape: true,
            close_on_scrim: true,
            scrim_max_alpha: 215,
            header_height: None,
            toolbar_height: None,
            footer_height: None,
            header_sunken: false,
            body_sunken: false,
            footer_sunken: false,
        }
    }

    pub fn width_fraction(mut self, frac: f32) -> Self {
        self.width_fraction = frac.clamp(0.2, 0.98);
        self
    }

    pub fn height_fraction(mut self, frac: f32) -> Self {
        self.height_fraction = frac.clamp(0.2, 0.98);
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }

    pub fn max_size(mut self, size: Vec2) -> Self {
        self.max_size = size;
        self
    }

    pub fn close_on_escape(mut self, allow: bool) -> Self {
        self.close_on_escape = allow;
        self
    }

    pub fn close_on_scrim(mut self, allow: bool) -> Self {
        self.close_on_scrim = allow;
        self
    }

    pub fn header_height(mut self, height: f32) -> Self {
        self.header_height = Some(height);
        self
    }

    pub fn toolbar_height(mut self, height: f32) -> Self {
        self.toolbar_height = Some(height);
        self
    }

    pub fn footer_height(mut self, height: f32) -> Self {
        self.footer_height = Some(height);
        self
    }

    pub fn header_sunken(mut self, sunken: bool) -> Self {
        self.header_sunken = sunken;
        self
    }

    pub fn body_sunken(mut self, sunken: bool) -> Self {
        self.body_sunken = sunken;
        self
    }

    pub fn footer_sunken(mut self, sunken: bool) -> Self {
        self.footer_sunken = sunken;
        self
    }

    /// Renders the modal. Returns `Some(ModalResponse)` if the modal is currently visible or animating,
    /// or `None` if it is completely closed.
    pub fn show<R>(
        self,
        ctx: &egui::Context,
        t: &Tokens,
        render_content: impl FnOnce(&mut ModalFrame<'_>) -> R,
    ) -> Option<ModalResponse> {
        let anim_id = Id::unique(self.id_str).with("modal_cinematic_anim");

        // If this modal ID has never been seen before and is opening for the first time,
        // egui's `animate_bool` would default to 1.0 immediately (skipping the opening animation).
        // To ensure it smoothly animates 0.0 -> 1.0, prime the animation state to 0.0 on the very first frame.
        let seen_id = anim_id.with("initialized");
        let was_seen = ctx.data(|d| d.get_temp::<bool>(seen_id)).unwrap_or(false);
        if !was_seen {
            ctx.data_mut(|d| d.insert_temp(seen_id, true));
            if self.is_open {
                ctx.animate_bool_with_time_and_easing(
                    anim_id,
                    false,
                    0.0001,
                    egui::emath::easing::linear,
                );
            }
        }

        // The skin owns the duration; the platform may veto motion entirely.
        let motion = t.metrics().motion;
        let seconds = if self.is_open {
            motion.modal_open
        } else {
            motion.modal_close
        };
        let anim = theme::motion::animate_bool(ctx, anim_id, self.is_open, seconds);

        // Track the open transition ourselves: egui only raises layers it did not see on the
        // previous pass, which misses a reopen while the close animation still runs.
        let open_id = Id::unique(self.id_str).with("was_open");
        let was_open = ctx.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        ctx.data_mut(|d| d.insert_temp(open_id, self.is_open));

        if anim <= 0.001 && !self.is_open {
            // Once fully closed, remove initialization marker so subsequent opens are always guaranteed
            // to prime from 0.0 even if egui clears animation memory during idle.
            ctx.data_mut(|d| d.remove_temp::<bool>(seen_id));
            return None;
        }

        // Keep repainting while animating so motion is silky 60/144fps smooth
        if anim > 0.001 && anim < 0.999 {
            ctx.request_repaint();
        }

        let full = ctx.content_rect();

        // 1. Scrim overlay with quadratic opacity ease.
        // The scrim shares `Order::Foreground` with the card: a modal opened on top of another
        // modal must dim the card below it, so its scrim cannot live in a lower order than
        // every card. The two layers are re-linked as a parent/child pair at the end of this
        // function, which keeps the scrim under its own card and under the popups opened from it.
        let scrim_layer = egui::LayerId::new(Order::Foreground, Id::unique(self.id_str).with("scrim"));
        let window_layer = egui::LayerId::new(Order::Foreground, Id::unique(self.id_str).with("window"));

        // A dialog that opens rises above every dialog already open, scrim and card together.
        if self.is_open && !was_open {
            ctx.move_to_top(scrim_layer);
            ctx.move_to_top(window_layer);
        }

        let mut close_requested = false;
        let scrim_alpha = (anim.powf(0.85) * (self.scrim_max_alpha as f32)).round() as u8;

        if scrim_alpha > 0 {
            egui::Area::new(scrim_layer.id)
                .order(scrim_layer.order)
                .fixed_pos(full.min)
                .show(ctx, |ui| {
                    let (rect, resp) = ui.allocate_exact_size(full.size(), Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::ZERO,
                        Color32::from_black_alpha(scrim_alpha),
                    );
                    if resp.clicked() && self.close_on_scrim {
                        close_requested = true;
                    }
                });
        }

        // Close on Escape key if allowed. An open ComboBox/filter popup consumes Escape
        // first, so guard on egui's popup state and close only when nothing is open.
        if self.close_on_escape
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && !egui::Popup::is_any_open(ctx)
        {
            close_requested = true;
        }

        // 2. Cinematic motion curves:
        // - Scale: 0.92 -> 1.00 (noticeable physical depth)
        // - Y-shift: +24px -> 0px (graceful upward glide)
        // - Card opacity: 0.0 -> 1.0 (smoothly fades in, avoiding the harsh dark pop)
        let scale = 0.92 + 0.08 * anim;
        let y_shift = (1.0 - anim) * 24.0;
        let card_alpha = anim;

        let raw_w = (full.width() * self.width_fraction)
            .clamp(self.min_size.x, self.max_size.x)
            .min(full.width() - 32.0);
        let raw_h = (full.height() * self.height_fraction)
            .clamp(self.min_size.y, self.max_size.y)
            .min(full.height() - 48.0);
        let center = full.center();

        let modal_w = (raw_w * scale).round();
        let modal_h = (raw_h * scale).round();
        let min_x = (center.x - modal_w * 0.5).round();
        let min_y = (center.y + y_shift - modal_h * 0.5).round();
        let modal_rect = Rect::from_min_size(pos2(min_x, min_y), vec2(modal_w, modal_h));

        // 3. Slice layout geometry
        let header_h = self.header_height.unwrap_or(0.0);
        let toolbar_h = self.toolbar_height.unwrap_or(0.0);
        let footer_h = self.footer_height.unwrap_or(0.0);

        let header_rect = if header_h > 0.0 {
            Some(Rect::from_min_size(modal_rect.min, vec2(modal_rect.width(), header_h)))
        } else {
            None
        };

        let toolbar_rect = if toolbar_h > 0.0 {
            let top = modal_rect.top() + header_h;
            Some(Rect::from_min_size(pos2(modal_rect.left(), top), vec2(modal_rect.width(), toolbar_h)))
        } else {
            None
        };

        let footer_rect = if footer_h > 0.0 {
            let top = modal_rect.bottom() - footer_h;
            Some(Rect::from_min_size(pos2(modal_rect.left(), top), vec2(modal_rect.width(), footer_h)))
        } else {
            None
        };

        let body_top = modal_rect.top() + header_h + toolbar_h;
        let body_bottom = modal_rect.bottom() - footer_h;
        let body_rect = Rect::from_min_max(
            pos2(modal_rect.left(), body_top),
            pos2(modal_rect.right(), body_bottom),
        );

        // 4. Modal Surface Window (Foreground order ensures child ComboBox & popups render on top)
        egui::Area::new(window_layer.id)
            .order(window_layer.order)
            .fixed_pos(modal_rect.min)
            .show(ctx, |ui| {
                ui.set_width(modal_rect.width());
                ui.set_height(modal_rect.height());
                ui.set_max_width(modal_rect.width());
                ui.set_max_height(modal_rect.height());
                // Expand clip rect so the 24px drop shadow and 1px outer stroke are never clipped
                ui.set_clip_rect(modal_rect.expand(32.0));

                // 1. Paint background surfaces and precision dividers first
                {
                    let p = ui.painter();
                    let radius = CornerRadius::same(t.metrics().modal_radius);
                    let top_radius = CornerRadius {
                        nw: radius.nw,
                        ne: radius.ne,
                        sw: 0,
                        se: 0,
                    };
                    let bottom_radius = CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: radius.sw,
                        se: radius.se,
                    };

                    // Blooming drop shadow
                    if card_alpha > 0.02 {
                        let mut shadow = theme::shadow(t);
                        shadow.color = shadow.color.gamma_multiply(card_alpha);
                        p.add(shadow.as_shape(modal_rect, radius));
                    }

                    // Base background
                    p.rect_filled(modal_rect, radius, t.bg.gamma_multiply(card_alpha));

                    // Header fill
                    if let Some(h_rect) = header_rect {
                        let fill = if self.header_sunken { t.sunken } else { t.bg };
                        p.rect_filled(h_rect, top_radius, fill.gamma_multiply(card_alpha));
                    }

                    // Toolbar fill
                    if let Some(tb_rect) = toolbar_rect {
                        p.rect_filled(tb_rect, CornerRadius::ZERO, t.bg.gamma_multiply(card_alpha));
                    }

                    // Body fill if sunken
                    if self.body_sunken {
                        let cr = if footer_rect.is_some() {
                            CornerRadius::ZERO
                        } else {
                            bottom_radius
                        };
                        p.rect_filled(body_rect, cr, t.sunken.gamma_multiply(card_alpha));
                    }

                    // Footer fill
                    if let Some(f_rect) = footer_rect {
                        let fill = if self.footer_sunken { t.sunken } else { t.bg };
                        p.rect_filled(f_rect, bottom_radius, fill.gamma_multiply(card_alpha));
                    }
                }

                // 2. Render inner contents
                let mut frame = ModalFrame {
                    ui,
                    header_rect,
                    toolbar_rect,
                    body_rect,
                    footer_rect,
                    close_requested: &mut close_requested,
                };
                render_content(&mut frame);

                // 3. CRITICAL: Paint dividers, outer border stroke and top accent bar AFTER all inner contents
                // so no inner fills, table rows, frames, or sidebars can ever obscure or overwrite them!
                {
                    let p = ui.painter();
                    let div_color = t.hairline().color.gamma_multiply(card_alpha);

                    // Header divider (under header)
                    if let Some(h_rect) = header_rect {
                        let div_rect = Rect::from_min_max(
                            pos2(modal_rect.left() + 1.0, h_rect.bottom()),
                            pos2(modal_rect.right() - 1.0, h_rect.bottom() + 1.0),
                        );
                        p.rect_filled(div_rect, CornerRadius::ZERO, div_color);
                    }

                    // Toolbar divider (under toolbar)
                    if let Some(tb_rect) = toolbar_rect {
                        let div_rect = Rect::from_min_max(
                            pos2(modal_rect.left() + 1.0, tb_rect.bottom()),
                            pos2(modal_rect.right() - 1.0, tb_rect.bottom() + 1.0),
                        );
                        p.rect_filled(div_rect, CornerRadius::ZERO, div_color);
                    }

                    // Footer divider (above footer)
                    if let Some(f_rect) = footer_rect {
                        let div_rect = Rect::from_min_max(
                            pos2(modal_rect.left() + 1.0, f_rect.top() - 1.0),
                            pos2(modal_rect.right() - 1.0, f_rect.top()),
                        );
                        p.rect_filled(div_rect, CornerRadius::ZERO, div_color);
                    }

                    // Outer border stroke
                    p.rect_stroke(
                        modal_rect,
                        CornerRadius::same(t.metrics().modal_radius),
                        Stroke::new(t.metrics().hairline_w, t.line_strong.gamma_multiply(card_alpha)),
                        egui::StrokeKind::Inside,
                    );
                }
            });

        // 5. Pin the card directly above its own scrim. egui consumes sublayer relations at the
        // end of every pass, so the pair is re-linked each frame; `set_sublayer` keeps the two
        // layers adjacent even when egui raises one of them on a click, so a click on the scrim
        // can never park it over the card and a card raise can never bury its own popups.
        if scrim_alpha > 0 {
            ctx.set_sublayer(scrim_layer, window_layer);
        }

        Some(ModalResponse {
            close_requested,
            #[cfg(test)]
            anim,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modal_animation_starts_from_zero_on_first_open_and_reopen() {
        let ctx = egui::Context::default();

        // 1. First open: must start from 0.0, NOT 1.0
        let raw_input = egui::RawInput { time: Some(0.0), ..Default::default() };
        let mut out = ctx.run_ui(raw_input, |ui| {
            let modal = Modal::new("test-modal-first-open", true);
            let res = modal.show(ui.ctx(), &Tokens::DARK, |_frame| {});
            assert!(res.is_some());
            let res = res.unwrap();
            assert!(res.anim < 0.2, "Expected anim to start near 0.0, but got {}", res.anim);
        });
        out.textures_delta.clear();

        // 2. Advance time until fully open (anim == 1.0)
        let raw_input = egui::RawInput { time: Some(0.35), ..Default::default() };
        let mut out = ctx.run_ui(raw_input, |ui| {
            let modal = Modal::new("test-modal-first-open", true);
            let res = modal.show(ui.ctx(), &Tokens::DARK, |_frame| {});
            assert!(res.is_some());
            let res = res.unwrap();
            assert!((res.anim - 1.0).abs() < 0.01, "Expected anim to reach 1.0, got {}", res.anim);
        });
        out.textures_delta.clear();

        // 3. Close modal and advance time until fully closed (returns None)
        let raw_input = egui::RawInput { time: Some(0.60), ..Default::default() };
        let mut out = ctx.run_ui(raw_input, |ui| {
            let modal = Modal::new("test-modal-first-open", false);
            let res = modal.show(ui.ctx(), &Tokens::DARK, |_frame| {});
            assert!(res.is_none(), "Expected modal to return None when fully closed");
        });
        out.textures_delta.clear();

        // 4. Reopen modal: must start from 0.0 again!
        let raw_input = egui::RawInput { time: Some(1.0), ..Default::default() };
        let mut out = ctx.run_ui(raw_input, |ui| {
            let modal = Modal::new("test-modal-first-open", true);
            let res = modal.show(ui.ctx(), &Tokens::DARK, |_frame| {});
            assert!(res.is_some());
            let res = res.unwrap();
            assert!(res.anim < 0.2, "Expected reopened modal anim to start near 0.0, but got {}", res.anim);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn test_modal_escape_ignores_the_key_while_a_popup_is_open() {
        let ctx = egui::Context::default();
        let escape_input = || egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            time: Some(0.1),
            ..Default::default()
        };

        // No popup: Escape closes the modal.
        let mut out = ctx.run_ui(escape_input(), |ui| {
            let response = Modal::new("test-modal-escape-popup", true)
                .show(ui.ctx(), &Tokens::DARK, |_frame| {})
                .expect("modal is open");
            assert!(response.close_requested);
        });
        out.textures_delta.clear();

        // A ComboBox-like popup is open: Escape must only dismiss the popup.
        let mut out = ctx.run_ui(escape_input(), |ui| {
            egui::Popup::open_id(ui.ctx(), egui::Id::unique("test-modal-popup"));
            let response = Modal::new("test-modal-escape-popup", true)
                .show(ui.ctx(), &Tokens::DARK, |_frame| {})
                .expect("modal is open");
            assert!(
                !response.close_requested,
                "Escape must not close the modal while a popup owns it"
            );
        });
        out.textures_delta.clear();
    }

    /// Nested modals: the scrim of a modal opened on top must dim the card of the modal below it.
    /// The frame sequence mirrors the real UI — the upper dialog is opened from a control inside
    /// the lower card, which also raises that card — and the assertion reads the painted shapes,
    /// so it proves the visual order and not just the layer bookkeeping.
    #[test]
    fn test_nested_modal_scrim_dims_the_card_below_it() {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
        // 0.9 x 0.9 and 0.4 x 0.4 of the screen, positioned the way `Modal::show` does.
        let lower_card = Rect::from_min_size(pos2(64.0, 40.0), vec2(1152.0, 720.0));
        let upper_card = Rect::from_min_size(pos2(350.0, 210.0), vec2(580.0, 380.0));

        fn run_frame(
            ctx: &egui::Context,
            screen: Rect,
            time: f64,
            upper_open: bool,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    Modal::new("test-nested-lower", true)
                        .width_fraction(0.9)
                        .height_fraction(0.9)
                        .show(ui.ctx(), &Tokens::DARK, |_frame| {});
                    Modal::new("test-nested-upper", upper_open)
                        .width_fraction(0.4)
                        .height_fraction(0.4)
                        .show(ui.ctx(), &Tokens::DARK, |_frame| {});
                },
            );
            out.textures_delta.clear();
            out
        }

        // The lower modal is open and settled.
        run_frame(&ctx, screen, 0.0, false, Vec::new());
        run_frame(&ctx, screen, 0.4, false, Vec::new());

        // A press inside the lower card opens the upper dialog in the same frame, exactly like the
        // studio's PRESETS button does; that press also raises the lower card.
        run_frame(
            &ctx,
            screen,
            0.5,
            true,
            vec![egui::Event::PointerButton {
                pos: pos2(100.0, 400.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        run_frame(&ctx, screen, 0.6, true, Vec::new());
        let out = run_frame(&ctx, screen, 0.8, true, Vec::new());

        let layer = |id: &str, salt: &str| {
            egui::LayerId::new(Order::Foreground, Id::unique(id).with(salt))
        };
        let upper_scrim = layer("test-nested-upper", "scrim");
        let upper_layer = layer("test-nested-upper", "window");
        let lower_layer = layer("test-nested-lower", "window");

        // Inside the (larger) lower card but outside the upper card: the upper scrim owns it, so
        // the dim lands above the card below instead of sliding under it. Inside the upper card
        // its own scrim must stay underneath.
        assert_eq!(ctx.layer_id_at(pos2(100.0, 400.0)), Some(upper_scrim));
        assert_ne!(ctx.layer_id_at(pos2(100.0, 400.0)), Some(lower_layer));
        assert_eq!(ctx.layer_id_at(pos2(640.0, 400.0)), Some(upper_layer));

        // Same order in the painted output: lower card, then the upper scrim, then the upper card.
        let scrim_fill = Color32::from_black_alpha(215);
        let card_fill = Tokens::DARK.bg;
        let (mut lower_bg, mut scrim_idx, mut upper_bg) = (None, None, None);
        for (index, clipped) in out.shapes.iter().enumerate() {
            let egui::Shape::Rect(rect) = &clipped.shape else { continue };
            if rect.rect == screen && rect.fill == scrim_fill {
                // Both scrims match; keep the last one painted, which is the upper scrim.
                scrim_idx = Some(index);
            } else if rect.rect == lower_card && rect.fill == card_fill {
                lower_bg = Some(index);
            } else if rect.rect == upper_card && rect.fill == card_fill {
                upper_bg = Some(index);
            }
        }
        let lower_bg = lower_bg.expect("the lower card background is painted");
        let scrim_idx = scrim_idx.expect("the upper scrim is painted");
        let upper_bg = upper_bg.expect("the upper card background is painted");
        assert!(
            lower_bg < scrim_idx,
            "the upper scrim must be painted over the lower card (card {lower_bg}, scrim {scrim_idx})"
        );
        assert!(
            scrim_idx < upper_bg,
            "the upper card must be painted over its own scrim (scrim {scrim_idx}, card {upper_bg})"
        );
    }

    /// A dialog reopened while its close animation still runs is still on screen, so egui sees its
    /// layers as visible and raises nothing. The open transition tracked by `Modal::show` has to
    /// put the pair back above a card that was raised in the meantime.
    #[test]
    fn test_reopen_during_close_animation_returns_to_the_top() {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0));
        let lower_layer =
            egui::LayerId::new(Order::Foreground, Id::unique("test-reopen-lower").with("window"));
        let lower_scrim =
            egui::LayerId::new(Order::Foreground, Id::unique("test-reopen-lower").with("scrim"));
        let upper_scrim =
            egui::LayerId::new(Order::Foreground, Id::unique("test-reopen-upper").with("scrim"));

        fn run_frame(
            ctx: &egui::Context,
            screen: Rect,
            time: f64,
            upper_open: bool,
        ) -> Option<super::ModalResponse> {
            let mut seen = None;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    Modal::new("test-reopen-lower", true)
                        .width_fraction(0.9)
                        .height_fraction(0.9)
                        .show(ui.ctx(), &Tokens::DARK, |_frame| {});
                    seen = Modal::new("test-reopen-upper", upper_open)
                        .width_fraction(0.4)
                        .height_fraction(0.4)
                        .show(ui.ctx(), &Tokens::DARK, |_frame| {});
                },
            );
            out.textures_delta.clear();
            seen
        }

        run_frame(&ctx, screen, 0.0, false);
        run_frame(&ctx, screen, 0.4, false);
        run_frame(&ctx, screen, 0.7, true);
        run_frame(&ctx, screen, 0.9, true);
        assert_eq!(run_frame(&ctx, screen, 1.0, true).map(|r| r.anim), Some(1.0));

        // Start closing, then raise the whole lower pair the way a click does: the card alone
        // stays pinned under its own scrim, so both layers travel together.
        let closing = run_frame(&ctx, screen, 1.05, false).expect("the close animation started");
        assert!(closing.anim > 0.2, "the upper modal must still be visible, got {}", closing.anim);
        ctx.move_to_top(lower_scrim);
        ctx.move_to_top(lower_layer);
        let still_closing = run_frame(&ctx, screen, 1.07, false).expect("the frame is still painted");
        assert!(still_closing.anim > 0.0, "the upper modal must still be painted");
        assert_eq!(ctx.layer_id_at(pos2(100.0, 400.0)), Some(lower_layer));

        // Reopen before the fade ends: the pair must come back above the raised lower card.
        let reopened = run_frame(&ctx, screen, 1.1, true).expect("the reopened modal is visible");
        assert!(reopened.anim > 0.0, "the reopened modal animates in");
        assert_eq!(ctx.layer_id_at(pos2(100.0, 400.0)), Some(upper_scrim));
    }

    #[test]
    fn test_scoped_slice_sets_exact_rect_and_clip_rect() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let rect = Rect::from_min_size(pos2(20.0, 30.0), vec2(200.0, 80.0));
            let (max_rect, clip_rect) =
                scoped_slice(ui, rect, |inner| (inner.max_rect(), inner.clip_rect()));
            assert_eq!(max_rect, rect);
            assert_eq!(clip_rect, rect);
        });
        out.textures_delta.clear();
    }
}
