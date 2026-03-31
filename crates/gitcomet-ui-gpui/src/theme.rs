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
    pub window_bg: Rgba,
    pub surface_bg: Rgba,
    pub surface_bg_elevated: Rgba,
    pub active_section: Rgba,
    pub border: Rgba,
    pub tooltip_bg: Rgba,
    pub tooltip_text: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub hover: Rgba,
    pub active: Rgba,
    pub focus_ring: Rgba,
    pub focus_ring_bg: Rgba,
    pub scrollbar_thumb: Rgba,
    pub scrollbar_thumb_hover: Rgba,
    pub scrollbar_thumb_active: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
    pub diff_add_bg: Rgba,
    pub diff_add_text: Rgba,
    pub diff_remove_bg: Rgba,
    pub diff_remove_text: Rgba,
    pub input_placeholder: Rgba,
    pub accent_text: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxColors {
    pub comment: Rgba,
    pub comment_doc: Rgba,
    pub string: Rgba,
    pub string_escape: Rgba,
    pub keyword: Rgba,
    pub keyword_control: Rgba,
    pub number: Rgba,
    pub boolean: Rgba,
    pub function: Rgba,
    pub function_method: Rgba,
    pub function_special: Rgba,
    pub type_name: Rgba,
    pub type_builtin: Rgba,
    pub type_interface: Rgba,
    pub variable: Option<Rgba>,
    pub variable_parameter: Rgba,
    pub variable_special: Rgba,
    pub property: Rgba,
    pub constant: Rgba,
    pub operator: Rgba,
    pub punctuation: Rgba,
    pub punctuation_bracket: Rgba,
    pub punctuation_delimiter: Rgba,
    pub tag: Rgba,
    pub attribute: Rgba,
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
            let light = if is_dark { 0.62 } else { 0.45 };
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
            let light = if is_dark { 0.62 } else { 0.45 };
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
}

impl AppTheme {
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

    #[allow(dead_code)]
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
        runtime_themes()
            .get(key)
            .map(|spec| spec.theme)
            .or_else(|| embedded_theme_cache().get(key).map(|spec| spec.theme))
    }

    /// GitComet's default dark theme loaded from an embedded JSON definition.
    #[allow(dead_code)]
    pub fn gitcomet_dark() -> Self {
        Self::from_key(DEFAULT_DARK_THEME_KEY)
            .unwrap_or_else(|| panic!("missing default dark theme `{DEFAULT_DARK_THEME_KEY}`"))
    }

    /// GitComet's default light theme loaded from an embedded JSON definition.
    #[allow(dead_code)]
    pub fn gitcomet_light() -> Self {
        Self::from_key(DEFAULT_LIGHT_THEME_KEY)
            .unwrap_or_else(|| panic!("missing default light theme `{DEFAULT_LIGHT_THEME_KEY}`"))
    }
}

pub(crate) fn available_themes() -> Vec<ThemeOption> {
    merged_theme_options(None, true)
}

pub(crate) fn has_theme_key(key: &str) -> bool {
    merged_theme_options(None, true)
        .iter()
        .any(|option| option.key == key)
}

pub(crate) fn theme_label(key: &str) -> Option<String> {
    merged_theme_options(None, true)
        .into_iter()
        .find(|option| option.key == key)
        .map(|option| option.label)
}

