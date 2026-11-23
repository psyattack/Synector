//! Unit-aware duration editor.
//!
//! The models store durations in seconds or milliseconds, but a fixed unit
//! makes long timers hard to scan and edit ("86400 s" instead of "1 d").
//! [`DurationField`] renders the value in the most readable unit and offers a
//! compact dropdown to switch it. The picked unit is remembered per widget in
//! egui's temporary data; the stored value is always written back in the
//! field's own base unit.

use std::fmt::Debug;
use std::hash::Hash;
use std::ops::RangeInclusive;

use egui::{ComboBox, DragValue, Id, Response, Ui};

use crate::i18n::Strings;

/// A time unit offered by [`DurationField`], ordered from smallest to largest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Milliseconds,
    Seconds,
    Minutes,
    Hours,
    Days,
}

impl TimeUnit {
    /// All units, smallest first.
    pub const ALL: [Self; 5] = [
        Self::Milliseconds,
        Self::Seconds,
        Self::Minutes,
        Self::Hours,
        Self::Days,
    ];

    /// How many seconds one unit holds.
    pub fn factor(self) -> f64 {
        match self {
            Self::Milliseconds => 0.001,
            Self::Seconds => 1.0,
            Self::Minutes => 60.0,
            Self::Hours => 3_600.0,
            Self::Days => 86_400.0,
        }
    }

    fn index(self) -> u8 {
        match self {
            Self::Milliseconds => 0,
            Self::Seconds => 1,
            Self::Minutes => 2,
            Self::Hours => 3,
            Self::Days => 4,
        }
    }

    fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }

    fn label(self, loc: &Strings) -> &'static str {
        match self {
            Self::Milliseconds => loc.time_unit_ms,
            Self::Seconds => loc.time_unit_seconds,
            Self::Minutes => loc.time_unit_minutes,
            Self::Hours => loc.time_unit_hours,
            Self::Days => loc.time_unit_days,
        }
    }

    /// Most readable unit for `seconds` before the user overrides it.
    fn auto(seconds: f64) -> Self {
        let seconds = seconds.abs();
        if seconds == 0.0 {
            Self::Seconds
        } else if seconds < 1.0 {
            Self::Milliseconds
        } else if seconds < 60.0 {
            Self::Seconds
        } else if seconds < 3_600.0 {
            Self::Minutes
        } else if seconds < 86_400.0 {
            Self::Hours
        } else {
            Self::Days
        }
    }

    fn max_decimals(self) -> usize {
        match self {
            Self::Milliseconds => 0,
            Self::Seconds | Self::Minutes | Self::Hours | Self::Days => 3,
        }
    }
}

/// Whether `unit` can express the upper bound of `range_secs` with at least
/// one whole unit: a 60-second range cannot be meaningfully edited in days.
fn unit_allowed(unit: TimeUnit, range_secs: &RangeInclusive<f64>) -> bool {
    let max = *range_secs.end();
    max.is_finite() && max > 0.0 && max / unit.factor() >= 1.0
}

/// Unit shown when the user has not picked one: the readable unit when the
/// range supports it, otherwise the largest unit the range can express.
fn unit_for_value(seconds: f64, range_secs: &RangeInclusive<f64>) -> TimeUnit {
    let auto = TimeUnit::auto(seconds);
    if unit_allowed(auto, range_secs) {
        return auto;
    }
    TimeUnit::ALL
        .iter()
        .rev()
        .copied()
        .find(|unit| unit_allowed(*unit, range_secs))
        .unwrap_or(TimeUnit::Milliseconds)
}

/// Seconds per dragged point for the displayed unit.
///
/// The step grows with the value so long timers can be scrubbed quickly while
/// short ones stay precise; `granularity_secs` keeps the field's own storage
/// step (a whole second or millisecond) reachable.
fn drag_speed(display_value: f64, unit: TimeUnit, granularity_secs: f64) -> f64 {
    let unit_floor_secs: f64 = match unit {
        TimeUnit::Milliseconds => 0.001,
        TimeUnit::Seconds => 0.1,
        TimeUnit::Minutes => 0.06,
        TimeUnit::Hours => 0.36,
        TimeUnit::Days => 8.64,
    };
    let floor_secs = unit_floor_secs.max(granularity_secs);
    let seconds_per_point = (display_value.abs() * unit.factor() / 100.0).max(floor_secs);
    seconds_per_point / unit.factor()
}

/// Round to the storage granularity and clamp to `range_secs`.
fn snap_seconds(value_secs: f64, granularity_secs: f64, range_secs: &RangeInclusive<f64>) -> f64 {
    let snapped = if granularity_secs > 0.0 {
        (value_secs / granularity_secs).round() * granularity_secs
    } else {
        value_secs
    };
    snapped.clamp(*range_secs.start(), *range_secs.end())
}

