//! Theme tiles: one gradient orb per theme, grouped Automatic / Dark / Light /
//! Custom. The Appearance page and a workspace's theme override share them.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::theme::{ThemeCatalog, ThemeOption, ThemePreviewColors};
use crate::view::components::{InteractionState, InteractionStyle};
use gitcomet_state::session::WorkspaceId;

const ORB_SIZE_PX: f32 = 44.0;
const ORB_RING_GAP_PX: f32 = 2.0;
/// Scaled with the orb, like the gap, so the ring keeps its proportions.
const ORB_RING_WIDTH_PX: f32 = 2.0;
const TILE_WIDTH_PX: f32 = 116.0;

/// Who a picked tile applies to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ThemeTarget {
    App,
    Workspace(WorkspaceId),
}

impl ThemeTarget {
    fn id_prefix(self) -> &'static str {
        match self {
            Self::App => "settings_window_theme",
            Self::Workspace(_) => "settings_window_workspace_theme",
        }
    }
}

/// What a tile's orb paints: one theme, or Automatic's light and dark halves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum OrbPaint {
    Single(ThemePreviewColors),
    Split {
        light: ThemePreviewColors,
        dark: ThemePreviewColors,
    },
}

impl OrbPaint {
    pub(super) fn for_mode(mode: &ThemeMode, themes: &ThemeCatalog) -> Option<Self> {
        let preview = |key: &str| themes.get(key).map(|option| option.preview);
        match mode {
            ThemeMode::Automatic => Some(Self::Split {
                light: preview(crate::theme::DEFAULT_LIGHT_THEME_KEY)?,
                dark: preview(crate::theme::DEFAULT_DARK_THEME_KEY)?,
            }),
            ThemeMode::Named(key) => preview(key).map(Self::Single),
        }
    }
}

struct ThemeTile {
    /// `None` is a workspace's "Follow app theme".
    mode: Option<ThemeMode>,
    label: SharedString,
    orb: OrbPaint,
}

impl ThemeTile {
    fn key(&self) -> &str {
        self.mode.as_ref().map_or("follow_app", |mode| mode.key())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThemeGroup {
    Automatic,
    Dark,
    Light,
    Custom,
}

impl ThemeGroup {
    const ALL: [Self; 4] = [Self::Automatic, Self::Dark, Self::Light, Self::Custom];

    fn key(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Custom => "custom",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Custom => "Custom",
        }
    }

    fn of(option: &ThemeOption) -> Self {
        if option.custom {
            Self::Custom
        } else if option.is_dark {
            Self::Dark
        } else {
            Self::Light
        }
    }
}

/// Tiles per group, in the catalog's order. Empty groups are left out.
fn grouped_tiles(
    themes: &ThemeCatalog,
    mut leading: Option<ThemeTile>,
) -> Vec<(ThemeGroup, Vec<ThemeTile>)> {
    let mut groups = Vec::new();
    for group in ThemeGroup::ALL {
        let tiles: Vec<ThemeTile> = match group {
            ThemeGroup::Automatic => leading
                .take()
                .into_iter()
                .chain(
                    OrbPaint::for_mode(&ThemeMode::Automatic, themes).map(|orb| ThemeTile {
                        mode: Some(ThemeMode::Automatic),
                        label: ThemeMode::Automatic.label().into(),
                        orb,
                    }),
                )
                .collect(),
            _ => themes
                .themes
                .iter()
                .filter(|option| ThemeGroup::of(option) == group)
                .map(|option| ThemeTile {
                    mode: Some(ThemeMode::Named(option.key.clone())),
                    label: option.label.clone().into(),
                    orb: OrbPaint::Single(option.preview),
                })
                .collect(),
        };
        if !tiles.is_empty() {
            groups.push((group, tiles));
        }
    }
    groups
}

/// Tile keys per group, for tests that check the grouping without a window.
#[cfg(test)]
pub(super) fn grouped_tile_keys() -> Vec<(&'static str, Vec<String>)> {
    grouped_tiles(&ThemeCatalog::load(), None)
        .into_iter()
        .map(|(group, tiles)| {
            let keys = tiles.iter().map(|tile| tile.key().to_string()).collect();
            (group.key(), keys)
        })
        .collect()
}

/// Glow centres as fractions of the orb, (accent, secondary). Light orbs glow
/// from the top-right, dark ones mirror it (t3code's layout); Automatic's
/// halves pull both glows inside their own half.
type GlowCentres = ((f32, f32), (f32, f32));

fn glow_centres(is_dark: bool) -> GlowCentres {
    if is_dark {
        ((0.28, 0.78), (0.82, 0.18))
    } else {
        ((0.72, 0.22), (0.18, 0.82))
    }
}

/// `#rrggbb`; alpha goes in the SVG's opacity attributes.
fn hex(color: gpui::Rgba) -> String {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.red),
        byte(color.green),
        byte(color.blue)
    )
}

