//! Charts drawn as SVG at their laid-out size (so strokes stay crisp), with
//! axis labels as real text in the interface font. Time runs in reading
//! order: right to left in Arabic.
//!
//! Values are integers (money in minor units, counts): charts only place
//! them, the labels come from the caller's formatter.

use std::fmt::Write as _;
use std::rc::Rc;

use nsh_core::element::{ImageSrc, View};
use nsh_core::prelude::*;

use super::kit::{caption, dot, figure, label};
use super::tokens::{FIGURE_FONT, Tone, series_hex, ty};

/// One series of an area/line chart.
#[derive(Clone)]
pub struct Series {
    pub name: String,
    pub values: Vec<i64>,
    /// Filled under the line (the main series) or a plain dashed line (a comparison).
    pub filled: bool,
}

/// The smallest round step (1, 2, 2.5, 3, 4, 5, 6, 8 × 10^n) at least `v`.
fn nice_step(v: i64) -> i64 {
    if v <= 0 {
        return 1;
    }
    let mut p = 1i64;
    while p.saturating_mul(10) <= v {
        p = p.saturating_mul(10);
    }
    // Tenths of the power, so 2.5 stays an integer.
    for m in [10, 20, 25, 30, 40, 50, 60, 70, 80, 100] {
        let c = p.saturating_mul(m) / 10;
        if c >= v {
            return c.max(1);
        }
    }
    p.saturating_mul(10)
}

/// The axis top for `rows` grid rows: a round step times the rows.
fn nice_ceiling(max: i64, rows: i64) -> i64 {
    nice_step((max + rows - 1) / rows) * rows
}

fn ratio(v: i64, max: i64) -> f32 {
    // Display geometry only: the ratio of two integers as a position.
    if max == 0 { 0.0 } else { v as f32 / max as f32 }
}

/// Smooth path through points that never overshoots them (monotone cubic,
/// Fritsch–Carlson), as cubic Béziers. Works with x increasing or decreasing.
fn smooth_path(pts: &[(f32, f32)]) -> String {
    let mut d = String::new();
    let n = pts.len();
    if let Some((x, y)) = pts.first() {
        let _ = write!(d, "M{x:.1},{y:.1}");
    }
    if n < 2 {
        return d;
    }
    let dx: Vec<f32> = pts.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let slope: Vec<f32> = pts
        .windows(2)
        .zip(&dx)
        .map(|(w, dx)| {
            if *dx == 0.0 {
                0.0
            } else {
                (w[1].1 - w[0].1) / dx
            }
        })
        .collect();
    let mut m = vec![0.0f32; n];
    m[0] = slope[0];
    m[n - 1] = slope[n - 2];
    for i in 1..n - 1 {
        let (a, b) = (slope[i - 1], slope[i]);
        m[i] = if a * b <= 0.0 {
            0.0
        } else {
            3.0 * (dx[i - 1] + dx[i])
                / ((2.0 * dx[i] + dx[i - 1]) / a + (dx[i] + 2.0 * dx[i - 1]) / b)
        };
    }
    for i in 0..n - 1 {
        let (p1, p2) = (pts[i], pts[i + 1]);
        let h = dx[i] / 3.0;
        let _ = write!(
            d,
            " C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
            p1.0 + h,
            p1.1 + m[i] * h,
            p2.0 - h,
            p2.1 - m[i + 1] * h,
            p2.0,
            p2.1
        );
    }
    d
}

