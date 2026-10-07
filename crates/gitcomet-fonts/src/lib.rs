//! Font data shared by native layout, SVG rasterization and the UI.
pub const FIRA_CODE_FONT_FAMILY: &str = "Fira Code";
pub const IBM_PLEX_SANS_FONT_FAMILY: &str = "IBM Plex Sans";
pub const LILEX_FONT_FAMILY: &str = "Lilex";

const FIRA_CODE_REGULAR_BYTES: &[u8] = include_bytes!("../assets/fira_code/FiraCode-Regular.ttf");
const FIRA_CODE_SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/fira_code/FiraCode-SemiBold.ttf");
const FIRA_CODE_BOLD_BYTES: &[u8] = include_bytes!("../assets/fira_code/FiraCode-Bold.ttf");

const IBM_PLEX_SANS_REGULAR_BYTES: &[u8] =
    include_bytes!("../assets/ibm_plex_sans/IBMPlexSans-Regular.ttf");
const IBM_PLEX_SANS_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/ibm_plex_sans/IBMPlexSans-Italic.ttf");
const IBM_PLEX_SANS_SEMIBOLD_BYTES: &[u8] =
    include_bytes!("../assets/ibm_plex_sans/IBMPlexSans-SemiBold.ttf");
const IBM_PLEX_SANS_SEMIBOLD_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/ibm_plex_sans/IBMPlexSans-SemiBoldItalic.ttf");
const IBM_PLEX_SANS_BOLD_BYTES: &[u8] =
    include_bytes!("../assets/ibm_plex_sans/IBMPlexSans-Bold.ttf");
const IBM_PLEX_SANS_BOLD_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/ibm_plex_sans/IBMPlexSans-BoldItalic.ttf");

const LILEX_REGULAR_BYTES: &[u8] = include_bytes!("../assets/lilex/Lilex-Regular.ttf");
const LILEX_ITALIC_BYTES: &[u8] = include_bytes!("../assets/lilex/Lilex-Italic.ttf");
const LILEX_SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/lilex/Lilex-SemiBold.ttf");
const LILEX_SEMIBOLD_ITALIC_BYTES: &[u8] =
    include_bytes!("../assets/lilex/Lilex-SemiBoldItalic.ttf");
const LILEX_BOLD_BYTES: &[u8] = include_bytes!("../assets/lilex/Lilex-Bold.ttf");
const LILEX_BOLD_ITALIC_BYTES: &[u8] = include_bytes!("../assets/lilex/Lilex-BoldItalic.ttf");

pub const BUNDLED_FONT_BYTES: &[&[u8]] = &[
    include_bytes!("../assets/noto_emoji/NotoEmoji.ttf"),
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