fn orb_base(colors: ThemePreviewColors) -> gpui::Rgba {
    let toward = if colors.is_dark {
        gpui::rgba(0x09090bff)
    } else {
        gpui::rgba(0xffffffff)
    };
    mix_color(colors.base, toward, 0.2)
}

/// Unlike `theme::mix_colors`, this mixes alpha too rather than forcing 1.0.
fn mix_color(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    gpui::Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}

/// One theme's layers in a 64-unit box: gradient defs, then the blurred fill.
fn orb_layers_svg(colors: ThemePreviewColors, id: &str, centres: GlowCentres) -> (String, String) {
    let ((ax, ay), (sx, sy)) = centres;
    // CSS `radial-gradient(circle at …)` reaches the farthest corner.
    let farthest = |x: f32, y: f32| x.max(1.0 - x).hypot(y.max(1.0 - y)) * 64.0;
    let opacity =
        |color: gpui::Rgba, scale: f32| format!("{:.3}", color.alpha.clamp(0.0, 1.0) * scale);
    let (glow, glow_full, glow_middle) = (
        hex(colors.glow),
        opacity(colors.glow, 1.0),
        opacity(colors.glow, if colors.is_dark { 0.62 } else { 0.72 }),
    );
    let (secondary, secondary_full) = (hex(colors.secondary), opacity(colors.secondary, 0.45));
    let defs = format!(
        r#"<radialGradient id="a{id}" gradientUnits="userSpaceOnUse" cx="{:.2}" cy="{:.2}" r="{:.2}"><stop offset="0" stop-color="{glow}" stop-opacity="{glow_full}"/><stop offset="0.28" stop-color="{glow}" stop-opacity="{glow_middle}"/><stop offset="0.58" stop-color="{glow}" stop-opacity="0"/></radialGradient><radialGradient id="s{id}" gradientUnits="userSpaceOnUse" cx="{:.2}" cy="{:.2}" r="{:.2}"><stop offset="0" stop-color="{secondary}" stop-opacity="{secondary_full}"/><stop offset="0.55" stop-color="{secondary}" stop-opacity="0"/></radialGradient>"#,
        ax * 64.0,
        ay * 64.0,
        farthest(ax, ay),
        sx * 64.0,
        sy * 64.0,
        farthest(sx, sy),
    );
    let base = orb_base(colors);
    let fill = format!(
        r#"<g filter="url(#f)" transform="translate(32 32) scale(1.1) translate(-32 -32)"><rect x="-16" y="-16" width="96" height="96" fill="{}" fill-opacity="{}"/><rect x="-16" y="-16" width="96" height="96" fill="url(#s{id})"/><rect x="-16" y="-16" width="96" height="96" fill="url(#a{id})"/></g>"#,
        hex(base),
        opacity(base, 1.0),
    );
    (defs, fill)
}

fn orb_edge(is_dark: bool) -> gpui::Rgba {
    if is_dark {
        with_alpha(gpui::rgba(0xffffffff), 0.14)
    } else {
        with_alpha(gpui::rgba(0x000000ff), 0.10)
    }
}

/// A theme orb's fill, t3code-style: the theme's chrome as the base, its accent
/// as a strong radial glow and its keyword colour as a soft one, blurred. gpui
/// has no radial gradient, so this is an SVG image. It stays square: the circle
/// and its edge are drawn by gpui, whose rounded corners are anti-aliased at any
/// size, where a circle baked into the image aliases once it is scaled down.
pub(super) fn orb_svg(paint: OrbPaint) -> String {
    // gpui rasterizes at 2x this. The content is all soft gradient, so the
    // exact scale it is drawn at does not matter.
    const INTRINSIC_PX: u32 = 64;
    let mut defs = String::from(
        r#"<filter id="f" x="-25%" y="-25%" width="150%" height="150%"><feGaussianBlur stdDeviation="3.4"/></filter>"#,
    );
    let body = match paint {
        OrbPaint::Single(colors) => {
            let (layer_defs, fill) = orb_layers_svg(colors, "1", glow_centres(colors.is_dark));
            defs.push_str(&layer_defs);
            fill
        }
        OrbPaint::Split { light, dark } => {
            let (light_defs, light_fill) = orb_layers_svg(light, "1", ((0.30, 0.26), (0.22, 0.84)));
            let (dark_defs, dark_fill) = orb_layers_svg(dark, "2", ((0.70, 0.74), (0.78, 0.16)));
            defs.push_str(&light_defs);
            defs.push_str(&dark_defs);
            defs.push_str(
                r#"<clipPath id="l"><rect x="-16" y="-16" width="48" height="96"/></clipPath><clipPath id="r"><rect x="32" y="-16" width="48" height="96"/></clipPath>"#,
            );
            format!(
                r#"<g clip-path="url(#l)">{light_fill}</g><g clip-path="url(#r)">{dark_fill}</g>"#
            )
        }
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{INTRINSIC_PX}" height="{INTRINSIC_PX}" viewBox="0 0 64 64"><defs>{defs}</defs>{body}</svg>"#
    )
}

