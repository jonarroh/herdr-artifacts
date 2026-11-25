//! Tables: content-sized columns, bold header, row scrolling.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Row, Table, Widget},
};

const MAX_COLUMN_WIDTH: usize = 40;

/// Width of each column: its widest cell, capped.
pub fn column_widths(columns: &[String], rows: &[Vec<String>]) -> Vec<u16> {
    columns
        .iter()
        .enumerate()
        .map(|(i, header)| {
            let widest = rows.iter().filter_map(|r| r.get(i)).map(|c| c.chars().count()).max().unwrap_or(0);
            widest.max(header.chars().count()).clamp(1, MAX_COLUMN_WIDTH) as u16
        })
        .collect()
}

pub fn render(columns: &[String], rows: &[Vec<String>], area: Rect, buf: &mut Buffer, scroll: u16) {
    let widths = column_widths(columns, rows);
    // Only the rows that can be visible are turned into widgets.
    let visible = usize::from(area.height.saturating_sub(1));
    let first = usize::from(scroll).min(rows.len().saturating_sub(visible.max(1)));
    let body = rows.iter().skip(first).take(visible).enumerate().map(|(i, row)| {
        let style = if (first + i) % 2 == 1 { Style::new().fg(Color::Gray) } else { Style::new() };
        Row::new(row.iter().map(String::as_str)).style(style)
    });
    Table::new(body, widths.iter().map(|w| Constraint::Length(*w)))
        .header(
            Row::new(columns.iter().map(String::as_str))
                .style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        )
        .column_spacing(2)
        .render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::buffer_text;

    fn data() -> (Vec<String>, Vec<Vec<String>>) {
        let columns = vec!["File".to_string(), "Size".to_string()];
        let rows = (1..=6).map(|i| vec![format!("file-{i}.rs"), format!("{i}00")]).collect();
        (columns, rows)
    }

    #[test]
    fn widths_follow_content() {
        let (columns, rows) = data();
        assert_eq!(column_widths(&columns, &rows), [9, 4]);
        let long = vec![vec!["x".repeat(100), String::new()]];
        assert_eq!(column_widths(&columns, &long), [40, 4]);
    }

    #[test]
    fn renders_header_and_scrolls_rows() {
        let (columns, rows) = data();
        let area = Rect::new(0, 0, 30, 4);
        let mut buf = Buffer::empty(area);
        render(&columns, &rows, area, &mut buf, 0);
        let text = buffer_text(&buf);
        assert!(text.lines().next().unwrap().starts_with("File       Size"), "{text}");
        assert!(text.contains("file-1.rs  100") && text.contains("file-3.rs") && !text.contains("file-4.rs"));

        let mut buf = Buffer::empty(area);
        render(&columns, &rows, area, &mut buf, 99);
        let text = buffer_text(&buf);
        assert!(text.contains("file-6.rs") && text.contains("file-4.rs") && !text.contains("file-3.rs"), "{text}");
    }
}
