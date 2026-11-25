//! Drawing artifact content with native-window widgets.

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Rect, RichText, ScrollArea, Sense, Stroke, TextStyle,
    TextureHandle, TextureOptions, Ui, pos2, text::LayoutJob, vec2,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::{size, theme};
use crate::{
    config::Config,
    protocol::{ChartPoint, ChartStyle, Content, Level, artifact::is_store_file_name},
    renderer::{image::decode, qr::Matrix},
};

/// Larger images are shrunk before upload; GPUs cap the texture size.
const MAX_TEXTURE_SIDE: u32 = 2048;
/// Light border around a QR code, in modules, so cameras can read it.
const QR_QUIET: usize = 4;

type Texture = Result<TextureHandle, String>;

/// The one image on screen. Replaced when another one is shown.
#[derive(Default)]
pub struct ImageSlot {
    entry: Option<(String, Texture)>,
}

impl ImageSlot {
    fn get(&mut self, ctx: &egui::Context, cfg: &Config, source: &str) -> &Texture {
        if self.entry.as_ref().is_none_or(|(s, _)| s != source) {
            self.entry = Some((source.to_string(), load_texture(ctx, cfg, source)));
        }
        &self.entry.as_ref().expect("just filled").1
    }
}

fn load_texture(ctx: &egui::Context, cfg: &Config, source: &str) -> Texture {
    // The source is re-validated here: nothing outside `files/` is ever opened.
    if !is_store_file_name(source) {
        return Err("invalid image source".into());
    }
    let mut image = decode(&cfg.files_dir().join(source), cfg.limits.max_file_bytes, cfg.limits.max_image_dimension)
        .map_err(|e| e.to_string())?;
    if image.width() > MAX_TEXTURE_SIDE || image.height() > MAX_TEXTURE_SIDE {
        image = image.thumbnail(MAX_TEXTURE_SIDE, MAX_TEXTURE_SIDE);
    }
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ctx.load_texture(source, ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), TextureOptions::LINEAR))
}

/// Draw `content` into the remaining space of `ui`.
pub fn content(ui: &mut Ui, content: &Content, images: &mut ImageSlot, cfg: &Config) {
    match content {
        Content::Image { source } => match images.get(ui.ctx(), cfg, source) {
            Ok(texture) => {
                let available = ui.available_size();
                let natural = texture.size_vec2();
                // Fit inside the window; small images may grow up to 4x.
                let scale = (available.x / natural.x).min(available.y / natural.y).min(4.0);
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Image::new((texture.id(), natural * scale)).corner_radius(theme::RADIUS));
                });
            }
            Err(e) => error(ui, &format!("Cannot display image: {e}")),
        },
        Content::Qr { data } => qr(ui, data),
        Content::Message { body, level } => message(ui, body, *level),
        Content::Markdown { text } => {
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| markdown(ui, text));
        }
        Content::Code { language, text } => code(ui, language.as_deref(), text),
        Content::Table { columns, rows } => table(ui, columns, rows),
        Content::Chart { style, points, unit } => {
            let unit = unit.as_deref().unwrap_or_default();
            match style {
                ChartStyle::Bar => bars(ui, points, unit),
                ChartStyle::Line => line(ui, points, unit),
            }
        }
    }
}

fn message(ui: &mut Ui, body: &str, level: Level) {
    let label = match level {
        Level::Info => "● INFO",
        Level::Success => "✔ SUCCESS",
        Level::Warning => "⚠ WARNING",
        Level::Error => "✖ ERROR",
    };
    let color = theme::level_color(level);
    let strong = ui.visuals().widgets.active.fg_stroke.color;
    if size::is_headline(body) {
        // A short message is the whole point: large and centred.
        let body = body.trim();
        let fitted = (ui.available_width() * 1.9 / body.chars().count().max(1) as f32).clamp(18.0, 34.0);
        let block = 34.0 + fitted * 1.4;
        ui.add_space(((ui.available_height() - block) / 2.0).max(0.0));
        ui.vertical_centered(|ui| {
            theme::chip(ui, label, color);
            ui.add_space(6.0);
            ui.label(RichText::new(body).size(fitted).strong().color(strong));
        });
        return;
    }
    theme::chip(ui, label, color);
    ui.add_space(10.0);
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.label(RichText::new(body).size(16.0).line_height(Some(24.0)));
    });
}

