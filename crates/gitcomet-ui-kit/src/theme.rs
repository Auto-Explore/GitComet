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

pub const DEFAULT_DARK_THEME_KEY: &str = "gitcomet_dark";
pub const DEFAULT_LIGHT_THEME_KEY: &str = "gitcomet_light";
pub const AMBER_DARK_THEME_KEY: &str = "amber_dark";
pub const GRAPH_LANE_PALETTE_SIZE: usize = 64;
pub const THEME_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeOption {
    pub key: String,
    pub label: String,
    pub is_dark: bool,
    /// Loaded from the user themes folder rather than bundled.
    pub custom: bool,
    pub preview: ThemePreviewColors,
}

/// The colours a theme's picker orb is painted from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemePreviewColors {
    pub is_dark: bool,
    /// The chrome band: what tells one window's theme from another's.
    pub base: Rgba,
    pub glow: Rgba,
    pub secondary: Rgba,
}

struct EmbeddedThemeFile {
    stem: &'static str,
    json: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/embedded_themes.rs"));

static EMBEDDED_THEME_CACHE: OnceLock<FxHashMap<String, RuntimeThemeSpec>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppTheme {
    /// Resolved application typography and density; not part of a theme file.
    pub metrics: crate::appearance::Appearance,
    pub is_dark: bool,
    pub colors: Colors,
    pub syntax: SyntaxColors,
    /// Interned rather than inlined: the palette is 64 colours, and `AppTheme` is
    /// `Copy` and captured by value into every per-row paint closure. Carrying the
    /// array here made each of those closures a kilobyte heavier for data every
    /// theme shares. See `intern_lane_palette`.
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
    pub notice: NoticeColors,
    /// Interned like the lane palette: `AppTheme` is copied per painted row.
    pub interstitial: &'static InterstitialColors,
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
    /// Wash for every other place the clicked name appears.
    ///
    /// Deliberately distinct from `bracket_match_background`: a click can light
    /// both at once, and from `search_match_background`, which already means
    /// "this matched your query".
    pub occurrence_highlight_background: Rgba,
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

/// Inline notices that ask for a decision, such as "File changed on disk".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoticeColors {
    pub background: Rgba,
    pub border: Rgba,
    /// The bold title.
    pub foreground: Rgba,
    /// The explanation beside the title.
    pub secondary: Rgba,
}

/// Content over an interstitial's backdrop artwork (loading, Home, a gate):
/// its text and call-to-action buttons, tuned to the artwork, not the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterstitialColors {
    pub text: Rgba,
    pub muted: Rgba,
    pub primary: CallToActionColors,
    pub secondary: CallToActionColors,
}