/// Inline duration editor: a numeric value plus a compact unit dropdown.
///
/// The model keeps its own base unit (seconds or milliseconds); only the
/// presentation changes. A unit picked by the user is remembered in egui's
/// temporary data for as long as the widget id stays the same.
pub struct DurationField<'a> {
    loc: &'a Strings,
    id: Id,
    range_secs: RangeInclusive<f64>,
    prefix: Option<String>,
    default_unit: Option<TimeUnit>,
}

impl<'a> DurationField<'a> {
    /// A field whose `range_secs` is expressed in seconds.
    pub fn secs(
        ui: &Ui,
        loc: &'a Strings,
        id_salt: impl Hash + Debug,
        range_secs: RangeInclusive<f64>,
    ) -> Self {
        Self {
            loc,
            id: ui.scope_id().with(("duration_field", id_salt)),
            range_secs,
            prefix: None,
            default_unit: None,
        }
    }

    /// A field whose `range_millis` is expressed in milliseconds.
    pub fn millis(
        ui: &Ui,
        loc: &'a Strings,
        id_salt: impl Hash + Debug,
        range_millis: RangeInclusive<f64>,
    ) -> Self {
        Self::secs(
            ui,
            loc,
            id_salt,
            (*range_millis.start() / 1000.0)..=(*range_millis.end() / 1000.0),
        )
    }

    /// Text shown inside the numeric field before the number.
    #[must_use]
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Unit used until the user picks another one explicitly.
    #[must_use]
    pub fn default_unit(mut self, unit: TimeUnit) -> Self {
        self.default_unit = Some(unit);
        self
    }

    /// Show the field and write the value back as seconds.
    pub fn show_f64(self, ui: &mut Ui, seconds: &mut f64) -> Response {
        self.show(ui, seconds, 0.0)
    }

    /// Show the field and write the value back as `f32` seconds.
    pub fn show_f32(self, ui: &mut Ui, seconds: &mut f32) -> Response {
        let mut value = f64::from(*seconds);
        let response = self.show(ui, &mut value, 0.0);
        if response.changed() {
            *seconds = value as f32;
        }
        response
    }

    /// Show the field and write the value back as whole seconds.
    pub fn show_secs_u32(self, ui: &mut Ui, seconds: &mut u32) -> Response {
        let mut value = f64::from(*seconds);
        let response = self.show(ui, &mut value, 1.0);
        if response.changed() {
            *seconds = value as u32;
        }
        response
    }

    /// Show the field and write the value back as whole seconds.
    pub fn show_secs_u64(self, ui: &mut Ui, seconds: &mut u64) -> Response {
        let mut value = *seconds as f64;
        let response = self.show(ui, &mut value, 1.0);
        if response.changed() {
            *seconds = value as u64;
        }
        response
    }

    /// Show the field and write the value back as whole milliseconds.
    pub fn show_millis_u32(self, ui: &mut Ui, millis: &mut u32) -> Response {
        let mut seconds = f64::from(*millis) / 1000.0;
        let response = self.show(ui, &mut seconds, 0.001);
        if response.changed() {
            *millis = (seconds * 1000.0).round() as u32;
        }
        response
    }

    /// Show the field and write the value back as whole milliseconds.
    pub fn show_millis_u64(self, ui: &mut Ui, millis: &mut u64) -> Response {
        let mut seconds = *millis as f64 / 1000.0;
        let response = self.show(ui, &mut seconds, 0.001);
        if response.changed() {
            *millis = (seconds * 1000.0).round() as u64;
        }
        response
    }

