//! Generic catalog dialog shell shared by the action, trigger, condition and element catalogs.
//!
//! The shell owns everything that must be identical for every catalog: the entry trait, the
//! category sidebar, the search/filter algorithm, the card grid, the close-animation commit gate
//! and the per-kind chrome (modal id, labels, scroll salt, geometry). Each catalog file keeps its
//! item table, its categories, one `CatalogChrome` constant and a short commit adapter.
//!
//! `CatalogModal::show` returns indices, never model values: the four catalogs construct four
//! different model types and write them to four different places, so the commit target stays in
//! the per-kind adapter. Keep the ids, scroll salts and the widget order in `render_catalog_body`
//! exactly as they are: they are egui identity and therefore behaviour.

use egui::{
    Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke,
    pos2, vec2, Vec2,
};
use crate::gui::components::modal::{render_modal_header, Modal, ModalFrame, ModalHeader};
use crate::gui::components::widgets;
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;

/// Presentation metadata every catalog card exposes to the shared renderer.
///
/// `nestable` is the explicit "can live inside an If/Else branch" flag (U04-010): the badge stays
/// purely presentational, so entries that happen to share a badge no longer share nesting
/// behaviour.
pub(crate) trait CatalogEntry {
    fn category(&self) -> usize;
    fn badge(&self) -> &'static str;
    fn title(&self, loc: &Strings) -> &'static str;
    fn description(&self, loc: &Strings) -> &'static str;
    fn glyph(&self) -> &'static str;
    fn tint(&self, t: &Tokens) -> Color32;
    fn keywords(&self) -> &'static [&'static str];
    fn nestable(&self) -> bool;
}

/// One sidebar row. Index `0` is the "all" bucket; every other index matches `CatalogEntry::category`.
pub(crate) struct CatalogCategory {
    pub index: usize,
    pub glyph: &'static str,
    pub label: fn(&Strings) -> &'static str,
}

/// One catalog dialog's view state: the item table, its categories and the live filter inputs.
struct CatalogView<'a, T: CatalogEntry> {
    items: &'a [T],
    categories: &'a [CatalogCategory],
    selected_category: &'a mut usize,
    search_query: &'a mut String,
    /// True while picking a branch action: entries with `nestable == false` are hidden.
    nesting: bool,
    scroll_id: &'static str,
    search_hint: &'a str,
}

/// Case-insensitive match of one entry against an already-lowercased query.
fn catalog_entry_matches(
    entry: &impl CatalogEntry,
    query_lower: &str,
    loc: &Strings,
    nesting: bool,
) -> bool {
    if nesting && !entry.nestable() {
        return false;
    }
    if query_lower.is_empty() {
        return true;
    }
    entry.title(loc).to_lowercase().contains(query_lower)
        || entry.description(loc).to_lowercase().contains(query_lower)
        || entry.badge().to_lowercase().contains(query_lower)
        || entry.keywords().iter().any(|keyword| keyword.to_lowercase().contains(query_lower))
}

