//! Hand-painted chart primitives for the analytics dashboard.
//!
//! The GUI does not take a plotting dependency: these are a few `Painter` calls on the design
//! tokens, which keeps the charts inside the same visual language as the rest of the window and
//! avoids a crate whose styling would have to be fought back into place. Charts read colours from
//! the caller (`Tokens::series`, status roles), never from literals.

use std::f32::consts::{FRAC_PI_2, TAU};

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2};

use crate::gui::theme::{self, Tokens};

/// One line of a [`line_chart`].
pub struct ChartSeries<'a> {
    pub label: &'a str,
    pub values: &'a [f32],
    pub color: Color32,
    /// Fill the area under the line with a translucent tint of `color`.
    pub area: bool,
}

/// A line/area chart on a fixed `0..=y_max` scale with a hairline grid and the left axis labels.
/// Values are clamped; the last sample is marked with a dot. Returns the response for tooltips.
pub fn line_chart(
    ui: &mut Ui,
    size: Vec2,
    series: &[ChartSeries<'_>],
    y_max: f32,
    unit: &str,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let plot = Rect::from_min_max(
        pos2(rect.left() + 30.0, rect.top() + 2.0),
        pos2(rect.right() - 8.0, rect.bottom() - 10.0),
    );
    let painter = ui.painter();
    if plot.width() <= 2.0 || plot.height() <= 2.0 {
        return response;
    }
    let y_max = y_max.max(0.001);

    // Grid and axis labels.
    for step in 0..=2 {
        let frac = step as f32 / 2.0;
        let y = plot.bottom() - plot.height() * frac;
        painter.hline(plot.x_range(), y, t.hairline());
        painter.text(
            pos2(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{:.0}", frac * y_max),
            egui::FontId::new(9.0, theme::mono()),
            t.faint,
        );
    }

    let pointer = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pos| plot.expand2(Vec2::new(12.0, 6.0)).contains(*pos));
    let sample_count = series.iter().map(|line| line.values.len()).max().unwrap_or(0);
    let hovered_index = pointer.and_then(|pos| {
        (sample_count > 0).then(|| {
            let frac = ((pos.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            (frac * (sample_count.saturating_sub(1)) as f32).round() as usize
        })
    });

    if let Some(index) = hovered_index {
        let x = sample_x(plot, index, sample_count);
        painter.vline(x, plot.y_range(), t.hairline());
    }

    for line in series {
        if line.values.len() < 2 {
            continue;
        }
        let points: Vec<Pos2> = line
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| sample_point(plot, index, line.values.len(), *value, y_max))
            .collect();
        if line.area {
            paint_area(painter, &points, plot.bottom(), line.color);
        }
        painter.add(egui::Shape::line(
            points.clone(),
            Stroke::new(1.5, line.color),
        ));
        if let Some(last) = points.last() {
            painter.circle_filled(*last, 2.5, line.color);
        }
        if let Some(index) = hovered_index {
            if let Some(value) = line.values.get(index) {
                let pos = sample_point(plot, index, line.values.len(), *value, y_max);
                painter.circle_filled(pos, 3.0, line.color);
            }
        }
    }

    let summary = hovered_index.map(|index| {
        let mut lines = Vec::new();
        for line in series {
            if let Some(value) = line.values.get(index) {
                lines.push(format!("{}  {:.0}{}", line.label, value, unit));
            }
        }
        lines.join("\n")
    });
    match summary {
        Some(text) if !text.is_empty() => response.on_hover_text(text),
        _ => response,
    }
}

/// A small filled sparkline without axes, for KPI cards.
pub fn sparkline(ui: &mut Ui, size: Vec2, values: &[f32], color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if values.len() < 2 {
        return response;
    }
    let painter = ui.painter();
    let plot = rect.shrink2(Vec2::new(1.0, 2.0));
    let max = values.iter().copied().fold(0.0_f32, f32::max).max(1.0);
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, value)| sample_point(plot, index, values.len(), *value, max))
        .collect();
    paint_area(painter, &points, plot.bottom(), color);
    painter.add(egui::Shape::line(points, Stroke::new(1.2, color)));
    response
}

