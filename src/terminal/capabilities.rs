//! Terminal capability detection.
//!
//! `ratatui-image` queries the terminal (Kitty graphics, Sixel, iTerm2 and
//! the cell size in pixels) and falls back to Unicode half-blocks when the
//! terminal does not answer. We add an explicit override and a summary that
//! the header and `doctor` can show.

use std::env;

use ratatui_image::picker::{Picker, ProtocolType};

/// `HERDR_ARTIFACTS_IMAGE_PROTOCOL` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolChoice {
    Auto,
    Force(ProtocolType),
}

pub fn parse_choice(value: Option<&str>) -> ProtocolChoice {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("halfblocks" | "unicode" | "blocks") => ProtocolChoice::Force(ProtocolType::Halfblocks),
        Some("kitty") => ProtocolChoice::Force(ProtocolType::Kitty),
        Some("sixel") => ProtocolChoice::Force(ProtocolType::Sixel),
        Some("iterm2" | "iterm") => ProtocolChoice::Force(ProtocolType::Iterm2),
        _ => ProtocolChoice::Auto,
    }
}

/// Pick the protocol to use given the user's choice and what was detected.
pub fn resolve(choice: ProtocolChoice, detected: ProtocolType) -> ProtocolType {
    match choice {
        ProtocolChoice::Auto => detected,
        ProtocolChoice::Force(p) => p,
    }
}

pub fn protocol_name(p: ProtocolType) -> &'static str {
    match p {
        ProtocolType::Halfblocks => "halfblocks",
        ProtocolType::Sixel => "sixel",
        ProtocolType::Kitty => "kitty",
        ProtocolType::Iterm2 => "iterm2",
    }
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub protocol: ProtocolType,
    /// Cell size in pixels (a 10x20 guess when the terminal did not report it).
    pub font_size: (u16, u16),
    /// The terminal answered the capability query.
    pub answered: bool,
    /// The protocol was forced through the environment.
    pub forced: bool,
    pub truecolor: bool,
    /// `TERM_PROGRAM`, e.g. `herdr`.
    pub host: Option<String>,
}

impl Capabilities {
    pub fn real_images(&self) -> bool {
        self.protocol != ProtocolType::Halfblocks
    }
}

/// Query the terminal. Must run after entering the alternate screen and
/// before reading input events (the query reads its answer from stdin).
pub fn detect() -> (Picker, Capabilities) {
    let choice = parse_choice(env::var("HERDR_ARTIFACTS_IMAGE_PROTOCOL").ok().as_deref());
    let mut picker = match choice {
        // No need to talk to the terminal when half-blocks are forced.
        ProtocolChoice::Force(ProtocolType::Halfblocks) => Picker::halfblocks(),
        _ => Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
    };
    let answered = !picker.capabilities().is_empty();
    picker.set_protocol_type(resolve(choice, picker.protocol_type()));
    let font = picker.font_size();
    let caps = Capabilities {
        protocol: picker.protocol_type(),
        font_size: (font.width, font.height),
        answered,
        forced: choice != ProtocolChoice::Auto,
        truecolor: env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit")),
        host: env::var("TERM_PROGRAM").ok().filter(|v| !v.is_empty()),
    };
    (picker, caps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_parsing() {
        assert_eq!(parse_choice(None), ProtocolChoice::Auto);
        assert_eq!(parse_choice(Some("auto")), ProtocolChoice::Auto);
        assert_eq!(parse_choice(Some("nonsense")), ProtocolChoice::Auto);
        assert_eq!(parse_choice(Some(" Kitty ")), ProtocolChoice::Force(ProtocolType::Kitty));
        assert_eq!(parse_choice(Some("unicode")), ProtocolChoice::Force(ProtocolType::Halfblocks));
        assert_eq!(parse_choice(Some("SIXEL")), ProtocolChoice::Force(ProtocolType::Sixel));
        assert_eq!(parse_choice(Some("iterm2")), ProtocolChoice::Force(ProtocolType::Iterm2));
    }

    #[test]
    fn resolution() {
        assert_eq!(resolve(ProtocolChoice::Auto, ProtocolType::Kitty), ProtocolType::Kitty);
        assert_eq!(resolve(ProtocolChoice::Auto, ProtocolType::Halfblocks), ProtocolType::Halfblocks);
        assert_eq!(
            resolve(ProtocolChoice::Force(ProtocolType::Halfblocks), ProtocolType::Kitty),
            ProtocolType::Halfblocks
        );
    }

    #[test]
    fn halfblocks_picker_is_the_safe_default() {
        let picker = Picker::halfblocks();
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);
        assert!(picker.capabilities().is_empty());
        assert_eq!(protocol_name(picker.protocol_type()), "halfblocks");
    }
}
