//! QR codes drawn with Unicode blocks.
//!
//! The code is always dark-on-light with a quiet zone, whatever the terminal
//! theme, so phone cameras can read it. Two sizes are tried: 2x1 cells per
//! module (larger, easier to scan) and half-block (1 cell = 1x2 modules).

use qrcode::{Color as Module, EcLevel, QrCode};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget, Wrap},
};

/// Preferred light border around the code, in modules.
const QUIET: usize = 2;
/// Used when the preferred border does not fit; phones still read it.
const QUIET_TIGHT: usize = 1;

/// Module matrix including the quiet zone. `true` is dark.
pub struct Matrix {
    pub side: usize,
    cells: Vec<bool>,
}

impl Matrix {
    pub fn encode(data: &str) -> Result<Self, String> {
        Self::encode_with_quiet(data, QUIET)
    }

    pub fn encode_with_quiet(data: &str, quiet: usize) -> Result<Self, String> {
        Self::encode_with(data, quiet, EcLevel::M)
    }

    /// Lower error correction gives a smaller code for the same data, which
    /// is a fair trade on a screen (no dirt or damage to recover from).
    pub fn encode_with(data: &str, quiet: usize, level: EcLevel) -> Result<Self, String> {
        let code = QrCode::with_error_correction_level(data.as_bytes(), level).map_err(|e| e.to_string())?;
        let width = code.width();
        let colors = code.to_colors();
        let side = width + 2 * quiet;
        let mut cells = vec![false; side * side];
        for y in 0..width {
            for x in 0..width {
                cells[(y + quiet) * side + x + quiet] = colors[y * width + x] == Module::Dark;
            }
        }
        Ok(Self { side, cells })
    }

    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < self.side && y < self.side && self.cells[y * self.side + x]
    }
}

/// Columns and rows needed for the compact (half-block) rendering.
pub fn compact_size(side: usize) -> (u16, u16) {
    (side as u16, side.div_ceil(2) as u16)
}