type OrbKey = [u32; 8];

fn orb_key(paint: OrbPaint) -> OrbKey {
    fn pack(colors: ThemePreviewColors) -> [u32; 4] {
        let rgba = |color: gpui::Rgba| {
            [color.red, color.green, color.blue, color.alpha]
                .into_iter()
                .fold(0u32, |packed, channel| {
                    (packed << 8) | (channel.clamp(0.0, 1.0) * 255.0).round() as u32
                })
        };
        [
            u32::from(colors.is_dark),
            rgba(colors.base),
            rgba(colors.glow),
            rgba(colors.secondary),
        ]
    }
    match paint {
        OrbPaint::Single(colors) => {
            let [a, b, c, d] = pack(colors);
            [a, b, c, d, u32::MAX, 0, 0, 0]
        }
        OrbPaint::Split { light, dark } => {
            let [a, b, c, d] = pack(light);
            let [e, f, g, h] = pack(dark);
            [a, b, c, d, e, f, g, h]
        }
    }
}

/// Far above one orb per theme; only live-reloading a custom theme, a new
/// palette per save, gets here.
const MAX_CACHED_ORB_IMAGES: usize = 128;

/// One decoded image per palette, so a repaint neither rebuilds the SVG nor
/// hands gpui a new image to rasterize.
#[derive(Default)]
pub(super) struct OrbImageCache {
    images: FxHashMap<OrbKey, Arc<gpui::Image>>,
}

impl OrbImageCache {
    pub(super) fn get(&mut self, paint: OrbPaint) -> Arc<gpui::Image> {
        let key = orb_key(paint);
        if self.images.len() >= MAX_CACHED_ORB_IMAGES && !self.images.contains_key(&key) {
            self.images.clear();
        }
        Arc::clone(self.images.entry(key).or_insert_with(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Svg,
                orb_svg(paint).into_bytes(),
            ))
        }))
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.images.len()
    }
}

fn orb_image(paint: OrbPaint) -> Arc<gpui::Image> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<OrbImageCache>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(paint)
}

pub(super) fn theme_orb(paint: OrbPaint, size: Pixels) -> gpui::Div {
    let (placeholder, edge) = match paint {
        OrbPaint::Single(colors) => (orb_base(colors), orb_edge(colors.is_dark)),
        OrbPaint::Split { dark, .. } => (orb_base(dark), orb_edge(true)),
    };
    div()
        .relative()
        .flex_none()
        .size(size)
        .child(
            gpui::img(orb_image(paint))
                .size_full()
                .rounded_full()
                .with_loading(move || {
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(placeholder)
                        .into_any_element()
                }),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded_full()
                .border_1()
                .border_color(edge),
        )
}

impl SettingsWindowView {
    /// The app theme's orb. Its key can name a custom theme deleted since it
    /// was picked; the windows still show `theme` then, so the orb does too.
    pub(super) fn app_theme_orb(&self, themes: &ThemeCatalog, theme: AppTheme) -> OrbPaint {
        OrbPaint::for_mode(&self.theme_mode, themes)
            .unwrap_or(OrbPaint::Single(theme.preview_colors()))
    }

    /// The app-wide theme picker on the Appearance page.
    pub(super) fn app_theme_tile_grid(
        &self,
        themes: &ThemeCatalog,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let selected = Some(self.theme_mode.clone());
        self.theme_tile_grid(themes, ThemeTarget::App, None, selected, theme, cx)
    }

    /// A workspace's override picker: "Follow app theme" first.
    pub(super) fn workspace_theme_tile_grid(
        &self,
        themes: &ThemeCatalog,
        id: WorkspaceId,
        current: Option<ThemeMode>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let follow = ThemeTile {
            mode: None,
            label: "Follow app theme".into(),
            orb: self.app_theme_orb(themes, theme),
        };
        let target = ThemeTarget::Workspace(id);
        self.theme_tile_grid(themes, target, Some(follow), current, theme, cx)
    }