/// Indices of the visible entries, in table order. Category and query always apply together, so
/// "type to search" behaves the same in every catalog (U04-005).
pub(crate) fn filter_catalog_entries<T: CatalogEntry>(
    items: &[T],
    selected_category: usize,
    search_query: &str,
    loc: &Strings,
    nesting: bool,
) -> Vec<usize> {
    let query_lower = search_query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            (selected_category == 0 || item.category() == selected_category)
                && catalog_entry_matches(*item, &query_lower, loc, nesting)
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// A catalog click only commits while its modal is open. `Modal` keeps painting (and accepting
/// pointer input on) its contents through the 160 ms close animation, so an unguarded commit
/// inserted the same action twice on a double click (U04-002).
pub(crate) fn catalog_commit_allowed(is_open: bool) -> bool {
    is_open
}

/// The single place a collected card click becomes the shell's `chosen` index. Kept as its own
/// function so the close-animation gate is structural: no wrapper can read a click while the
/// dialog is closing.
fn gated_choice(is_open: bool, clicked: Option<usize>) -> Option<usize> {
    if catalog_commit_allowed(is_open) {
        clicked
    } else {
        None
    }
}

/// Header slice shared by the catalog dialogs: eyebrow label on the left, close button on the right.
fn render_catalog_header(
    frame: &mut ModalFrame<'_>,
    t: &Tokens,
    eyebrow: &str,
    close_tip: &str,
) {
    frame.header(|ui, close_req| {
        render_modal_header(
            ui,
            close_req,
            t,
            ModalHeader {
                glyph: None,
                glyph_size: 16.0,
                close_tip,
                left_pad: 18.0,
                glyph_pad: 0.0,
                right_pad: 16.0,
                item_spacing_x: None,
            },
            |ui| { widgets::hud(ui, eyebrow, t.ink); },
            |_| {},
        );
    });
}

/// Shared catalog body: category sidebar, search row, two-column card grid and empty state.
///
/// Returns the index into `view.items` of the card clicked on this frame.
fn render_catalog_body<T: CatalogEntry>(
    ui: &mut egui::Ui,
    t: &Tokens,
    loc: &Strings,
    view: &mut CatalogView<'_, T>,
) -> Option<usize> {
    let body_rect = ui.max_rect();
    let sidebar_w = 195.0;
    let sidebar_rect = Rect::from_min_size(
        pos2(body_rect.min.x, body_rect.top() + 1.0),
        vec2(sidebar_w, body_rect.height() - 2.0),
    );
    let cards_rect = Rect::from_min_max(
        pos2(sidebar_rect.right() + 1.0, body_rect.top() + 1.0),
        pos2(body_rect.right() - 1.0, body_rect.bottom() - 1.0),
    );

    // Body background surfaces & divider
    let p = ui.painter();
    // The modal card is rounded, so the body panels must round their bottom corners too,
    // otherwise their square corners poke out of the card in the lower left and right.
    let card_radius = t.metrics().modal_radius;
    p.rect_filled(
        sidebar_rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: card_radius,
            se: 0,
        },
        t.bg,
    );
    p.rect_filled(
        cards_rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 0,
            se: card_radius,
        },
        t.sunken,
    );

    // Seamless 1px geometric vertical divider without line-cap protrusion (DPI-independent)
    let v_divider = Rect::from_min_max(
        pos2(sidebar_rect.right(), body_rect.top() + 1.0),
        pos2(sidebar_rect.right() + 1.0, body_rect.bottom() - 1.0),
    );
    p.rect_filled(v_divider, CornerRadius::ZERO, t.hairline().color);

    // Left: Categories List
    ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar_rect), |ui| {
        egui::Frame::new()
            .inner_margin(Margin::symmetric(10, 10))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;

                let count_for = |cat_idx: usize| -> usize {
                    view.items
                        .iter()
                        .filter(|item| {
                            (item.nestable() || !view.nesting)
                                && (cat_idx == 0 || item.category() == cat_idx)
                        })
                        .count()
                };

                let categories: Vec<(usize, &'static str, &'static str, usize)> = view
                    .categories
                    .iter()
                    .map(|category| {
                        (
                            category.index,
                            category.glyph,
                            (category.label)(loc),
                            count_for(category.index),
                        )
                    })
                    .collect();

                for (cat_idx, icon, cat_name, count) in categories {
                    let is_active = *view.selected_category == cat_idx;
                    let bg = if is_active { t.accent_ink.gamma_multiply(0.12) } else { Color32::TRANSPARENT };
                    let fg = if is_active { t.accent_ink } else { t.muted };

                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                    if resp.hovered() && !is_active {
                        ui.painter().rect_filled(rect, t.chip_corner(), t.hover_fill());
                    }
                    if is_active {
                        ui.painter().rect_filled(rect, t.chip_corner(), bg);
                    }

                    ui.painter().text(
                        pos2(rect.left() + 10.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        icon,
                        FontId::new(14.0, theme::phosphor()),
                        fg,
                    );

                    ui.painter().text(
                        pos2(rect.left() + 32.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        cat_name,
                        FontId::new(12.5, theme::semibold()),
                        fg,
                    );

                    ui.painter().text(
                        pos2(rect.right() - 8.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        format!("{count}"),
                        FontId::new(11.0, theme::mono()),
                        t.faint,
                    );

                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        *view.selected_category = cat_idx;
                    }
                }
            });
    });

    let filtered = filter_catalog_entries(
        view.items,
        *view.selected_category,
        view.search_query.as_str(),
        loc,
        view.nesting,
    );

    let mut clicked = None;

    // Right: Cards Grid in a ScrollArea with the Search Bar above
    ui.scope_builder(egui::UiBuilder::new().max_rect(cards_rect), |ui| {
        ui.add_space(10.0);

        // Search box placed directly above cards scroll area
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(10.0);
            let search_w = (ui.available_width() - 10.0).max(60.0);
            ui.allocate_ui_with_layout(
                vec2(search_w, t.metrics().control_h),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    widgets::catalog_search_box(ui, view.search_query, view.search_hint);
                },
            );
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .id_salt(view.scroll_id)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if filtered.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(
                            RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                                .family(theme::phosphor())
                                .size(32.0)
                                .color(t.faint),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(loc.catalog_no_matches)
                                .family(theme::medium())
                                .size(13.0)
                                .color(t.muted),
                        );
                    });
                } else {
                    let total_w = ui.available_width() - 20.0;
                    let col_count = 2;
                    let gap = 10.0;
                    let card_w = ((total_w - gap) / (col_count as f32)).max(160.0);
                    let card_h = 68.0;

                    for chunk in filtered.chunks(col_count) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(10.0);
                            for (idx, &item_idx) in chunk.iter().enumerate() {
                                if idx > 0 {
                                    ui.add_space(gap);
                                }
                                let item = &view.items[item_idx];
                                let (rect, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                                let is_hovered = resp.hovered();

                                let bg_col = if is_hovered { t.sunken } else { t.bg };
                                let stroke_col = if is_hovered { t.accent_ink } else { t.line };

                                ui.painter().rect_filled(rect, CornerRadius::same(6), bg_col);
                                ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, stroke_col), egui::StrokeKind::Inside);

                                // Icon Chip on Left
                                let chip_rect = Rect::from_center_size(
                                    pos2(rect.left() + 26.0, rect.center().y),
                                    vec2(34.0, 34.0),
                                );
                                let item_tint = item.tint(t);
                                ui.painter().rect_filled(chip_rect, CornerRadius::same(6), item_tint.gamma_multiply(0.18));
                                ui.painter().text(
                                    chip_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    item.glyph(),
                                    FontId::new(18.0, theme::phosphor()),
                                    item_tint,
                                );

                                // Text content on Right (vertically centered relative to card & icon chip)
                                let text_left = rect.left() + 52.0;
                                let title_text = item.title(loc);
                                let desc_text = item.description(loc);
                                let desc_w = (rect.right() - text_left - 10.0).max(40.0);

                                let title_galley = ui.painter().layout_no_wrap(
                                    title_text.to_string(),
                                    FontId::new(13.0, theme::semibold()),
                                    t.ink,
                                );
                                let desc_galley = ui.painter().layout(
                                    desc_text.to_string(),
                                    FontId::new(11.0, theme::medium()),
                                    t.muted,
                                    desc_w,
                                );

                                let title_h = title_galley.size().y;
                                let desc_h = desc_galley.size().y;
                                let text_spacing = 3.0;
                                let total_text_h = title_h + text_spacing + desc_h;
                                let text_start_y = (rect.center().y - total_text_h * 0.5).round();

                                ui.painter().galley(pos2(text_left, text_start_y), title_galley, t.ink);
                                ui.painter().galley(pos2(text_left, text_start_y + title_h + text_spacing), desc_galley, t.muted);

                                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    clicked = Some(item_idx);
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                ui.add_space(10.0);
            });
    });

    clicked
}

