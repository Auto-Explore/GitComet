use gpui::{App, Result};
use std::borrow::Cow;

pub const FIRA_CODE_FONT_FAMILY: &str = "Fira Code";
pub const IBM_PLEX_SANS_FONT_FAMILY: &str = "IBM Plex Sans";
pub const LILEX_FONT_FAMILY: &str = "Lilex";

const FIRA_CODE_REGULAR_BYTES: &[u8] =
    include_bytes!("../assets/fonts/fira_code/FiraCode-Regular.ttf");
const FIRA_CODE_SEMIBOLD_BYTES: &[u8] =
    include_bytes!("../assets/fonts/fira_code/FiraCode-SemiBold.ttf");
const FIRA_CODE_BOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/fira_code/FiraCode-Bold.ttf");

const IBM_PLEX_SANS_REGULAR_BYTES: &[u8] =
    include_bytes!("../assets/fonts/ibm_plex_sans/IBMPlexSans-Regular.ttf");
const IBM_PLEX_SANS_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/fonts/ibm_plex_sans/IBMPlexSans-Italic.ttf");
const IBM_PLEX_SANS_SEMIBOLD_BYTES: &[u8] =
    include_bytes!("../assets/fonts/ibm_plex_sans/IBMPlexSans-SemiBold.ttf");
const IBM_PLEX_SANS_SEMIBOLD_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/fonts/ibm_plex_sans/IBMPlexSans-SemiBoldItalic.ttf");
const IBM_PLEX_SANS_BOLD_BYTES: &[u8] =
    include_bytes!("../assets/fonts/ibm_plex_sans/IBMPlexSans-Bold.ttf");
const IBM_PLEX_SANS_BOLD_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/fonts/ibm_plex_sans/IBMPlexSans-BoldItalic.ttf");

const LILEX_REGULAR_BYTES: &[u8] = include_bytes!("../assets/fonts/lilex/Lilex-Regular.ttf");
const LILEX_ITALIC_BYTES: &[u8] = include_bytes!("../assets/fonts/lilex/Lilex-Italic.ttf");
const LILEX_SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/lilex/Lilex-SemiBold.ttf");
const LILEX_SEMIBOLD_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/fonts/lilex/Lilex-SemiBoldItalic.ttf");
const LILEX_BOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/lilex/Lilex-Bold.ttf");
const LILEX_BOLD_ITALIC_BYTES: &[u8] = include_bytes!("../assets/fonts/lilex/Lilex-BoldItalic.ttf");

const BUNDLED_FONT_BYTES: &[&[u8]] = &[
    FIRA_CODE_REGULAR_BYTES,
    FIRA_CODE_SEMIBOLD_BYTES,
    FIRA_CODE_BOLD_BYTES,
    IBM_PLEX_SANS_REGULAR_BYTES,
    IBM_PLEX_SANS_ITALIC_BYTES,
    IBM_PLEX_SANS_SEMIBOLD_BYTES,
    IBM_PLEX_SANS_SEMIBOLD_ITALIC_BYTES,
    IBM_PLEX_SANS_BOLD_BYTES,
    IBM_PLEX_SANS_BOLD_ITALIC_BYTES,
    LILEX_REGULAR_BYTES,
    LILEX_ITALIC_BYTES,
    LILEX_SEMIBOLD_BYTES,
    LILEX_SEMIBOLD_ITALIC_BYTES,
    LILEX_BOLD_BYTES,
    LILEX_BOLD_ITALIC_BYTES,
];

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
        db.load_font_data(bytes.to_vec());
    }
}

pub fn should_skip_font_option_alias(font_family: &str) -> bool {
    FILTERED_FONT_ALIASES.contains(&font_family)
}

/// The monospace family UI text uses (paths, hashes, shortcuts).
pub const UI_MONOSPACE_FONT_FAMILY: &str = LILEX_FONT_FAMILY;
