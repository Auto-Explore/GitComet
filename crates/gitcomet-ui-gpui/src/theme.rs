use gpui::Rgba;
use gpui::WindowAppearance;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

static EMBEDDED_THEME_CACHE: OnceLock<HashMap<String, RuntimeThemeSpec>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppTheme {
    pub is_dark: bool,
    pub colors: Colors,
    pub syntax: SyntaxColors,
    pub graph_lane_palette: GraphLanePalette,
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
    pub line_number_active: Rgba,
    pub cursor: Rgba,
    pub selection_background: Rgba,
    pub selection_foreground: Rgba,
    pub inactive_selection_background: Rgba,
    pub current_line_background: Rgba,
    pub search_match_background: Rgba,
    pub search_match_foreground: Rgba,
    pub search_match_border: Rgba,
    pub bracket_match_background: Rgba,
    pub whitespace: Rgba,
    pub indent_guide: Rgba,
    pub indent_guide_active: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColorSet {
    pub foreground: Rgba,
    pub background: Rgba,
    pub word_background: Rgba,
    pub focused_background: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColors {
    pub added: DiffColorSet,
    pub removed: DiffColorSet,
    pub modified: DiffColorSet,
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
        let mut colors = [Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }; GRAPH_LANE_PALETTE_SIZE];
        for (i, color) in colors.iter_mut().enumerate() {
            let hue = (i as f32 * 0.13) % 1.0;
            let sat = 0.75;
            let light = if is_dark { 0.62 } else { 0.33 };
            *color = gpui::hsla(hue, sat, light, 1.0).into();
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
                .map(|hue| gpui::hsla(hue.rem_euclid(1.0), sat, light, 1.0).into())
                .collect::<Vec<_>>();
            return Self::from_rgba_slice(&colors);
        }

        Self::generated(is_dark)
    }

    fn from_rgba_slice(colors: &[Rgba]) -> Self {
        let mut out = [Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }; GRAPH_LANE_PALETTE_SIZE];
        let len = colors.len().min(GRAPH_LANE_PALETTE_SIZE);
        for (slot, color) in out.iter_mut().zip(colors.iter().take(len)) {
            *slot = *color;
        }
        Self {
            colors: out,
            len: len as u8,
        }
    }

    #[cfg(test)]
    pub fn as_slice(&self) -> &[Rgba] {
        let len = usize::from(self.len).max(1);
        &self.colors[..len]
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
    schema_version: u32,
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
    line_number_active: ThemeColor,
    cursor: ThemeColor,
    selection_background: ThemeColor,
    selection_foreground: ThemeColor,
    inactive_selection_background: ThemeColor,
    current_line_background: ThemeColor,
    search_match_background: ThemeColor,
    search_match_foreground: ThemeColor,
    search_match_border: ThemeColor,
    bracket_match_background: ThemeColor,
    whitespace: ThemeColor,
    indent_guide: ThemeColor,
    indent_guide_active: ThemeColor,
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

#[derive(Clone, Copy, Deserialize)]
#[serde(untagged)]
enum ThemeColor {
    Hex(Rgba),
    HexWithAlpha { hex: Rgba, alpha: f32 },
}

impl ThemeColor {
    fn into_rgba(self) -> Rgba {
        match self {
            Self::Hex(color) => color,
            Self::HexWithAlpha { hex, alpha } => with_alpha(hex, alpha),
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
                line_number_active: editor.line_number_active.into_rgba(),
                cursor: editor.cursor.into_rgba(),
                selection_background: editor.selection_background.into_rgba(),
                selection_foreground: editor.selection_foreground.into_rgba(),
                inactive_selection_background: editor.inactive_selection_background.into_rgba(),
                current_line_background: editor.current_line_background.into_rgba(),
                search_match_background: editor.search_match_background.into_rgba(),
                search_match_foreground: editor.search_match_foreground.into_rgba(),
                search_match_border: editor.search_match_border.into_rgba(),
                bracket_match_background: editor.bracket_match_background.into_rgba(),
                whitespace: editor.whitespace.into_rgba(),
                indent_guide: editor.indent_guide.into_rgba(),
                indent_guide_active: editor.indent_guide_active.into_rgba(),
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
            graph_lane_palette,
            radii,
        }
    }
}

pub(crate) fn mix_colors(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
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

fn embedded_theme_cache() -> &'static HashMap<String, RuntimeThemeSpec> {
    EMBEDDED_THEME_CACHE.get_or_init(|| {
        let mut themes = HashMap::default();
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
    for spec in runtime_themes_with_dir(runtime_dir).into_values() {
        options.insert(spec.option.key.clone(), spec.option);
    }
    for spec in embedded_theme_cache().values() {
        options.insert(spec.option.key.clone(), spec.option.clone());
    }

    options.into_values().collect()
}

fn runtime_themes() -> HashMap<String, RuntimeThemeSpec> {
    runtime_themes_with_dir(None)
}

fn runtime_themes_with_dir(runtime_dir: Option<&Path>) -> HashMap<String, RuntimeThemeSpec> {
    let Some(dir) = resolved_runtime_themes_dir(runtime_dir) else {
        return HashMap::default();
    };

    load_runtime_themes_from_dir(&dir)
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

fn load_runtime_themes_from_dir(dir: &Path) -> HashMap<String, RuntimeThemeSpec> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashMap::default();
    };

    let mut files = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter(|path| !is_reserved_runtime_theme_path(path))
        .collect::<Vec<_>>();
    files.sort();

    let mut themes = HashMap::default();
    for path in files {
        let json = match fs::read_to_string(&path) {
            Ok(json) => json,
            Err(error) => {
                eprintln!(
                    "Ignoring custom theme {}: failed to read file: {error}",
                    path.display()
                );
                continue;
            }
        };
        let specs = match load_runtime_theme_specs_from_json(&json) {
            Ok(specs) => specs,
            Err(error) => {
                eprintln!("Ignoring custom theme {}: {error}", path.display());
                continue;
            }
        };

        for spec in specs {
            themes.insert(spec.option.key.clone(), spec);
        }
    }

    themes
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

    let mut seen_keys = HashSet::<String>::default();
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

fn parse_theme_bundle(json: &str) -> Result<ThemeBundleFile, ThemeParseError> {
    let bundle: ThemeBundleFile = serde_json::from_str(json).map_err(ThemeParseError::Parse)?;
    if bundle.schema_version != THEME_SCHEMA_VERSION {
        return Err(ThemeParseError::Invalid(format!(
            "unsupported theme schema version {}; expected {}",
            bundle.schema_version, THEME_SCHEMA_VERSION
        )));
    }
    Ok(bundle)
}

pub(crate) fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

/// Flattens a translucent overlay onto an opaque base, giving the single color
/// the eye sees where the two are stacked. Anything that has to blend into a
/// surface it doesn't paint itself — a label fade over a hovered row, say —
/// needs this rather than the overlay color alone.
pub(crate) fn composite_over(base: Rgba, overlay: Rgba) -> Rgba {
    let t = overlay.a.clamp(0.0, 1.0);
    Rgba {
        r: base.r + (overlay.r - base.r) * t,
        g: base.g + (overlay.g - base.g) * t,
        b: base.b + (overlay.b - base.b) * t,
        a: base.a,
    }
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
        GRAPH_LANE_PALETTE_SIZE, Rgba, THEME_SCHEMA_VERSION, available_themes, content_header_bg,
        derived_syntax_color, has_theme_key, load_theme_specs_from_json, merged_theme_options,
        resolved_runtime_themes_dir, runtime_themes_with_dir, test_theme_bundle_value,
        test_theme_json_with_syntax, theme_label, with_alpha,
    };
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

        0.2126 * linear_channel(color.r)
            + 0.7152 * linear_channel(color.g)
            + 0.0722 * linear_channel(color.b)
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
        let color = Rgba {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.4,
        };

        let adjusted = with_alpha(color, 0.75);

        assert_eq!(adjusted.r, color.r);
        assert_eq!(adjusted.g, color.g);
        assert_eq!(adjusted.b, color.b);
        assert_eq!(adjusted.a, 0.75);
    }

    #[test]
    fn rejects_theme_bundle_without_schema_version() {
        let error = load_theme_specs_from_json(r#"{"name":"Missing version","themes":[]}"#)
            .err()
            .expect("a theme bundle without schema_version must be rejected");

        assert!(
            error.to_string().contains("schema_version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_unsupported_theme_schema_versions() {
        let error =
            load_theme_specs_from_json(r#"{"schema_version":999,"name":"Future","themes":[]}"#)
                .err()
                .expect("unsupported schema version must be rejected");

        assert_eq!(
            error.to_string(),
            format!("unsupported theme schema version 999; expected {THEME_SCHEMA_VERSION}")
        );
    }

    #[test]
    fn semantic_groups_are_strictly_validated() {
        use serde_json::json;

        let mut missing = test_theme_bundle_value(DEFAULT_DARK_THEME_KEY);
        missing["themes"][0]["colors"]["surface"]
            .as_object_mut()
            .expect("surface should be an object")
            .remove("input");
        let missing_error = AppTheme::from_json_str(
            &serde_json::to_string(&missing).expect("fixture should serialize"),
        )
        .expect_err("missing required semantic token must fail");
        assert!(
            missing_error.to_string().contains("missing field `input`"),
            "unexpected error: {missing_error}"
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
            gpui::hsla(0.25, 0.75, 0.62, 1.0).into()
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
                for (surface_name, surface) in [
                    ("editor", colors.editor.background),
                    ("editor.current_line", colors.editor.current_line_background),
                ] {
                    assert_min_contrast(
                        key,
                        &format!("syntax.{token}/{surface_name}"),
                        color,
                        surface,
                        7.0,
                    );
                }

                for (surface_name, surface) in [
                    ("editor.selection", colors.editor.selection_background),
                    (
                        "editor.inactive_selection",
                        colors.editor.inactive_selection_background,
                    ),
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