/// One slice of a [`donut`].
pub struct DonutSegment<'a> {
    pub value: f32,
    pub color: Color32,
    pub label: &'a str,
}

/// A donut chart with an optional centre caption. Slices keep a small gap; the hovered slice
/// reports its label, value and share. A zero total paints an empty ring.
pub fn donut(
    ui: &mut Ui,
    diameter: f32,
    segments: &[DonutSegment<'_>],
    center_value: &str,
    center_caption: &str,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    if diameter < 8.0 {
        // Too small for a ring: the outer radius would go negative.
        return response;
    }
    let center = rect.center();
    let outer = diameter * 0.5 - 3.0;
    let ring = (diameter * 0.16).clamp(9.0, 18.0);
    let inner = (outer - ring).max(1.0);
    let painter = ui.painter();

    let total: f32 = segments.iter().map(|segment| segment.value.max(0.0)).sum();
    if total <= 0.0 {
        painter.circle_stroke(center, (outer + inner) * 0.5, Stroke::new(ring, t.line));
        painter.text(
            center,
            Align2::CENTER_CENTER,
            "—",
            egui::FontId::new(18.0, theme::mono_bold()),
            t.faint,
        );
        return response;
    }

    let sweeps = segment_sweeps(
        &segments
            .iter()
            .map(|segment| segment.value.max(0.0))
            .collect::<Vec<_>>(),
    );
    let gap = if segments.len() > 1 { 0.02 } else { 0.0 };
    let mut hovered: Option<usize> = None;
    if let Some(pointer) = ui.ctx().pointer_hover_pos() {
        let delta = pointer - center;
        let distance = delta.length();
        if distance >= inner - 2.0 && distance <= outer + 2.0 {
            let angle = normalize_angle(delta.y.atan2(delta.x));
            hovered = sweeps.iter().position(|(start, sweep)| {
                let start = normalize_angle(*start);
                let end = normalize_angle(start + *sweep);
                if start <= end {
                    angle >= start && angle < end
                } else {
                    angle >= start || angle < end
                }
            });
        }
    }

    for (index, segment) in segments.iter().enumerate() {
        let Some((start, sweep)) = sweeps.get(index).copied() else {
            continue;
        };
        if sweep <= 0.0 {
            continue;
        }
        let start = start + gap * 0.5;
        let sweep = (sweep - gap).max(0.0);
        paint_ring_segment(painter, center, inner, outer, start, sweep, segment.color);
    }

    painter.text(
        center - Vec2::new(0.0, if center_caption.is_empty() { 0.0 } else { 6.0 }),
        Align2::CENTER_CENTER,
        center_value,
        egui::FontId::new(20.0, theme::mono_bold()),
        t.ink_strong,
    );
    if !center_caption.is_empty() {
        painter.text(
            center + Vec2::new(0.0, 12.0),
            Align2::CENTER_CENTER,
            center_caption.to_uppercase(),
            egui::FontId::new(9.0, theme::mono()),
            t.muted,
        );
    }

    if let Some(index) = hovered {
        if let Some((start, sweep)) = sweeps.get(index).copied() {
            let mid = start + sweep * 0.5;
            let radius = (outer + inner) * 0.5;
            let tip = center + Vec2::angled(mid) * radius;
            painter.circle_stroke(tip, ring * 0.45, Stroke::new(1.0, t.ink_strong));
            let value = segments[index].value.max(0.0);
            let share = value / total * 100.0;
            return response.on_hover_text(format!(
                "{}  {:.0} ({:.0}%)",
                segments[index].label, value, share
            ));
        }
    }
    response
}

/// One column of a [`bar_chart`].
pub struct BarDatum<'a> {
    pub value: u32,
    pub errors: u32,
    pub label: &'a str,
}

/// Corner radius of a bar mark: a property of the mark itself, not chrome.
const BAR_CORNER: u8 = 2;

