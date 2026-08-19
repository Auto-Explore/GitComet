use gpui::Hsla;
use gpui::Rgba;
use gpui::WindowAppearance;
use palette::IntoColor;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub(crate) const DEFAULT_DARK_THEME_KEY: &str = "gitcomet_dark";
pub(crate) const DEFAULT_LIGHT_THEME_KEY: &str = "gitcomet_light";
pub(crate) const GRAPH_LANE_PALETTE_SIZE: usize = 64;
pub(crate) const THEME_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ThemeOption {
    pub key: String,
    pub label: String,
}

struct EmbeddedThemeFile {
    stem: &'static str,
    json: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/embedded_themes.rs"));

static EMBEDDED_THEME_CACHE: OnceLock<FxHashMap<String, RuntimeThemeSpec>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppTheme {
    pub is_dark: bool,
    pub colors: Colors,
    pub syntax: SyntaxColors,
    /// Interned rather than inlined: the palette is 64 colours, and `AppTheme` is
    /// `Copy` and captured by value into every per-row paint closure. Carrying the
    /// array here made each of those closures a kilobyte heavier for data every
    /// theme shares. See [`intern_lane_palette`].
    pub graph_lane_palette: &'static GraphLanePalette,
    pub radii: Radii,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub surface: SurfaceColors,
    pub foreground: ForegroundColors,
    pub stroke: StrokeColors,
    pub interaction: InteractionColors,
    pub accent: AccentColors,
    pub status: StatusColors,
    pub editor: EditorColors,
    pub diff: DiffColors,
    pub tooltip: TooltipColors,
    pub scrollbar: ScrollbarColors,
    pub shadow: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceColors {
    /// Main editor, diff, merge, and history canvas.
    pub canvas: Rgba,
    /// Window chrome around the main canvas: title/action/sidebar/status bands.
    pub chrome: Rgba,
    pub panel: Rgba,
    pub raised: Rgba,
    pub input: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForegroundColors {
    pub primary: Rgba,
    pub secondary: Rgba,
    pub disabled: Rgba,
    pub placeholder: Rgba,
    pub emphasis: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeColors {
    /// Quiet, decorative separators that are not needed to identify a control.
    pub subtle: Rgba,
    pub default: Rgba,
    /// Necessary control boundaries; bundled light themes keep this at 3:1.
    pub control: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionColors {
    pub hover_overlay: Rgba,
    pub pressed_overlay: Rgba,
    pub hover_background: Rgba,
    pub pressed_background: Rgba,
    pub selected_background: Rgba,
    pub selected_foreground: Rgba,
    pub selected_indicator: Rgba,
    pub focus_ring: Rgba,
    pub focus_background: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccentColors {
    pub foreground: Rgba,
    pub solid: Rgba,
    pub on_solid: Rgba,
    pub subtle_background: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusColorSet {
    pub foreground: Rgba,
    pub background: Rgba,
    pub border: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusColors {
    pub info: StatusColorSet,
    pub success: StatusColorSet,
    pub warning: StatusColorSet,
    pub danger: StatusColorSet,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorColors {
    pub background: Rgba,
    pub foreground: Rgba,
    pub gutter_background: Rgba,
    pub line_number: Rgba,
    pub cursor: Rgba,
    pub selection_background: Rgba,
    pub search_match_background: Rgba,
    pub search_match_foreground: Rgba,
    pub bracket_match_background: Rgba,
    pub indent_guide: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColorSet {
    pub foreground: Rgba,
    pub background: Rgba,
    pub word_background: Rgba,
    /// The row under the keyboard focus. `modified` has no reader: a diff row is
    /// an add or a remove, and the split view draws a modification as one of
    /// each. It stays here because the three sets are one shape -- a schema with
    /// a hole in exactly one of them costs an author more than an unused token
    /// does.
    pub focused_background: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColors {
    pub added: DiffColorSet,
    pub removed: DiffColorSet,
    pub modified: DiffColorSet,
}

/// Which of the three diff palettes a piece of diff decoration belongs to.
///
/// Carried alongside the decoration instead of one colour out of the set, so a
/// renderer that needs a second colour from the same palette (the word wash, the
/// focused-row background) can ask for it rather than trying to recognise the
/// set from a colour it was handed -- a theme is free to paint two of the three
/// kinds the same hue, and then recognising it is guesswork.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiffColorKind {
    Added,
    Removed,
    Modified,
}

impl DiffColors {
    pub fn set(self, kind: DiffColorKind) -> DiffColorSet {
        match kind {
            DiffColorKind::Added => self.added,
            DiffColorKind::Removed => self.removed,
            DiffColorKind::Modified => self.modified,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipColors {
    pub background: Rgba,
    pub foreground: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarColors {
    pub thumb: Rgba,
    pub thumb_hover: Rgba,
    pub thumb_pressed: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxColors {
    pub comment: Rgba,
    pub comment_doc: Rgba,
    pub string: Rgba,
    pub string_escape: Rgba,
    pub string_regex: Rgba,
    pub string_special: Rgba,
    pub keyword: Rgba,
    pub keyword_control: Rgba,
    pub preproc: Rgba,
    pub number: Rgba,
    pub boolean: Rgba,
    pub function: Rgba,
    pub function_method: Rgba,
    pub function_special: Rgba,
    pub constructor: Rgba,
    pub type_name: Rgba,
    pub type_builtin: Rgba,
    pub type_interface: Rgba,
    pub namespace: Rgba,
    pub variable: Option<Rgba>,
    pub variable_parameter: Rgba,
    pub variable_special: Rgba,
    pub variable_builtin: Rgba,
    pub property: Rgba,
    pub label: Option<Rgba>,
    pub constant: Rgba,
    pub constant_builtin: Rgba,
    pub operator: Rgba,
    pub punctuation: Rgba,
    pub punctuation_bracket: Rgba,
    pub punctuation_delimiter: Rgba,
    pub punctuation_special: Rgba,
    pub punctuation_list_marker: Rgba,
    pub tag: Rgba,
    pub attribute: Rgba,
    pub markup_heading: Rgba,
    pub markup_link: Rgba,
    pub text_literal: Rgba,
    pub diff_plus: Rgba,
    pub diff_minus: Rgba,
    pub diff_delta: Rgba,
    pub lifetime: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphLanePalette {
    colors: [Rgba; GRAPH_LANE_PALETTE_SIZE],
    len: u8,
}

impl GraphLanePalette {
    fn generated(is_dark: bool) -> Self {
        let mut colors = [Rgba::new(0.0, 0.0, 0.0, 0.0); GRAPH_LANE_PALETTE_SIZE];
        for (i, color) in colors.iter_mut().enumerate() {
            let hue = (i as f32 * 0.13) % 1.0;
            let sat = 0.75;
            let light = if is_dark { 0.62 } else { 0.33 };
            *color = hsla_from_hue_fraction(hue, sat, light, 1.0).into_color();
        }
        Self {
            colors,
            len: GRAPH_LANE_PALETTE_SIZE as u8,
        }
    }

    fn from_theme_colors(
        is_dark: bool,
        palette: Option<Vec<ThemeColor>>,
        hues: Option<Vec<f32>>,
    ) -> Self {
        if let Some(palette) = palette.filter(|palette| !palette.is_empty()) {
            return Self::from_rgba_slice(
                &palette
                    .into_iter()
                    .map(ThemeColor::into_rgba)
                    .collect::<Vec<_>>(),
            );
        }

        if let Some(hues) = hues.filter(|hues| !hues.is_empty()) {
            let sat = 0.75;
            let light = if is_dark { 0.62 } else { 0.33 };
            let colors = hues
                .into_iter()
                .map(|hue| hsla_from_hue_fraction(hue, sat, light, 1.0).into_color())
                .collect::<Vec<_>>();
            return Self::from_rgba_slice(&colors);
        }

        Self::generated(is_dark)
    }

    fn from_rgba_slice(colors: &[Rgba]) -> Self {
        let mut out = [Rgba::new(0.0, 0.0, 0.0, 0.0); GRAPH_LANE_PALETTE_SIZE];
        let len = colors.len().min(GRAPH_LANE_PALETTE_SIZE);
        for (slot, color) in out.iter_mut().zip(colors.iter().take(len)) {
            *slot = *color;
        }
        Self {
            colors: out,
            len: len as u8,
        }
    }

    pub fn as_slice(&self) -> &[Rgba] {
        let len = usize::from(self.len).max(1);
        &self.colors[..len]
    }

    /// A palette shorter than [`GRAPH_LANE_PALETTE_SIZE`], for tests that need
    /// [`color_at`](Self::color_at) to actually wrap. Leaked because
    /// [`AppTheme::graph_lane_palette`] is a `&'static`.
    #[cfg(test)]
    pub(crate) fn leaked_for_test(colors: &[Rgba]) -> &'static Self {
        Box::leak(Box::new(Self::from_rgba_slice(colors)))
    }

    /// Colour for a lane index, wrapping at the palette's own length.
    ///
    /// A custom theme may supply fewer than [`GRAPH_LANE_PALETTE_SIZE`] colours,
    /// leaving the rest of the backing array transparent black, while lane
    /// indices are handed out cyclically over the full palette size. Indexing
    /// the array directly would therefore paint invisible lanes; wrapping at
    /// `len` reuses the theme's colours instead.
    #[inline]
    pub fn color_at(&self, ix: u8) -> Rgba {
        let slice = self.as_slice();
        slice[usize::from(ix) % slice.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Radii {
    pub panel: f32,
    pub pill: f32,
    pub row: f32,
    /// Corner radius for compact controls (buttons, inputs, tabs).
    #[serde(default = "default_radius_control")]
    pub control: f32,
    /// Corner radius for floating surfaces (menus, popovers, dialogs).
    #[serde(default = "default_radius_popover")]
    pub popover: f32,
    /// Corner radius for the outer window frame (client-side decorations).
    #[serde(default = "default_radius_window")]
    pub window: f32,
}

fn default_radius_control() -> f32 {
    8.0
}

fn default_radius_popover() -> f32 {
    10.0
}

fn default_radius_window() -> f32 {
    12.0
}

impl AppTheme {
    /// Canonical translucent background for hovered standard controls.
    pub fn hover_overlay(&self) -> Rgba {
        self.colors.interaction.hover_overlay
    }

    /// Canonical translucent background for pressed standard controls.
    pub fn active_overlay(&self) -> Rgba {
        self.colors.interaction.pressed_overlay
    }

    /// Stronger hover overlay used by title-bar controls.
    pub fn titlebar_hover_overlay(&self) -> Rgba {
        with_alpha(self.colors.foreground.primary, 0.10)
    }

    /// Stronger pressed overlay used by title-bar controls.
    pub fn titlebar_active_overlay(&self) -> Rgba {
        with_alpha(
            self.colors.foreground.primary,
            if self.is_dark { 0.16 } else { 0.15 },
        )
    }

    #[cfg(test)]
    pub(crate) fn from_json_str(json: &str) -> Result<Self, ThemeParseError> {
        let mut bundle = parse_theme_bundle(json)?;
        if bundle.themes.len() != 1 {
            return Err(ThemeParseError::Invalid(format!(
                "theme bundle must contain exactly one theme, found {}",
                bundle.themes.len()
            )));
        }

        let theme = bundle
            .themes
            .pop()
            .expect("bundle length checked before popping");
        Ok(theme.into_app_theme())
    }

    #[cfg(test)]
    pub(crate) fn from_json_path(path: impl AsRef<Path>) -> Result<Self, ThemeLoadError> {
        let path = path.as_ref();
        let json = fs::read_to_string(path).map_err(|source| ThemeLoadError::Read {
            path: path.to_path_buf(),
            source,
        })?;

        Self::from_json_str(&json).map_err(|source| ThemeLoadError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn default_for_window_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => {
                Self::from_key(DEFAULT_LIGHT_THEME_KEY).unwrap_or_else(|| {
                    panic!("missing default light theme `{DEFAULT_LIGHT_THEME_KEY}`")
                })
            }
            WindowAppearance::Dark | WindowAppearance::VibrantDark => {
                Self::from_key(DEFAULT_DARK_THEME_KEY).unwrap_or_else(|| {
                    panic!("missing default dark theme `{DEFAULT_DARK_THEME_KEY}`")
                })
            }
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        embedded_theme_cache()
            .get(key)
            .map(|spec| spec.theme)
            .or_else(|| runtime_themes().get(key).map(|spec| spec.theme))
    }

    /// GitComet's default dark theme loaded from an embedded JSON definition.
    pub fn gitcomet_dark() -> Self {
        Self::from_key(DEFAULT_DARK_THEME_KEY)
            .unwrap_or_else(|| panic!("missing default dark theme `{DEFAULT_DARK_THEME_KEY}`"))
    }

    /// GitComet's default light theme loaded from an embedded JSON definition.
    #[cfg(test)]
    pub fn gitcomet_light() -> Self {
        Self::from_key(DEFAULT_LIGHT_THEME_KEY)
            .unwrap_or_else(|| panic!("missing default light theme `{DEFAULT_LIGHT_THEME_KEY}`"))
    }
}

pub(crate) fn available_themes() -> Vec<ThemeOption> {
    merged_theme_options(None)
}

pub(crate) fn has_theme_key(key: &str) -> bool {
    merged_theme_options(None)
        .iter()
        .any(|option| option.key == key)
}

pub(crate) fn theme_label(key: &str) -> Option<String> {
    merged_theme_options(None)
        .into_iter()
        .find(|option| option.key == key)
        .map(|option| option.label)
}

pub(crate) fn ensure_user_themes_dir_exists() -> Option<PathBuf> {
    resolved_runtime_themes_dir(None)
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) enum ThemeLoadError {
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        source: ThemeParseError,
    },
}

#[cfg(test)]
impl fmt::Display for ThemeLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    f,
                    "failed to read theme JSON from {}: {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse theme JSON from {}: {source}",
                    path.display()
                )
            }
        }
    }
}

#[cfg(test)]
impl Error for ThemeLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ThemeParseError {
    Parse(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for ThemeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(source) => source.fmt(f),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl Error for ThemeParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(source) => Some(source),
            Self::Invalid(_) => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeBundleFile {
    /// Declared here only so `deny_unknown_fields` accepts the key. The value is
    /// read off the raw JSON in [`parse_theme_bundle`], before this struct is
    /// built -- see there for why it cannot wait until afterwards.
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "author", default)]
    _author: Option<String>,
    themes: Vec<ThemeBundleEntry>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThemeAppearance {
    Light,
    Dark,
}

impl ThemeAppearance {
    const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeBundleEntry {
    key: String,
    name: String,
    appearance: ThemeAppearance,
    colors: ThemeFileColors,
    #[serde(default)]
    syntax: Option<ThemeFileSyntaxColors>,
    radii: Radii,
}

impl ThemeBundleEntry {
    fn into_app_theme(self) -> AppTheme {
        ThemeFile {
            appearance: self.appearance,
            colors: self.colors,
            syntax: self.syntax,
            radii: self.radii,
        }
        .into()
    }
}

struct ThemeFile {
    appearance: ThemeAppearance,
    colors: ThemeFileColors,
    syntax: Option<ThemeFileSyntaxColors>,
    radii: Radii,
}

impl ThemeFile {
    fn is_dark(&self) -> bool {
        self.appearance.is_dark()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileColors {
    surface: ThemeFileSurfaceColors,
    foreground: ThemeFileForegroundColors,
    stroke: ThemeFileStrokeColors,
    interaction: ThemeFileInteractionColors,
    accent: ThemeFileAccentColors,
    status: ThemeFileStatusColors,
    editor: ThemeFileEditorColors,
    diff: ThemeFileDiffColors,
    tooltip: ThemeFileTooltipColors,
    scrollbar: ThemeFileScrollbarColors,
    shadow: ThemeColor,
    #[serde(default)]
    graph_lane_palette: Option<Vec<ThemeColor>>,
    #[serde(default)]
    graph_lane_hues: Option<Vec<f32>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileSurfaceColors {
    canvas: ThemeColor,
    chrome: ThemeColor,
    panel: ThemeColor,
    raised: ThemeColor,
    input: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileForegroundColors {
    primary: ThemeColor,
    secondary: ThemeColor,
    disabled: ThemeColor,
    placeholder: ThemeColor,
    emphasis: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileStrokeColors {
    subtle: ThemeColor,
    default: ThemeColor,
    control: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileInteractionColors {
    hover_overlay: ThemeColor,
    pressed_overlay: ThemeColor,
    hover_background: ThemeColor,
    pressed_background: ThemeColor,
    selected_background: ThemeColor,
    selected_foreground: ThemeColor,
    selected_indicator: ThemeColor,
    focus_ring: ThemeColor,
    focus_background: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileAccentColors {
    foreground: ThemeColor,
    solid: ThemeColor,
    on_solid: ThemeColor,
    subtle_background: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileStatusColorSet {
    foreground: ThemeColor,
    background: ThemeColor,
    border: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileStatusColors {
    info: ThemeFileStatusColorSet,
    success: ThemeFileStatusColorSet,
    warning: ThemeFileStatusColorSet,
    danger: ThemeFileStatusColorSet,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileEditorColors {
    background: ThemeColor,
    foreground: ThemeColor,
    gutter_background: ThemeColor,
    line_number: ThemeColor,
    cursor: ThemeColor,
    selection_background: ThemeColor,
    search_match_background: ThemeColor,
    search_match_foreground: ThemeColor,
    bracket_match_background: ThemeColor,
    indent_guide: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileDiffColorSet {
    foreground: ThemeColor,
    background: ThemeColor,
    word_background: ThemeColor,
    focused_background: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileDiffColors {
    added: ThemeFileDiffColorSet,
    removed: ThemeFileDiffColorSet,
    modified: ThemeFileDiffColorSet,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileTooltipColors {
    background: ThemeColor,
    foreground: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileScrollbarColors {
    thumb: ThemeColor,
    thumb_hover: ThemeColor,
    thumb_pressed: ThemeColor,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileSyntaxColors {
    #[serde(default)]
    comment: Option<ThemeColor>,
    #[serde(default)]
    comment_doc: Option<ThemeColor>,
    #[serde(default)]
    string: Option<ThemeColor>,
    #[serde(default)]
    string_escape: Option<ThemeColor>,
    #[serde(default)]
    string_regex: Option<ThemeColor>,
    #[serde(default)]
    string_special: Option<ThemeColor>,
    #[serde(default)]
    keyword: Option<ThemeColor>,
    #[serde(default)]
    keyword_control: Option<ThemeColor>,
    #[serde(default)]
    preproc: Option<ThemeColor>,
    #[serde(default)]
    number: Option<ThemeColor>,
    #[serde(default)]
    boolean: Option<ThemeColor>,
    #[serde(default)]
    function: Option<ThemeColor>,
    #[serde(default)]
    function_method: Option<ThemeColor>,
    #[serde(default)]
    function_special: Option<ThemeColor>,
    #[serde(default)]
    constructor: Option<ThemeColor>,
    #[serde(rename = "type", default)]
    type_name: Option<ThemeColor>,
    #[serde(default)]
    type_builtin: Option<ThemeColor>,
    #[serde(default)]
    type_interface: Option<ThemeColor>,
    #[serde(default)]
    namespace: Option<ThemeColor>,
    #[serde(default)]
    variable: Option<ThemeColor>,
    #[serde(default)]
    variable_parameter: Option<ThemeColor>,
    #[serde(default)]
    variable_special: Option<ThemeColor>,
    #[serde(default)]
    variable_builtin: Option<ThemeColor>,
    #[serde(default)]
    property: Option<ThemeColor>,
    #[serde(default)]
    label: Option<ThemeColor>,
    #[serde(default)]
    constant: Option<ThemeColor>,
    #[serde(default)]
    constant_builtin: Option<ThemeColor>,
    #[serde(default)]
    operator: Option<ThemeColor>,
    #[serde(default)]
    punctuation: Option<ThemeColor>,
    #[serde(default)]
    punctuation_bracket: Option<ThemeColor>,
    #[serde(default)]
    punctuation_delimiter: Option<ThemeColor>,
    #[serde(default)]
    punctuation_special: Option<ThemeColor>,
    #[serde(default)]
    punctuation_list_marker: Option<ThemeColor>,
    #[serde(default)]
    tag: Option<ThemeColor>,
    #[serde(default)]
    attribute: Option<ThemeColor>,
    #[serde(default)]
    markup_heading: Option<ThemeColor>,
    #[serde(default)]
    markup_link: Option<ThemeColor>,
    #[serde(default)]
    text_literal: Option<ThemeColor>,
    #[serde(default)]
    diff_plus: Option<ThemeColor>,
    #[serde(default)]
    diff_minus: Option<ThemeColor>,
    #[serde(default)]
    diff_delta: Option<ThemeColor>,
    #[serde(default)]
    lifetime: Option<ThemeColor>,
}

/// An `Rgba` written as a CSS-style hex string in a theme file.
///
/// `gpui::Rgba` is a `palette` re-export, and its `Deserialize` reads a
/// `{"red":…,"green":…,"blue":…,"alpha":…}` object, not the `"#rrggbbaa"`
/// strings every theme file — shipped and user-authored alike — is written in.
/// `palette`'s own `FromStr` is closer but only accepts the two forms that
/// carry alpha, so the four-form parser lives here.
#[derive(Clone, Copy)]
struct HexColor(Rgba);

impl HexColor {
    /// Accepts `#rgb`, `#rgba`, `#rrggbb` and `#rrggbbaa`, with the short forms
    /// expanding each digit (`#f0c` is `#ff00cc`) and alpha defaulting to
    /// opaque. Anything else is an error rather than a silent black.
    fn parse(value: &str) -> Result<Self, String> {
        let hex = value.trim().strip_prefix('#').ok_or_else(|| {
            format!("invalid hex color {value:?}: expected #rgb, #rgba, #rrggbb, or #rrggbbaa")
        })?;

        let digit = |ix: usize| -> Result<u8, String> {
            u8::from_str_radix(&hex[ix..ix + 1], 16)
                .map_err(|err| format!("invalid hex color {value:?}: {err}"))
        };
        let pair = |ix: usize| -> Result<u8, String> {
            u8::from_str_radix(&hex[ix..ix + 2], 16)
                .map_err(|err| format!("invalid hex color {value:?}: {err}"))
        };

        let components = match hex.len() {
            len @ (3 | 4) if hex.is_ascii() => {
                let alpha = if len == 4 { digit(3)? } else { 0xf };
                // `#abc` means `#aabbcc`, so each digit is duplicated rather
                // than shifted — `0xa` widens to `0xaa`, not `0xa0`.
                [digit(0)?, digit(1)?, digit(2)?, alpha].map(|d| (d << 4) | d)
            }
            len @ (6 | 8) if hex.is_ascii() => {
                let alpha = if len == 8 { pair(6)? } else { 0xff };
                [pair(0)?, pair(2)?, pair(4)?, alpha]
            }
            _ => {
                return Err(format!(
                    "invalid hex color {value:?}: expected #rgb, #rgba, #rrggbb, or #rrggbbaa"
                ));
            }
        };

        let [r, g, b, a] = components.map(|c| f32::from(c) / 255.0);
        Ok(Self(Rgba::new(r, g, b, a)))
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
enum ThemeColor {
    Hex(HexColor),
    HexWithAlpha { hex: HexColor, alpha: f32 },
}

impl ThemeColor {
    fn into_rgba(self) -> Rgba {
        match self {
            Self::Hex(color) => color.0,
            Self::HexWithAlpha { hex, alpha } => with_alpha(hex.0, alpha),
        }
    }
}

impl From<ThemeFile> for AppTheme {
    fn from(theme: ThemeFile) -> Self {
        let is_dark = theme.is_dark();
        let ThemeFile {
            appearance: _,
            colors,
            syntax,
            radii,
            ..
        } = theme;
        let ThemeFileColors {
            surface,
            foreground,
            stroke,
            interaction,
            accent,
            status,
            editor,
            diff,
            tooltip,
            scrollbar,
            shadow,
            graph_lane_palette,
            graph_lane_hues,
        } = colors;
        let graph_lane_palette =
            GraphLanePalette::from_theme_colors(is_dark, graph_lane_palette, graph_lane_hues);
        let status_set = |set: ThemeFileStatusColorSet| StatusColorSet {
            foreground: set.foreground.into_rgba(),
            background: set.background.into_rgba(),
            border: set.border.into_rgba(),
        };
        let diff_set = |set: ThemeFileDiffColorSet| DiffColorSet {
            foreground: set.foreground.into_rgba(),
            background: set.background.into_rgba(),
            word_background: set.word_background.into_rgba(),
            focused_background: set.focused_background.into_rgba(),
        };
        let colors = Colors {
            surface: SurfaceColors {
                canvas: surface.canvas.into_rgba(),
                chrome: surface.chrome.into_rgba(),
                panel: surface.panel.into_rgba(),
                raised: surface.raised.into_rgba(),
                input: surface.input.into_rgba(),
            },
            foreground: ForegroundColors {
                primary: foreground.primary.into_rgba(),
                secondary: foreground.secondary.into_rgba(),
                disabled: foreground.disabled.into_rgba(),
                placeholder: foreground.placeholder.into_rgba(),
                emphasis: foreground.emphasis.into_rgba(),
            },
            stroke: StrokeColors {
                subtle: stroke.subtle.into_rgba(),
                default: stroke.default.into_rgba(),
                control: stroke.control.into_rgba(),
            },
            interaction: InteractionColors {
                hover_overlay: interaction.hover_overlay.into_rgba(),
                pressed_overlay: interaction.pressed_overlay.into_rgba(),
                hover_background: interaction.hover_background.into_rgba(),
                pressed_background: interaction.pressed_background.into_rgba(),
                selected_background: interaction.selected_background.into_rgba(),
                selected_foreground: interaction.selected_foreground.into_rgba(),
                selected_indicator: interaction.selected_indicator.into_rgba(),
                focus_ring: interaction.focus_ring.into_rgba(),
                focus_background: interaction.focus_background.into_rgba(),
            },
            accent: AccentColors {
                foreground: accent.foreground.into_rgba(),
                solid: accent.solid.into_rgba(),
                on_solid: accent.on_solid.into_rgba(),
                subtle_background: accent.subtle_background.into_rgba(),
            },
            status: StatusColors {
                info: status_set(status.info),
                success: status_set(status.success),
                warning: status_set(status.warning),
                danger: status_set(status.danger),
            },
            editor: EditorColors {
                background: editor.background.into_rgba(),
                foreground: editor.foreground.into_rgba(),
                gutter_background: editor.gutter_background.into_rgba(),
                line_number: editor.line_number.into_rgba(),
                cursor: editor.cursor.into_rgba(),
                selection_background: editor.selection_background.into_rgba(),
                search_match_background: editor.search_match_background.into_rgba(),
                search_match_foreground: editor.search_match_foreground.into_rgba(),
                bracket_match_background: editor.bracket_match_background.into_rgba(),
                indent_guide: editor.indent_guide.into_rgba(),
            },
            diff: DiffColors {
                added: diff_set(diff.added),
                removed: diff_set(diff.removed),
                modified: diff_set(diff.modified),
            },
            tooltip: TooltipColors {
                background: tooltip.background.into_rgba(),
                foreground: tooltip.foreground.into_rgba(),
            },
            scrollbar: ScrollbarColors {
                thumb: scrollbar.thumb.into_rgba(),
                thumb_hover: scrollbar.thumb_hover.into_rgba(),
                thumb_pressed: scrollbar.thumb_pressed.into_rgba(),
            },
            shadow: shadow.into_rgba(),
        };
        let syntax = resolve_syntax_colors(is_dark, &colors, syntax.as_ref());

        Self {
            is_dark,
            colors,
            syntax,
            graph_lane_palette: intern_lane_palette(graph_lane_palette),
            radii,
        }
    }
}

static INTERNED_LANE_PALETTES: OnceLock<Mutex<Vec<&'static GraphLanePalette>>> = OnceLock::new();

/// Hands out a `'static` reference to a lane palette, reusing one already
/// interned when the colours match.
///
/// Themes are loaded a handful of times over a session -- at startup, and when
/// the user picks another one -- while their palettes are copied into every
/// per-row paint closure of every frame. Trading a one-time leak per *distinct*
/// palette for a pointer-sized field in `AppTheme` is the right side of that
/// exchange; reloading the same theme file interns nothing new.
fn intern_lane_palette(palette: GraphLanePalette) -> &'static GraphLanePalette {
    let interned = INTERNED_LANE_PALETTES.get_or_init(|| Mutex::new(Vec::new()));
    let mut interned = interned
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = interned
        .iter()
        .find(|existing| lane_palettes_are_identical(existing, &palette))
    {
        return existing;
    }
    let leaked: &'static GraphLanePalette = Box::leak(Box::new(palette));
    interned.push(leaked);
    leaked
}

/// Compares two palettes bit for bit rather than by `PartialEq`.
///
/// A theme can put a non-finite value into a lane colour -- a JSON hue of `1e40`
/// deserializes to infinity, and `rem_euclid` turns that into NaN -- and NaN is
/// never equal to itself, so `PartialEq` would report every such palette as new
/// and leak a fresh copy on every parse. Bit equality is also exactly the
/// identity the interner wants: two palettes reuse one allocation only when they
/// are byte-for-byte the same.
fn lane_palettes_are_identical(a: &GraphLanePalette, b: &GraphLanePalette) -> bool {
    a.as_slice().len() == b.as_slice().len()
        && a.as_slice().iter().zip(b.as_slice()).all(|(a, b)| {
            a.red.to_bits() == b.red.to_bits()
                && a.green.to_bits() == b.green.to_bits()
                && a.blue.to_bits() == b.blue.to_bits()
                && a.alpha.to_bits() == b.alpha.to_bits()
        })
}

pub(crate) fn mix_colors(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        1.0,
    )
}

fn derived_syntax_color(is_dark: bool, colors: &Colors, token: Rgba) -> Rgba {
    let blend_to_text = if is_dark { 0.42 } else { 0.58 };
    mix_colors(token, colors.foreground.primary, blend_to_text)
}

fn resolve_syntax_color(override_color: Option<ThemeColor>, fallback: Rgba) -> Rgba {
    override_color
        .map(ThemeColor::into_rgba)
        .unwrap_or(fallback)
}

fn resolve_optional_syntax_color(override_color: Option<ThemeColor>) -> Option<Rgba> {
    override_color.map(ThemeColor::into_rgba)
}

fn resolve_syntax_colors(
    is_dark: bool,
    colors: &Colors,
    syntax: Option<&ThemeFileSyntaxColors>,
) -> SyntaxColors {
    let overrides = syntax.cloned().unwrap_or_default();
    let accent = derived_syntax_color(is_dark, colors, colors.accent.foreground);
    let warning = derived_syntax_color(is_dark, colors, colors.status.warning.foreground);
    let success = derived_syntax_color(is_dark, colors, colors.status.success.foreground);

    SyntaxColors {
        comment: resolve_syntax_color(overrides.comment, colors.foreground.secondary),
        comment_doc: resolve_syntax_color(overrides.comment_doc, colors.foreground.secondary),
        string: resolve_syntax_color(overrides.string, warning),
        string_escape: resolve_syntax_color(overrides.string_escape, success),
        string_regex: resolve_syntax_color(
            overrides.string_regex,
            resolve_syntax_color(overrides.string, warning),
        ),
        string_special: resolve_syntax_color(
            overrides.string_special,
            resolve_syntax_color(overrides.string, warning),
        ),
        keyword: resolve_syntax_color(overrides.keyword, accent),
        keyword_control: resolve_syntax_color(overrides.keyword_control, accent),
        preproc: resolve_syntax_color(
            overrides.preproc,
            resolve_syntax_color(overrides.keyword, accent),
        ),
        number: resolve_syntax_color(overrides.number, success),
        boolean: resolve_syntax_color(overrides.boolean, success),
        function: resolve_syntax_color(overrides.function, accent),
        function_method: resolve_syntax_color(overrides.function_method, accent),
        function_special: resolve_syntax_color(overrides.function_special, accent),
        constructor: resolve_syntax_color(
            overrides.constructor,
            resolve_syntax_color(overrides.function, accent),
        ),
        type_name: resolve_syntax_color(overrides.type_name, warning),
        type_builtin: resolve_syntax_color(overrides.type_builtin, warning),
        type_interface: resolve_syntax_color(overrides.type_interface, warning),
        namespace: resolve_syntax_color(
            overrides.namespace,
            resolve_syntax_color(overrides.type_name, warning),
        ),
        variable: resolve_optional_syntax_color(overrides.variable),
        variable_parameter: resolve_syntax_color(
            overrides.variable_parameter,
            colors.foreground.secondary,
        ),
        variable_special: resolve_syntax_color(overrides.variable_special, accent),
        variable_builtin: resolve_syntax_color(
            overrides.variable_builtin,
            resolve_syntax_color(overrides.variable_special, accent),
        ),
        property: resolve_syntax_color(overrides.property, accent),
        label: resolve_optional_syntax_color(overrides.label)
            .or(resolve_optional_syntax_color(overrides.variable)),
        constant: resolve_syntax_color(overrides.constant, success),
        constant_builtin: resolve_syntax_color(
            overrides.constant_builtin,
            resolve_syntax_color(overrides.constant, success),
        ),
        operator: resolve_syntax_color(overrides.operator, colors.foreground.secondary),
        punctuation: resolve_syntax_color(overrides.punctuation, colors.foreground.secondary),
        punctuation_bracket: resolve_syntax_color(
            overrides.punctuation_bracket,
            colors.foreground.secondary,
        ),
        punctuation_delimiter: resolve_syntax_color(
            overrides.punctuation_delimiter,
            colors.foreground.secondary,
        ),
        punctuation_special: resolve_syntax_color(
            overrides.punctuation_special,
            resolve_syntax_color(overrides.punctuation, colors.foreground.secondary),
        ),
        punctuation_list_marker: resolve_syntax_color(
            overrides.punctuation_list_marker,
            resolve_syntax_color(overrides.punctuation, colors.foreground.secondary),
        ),
        tag: resolve_syntax_color(overrides.tag, warning),
        attribute: resolve_syntax_color(overrides.attribute, accent),
        markup_heading: resolve_syntax_color(
            overrides.markup_heading,
            resolve_syntax_color(overrides.keyword, accent),
        ),
        markup_link: resolve_syntax_color(
            overrides.markup_link,
            resolve_syntax_color(overrides.string, warning),
        ),
        text_literal: resolve_syntax_color(
            overrides.text_literal,
            resolve_syntax_color(overrides.string, warning),
        ),
        diff_plus: resolve_syntax_color(
            overrides.diff_plus,
            resolve_syntax_color(overrides.string, warning),
        ),
        diff_minus: resolve_syntax_color(
            overrides.diff_minus,
            resolve_syntax_color(overrides.keyword, accent),
        ),
        diff_delta: resolve_syntax_color(
            overrides.diff_delta,
            resolve_syntax_color(overrides.type_name, warning),
        ),
        lifetime: resolve_syntax_color(overrides.lifetime, accent),
    }
}

fn shadow_layer(base: Rgba, alpha: f32, y: f32, blur: f32) -> gpui::BoxShadow {
    gpui::BoxShadow {
        color: with_alpha(base, alpha).into_color(),
        offset: gpui::point(gpui::px(0.0), gpui::px(y)),
        blur_radius: gpui::px(blur),
        spread_radius: gpui::px(0.0),
        inset: false,
    }
}

// Design-system stance: modern developer tools lean on borders, not shadows,
// for separation. Inline surfaces stay flat (no shadow); only elements that
// genuinely float off the canvas (menus, dialogs) get a single, restrained lift.

/// Resting "elevation" for inline cards/panels — intentionally flat. Separation
/// comes from the default and subtle strokes, not shadow.
pub(crate) fn shadow_surface(_theme: AppTheme) -> Vec<gpui::BoxShadow> {
    Vec::new()
}

/// A single, restrained lift for dropdowns, context menus and hover panels.
pub(crate) fn shadow_popover(theme: AppTheme) -> Vec<gpui::BoxShadow> {
    let base = theme.colors.shadow;
    let m = if theme.is_dark { 1.0 } else { 0.5 };
    vec![shadow_layer(base, 0.22 * m, 4.0, 12.0)]
}

/// Slightly stronger (still understated) lift for modal dialogs.
pub(crate) fn shadow_modal(theme: AppTheme) -> Vec<gpui::BoxShadow> {
    let base = theme.colors.shadow;
    let m = if theme.is_dark { 1.0 } else { 0.6 };
    vec![
        shadow_layer(base, 0.24 * m, 2.0, 8.0),
        shadow_layer(base, 0.18 * m, 10.0, 28.0),
    ]
}

fn embedded_theme_cache() -> &'static FxHashMap<String, RuntimeThemeSpec> {
    EMBEDDED_THEME_CACHE.get_or_init(|| {
        let mut themes = FxHashMap::default();
        for file in EMBEDDED_THEME_FILES {
            let specs = load_theme_specs_from_json(file.json).unwrap_or_else(|err| {
                panic!("failed to load built-in theme file {}: {err}", file.stem)
            });
            for spec in specs {
                themes.insert(spec.option.key.clone(), spec);
            }
        }
        themes
    })
}

#[derive(Clone)]
struct RuntimeThemeSpec {
    option: ThemeOption,
    theme: AppTheme,
}

fn is_embedded_theme_key(key: &str) -> bool {
    embedded_theme_cache().contains_key(key)
}

fn is_embedded_theme_stem(stem: &str) -> bool {
    EMBEDDED_THEME_FILES.iter().any(|file| file.stem == stem)
}

fn is_reserved_runtime_theme_path(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(is_embedded_theme_stem)
}

fn merged_theme_options(runtime_dir: Option<&Path>) -> Vec<ThemeOption> {
    let mut options = BTreeMap::<String, ThemeOption>::new();
    for spec in runtime_themes_with_dir(runtime_dir).values() {
        options.insert(spec.option.key.clone(), spec.option.clone());
    }
    for spec in embedded_theme_cache().values() {
        options.insert(spec.option.key.clone(), spec.option.clone());
    }

    options.into_values().collect()
}

fn runtime_themes() -> Arc<FxHashMap<String, RuntimeThemeSpec>> {
    runtime_themes_with_dir(None)
}

/// Custom themes from disk, re-parsed only when the directory has actually
/// changed.
///
/// Reading and parsing every theme file is far too expensive to do per call: the
/// settings theme list asks for it from inside a `uniform_list` processor, so an
/// unmemoized load is a directory read plus a full parse per file *per frame*
/// while that dropdown is open. Theme authors still expect an edit to show up
/// without a restart, so the cache is validated against a cheap stat of the
/// directory rather than held forever.
fn runtime_themes_with_dir(runtime_dir: Option<&Path>) -> Arc<FxHashMap<String, RuntimeThemeSpec>> {
    runtime_theme_cache_entry(runtime_dir)
        .map(|entry| entry.themes)
        .unwrap_or_default()
}

/// Why the themes that are *not* in the picker were left out.
///
/// A rejected file is otherwise invisible: it disappears from the list, the app
/// falls back to a bundled theme, and the only account of it goes to stderr,
/// which nobody running a windowed build ever sees. That matters most right
/// after a schema break -- every v1 custom theme in the folder is rejected at
/// once, and "my theme is gone" needs to be answerable without a terminal.
pub(crate) fn runtime_theme_issues() -> Arc<[RuntimeThemeIssue]> {
    runtime_theme_issues_with_dir(None)
}

fn runtime_theme_issues_with_dir(runtime_dir: Option<&Path>) -> Arc<[RuntimeThemeIssue]> {
    runtime_theme_cache_entry(runtime_dir)
        .map(|entry| entry.issues)
        .unwrap_or_else(|| Arc::from(Vec::new()))
}

fn runtime_theme_cache_entry(runtime_dir: Option<&Path>) -> Option<RuntimeThemeCache> {
    let dir = resolved_runtime_themes_dir(runtime_dir)?;

    let signature = runtime_themes_dir_signature(&dir);
    let cache = RUNTIME_THEME_CACHE.get_or_init(|| Mutex::new(FxHashMap::default()));
    {
        let cached = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = cached.get(&dir)
            && cached.signature == signature
        {
            return Some(cached.clone());
        }
    }

    let (themes, issues) = load_runtime_themes_from_dir(&dir);
    let entry = RuntimeThemeCache {
        signature,
        themes: Arc::new(themes),
        issues: Arc::from(issues),
    };
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // The app only ever asks for the one user theme directory, so this map holds
    // a single entry in practice; the bound only keeps tests -- which each hand
    // in their own temp directory -- from growing it without limit.
    if cached.len() >= MAX_CACHED_RUNTIME_THEME_DIRS && !cached.contains_key(&dir) {
        cached.clear();
    }
    cached.insert(dir, entry.clone());
    Some(entry)
}

/// A theme file the loader refused, named so the picker can say so.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeThemeIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone)]
struct RuntimeThemeCache {
    signature: u64,
    themes: Arc<FxHashMap<String, RuntimeThemeSpec>>,
    issues: Arc<[RuntimeThemeIssue]>,
}

/// One entry per theme directory, rather than a single slot: a single slot is
/// evicted by any interleaved load of a *different* directory, which would drop
/// the memoization the settings dropdown depends on the moment anything else
/// asks for themes elsewhere.
static RUNTIME_THEME_CACHE: OnceLock<Mutex<FxHashMap<PathBuf, RuntimeThemeCache>>> =
    OnceLock::new();

const MAX_CACHED_RUNTIME_THEME_DIRS: usize = 16;

/// Identity of the theme directory's contents: every `.json` file's name, size
/// and modification time. Stat-only, so validating the cache costs a directory
/// walk rather than a parse, and an edited or added theme still invalidates it.
fn runtime_themes_dir_signature(dir: &Path) -> u64 {
    use std::hash::{Hash, Hasher};

    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<(PathBuf, u64, Option<std::time::SystemTime>)> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|path| {
            let meta = fs::metadata(&path).ok();
            let len = meta.as_ref().map(|meta| meta.len()).unwrap_or(0);
            let modified = meta.as_ref().and_then(|meta| meta.modified().ok());
            (path, len, modified)
        })
        .collect();
    files.sort();

    let mut hasher = FxHasher::default();
    for (path, len, modified) in files {
        path.hash(&mut hasher);
        len.hash(&mut hasher);
        modified
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_nanos())
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn resolved_runtime_themes_dir(runtime_dir: Option<&Path>) -> Option<PathBuf> {
    let dir = match runtime_dir {
        Some(path) => path.to_path_buf(),
        None => gitcomet_state::session::user_themes_dir()?,
    };

    if fs::create_dir_all(&dir).is_err() {
        return None;
    }

    Some(dir)
}

fn load_runtime_themes_from_dir(
    dir: &Path,
) -> (FxHashMap<String, RuntimeThemeSpec>, Vec<RuntimeThemeIssue>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (FxHashMap::default(), Vec::new());
    };

    let mut files = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter(|path| !is_reserved_runtime_theme_path(path))
        .collect::<Vec<_>>();
    files.sort();

    let mut themes = FxHashMap::default();
    let mut issues = Vec::new();
    let reject = |issues: &mut Vec<RuntimeThemeIssue>, path: &Path, message: String| {
        eprintln!("Ignoring custom theme {}: {message}", path.display());
        issues.push(RuntimeThemeIssue {
            path: path.to_path_buf(),
            message,
        });
    };
    for path in files {
        let json = match fs::read_to_string(&path) {
            Ok(json) => json,
            Err(error) => {
                reject(&mut issues, &path, format!("failed to read file: {error}"));
                continue;
            }
        };
        let specs = match load_runtime_theme_specs_from_json(&json) {
            Ok(specs) => specs,
            Err(error) => {
                reject(&mut issues, &path, error.to_string());
                continue;
            }
        };

        for spec in specs {
            themes.insert(spec.option.key.clone(), spec);
        }
    }

    (themes, issues)
}

fn load_theme_specs_from_json(json: &str) -> Result<Vec<RuntimeThemeSpec>, ThemeParseError> {
    let bundle = parse_theme_bundle(json)?;
    load_theme_specs_from_bundle(bundle)
}

fn load_runtime_theme_specs_from_json(
    json: &str,
) -> Result<Vec<RuntimeThemeSpec>, ThemeParseError> {
    let bundle = parse_theme_bundle(json)?;
    load_runtime_theme_specs_from_bundle(bundle)
}

fn load_theme_specs_from_bundle(
    bundle: ThemeBundleFile,
) -> Result<Vec<RuntimeThemeSpec>, ThemeParseError> {
    collect_theme_specs(bundle, false)
}

fn load_runtime_theme_specs_from_bundle(
    bundle: ThemeBundleFile,
) -> Result<Vec<RuntimeThemeSpec>, ThemeParseError> {
    collect_theme_specs(bundle, true)
}

fn collect_theme_specs(
    bundle: ThemeBundleFile,
    skip_embedded_keys: bool,
) -> Result<Vec<RuntimeThemeSpec>, ThemeParseError> {
    if bundle.themes.is_empty() {
        return Err(ThemeParseError::Invalid(
            "theme bundle must define at least one theme".to_string(),
        ));
    }

    let mut seen_keys = FxHashSet::<String>::default();
    let mut themes = Vec::with_capacity(bundle.themes.len());

    for entry in bundle.themes {
        let key = entry.key.clone();
        if skip_embedded_keys && is_embedded_theme_key(&key) {
            continue;
        }

        if !seen_keys.insert(key.clone()) {
            return Err(ThemeParseError::Invalid(format!(
                "theme bundle defines duplicate key `{key}`"
            )));
        }

        themes.push(RuntimeThemeSpec {
            option: ThemeOption {
                key,
                label: entry.name.clone(),
            },
            theme: entry.into_app_theme(),
        });
    }

    Ok(themes)
}

/// Tokens deliberately left out of [`fill_missing_color_tokens`]. Both are
/// optional by design and already have a fallback of their own: omitting them
/// generates a lane palette for the theme's appearance
/// ([`GraphLanePalette::from_theme_colors`]). Filling them from the bundled theme
/// instead would hand every such theme gitcomet's hand-picked lane colours and
/// silently repaint its graph.
const UNFILLED_COLOR_TOKENS: &[&str] = &["graph_lane_palette", "graph_lane_hues"];

static BUNDLED_COLOR_TOKENS: OnceLock<FxHashMap<&'static str, serde_json::Value>> = OnceLock::new();

/// The `colors` object of the bundled theme for `appearance`, as raw JSON.
///
/// Raw rather than parsed on purpose: it is merged into a theme file *before*
/// that file is deserialized, so both sides have to be JSON.
fn bundled_color_tokens(appearance: &str) -> Option<&'static serde_json::Value> {
    let wanted = if appearance == "light" {
        DEFAULT_LIGHT_THEME_KEY
    } else {
        DEFAULT_DARK_THEME_KEY
    };
    BUNDLED_COLOR_TOKENS
        .get_or_init(|| {
            let mut tokens = FxHashMap::default();
            for file in EMBEDDED_THEME_FILES {
                let Ok(bundle) = serde_json::from_str::<serde_json::Value>(file.json) else {
                    continue;
                };
                let Some(themes) = bundle.get("themes").and_then(|themes| themes.as_array()) else {
                    continue;
                };
                for theme in themes {
                    let key = match theme.get("key").and_then(|key| key.as_str()) {
                        Some(key) if key == DEFAULT_DARK_THEME_KEY => DEFAULT_DARK_THEME_KEY,
                        Some(key) if key == DEFAULT_LIGHT_THEME_KEY => DEFAULT_LIGHT_THEME_KEY,
                        _ => continue,
                    };
                    if let Some(colors) = theme.get("colors") {
                        tokens.insert(key, colors.clone());
                    }
                }
            }
            tokens
        })
        .get(wanted)
}

/// Fills tokens a theme file leaves out with the bundled theme of the same
/// appearance, so a file written against an older token set keeps loading after
/// new tokens are added — the alternative is that every custom theme in the wild
/// breaks at once, and a broken theme is dropped from the picker with nothing
/// said in-app.
///
/// Only `colors` is filled. `key`, `name` and `appearance` identify the theme and
/// must come from the file itself, or a half-written file would take the bundled
/// theme's identity and collide with it in the picker. Unknown tokens are still
/// rejected: filling in what is missing must not make a typo silently do nothing.
fn fill_missing_color_tokens(bundle: &mut serde_json::Value) {
    let Some(themes) = bundle
        .get_mut("themes")
        .and_then(|themes| themes.as_array_mut())
    else {
        return;
    };

    for theme in themes {
        let appearance = theme
            .get("appearance")
            .and_then(|appearance| appearance.as_str())
            .unwrap_or("dark")
            .to_string();
        let Some(base) = bundled_color_tokens(&appearance) else {
            continue;
        };
        let Some(entry) = theme.as_object_mut() else {
            continue;
        };
        let colors = entry
            .entry("colors")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        fill_missing_json_fields(base, colors, UNFILLED_COLOR_TOKENS);
    }
}

/// Whether a JSON object is one colour rather than a group of them.
///
/// [`ThemeColor`]'s object form is `{hex, alpha}` and needs both halves. Filling
/// a half-written one from the bundled theme would rewrite a colour the author
/// *did* write, at an opacity they never asked for and get no diagnostic about —
/// so the fill stops here and the half-written colour stays the parse error it
/// has always been. Groups never carry a `hex` key, which is what tells them
/// apart.
fn is_color_leaf(value: &serde_json::Value) -> bool {
    value.get("hex").is_some()
}

/// Copies every field of `base` that `target` does not define. Groups present on
/// both sides recurse, so a file may define part of a group and inherit the rest;
/// individual colours are left exactly as the file wrote them. `skip` applies to
/// the top level only.
fn fill_missing_json_fields(
    base: &serde_json::Value,
    target: &mut serde_json::Value,
    skip: &[&str],
) {
    let (Some(base), Some(target)) = (base.as_object(), target.as_object_mut()) else {
        return;
    };
    for (key, base_value) in base {
        if skip.contains(&key.as_str()) {
            continue;
        }
        match target.get_mut(key) {
            Some(existing) if !is_color_leaf(base_value) && !is_color_leaf(existing) => {
                fill_missing_json_fields(base_value, existing, &[])
            }
            Some(_) => {}
            None => {
                target.insert(key.clone(), base_value.clone());
            }
        }
    }
}

fn parse_theme_bundle(json: &str) -> Result<ThemeBundleFile, ThemeParseError> {
    let mut value: serde_json::Value =
        serde_json::from_str(json).map_err(ThemeParseError::Parse)?;
    // Checked here, off the raw value, rather than after deserializing into
    // `ThemeBundleFile`. The structural parse is strict -- `deny_unknown_fields`
    // throughout, and the colour groups changed shape between schema versions --
    // so a file from an older schema dies inside it with something like
    // `invalid type: string "#59b7ffff", expected struct ThemeFileAccentColors`,
    // and the one message that tells the author what actually happened never
    // gets a chance to run.
    let schema_version = value.get("schema_version").and_then(|value| value.as_u64());
    match schema_version {
        Some(version) if version == u64::from(THEME_SCHEMA_VERSION) => {}
        Some(version) => {
            return Err(ThemeParseError::Invalid(format!(
                "unsupported theme schema version {version}; expected {THEME_SCHEMA_VERSION}"
            )));
        }
        None => {
            return Err(ThemeParseError::Invalid(format!(
                "missing schema_version; expected {THEME_SCHEMA_VERSION}"
            )));
        }
    }
    fill_missing_color_tokens(&mut value);
    serde_json::from_value(value).map_err(ThemeParseError::Parse)
}

/// Build an [`Hsla`] from a hue given as a 0..1 fraction of the colour wheel.
///
/// Not `gpui::hsla`, which cannot express this: it clamps its hue argument to
/// `0..=1` and stores the result in a `palette::RgbHue`, which is measured in
/// **degrees**. Every fraction therefore lands within one degree of red, and
/// scaling by 360 at the call site is clamped straight back off. Everything
/// downstream reads the hue as degrees — `SceneHsla::from` divides it by 360 on
/// the way to the renderer, and palette's `into_color` agrees — so the scaling
/// has to happen where the `RgbHue` is built. `RgbHue` normalizes cyclically,
/// so hues outside 0..1 wrap rather than clamp.
///
/// Revisit this if gpui's `hsla` ever scales its argument itself; the two would
/// then compound.
pub(crate) fn hsla_from_hue_fraction(
    hue: f32,
    saturation: f32,
    lightness: f32,
    alpha: f32,
) -> Hsla {
    Hsla::new(
        hue * 360.0,
        saturation.clamp(0.0, 1.0),
        lightness.clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    )
}

pub(crate) fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.alpha = alpha;
    color
}

/// Flattens a translucent overlay onto an opaque base, giving the single color
/// the eye sees where the two are stacked. Anything that has to blend into a
/// surface it doesn't paint itself — a label fade over a hovered row, say —
/// needs this rather than the overlay color alone.
pub(crate) fn composite_over(base: Rgba, overlay: Rgba) -> Rgba {
    let t = overlay.alpha.clamp(0.0, 1.0);
    Rgba::new(
        base.red + (overlay.red - base.red) * t,
        base.green + (overlay.green - base.green) * t,
        base.blue + (overlay.blue - base.blue) * t,
        base.alpha,
    )
}

/// A fixed, deliberately-distinct purple flagging that the user is browsing a
/// historical commit rather than the live repository state. Intentionally outside
/// the theme palette so it reads as "off-live" in every theme.
pub(crate) fn historical_outline(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xa78bfa)
    } else {
        gpui::rgb(0x7c3aed)
    }
}

/// `base` washed with just enough [`historical_outline`] to mark a whole content
/// surface as off-live. Deliberately faint: it sits under body text and syntax
/// colors, so it may tint the surface without competing with what is on it.
pub(crate) fn historical_surface_bg(theme: AppTheme, base: Rgba) -> Rgba {
    composite_over(
        base,
        with_alpha(
            historical_outline(theme.is_dark),
            if theme.is_dark { 0.10 } else { 0.05 },
        ),
    )
}

/// The same wash at header strength. A header panel carries only its own label
/// and controls, so it can take the stronger tint that makes browse mode
/// obvious once the frame around the content is gone.
pub(crate) fn historical_header_bg(theme: AppTheme, base: Rgba) -> Rgba {
    composite_over(
        base,
        with_alpha(
            historical_outline(theme.is_dark),
            if theme.is_dark { 0.24 } else { 0.20 },
        ),
    )
}

/// Background for a header band that sits directly on the main content canvas
/// (the diff/file toolbar, per-file diff headers, split column headers).
///
/// Dark themes match `surface.canvas` so the pane reads as one unbroken dark ground
/// and the band is set off only by its bottom border. Light themes use the
/// subtly darker `surface.raised` to separate the band from the white
/// content below it.
pub(crate) fn content_header_bg(theme: AppTheme) -> Rgba {
    if theme.is_dark {
        theme.colors.surface.canvas
    } else {
        theme.colors.surface.raised
    }
}

/// Recency "heat" border color for the blame/annotate column.
///
/// `t` is the line's recency normalized to `[0, 1]` (0 = oldest commit in the
/// file, 1 = newest). Older edits render cool/faint, newer edits warm/bright.
/// The anchor colors are intentionally outside the theme palette so the heat
/// gradient reads consistently in every theme.
pub(crate) fn blame_heat_color(is_dark: bool, t: f32) -> Rgba {
    // old (cool, dim) -> new (warm, bright)
    let (old, new) = if is_dark {
        (gpui::rgb(0x2f4858), gpui::rgb(0xf6c453))
    } else {
        (gpui::rgb(0xbcd0dd), gpui::rgb(0xd98324))
    };
    mix_colors(old, new, t)
}

/// Border color for uncommitted ("Local change") rows in the blame/annotate
/// column. A bright yellow that stands apart from the recency heat gradient so
/// not-yet-committed lines are immediately distinguishable. Used when blaming a
/// committed revision, where staged/unstaged has no meaning.
pub(crate) fn blame_local_change_color(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xffe000)
    } else {
        gpui::rgb(0xf5c400)
    }
}

/// Border color for *staged* local changes in the blame/annotate column. Reuses
/// the theme's diff "added" accent so staged lines read green, consistent with
/// the rest of the diff UI.
pub(crate) fn blame_staged_color(theme: AppTheme) -> Rgba {
    theme.colors.diff.added.foreground
}

/// Border color for *unstaged* local changes in the blame/annotate column.
/// Reuses the theme's diff "removed" accent so unstaged lines read red, standing
/// apart from the green staged bar.
pub(crate) fn blame_unstaged_color(theme: AppTheme) -> Rgba {
    theme.colors.diff.removed.foreground
}

#[cfg(test)]
pub(crate) fn test_theme_bundle_value(base_key: &str) -> serde_json::Value {
    for file in EMBEDDED_THEME_FILES {
        let mut bundle: serde_json::Value =
            serde_json::from_str(file.json).expect("embedded theme JSON should parse");
        let themes = bundle["themes"]
            .as_array_mut()
            .expect("embedded themes should be an array");
        if let Some(index) = themes.iter().position(|theme| theme["key"] == base_key) {
            let theme = themes.remove(index);
            *themes = vec![theme];
            bundle["name"] = serde_json::json!("Test Theme");
            return bundle;
        }
    }

    panic!("embedded test theme `{base_key}` should exist")
}

#[cfg(test)]
pub(crate) fn test_theme_json_with_syntax(base_key: &str, syntax_json: &str) -> String {
    let mut bundle = test_theme_bundle_value(base_key);
    bundle["themes"][0]["syntax"] =
        serde_json::from_str(syntax_json).expect("syntax fixture JSON should parse");
    serde_json::to_string(&bundle).expect("theme fixture should serialize")
}

#[cfg(test)]
mod tests {
    use super::{
        AppTheme, DEFAULT_DARK_THEME_KEY, DEFAULT_LIGHT_THEME_KEY, EMBEDDED_THEME_FILES,
        GRAPH_LANE_PALETTE_SIZE, GraphLanePalette, HexColor, Hsla, Rgba, THEME_SCHEMA_VERSION,
        ThemeColor, UNFILLED_COLOR_TOKENS, available_themes, content_header_bg,
        derived_syntax_color, fill_missing_color_tokens, has_theme_key, hsla_from_hue_fraction,
        load_theme_specs_from_json, merged_theme_options, resolved_runtime_themes_dir,
        runtime_themes_with_dir, test_theme_bundle_value, test_theme_json_with_syntax, theme_label,
        with_alpha,
    };
    use palette::IntoColor;
    use std::{fs, path::PathBuf};
    use tempfile::tempdir;

    fn themes_markdown_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/themes.md")
    }

    fn readme_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../README.md")
    }

    fn test_theme_entry(base_key: &str) -> serde_json::Value {
        test_theme_bundle_value(base_key)["themes"][0].take()
    }

    fn test_theme_bundle_json(name: &str, themes: Vec<serde_json::Value>) -> String {
        serde_json::to_string(&serde_json::json!({
            "schema_version": THEME_SCHEMA_VERSION,
            "name": name,
            "themes": themes,
        }))
        .expect("theme fixture should serialize")
    }

    fn themes_markdown_example() -> String {
        let markdown = fs::read_to_string(themes_markdown_path())
            .expect("THEMES.md should be readable for theme docs tests");
        let start = markdown
            .find("```javascript")
            .expect("THEMES.md should include a javascript example block");
        let example = &markdown[start + "```javascript".len()..];
        let end = example
            .find("```")
            .expect("THEMES.md example block should be closed");
        example[..end].trim().to_string()
    }

    fn strip_json_line_comments(json_with_comments: &str) -> String {
        let mut out = String::with_capacity(json_with_comments.len());
        let mut chars = json_with_comments.chars().peekable();
        let mut in_string = false;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if in_string {
                out.push(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            if ch == '"' {
                in_string = true;
                out.push(ch);
                continue;
            }

            if ch == '/' && chars.peek() == Some(&'/') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
                continue;
            }

            out.push(ch);
        }

        out
    }

    fn relative_luminance(color: Rgba) -> f32 {
        fn linear_channel(channel: f32) -> f32 {
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        }

        0.2126 * linear_channel(color.red)
            + 0.7152 * linear_channel(color.green)
            + 0.0722 * linear_channel(color.blue)
    }

    fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
        let a = relative_luminance(a);
        let b = relative_luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn assert_min_contrast(
        theme_key: &str,
        token: &str,
        foreground: Rgba,
        background: Rgba,
        minimum: f32,
    ) {
        let actual = contrast_ratio(foreground, background);
        assert!(
            actual >= minimum,
            "{theme_key} {token} contrast was {actual:.2}, expected at least {minimum:.2}"
        );
    }

    fn syntax_foregrounds(theme: AppTheme) -> Vec<(&'static str, Rgba)> {
        let syntax = theme.syntax;
        vec![
            ("comment", syntax.comment),
            ("comment_doc", syntax.comment_doc),
            ("string", syntax.string),
            ("string_escape", syntax.string_escape),
            ("string_regex", syntax.string_regex),
            ("string_special", syntax.string_special),
            ("keyword", syntax.keyword),
            ("keyword_control", syntax.keyword_control),
            ("preproc", syntax.preproc),
            ("number", syntax.number),
            ("boolean", syntax.boolean),
            ("function", syntax.function),
            ("function_method", syntax.function_method),
            ("function_special", syntax.function_special),
            ("constructor", syntax.constructor),
            ("type", syntax.type_name),
            ("type_builtin", syntax.type_builtin),
            ("type_interface", syntax.type_interface),
            ("namespace", syntax.namespace),
            (
                "variable",
                syntax.variable.unwrap_or(theme.colors.foreground.primary),
            ),
            ("variable_parameter", syntax.variable_parameter),
            ("variable_special", syntax.variable_special),
            ("variable_builtin", syntax.variable_builtin),
            ("property", syntax.property),
            (
                "label",
                syntax.label.unwrap_or(theme.colors.foreground.primary),
            ),
            ("constant", syntax.constant),
            ("constant_builtin", syntax.constant_builtin),
            ("operator", syntax.operator),
            ("punctuation", syntax.punctuation),
            ("punctuation_bracket", syntax.punctuation_bracket),
            ("punctuation_delimiter", syntax.punctuation_delimiter),
            ("punctuation_special", syntax.punctuation_special),
            ("punctuation_list_marker", syntax.punctuation_list_marker),
            ("tag", syntax.tag),
            ("attribute", syntax.attribute),
            ("markup_heading", syntax.markup_heading),
            ("markup_link", syntax.markup_link),
            ("text_literal", syntax.text_literal),
            ("diff_plus", syntax.diff_plus),
            ("diff_minus", syntax.diff_minus),
            ("diff_delta", syntax.diff_delta),
            ("lifetime", syntax.lifetime),
        ]
    }

    #[test]
    fn with_alpha_preserves_rgb_and_overwrites_alpha() {
        let color = Rgba::new(0.1, 0.2, 0.3, 0.4);

        let adjusted = with_alpha(color, 0.75);

        assert_eq!(adjusted.red, color.red);
        assert_eq!(adjusted.green, color.green);
        assert_eq!(adjusted.blue, color.blue);
        assert_eq!(adjusted.alpha, 0.75);
    }

    #[test]
    fn rejects_theme_bundle_without_schema_version() {
        // Carries a real theme, not `"themes": []`: an empty bundle parses
        // structurally whatever the schema, so it cannot tell whether the
        // version was checked before the structure or after it.
        let json = serde_json::to_string(&serde_json::json!({
            "name": "Missing version",
            "themes": [test_theme_entry(DEFAULT_DARK_THEME_KEY)],
        }))
        .expect("theme fixture should serialize");
        let error = load_theme_specs_from_json(&json)
            .err()
            .expect("a theme bundle without schema_version must be rejected");

        assert!(
            error.to_string().contains("schema_version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_unsupported_theme_schema_versions() {
        let json = serde_json::to_string(&serde_json::json!({
            "schema_version": 999,
            "name": "Future",
            "themes": [test_theme_entry(DEFAULT_DARK_THEME_KEY)],
        }))
        .expect("theme fixture should serialize");
        let error = load_theme_specs_from_json(&json)
            .err()
            .expect("unsupported schema version must be rejected");

        assert_eq!(
            error.to_string(),
            format!("unsupported theme schema version 999; expected {THEME_SCHEMA_VERSION}")
        );
    }

    /// The v1 -> v2 break reshaped the colour groups, and every struct in the
    /// chain is `deny_unknown_fields`. A v1 file therefore fails the structural
    /// parse first unless the version is read off the raw JSON, and the author
    /// gets `invalid type: string ..., expected struct ThemeFileAccentColors`
    /// instead of being told which version their file is.
    #[test]
    fn a_v1_theme_reports_its_version_rather_than_a_type_error() {
        let error = load_theme_specs_from_json(
            r##"{
                "schema_version": 1,
                "name": "Old",
                "themes": [{
                    "key": "old",
                    "name": "Old",
                    "appearance": "dark",
                    "colors": { "accent": "#59b7ffff", "background": "#101014ff" }
                }]
            }"##,
        )
        .err()
        .expect("a v1 theme must be rejected");

        assert_eq!(
            error.to_string(),
            format!("unsupported theme schema version 1; expected {THEME_SCHEMA_VERSION}")
        );
    }

    /// `AppTheme` is `Copy` and captured by value into every per-row paint
    /// closure, so its size is paid per visible row per frame. The bound is
    /// deliberately loose -- it is a tripwire for a field that brings a large
    /// array along, not a budget to tune against.
    #[test]
    fn app_theme_stays_small_enough_to_copy_per_row() {
        let size = std::mem::size_of::<AppTheme>();
        assert!(
            size <= 2048,
            "AppTheme grew to {size} bytes; interning large fields (see \
             `intern_lane_palette`) keeps per-row paint closures cheap"
        );
    }

    /// Two themes built from the same palette share one interned copy, so
    /// reloading a theme file does not leak another kilobyte each time.
    #[test]
    fn lane_palettes_are_interned_across_theme_loads() {
        let first = AppTheme::from_key(DEFAULT_DARK_THEME_KEY).expect("bundled theme should load");
        let reparsed = AppTheme::from_json_str(
            &serde_json::to_string(&test_theme_bundle_value(DEFAULT_DARK_THEME_KEY))
                .expect("fixture should serialize"),
        )
        .expect("fixture should parse");

        assert!(
            std::ptr::eq(first.graph_lane_palette, reparsed.graph_lane_palette),
            "an identical palette must reuse the interned one"
        );
    }

    /// The one exclusion in the token fill. A theme that names neither lane token
    /// must keep the palette generated for its appearance -- filling these from
    /// the bundled theme would repaint the graph of every custom theme in the
    /// wild with gitcomet's own lane colours.
    #[test]
    fn omitting_both_lane_tokens_generates_a_palette_instead_of_inheriting_one() {
        let bundled =
            AppTheme::from_key(DEFAULT_DARK_THEME_KEY).expect("bundled theme should load");
        let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        let colors = fixture["themes"][0]["colors"]
            .as_object_mut()
            .expect("colors should be an object");
        for token in UNFILLED_COLOR_TOKENS {
            assert!(
                colors.contains_key(*token),
                "fixture should start with {token} so removing it means something"
            );
            colors.remove(*token);
        }

        let theme = AppTheme::from_json_str(
            &serde_json::to_string(&fixture).expect("fixture should serialize"),
        )
        .expect("a theme without lane tokens should load");

        assert_eq!(
            theme.graph_lane_palette.as_slice(),
            GraphLanePalette::generated(true).as_slice(),
            "lane colours must be generated for the appearance, not inherited"
        );
        assert_ne!(
            theme.graph_lane_palette.as_slice(),
            bundled.graph_lane_palette.as_slice(),
            "and specifically not the bundled theme's hand-picked lanes"
        );
    }

    /// Interning must reuse an allocation only for palettes that are actually the
    /// same: a looser comparison would collapse every theme onto one palette.
    #[test]
    fn different_lane_palettes_are_interned_separately() {
        use serde_json::json;

        let theme_with_hues = |hues: serde_json::Value| {
            let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
            fixture["themes"][0]["colors"]["graph_lane_palette"] = serde_json::Value::Null;
            fixture["themes"][0]["colors"]["graph_lane_hues"] = hues;
            AppTheme::from_json_str(
                &serde_json::to_string(&fixture).expect("fixture should serialize"),
            )
            .expect("fixture should parse")
        };

        let one = theme_with_hues(json!([0.1, 0.2]));
        let other = theme_with_hues(json!([0.6, 0.8]));
        assert_ne!(
            one.graph_lane_palette.as_slice(),
            other.graph_lane_palette.as_slice(),
            "fixture must actually produce two different palettes"
        );
        assert!(
            !std::ptr::eq(one.graph_lane_palette, other.graph_lane_palette),
            "distinct palettes must not share an interned allocation"
        );
    }

    /// A palette carrying NaN (an out-of-range hue reaches `rem_euclid` as
    /// infinity) is never `PartialEq` to itself, so a `PartialEq`-based interner
    /// would leak a fresh copy on every parse.
    #[test]
    fn a_palette_with_non_finite_channels_still_interns_once() {
        use serde_json::json;

        let parse_nan_theme = || {
            let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
            fixture["themes"][0]["colors"]["graph_lane_palette"] = serde_json::Value::Null;
            fixture["themes"][0]["colors"]["graph_lane_hues"] = json!([1e40]);
            AppTheme::from_json_str(
                &serde_json::to_string(&fixture).expect("fixture should serialize"),
            )
            .expect("fixture should parse")
        };

        let first = parse_nan_theme();
        assert!(
            first
                .graph_lane_palette
                .as_slice()
                .iter()
                .any(|color| color.red.is_nan() || color.green.is_nan() || color.blue.is_nan()),
            "fixture must actually produce a non-finite channel"
        );
        assert!(
            std::ptr::eq(
                first.graph_lane_palette,
                parse_nan_theme().graph_lane_palette
            ),
            "re-parsing the same palette must reuse the interned copy, NaN or not"
        );
    }

    /// A theme file may leave tokens out; it may not misspell them. The first
    /// half is what keeps themes written against an older token set loading, the
    /// second is what stops a typo from quietly doing nothing.
    #[test]
    fn missing_semantic_tokens_fall_back_and_unknown_ones_are_rejected() {
        use serde_json::json;

        for (base_key, appearance) in [
            (DEFAULT_DARK_THEME_KEY, "dark"),
            (DEFAULT_LIGHT_THEME_KEY, "light"),
        ] {
            let bundled = AppTheme::from_key(base_key).expect("bundled theme should load");
            let mut missing = test_theme_bundle_value(base_key);
            missing["themes"][0]["colors"]["surface"]
                .as_object_mut()
                .expect("surface should be an object")
                .remove("input");
            // Not the bundled palette: a token filled from the wrong appearance
            // would still parse, so the fixture has to differ somewhere visible.
            missing["themes"][0]["colors"]["surface"]["canvas"] = json!("#123456ff");
            let filled = AppTheme::from_json_str(
                &serde_json::to_string(&missing).expect("fixture should serialize"),
            )
            .expect("a theme that omits a token should load");
            assert_eq!(
                filled.colors.surface.input, bundled.colors.surface.input,
                "omitted token should come from the bundled {appearance} theme"
            );
            assert_eq!(
                filled.colors.surface.canvas,
                gpui::rgba(0x123456ff),
                "a token the file does define must survive the fill"
            );
        }

        // A colour is filled or not filled whole. Half of one is still an error:
        // inheriting the bundled `alpha` would render a colour the author *did*
        // write at an opacity they never asked for, with nothing to show for it.
        let mut half_color = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        assert!(
            half_color["themes"][0]["colors"]["interaction"]["focus_background"]
                .get("alpha")
                .is_some(),
            "fixture token must use the {{hex, alpha}} form for this to mean anything"
        );
        half_color["themes"][0]["colors"]["interaction"]["focus_background"] =
            json!({ "hex": "#5ac1feff" });
        let half_error = AppTheme::from_json_str(
            &serde_json::to_string(&half_color).expect("fixture should serialize"),
        )
        .expect_err("a colour missing its alpha must fail rather than inherit one");
        assert!(
            half_error.to_string().contains("ThemeColor"),
            "unexpected error: {half_error}"
        );

        // Identity is never filled in: a file that omits its key would otherwise
        // inherit the bundled theme's and collide with it in the picker.
        let mut no_key = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        no_key["themes"][0]
            .as_object_mut()
            .expect("theme entry should be an object")
            .remove("key");
        let key_error = AppTheme::from_json_str(
            &serde_json::to_string(&no_key).expect("fixture should serialize"),
        )
        .expect_err("a theme without a key must fail");
        assert!(
            key_error.to_string().contains("missing field `key`"),
            "unexpected error: {key_error}"
        );

        let mut unknown = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        unknown["themes"][0]["colors"]["surface"]["mystery"] = json!("#000000ff");
        let unknown_error = AppTheme::from_json_str(
            &serde_json::to_string(&unknown).expect("fixture should serialize"),
        )
        .expect_err("unknown semantic token must fail");
        assert!(
            unknown_error
                .to_string()
                .contains("unknown field `mystery`"),
            "unexpected error: {unknown_error}"
        );
    }

    #[test]
    fn parses_theme_json_with_alpha_overrides() {
        use serde_json::json;

        let mut fixture = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        let theme = &mut fixture["themes"][0];
        theme["key"] = json!("fixture");
        theme["name"] = json!("Fixture");
        theme["colors"]["surface"]["canvas"] = json!("#0d1016ff");
        theme["colors"]["stroke"]["default"] = json!("#2d2f34ff");
        theme["colors"]["tooltip"]["background"] = json!("#000000ff");
        theme["colors"]["tooltip"]["foreground"] = json!("#ffffffff");
        theme["colors"]["interaction"]["pressed_background"] =
            json!({ "hex": "#2d2f34ff", "alpha": 0.78 });
        theme["colors"]["scrollbar"]["thumb_pressed"] =
            json!({ "hex": "#8a8986ff", "alpha": 0.52 });
        theme["colors"]["diff"]["added"]["background"] = json!("#102030ff");
        theme["colors"]["diff"]["added"]["foreground"] = json!("#405060ff");
        theme["colors"]["diff"]["removed"]["background"] = json!("#203040ff");
        theme["colors"]["diff"]["removed"]["foreground"] = json!("#506070ff");
        theme["colors"]["foreground"]["placeholder"] = json!("#708090ff");
        theme["colors"]["accent"]["on_solid"] = json!("#112233ff");
        theme["colors"]["foreground"]["emphasis"] = json!("#a1b2c3ff");
        theme["colors"]["graph_lane_palette"] = serde_json::Value::Null;
        theme["colors"]["graph_lane_hues"] = json!([0.25, 0.75]);
        theme["radii"] = json!({ "panel": 2.0, "pill": 2.0, "row": 2.0 });
        theme
            .as_object_mut()
            .expect("theme should be an object")
            .remove("syntax");

        let theme = AppTheme::from_json_str(
            &serde_json::to_string(&fixture).expect("fixture should serialize"),
        )
        .expect("theme JSON should parse");

        assert!(theme.is_dark);
        assert_eq!(theme.colors.surface.canvas, gpui::rgba(0x0d1016ff));
        assert_eq!(theme.colors.stroke.default, gpui::rgba(0x2d2f34ff));
        assert_eq!(theme.colors.tooltip.background, gpui::rgba(0x000000ff));
        assert_eq!(theme.colors.tooltip.foreground, gpui::rgba(0xffffffff));
        assert_eq!(
            theme.colors.interaction.pressed_background,
            with_alpha(gpui::rgba(0x2d2f34ff), 0.78)
        );
        assert_eq!(
            theme.colors.scrollbar.thumb_pressed,
            with_alpha(gpui::rgba(0x8a8986ff), 0.52)
        );
        assert_eq!(theme.colors.diff.added.background, gpui::rgba(0x102030ff));
        assert_eq!(theme.colors.diff.added.foreground, gpui::rgba(0x405060ff));
        assert_eq!(theme.colors.diff.removed.background, gpui::rgba(0x203040ff));
        assert_eq!(theme.colors.diff.removed.foreground, gpui::rgba(0x506070ff));
        assert_eq!(theme.colors.foreground.placeholder, gpui::rgba(0x708090ff));
        assert_eq!(theme.colors.accent.on_solid, gpui::rgba(0x112233ff));
        assert_eq!(theme.colors.foreground.emphasis, gpui::rgba(0xa1b2c3ff));
        assert_eq!(theme.graph_lane_palette.as_slice().len(), 2);
        assert_eq!(
            theme.graph_lane_palette.as_slice()[0],
            hsla_from_hue_fraction(0.25, 0.75, 0.62, 1.0).into_color()
        );
        assert_eq!(theme.syntax.comment, theme.colors.foreground.secondary);
        assert_eq!(
            theme.syntax.keyword,
            derived_syntax_color(theme.is_dark, &theme.colors, theme.colors.accent.foreground)
        );
        assert_eq!(theme.syntax.variable, None);
        assert_eq!(theme.radii.panel, 2.0);
    }

    #[test]
    fn short_custom_lane_palettes_wrap_instead_of_painting_transparent_lanes() {
        let palette = GraphLanePalette::from_theme_colors(
            true,
            Some(vec![
                ThemeColor::Hex(HexColor(gpui::rgba(0x112233ff))),
                ThemeColor::Hex(HexColor(gpui::rgba(0x445566ff))),
                ThemeColor::Hex(HexColor(gpui::rgba(0x778899ff))),
            ]),
            None,
        );
        assert_eq!(palette.as_slice().len(), 3);

        // Lane indices are handed out cyclically over the full palette size, so
        // a three-colour theme must keep reusing its own colours rather than
        // reading the transparent tail of the backing array.
        for ix in 0..(GRAPH_LANE_PALETTE_SIZE as u8) {
            let color = palette.color_at(ix);
            assert_eq!(color, palette.as_slice()[usize::from(ix) % 3]);
            assert!(color.alpha > 0.0, "lane {ix} would be invisible");
        }
    }

    #[test]
    fn generated_lane_palette_matches_the_theme_ramp() {
        // `lane_color` reads the theme palette; themes that ship no explicit
        // lane colours must fall back to the generated ramp.
        for is_dark in [true, false] {
            let palette = GraphLanePalette::generated(is_dark);
            let light = if is_dark { 0.62 } else { 0.33 };
            for ix in 0..(GRAPH_LANE_PALETTE_SIZE as u8) {
                let hue = (f32::from(ix) * 0.13) % 1.0;
                assert_eq!(
                    palette.color_at(ix),
                    hsla_from_hue_fraction(hue, 0.75, light, 1.0).into_color(),
                    "lane {ix} (is_dark={is_dark})"
                );
            }
        }
    }

    /// The ramp tests above compare [`hsla_from_hue_fraction`] against itself,
    /// so they hold just as well when every lane is the same colour. This pins
    /// the mapping itself: a hue fraction has to reach the primary it names.
    ///
    /// `gpui::hsla` fails this — it clamps the fraction and then reads it as
    /// degrees, so 1/3 and 2/3 both come back red.
    #[test]
    fn hue_fractions_reach_the_primaries_they_name() {
        let cases = [
            (0.0 / 3.0, [1.0, 0.0, 0.0]),
            (1.0 / 3.0, [0.0, 1.0, 0.0]),
            (2.0 / 3.0, [0.0, 0.0, 1.0]),
            // A hue is cyclic, so a full turn is the same red it started on.
            (3.0 / 3.0, [1.0, 0.0, 0.0]),
        ];

        for (hue, [r, g, b]) in cases {
            let color: Rgba = hsla_from_hue_fraction(hue, 1.0, 0.5, 1.0).into_color();
            let actual = [color.red, color.green, color.blue];
            for (channel, (actual, expected)) in actual.iter().zip([r, g, b]).enumerate() {
                assert!(
                    (actual - expected).abs() < 1e-4,
                    "hue {hue}: channel {channel} was {actual}, expected {expected} \
                     (got {actual:?} for the whole colour)"
                );
            }
        }
    }

    /// Distinct authors must land on distinct hues, not four shades of one.
    #[test]
    fn author_colors_spread_across_the_hue_wheel() {
        let theme = AppTheme::gitcomet_dark();
        let names = [
            "Ada Lovelace",
            "Grace Hopper",
            "Alan Turing",
            "Barbara Liskov",
        ];
        let mut hues = names.map(|name| {
            let color: Hsla = crate::view::components::author_color(theme, name).into_color();
            color.hue.into_positive_degrees()
        });
        hues.sort_by(f32::total_cmp);

        // Under the bug this guards, every hue lands under one degree, so the
        // whole spread collapses and adjacent authors become indistinguishable.
        let spread = hues[hues.len() - 1] - hues[0];
        assert!(
            spread > 90.0,
            "author hues span only {spread}°, so they have collapsed onto one colour: {hues:?}"
        );
        for pair in hues.windows(2) {
            assert!(
                pair[1] - pair[0] > 1.0,
                "author hues {:?} and {:?} are within a degree of each other: {hues:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn parses_theme_json_with_optional_syntax_overrides() {
        let theme = AppTheme::from_json_str(&test_theme_json_with_syntax(
            DEFAULT_LIGHT_THEME_KEY,
            r##"{
                "keyword": "#112233ff",
                "variable": "#445566ff",
                "comment_doc": "#778899ff",
                "diff_plus": "#aabbccff",
                "label": "#998877ff"
            }"##,
        ))
        .expect("theme JSON should parse");

        assert_eq!(theme.syntax.keyword, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.variable, Some(gpui::rgba(0x445566ff)));
        assert_eq!(theme.syntax.comment_doc, gpui::rgba(0x778899ff));
        assert_eq!(theme.syntax.diff_plus, gpui::rgba(0xaabbccff));
        assert_eq!(theme.syntax.label, Some(gpui::rgba(0x998877ff)));
        assert_eq!(theme.syntax.comment, theme.colors.foreground.secondary);
        assert_eq!(
            theme.syntax.string,
            derived_syntax_color(
                theme.is_dark,
                &theme.colors,
                theme.colors.status.warning.foreground
            )
        );
    }

    #[test]
    fn specialized_syntax_categories_fallback_to_base_categories() {
        let theme = AppTheme::from_json_str(&test_theme_json_with_syntax(
            DEFAULT_LIGHT_THEME_KEY,
            r##"{
                "string": "#112233ff",
                "keyword": "#223344ff",
                "type": "#334455ff",
                "variable": "#445566ff",
                "variable_special": "#556677ff",
                "constant": "#667788ff",
                "punctuation": "#778899ff"
            }"##,
        ))
        .expect("theme JSON should parse");

        assert_eq!(theme.syntax.string_regex, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.string_special, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.preproc, gpui::rgba(0x223344ff));
        assert_eq!(theme.syntax.namespace, gpui::rgba(0x334455ff));
        assert_eq!(theme.syntax.label, Some(gpui::rgba(0x445566ff)));
        assert_eq!(theme.syntax.variable_builtin, gpui::rgba(0x556677ff));
        assert_eq!(theme.syntax.constant_builtin, gpui::rgba(0x667788ff));
        assert_eq!(theme.syntax.punctuation_special, gpui::rgba(0x778899ff));
        assert_eq!(theme.syntax.punctuation_list_marker, gpui::rgba(0x778899ff));
        assert_eq!(theme.syntax.markup_heading, gpui::rgba(0x223344ff));
        assert_eq!(theme.syntax.markup_link, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.text_literal, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.diff_plus, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.diff_minus, gpui::rgba(0x223344ff));
        assert_eq!(theme.syntax.diff_delta, gpui::rgba(0x334455ff));
    }

    #[test]
    fn specialized_syntax_overrides_beat_base_category_fallbacks() {
        let theme = AppTheme::from_json_str(&test_theme_json_with_syntax(
            DEFAULT_DARK_THEME_KEY,
            r##"{
                "string": "#111111ff",
                "keyword": "#222222ff",
                "type": "#333333ff",
                "variable": "#444444ff",
                "variable_special": "#555555ff",
                "constant": "#666666ff",
                "punctuation": "#777777ff",
                "function": "#888888ff",
                "string_regex": "#010101ff",
                "string_special": "#020202ff",
                "preproc": "#030303ff",
                "constructor": "#040404ff",
                "namespace": "#050505ff",
                "variable_builtin": "#060606ff",
                "label": "#070707ff",
                "constant_builtin": "#080808ff",
                "punctuation_special": "#090909ff",
                "punctuation_list_marker": "#0a0a0aff",
                "markup_heading": "#0b0b0bff",
                "markup_link": "#0c0c0cff",
                "text_literal": "#0d0d0dff",
                "diff_plus": "#0e0e0eff",
                "diff_minus": "#0f0f0fff",
                "diff_delta": "#101010ff"
            }"##,
        ))
        .expect("theme JSON should parse");

        assert_eq!(theme.syntax.string_regex, gpui::rgba(0x010101ff));
        assert_eq!(theme.syntax.string_special, gpui::rgba(0x020202ff));
        assert_eq!(theme.syntax.preproc, gpui::rgba(0x030303ff));
        assert_eq!(theme.syntax.constructor, gpui::rgba(0x040404ff));
        assert_eq!(theme.syntax.namespace, gpui::rgba(0x050505ff));
        assert_eq!(theme.syntax.variable_builtin, gpui::rgba(0x060606ff));
        assert_eq!(theme.syntax.label, Some(gpui::rgba(0x070707ff)));
        assert_eq!(theme.syntax.constant_builtin, gpui::rgba(0x080808ff));
        assert_eq!(theme.syntax.punctuation_special, gpui::rgba(0x090909ff));
        assert_eq!(theme.syntax.punctuation_list_marker, gpui::rgba(0x0a0a0aff));
        assert_eq!(theme.syntax.markup_heading, gpui::rgba(0x0b0b0bff));
        assert_eq!(theme.syntax.markup_link, gpui::rgba(0x0c0c0cff));
        assert_eq!(theme.syntax.text_literal, gpui::rgba(0x0d0d0dff));
        assert_eq!(theme.syntax.diff_plus, gpui::rgba(0x0e0e0eff));
        assert_eq!(theme.syntax.diff_minus, gpui::rgba(0x0f0f0fff));
        assert_eq!(theme.syntax.diff_delta, gpui::rgba(0x101010ff));
    }

    #[test]
    fn loads_theme_json_from_file() {
        let dir = tempdir().expect("temp dir should exist");
        let path = dir.path().join("theme.json");
        let fixture = test_theme_bundle_value(DEFAULT_LIGHT_THEME_KEY);
        fs::write(
            &path,
            serde_json::to_string(&fixture).expect("fixture should serialize"),
        )
        .expect("theme file should be written");

        let theme = AppTheme::from_json_path(&path).expect("theme file should load");

        assert!(!theme.is_dark);
        assert_eq!(theme.colors.surface.canvas, gpui::rgba(0xffffffff));
        assert_eq!(theme.colors.foreground.primary, gpui::rgba(0x111827ff));
        assert_eq!(
            theme.graph_lane_palette.as_slice().len(),
            GRAPH_LANE_PALETTE_SIZE
        );
    }

    #[test]
    fn built_in_themes_load_from_embedded_json() {
        let dark = AppTheme::gitcomet_dark();
        let light = AppTheme::gitcomet_light();

        assert!(dark.is_dark);
        assert!(!light.is_dark);
        assert_eq!(
            dark.colors.interaction.focus_ring,
            with_alpha(gpui::rgba(0x4f8ef7ff), 0.55)
        );
        assert_eq!(light.colors.surface.canvas, gpui::rgba(0xffffffff));
        assert_eq!(light.colors.surface.panel, gpui::rgba(0xf2f4f7ff));
        assert_eq!(light.colors.surface.raised, gpui::rgba(0xf8fafcff));
        assert_eq!(light.colors.surface.chrome, gpui::rgba(0xdfe3eaff));
        assert_eq!(light.colors.stroke.default, gpui::rgba(0xaeb7c4ff));
        assert_eq!(light.colors.foreground.primary, gpui::rgba(0x111827ff));
        assert_eq!(light.colors.foreground.secondary, gpui::rgba(0x465166ff));
        assert_eq!(light.colors.accent.foreground, gpui::rgba(0x365bb7ff));
        assert_eq!(
            light.colors.scrollbar.thumb_hover,
            with_alpha(gpui::rgba(0x465166ff), 0.52)
        );
        assert_eq!(dark.colors.diff.added.background, gpui::rgba(0x102a1cff));
        assert_eq!(light.colors.diff.removed.foreground, gpui::rgba(0xa52a35ff));
        assert_eq!(dark.colors.foreground.placeholder, gpui::rgba(0x6f7683ff));
        assert_eq!(light.colors.accent.on_solid, gpui::rgba(0xffffffff));
        assert_eq!(dark.colors.foreground.emphasis, gpui::rgba(0xffffffff));
        assert_eq!(light.colors.foreground.emphasis, gpui::rgba(0x000000ff));
        assert_eq!(dark.syntax.comment, gpui::rgba(0x6f7b94ff));
        assert_eq!(dark.syntax.keyword, gpui::rgba(0xedb981ff));
        assert_eq!(dark.syntax.keyword_control, dark.syntax.keyword);
        assert_eq!(dark.syntax.preproc, gpui::rgba(0xa79aebff));
        assert_eq!(dark.syntax.string, gpui::rgba(0xbbd57fff));
        assert_eq!(dark.syntax.string_regex, dark.syntax.string);
        assert_eq!(dark.syntax.function_method, gpui::rgba(0x5ac1feff));
        assert_eq!(dark.syntax.function_special, dark.syntax.function_method);
        assert_eq!(dark.syntax.property, dark.syntax.function_method);
        assert_eq!(dark.syntax.namespace, dark.syntax.function_method);
        assert_eq!(dark.syntax.markup_link, dark.syntax.function_method);
        assert_eq!(dark.syntax.type_name, gpui::rgba(0xbbd57fff));
        assert_eq!(dark.syntax.type_builtin, dark.syntax.type_name);
        assert_eq!(dark.syntax.number, gpui::rgba(0xe4a688ff));
        assert_eq!(dark.syntax.constant, gpui::rgba(0xde9fc1ff));
        assert_eq!(dark.syntax.constant_builtin, dark.syntax.constant);
        assert_eq!(dark.syntax.variable, Some(dark.colors.foreground.primary));
        assert_eq!(
            dark.syntax.variable_parameter,
            dark.colors.foreground.primary
        );
        assert_eq!(dark.syntax.variable_special, dark.colors.foreground.primary);
        assert_eq!(dark.syntax.operator, gpui::rgba(0x8d96aaff));
        assert_eq!(dark.syntax.punctuation, dark.syntax.operator);
        assert_eq!(dark.syntax.diff_delta, dark.syntax.function_method);
        assert_eq!(dark.syntax.diff_plus, gpui::rgba(0xbbf7d0ff));
        assert_eq!(dark.syntax.diff_minus, gpui::rgba(0xfecacaff));
        assert_eq!(light.syntax.comment, gpui::rgba(0x4b556aff));
        assert_eq!(light.syntax.keyword, gpui::rgba(0x7f470cff));
        assert_eq!(light.syntax.keyword_control, light.syntax.keyword);
        assert_eq!(light.syntax.preproc, gpui::rgba(0x5745a7ff));
        assert_eq!(light.syntax.string, gpui::rgba(0x455c0eff));
        assert_eq!(light.syntax.string_special, light.syntax.string);
        assert_eq!(light.syntax.function, gpui::rgba(0x005b80ff));
        assert_eq!(light.syntax.function_method, light.syntax.function);
        assert_eq!(light.syntax.function_special, light.syntax.function);
        assert_eq!(light.syntax.property, light.syntax.function);
        assert_eq!(light.syntax.namespace, light.syntax.function);
        assert_eq!(light.syntax.markup_link, light.syntax.function);
        assert_eq!(light.syntax.type_name, gpui::rgba(0x455c0eff));
        assert_eq!(light.syntax.type_builtin, light.syntax.type_name);
        assert_eq!(light.syntax.constructor, light.syntax.function);
        assert_eq!(light.syntax.constant, gpui::rgba(0x7c4261ff));
        assert_eq!(light.syntax.constant_builtin, light.syntax.constant);
        assert_eq!(light.syntax.number, gpui::rgba(0x814431ff));
        assert_eq!(light.syntax.variable, Some(light.colors.foreground.primary));
        assert_eq!(
            light.syntax.variable_parameter,
            light.colors.foreground.primary
        );
        assert_eq!(
            light.syntax.variable_special,
            light.colors.foreground.primary
        );
        assert_eq!(light.syntax.operator, gpui::rgba(0x49556bff));
        assert_eq!(light.syntax.punctuation, light.syntax.operator);
        assert_eq!(light.syntax.diff_delta, light.syntax.function);
        assert_eq!(
            dark.graph_lane_palette.as_slice().len(),
            GRAPH_LANE_PALETTE_SIZE
        );
    }

    #[test]
    fn dark_semantic_tokens_preserve_established_resolved_colors() {
        let theme = AppTheme::gitcomet_dark();
        let colors = theme.colors;

        assert_eq!(colors.surface.canvas, gpui::rgba(0x17191eff));
        assert_eq!(colors.surface.chrome, gpui::rgba(0x21242cff));
        assert_eq!(colors.surface.panel, gpui::rgba(0x1d2026ff));
        assert_eq!(colors.surface.raised, gpui::rgba(0x242831ff));
        assert_eq!(colors.interaction.hover_background, gpui::rgba(0x232733ff));
        assert_eq!(
            colors.interaction.pressed_background,
            with_alpha(gpui::rgba(0x2c3242ff), 0.80)
        );
        assert_eq!(
            colors.interaction.selected_background,
            gpui::rgba(0x2c3242ff)
        );
        assert_eq!(colors.accent.foreground, gpui::rgba(0x4f8ef7ff));
        assert_eq!(colors.status.danger.foreground, gpui::rgba(0xf0625dff));
        assert_eq!(colors.status.warning.foreground, gpui::rgba(0xf2a53aff));
        assert_eq!(colors.status.success.foreground, gpui::rgba(0x33c06bff));
        assert_eq!(colors.diff.added.background, gpui::rgba(0x102a1cff));
        assert_eq!(colors.diff.removed.background, gpui::rgba(0x33141aff));
        assert_eq!(colors.tooltip.background, gpui::rgba(0x242831ff));
    }

    #[test]
    fn built_in_tokyo_night_theme_loads_from_embedded_json() {
        let theme = AppTheme::from_key("tokyo_night").expect("Tokyo Night theme should load");

        assert!(theme.is_dark);
        assert_eq!(theme.colors.surface.canvas, gpui::rgba(0x1a1b26ff));
        assert_eq!(theme.colors.foreground.emphasis, gpui::rgba(0xffffffff));
        assert_eq!(theme.syntax.keyword, gpui::rgba(0xbb9af7ff));
        assert_eq!(theme.syntax.string, gpui::rgba(0x9ece6aff));
        assert_eq!(theme.syntax.string_regex, gpui::rgba(0xff9e64ff));
        assert_eq!(theme.syntax.diff_minus, gpui::rgba(0xf7768eff));
        assert_eq!(theme.syntax.variable, Some(gpui::rgba(0xc0caf5ff)));
    }

    #[test]
    fn built_in_sunset_veil_theme_loads_from_embedded_json() {
        let theme = AppTheme::from_key("sunset_veil").expect("Sunset Veil theme should load");

        assert!(!theme.is_dark);
        assert_eq!(theme.colors.surface.canvas, gpui::rgba(0xfff7edff));
        assert_eq!(theme.colors.surface.chrome, gpui::rgba(0xe6d8c9ff));
        assert_eq!(theme.colors.accent.foreground, gpui::rgba(0x854718ff));
        assert_eq!(theme.colors.diff.added.foreground, gpui::rgba(0x2f682bff));
        assert_eq!(theme.syntax.keyword, gpui::rgba(0x22586aff));
        assert_eq!(theme.syntax.markup_heading, gpui::rgba(0x26586aff));
        assert_eq!(theme.syntax.diff_plus, gpui::rgba(0x225d2bff));
        assert_eq!(theme.syntax.variable, Some(gpui::rgba(0x211a14ff)));
        assert_eq!(theme_label("sunset_veil"), Some("Sunset Veil".to_string()));
    }

    #[test]
    fn bundled_themes_keep_the_canvas_and_chrome_hierarchy_for_their_appearance() {
        assert_eq!(
            AppTheme::gitcomet_light().colors.surface.canvas,
            gpui::rgba(0xffffffff),
            "GitComet Light should keep its pure-white canvas"
        );
        assert_eq!(
            AppTheme::from_key("sunset_veil")
                .expect("Sunset Veil theme should load")
                .colors
                .surface
                .canvas,
            gpui::rgba(0xfff7edff),
            "Sunset Veil should use a warm light-orange canvas"
        );

        for key in ["gitcomet_light", "sunset_veil"] {
            let theme = AppTheme::from_key(key).expect("light theme should load");
            let colors = theme.colors;

            assert!(
                relative_luminance(colors.surface.chrome)
                    < relative_luminance(colors.surface.panel),
                "{key}: surrounding chrome should be darker than panel surfaces"
            );
            assert!(
                relative_luminance(colors.surface.panel)
                    < relative_luminance(colors.surface.raised),
                "{key}: elevated surfaces should remain distinguishable"
            );
            assert!(
                relative_luminance(colors.surface.raised)
                    < relative_luminance(colors.surface.canvas),
                "{key}: the main canvas should remain the brightest area"
            );
        }

        for key in ["gitcomet_dark", "tokyo_night"] {
            let theme = AppTheme::from_key(key).expect("dark theme should load");
            assert!(
                relative_luminance(theme.colors.surface.canvas)
                    < relative_luminance(theme.colors.surface.chrome),
                "{key}: surrounding chrome should remain lighter than the dark canvas"
            );
        }
    }

    #[test]
    fn bundled_light_theme_foregrounds_have_strong_canvas_contrast() {
        for key in ["gitcomet_light", "sunset_veil"] {
            let theme = AppTheme::from_key(key).expect("light theme should load");
            let colors = theme.colors;
            let canvas = colors.surface.canvas;

            for (token, color, minimum) in [
                ("primary", colors.foreground.primary, 7.0),
                ("secondary", colors.foreground.secondary, 4.5),
                ("accent", colors.accent.foreground, 4.5),
                ("danger", colors.status.danger.foreground, 4.5),
                ("warning", colors.status.warning.foreground, 4.5),
                ("success", colors.status.success.foreground, 4.5),
                ("diff.added", colors.diff.added.foreground, 4.5),
                ("diff.removed", colors.diff.removed.foreground, 4.5),
            ] {
                assert_min_contrast(key, token, color, canvas, minimum);
            }

            for (surface_name, surface) in [
                ("canvas", colors.surface.canvas),
                ("chrome", colors.surface.chrome),
                ("panel", colors.surface.panel),
                ("raised", colors.surface.raised),
                ("input", colors.surface.input),
            ] {
                assert_min_contrast(
                    key,
                    &format!("primary/{surface_name}"),
                    colors.foreground.primary,
                    surface,
                    7.0,
                );
                assert_min_contrast(
                    key,
                    &format!("secondary/{surface_name}"),
                    colors.foreground.secondary,
                    surface,
                    4.5,
                );
            }

            assert_min_contrast(
                key,
                "accent.on_solid",
                colors.accent.on_solid,
                colors.accent.solid,
                4.5,
            );
            for (name, set) in [
                ("status.info", colors.status.info),
                ("status.success", colors.status.success),
                ("status.warning", colors.status.warning),
                ("status.danger", colors.status.danger),
            ] {
                assert_min_contrast(key, name, set.foreground, set.background, 4.5);
                assert_min_contrast(
                    key,
                    &format!("{name}.border"),
                    set.border,
                    set.background,
                    3.0,
                );
            }
            for (name, set) in [
                ("diff.added", colors.diff.added),
                ("diff.removed", colors.diff.removed),
                ("diff.modified", colors.diff.modified),
            ] {
                assert_min_contrast(key, name, set.foreground, set.background, 4.5);
                assert_min_contrast(
                    key,
                    &format!("{name}.word"),
                    set.foreground,
                    set.word_background,
                    4.5,
                );
            }
            assert_min_contrast(
                key,
                "stroke.control",
                colors.stroke.control,
                colors.surface.input,
                3.0,
            );
            assert_min_contrast(
                key,
                "focus_ring",
                colors.interaction.focus_ring,
                colors.surface.input,
                3.0,
            );
            assert_min_contrast(
                key,
                "selected_indicator",
                colors.interaction.selected_indicator,
                colors.interaction.selected_background,
                3.0,
            );

            for (token, color) in syntax_foregrounds(theme) {
                assert_min_contrast(
                    key,
                    &format!("syntax.{token}/editor"),
                    color,
                    colors.editor.background,
                    7.0,
                );

                for (surface_name, surface) in [
                    ("editor.selection", colors.editor.selection_background),
                    ("editor.search_match", colors.editor.search_match_background),
                    (
                        "editor.bracket_match",
                        colors.editor.bracket_match_background,
                    ),
                ] {
                    assert_min_contrast(
                        key,
                        &format!("syntax.{token}/{surface_name}"),
                        color,
                        surface,
                        5.5,
                    );
                }

                for (surface_name, surface) in [
                    ("diff.added", colors.diff.added.background),
                    ("diff.added.word", colors.diff.added.word_background),
                    ("diff.removed", colors.diff.removed.background),
                    ("diff.removed.word", colors.diff.removed.word_background),
                    ("diff.modified", colors.diff.modified.background),
                    ("diff.modified.word", colors.diff.modified.word_background),
                ] {
                    assert_min_contrast(
                        key,
                        &format!("syntax.{token}/{surface_name}"),
                        color,
                        surface,
                        6.0,
                    );
                }
            }

            for (index, color) in theme.graph_lane_palette.as_slice().iter().enumerate() {
                assert_min_contrast(
                    key,
                    &format!("graph_lane_palette[{index}]"),
                    *color,
                    canvas,
                    3.0,
                );
            }
        }
    }

    #[test]
    fn content_header_bg_matches_the_canvas_on_dark_and_is_distinct_on_light() {
        for key in ["gitcomet_dark", "tokyo_night"] {
            let theme = AppTheme::from_key(key).expect("dark theme should load");
            assert_eq!(
                content_header_bg(theme),
                theme.colors.surface.canvas,
                "{key}: header band should be the canvas color"
            );
        }

        for key in ["gitcomet_light", "sunset_veil"] {
            let theme = AppTheme::from_key(key).expect("light theme should load");
            assert_eq!(
                content_header_bg(theme),
                theme.colors.surface.raised,
                "{key}: header band should stay raised"
            );
        }
    }

    #[test]
    fn bundled_theme_assets_explicitly_define_new_syntax_keys() {
        const REQUIRED_KEYS: &[&str] = &[
            "\"string_regex\"",
            "\"string_special\"",
            "\"preproc\"",
            "\"constructor\"",
            "\"namespace\"",
            "\"variable_builtin\"",
            "\"label\"",
            "\"constant_builtin\"",
            "\"punctuation_special\"",
            "\"punctuation_list_marker\"",
            "\"markup_heading\"",
            "\"markup_link\"",
            "\"text_literal\"",
            "\"diff_plus\"",
            "\"diff_minus\"",
            "\"diff_delta\"",
        ];

        for file in EMBEDDED_THEME_FILES {
            for key in REQUIRED_KEYS {
                assert!(
                    file.json.contains(key),
                    "embedded theme file {} should explicitly define {}",
                    file.stem,
                    key
                );
            }
        }
    }

    /// Dotted paths of every token `filled` gained over `original`.
    fn collect_filled_token_paths(
        original: &serde_json::Value,
        filled: &serde_json::Value,
        path: &mut String,
        out: &mut Vec<String>,
    ) {
        let Some(filled) = filled.as_object() else {
            return;
        };
        for (key, filled_value) in filled {
            let original_value = original.get(key);
            let len = path.len();
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(key);
            match original_value {
                None => out.push(path.clone()),
                Some(original_value) => {
                    collect_filled_token_paths(original_value, filled_value, path, out)
                }
            }
            path.truncate(len);
        }
    }

    /// The colour-token counterpart of the syntax-key guard above.
    ///
    /// Before `fill_missing_color_tokens` existed, a bundled theme missing a
    /// colour token was a parse error that `embedded_theme_cache` panicked on, so
    /// an incomplete one could not ship. The fill exists for *custom* themes
    /// written against an older token set; applied to the bundled themes it
    /// quietly hands them colours tuned against a different palette. Asserting
    /// the fill is a no-op restores the startup guarantee without exempting them
    /// from it.
    #[test]
    fn bundled_themes_explicitly_define_every_color_token() {
        for file in EMBEDDED_THEME_FILES {
            let bundle: serde_json::Value = serde_json::from_str(file.json).unwrap_or_else(|err| {
                panic!("embedded theme file {} is not JSON: {err}", file.stem)
            });
            let themes = bundle["themes"]
                .as_array()
                .unwrap_or_else(|| panic!("embedded theme file {} has no themes", file.stem));

            for theme in themes {
                let key = theme["key"].as_str().unwrap_or("<unnamed>").to_string();
                let before = theme["colors"].clone();
                let mut filled = serde_json::json!({ "themes": [theme.clone()] });
                fill_missing_color_tokens(&mut filled);

                // Reported as paths rather than by diffing the two objects: the
                // palettes are large enough that an `assert_eq!` dump buries the
                // one token that is actually missing.
                let mut missing = Vec::new();
                collect_filled_token_paths(
                    &before,
                    &filled["themes"][0]["colors"],
                    &mut String::new(),
                    &mut missing,
                );

                assert!(
                    missing.is_empty(),
                    "bundled theme {key} in {} inherits {} from another theme; define them \
                     explicitly",
                    file.stem,
                    missing.join(", ")
                );
            }
        }
    }

    #[test]
    fn bundled_theme_file_exposes_multiple_themes() {
        use serde_json::json;

        let mut light = test_theme_entry(DEFAULT_LIGHT_THEME_KEY);
        light["key"] = json!("classic_light");
        light["name"] = json!("Classic Light");

        let mut dark = test_theme_entry(DEFAULT_DARK_THEME_KEY);
        dark["key"] = json!("classic_dark");
        dark["name"] = json!("Classic Dark");

        let json = test_theme_bundle_json("Classic", vec![light, dark]);
        let specs = load_theme_specs_from_json(&json).expect("bundle should parse");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].option.key, "classic_light");
        assert_eq!(specs[0].option.label, "Classic Light");
        assert!(!specs[0].theme.is_dark);
        assert_eq!(specs[1].option.key, "classic_dark");
        assert_eq!(specs[1].option.label, "Classic Dark");
        assert!(specs[1].theme.is_dark);
    }
    #[test]
    fn embedded_theme_registry_exposes_default_keys() {
        let themes = available_themes();

        assert!(!themes.is_empty());
        assert!(has_theme_key(DEFAULT_DARK_THEME_KEY));
        assert!(has_theme_key(DEFAULT_LIGHT_THEME_KEY));
        assert_eq!(
            theme_label(DEFAULT_DARK_THEME_KEY),
            Some("GitComet Dark".to_string())
        );
        assert_eq!(
            theme_label(DEFAULT_LIGHT_THEME_KEY),
            Some("GitComet Light".to_string())
        );
    }

    #[test]
    fn ensure_runtime_theme_dir_creates_missing_directory() {
        let dir = tempdir().expect("temp dir should exist");
        let path = dir.path().join("themes");

        assert!(!path.exists(), "theme subdirectory should start absent");

        let resolved = resolved_runtime_themes_dir(Some(&path))
            .expect("runtime theme helper should resolve a writable directory");

        assert_eq!(resolved, path);
        assert!(resolved.is_dir(), "theme directory should be created");
    }

    #[test]
    fn runtime_theme_dir_extends_embedded_themes_with_custom_entries() {
        use serde_json::json;

        let dir = tempdir().expect("temp dir should exist");
        let mut custom = test_theme_entry(DEFAULT_DARK_THEME_KEY);
        custom["key"] = json!("custom_theme");
        custom["name"] = json!("Custom Theme");
        fs::write(
            dir.path().join("custom_theme.json"),
            test_theme_bundle_json("Custom Theme", vec![custom]),
        )
        .expect("custom theme file should be written");

        let themes = merged_theme_options(Some(dir.path()));
        let custom = themes
            .iter()
            .find(|theme| theme.key == "custom_theme")
            .expect("custom theme should be discovered");

        assert_eq!(custom.label, "Custom Theme");
        assert!(
            themes
                .iter()
                .any(|theme| theme.key == DEFAULT_DARK_THEME_KEY)
        );
    }
    /// The settings theme list asks for the runtime themes from inside a
    /// `uniform_list` processor, so an unmemoized load re-reads and re-parses
    /// every theme file per frame. The cache has to hold across calls -- and
    /// still notice an edit, because theme authors expect a save to show up
    /// without restarting.
    #[test]
    fn runtime_themes_are_reused_until_the_directory_changes() {
        use serde_json::json;

        let dir = tempdir().expect("temp dir should exist");
        let write_theme = |label: &str| {
            let mut custom = test_theme_entry(DEFAULT_DARK_THEME_KEY);
            custom["key"] = json!("custom_theme");
            custom["name"] = json!(label);
            fs::write(
                dir.path().join("custom_theme.json"),
                test_theme_bundle_json(label, vec![custom]),
            )
            .expect("custom theme file should be written");
        };

        write_theme("First");
        let first = runtime_themes_with_dir(Some(dir.path()));
        let again = runtime_themes_with_dir(Some(dir.path()));
        assert!(
            std::sync::Arc::ptr_eq(&first, &again),
            "an unchanged theme directory must not be re-read and re-parsed"
        );
        assert_eq!(first["custom_theme"].option.label, "First");

        // Rewriting the file changes its size, so the stat signature moves even
        // where the filesystem's mtime resolution is coarse.
        write_theme("Second edit");
        let reloaded = runtime_themes_with_dir(Some(dir.path()));
        assert!(
            !std::sync::Arc::ptr_eq(&first, &reloaded),
            "an edited theme file must invalidate the cache"
        );
        assert_eq!(reloaded["custom_theme"].option.label, "Second edit");
    }

    #[test]
    fn runtime_theme_dir_ignores_reserved_system_theme_filenames() {
        let dir = tempdir().expect("temp dir should exist");
        fs::write(dir.path().join("gitcomet.json"), "not parsed")
            .expect("reserved theme file should be written");

        let themes = merged_theme_options(Some(dir.path()));

        assert_eq!(
            themes,
            available_themes(),
            "custom themes in reserved bundled filenames should be ignored"
        );
    }
    #[test]
    fn runtime_theme_dir_ignores_every_reserved_system_theme_filename() {
        let dir = tempdir().expect("temp dir should exist");

        for file in EMBEDDED_THEME_FILES {
            fs::write(dir.path().join(format!("{}.json", file.stem)), "not parsed")
                .expect("reserved theme file should be written");
        }

        assert!(
            runtime_themes_with_dir(Some(dir.path())).is_empty(),
            "runtime themes should ignore every reserved bundled filename"
        );
        assert_eq!(
            merged_theme_options(Some(dir.path())),
            available_themes(),
            "reserved files should not change the available theme list"
        );
    }
    #[test]
    fn runtime_theme_dir_ignores_embedded_theme_key_collisions_but_keeps_custom_entries() {
        use serde_json::json;

        let dir = tempdir().expect("temp dir should exist");
        let mut collision = test_theme_entry(DEFAULT_DARK_THEME_KEY);
        collision["name"] = json!("Fake GitComet Dark");

        let mut custom = test_theme_entry(DEFAULT_LIGHT_THEME_KEY);
        custom["key"] = json!("custom_keep");
        custom["name"] = json!("Custom Keep");

        fs::write(
            dir.path().join("mixed_theme.json"),
            test_theme_bundle_json("Mixed Theme", vec![collision, custom]),
        )
        .expect("mixed theme file should be written");

        let runtime_themes = runtime_themes_with_dir(Some(dir.path()));
        assert!(
            !runtime_themes.contains_key(DEFAULT_DARK_THEME_KEY),
            "runtime themes should ignore entries that reuse embedded system keys"
        );
        assert!(
            runtime_themes.contains_key("custom_keep"),
            "runtime themes should keep valid custom entries from mixed bundles"
        );

        let themes = merged_theme_options(Some(dir.path()));
        assert_eq!(
            themes
                .iter()
                .find(|theme| theme.key == DEFAULT_DARK_THEME_KEY)
                .map(|theme| theme.label.as_str()),
            Some("GitComet Dark"),
            "embedded theme labels should remain authoritative"
        );
        assert_eq!(
            themes
                .iter()
                .filter(|theme| theme.key == DEFAULT_DARK_THEME_KEY)
                .count(),
            1,
            "embedded system keys should appear only once in the merged theme list"
        );
        assert!(
            themes.iter().any(|theme| theme.key == "custom_keep"),
            "valid custom themes should still appear in available theme options"
        );
    }
    #[test]
    fn themes_markdown_example_matches_current_theme_parser() {
        let example = themes_markdown_example();
        let json = strip_json_line_comments(&example);
        let themes = load_theme_specs_from_json(&json)
            .expect("THEMES.md example should stay in sync with the runtime parser");

        assert_eq!(themes.len(), 1, "docs example should define a single theme");
        assert_eq!(themes[0].option.key, "my_theme_dark");
    }

    #[test]
    fn themes_markdown_lists_current_supported_syntax_keys() {
        const REQUIRED_DOC_KEYS: &[&str] = &[
            "comment",
            "comment_doc",
            "string",
            "string_escape",
            "string_regex",
            "string_special",
            "keyword",
            "keyword_control",
            "preproc",
            "number",
            "boolean",
            "function",
            "function_method",
            "function_special",
            "constructor",
            "type",
            "type_builtin",
            "type_interface",
            "namespace",
            "variable",
            "variable_parameter",
            "variable_special",
            "variable_builtin",
            "property",
            "label",
            "constant",
            "constant_builtin",
            "operator",
            "punctuation",
            "punctuation_bracket",
            "punctuation_delimiter",
            "punctuation_special",
            "punctuation_list_marker",
            "tag",
            "attribute",
            "markup_heading",
            "markup_link",
            "text_literal",
            "diff_plus",
            "diff_minus",
            "diff_delta",
            "lifetime",
        ];

        let markdown = fs::read_to_string(themes_markdown_path())
            .expect("THEMES.md should be readable for supported-key checks");

        for key in REQUIRED_DOC_KEYS {
            assert!(
                markdown.contains(&format!("`{key}`")),
                "THEMES.md should mention the supported syntax key `{key}`"
            );
        }
    }

    #[test]
    fn themes_markdown_documents_custom_theme_override_rules() {
        let markdown = fs::read_to_string(themes_markdown_path())
            .expect("THEMES.md should be readable for override behavior checks");

        for snippet in [
            "GitComet creates the user themes directory on startup",
            "ignores files whose basename matches a bundled system theme file",
            "cannot override built-in system theme keys",
        ] {
            assert!(
                markdown.contains(snippet),
                "THEMES.md should document `{snippet}`"
            );
        }
    }

    #[test]
    fn readme_themes_section_points_to_theme_guide() {
        let readme =
            fs::read_to_string(readme_path()).expect("README.md should be readable for docs tests");

        for snippet in [
            "Custom themes are loaded from JSON bundle files in your per-user themes directory",
            "creates on startup",
            "[THEMES.md](docs/themes.md)",
        ] {
            assert!(
                readme.contains(snippet),
                "README.md theme section should mention `{snippet}`"
            );
        }
    }
}
