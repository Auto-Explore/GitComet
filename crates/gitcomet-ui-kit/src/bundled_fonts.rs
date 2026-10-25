use gpui::{App, Result};
use std::borrow::Cow;

pub use gitcomet_fonts::{
    BUNDLED_FONT_BYTES, FIRA_CODE_FONT_FAMILY, IBM_PLEX_SANS_FONT_FAMILY, LILEX_FONT_FAMILY,
};

const FILTERED_FONT_ALIASES: &[&str] = &[
    "Fira Code SemiBold",
    "IBM Plex Sans SmBld",
    "IBM Plex Sans SemiBold",
    "Lilex Italic",
    "Lilex SemiBold",
    "Lilex SemiBold Italic",
];

pub fn register(cx: &mut App) -> Result<()> {
    cx.text_system().add_fonts(
        BUNDLED_FONT_BYTES
            .iter()
            .map(|bytes| Cow::Borrowed(*bytes))
            .collect(),
    )
}

pub fn load_into_fontdb(db: &mut fontdb::Database) {
    for bytes in BUNDLED_FONT_BYTES {
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(*bytes)));
    }
}

pub fn should_skip_font_option_alias(font_family: &str) -> bool {
    FILTERED_FONT_ALIASES.contains(&font_family)
}

/// The monospace family UI text uses (paths, hashes, shortcuts).
pub const UI_MONOSPACE_FONT_FAMILY: &str = LILEX_FONT_FAMILY;