/// A column chart with an error overlay painted on top of each bar. Labels are thinned out so a
/// 24-hour axis never turns into a wall of text.
pub fn bar_chart(
    ui: &mut Ui,
    size: Vec2,
    bars: &[BarDatum<'_>],
    color: Color32,
    error_color: Color32,
    format_tooltip: impl Fn(&BarDatum<'_>) -> String,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if bars.is_empty() {
        return response;
    }
    let plot = Rect::from_min_max(
        pos2(rect.left() + 4.0, rect.top() + 2.0),
        pos2(rect.right() - 4.0, rect.bottom() - 16.0),
    );
    if plot.height() <= 2.0 || plot.width() <= 2.0 {
        return response;
    }
    let painter = ui.painter();
    let max = bars
        .iter()
        .map(|bar| bar.value.max(bar.errors))
        .max()
        .unwrap_or(0)
        .max(1);
    painter.hline(plot.x_range(), plot.bottom(), t.hairline());

    let slot = plot.width() / bars.len() as f32;
    let bar_w = (slot * 0.62).clamp(2.0, 26.0);
    let pointer = ui.ctx().pointer_hover_pos().filter(|pos| plot.contains(*pos));
    let hovered_index = pointer.map(|pos| {
        (((pos.x - plot.left()) / slot).floor() as usize).min(bars.len() - 1)
    });

    let label_stride = (bars.len() as f32 / 6.0).ceil().max(1.0) as usize;
    for (index, bar) in bars.iter().enumerate() {
        let x = plot.left() + slot * (index as f32 + 0.5);
        let height = plot.height() * (bar.value as f32 / max as f32);
        if bar.value > 0 {
            let top = plot.bottom() - height.max(2.0);
            painter.rect_filled(
                Rect::from_min_max(pos2(x - bar_w * 0.5, top), pos2(x + bar_w * 0.5, plot.bottom())),
                CornerRadius::same(BAR_CORNER),
                color,
            );
        }
        if bar.errors > 0 {
            let error_h = (plot.height() * (bar.errors as f32 / max as f32)).max(2.0);
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(x - bar_w * 0.5, plot.bottom() - error_h),
                    pos2(x + bar_w * 0.5, plot.bottom()),
                ),
                CornerRadius::same(BAR_CORNER),
                error_color,
            );
        }
        if index % label_stride == 0 || index == bars.len() - 1 {
            painter.text(
                pos2(x, plot.bottom() + 4.0),
                Align2::CENTER_TOP,
                bar.label,
                egui::FontId::new(8.5, theme::mono()),
                t.faint,
            );
        }
    }

    match hovered_index.and_then(|index| bars.get(index)) {
        Some(bar) if !bar.label.is_empty() => response.on_hover_text(format_tooltip(bar)),
        _ => response,
    }
}

fn sample_point(plot: Rect, index: usize, count: usize, value: f32, y_max: f32) -> Pos2 {
    let x = sample_x(plot, index, count);
    let frac = (value / y_max).clamp(0.0, 1.0);
    pos2(x, plot.bottom() - plot.height() * frac)
}

fn sample_x(plot: Rect, index: usize, count: usize) -> f32 {
    if count <= 1 {
        return plot.center().x;
    }
    plot.left() + plot.width() * index as f32 / (count - 1) as f32
}