/// An area chart with a grid, Y labels at the reading start and X labels below.
pub fn area_chart(
    series: Vec<Series>,
    x_labels: Vec<String>,
    height: f32,
    rtl: bool,
    y_label: Rc<dyn Fn(i64) -> String>,
) -> View {
    let size = signal((0.0f32, 0.0f32));
    const ROWS: i64 = 4;
    let max = nice_ceiling(
        series
            .iter()
            .flat_map(|s| s.values.iter().copied())
            .max()
            .unwrap_or(0),
        ROWS,
    );
    let axis_w = 64.0;
    let pad_top = 8.0;
    let plot_h = height - pad_top;

    let series_svg = series.clone();
    let svg_src = move || {
        let (w, h) = size.get();
        if w < 2.0 || h < 2.0 {
            return ImageSrc::svg_markup("<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
        }
        let grid = Tone::Border.hex();
        let mut s = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.1} {h:.1}\">"
        );
        for r in 0..=ROWS {
            let y = pad_top + plot_h * (r as f32) / ROWS as f32 - 0.5;
            let dash = if r == ROWS {
                ""
            } else {
                " stroke-dasharray=\"3 4\""
            };
            let _ = write!(
                s,
                "<line x1=\"0\" x2=\"{w:.1}\" y1=\"{y:.1}\" y2=\"{y:.1}\" stroke=\"{grid}\" stroke-width=\"1\"{dash}/>"
            );
        }
        for (i, se) in series_svg.iter().enumerate().rev() {
            let n = se.values.len().max(2) - 1;
            let pts: Vec<(f32, f32)> = se
                .values
                .iter()
                .enumerate()
                .map(|(k, &v)| {
                    let f = k as f32 / n as f32;
                    let x = if rtl { 1.0 - f } else { f } * (w - 12.0) + 6.0;
                    let y = pad_top + plot_h * (1.0 - ratio(v, max)) - 1.0;
                    (x, y)
                })
                .collect();
            let color = series_hex(i);
            let d = smooth_path(&pts);
            if se.filled {
                let (fx, lx) = (
                    pts.first().map_or(0.0, |p| p.0),
                    pts.last().map_or(0.0, |p| p.0),
                );
                let _ = write!(
                    s,
                    "<defs><linearGradient id=\"g{i}\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"{color}\" stop-opacity=\"0.22\"/><stop offset=\"1\" stop-color=\"{color}\" stop-opacity=\"0\"/></linearGradient></defs><path d=\"{d} L{lx:.1},{h:.1} L{fx:.1},{h:.1} Z\" fill=\"url(#g{i})\"/>"
                );
                let _ = write!(
                    s,
                    "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
                );
                if let Some((x, y)) = pts.last() {
                    let bg = Tone::Surface.hex();
                    let _ = write!(
                        s,
                        "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\" fill=\"{bg}\" stroke=\"{color}\" stroke-width=\"2\"/>"
                    );
                }
            } else {
                let _ = write!(
                    s,
                    "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\" stroke-dasharray=\"4 4\"/>"
                );
            }
        }
        s.push_str("</svg>");
        ImageSrc::svg_markup(&s)
    };

    // Y labels, placed at their grid lines.
    let mut axis = view().w(px(axis_w)).h(px(height));
    for r in 0..=ROWS {
        let v = max - max * r / ROWS;
        let y = pad_top + plot_h * (r as f32) / ROWS as f32;
        let text_of = y_label.clone();
        axis = axis.child(
            view()
                .absolute()
                .top(px(y - 9.0))
                .left(px(0.0))
                .right(px(0.0))
                .pe(px(10.0))
                .items_end()
                .child(
                    label(move || text_of(v), ty::XS, FontWeight::NORMAL, Tone::Muted)
                        .font_family(FIGURE_FONT)
                        .number_of_lines(1),
                ),
        );
    }

    let plot = view()
        .flex_1()
        .h(px(height))
        .on_layout(move |b: &Bounds<Points>| {
            let next = (b.size.width.0, b.size.height.0);
            if size.get_untracked() != next {
                size.set(next);
            }
        })
        .child(
            svg(svg_src)
                .absolute()
                .top_0()
                .left_0()
                .w(move || px(size.get().0))
                .h(px(height)),
        );

    // X labels: spread under the plot, thinned to fit.
    let step = (x_labels.len() / 7).max(1);
    let mut xrow = view()
        .flex_row()
        .justify_between()
        .ms(px(axis_w))
        .pt(px(8.0));
    for (i, l) in x_labels.into_iter().enumerate() {
        let shown = i % step == 0;
        xrow = xrow.child(
            view().w(px(1.0)).items_center().child(
                caption(
                    if shown { l } else { String::new() },
                    ty::XS,
                    FontWeight::NORMAL,
                    Tone::Muted,
                )
                .number_of_lines(1)
                .w(px(64.0))
                .text_center(),
            ),
        );
    }

    view()
        .child(view().flex_row().child(axis).child(plot))
        .child(xrow)
}

