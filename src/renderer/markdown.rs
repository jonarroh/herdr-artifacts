//! Markdown to styled terminal lines via `pulldown-cmark`.
//!
//! Raw HTML is never interpreted: it is shown as dimmed literal text.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn lines(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut b = Builder { width, ..Builder::default() };
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(text, options) {
        b.event(event);
    }
    b.flush();
    while b.lines.last().is_some_and(|l| l.spans.is_empty()) {
        b.lines.pop();
    }
    b.lines
}

#[derive(Default)]
struct Builder {
    width: u16,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    styles: Vec<Style>,
    /// One entry per open list: the next number for ordered lists.
    lists: Vec<Option<u64>>,
    quotes: usize,
    in_code: bool,
    links: Vec<String>,
}

const CODE: Style = Style::new().fg(Color::Yellow);
const DIM: Style = Style::new().fg(Color::DarkGray);

impl Builder {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, f: impl FnOnce(Style) -> Style) {
        self.styles.push(f(self.style()));
    }

    fn flush(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let mut spans = Vec::with_capacity(self.current.len() + 1);
        if self.quotes > 0 {
            spans.push(Span::styled("▌ ".repeat(self.quotes), DIM));
        }
        spans.append(&mut self.current);
        self.lines.push(Line::from(spans));
    }

    fn blank(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) if self.in_code => {
                for line in t.lines() {
                    self.current.push(Span::styled(format!("  {line}"), CODE));
                    self.flush();
                }
            }
            Event::Text(t) => self.current.push(Span::styled(t.into_string(), self.style())),
            Event::Code(t) => self.current.push(Span::styled(t.into_string(), CODE)),
            // Shown verbatim, never interpreted.
            Event::Html(t) => {
                // Block-level HTML: each source line is its own line.
                for line in t.lines() {
                    self.current.push(Span::styled(line.to_string(), DIM));
                    self.flush();
                }
            }
            Event::InlineHtml(t) => self.current.push(Span::styled(t.into_string(), DIM)),
            Event::SoftBreak => self.current.push(Span::raw(" ")),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.blank();
                self.lines.push(Line::from(Span::styled("─".repeat(usize::from(self.width.clamp(3, 80))), DIM)));
                self.lines.push(Line::default());
            }
            Event::TaskListMarker(done) => self.current.push(Span::raw(if done { "[x] " } else { "[ ] " })),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.blank();
                let (marker, color) = match level {
                    HeadingLevel::H1 => ("█ ", Color::Cyan),
                    HeadingLevel::H2 => ("▓ ", Color::Blue),
                    _ => ("▒ ", Color::Magenta),
                };
                let style = Style::new().fg(color).add_modifier(Modifier::BOLD);
                self.current.push(Span::styled(marker, style));
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quotes += 1;
            }
            Tag::CodeBlock(kind) => {
                self.blank();
                self.in_code = true;
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty() {
                        self.lines.push(Line::from(Span::styled(format!("  ╭ {lang}"), DIM)));
                    }
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let indent = "  ".repeat(self.lists.len().saturating_sub(1));
                let bullet = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{indent}{}. ", *n - 1)
                    }
                    _ => format!("{indent}• "),
                };
                self.current.push(Span::styled(bullet, Style::new().fg(Color::Cyan)));
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.links.push(dest_url.into_string());
                self.push_style(|s| s.fg(Color::Blue).add_modifier(Modifier::UNDERLINED));
            }
            Tag::Image { .. } => {
                self.current.push(Span::styled("[image: ", DIM));
                self.styles.push(DIM);
            }
            Tag::TableHead => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.styles.pop();
                self.blank();
            }
            TagEnd::Paragraph => {
                // Paragraphs inside list items stay tight.
                if self.lists.is_empty() { self.blank() } else { self.flush() }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quotes = self.quotes.saturating_sub(1);
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some(url) = self.links.pop() {
                    let shown: String = self.current.last().map(|s| s.content.to_string()).unwrap_or_default();
                    if !url.is_empty() && shown != url {
                        self.current.push(Span::styled(format!(" ({url})"), DIM));
                    }
                }
            }
            TagEnd::Image => {
                self.styles.pop();
                self.current.push(Span::styled("]", DIM));
            }
            TagEnd::TableHead => {
                self.styles.pop();
                self.flush();
            }
            TagEnd::TableRow => self.flush(),
            TagEnd::TableCell => self.current.push(Span::styled(" │ ", DIM)),
            TagEnd::Table => self.blank(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Vec<String> {
        lines(text, 40).iter().map(Line::to_string).collect()
    }

    #[test]
    fn headings_paragraphs_and_inline_styles() {
        let out = lines("# Title\n\nSome **bold** and *italic* and `code`.", 40);
        assert_eq!(out[0].to_string(), "█ Title");
        assert!(out[0].spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(out[1].spans.is_empty(), "blank line after heading");
        let para = &out[2];
        assert_eq!(para.to_string(), "Some bold and italic and code.");
        let style_of = |needle: &str| para.spans.iter().find(|s| s.content == needle).unwrap().style;
        assert!(style_of("bold").add_modifier.contains(Modifier::BOLD));
        assert!(style_of("italic").add_modifier.contains(Modifier::ITALIC));
        assert_eq!(style_of("code").fg, Some(Color::Yellow));
    }

    #[test]
    fn lists_and_tasks() {
        let out = plain("- one\n- two\n  - nested\n\n1. first\n2. second\n\n- [x] done\n- [ ] todo");
        assert_eq!(out, ["• one", "• two", "  • nested", "", "1. first", "2. second", "", "• [x] done", "• [ ] todo"]);
    }

    #[test]
    fn code_blocks_quotes_rules_links() {
        let out = plain("```rust\nfn main() {}\n```\n\n> quoted\n\n---\n\n[docs](https://herdr.dev)");
        assert_eq!(out[0], "  ╭ rust");
        assert_eq!(out[1], "  fn main() {}");
        assert!(out.contains(&"▌ quoted".to_string()), "{out:?}");
        assert!(out.iter().any(|l| l.starts_with("───")));
        assert_eq!(out.last().unwrap(), "docs (https://herdr.dev)");
    }

    #[test]
    fn html_is_shown_literally_never_interpreted() {
        let out = plain("before\n\n<script>alert(1)</script>\n\ninline <b>tag</b>");
        assert!(out.contains(&"<script>alert(1)</script>".to_string()), "{out:?}");
        assert_eq!(out.last().unwrap(), "inline <b>tag</b>");
    }

    #[test]
    fn tables_become_rows() {
        let out = plain("| a | b |\n|---|---|\n| 1 | 2 |");
        assert_eq!(out, ["a │ b │ ", "1 │ 2 │ "]);
    }
}
