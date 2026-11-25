//! Colours and widget styling for the native window, in light and dark. The
//! window follows the system theme.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, Visuals, vec2,
};

use crate::protocol::Level;

pub const RADIUS: u8 = 8;
pub const CARD_RADIUS: u8 = 14;
/// Space between the window edge and its content.
pub const GUTTER: i8 = 18;

struct Palette {
    background: Color32,
    card: Color32,
    raised: Color32,
    hover: Color32,
    border: Color32,
    text: Color32,
    strong: Color32,
    accent: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const DARK: Palette = Palette {
    background: rgb(0x14151b),
    card: rgb(0x1d1f27),
    raised: rgb(0x282b36),
    hover: rgb(0x343846),
    border: rgb(0x2e313d),
    text: rgb(0xd5d8e2),
    strong: rgb(0xf6f7fb),
    accent: rgb(0x6c7cf5),
};

const LIGHT: Palette = Palette {
    background: rgb(0xf3f4f8),
    card: rgb(0xffffff),
    raised: rgb(0xe9ebf2),
    hover: rgb(0xdde0ea),
    border: rgb(0xe0e3ec),
    text: rgb(0x34384a),
    strong: rgb(0x141724),
    accent: rgb(0x4a5ae0),
};

pub fn apply(ctx: &egui::Context) {
    for (theme, palette) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
        ctx.style_mut_of(theme, |style| {
            style.visuals = visuals(theme, palette);
            style.spacing.item_spacing = vec2(10.0, 8.0);
            style.spacing.button_padding = vec2(14.0, 7.0);
            style.spacing.interact_size.y = 28.0;
            style.spacing.menu_margin = Margin::same(8);
            for (name, size, family) in [
                (TextStyle::Heading, 21.0, FontFamily::Proportional),
                (TextStyle::Body, 15.0, FontFamily::Proportional),
                (TextStyle::Button, 14.5, FontFamily::Proportional),
                (TextStyle::Small, 12.0, FontFamily::Proportional),
                (TextStyle::Monospace, 13.5, FontFamily::Monospace),
            ] {
                style.text_styles.insert(name, FontId::new(size, family));
            }
        });
    }
}

fn visuals(theme: Theme, p: &Palette) -> Visuals {
    let mut v = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.background;
    v.window_fill = p.card;
    v.faint_bg_color = p.raised.gamma_multiply(0.55);
    v.extreme_bg_color = p.background;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.bg_fill = p.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke.color = p.text;
    for (widget, fill) in [
        (&mut v.widgets.inactive, p.raised),
        (&mut v.widgets.hovered, p.hover),
        (&mut v.widgets.active, p.hover),
        (&mut v.widgets.open, p.hover),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke.color = p.strong;
        widget.expansion = 0.0;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.7));
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(RADIUS);
    }
    v
}

/// The accent of each artifact type, readable on both themes.
pub fn kind_color(kind: &str) -> Color32 {
    match kind {
        "message" => rgb(0x5b8def),
        "qr" => rgb(0x9b72e8),
        "image" => rgb(0xd9669d),
        "markdown" => rgb(0x2fa4a9),
        "code" => rgb(0xd98a3d),
        "table" => rgb(0x45a869),
        "chart" => rgb(0x6c7cf5),
        _ => rgb(0x8a90a0),
    }
}

pub fn level_color(level: Level) -> Color32 {
    match level {
        Level::Info => rgb(0x5b8def),
        Level::Success => rgb(0x3dab66),
        Level::Warning => rgb(0xd99a1e),
        Level::Error => rgb(0xe05563),
    }
}

/// The surface the artifact is drawn on.
pub fn card(visuals: &Visuals) -> egui::Frame {
    egui::Frame::new()
        .fill(visuals.window_fill)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(GUTTER))
}

/// A small rounded label tinted with `color`.
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    let font = TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    // Exactly as big as its text, so centred layouts can centre it.
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(22.0, 8.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    ui.painter().rect_filled(rect, rect.height() / 2.0, color.gamma_multiply(0.18));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
}