pub fn render(data: &str, area: Rect, buf: &mut Buffer) {
    let matrix = match Matrix::encode(data) {
        Ok(m) => m,
        Err(e) => {
            Paragraph::new(format!("Cannot encode QR: {e}")).style(Style::new().fg(Color::Red)).render(area, buf);
            return;
        }
    };
    // If the code does not fit as is, try progressively smaller variants
    // before giving up: a thinner border, then lower error correction.
    let fits = |m: &Matrix| {
        let (cw, ch) = compact_size(m.side);
        area.width >= cw && area.height >= ch
    };
    let mut matrix = matrix;
    for (quiet, level) in [(QUIET_TIGHT, EcLevel::M), (QUIET, EcLevel::L), (QUIET_TIGHT, EcLevel::L)] {
        if fits(&matrix) {
            break;
        }
        if let Ok(smaller) = Matrix::encode_with(data, quiet, level)
            && smaller.side <= matrix.side
        {
            matrix = smaller;
        }
    }
    let side = matrix.side as u16;
    let (cw, ch) = compact_size(matrix.side);
    // Keep one row for the caption when there is room.
    let caption = area.height > ch;
    let avail_h = area.height - u16::from(caption);

    let (w, h, large) = if area.width >= side * 2 && avail_h >= side {
        (side * 2, side, true)
    } else if area.width >= cw && avail_h >= ch {
        (cw, ch, false)
    } else {
        let msg = format!("QR needs {cw}x{ch} cells, this pane has {}x{}. Press Enter to enlarge.\n{data}", area.width, area.height);
        Paragraph::new(msg).style(Style::new().fg(Color::Yellow)).wrap(Wrap { trim: true }).render(area, buf);
        return;
    };

    let x0 = area.x + (area.width - w) / 2;
    let y0 = area.y + (avail_h - h) / 2;
    let style = Style::new().fg(Color::Black).bg(Color::White);
    for row in 0..h {
        for col in 0..w {
            let symbol = if large {
                if matrix.dark(usize::from(col / 2), usize::from(row)) { "█" } else { " " }
            } else {
                let top = matrix.dark(usize::from(col), usize::from(row) * 2);
                let bottom = matrix.dark(usize::from(col), usize::from(row) * 2 + 1);
                match (top, bottom) {
                    (true, true) => "█",
                    (true, false) => "▀",
                    (false, true) => "▄",
                    (false, false) => " ",
                }
            };
            buf[(x0 + col, y0 + row)].set_symbol(symbol).set_style(style);
        }
    }
    if caption {
        let line = Line::from(data.to_string()).centered().style(Style::new().add_modifier(Modifier::DIM));
        line.render(Rect { x: area.x, y: y0 + h, width: area.width, height: 1 }, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::buffer_text;

    const DATA: &str = "exp://192.168.1.100:8081";

    #[test]
    fn matrix_has_quiet_zone_and_finder_pattern() {
        let m = Matrix::encode(DATA).unwrap();
        assert_eq!(m.side, 25 + 2 * QUIET, "version 2 code plus quiet zone");
        for i in 0..m.side {
            assert!(!m.dark(i, 0) && !m.dark(0, i) && !m.dark(i, m.side - 1) && !m.dark(m.side - 1, i));
        }
        // Top-left finder: 7 dark modules in a row, then a light separator.
        assert!((0..7).all(|i| m.dark(QUIET + i, QUIET)));
        assert!(!m.dark(QUIET + 7, QUIET));
        assert!(!m.dark(QUIET + 1, QUIET + 1));
        assert!(m.dark(QUIET + 3, QUIET + 3));
    }

    /// Decode the half-block rendering back to modules and compare.
    #[test]
    fn compact_rendering_matches_the_matrix() {
        let m = Matrix::encode(DATA).unwrap();
        let (w, h) = compact_size(m.side);
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render(DATA, area, &mut buf);
        for y in 0..m.side {
            for x in 0..m.side {
                let cell = &buf[(x as u16, (y / 2) as u16)];
                let dark = match cell.symbol() {
                    "█" => true,
                    "▀" => y % 2 == 0,
                    "▄" => y % 2 == 1,
                    _ => false,
                };
                assert_eq!(dark, m.dark(x, y), "module {x},{y}");
                assert_eq!((cell.fg, cell.bg), (Color::Black, Color::White), "scannable colors");
            }
        }
    }

    #[test]
    fn large_rendering_and_caption_when_there_is_room() {
        let m = Matrix::encode(DATA).unwrap();
        let side = m.side as u16;
        let area = Rect::new(0, 0, side * 2 + 4, side + 1);
        let mut buf = Buffer::empty(area);
        render(DATA, area, &mut buf);
        // Finder corner is two cells wide per module at x offset 2 (centering).
        assert_eq!(buf[(2 + QUIET as u16 * 2, QUIET as u16)].symbol(), "█");
        assert_eq!(buf[(2 + QUIET as u16 * 2 + 1, QUIET as u16)].symbol(), "█");
        assert!(buffer_text(&buf).contains(DATA), "caption shows the encoded data");
    }

    #[test]
    fn too_small_area_explains_instead_of_drawing_garbage() {
        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render(DATA, area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("QR needs 27x14 cells"), "reports the smallest size that would work\n{text}");
        assert!(!text.contains('█'));
    }

    #[test]
    fn lower_error_correction_is_used_when_the_code_is_too_tall() {
        // 43 bytes: version 4 (33 modules) at level M, version 3 (29) at level L.
        let url = "https://github.com/jonarroh/herdr-artifacts";
        assert_eq!(Matrix::encode_with(url, 0, EcLevel::M).unwrap().side, 33);
        assert_eq!(Matrix::encode_with(url, 0, EcLevel::L).unwrap().side, 29);
        let area = Rect::new(0, 0, 45, 16);
        let mut buf = Buffer::empty(area);
        render(url, area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains("QR needs") && text.contains("█"), "{text}");
    }

    #[test]
    fn border_shrinks_when_one_row_short() {
        // 29x15 is needed with the preferred border; 14 rows only fit the tight one.
        let area = Rect::new(0, 0, 44, 14);
        let mut buf = Buffer::empty(area);
        render(DATA, area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains("QR needs"), "{text}");
        let tight = Matrix::encode_with_quiet(DATA, QUIET_TIGHT).unwrap();
        assert_eq!(compact_size(tight.side), (27, 14));
        // Finder pattern starts one module in: its top edge is the lower half of row 0.
        let x0 = (44 - 27) / 2 + 1;
        assert_eq!(buf[(x0, 0)].symbol(), "▄");
        assert_eq!(buf[(x0 - 1, 0)].symbol(), " ", "light border is kept");
        assert_eq!(buf[(x0 - 1, 0)].bg, Color::White);
    }
}