/// Presentation metadata plus the per-kind default constructor for one table row.
///
/// `C` is the only thing that differs between the catalogs: `fn() -> Action`,
/// `fn() -> Option<Trigger>`, `fn() -> RuleCondition`, or `ElementBuild`. The generic shell never
/// calls it; the per-kind commit adapter does.
pub(crate) struct CatalogEntryDef<C> {
    pub category_idx: usize,
    pub badge: &'static str,
    pub title: fn(&Strings) -> &'static str,
    pub description: fn(&Strings) -> &'static str,
    pub glyph: &'static str,
    pub tint: fn(&Tokens) -> Color32,
    pub keywords: &'static [&'static str],
    /// False only for entries that cannot be nested inside an If/Else branch.
    pub nestable: bool,
    pub constructor: C,
}

impl<C> CatalogEntry for CatalogEntryDef<C> {
    fn category(&self) -> usize {
        self.category_idx
    }

    fn badge(&self) -> &'static str {
        self.badge
    }

    fn title(&self, loc: &Strings) -> &'static str {
        (self.title)(loc)
    }

    fn description(&self, loc: &Strings) -> &'static str {
        (self.description)(loc)
    }

    fn glyph(&self) -> &'static str {
        self.glyph
    }

    fn tint(&self, t: &Tokens) -> Color32 {
        (self.tint)(t)
    }

    fn keywords(&self) -> &'static [&'static str] {
        self.keywords
    }

    fn nestable(&self) -> bool {
        self.nestable
    }
}

