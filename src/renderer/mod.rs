//! Rendering: one module per artifact type, dispatched from [`render`].

pub mod chart;
pub mod code;
pub mod image;
pub mod markdown;
pub mod message;
pub mod qr;
pub mod table;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Widget, Wrap},
};

use crate::{config::Config, protocol::Content};

pub use self::image::ImageCache;

/// Single-cell glyph identifying an artifact type in titles and history.
pub fn type_icon(kind: &str) -> &'static str {
    match kind {
        "image" => "◩",
        "qr" => "▦",
        "message" => "●",
        "markdown" => "¶",
        "table" => "▤",
        "chart" => "▟",
        "code" => "#",
        _ => "?",
    }
}

/// Draw `content` into `area`. `scroll` is the vertical offset for types
/// that can be longer than the area.
pub fn render(content: &Content, area: Rect, buf: &mut Buffer, scroll: u16, images: &mut ImageCache, cfg: &Config) {
    if area.is_empty() {
        return;
    }
    match content {
        Content::Image { source } => images.render(cfg, source, area, buf),
        Content::Qr { data } => qr::render(data, area, buf),
        Content::Message { body, level } => text(message::lines(body, *level), area, buf, scroll, true),
        Content::Markdown { text: md } => text(markdown::lines(md, area.width), area, buf, scroll, true),
        Content::Code { language, text: src } => text(code::lines(src, language.as_deref()), area, buf, scroll, false),
        Content::Table { columns, rows } => table::render(columns, rows, area, buf, scroll),
        Content::Chart { style, points, unit } => {
            text(chart::lines(*style, points, unit.as_deref(), area.width, area.height), area, buf, 0, false)
        }
    }
}

fn text(lines: Vec<Line<'static>>, area: Rect, buf: &mut Buffer, scroll: u16, wrap: bool) {
    // Never scroll past the last line.
    let scroll = scroll.min(lines.len().saturating_sub(1) as u16);
    let mut paragraph = Paragraph::new(lines).scroll((scroll, 0));
    if wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    paragraph.render(area, buf);
}

/// Flatten a buffer region to plain text (test helper shared by renderers).
#[cfg(test)]
pub(crate) fn buffer_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