/// One call-to-action button; `text` also tints its icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CallToActionColors {
    pub text: Rgba,
    pub background: Rgba,
    pub background_hover: Rgba,
    pub background_active: Rgba,
    pub border: Rgba,
    pub border_hover: Rgba,
    pub border_active: Rgba,
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn leaked_for_test(colors: &[Rgba]) -> &'static Self {
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
    pub fn with_appearance(mut self, metrics: crate::appearance::Appearance) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn editor_row_height(self, scale_percent: u32) -> gpui::Pixels {
        crate::ui_scale::design_px_from_percent(self.metrics.editor_line_height(), scale_percent)
    }

    pub fn editor_font_size(self, scale_percent: u32) -> gpui::Pixels {
        crate::ui_scale::design_px_from_percent(
            self.metrics.editor_font_size_px as f32,
            scale_percent,
        )
    }

    pub fn markdown_px(self, value: f32, scale_percent: u32) -> gpui::Pixels {
        crate::ui_scale::design_px_from_percent(
            value * self.metrics.markdown_preview_font_size_px as f32 / 13.0,
            scale_percent,
        )
    }

    pub fn ui_text(self, pixels: f32) -> gpui::Rems {
        gpui::rems(self.metrics.ui_text(pixels) / 16.0)
    }

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

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_json_str(json: &str) -> Result<Self, ThemeParseError> {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_json_path(path: impl AsRef<Path>) -> Result<Self, ThemeLoadError> {
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

    pub fn from_key(key: &str) -> Option<Self> {
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn gitcomet_light() -> Self {
        Self::from_key(DEFAULT_LIGHT_THEME_KEY)
            .unwrap_or_else(|| panic!("missing default light theme `{DEFAULT_LIGHT_THEME_KEY}`"))
    }
}

pub fn available_themes() -> Vec<ThemeOption> {
    merged_theme_options(None)
}

/// Every theme and every refused theme file from one read of the themes
/// folder, for a render that looks up more than one key.
pub struct ThemeCatalog {
    pub themes: Vec<ThemeOption>,
    /// Why the themes that are *not* in the picker were left out.
    ///
    /// A rejected file is otherwise invisible: it disappears from the list, the
    /// app falls back to a bundled theme, and the only account of it goes to
    /// stderr, which nobody running a windowed build ever sees. That matters
    /// most right after a schema break -- every v1 custom theme in the folder is
    /// rejected at once, and "my theme is gone" needs to be answerable without a
    /// terminal.
    pub issues: Arc<[RuntimeThemeIssue]>,
}

impl ThemeCatalog {
    pub fn load() -> Self {
        let entry = runtime_theme_cache_entry(None);
        let (runtime, issues) = entry.map_or_else(
            || (Arc::default(), Arc::from(Vec::new())),
            |entry| (entry.themes, entry.issues),
        );
        Self {
            themes: merge_theme_options(&runtime),
            issues,
        }
    }

    pub fn get(&self, key: &str) -> Option<&ThemeOption> {
        self.themes.iter().find(|option| option.key == key)
    }
}

pub fn has_theme_key(key: &str) -> bool {
    merged_theme_options(None)
        .iter()
        .any(|option| option.key == key)
}

pub fn theme_label(key: &str) -> Option<String> {
    merged_theme_options(None)
        .into_iter()
        .find(|option| option.key == key)
        .map(|option| option.label)
}

#[cfg(any(test, feature = "test-support"))]
pub fn theme_preview_colors(key: &str) -> Option<ThemePreviewColors> {
    AppTheme::from_key(key).map(|theme| theme.preview_colors())
}

impl AppTheme {
    pub fn preview_colors(self) -> ThemePreviewColors {
        ThemePreviewColors {
            is_dark: self.is_dark,
            base: self.colors.surface.chrome,
            glow: self.colors.accent.solid,
            secondary: self.syntax.keyword,
        }
    }
}

/// Every bundled theme key with its appearance, in picker order.
#[cfg(any(test, feature = "test-support"))]
pub fn bundled_theme_keys() -> Vec<(String, bool)> {
    let mut options = embedded_theme_cache()
        .values()
        .map(|spec| &spec.option)
        .collect::<Vec<_>>();
    options.sort_by(|left, right| theme_option_order(left, right));
    options
        .into_iter()
        .map(|option| (option.key.clone(), option.is_dark))
        .collect()
}

pub fn ensure_user_themes_dir_exists() -> Option<PathBuf> {
    resolved_runtime_themes_dir(None)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub enum ThemeLoadError {
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: std::path::PathBuf,
        source: ThemeParseError,
    },
}

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
impl Error for ThemeLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

#[derive(Debug)]
pub enum ThemeParseError {
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
    notice: ThemeFileNoticeColors,
    interstitial: ThemeFileInterstitialColors,
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
    /// Optional: a theme written before this existed still parses, and falls
    /// back to the bracket wash rather than to nothing.
    #[serde(default)]
    occurrence_highlight_background: Option<ThemeColor>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileNoticeColors {
    background: ThemeColor,
    border: ThemeColor,
    foreground: ThemeColor,
    secondary: ThemeColor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileInterstitialColors {
    text: ThemeColor,
    muted: ThemeColor,
    primary: ThemeFileCallToActionColors,
    secondary: ThemeFileCallToActionColors,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFileCallToActionColors {
    text: ThemeColor,
    background: ThemeColor,
    background_hover: ThemeColor,
    background_active: ThemeColor,
    border: ThemeColor,
    border_hover: ThemeColor,
    border_active: ThemeColor,
}

impl ThemeFileCallToActionColors {
    fn into_colors(self) -> CallToActionColors {
        CallToActionColors {
            text: self.text.into_rgba(),
            background: self.background.into_rgba(),
            background_hover: self.background_hover.into_rgba(),
            background_active: self.background_active.into_rgba(),
            border: self.border.into_rgba(),
            border_hover: self.border_hover.into_rgba(),
            border_active: self.border_active.into_rgba(),
        }
    }
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
/// Dispatch to palette's RGB or RGBA parser while requiring a leading `#`
/// and preserving the theme format's four supported spellings.
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

        // Palette slices at byte offsets and its integer parser accepts `+`;
        // require ASCII hex digits before handing it user-authored colors.
        if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit)
        {
            return Err(format!(
                "invalid hex color {value:?}: expected #rgb, #rgba, #rrggbb, or #rrggbbaa"
            ));
        }
        let components = match hex.len() {
            3 | 6 => hex.parse::<palette::Srgb<u8>>().map(|rgb| {
                let (r, g, b) = rgb.into_components();
                [r, g, b, 255]
            }),
            _ => hex.parse::<palette::Srgba<u8>>().map(|rgba| {
                let (r, g, b, a) = rgba.into_components();
                [r, g, b, a]
            }),
        }
        .map_err(|err| format!("invalid hex color {value:?}: {err}"))?;

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
            notice,
            interstitial,
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
                occurrence_highlight_background: editor
                    .occurrence_highlight_background
                    .unwrap_or(editor.bracket_match_background)
                    .into_rgba(),
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
            notice: NoticeColors {
                background: notice.background.into_rgba(),
                border: notice.border.into_rgba(),
                foreground: notice.foreground.into_rgba(),
                secondary: notice.secondary.into_rgba(),
            },
            interstitial: intern_interstitial(InterstitialColors {
                text: interstitial.text.into_rgba(),
                muted: interstitial.muted.into_rgba(),
                primary: interstitial.primary.into_colors(),
                secondary: interstitial.secondary.into_colors(),
            }),
            shadow: shadow.into_rgba(),
        };
        let syntax = resolve_syntax_colors(is_dark, &colors, syntax.as_ref());

        Self {
            metrics: crate::appearance::Appearance::default(),
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
/// One allocation per distinct interstitial palette, compared bit for bit
/// like [`lane_palettes_are_identical`].
fn intern_interstitial(colors: InterstitialColors) -> &'static InterstitialColors {
    static INTERNED: OnceLock<Mutex<Vec<&'static InterstitialColors>>> = OnceLock::new();
    fn bits(colors: &InterstitialColors) -> Vec<u32> {
        let cta = |c: &CallToActionColors| {
            [
                c.text,
                c.background,
                c.background_hover,
                c.background_active,
                c.border,
                c.border_hover,
                c.border_active,
            ]
        };
        [colors.text, colors.muted]
            .into_iter()
            .chain(cta(&colors.primary))
            .chain(cta(&colors.secondary))
            .flat_map(|c| [c.red, c.green, c.blue, c.alpha].map(f32::to_bits))
            .collect()
    }
    let mut interned = INTERNED
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = bits(&colors);
    if let Some(existing) = interned.iter().find(|existing| bits(existing) == key) {
        return existing;
    }
    let leaked: &'static InterstitialColors = Box::leak(Box::new(colors));
    interned.push(leaked);
    leaked
}

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

pub fn mix_colors(a: Rgba, b: Rgba, t: f32) -> Rgba {
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
        color: with_alpha(base, alpha).into(),
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
pub fn shadow_surface(_theme: AppTheme) -> Vec<gpui::BoxShadow> {
    Vec::new()
}

/// A single, restrained lift for dropdowns, context menus and hover panels.
pub fn shadow_popover(theme: AppTheme) -> Vec<gpui::BoxShadow> {
    let base = theme.colors.shadow;
    let m = if theme.is_dark { 1.0 } else { 0.5 };
    vec![shadow_layer(base, 0.22 * m, 4.0, 12.0)]
}

/// Slightly stronger (still understated) lift for modal dialogs.
pub fn shadow_modal(theme: AppTheme) -> Vec<gpui::BoxShadow> {
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
    merge_theme_options(&runtime_themes_with_dir(runtime_dir))
}

fn merge_theme_options(runtime: &FxHashMap<String, RuntimeThemeSpec>) -> Vec<ThemeOption> {
    let mut options = BTreeMap::<String, ThemeOption>::new();
    for spec in runtime.values() {
        options.insert(spec.option.key.clone(), spec.option.clone());
    }
    for spec in embedded_theme_cache().values() {
        let mut option = spec.option.clone();
        option.label = house_theme_label(&option.key, option.label);
        options.insert(option.key.clone(), option);
    }

    let mut options = options.into_values().collect::<Vec<_>>();
    options.sort_by(theme_option_order);
    options
}

/// Picker order, which the tile grid keeps within each group: GitComet's own
/// themes first, then by name, ignoring case.
fn theme_option_order(left: &ThemeOption, right: &ThemeOption) -> std::cmp::Ordering {
    // Compared lazily: a sort must not allocate per comparison.
    fn name(option: &ThemeOption) -> impl Iterator<Item = char> + '_ {
        option.label.chars().flat_map(char::to_lowercase)
    }
    theme_option_rank(&left.key)
        .cmp(&theme_option_rank(&right.key))
        .then_with(|| name(left).cmp(name(right)))
        .then_with(|| left.key.cmp(&right.key))
}

/// The house themes carry the product's name ("<Product> Dark").
fn house_theme_label(key: &str, label: String) -> String {
    let appearance = match key {
        DEFAULT_DARK_THEME_KEY => "Dark",
        DEFAULT_LIGHT_THEME_KEY => "Light",
        _ => return label,
    };
    format!(
        "{} {appearance}",
        gitcomet_core::identity::current().display_name()
    )
}

/// GitComet's two defaults, then its alternate house palette.
fn theme_option_rank(key: &str) -> u8 {
    match key {
        DEFAULT_DARK_THEME_KEY => 0,
        DEFAULT_LIGHT_THEME_KEY => 1,
        AMBER_DARK_THEME_KEY => 2,
        _ => 3,
    }
}

fn runtime_themes() -> Arc<FxHashMap<String, RuntimeThemeSpec>> {
    runtime_themes_with_dir(None)
}

/// Custom themes from disk, re-parsed only when the directory has actually
/// changed.
///
/// Reading and parsing every theme file is far too expensive to do per call:
/// the settings pages look themes up on every render, so an unmemoized load is a
/// directory read plus a full parse per file *per frame*. Theme authors still
/// expect an edit to show up without a restart, so the cache is validated
/// against a cheap stat of the directory rather than held forever.
fn runtime_themes_with_dir(runtime_dir: Option<&Path>) -> Arc<FxHashMap<String, RuntimeThemeSpec>> {
    runtime_theme_cache_entry(runtime_dir)
        .map(|entry| entry.themes)
        .unwrap_or_default()
}

#[cfg(test)]
fn runtime_theme_issues_with_dir(runtime_dir: Option<&Path>) -> Arc<[RuntimeThemeIssue]> {
    runtime_theme_cache_entry(runtime_dir)
        .map(|entry| entry.issues)
        .unwrap_or_else(|| Arc::from(Vec::new()))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static RUNTIME_THEME_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Theme-folder lookups on this thread. Each walks the folder in the app;
/// tests have no folder, so this is how they see the cost.
#[cfg(any(test, feature = "test-support"))]
pub fn runtime_theme_lookups_for_test() -> usize {
    RUNTIME_THEME_LOOKUPS.with(std::cell::Cell::get)
}

fn runtime_theme_cache_entry(runtime_dir: Option<&Path>) -> Option<RuntimeThemeCache> {
    #[cfg(any(test, feature = "test-support"))]
    RUNTIME_THEME_LOOKUPS.with(|lookups| lookups.set(lookups.get() + 1));
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

/// A theme file, or themes in it, the loader refused, named so the picker can
/// say so.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeThemeIssue {
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
    files.sort_unstable();

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

static USER_THEMES_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Where custom themes are read from. The host sets it at launch; unset means
/// only the bundled themes, which keeps tests off the user's theme folder.
pub fn set_user_themes_dir(dir: Option<PathBuf>) {
    *USER_THEMES_DIR
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = dir;
}

fn user_themes_dir() -> Option<PathBuf> {
    USER_THEMES_DIR
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn resolved_runtime_themes_dir(runtime_dir: Option<&Path>) -> Option<PathBuf> {
    let dir = match runtime_dir {
        Some(path) => path.to_path_buf(),
        None => user_themes_dir()?,
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
        .collect::<Vec<_>>();
    files.sort_unstable();

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
        // Reported, not just skipped: a later release can bundle a theme under
        // a name an existing custom theme already uses.
        if is_reserved_runtime_theme_path(&path) {
            reject(
                &mut issues,
                &path,
                "A bundled theme uses this file name, so the file is skipped. Rename it to load it."
                    .to_string(),
            );
            continue;
        }
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

        let (clashing, specs): (Vec<_>, Vec<_>) = specs
            .into_iter()
            .partition(|spec| is_embedded_theme_key(&spec.option.key));
        if !clashing.is_empty() {
            let keys = clashing
                .iter()
                .map(|spec| format!("`{}`", spec.option.key))
                .collect::<Vec<_>>()
                .join(", ");
            let message = if clashing.len() == 1 {
                format!(
                    "A bundled theme already uses the key {keys}, so that theme is skipped. Give it a different `key`."
                )
            } else {
                format!(
                    "Bundled themes already use the keys {keys}, so those themes are skipped. Give them different keys."
                )
            };
            reject(&mut issues, &path, message);
        }
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
    custom: bool,
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
        if !seen_keys.insert(key.clone()) {
            return Err(ThemeParseError::Invalid(format!(
                "theme bundle defines duplicate key `{key}`"
            )));
        }

        let label = entry.name.clone();
        let theme = entry.into_app_theme();
        themes.push(RuntimeThemeSpec {
            option: ThemeOption {
                key,
                label,
                is_dark: theme.is_dark,
                custom,
                preview: theme.preview_colors(),
            },
            theme,
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
pub fn hsla_from_hue_fraction(hue: f32, saturation: f32, lightness: f32, alpha: f32) -> Hsla {
    Hsla::new(
        hue * 360.0,
        saturation.clamp(0.0, 1.0),
        lightness.clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    )
}

pub fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.alpha = alpha;
    color
}

/// Flattens a translucent overlay onto an opaque base, giving the single color
/// the eye sees where the two are stacked. Anything that has to blend into a
/// surface it doesn't paint itself — a label fade over a hovered row, say —
/// needs this rather than the overlay color alone.
pub fn composite_over(base: Rgba, overlay: Rgba) -> Rgba {
    let t = overlay.alpha.clamp(0.0, 1.0);
    // This form, unlike `base + (overlay - base) * t`, is exact at t = 0 and 1.
    let mix = |base: f32, overlay: f32| base * (1.0 - t) + overlay * t;
    Rgba::new(
        mix(base.red, overlay.red),
        mix(base.green, overlay.green),
        mix(base.blue, overlay.blue),
        base.alpha,
    )
}

/// `overlay` stacked on a possibly translucent `base`: painting the result on
/// any surface looks the same as painting `base` and then `overlay` on it.
pub fn layer_over(base: Rgba, overlay: Rgba) -> Rgba {
    let top = overlay.alpha.clamp(0.0, 1.0);
    let bottom = base.alpha.clamp(0.0, 1.0) * (1.0 - top);
    let alpha = top + bottom;
    if alpha <= 0.0 {
        return Rgba::new(0.0, 0.0, 0.0, 0.0);
    }
    let mix = |over: f32, under: f32| (over * top + under * bottom) / alpha;
    Rgba::new(
        mix(overlay.red, base.red),
        mix(overlay.green, base.green),
        mix(overlay.blue, base.blue),
        alpha,
    )
}

/// A fixed, deliberately-distinct purple flagging that the user is browsing a
/// historical commit rather than the live repository state. Intentionally outside
/// the theme palette so it reads as "off-live" in every theme.
pub fn historical_outline(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xa78bfa)
    } else {
        gpui::rgb(0x7c3aed)
    }
}

/// `base` washed with just enough [`historical_outline`] to mark a whole content
/// surface as off-live. Deliberately faint: it sits under body text and syntax
/// colors, so it may tint the surface without competing with what is on it.
pub fn historical_surface_bg(theme: AppTheme, base: Rgba) -> Rgba {
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
pub fn historical_header_bg(theme: AppTheme, base: Rgba) -> Rgba {
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
pub fn content_header_bg(theme: AppTheme) -> Rgba {
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
pub fn blame_heat_color(is_dark: bool, t: f32) -> Rgba {
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
pub fn blame_local_change_color(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xffe000)
    } else {
        gpui::rgb(0xf5c400)
    }
}

/// Border color for *staged* local changes in the blame/annotate column. Reuses
/// the theme's diff "added" accent so staged lines read green, consistent with
/// the rest of the diff UI.
pub fn blame_staged_color(theme: AppTheme) -> Rgba {
    theme.colors.diff.added.foreground
}

/// Border color for *unstaged* local changes in the blame/annotate column.
/// Reuses the theme's diff "removed" accent so unstaged lines read red, standing
/// apart from the green staged bar.
pub fn blame_unstaged_color(theme: AppTheme) -> Rgba {
    theme.colors.diff.removed.foreground
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_theme_bundle_value(base_key: &str) -> serde_json::Value {
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

#[cfg(any(test, feature = "test-support"))]
pub fn test_theme_json_with_syntax(base_key: &str, syntax_json: &str) -> String {
    let mut bundle = test_theme_bundle_value(base_key);
    bundle["themes"][0]["syntax"] =
        serde_json::from_str(syntax_json).expect("syntax fixture JSON should parse");
    serde_json::to_string(&bundle).expect("theme fixture should serialize")
}

#[cfg(test)]
mod tests;