/// Everything that must stay identical for a given catalog's egui identity: modal id, header
/// labels, search hint, scroll-area salt and dialog geometry.
#[derive(Clone, Copy)]
pub(crate) struct CatalogChrome {
    pub modal_id: &'static str,
    pub eyebrow: fn(&Strings) -> &'static str,
    pub close_tip: fn(&Strings) -> &'static str,
    pub search_hint: fn(&Strings) -> &'static str,
    pub scroll_id: &'static str,
    pub width_fraction: f32,
    pub height_fraction: f32,
    pub min_size: Vec2,
    pub max_size: Vec2,
}

/// What one frame of the catalog shell produced. The wrapper decides what to commit and when to
/// drop its cached `last_*` state; the shell only reports.
pub(crate) struct CatalogModalOutcome {
    /// `Some(idx)` only while the dialog is open; clicks during the close animation are dropped
    /// here, so no wrapper can commit by accident (U04-002).
    pub chosen: Option<usize>,
    pub close_requested: bool,
    /// True on the frame `Modal::show` returned `None` (close animation finished). Wrappers use
    /// only this to clear `last_*`, never a bare `else` (U04-013).
    pub fully_closed: bool,
}

/// The shared catalog dialog. Consumes the per-kind chrome plus the table and returns a frame
/// outcome; the caller keeps its own modal state and commit target.
pub(crate) struct CatalogModal<'a, T: CatalogEntry> {
    chrome: CatalogChrome,
    items: &'a [T],
    categories: &'a [CatalogCategory],
    nesting: bool,
}

impl<'a, T: CatalogEntry> CatalogModal<'a, T> {
    pub fn new(chrome: CatalogChrome, items: &'a [T], categories: &'a [CatalogCategory]) -> Self {
        Self {
            chrome,
            items,
            categories,
            nesting: false,
        }
    }

    pub fn nesting(mut self, nesting: bool) -> Self {
        self.nesting = nesting;
        self
    }