    fn theme_tile_grid(
        &self,
        themes: &ThemeCatalog,
        target: ThemeTarget,
        leading: Option<ThemeTile>,
        selected: Option<ThemeMode>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let ui_scale = self.row_scale(theme);
        let prefix = target.id_prefix();
        let grid_id = format!("{prefix}_grid");
        let grid_debug_id = grid_id.clone();
        let mut grid = div()
            .id(SharedString::from(grid_id))
            .debug_selector(move || grid_debug_id.clone())
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(ui_scale.px(12.0))
            .pb(ui_scale.px(8.0));
        for (group, tiles) in grouped_tiles(themes, leading) {
            let group_id = format!("{prefix}_group_{}", group.key());
            let group_debug_id = group_id.clone();
            let mut row = div()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .flex_wrap()
                .gap(ui_scale.px(4.0));
            for tile in tiles {
                let is_selected = tile.mode == selected;
                row = row.child(self.theme_tile(target, tile, is_selected, theme, cx));
            }
            grid = grid.child(
                div()
                    .id(SharedString::from(group_id))
                    .debug_selector(move || group_debug_id.clone())
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(ui_scale.px(4.0))
                    .child(
                        div()
                            .px_2()
                            .text_size(theme.ui_text(12.0))
                            .line_height(theme.ui_text(16.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.foreground.secondary)
                            .child(group.label()),
                    )
                    .child(row),
            );
        }
        grid
    }

    fn theme_tile(
        &self,
        target: ThemeTarget,
        tile: ThemeTile,
        selected: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let ui_scale = self.row_scale(theme);
        let id = format!("{}_{}", target.id_prefix(), tile.key());
        let debug_id = id.clone();
        let orb_debug_id = format!("{id}_orb");
        let ring = if selected {
            theme.colors.accent.foreground
        } else {
            gpui::rgba(0x00000000)
        };
        let ring_box = ORB_SIZE_PX + 2.0 * (ORB_RING_GAP_PX + ORB_RING_WIDTH_PX);
        let mode = tile.mode.clone();

        div()
            .id(SharedString::from(id))
            .debug_selector(move || debug_id.clone())
            .w(ui_scale.px(TILE_WIDTH_PX))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(ui_scale.px(4.0))
            .px_1()
            .pt(ui_scale.px(6.0))
            .pb(ui_scale.px(6.0))
            .rounded(px(theme.radii.control))
            .cursor(CursorStyle::PointingHand)
            .control_interaction(InteractionStyle::new(theme), InteractionState::default())
            .child(
                div()
                    .debug_selector(move || orb_debug_id.clone())
                    .flex_none()
                    .size(ui_scale.px(ring_box))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border(ui_scale.px(ORB_RING_WIDTH_PX))
                    .border_color(ring)
                    .child(theme_orb(tile.orb, ui_scale.px(ORB_SIZE_PX))),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .text_center()
                    .truncate()
                    .text_size(theme.ui_text(12.0))
                    .line_height(theme.ui_text(16.0))
                    .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                    .text_color(if selected {
                        theme.colors.foreground.primary
                    } else {
                        theme.colors.foreground.secondary
                    })
                    .child(tile.label),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, _e: &ClickEvent, window, cx| match target {
                    ThemeTarget::App => {
                        if let Some(mode) = mode.clone() {
                            this.set_theme_mode(mode, window, cx);
                        }
                    }
                    ThemeTarget::Workspace(id) => {
                        this.set_workspace_theme(id, mode.clone(), cx);
                    }
                }),
            )
    }

    /// A global theme change skips windows whose workspace overrides it; say
    /// so, or the pick looks broken in those windows.
    pub(super) fn workspace_theme_override_row(
        &self,
        themes: &ThemeCatalog,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Option<Stateful<gpui::Div>> {
        // An unknown key (a deleted user theme) follows the app theme.
        let count = crate::workspaces::with_workspaces(cx, |workspaces| {
            workspaces
                .iter()
                .filter_map(|workspace| workspace.theme_mode.as_deref())
                .filter(|raw| ThemeMode::from_catalog_key(raw, themes).is_some())
                .count()
        });
        if count == 0 {
            return None;
        }
        let label = if count == 1 {
            "1 workspace uses its own theme.".to_string()
        } else {
            format!("{count} workspaces use their own theme.")
        };
        Some(
            div()
                .id("settings_window_theme_workspace_overrides")
                .debug_selector(|| "settings_window_theme_workspace_overrides".to_string())
                .w_full()
                .px_2()
                .py_1()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .rounded(px(theme.radii.row))
                .cursor(CursorStyle::PointingHand)
                .control_interaction(InteractionStyle::new(theme), InteractionState::default())
                .text_size(theme.ui_text(13.0))
                .child(
                    div()
                        .text_color(theme.colors.foreground.secondary)
                        .child(label),
                )
                .child(
                    div()
                        .text_color(theme.colors.accent.foreground)
                        .child("Manage in Workspaces"),
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.select_category(SettingsCategory::Workspaces, cx);
                    }),
                ),
        )
    }
}
