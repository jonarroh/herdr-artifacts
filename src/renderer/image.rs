//! Image rendering with a small bounded cache.
//!
//! Images are decoded lazily (only when an image artifact is on screen),
//! shrunk to the target area, and encoded once for the detected protocol
//! (Kitty / Sixel / iTerm2, or Unicode half-blocks). Only the encoded
//! result is kept; the decoded bitmap is dropped immediately. The cache is
//! an LRU of a few entries, so leaving an artifact frees its memory.

use std::{collections::VecDeque, fs, path::Path};

use ::image::{DynamicImage, ImageReader, Limits as DecodeLimits};
use ratatui::{
    buffer::Buffer,
    layout::{Rect, Size},
    style::{Color, Style},
    widgets::{Paragraph, Widget, Wrap},
};
use ratatui_image::{
    Image, Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};

use crate::{config::Config, protocol::artifact::is_store_file_name};

/// Upper bound for a decoder allocation, independent of the file size.
const MAX_DECODE_ALLOC: u64 = 128 * 1024 * 1024;

type Key = (String, u16, u16);

pub struct ImageCache {
    picker: Picker,
    capacity: usize,
    entries: VecDeque<(Key, Result<Protocol, String>)>,
}

impl ImageCache {
    pub fn new(picker: Picker, capacity: usize) -> Self {
        Self { picker, capacity: capacity.max(1), entries: VecDeque::new() }
    }

    pub fn protocol_type(&self) -> ProtocolType {
        self.picker.protocol_type()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop every cached image (e.g. after the terminal was resized).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn render(&mut self, cfg: &Config, source: &str, area: Rect, buf: &mut Buffer) {
        let key: Key = (source.to_string(), area.width, area.height);
        let index = match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let loaded = load(cfg, &self.picker, source, area).map_err(|e| e.to_string());
                self.entries.push_back((key, loaded));
                while self.entries.len() > self.capacity {
                    self.entries.pop_front();
                }
                self.entries.len() - 1
            }
        };
        // Most recently used goes to the back.
        let entry = self.entries.remove(index).expect("index is valid");
        match &entry.1 {
            Ok(protocol) => {
                let size = protocol.size();
                let rect = Rect {
                    x: area.x + area.width.saturating_sub(size.width) / 2,
                    y: area.y + area.height.saturating_sub(size.height) / 2,
                    width: size.width.min(area.width),
                    height: size.height.min(area.height),
                };
                Image::new(protocol).render(rect, buf);
            }
            Err(message) => Paragraph::new(format!("Cannot display image: {message}"))
                .style(Style::new().fg(Color::Red))
                .wrap(Wrap { trim: true })
                .render(area, buf),
        }
        self.entries.push_back(entry);
    }
}

fn load(cfg: &Config, picker: &Picker, source: &str, area: Rect) -> Result<Protocol, Box<dyn std::error::Error>> {
    // The source is re-validated here: nothing outside `files/` is ever opened.
    if !is_store_file_name(source) {
        return Err("invalid image source".into());
    }
    let image = decode(&cfg.files_dir().join(source), cfg.limits.max_file_bytes, cfg.limits.max_image_dimension)?;
    Ok(encode(picker, image, area)?)
}

