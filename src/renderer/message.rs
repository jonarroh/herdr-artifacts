//! Messages and notifications.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::protocol::Level;

pub fn level_style(level: Level) -> (&'static str, Color) {
    match level {
        Level::Info => ("ℹ INFO", Color::Cyan),
        Level::Success => ("✔ SUCCESS", Color::Green),
        Level::Warning => ("▲ WARNING", Color::Yellow),
        Level::Error => ("✖ ERROR", Color::Red),
    }
}

pub fn lines(body: &str, level: Level) -> Vec<Line<'static>> {
    let (label, color) = level_style(level);
    let mut lines = vec![
        Line::from(Span::styled(label, Style::new().fg(color).add_modifier(Modifier::BOLD))),
        Line::default(),
    ];
    lines.extend(body.lines().map(|l| Line::from(l.to_string())));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_then_body() {
        let lines = lines("Build completed\n12 files generated", Level::Success);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].to_string(), "✔ SUCCESS");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
        assert_eq!(lines[3].to_string(), "12 files generated");
    }
}