pub fn error(ui: &mut Ui, message: &str) {
    ui.colored_label(ui.visuals().error_fg_color, message);
}

fn qr(ui: &mut Ui, data: &str) {
    let matrix = match Matrix::encode_with_quiet(data, QR_QUIET) {
        Ok(m) => m,
        Err(e) => return error(ui, &format!("Cannot encode QR: {e}")),
    };
    let caption_height = 40.0;
    let area = ui.available_size();
    let modules = matrix.side as f32;
    // Whole physical pixels per module keep the edges sharp.
    let ppp = ui.ctx().pixels_per_point();
    let module = ((area.x.min(area.y - caption_height) * ppp / modules).floor().max(1.0)) / ppp;
    let side = module * modules;
    ui.add_space(((area.y - side - caption_height) / 2.0).max(0.0));
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        // Always dark on light, whatever the theme.
        ui.painter().rect_filled(rect, f32::from(theme::CARD_RADIUS), Color32::WHITE);
        for y in 0..matrix.side {
            for x in 0..matrix.side {
                if matrix.dark(x, y) {
                    let min = rect.min + vec2(x as f32 * module, y as f32 * module);
                    ui.painter().rect_filled(Rect::from_min_size(min, vec2(module, module)), 0.0, Color32::BLACK);
                }
            }
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(data).monospace().weak()).truncate());
    });
}

fn code(ui: &mut Ui, language: Option<&str>, text: &str) {
    if let Some(language) = language {
        ui.weak(language);
    }
    let lines: Vec<&str> = text.lines().collect();
    let digits = lines.len().to_string().len();
    let height = ui.text_style_height(&TextStyle::Monospace);
    // Only the visible lines are laid out, so huge files stay responsive.
    ScrollArea::both().auto_shrink(false).show_rows(ui, height, lines.len(), |ui, range| {
        for i in range {
            let line = format!("{:>digits$}  {}", i + 1, lines[i]);
            ui.add(egui::Label::new(RichText::new(line).monospace()).extend());
        }
    });
}

fn table(ui: &mut Ui, columns: &[String], rows: &[Vec<String>]) {
    ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("artifact-table").striped(true).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
            for column in columns {
                ui.add(egui::Label::new(RichText::new(column).strong()).extend());
            }
            ui.end_row();
            for row in rows {
                for cell in row {
                    ui.add(egui::Label::new(cell).extend());
                }
                ui.end_row();
            }
        });
    });
}

fn number(value: f64, unit: &str) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}{unit}")
}

fn bars(ui: &mut Ui, points: &[ChartPoint], unit: &str) {
    let max = points.iter().map(|p| p.value.abs()).fold(0.0, f64::max);
    let color = theme::kind_color("chart");
    // Measured outside the grid, whose columns only know their contents.
    let longest = points.iter().map(|p| p.label.chars().count()).max().unwrap_or(0);
    let track = (ui.available_width() - longest.min(30) as f32 * 8.0 - 110.0).max(120.0);
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("artifact-bars").spacing(vec2(12.0, 8.0)).show(ui, |ui| {
            let rail = ui.visuals().faint_bg_color;
            for point in points {
                ui.label(&point.label);
                let (rect, _) = ui.allocate_exact_size(vec2(track, 16.0), Sense::hover());
                let fraction = if max > 0.0 { (point.value.abs() / max) as f32 } else { 0.0 };
                // Negative values keep their length but are drawn in red.
                let fill = if point.value < 0.0 { Color32::from_rgb(220, 70, 70) } else { color };
                let bar = Rect::from_min_size(rect.min, vec2(track * fraction, rect.height()));
                ui.painter().rect_filled(rect, 8.0, rail);
                ui.painter().rect_filled(bar, 8.0, fill);
                ui.label(number(point.value, unit));
                ui.end_row();
            }
        });
    });
}

