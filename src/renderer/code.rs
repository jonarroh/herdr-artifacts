//! Code view: line numbers and verbatim text (no highlighting in the MVP).

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn lines(text: &str, language: Option<&str>) -> Vec<Line<'static>> {
    let total = text.lines().count().max(1);
    let gutter = total.to_string().len();
    let dim = Style::new().fg(Color::DarkGray);
    let mut out = Vec::with_capacity(total + 1);
    if let Some(language) = language {
        out.push(Line::from(Span::styled(format!("{:gutter$} │ {language}", ""), dim.add_modifier(Modifier::ITALIC))));
    }
    for (i, line) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(format!("{:>gutter$} │ ", i + 1), dim),
            Span::raw(line.to_string()),
        ]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_lines_with_aligned_gutter() {
        let src = (1..=10).map(|i| format!("let x{i} = {i};")).collect::<Vec<_>>().join("\n");
        let lines = lines(&src, Some("rust"));
        assert_eq!(lines.len(), 11);
        assert_eq!(lines[0].to_string(), "   │ rust");
        assert_eq!(lines[1].to_string(), " 1 │ let x1 = 1;");
        assert_eq!(lines[10].to_string(), "10 │ let x10 = 10;");
    }
}