/// Decode an image file with strict size limits.
pub fn decode(path: &Path, max_bytes: u64, max_dimension: u32) -> Result<DynamicImage, Box<dyn std::error::Error>> {
    let len = fs::metadata(path)?.len();
    if len > max_bytes {
        return Err(format!("file is {len} bytes, limit is {max_bytes}").into());
    }
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = DecodeLimits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// Shrink `image` to roughly the pixel size of `area`, then encode it for
/// the picker's protocol, fitted inside `area`.
pub fn encode(picker: &Picker, image: DynamicImage, area: Rect) -> Result<Protocol, ratatui_image::errors::Errors> {
    let font = picker.font_size();
    let max_w = u32::from(area.width) * u32::from(font.width);
    let max_h = u32::from(area.height) * u32::from(font.height);
    // `thumbnail` is a fast box filter; the precise fit is done by the protocol.
    let image = if image.width() > max_w * 2 || image.height() > max_h * 2 {
        image.thumbnail(max_w * 2, max_h * 2)
    } else {
        image
    };
    // `Scale` also enlarges small images, so the expanded view is actually used.
    picker.new_protocol(image, Size::new(area.width, area.height), Resize::Scale(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::{Rgb, RgbImage};

    fn gradient(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 { Rgb([230, 40, 40]) } else { Rgb([40, 40, 230]) }
        }))
    }

    #[test]
    fn unicode_fallback_draws_halfblocks_with_colors() {
        let picker = Picker::halfblocks();
        let area = Rect::new(0, 0, 20, 10);
        let protocol = encode(&picker, gradient(64, 64), area).unwrap();
        assert!(matches!(protocol, Protocol::Halfblocks(_)));
        let size = protocol.size();
        assert!(size.width <= 20 && size.height <= 10, "fits the area: {size:?}");

        let mut buf = Buffer::empty(area);
        Image::new(&protocol).render(Rect::new(0, 0, size.width, size.height), &mut buf);
        // A cell is a half-block (two colours) or, when both halves match, a
        // space painted with the background colour.
        let cells: Vec<_> = (0..size.height).flat_map(|y| (0..size.width).map(move |x| (x, y))).map(|p| &buf[p]).collect();
        assert!(cells.iter().all(|c| matches!(c.symbol(), "▀" | "▄" | " ")));
        assert!(cells.iter().any(|c| matches!(c.symbol(), "▀" | "▄")), "edges use half-blocks");
        assert!(cells.iter().all(|c| matches!(c.bg, Color::Rgb(..))), "fallback keeps real colors");
    }

    #[test]
    fn large_images_are_scaled_down_to_the_area() {
        let picker = Picker::halfblocks();
        let area = Rect::new(0, 0, 30, 8);
        let protocol = encode(&picker, gradient(2000, 1000), area).unwrap();
        let size = protocol.size();
        assert!(size.width <= 30 && size.height <= 8, "{size:?}");
        assert!(size.width >= 20, "keeps aspect ratio and uses the space: {size:?}");
    }

    /// No terminal in tests can display them, but every real-image protocol
    /// must at least encode and render into the buffer without failing.
    #[test]
    fn real_image_protocols_encode() {
        let area = Rect::new(0, 0, 20, 10);
        for kind in [ProtocolType::Kitty, ProtocolType::Sixel, ProtocolType::Iterm2] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(kind);
            let protocol = encode(&picker, gradient(64, 64), area).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            assert!(!matches!(protocol, Protocol::Halfblocks(_)), "{kind:?} uses its own encoding");
            let size = protocol.size();
            assert!(size.width > 0 && size.width <= 20 && size.height <= 10, "{kind:?}: {size:?}");
            let mut buf = Buffer::empty(area);
            Image::new(&protocol).render(Rect::new(0, 0, size.width, size.height), &mut buf);
        }
    }

    #[test]
    fn small_images_are_enlarged_to_use_the_area() {
        let picker = Picker::halfblocks();
        // 40x40 px is 4x2 cells at the assumed 10x20 cell size.
        let protocol = encode(&picker, gradient(40, 40), Rect::new(0, 0, 60, 20)).unwrap();
        let size = protocol.size();
        assert!(size.height >= 18 && size.width <= 60, "fills the height, keeps the aspect ratio: {size:?}");
    }

    #[test]
    fn cache_is_bounded_and_reports_errors() {
        let dir = std::env::temp_dir().join(format!("ha-img-{}", crate::storage::random_hex(6)));
        let cfg = Config::with_home(&dir).unwrap();
        cfg.ensure_dirs().unwrap();
        for i in 0..3 {
            gradient(32, 32).save(cfg.files_dir().join(format!("img-{i}.png"))).unwrap();
        }
        fs::write(cfg.files_dir().join("broken.png"), b"not an image").unwrap();

        let mut cache = ImageCache::new(Picker::halfblocks(), 2);
        let area = Rect::new(0, 0, 16, 8);
        let mut buf = Buffer::empty(area);
        for i in 0..3 {
            cache.render(&cfg, &format!("img-{i}.png"), area, &mut buf);
        }
        assert_eq!(cache.len(), 2, "oldest entry was evicted");

        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 3));
        cache.render(&cfg, "broken.png", buf.area, &mut buf);
        assert!(crate::renderer::buffer_text(&buf).contains("Cannot display image"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 3));
        cache.render(&cfg, "../outside.png", buf.area, &mut buf);
        assert!(crate::renderer::buffer_text(&buf).contains("invalid image source"));
        let _ = fs::remove_dir_all(dir);
    }
}