fn line(ui: &mut Ui, points: &[ChartPoint], unit: &str) {
    let min = points.iter().map(|p| p.value).fold(f64::INFINITY, f64::min);
    let max = points.iter().map(|p| p.value).fold(f64::NEG_INFINITY, f64::max);
    let span = if max > min { max - min } else { 1.0 };
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(8.0, 22.0), rect.max - vec2(8.0, 22.0));
    let step = if points.len() > 1 { plot.width() / (points.len() - 1) as f32 } else { 0.0 };
    let place = |i: usize, value: f64| {
        let x = if points.len() > 1 { plot.left() + step * i as f32 } else { plot.center().x };
        pos2(x, plot.bottom() - ((value - min) / span) as f32 * plot.height())
    };
    let path: Vec<_> = points.iter().enumerate().map(|(i, p)| place(i, p.value)).collect();
    let color = ui.visuals().selection.bg_fill;
    let text = ui.visuals().text_color();
    let painter = ui.painter();
    for &point in &path {
        painter.circle_filled(point, 3.0, color);
    }
    painter.line(path, Stroke::new(2.0, color));

    let font = FontId::proportional(12.0);
    painter.text(rect.left_top(), Align2::LEFT_TOP, format!("max {}", number(max, unit)), font.clone(), text);
    painter.text(rect.right_top(), Align2::RIGHT_TOP, format!("min {}", number(min, unit)), font.clone(), text);
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        painter.text(rect.left_bottom(), Align2::LEFT_BOTTOM, &first.label, font.clone(), text);
        painter.text(rect.right_bottom(), Align2::RIGHT_BOTTOM, &last.label, font, text);
    }
    // The nearest point's exact value on hover.
    if let Some(pointer) = response.hover_pos()
        && let Some(i) = (0..points.len()).min_by(|&a, &b| {
            let distance = |i: usize| (place(i, points[i].value).x - pointer.x).abs();
            distance(a).total_cmp(&distance(b))
        })
    {
        response.on_hover_text(format!("{}: {}", points[i].label, number(points[i].value, unit)));
    }
}

/// A block of Markdown, ready to be laid out.
#[derive(Debug, Default, PartialEq)]
pub struct Block {
    pub spans: Vec<(String, Inline)>,
    /// Fenced or indented code: framed and monospaced.
    pub code: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Inline {
    pub heading: u8,
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub weak: bool,
}

/// Turn Markdown into styled blocks. HTML is kept as literal text and links
/// are not clickable: only artifact actions may open URLs.
pub fn markdown_blocks(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut block = Block::default();
    let mut style = Inline::default();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut quotes = 0usize;

    fn flush(blocks: &mut Vec<Block>, block: &mut Block) {
        if !block.spans.is_empty() {
            blocks.push(std::mem::take(block));
        }
        block.code = false;
    }
    let push = |block: &mut Block, quotes: usize, text: &str, style: Inline| {
        if block.spans.is_empty() && quotes > 0 {
            block.spans.push(("▎ ".repeat(quotes), Inline { weak: true, ..Inline::default() }));
        }
        block.spans.push((text.to_string(), style));
    };

    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush(&mut blocks, &mut block);
                    style.heading = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        _ => 3,
                    };
                }
                Tag::BlockQuote(_) => {
                    flush(&mut blocks, &mut block);
                    quotes += 1;
                }
                Tag::CodeBlock(kind) => {
                    flush(&mut blocks, &mut block);
                    if let CodeBlockKind::Fenced(language) = kind
                        && !language.is_empty()
                    {
                        let weak = Inline { weak: true, ..Inline::default() };
                        blocks.push(Block { spans: vec![(language.into_string(), weak)], code: false });
                    }
                    block.code = true;
                    style.code = true;
                }
                Tag::List(start) => {
                    flush(&mut blocks, &mut block);
                    lists.push(start);
                }
                Tag::Item => {
                    flush(&mut blocks, &mut block);
                    let indent = "    ".repeat(lists.len().saturating_sub(1));
                    let bullet = match lists.last_mut() {
                        Some(Some(n)) => {
                            *n += 1;
                            format!("{indent}{}. ", *n - 1)
                        }
                        _ => format!("{indent}• "),
                    };
                    push(&mut block, quotes, &bullet, Inline::default());
                }
                Tag::Emphasis => style.emphasis = true,
                Tag::Strong | Tag::TableHead => style.strong = true,
                Tag::Strikethrough => style.strike = true,
                Tag::Link { dest_url, .. } => links.push(dest_url.into_string()),
                Tag::Image { .. } => push(&mut block, quotes, "[image: ", Inline { weak: true, ..style }),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    flush(&mut blocks, &mut block);
                    style.heading = 0;
                }
                TagEnd::Paragraph | TagEnd::Item | TagEnd::TableRow => flush(&mut blocks, &mut block),
                TagEnd::TableHead => {
                    flush(&mut blocks, &mut block);
                    style.strong = false;
                }
                TagEnd::TableCell => push(&mut block, quotes, "    ", style),
                TagEnd::BlockQuote(_) => {
                    flush(&mut blocks, &mut block);
                    quotes = quotes.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some((text, _)) = block.spans.last_mut() {
                        text.truncate(text.trim_end_matches('\n').len());
                    }
                    flush(&mut blocks, &mut block);
                    style.code = false;
                }
                TagEnd::List(_) => {
                    flush(&mut blocks, &mut block);
                    lists.pop();
                }
                TagEnd::Emphasis => style.emphasis = false,
                TagEnd::Strong => style.strong = false,
                TagEnd::Strikethrough => style.strike = false,
                TagEnd::Link => {
                    if let Some(url) = links.pop() {
                        push(&mut block, quotes, &format!(" ({url})"), Inline { weak: true, ..style });
                    }
                }
                TagEnd::Image => push(&mut block, quotes, "]", Inline { weak: true, ..style }),
                _ => {}
            },
            Event::Text(text) => push(&mut block, quotes, &text, style),
            Event::Code(text) => push(&mut block, quotes, &text, Inline { code: true, ..style }),
            Event::Html(text) | Event::InlineHtml(text) => {
                push(&mut block, quotes, &text, Inline { weak: true, ..style })
            }
            Event::SoftBreak => push(&mut block, quotes, " ", style),
            Event::HardBreak => push(&mut block, quotes, "\n", style),
            Event::Rule => {
                flush(&mut blocks, &mut block);
                // An empty block is drawn as a separator line.
                blocks.push(Block::default());
            }
            Event::TaskListMarker(done) => push(&mut block, quotes, if done { "☑ " } else { "☐ " }, style),
            _ => {}
        }
    }
    flush(&mut blocks, &mut block);
    blocks
}