#[allow(dead_code)]
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
    window_bg: ThemeColor,
    surface_bg: ThemeColor,
    surface_bg_elevated: ThemeColor,
    active_section: ThemeColor,
    border: ThemeColor,
    #[serde(default = "default_tooltip_bg_theme_color")]
    tooltip_bg: ThemeColor,
    #[serde(default = "default_tooltip_text_theme_color")]
    tooltip_text: ThemeColor,
    text: ThemeColor,
    text_muted: ThemeColor,
    accent: ThemeColor,
    hover: ThemeColor,
    active: ThemeColor,
    focus_ring: ThemeColor,
    focus_ring_bg: ThemeColor,
    scrollbar_thumb: ThemeColor,
    scrollbar_thumb_hover: ThemeColor,
    scrollbar_thumb_active: ThemeColor,
    danger: ThemeColor,
    warning: ThemeColor,
    success: ThemeColor,
    #[serde(default)]
    diff_add_bg: Option<ThemeColor>,
    #[serde(default)]
    diff_add_text: Option<ThemeColor>,
    #[serde(default)]
    diff_remove_bg: Option<ThemeColor>,
    #[serde(default)]
    diff_remove_text: Option<ThemeColor>,
    #[serde(default)]
    input_placeholder: Option<ThemeColor>,
    #[serde(default)]
    accent_text: Option<ThemeColor>,
    #[serde(default)]
    graph_lane_palette: Option<Vec<ThemeColor>>,
    #[serde(default)]
    graph_lane_hues: Option<Vec<f32>>,
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
    keyword: Option<ThemeColor>,
    #[serde(default)]
    keyword_control: Option<ThemeColor>,
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
    #[serde(rename = "type", default)]
    type_name: Option<ThemeColor>,
    #[serde(default)]
    type_builtin: Option<ThemeColor>,
    #[serde(default)]
    type_interface: Option<ThemeColor>,
    #[serde(default)]
    variable: Option<ThemeColor>,
    #[serde(default)]
    variable_parameter: Option<ThemeColor>,
    #[serde(default)]
    variable_special: Option<ThemeColor>,
    #[serde(default)]
    property: Option<ThemeColor>,
    #[serde(default)]
    constant: Option<ThemeColor>,
    #[serde(default)]
    operator: Option<ThemeColor>,
    #[serde(default)]
    punctuation: Option<ThemeColor>,
    #[serde(default)]
    punctuation_bracket: Option<ThemeColor>,
    #[serde(default)]
    punctuation_delimiter: Option<ThemeColor>,
    #[serde(default)]
    tag: Option<ThemeColor>,
    #[serde(default)]
    attribute: Option<ThemeColor>,
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
            window_bg,
            surface_bg,
            surface_bg_elevated,
            active_section,
            border,
            tooltip_bg,
            tooltip_text,
            text,
            text_muted,
            accent,
            hover,
            active,
            focus_ring,
            focus_ring_bg,
            scrollbar_thumb,
            scrollbar_thumb_hover,
            scrollbar_thumb_active,
            danger,
            warning,
            success,
            diff_add_bg,
            diff_add_text,
            diff_remove_bg,
            diff_remove_text,
            input_placeholder,
            accent_text,
            graph_lane_palette,
            graph_lane_hues,
        } = colors;
        let graph_lane_palette =
            GraphLanePalette::from_theme_colors(is_dark, graph_lane_palette, graph_lane_hues);

        let colors = Colors {
            window_bg: window_bg.into_rgba(),
            surface_bg: surface_bg.into_rgba(),
            surface_bg_elevated: surface_bg_elevated.into_rgba(),
            active_section: active_section.into_rgba(),
            border: border.into_rgba(),
            tooltip_bg: tooltip_bg.into_rgba(),
            tooltip_text: tooltip_text.into_rgba(),
            text: text.into_rgba(),
            text_muted: text_muted.into_rgba(),
            accent: accent.into_rgba(),
            hover: hover.into_rgba(),
            active: active.into_rgba(),
            focus_ring: focus_ring.into_rgba(),
            focus_ring_bg: focus_ring_bg.into_rgba(),
            scrollbar_thumb: scrollbar_thumb.into_rgba(),
            scrollbar_thumb_hover: scrollbar_thumb_hover.into_rgba(),
            scrollbar_thumb_active: scrollbar_thumb_active.into_rgba(),
            danger: danger.into_rgba(),
            warning: warning.into_rgba(),
            success: success.into_rgba(),
            diff_add_bg: diff_add_bg
                .map(ThemeColor::into_rgba)
                .unwrap_or_else(|| default_diff_add_bg(is_dark)),
            diff_add_text: diff_add_text
                .map(ThemeColor::into_rgba)
                .unwrap_or_else(|| default_diff_add_text(is_dark)),
            diff_remove_bg: diff_remove_bg
                .map(ThemeColor::into_rgba)
                .unwrap_or_else(|| default_diff_remove_bg(is_dark)),
            diff_remove_text: diff_remove_text
                .map(ThemeColor::into_rgba)
                .unwrap_or_else(|| default_diff_remove_text(is_dark)),
            input_placeholder: input_placeholder
                .map(ThemeColor::into_rgba)
                .unwrap_or_else(|| default_input_placeholder(is_dark)),
            accent_text: accent_text
                .map(ThemeColor::into_rgba)
                .unwrap_or_else(default_accent_text),
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

fn mix_colors(a: Rgba, b: Rgba, t: f32) -> Rgba {
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
    mix_colors(token, colors.text, blend_to_text)
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
    let accent = derived_syntax_color(is_dark, colors, colors.accent);
    let warning = derived_syntax_color(is_dark, colors, colors.warning);
    let success = derived_syntax_color(is_dark, colors, colors.success);

    SyntaxColors {
        comment: resolve_syntax_color(overrides.comment, colors.text_muted),
        comment_doc: resolve_syntax_color(overrides.comment_doc, colors.text_muted),
        string: resolve_syntax_color(overrides.string, warning),
        string_escape: resolve_syntax_color(overrides.string_escape, success),
        keyword: resolve_syntax_color(overrides.keyword, accent),
        keyword_control: resolve_syntax_color(overrides.keyword_control, accent),
        number: resolve_syntax_color(overrides.number, success),
        boolean: resolve_syntax_color(overrides.boolean, success),
        function: resolve_syntax_color(overrides.function, accent),
        function_method: resolve_syntax_color(overrides.function_method, accent),
        function_special: resolve_syntax_color(overrides.function_special, accent),
        type_name: resolve_syntax_color(overrides.type_name, warning),
        type_builtin: resolve_syntax_color(overrides.type_builtin, warning),
        type_interface: resolve_syntax_color(overrides.type_interface, warning),
        variable: resolve_optional_syntax_color(overrides.variable),
        variable_parameter: resolve_syntax_color(overrides.variable_parameter, colors.text_muted),
        variable_special: resolve_syntax_color(overrides.variable_special, accent),
        property: resolve_syntax_color(overrides.property, accent),
        constant: resolve_syntax_color(overrides.constant, success),
        operator: resolve_syntax_color(overrides.operator, colors.text_muted),
        punctuation: resolve_syntax_color(overrides.punctuation, colors.text_muted),
        punctuation_bracket: resolve_syntax_color(overrides.punctuation_bracket, colors.text_muted),
        punctuation_delimiter: resolve_syntax_color(
            overrides.punctuation_delimiter,
            colors.text_muted,
        ),
        tag: resolve_syntax_color(overrides.tag, warning),
        attribute: resolve_syntax_color(overrides.attribute, accent),
        lifetime: resolve_syntax_color(overrides.lifetime, accent),
    }
}

fn default_tooltip_bg_theme_color() -> ThemeColor {
    ThemeColor::Hex(gpui::rgba(0x000000ff))
}

fn default_tooltip_text_theme_color() -> ThemeColor {
    ThemeColor::Hex(gpui::rgba(0xffffffff))
}

fn default_diff_add_bg(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0x0B2E1C)
    } else {
        gpui::rgba(0xe6ffedff)
    }
}