/// A legend: colored dots with names.
pub fn legend(names: Vec<String>) -> View {
    let mut row = view().flex_row().items_center().gap(px(16.0));
    for (i, n) in names.into_iter().enumerate() {
        let c = i;
        row = row.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    view()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded_full()
                        .bg(move || -> AdaptiveColor { hex_color(&series_hex(c)) }),
                )
                .child(caption(n, ty::XS, FontWeight::MEDIUM, Tone::Muted)),
        );
    }
    row
}

fn hex_color(h: &str) -> AdaptiveColor {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0);
    AdaptiveColor::new(rgb(v), rgb(v))
}

/// A tiny trend line for KPI cards.
pub fn sparkline(values: Vec<i64>, w: f32, h: f32, tone: Tone, rtl: bool) -> View {
    let src = move || {
        let max = values.iter().copied().max().unwrap_or(1).max(1);
        let min = values.iter().copied().min().unwrap_or(0);
        let span = (max - min).max(1);
        let n = values.len().max(2) - 1;
        let pts: Vec<(f32, f32)> = values
            .iter()
            .enumerate()
            .map(|(k, &v)| {
                let f = k as f32 / n as f32;
                let x = if rtl { 1.0 - f } else { f } * (w - 4.0) + 2.0;
                let y = 2.0 + (h - 4.0) * (1.0 - ratio(v - min, span));
                (x, y)
            })
            .collect();
        let c = tone.hex();
        let d = smooth_path(&pts);
        let (fx, lx) = (
            pts.first().map_or(0.0, |p| p.0),
            pts.last().map_or(0.0, |p| p.0),
        );
        ImageSrc::svg_markup(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><defs><linearGradient id=\"s\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"{c}\" stop-opacity=\"0.18\"/><stop offset=\"1\" stop-color=\"{c}\" stop-opacity=\"0\"/></linearGradient></defs><path d=\"{d} L{lx:.1},{h} L{fx:.1},{h} Z\" fill=\"url(#s)\"/><path d=\"{d}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"1.75\" stroke-linecap=\"round\"/></svg>"
        ))
    };
    view().child(svg(src).w(px(w)).h(px(h)))
}

/// A donut of shares (payment methods…), with the total in the middle.
pub fn donut(
    parts: Vec<(i64, Tone)>,
    size: f32,
    center: impl Fn() -> String + 'static,
    center_caption: impl Fn() -> String + 'static,
) -> View {
    let src = move || {
        let total: i64 = parts.iter().map(|p| p.0).sum::<i64>().max(1);
        let r = size / 2.0 - 8.0;
        let circ = 2.0 * std::f32::consts::PI * r;
        let c = size / 2.0;
        let track = Tone::Subtle.hex();
        let mut s = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 {size} {size}\"><circle cx=\"{c}\" cy=\"{c}\" r=\"{r}\" fill=\"none\" stroke=\"{track}\" stroke-width=\"12\"/>"
        );
        let gap = 3.0;
        let mut offset = 0.0f32;
        for (v, tone) in &parts {
            let len = circ * ratio(*v, total);
            let seg = (len - gap).max(0.5);
            let color = tone.hex();
            let _ = write!(
                s,
                "<circle cx=\"{c}\" cy=\"{c}\" r=\"{r}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"12\" stroke-dasharray=\"{seg:.2} {:.2}\" stroke-dashoffset=\"{:.2}\" transform=\"rotate(-90 {c} {c})\"/>",
                circ - seg,
                -offset
            );
            offset += len;
        }
        s.push_str("</svg>");
        ImageSrc::svg_markup(&s)
    };
    view()
        .w(px(size))
        .h(px(size))
        .items_center()
        .justify_center()
        .child(svg(src).absolute().top_0().left_0().w(px(size)).h(px(size)))
        .child(figure(center, ty::XL, FontWeight::BOLD, || Tone::Ink))
        .child(label(
            center_caption,
            ty::XS,
            FontWeight::NORMAL,
            Tone::Muted,
        ))
}