fn markdown(ui: &mut Ui, source: &str) {
    for block in markdown_blocks(source) {
        if block.spans.is_empty() {
            ui.separator();
            continue;
        }
        let mut job = LayoutJob::default();
        for (text, style) in &block.spans {
            let visuals = ui.visuals();
            let size = match style.heading {
                1 => 24.0,
                2 => 20.0,
                3 => 17.0,
                _ => 14.5,
            };
            let mut rich = RichText::new(text).size(size);
            if style.strong || style.heading > 0 {
                rich = rich.strong().color(visuals.strong_text_color());
            }
            if style.emphasis {
                rich = rich.italics();
            }
            if style.strike {
                rich = rich.strikethrough();
            }
            if style.weak {
                rich = rich.weak();
            }
            if style.code {
                rich = rich.monospace().size(13.5);
                if !block.code {
                    rich = rich.code();
                }
            }
            rich.append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
        }
        if block.code {
            egui::Frame::group(ui.style()).fill(ui.visuals().code_bg_color).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(job);
            });
        } else {
            ui.label(job);
        }
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(block: &Block) -> String {
        block.spans.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn markdown_blocks_keep_structure_and_show_html_literally() {
        let source = "# Title\n\nSome **bold** [link](http://x.dev).\n\n- one\n- two\n\n> quoted\n\n```rs\nfn a() {}\n```\n\n<script>x</script>\n";
        let blocks = markdown_blocks(source);
        let lines: Vec<String> = blocks.iter().map(text).collect();
        assert_eq!(
            lines,
            ["Title", "Some bold link (http://x.dev).", "• one", "• two", "▎ quoted", "rs", "fn a() {}", "<script>x</script>\n"]
        );
        assert_eq!(blocks[0].spans[0].1.heading, 1);
        assert!(blocks[1].spans[1].1.strong);
        assert!(blocks[6].code && !blocks[5].code);
    }

    #[test]
    fn chart_numbers_drop_trailing_zeros() {
        assert_eq!(number(38.4, "s"), "38.4s");
        assert_eq!(number(12.0, ""), "12");
        assert_eq!(number(0.125, "ms"), "0.12ms");
    }
}