fn default_diff_add_text(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xBBF7D0)
    } else {
        gpui::rgba(0x22863aff)
    }
}

fn default_diff_remove_bg(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0x3A0D13)
    } else {
        gpui::rgba(0xffeef0ff)
    }
}

fn default_diff_remove_text(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::rgb(0xFECACA)
    } else {
        gpui::rgba(0xcb2431ff)
    }
}

fn default_input_placeholder(is_dark: bool) -> Rgba {
    if is_dark {
        gpui::hsla(0.0, 0.0, 1.0, 0.35).into()
    } else {
        gpui::hsla(0.0, 0.0, 0.0, 0.2).into()
    }
}

fn default_accent_text() -> Rgba {
    gpui::rgba(0xffffffff)
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

fn merged_theme_options(runtime_dir: Option<&Path>, seed_embedded: bool) -> Vec<ThemeOption> {
    let mut options = BTreeMap::<String, ThemeOption>::new();
    for spec in embedded_theme_cache().values() {
        options.insert(spec.option.key.clone(), spec.option.clone());
    }

    for spec in runtime_themes_with_dir(runtime_dir, seed_embedded).into_values() {
        options.insert(spec.option.key.clone(), spec.option);
    }

    options.into_values().collect()
}

fn runtime_themes() -> HashMap<String, RuntimeThemeSpec> {
    runtime_themes_with_dir(None, true)
}

fn runtime_themes_with_dir(
    runtime_dir: Option<&Path>,
    seed_embedded: bool,
) -> HashMap<String, RuntimeThemeSpec> {
    let Some(dir) = resolved_runtime_themes_dir(runtime_dir, seed_embedded) else {
        return HashMap::default();
    };

    load_runtime_themes_from_dir(&dir)
}

fn resolved_runtime_themes_dir(runtime_dir: Option<&Path>, seed_embedded: bool) -> Option<PathBuf> {
    let dir = match runtime_dir {
        Some(path) => path.to_path_buf(),
        None => gitcomet_state::session::user_themes_dir()?,
    };

    if fs::create_dir_all(&dir).is_err() {
        return None;
    }

    if seed_embedded {
        seed_runtime_themes_dir(&dir);
    }

    Some(dir)
}

fn seed_runtime_themes_dir(dir: &Path) {
    for file in EMBEDDED_THEME_FILES {
        let path = dir.join(format!("{}.json", file.stem));
        if path.exists() {
            continue;
        }
        let _ = fs::write(path, file.json);
    }
}

fn load_runtime_themes_from_dir(dir: &Path) -> HashMap<String, RuntimeThemeSpec> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashMap::default();
    };

    let mut files = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    files.sort();

    let mut themes = HashMap::default();
    for path in files {
        let Ok(json) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(specs) = load_theme_specs_from_json(&json) else {
            continue;
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

fn load_theme_specs_from_bundle(
    bundle: ThemeBundleFile,
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
    serde_json::from_str(json).map_err(ThemeParseError::Parse)
}

pub(crate) fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

#[cfg(test)]
mod tests {
    use super::{
        AppTheme, DEFAULT_DARK_THEME_KEY, DEFAULT_LIGHT_THEME_KEY, GRAPH_LANE_PALETTE_SIZE, Rgba,
        available_themes, derived_syntax_color, has_theme_key, load_theme_specs_from_json,
        merged_theme_options, theme_label, with_alpha,
    };
    use std::fs;
    use tempfile::tempdir;

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
    fn parses_theme_json_with_alpha_overrides() {
        let json = r##"{
            "name": "Fixture",
            "themes": [
                {
                    "key": "fixture",
                    "name": "Fixture",
                    "appearance": "dark",
                    "colors": {
                        "window_bg": "#0d1016ff",
                        "surface_bg": "#1f2127ff",
                        "surface_bg_elevated": "#1f2127ff",
                        "active_section": "#2d2f34ff",
                        "border": "#2d2f34ff",
                        "tooltip_bg": "#000000ff",
                        "tooltip_text": "#ffffffff",
                        "text": "#bfbdb6ff",
                        "text_muted": "#8a8986ff",
                        "accent": "#5ac1feff",
                        "hover": "#2d2f34ff",
                        "active": { "hex": "#2d2f34ff", "alpha": 0.78 },
                        "focus_ring": { "hex": "#5ac1feff", "alpha": 0.60 },
                        "focus_ring_bg": { "hex": "#5ac1feff", "alpha": 0.16 },
                        "scrollbar_thumb": { "hex": "#8a8986ff", "alpha": 0.30 },
                        "scrollbar_thumb_hover": { "hex": "#8a8986ff", "alpha": 0.42 },
                        "scrollbar_thumb_active": { "hex": "#8a8986ff", "alpha": 0.52 },
                        "danger": "#ef7177ff",
                        "warning": "#feb454ff",
                        "success": "#aad84cff",
                        "diff_add_bg": "#102030ff",
                        "diff_add_text": "#405060ff",
                        "diff_remove_bg": "#203040ff",
                        "diff_remove_text": "#506070ff",
                        "input_placeholder": "#708090ff",
                        "accent_text": "#112233ff",
                        "graph_lane_hues": [0.25, 0.75]
                    },
                    "radii": {
                        "panel": 2.0,
                        "pill": 2.0,
                        "row": 2.0
                    }
                }
            ]
        }"##;

        let theme = AppTheme::from_json_str(json).expect("theme JSON should parse");

        assert!(theme.is_dark);
        assert_eq!(theme.colors.window_bg, gpui::rgba(0x0d1016ff));
        assert_eq!(theme.colors.border, gpui::rgba(0x2d2f34ff));
        assert_eq!(theme.colors.tooltip_bg, gpui::rgba(0x000000ff));
        assert_eq!(theme.colors.tooltip_text, gpui::rgba(0xffffffff));
        assert_eq!(
            theme.colors.active,
            with_alpha(gpui::rgba(0x2d2f34ff), 0.78)
        );
        assert_eq!(
            theme.colors.scrollbar_thumb_active,
            with_alpha(gpui::rgba(0x8a8986ff), 0.52)
        );
        assert_eq!(theme.colors.diff_add_bg, gpui::rgba(0x102030ff));
        assert_eq!(theme.colors.diff_add_text, gpui::rgba(0x405060ff));
        assert_eq!(theme.colors.diff_remove_bg, gpui::rgba(0x203040ff));
        assert_eq!(theme.colors.diff_remove_text, gpui::rgba(0x506070ff));
        assert_eq!(theme.colors.input_placeholder, gpui::rgba(0x708090ff));
        assert_eq!(theme.colors.accent_text, gpui::rgba(0x112233ff));
        assert_eq!(theme.graph_lane_palette.as_slice().len(), 2);
        assert_eq!(
            theme.graph_lane_palette.as_slice()[0],
            gpui::hsla(0.25, 0.75, 0.62, 1.0).into()
        );
        assert_eq!(theme.syntax.comment, theme.colors.text_muted);
        assert_eq!(
            theme.syntax.keyword,
            derived_syntax_color(theme.is_dark, &theme.colors, theme.colors.accent)
        );
        assert_eq!(theme.syntax.variable, None);
        assert_eq!(theme.radii.panel, 2.0);
    }

    #[test]
    fn parses_theme_json_with_optional_syntax_overrides() {
        let json = r##"{
            "name": "Fixture",
            "themes": [
                {
                    "key": "fixture",
                    "name": "Fixture",
                    "appearance": "light",
                    "colors": {
                        "window_bg": "#fafafaff",
                        "surface_bg": "#ebebecff",
                        "surface_bg_elevated": "#ebebecff",
                        "active_section": "#fafafaff",
                        "border": "#dfdfe0ff",
                        "text": "#242529ff",
                        "text_muted": "#58585aff",
                        "accent": "#5c78e2ff",
                        "hover": "#dfdfe0ff",
                        "active": { "hex": "#dfdfe0ff", "alpha": 0.88 },
                        "focus_ring": { "hex": "#5c78e2ff", "alpha": 0.52 },
                        "focus_ring_bg": { "hex": "#5c78e2ff", "alpha": 0.12 },
                        "scrollbar_thumb": { "hex": "#58585aff", "alpha": 0.26 },
                        "scrollbar_thumb_hover": { "hex": "#58585aff", "alpha": 0.36 },
                        "scrollbar_thumb_active": { "hex": "#58585aff", "alpha": 0.46 },
                        "danger": "#de3e35ff",
                        "warning": "#d2b67cff",
                        "success": "#3f953aff"
                    },
                    "syntax": {
                        "keyword": "#112233ff",
                        "variable": "#445566ff",
                        "comment_doc": "#778899ff"
                    },
                    "radii": {
                        "panel": 2.0,
                        "pill": 2.0,
                        "row": 2.0
                    }
                }
            ]
        }"##;

        let theme = AppTheme::from_json_str(json).expect("theme JSON should parse");

        assert_eq!(theme.syntax.keyword, gpui::rgba(0x112233ff));
        assert_eq!(theme.syntax.variable, Some(gpui::rgba(0x445566ff)));
        assert_eq!(theme.syntax.comment_doc, gpui::rgba(0x778899ff));
        assert_eq!(theme.syntax.comment, theme.colors.text_muted);
        assert_eq!(
            theme.syntax.string,
            derived_syntax_color(theme.is_dark, &theme.colors, theme.colors.warning)
        );
    }

    #[test]
    fn loads_theme_json_from_file() {
        let dir = tempdir().expect("temp dir should exist");
        let path = dir.path().join("theme.json");
        fs::write(
            &path,
            r##"{
                "name": "Fixture",
                "themes": [
                    {
                        "key": "fixture",
                        "name": "Fixture",
                        "appearance": "light",
                        "colors": {
                            "window_bg": "#fafafaff",
                            "surface_bg": "#ebebecff",
                            "surface_bg_elevated": "#ebebecff",
                            "active_section": "#fafafaff",
                            "border": "#dfdfe0ff",
                            "text": "#242529ff",
                            "text_muted": "#58585aff",
                            "accent": "#5c78e2ff",
                            "hover": "#dfdfe0ff",
                            "active": { "hex": "#dfdfe0ff", "alpha": 0.88 },
                            "focus_ring": { "hex": "#5c78e2ff", "alpha": 0.52 },
                            "focus_ring_bg": { "hex": "#5c78e2ff", "alpha": 0.12 },
                            "scrollbar_thumb": { "hex": "#58585aff", "alpha": 0.26 },
                            "scrollbar_thumb_hover": { "hex": "#58585aff", "alpha": 0.36 },
                            "scrollbar_thumb_active": { "hex": "#58585aff", "alpha": 0.46 },
                            "danger": "#de3e35ff",
                            "warning": "#d2b67cff",
                            "success": "#3f953aff"
                        },
                        "radii": {
                            "panel": 2.0,
                            "pill": 2.0,
                            "row": 2.0
                        }
                    }
                ]
            }"##,
        )
        .expect("theme file should be written");

        let theme = AppTheme::from_json_path(&path).expect("theme file should load");

        assert!(!theme.is_dark);
        assert_eq!(theme.colors.text, gpui::rgba(0x242529ff));
        assert_eq!(theme.colors.tooltip_bg, gpui::rgba(0x000000ff));
        assert_eq!(theme.colors.tooltip_text, gpui::rgba(0xffffffff));
        assert_eq!(
            theme.colors.active,
            with_alpha(gpui::rgba(0xdfdfe0ff), 0.88)
        );
        assert_eq!(theme.colors.diff_add_bg, gpui::rgba(0xe6ffedff));
        assert_eq!(theme.colors.diff_add_text, gpui::rgba(0x22863aff));
        assert_eq!(theme.colors.diff_remove_bg, gpui::rgba(0xffeef0ff));
        assert_eq!(theme.colors.diff_remove_text, gpui::rgba(0xcb2431ff));
        assert_eq!(theme.colors.input_placeholder, gpui::rgba(0x00000033));
        assert_eq!(theme.colors.accent_text, gpui::rgba(0xffffffff));
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
            dark.colors.focus_ring,
            with_alpha(gpui::rgba(0x5ac1feff), 0.60)
        );
        assert_eq!(
            light.colors.scrollbar_thumb_hover,
            with_alpha(gpui::rgba(0x58585aff), 0.36)
        );
        assert_eq!(dark.colors.diff_add_bg, gpui::rgba(0x0b2e1cff));
        assert_eq!(light.colors.diff_remove_text, gpui::rgba(0xcb2431ff));
        assert_eq!(dark.colors.input_placeholder, gpui::rgba(0xffffff59));
        assert_eq!(light.colors.accent_text, gpui::rgba(0xffffffff));
        assert_eq!(
            dark.graph_lane_palette.as_slice().len(),
            GRAPH_LANE_PALETTE_SIZE
        );
    }

    #[test]
    fn built_in_tokyo_night_theme_loads_from_embedded_json() {
        let theme = AppTheme::from_key("tokyo_night").expect("Tokyo Night theme should load");

        assert!(theme.is_dark);
        assert_eq!(theme.colors.window_bg, gpui::rgba(0x1a1b26ff));
        assert_eq!(theme.syntax.keyword, gpui::rgba(0xbb9af7ff));
        assert_eq!(theme.syntax.string, gpui::rgba(0x9ece6aff));
        assert_eq!(theme.syntax.variable, Some(gpui::rgba(0xc0caf5ff)));
    }

    #[test]
    fn bundled_theme_file_exposes_multiple_themes() {
        let json = r##"{
            "name": "Classic",
            "themes": [
                {
                    "key": "classic_light",
                    "name": "Classic Light",
                    "appearance": "light",
                    "colors": {
                        "window_bg": "#ffffffff",
                        "surface_bg": "#f9f9f9ff",
                        "surface_bg_elevated": "#f7f7f7ff",
                        "active_section": "#ffffffff",
                        "border": "#d2d2d2ff",
                        "text": "#000000ff",
                        "text_muted": "#505050ff",
                        "accent": "#1f6ae2ff",
                        "hover": "#d0d0d0ff",
                        "active": "#c7deffff",
                        "focus_ring": { "hex": "#1f6ae2ff", "alpha": 0.52 },
                        "focus_ring_bg": { "hex": "#1f6ae2ff", "alpha": 0.12 },
                        "scrollbar_thumb": "#c8c8c8aa",
                        "scrollbar_thumb_hover": "#c8c8c8aa",
                        "scrollbar_thumb_active": "#c8c8c8ff",
                        "danger": "#c5060bff",
                        "warning": "#c99401ff",
                        "success": "#036a07ff"
                    },
                    "radii": {
                        "panel": 2.0,
                        "pill": 2.0,
                        "row": 2.0
                    }
                },
                {
                    "key": "classic_dark",
                    "name": "Classic Dark",
                    "appearance": "dark",
                    "colors": {
                        "window_bg": "#131313ff",
                        "surface_bg": "#1e1d1eff",
                        "surface_bg_elevated": "#1e1d1eff",
                        "active_section": "#353436ff",
                        "border": "#404040ff",
                        "text": "#cacccaff",
                        "text_muted": "#9e9e9eff",
                        "accent": "#c28b12ff",
                        "hover": "#353436ff",
                        "active": "#474646ff",
                        "focus_ring": { "hex": "#c28b12ff", "alpha": 0.60 },
                        "focus_ring_bg": { "hex": "#c28b12ff", "alpha": 0.16 },
                        "scrollbar_thumb": "#4c4d4daa",
                        "scrollbar_thumb_hover": "#4c4d4dff",
                        "scrollbar_thumb_active": "#4c4d4dff",
                        "danger": "#c74028ff",
                        "warning": "#b0a878ff",
                        "success": "#62ba46ff"
                    },
                    "radii": {
                        "panel": 2.0,
                        "pill": 2.0,
                        "row": 2.0
                    }
                }
            ]
        }"##;

        let specs = load_theme_specs_from_json(json).expect("bundle should parse");

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
    fn runtime_theme_dir_overrides_and_extends_embedded_themes() {
        let dir = tempdir().expect("temp dir should exist");
        fs::write(
            dir.path().join("custom_theme.json"),
            r##"{
                "name": "Custom Theme",
                "themes": [
                    {
                        "key": "custom_theme",
                        "name": "Custom Theme",
                        "appearance": "dark",
                        "colors": {
                            "window_bg": "#000000ff",
                            "surface_bg": "#111111ff",
                            "surface_bg_elevated": "#222222ff",
                            "active_section": "#333333ff",
                            "border": "#444444ff",
                            "text": "#eeeeeeff",
                            "text_muted": "#999999ff",
                            "accent": "#abcdef12",
                            "hover": "#555555ff",
                            "active": { "hex": "#666666ff", "alpha": 0.9 },
                            "focus_ring": { "hex": "#777777ff", "alpha": 0.5 },
                            "focus_ring_bg": { "hex": "#777777ff", "alpha": 0.2 },
                            "scrollbar_thumb": "#88888880",
                            "scrollbar_thumb_hover": "#888888ff",
                            "scrollbar_thumb_active": "#999999ff",
                            "danger": "#aa0000ff",
                            "warning": "#bb9900ff",
                            "success": "#00aa00ff"
                        },
                        "radii": {
                            "panel": 2.0,
                            "pill": 2.0,
                            "row": 2.0
                        }
                    }
                ]
            }"##,
        )
        .expect("custom theme file should be written");

        let themes = merged_theme_options(Some(dir.path()), false);
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
}