/// Horizontal bars with a name and a value per row (shares by category).
pub fn bar_list(rows: Vec<(String, i64, String)>, tone: Tone) -> View {
    let max = rows.iter().map(|r| r.1).max().unwrap_or(1).max(1);
    let mut list = view().gap(px(14.0));
    for (name, v, shown) in rows {
        let share = ratio(v, max);
        list = list.child(
            view()
                .gap(px(6.0))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            view().flex_1().child(
                                caption(name, ty::SM, FontWeight::MEDIUM, Tone::Body)
                                    .number_of_lines(1),
                            ),
                        )
                        .child(figure(
                            move || shown.clone(),
                            ty::SM,
                            FontWeight::SEMIBOLD,
                            || Tone::Ink,
                        )),
                )
                .child(
                    view()
                        .h(px(6.0))
                        .rounded_full()
                        .bg(Tone::Subtle.color())
                        .flex_row()
                        .child(
                            view()
                                .w(relative(share))
                                .h_full()
                                .rounded_full()
                                .bg(tone.color()),
                        ),
                ),
        );
    }
    list
}

/// Upright bars (hours, days of the week): a bar per value, its label
/// under it (empty labels leave the place blank), the highest in the
/// accent. Reads from the start side, whichever way the text goes.
pub fn column_chart(values: Vec<i64>, labels: Vec<String>, h: f32, tone: Tone) -> View {
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let top = values.iter().copied().max().unwrap_or(0);
    let mut row = view().flex_row().items_end().gap(px(4.0)).h(px(h));
    let mut names = view().flex_row().gap(px(4.0));
    for (i, v) in values.iter().copied().enumerate() {
        let share = ratio(v.max(0), max).max(if v > 0 { 0.03 } else { 0.0 });
        let strong = v > 0 && v == top;
        row = row.child(
            view().flex_1().h_full().justify_end().child(
                view()
                    .w_full()
                    .h(relative(share))
                    .min_h(px(2.0))
                    .rounded(px(3.0))
                    .opacity(if strong || v <= 0 { 1.0 } else { 0.45 })
                    .bg(if v > 0 {
                        tone.color()
                    } else {
                        Tone::Subtle.color()
                    }),
            ),
        );
        let name = labels.get(i).cloned().unwrap_or_default();
        names = names.child(
            view()
                .flex_1()
                .items_center()
                .child(caption(name, ty::XS, FontWeight::NORMAL, Tone::Muted).number_of_lines(1)),
        );
    }
    view().gap(px(6.0)).child(row).child(names)
}

/// A progress bar toward a goal (0–100 %).
pub fn progress(share: f32, tone: Tone) -> View {
    view()
        .h(px(10.0))
        .rounded_full()
        .bg(Tone::Subtle.color())
        .flex_row()
        .child(
            view()
                .w(relative(share.clamp(0.0, 1.0)))
                .h_full()
                .rounded_full()
                .bg(tone.color()),
        )
}

/// A legend row with a colored dot, a name and a value (for the donut).
pub fn legend_row(
    tone: Tone,
    name: impl Fn() -> String + 'static,
    value: impl Fn() -> String + 'static,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(dot(tone, 8.0))
        .child(
            view()
                .flex_1()
                .child(label(name, ty::SM, FontWeight::NORMAL, Tone::Body).number_of_lines(1)),
        )
        .child(figure(value, ty::SM, FontWeight::SEMIBOLD, || Tone::Ink))
}

#[cfg(test)]
mod tests {
    use super::nice_ceiling;

    #[test]
    fn axis_ceilings_are_round() {
        assert_eq!(nice_ceiling(0, 4), 4);
        assert_eq!(nice_ceiling(7, 4), 8);
        assert_eq!(nice_ceiling(180, 4), 200);
        assert_eq!(nice_ceiling(230, 4), 240);
        assert_eq!(nice_ceiling(23_000_000, 4), 24_000_000);
        assert_eq!(nice_ceiling(1_000, 4), 1_000);
    }
}
