//! Charts drawn with Unicode block elements: horizontal bars and a
//! column-based line/area chart. Both use eighth-blocks for sub-cell
//! precision.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::protocol::{ChartPoint, ChartStyle};

const H_BLOCKS: [&str; 9] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];
const V_BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const MAX_LABEL: usize = 16;

pub fn lines(style: ChartStyle, points: &[ChartPoint], unit: Option<&str>, width: u16, height: u16) -> Vec<Line<'static>> {
    match style {
        ChartStyle::Bar => bars(points, unit, width),
        ChartStyle::Line => columns(points, unit, width, height),
    }
}

/// Up to two decimals, without trailing zeros: `6`, `2.1`, `38.45`.
pub fn format_value(v: f64) -> String {
    let text = format!("{v:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { s.chars().take(max.saturating_sub(1)).chain(['…']).collect() }
}

/// One row per point: `label ██████▍ value`. Bar length is proportional to
/// `|value|`; negative values are drawn in red.
fn bars(points: &[ChartPoint], unit: Option<&str>, width: u16) -> Vec<Line<'static>> {
    let unit = unit.unwrap_or_default();
    let label_w = points.iter().map(|p| p.label.chars().count()).max().unwrap_or(0).min(MAX_LABEL);
    let values: Vec<String> = points.iter().map(|p| format!("{}{unit}", format_value(p.value))).collect();
    let value_w = values.iter().map(|v| v.chars().count()).max().unwrap_or(0);
    let bar_w = usize::from(width).saturating_sub(label_w + value_w + 2).max(1);
    let max = points.iter().map(|p| p.value.abs()).fold(0.0, f64::max);

    points
        .iter()
        .zip(values)
        .map(|(p, value)| {
            let eighths = if max > 0.0 { (p.value.abs() / max * (bar_w * 8) as f64).round() as usize } else { 0 };
            let mut bar = "█".repeat(eighths / 8);
            bar.push_str(H_BLOCKS[eighths % 8]);
            let pad = bar_w.saturating_sub(bar.chars().count());
            let color = if p.value < 0.0 { Color::Red } else { Color::Cyan };
            Line::from(vec![
                Span::raw(format!("{:>label_w$} ", truncate(&p.label, label_w))),
                Span::styled(bar, Style::new().fg(color)),
                Span::raw(format!("{:pad$} {value:>value_w$}", "")),
            ])
        })
        .collect()
}

/// A filled column chart scaled between the minimum and maximum value, with
/// a caption line giving the range and the first/last labels.
fn columns(points: &[ChartPoint], unit: Option<&str>, width: u16, height: u16) -> Vec<Line<'static>> {
    let unit = unit.unwrap_or_default();
    let width = usize::from(width).max(1);
    let rows = usize::from(height).saturating_sub(1).clamp(1, 20);
    let (min, max) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.value), hi.max(p.value)));
    let span = max - min;

    // Resample to exactly one value per column (bucket average when there
    // are more points than columns, repetition when there are fewer).
    let n = points.len();
    let cols = if n >= width { width } else { (width / n).max(1) * n };
    let levels: Vec<usize> = (0..cols)
        .map(|c| {
            let (start, end) = (c * n / cols, ((c + 1) * n).div_ceil(cols).max(c * n / cols + 1).min(n));
            let avg = points[start..end].iter().map(|p| p.value).sum::<f64>() / (end - start) as f64;
            // Keep at least one eighth so the minimum is still visible.
            let frac = if span > 0.0 { (avg - min) / span } else { 0.5 };
            ((frac * (rows * 8) as f64).round() as usize).max(1)
        })
        .collect();

    let mut out: Vec<Line<'static>> = (0..rows)
        .rev()
        .map(|row| {
            let text: String = levels.iter().map(|&l| V_BLOCKS[l.saturating_sub(row * 8).min(8)]).collect();
            Line::from(Span::styled(text, Style::new().fg(Color::Cyan)))
        })
        .collect();
    let first = truncate(&points[0].label, MAX_LABEL);
    let last = truncate(&points[n - 1].label, MAX_LABEL);
    out.push(Line::from(Span::styled(
        format!("{first} → {last}   min {}{unit}  max {}{unit}", format_value(min), format_value(max)),
        Style::new().fg(Color::DarkGray),
    )));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(values: &[(&str, f64)]) -> Vec<ChartPoint> {
        values.iter().map(|(l, v)| ChartPoint { label: l.to_string(), value: *v }).collect()
    }

    #[test]
    fn bars_are_proportional() {
        let lines = lines(ChartStyle::Bar, &pts(&[("build", 10.0), ("test", 5.0), ("lint", 0.0)]), Some("s"), 30, 10);
        let text: Vec<String> = lines.iter().map(Line::to_string).collect();
        // width 30 - label 5 - value 3 - 2 = 20 cells of bar.
        assert_eq!(text[0], format!("build {} 10s", "█".repeat(20)));
        assert_eq!(text[1], format!(" test {}{}  5s", "█".repeat(10), " ".repeat(10)));
        assert_eq!(text[2], format!(" lint {}  0s", " ".repeat(20)));
    }

    #[test]
    fn negative_bars_are_red_and_fractional_values_formatted() {
        let lines = lines(ChartStyle::Bar, &pts(&[("a", -2.5), ("b", 5.0)]), None, 20, 5);
        assert_eq!(lines[0].spans[1].style.fg, Some(Color::Red));
        assert!(lines[0].to_string().ends_with("-2.5"));
        assert_eq!(["6", "2.1", "38.45", "0", "0.13"], [6.0, 2.1, 38.449, -0.001, 0.126].map(format_value));
        assert_eq!(lines[1].spans[1].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn line_chart_rises_with_the_data() {
        let lines = lines(ChartStyle::Line, &pts(&[("mon", 0.0), ("tue", 5.0), ("wed", 10.0)]), None, 3, 3);
        let text: Vec<String> = lines.iter().map(Line::to_string).collect();
        assert_eq!(text[0], "  █", "top row: only the maximum reaches it");
        assert_eq!(text[1], "▁██", "bottom row: minimum keeps a sliver");
        assert_eq!(text[2], "mon → wed   min 0  max 10");
    }

    #[test]
    fn line_chart_resamples_to_the_width() {
        let many: Vec<ChartPoint> = (0..100).map(|i| ChartPoint { label: i.to_string(), value: f64::from(i) }).collect();
        let lines = lines(ChartStyle::Line, &many, None, 10, 4);
        assert_eq!(lines[0].to_string().chars().count(), 10);
        let flat = super::lines(ChartStyle::Line, &pts(&[("a", 3.0), ("b", 3.0)]), None, 8, 3);
        // Two chart rows: a constant series fills the lower half only.
        assert_eq!(flat[0].to_string(), "        ");
        assert_eq!(flat[1].to_string(), "████████", "constant series sits mid-height");
    }
}