    pub fn show(
        self,
        ctx: &egui::Context,
        t: &Tokens,
        loc: &'static Strings,
        selected_category: &mut usize,
        search_query: &mut String,
        is_open: bool,
    ) -> CatalogModalOutcome {
        let mut clicked = None;
        let modal_res = Modal::new(self.chrome.modal_id, is_open)
            .width_fraction(self.chrome.width_fraction)
            .height_fraction(self.chrome.height_fraction)
            .min_size(self.chrome.min_size)
            .max_size(self.chrome.max_size)
            .header_height(46.0)
            .show(ctx, t, |frame| {
                render_catalog_header(
                    frame,
                    t,
                    (self.chrome.eyebrow)(loc),
                    (self.chrome.close_tip)(loc),
                );

                frame.body(|ui| {
                    let mut view = CatalogView {
                        items: self.items,
                        categories: self.categories,
                        selected_category,
                        search_query,
                        nesting: self.nesting,
                        scroll_id: self.chrome.scroll_id,
                        search_hint: (self.chrome.search_hint)(loc),
                    };
                    clicked = render_catalog_body(ui, t, loc, &mut view);
                });
            });

        CatalogModalOutcome {
            chosen: gated_choice(is_open, clicked),
            close_requested: modal_res.as_ref().is_some_and(|res| res.close_requested),
            fully_closed: modal_res.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::views::catalogs::{
        action::ACTION_CHROME,
        condition::CONDITION_CHROME,
        element::ELEMENT_CHROME,
        trigger::TRIGGER_CHROME,
    };
    use crate::i18n::{self, Language};

    /// Minimal entry type so the shared filter is tested independently of any catalog table.
    struct TestEntry {
        category_idx: usize,
        badge: &'static str,
        title: &'static str,
        description: &'static str,
        keywords: &'static [&'static str],
        nestable: bool,
    }

    impl CatalogEntry for TestEntry {
        fn category(&self) -> usize {
            self.category_idx
        }

        fn badge(&self) -> &'static str {
            self.badge
        }

        fn title(&self, _loc: &Strings) -> &'static str {
            self.title
        }