    fn show(self, ui: &mut Ui, seconds: &mut f64, granularity_secs: f64) -> Response {
        let range = self.range_secs;
        let loc = self.loc;
        let id = self.id;
        let prefix = self.prefix;

        let stored = ui
            .data(|data| data.get_temp::<u8>(id))
            .and_then(TimeUnit::from_index)
            .filter(|unit| unit_allowed(*unit, &range));
        let fallback = self
            .default_unit
            .filter(|unit| unit_allowed(*unit, &range))
            .unwrap_or_else(|| unit_for_value(*seconds, &range));
        let mut unit = stored.unwrap_or(fallback);
        let edit_unit = unit;

        let mut display = *seconds / edit_unit.factor();
        let min = *range.start() / edit_unit.factor();
        let max = *range.end() / edit_unit.factor();

        let mut unit_changed = false;
        let result = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            // Both controls are framed and their natural height is computed from the
            // button font and padding. Pinning `interact_size.y` to that height makes the
            // unit dropdown line up with the numeric box instead of sitting a pixel lower.
            ui.spacing_mut().interact_size.y = crate::gui::theme::framed_control_height(ui);
            let speed = drag_speed(display, edit_unit, granularity_secs);
            let mut drag = DragValue::new(&mut display)
                .range(min..=max)
                .speed(speed)
                .max_decimals(edit_unit.max_decimals());
            if let Some(prefix) = prefix.as_deref() {
                drag = drag.prefix(prefix);
            }
            // The numeric box shares its row with the unit dropdown, so trim its
            // horizontal padding a little to keep the pair compact.
            let button_padding_x = ui.spacing().button_padding.x;
            ui.spacing_mut().button_padding.x = (button_padding_x * 2.0 / 3.0).max(6.0);
            let drag_response = ui.add(drag);
            ui.spacing_mut().button_padding.x = button_padding_x;

            let selected_before = unit;
            let combo = ComboBox::from_id_salt(id.with("unit"))
                .selected_text(unit.label(loc))
                .width(56.0)
                .height(200.0)
                .show_ui(ui, |ui| {
                    for option in TimeUnit::ALL {
                        if unit_allowed(option, &range) {
                            ui.selectable_value(&mut unit, option, option.label(loc));
                        }
                    }
                });
            unit_changed = unit != selected_before;

            let _combo_rect = combo.response.rect;
            #[cfg(test)]
            ui.data_mut(|data| {
                data.insert_temp(Id::unique("duration_test_combo_rect"), _combo_rect);
            });

            if drag_response.changed() {
                *seconds = snap_seconds(display * edit_unit.factor(), granularity_secs, &range);
            }

            drag_response
        });

        // Remember the unit once the user touches the value or picks a unit,
        // so the automatic scaling stops overriding their choice.
        if unit_changed || result.inner.changed() {
            ui.data_mut(|data| data.insert_temp(id, unit.index()));
        }

        result.inner
    }
}

/// Show a seconds value in a unit-aware field.
pub fn duration_secs_f64(
    ui: &mut Ui,
    loc: &Strings,
    id_salt: impl Hash + Debug,
    seconds: &mut f64,
    range_secs: RangeInclusive<f64>,
) -> Response {
    DurationField::secs(ui, loc, id_salt, range_secs).show_f64(ui, seconds)
}

/// Show an `f32` seconds value in a unit-aware field.
pub fn duration_secs_f32(
    ui: &mut Ui,
    loc: &Strings,
    id_salt: impl Hash + Debug,
    seconds: &mut f32,
    range_secs: RangeInclusive<f64>,
) -> Response {
    DurationField::secs(ui, loc, id_salt, range_secs).show_f32(ui, seconds)
}

/// Show a whole-seconds `u32` value in a unit-aware field.
pub fn duration_secs_u32(
    ui: &mut Ui,
    loc: &Strings,
    id_salt: impl Hash + Debug,
    seconds: &mut u32,
    range_secs: RangeInclusive<f64>,
) -> Response {
    DurationField::secs(ui, loc, id_salt, range_secs).show_secs_u32(ui, seconds)
}

/// Show a whole-seconds `u64` value in a unit-aware field.
pub fn duration_secs_u64(
    ui: &mut Ui,
    loc: &Strings,
    id_salt: impl Hash + Debug,
    seconds: &mut u64,
    range_secs: RangeInclusive<f64>,
) -> Response {
    DurationField::secs(ui, loc, id_salt, range_secs).show_secs_u64(ui, seconds)
}

/// Show a whole-milliseconds `u32` value in a unit-aware field.
pub fn duration_millis_u32(
    ui: &mut Ui,
    loc: &Strings,
    id_salt: impl Hash + Debug,
    millis: &mut u32,
    range_millis: RangeInclusive<f64>,
) -> Response {
    DurationField::millis(ui, loc, id_salt, range_millis).show_millis_u32(ui, millis)
}

