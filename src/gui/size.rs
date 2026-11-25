//! How big the window should be for what it shows. These are estimates from
//! the content, in logical points; the user can still resize the window.

use crate::protocol::{Artifact, ChartStyle, Content};

pub const MIN: [f32; 2] = [420.0, 300.0];
pub const MAX: [f32; 2] = [1100.0, 820.0];
/// Extra width while the history sidebar is open.
pub const HISTORY_WIDTH: f32 = 240.0;
const HISTORY_MIN_HEIGHT: f32 = 460.0;

/// Window height taken by the header, the footer and the card's padding.
const CHROME_HEIGHT: f32 = 192.0;
/// Window width taken by margins and the card's padding.
const CHROME_WIDTH: f32 = 76.0;

/// A message this short is shown large and centred, like a headline.
pub const HEADLINE_CHARS: usize = 60;

pub fn is_headline(body: &str) -> bool {
    let body = body.trim();
    !body.contains('\n') && body.chars().count() <= HEADLINE_CHARS
}

/// Number of lines `text` takes when wrapped at `columns` characters.
fn wrapped_lines(text: &str, columns: usize) -> usize {
    text.lines().map(|l| l.chars().count().div_ceil(columns).max(1)).sum()
}

fn longest_line(text: &str) -> usize {
    text.lines().map(|l| l.chars().count()).max().unwrap_or(0)
}

/// Width and height the content alone would like.
fn content_size(content: &Content, image: Option<(u32, u32)>) -> [f32; 2] {
    match content {
        Content::Message { body, .. } if is_headline(body) => [body.chars().count() as f32 * 15.0, 108.0],
        Content::Message { body, .. } => [540.0, wrapped_lines(body, 66) as f32 * 23.0 + 56.0],
        Content::Qr { .. } => [400.0, 400.0],
        Content::Image { .. } => {
            let Some((w, h)) = image.filter(|&(w, h)| w > 0 && h > 0) else { return [480.0, 340.0] };
            let (w, h) = (w as f32, h as f32);
            let fit = (MAX[0] - CHROME_WIDTH) / w;
            let fit = fit.min((MAX[1] - CHROME_HEIGHT) / h);
            // Shrink to fit; enlarge small images to a comfortable width.
            let scale = fit.min((440.0 / w).clamp(1.0, 4.0));
            [w * scale, h * scale]
        }
        Content::Markdown { text } => [640.0, wrapped_lines(text, 76) as f32 * 27.0 + 20.0],
        Content::Code { text, language } => {
            let label = if language.is_some() { 24.0 } else { 0.0 };
            [longest_line(text) as f32 * 7.6 + 72.0, text.lines().count() as f32 * 24.0 + 24.0 + label]
        }
        Content::Table { columns, rows } => {
            let width: f32 = (0..columns.len())
                .map(|i| {
                    let cells = rows.iter().filter_map(|r| r.get(i)).map(|c| c.chars().count());
                    let widest = cells.chain([columns[i].chars().count()]).max().unwrap_or(0);
                    widest.min(60) as f32 * 8.0 + 28.0
                })
                .sum();
            [width + 16.0, (rows.len() + 1) as f32 * 36.0 + 20.0]
        }
        Content::Chart { style: ChartStyle::Bar, points, .. } => [540.0, points.len() as f32 * 34.0 + 12.0],
        Content::Chart { style: ChartStyle::Line, .. } => [640.0, 320.0],
    }
}

/// Width needed to keep the action buttons and the tools on one row.
fn footer_width(artifact: &Artifact) -> f32 {
    let actions: f32 =
        artifact.actions.iter().map(|a| (a.label.chars().count() as f32 * 8.5 + 40.0).max(104.0) + 10.0).sum();
    actions + 200.0
}