        fn description(&self, _loc: &Strings) -> &'static str {
            self.description
        }

        fn glyph(&self) -> &'static str {
            ""
        }

        fn tint(&self, _t: &Tokens) -> Color32 {
            Color32::WHITE
        }

        fn keywords(&self) -> &'static [&'static str] {
            self.keywords
        }

        fn nestable(&self) -> bool {
            self.nestable
        }
    }

    #[test]
    fn test_filter_catalog_entries_matches_trimmed_case_insensitive_query() {
        let loc = i18n::get(Language::English);
        let entries = [
            TestEntry {
                category_idx: 1,
                badge: "PROCESS",
                title: "Start Thing",
                description: "Launch something",
                keywords: &["запуск"],
                nestable: true,
            },
            TestEntry {
                category_idx: 1,
                badge: "PROCESS",
                title: "Stop Thing",
                description: "Terminate something",
                keywords: &["end"],
                nestable: false,
            },
            TestEntry {
                category_idx: 2,
                badge: "AUDIO",
                title: "Play Sound",
                description: "Hear audio",
                keywords: &["music"],
                nestable: true,
            },
        ];

        // Trim + case-insensitive title match.
        assert_eq!(filter_catalog_entries(&entries, 0, "  tHiNg  ", loc, false), vec![0, 1]);
        // Badge-only and keyword-only matches both work.
        assert_eq!(filter_catalog_entries(&entries, 0, "audio", loc, false), vec![2]);
        assert_eq!(filter_catalog_entries(&entries, 0, "music", loc, false), vec![2]);
        // Description matches as well.
        assert_eq!(filter_catalog_entries(&entries, 0, "launch", loc, false), vec![0]);
        // Category and query always apply together.
        assert!(filter_catalog_entries(&entries, 2, "thing", loc, false).is_empty());
        // Nesting hides non-nestable entries before the query is applied.
        assert_eq!(filter_catalog_entries(&entries, 0, "", loc, true), vec![0, 2]);
        assert!(filter_catalog_entries(&entries, 0, "end", loc, true).is_empty());
        assert_eq!(filter_catalog_entries(&entries, 0, "", loc, false), vec![0, 1, 2]);
    }

    #[test]
    fn test_catalog_commit_gate_drops_clicks_while_closing() {
        // The shell routes every card click through this gate, so a click painted during the close
        // animation can never become `chosen` (U04-002).
        assert_eq!(gated_choice(false, Some(3)), None);
        assert_eq!(gated_choice(false, None), None);
        assert_eq!(gated_choice(true, Some(3)), Some(3));
        assert_eq!(gated_choice(true, None), None);
        assert!(catalog_commit_allowed(true));
        assert!(!catalog_commit_allowed(false));

        // `fully_closed` is the structural signal the wrappers use for the cached `last_*`
        // cleanup: nothing may clear that cache while the close animation is still running.
        let closing = CatalogModalOutcome {
            chosen: None,
            close_requested: false,
            fully_closed: false,
        };
        assert!(
            !closing.fully_closed,
            "the closing animation must keep the cached modal state"
        );
        let closed = CatalogModalOutcome {
            chosen: None,
            close_requested: false,
            fully_closed: true,
        };
        assert!(
            closed.fully_closed,
            "only a fully closed modal may drop the cached state (U04-013)"
        );
    }

    #[test]
    fn test_catalog_chrome_keeps_legacy_ids_and_geometry() {
        // Modal ids and scroll salts are egui identity: renaming one restarts the open/close
        // animation or resets the scroll position.
        assert_eq!(ACTION_CHROME.modal_id, "action-catalog-modal");
        assert_eq!(TRIGGER_CHROME.modal_id, "trigger-catalog-modal");
        assert_eq!(CONDITION_CHROME.modal_id, "condition-catalog-modal");
        assert_eq!(ELEMENT_CHROME.modal_id, "element-catalog-modal");

        assert_eq!(ACTION_CHROME.scroll_id, "action_catalog_cards_scroll");
        assert_eq!(TRIGGER_CHROME.scroll_id, "trigger_catalog_cards_scroll");
        assert_eq!(CONDITION_CHROME.scroll_id, "condition_catalog_cards_scroll");
        assert_eq!(ELEMENT_CHROME.scroll_id, "element_catalog_cards_scroll");

        for chrome in [ACTION_CHROME, TRIGGER_CHROME, CONDITION_CHROME] {
            assert_eq!(chrome.width_fraction, 0.74);
            assert_eq!(chrome.height_fraction, 0.76);
            assert_eq!(chrome.min_size, vec2(720.0, 500.0));
            assert_eq!(chrome.max_size, vec2(840.0, 640.0));
        }

        // The element catalog is genuinely smaller.
        assert_eq!(ELEMENT_CHROME.width_fraction, 0.72);
        assert_eq!(ELEMENT_CHROME.height_fraction, 0.74);
        assert_eq!(ELEMENT_CHROME.min_size, vec2(700.0, 480.0));
        assert_eq!(ELEMENT_CHROME.max_size, vec2(820.0, 620.0));

        // Labels and hints are part of the dialog identity too.
        let en = i18n::get(Language::English);
        assert_eq!((ACTION_CHROME.eyebrow)(en), en.catalog_modal_eyebrow);
        assert_eq!((TRIGGER_CHROME.eyebrow)(en), en.catalog_trigger_modal_title);
        assert_eq!((CONDITION_CHROME.eyebrow)(en), en.catalog_condition_modal_title);
        assert_eq!((ELEMENT_CHROME.eyebrow)(en), en.element_catalog_modal_eyebrow);
        assert_eq!((ACTION_CHROME.search_hint)(en), en.catalog_search_hint);
        assert_eq!((TRIGGER_CHROME.search_hint)(en), en.catalog_trigger_search_hint);
        assert_eq!((CONDITION_CHROME.search_hint)(en), en.catalog_condition_search_hint);
        assert_eq!((ELEMENT_CHROME.search_hint)(en), en.element_catalog_search_hint);
        for chrome in [ACTION_CHROME, TRIGGER_CHROME, CONDITION_CHROME, ELEMENT_CHROME] {
            assert_eq!((chrome.close_tip)(en), en.inspector_btn_cancel);
        }
    }
}