/// Show a whole-milliseconds `u64` value in a unit-aware field.
pub fn duration_millis_u64(
    ui: &mut Ui,
    loc: &Strings,
    id_salt: impl Hash + Debug,
    millis: &mut u64,
    range_millis: RangeInclusive<f64>,
) -> Response {
    DurationField::millis(ui, loc, id_salt, range_millis).show_millis_u64(ui, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_unit_keeps_values_readable() {
        assert_eq!(TimeUnit::auto(0.0), TimeUnit::Seconds);
        assert_eq!(TimeUnit::auto(0.25), TimeUnit::Milliseconds);
        assert_eq!(TimeUnit::auto(1.0), TimeUnit::Seconds);
        assert_eq!(TimeUnit::auto(59.0), TimeUnit::Seconds);
        assert_eq!(TimeUnit::auto(60.0), TimeUnit::Minutes);
        assert_eq!(TimeUnit::auto(3_599.0), TimeUnit::Minutes);
        assert_eq!(TimeUnit::auto(3_600.0), TimeUnit::Hours);
        assert_eq!(TimeUnit::auto(86_399.0), TimeUnit::Hours);
        assert_eq!(TimeUnit::auto(86_400.0), TimeUnit::Days);
    }

    #[test]
    fn display_conversion_round_trips() {
        for seconds in [0.001, 0.05, 1.0, 1.5, 59.9, 60.0, 3_599.0, 3_600.0, 86_400.0] {
            for unit in TimeUnit::ALL {
                let display = seconds / unit.factor();
                let restored = display * unit.factor();
                assert!(
                    (restored - seconds).abs() < 1e-9,
                    "{unit:?}: {seconds} -> {display} -> {restored}"
                );
            }
        }
    }

    #[test]
    fn units_outside_the_range_are_hidden() {
        assert!(unit_allowed(TimeUnit::Milliseconds, &(0.0..=60.0)));
        assert!(unit_allowed(TimeUnit::Seconds, &(0.0..=60.0)));
        assert!(unit_allowed(TimeUnit::Minutes, &(0.0..=60.0)));
        assert!(!unit_allowed(TimeUnit::Hours, &(0.0..=60.0)));
        assert!(!unit_allowed(TimeUnit::Days, &(0.0..=60.0)));

        assert!(!unit_allowed(TimeUnit::Seconds, &(0.05..=0.5)));
        assert!(unit_allowed(TimeUnit::Milliseconds, &(0.05..=0.5)));
    }

    #[test]
    fn default_unit_falls_back_to_what_the_range_supports() {
        assert_eq!(unit_for_value(0.0, &(0.0..=60.0)), TimeUnit::Seconds);
        assert_eq!(unit_for_value(0.0, &(0.05..=0.3)), TimeUnit::Milliseconds);
        assert_eq!(unit_for_value(300.0, &(0.0..=600.0)), TimeUnit::Minutes);
    }

    #[test]
    fn snapping_respects_granularity_and_range() {
        assert_eq!(snap_seconds(3.4, 1.0, &(1.0..=10.0)), 3.0);
        assert_eq!(snap_seconds(3.6, 1.0, &(1.0..=10.0)), 4.0);
        assert_eq!(snap_seconds(0.0004, 0.001, &(0.0..=1.0)), 0.0);
        assert_eq!(snap_seconds(12.0, 1.0, &(1.0..=10.0)), 10.0);
        assert_eq!(snap_seconds(-1.0, 1.0, &(0.0..=10.0)), 0.0);
    }

    #[test]
    fn drag_speed_stays_precise_for_small_values() {
        let fine = drag_speed(0.05, TimeUnit::Seconds, 0.0);
        assert!(fine < 0.2, "fine = {fine}");

        let coarse_seconds = drag_speed(1.0, TimeUnit::Hours, 1.0) * TimeUnit::Hours.factor();
        assert!(coarse_seconds >= 1.0, "coarse = {coarse_seconds} s/point");

        // Whole-second storage never moves by less than one second per point.
        let stepped_seconds =
            drag_speed(0.0, TimeUnit::Hours, 1.0) * TimeUnit::Hours.factor();
        assert!((stepped_seconds - 1.0).abs() < 1e-9);
    }

    #[test]
    fn unit_dropdown_aligns_with_numeric_field() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(&ctx, crate::config::AppTheme::Dark, crate::config::UiSkin::Classic, None);
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let mut seconds = 90.0_f64;
        let mut drag_rect = None;

        let mut output = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Mimic the compact cooldown row that exposed the offset.
                ui.spacing_mut().interact_size.y = 24.0;
                drag_rect = Some(
                    duration_secs_f64(ui, loc, "align_test", &mut seconds, 0.0..=3_600.0).rect,
                );
            });
        });
        output.textures_delta.clear();

        let drag_rect = drag_rect.expect("numeric field rect");
        let combo_rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(Id::unique("duration_test_combo_rect")))
            .expect("unit dropdown rect");

        assert!((drag_rect.top() - combo_rect.top()).abs() < 0.01);
        assert!((drag_rect.height() - combo_rect.height()).abs() < 0.01);
    }

    #[test]
    fn renders_without_side_effects() {
        let ctx = egui::Context::default();
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let mut seconds = 90.0_f64;
        let mut millis = 2_500_u32;

        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    duration_secs_f64(ui, loc, "smoke_seconds", &mut seconds, 0.0..=3_600.0);
                    duration_millis_u32(ui, loc, "smoke_millis", &mut millis, 0.0..=60_000.0);
                });
            });
            output.textures_delta.clear();
        }

        assert_eq!(seconds, 90.0);
        assert_eq!(millis, 2_500);
    }
}