/// Fills the area under a polyline with a vertical gradient from the series colour to
/// transparent. A wiggly curve is not convex, so this builds a triangle strip mesh instead of a
/// path fill.
fn paint_area(painter: &egui::Painter, points: &[Pos2], bottom: f32, color: Color32) {
    if points.len() < 2 {
        return;
    }
    let top = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 46);
    let low = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let mut mesh = egui::Mesh::default();
    for window in points.windows(2) {
        let (left, right) = (window[0], window[1]);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(left, top);
        mesh.colored_vertex(right, top);
        mesh.colored_vertex(pos2(left.x, bottom), low);
        mesh.colored_vertex(pos2(right.x, bottom), low);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Start angle and sweep of every slice, in clockwise screen coordinates starting at twelve
/// o'clock. Zero values get a zero sweep.
fn segment_sweeps(values: &[f32]) -> Vec<(f32, f32)> {
    let total: f32 = values.iter().copied().filter(|value| *value > 0.0).sum();
    if total <= 0.0 {
        return vec![(0.0, 0.0); values.len()];
    }
    let mut angle = -FRAC_PI_2;
    values
        .iter()
        .map(|value| {
            let sweep = (value.max(0.0) / total) * TAU;
            let start = angle;
            angle += sweep;
            (start, sweep)
        })
        .collect()
}

fn paint_ring_segment(
    painter: &egui::Painter,
    center: Pos2,
    inner: f32,
    outer: f32,
    start: f32,
    sweep: f32,
    color: Color32,
) {
    let steps = ((sweep / TAU) * 96.0).ceil().max(2.0) as usize;
    let mut mesh = egui::Mesh::default();
    for step in 0..=steps {
        let angle = start + sweep * step as f32 / steps as f32;
        let direction = Vec2::angled(angle);
        let outer_point = center + direction * outer;
        let inner_point = center + direction * inner;
        mesh.colored_vertex(outer_point, color);
        mesh.colored_vertex(inner_point, color);
    }
    for step in 0..steps {
        let base = (step * 2) as u32;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn normalize_angle(angle: f32) -> f32 {
    angle.rem_euclid(TAU)
}

/// Shared formatting of a chart axis hour label: the local `HH:00` of a bucket start.
pub fn hour_label(hour_start_unix: i64) -> String {
    chrono::DateTime::from_timestamp(hour_start_unix, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%H:00")
                .to_string()
        })
        .unwrap_or_else(|| "--:--".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot() -> Rect {
        Rect::from_min_size(pos2(30.0, 8.0), Vec2::new(300.0, 100.0))
    }

    #[test]
    fn sample_points_span_the_plot_and_clamp_values() {
        let plot = plot();
        let first = sample_point(plot, 0, 5, 0.0, 100.0);
        let last = sample_point(plot, 4, 5, 100.0, 100.0);
        assert!((first.x - plot.left()).abs() < 0.01);
        assert!((first.y - plot.bottom()).abs() < 0.01);
        assert!((last.x - plot.right()).abs() < 0.01);
        assert!((last.y - plot.top()).abs() < 0.01);

        let over = sample_point(plot, 2, 5, 250.0, 100.0);
        assert!((over.y - plot.top()).abs() < 0.01, "values above the scale clamp to the top");
        let under = sample_point(plot, 2, 5, -50.0, 100.0);
        assert!((under.y - plot.bottom()).abs() < 0.01);
    }

    #[test]
    fn a_single_sample_centres_in_the_plot() {
        let plot = plot();
        let point = sample_point(plot, 0, 1, 50.0, 100.0);
        assert!((point.x - plot.center().x).abs() < 0.01);
    }

    #[test]
    fn donut_sweeps_cover_a_full_turn_and_keep_zero_slices_empty() {
        let sweeps = segment_sweeps(&[1.0, 1.0, 2.0, 0.0]);
        assert_eq!(sweeps.len(), 4);
        let total: f32 = sweeps.iter().map(|(_, sweep)| *sweep).sum();
        assert!((total - TAU).abs() < 0.001, "slices must cover exactly one turn");
        assert_eq!(sweeps[3].1, 0.0, "a zero slice has no sweep");
        assert!((sweeps[0].0 + FRAC_PI_2).abs() < 0.001, "the first slice starts at twelve o'clock");
    }

    #[test]
    fn an_empty_donut_has_no_sweeps() {
        assert_eq!(segment_sweeps(&[0.0, 0.0]), vec![(0.0, 0.0), (0.0, 0.0)]);
    }

    #[test]
    fn hour_labels_are_local_hour_starts() {
        // A fixed UTC instant is formatted through the local timezone; the exact hour depends on
        // the machine, but the format is always HH:00.
        let label = hour_label(1_764_000_000);
        assert_eq!(label.len(), 5, "was {label:?}");
        assert!(label.ends_with(":00"), "was {label:?}");
    }
}