/// The window size for `artifact`. `image` is the pixel size of an image
/// artifact's file, if known.
pub fn preferred(artifact: &Artifact, content: Option<&Content>, image: Option<(u32, u32)>, history: bool) -> [f32; 2] {
    let [content_width, content_height] = content.map_or([480.0, 160.0], |c| content_size(c, image));
    let title = artifact.title.chars().count().min(56) as f32 * 11.0 + 120.0;
    let description = if artifact.description.is_some() { 30.0 } else { 0.0 };
    let width = (content_width + CHROME_WIDTH).max(footer_width(artifact)).max(title);
    let height = content_height + CHROME_HEIGHT + description;
    let mut size = [width.clamp(MIN[0], MAX[0]).round(), height.clamp(MIN[1], MAX[1]).round()];
    if history {
        size = [size[0] + HISTORY_WIDTH, size[1].max(HISTORY_MIN_HEIGHT)];
    }
    size
}

/// Shrink `size` so the window fits on a monitor of `monitor` points.
pub fn fit_monitor(size: [f32; 2], monitor: Option<[f32; 2]>) -> [f32; 2] {
    match monitor {
        Some([w, h]) if w > 0.0 && h > 0.0 => [size[0].min(w * 0.94), size[1].min(h * 0.88)],
        _ => size,
    }
}

/// Whether a new preferred size is different enough to be worth moving the
/// window. Small changes, such as a ticking counter, must not undo a manual
/// resize or make the window jitter.
pub fn differs(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() > 60.0 || (a[1] - b[1]).abs() > 60.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Limits, protocol::Action};
    use serde_json::{Value, json};

    fn size(kind: &str, content: Value) -> [f32; 2] {
        let parsed = Content::parse(kind, &content, &Limits::default()).unwrap();
        preferred(&artifact(kind, content), Some(&parsed), None, false)
    }

    fn artifact(kind: &str, content: Value) -> Artifact {
        serde_json::from_value(
            json!({"id": "a", "type": kind, "title": "T", "created_at": "2026-11-24T20:00:00Z", "content": content}),
        )
        .unwrap()
    }

    #[test]
    fn each_kind_of_content_gets_its_own_size() {
        let headline = size("message", json!({"body": "29 días 04 h"}));
        let report = size("markdown", json!({"text": "line\n\n".repeat(80)}));
        assert_eq!(headline, MIN);
        assert!(report[0] > headline[0] && report[1] == MAX[1], "{report:?}");
        let wide = size("code", json!({"text": "x".repeat(400)}));
        assert_eq!(wide[0], MAX[0], "never wider than the maximum");
        let few = size("chart", json!({"points": [{"label": "a", "value": 1}]}));
        let many = size("chart", json!({"points": vec![json!({"label": "a", "value": 1}); 12]}));
        assert!(many[1] > few[1]);
    }

    #[test]
    fn images_keep_their_shape_within_limits() {
        let content = Content::Image { source: "a.png".into() };
        let a = artifact("image", json!({"source": "a.png"}));
        let wide = preferred(&a, Some(&content), Some((4000, 1000)), false);
        let tall = preferred(&a, Some(&content), Some((1000, 4000)), false);
        assert!(wide[0] > wide[1] && tall[1] == MAX[1] && tall[0] == MIN[0], "{wide:?} {tall:?}");
    }

    #[test]
    fn buttons_history_and_monitor_are_taken_into_account() {
        let mut a = artifact("message", json!({"body": "x"}));
        let narrow = preferred(&a, None, None, false);
        let action = |id: &str| Action {
            id: id.into(),
            label: "Una acción larga".into(),
            kind: "respond".into(),
            payload: Value::Null,
            confirm: false,
        };
        a.actions = vec![action("a"), action("b"), action("c")];
        let wide = preferred(&a, None, None, false);
        assert!(wide[0] > narrow[0]);
        assert_eq!(preferred(&a, None, None, true)[0], wide[0] + HISTORY_WIDTH);
        assert_eq!(fit_monitor([1100.0, 820.0], Some([1000.0, 600.0])), [940.0, 528.0]);
        assert!(!differs([500.0, 400.0], [530.0, 380.0]));
        assert!(differs([500.0, 400.0], [700.0, 400.0]));
    }
}
