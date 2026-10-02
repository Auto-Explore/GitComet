use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction as controls;
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};
use std::sync::OnceLock;

const SPLASH_BACKDROP_DARK_PNG_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/splash_backdrop_dark.png"));
const SPLASH_BACKDROP_LIGHT_PNG_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/splash_backdrop_light.png"));
/// Bottom margin the main content card leaves for the bottom bar. The collapsed
/// section popover matches it so its top/bottom gaps read symmetric.
const CONTENT_CARD_BOTTOM_MARGIN_PX: f32 = 2.0;
/// Width of the panel a collapsed-rail section (Local/Remote branches,
/// Worktrees, Submodules, Stashes) opens into. Wider than the expanded
/// sidebar's 280px default: the rail's popover is transient and floats over the
/// canvas, so it can afford the room that branch names, worktree paths and
/// stash summaries want, without the pane's permanent cost.
const COLLAPSED_POPOVER_WIDTH_PX: f32 = 340.0;
static SPLASH_BACKDROP_DARK_IMAGE_CACHE: OnceLock<Arc<gpui::Image>> = OnceLock::new();
static SPLASH_BACKDROP_LIGHT_IMAGE_CACHE: OnceLock<Arc<gpui::Image>> = OnceLock::new();

/// Interstitial geometry (loading and git-unavailable cards, Home buttons).
const SPLASH_CARD_MAX_WIDTH_PX: f32 = components::INTERSTITIAL_CARD_MAX_WIDTH_PX;
const SPLASH_BODY_MAX_WIDTH_PX: f32 = 440.0;
const SPLASH_DETAIL_MAX_WIDTH_PX: f32 = 460.0;

fn main_content_card_radius(theme: AppTheme) -> f32 {
    theme.radii.control
}

/// The splash backdrop for the theme's appearance: the product's own, else
/// GitComet's.
pub(in crate::view) fn load_splash_backdrop_image(is_dark: bool) -> Arc<gpui::Image> {
    let branding = gitcomet_core::identity::current().branding();
    let (cache, bytes) = if is_dark {
        (
            &SPLASH_BACKDROP_DARK_IMAGE_CACHE,
            branding
                .splash_backdrop_dark_png
                .unwrap_or(SPLASH_BACKDROP_DARK_PNG_BYTES),
        )
    } else {
        (
            &SPLASH_BACKDROP_LIGHT_IMAGE_CACHE,
            branding
                .splash_backdrop_light_png
                .unwrap_or(SPLASH_BACKDROP_LIGHT_PNG_BYTES),
        )
    };
    cache
        .get_or_init(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                bytes.to_vec(),
            ))
        })
        .clone()
}

/// Children clip to rectangles, so full-bleed content inside the card can
/// square off its rounded left corners. These caps repaint the two left corner
/// notches (the area between the content rectangle's corner and the card's
/// inner arc) in the surrounding surface color, restoring the rounding over
/// anything the content paints. Canvas elements take no hitboxes, so the
/// overlay is invisible to the mouse.
fn card_left_corner_caps(radius: Pixels, color: gpui::Rgba) -> AnyElement {
    #[derive(Clone, Copy)]
    enum CapCorner {
        TopLeft,
        BottomLeft,
    }

    let cap = move |corner: CapCorner| {
        let paint = move |bounds: gpui::Bounds<Pixels>, window: &mut Window| {
            use gpui::PathBuilder;
            // Quarter-circle bezier approximation constant.
            const K: f32 = 0.552_284_7;
            let r = bounds.size.width;
            let k = r * K;
            let (corner_pt, arc_start, arc_end, c1, c2) = match corner {
                CapCorner::TopLeft => (
                    bounds.origin,
                    point(bounds.left(), bounds.top() + r),
                    point(bounds.left() + r, bounds.top()),
                    point(bounds.left(), bounds.top() + r - k),
                    point(bounds.left() + r - k, bounds.top()),
                ),
                CapCorner::BottomLeft => (
                    point(bounds.left(), bounds.bottom()),
                    point(bounds.left() + r, bounds.bottom()),
                    point(bounds.left(), bounds.top()),
                    point(bounds.left() + r - k, bounds.bottom()),
                    point(bounds.left(), bounds.top() + k),
                ),
            };
            let mut path = PathBuilder::fill();
            path.move_to(arc_start);
            path.cubic_bezier_to(arc_end, c1, c2);
            path.line_to(corner_pt);
            path.line_to(arc_start);
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        };
        let positioned = div().absolute().size(radius);
        let positioned = match corner {
            CapCorner::TopLeft => positioned.top_0().left_0(),
            CapCorner::BottomLeft => positioned.bottom_0().left_0(),
        };
        positioned.child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, _, window, _| paint(bounds, window),
            )
            .size_full(),
        )
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(cap(CapCorner::TopLeft))
        .child(cap(CapCorner::BottomLeft))
        .into_any_element()
}

impl GitCometView {
    pub(super) fn splash_backdrop_base(&self) -> gpui::Background {
        if self.theme.is_dark {
            gpui::rgba(0x0d0f13ff).into()
        } else {
            gpui::linear_gradient(
                180.0,
                gpui::linear_color_stop(gpui::rgba(0xe7f3fdff), 0.0),
                gpui::linear_color_stop(gpui::rgba(0xffffffff), 0.7),
            )
        }
    }

    fn splash_backdrop_image_layer(&self) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .id("splash_backdrop_image")
            .debug_selector(|| "splash_backdrop_image".to_string())
            .child(
                gpui::img(self.splash_backdrop_image.clone())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover),
            )
            .into_any_element()
    }

    pub(super) fn has_repo_tabs(&self) -> bool {
        !self.state.repos.is_empty()
    }

    pub(super) fn git_runtime_unavailable(&self) -> bool {
        matches!(
            self.state.git_runtime.availability,
            gitcomet_core::process::GitExecutableAvailability::Unavailable { .. }
        )
    }

    fn git_runtime_unavailable_detail(&self) -> String {
        self.state
            .git_runtime
            .unavailable_detail()
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{} could not find a usable Git executable.",
                    crate::view::product_name()
                )
            })
    }

    fn git_unavailable_status_icon(theme: AppTheme, ui_scale_percent: u32) -> AnyElement {
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        div()
            .id("git_unavailable_status_icon")
            .debug_selector(|| "git_unavailable_status_icon".to_string())
            .size(scaled_px(56.0))
            .flex()
            .items_center()
            .justify_center()
            .child(svg_icon(
                "icons/warning.svg",
                theme.colors.status.warning.foreground,
                scaled_px(36.0),
            ))
            .into_any_element()
    }

    fn git_runtime_unavailable_detail_content(&self) -> AnyElement {
        let detail = self.git_runtime_unavailable_detail();
        if let Some((summary, recovery)) = detail.split_once(". ") {
            return div()
                .flex()
                .flex_col()
                .gap(px(0.0))
                .child(format!("{summary}."))
                .child(recovery.to_string())
                .into_any_element();
        }

        div().child(detail).into_any_element()
    }

    fn should_show_git_unavailable_overlay(&self) -> bool {
        renders_full_chrome(self.view_mode)
            && self.has_repo_tabs()
            && self.git_runtime_unavailable()
    }

    #[cfg(test)]
    pub(crate) fn blocks_non_repository_actions(&self) -> bool {
        repository_entry_interstitial_active(self.view_mode, self.has_repo_tabs())
            || matches!(self.view_mode, GitCometViewMode::Normal)
                && !self.state.git_runtime.is_available()
    }

    pub(crate) fn blocks_repository_management_actions(&self) -> bool {
        !self.shell_action_allowed(super::shell_policy::ShellAction::RepositoryEntry)
    }

    pub(crate) fn is_home_screen_active(&self) -> bool {
        should_show_home_screen(
            self.view_mode,
            self.has_repo_tabs(),
            self.startup_bootstrap_loading(),
        )
    }

    fn is_startup_repository_loading_screen_active(&self) -> bool {
        should_show_startup_repository_loading_screen(
            self.view_mode,
            self.has_repo_tabs(),
            self.startup_bootstrap_loading(),
        )
    }

    /// A bootstrap deferred until Git recovers stays pending (it keeps the
    /// workspace membership) but shows Home's Git-unavailable screen.
    fn startup_bootstrap_loading(&self) -> bool {
        self.startup_repo_bootstrap_pending && !self.git_runtime_unavailable()
    }

    pub(super) fn sync_title_bar_repo_tab_actions(&mut self, cx: &mut gpui::Context<Self>) {
        let enabled = titlebar_repo_tab_actions_enabled(self.view_mode, self.has_repo_tabs());
        self.title_bar
            .update(cx, |bar, cx| bar.set_repo_tab_actions_enabled(enabled, cx));
    }

    pub(super) fn interstitial_logo(_theme: AppTheme, size: Pixels) -> AnyElement {
        div()
            .id("repository_entry_logo")
            .size(size)
            // `svg()` paints a one-colour mask; the logo has two colours.
            .child(gpui::img("brand/logo.svg").w(size).h(size))
            .into_any_element()
    }

    pub(super) fn interstitial_backdrop(&self) -> AnyElement {
        div()
            .id("splash_backdrop_native")
            .debug_selector(|| "splash_backdrop_native".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .bg(self.splash_backdrop_base())
            .child(self.splash_backdrop_image_layer())
            .into_any_element()
    }

    fn interstitial_shell(
        &self,
        id: &'static str,
        content: impl IntoElement,
        theme: AppTheme,
    ) -> AnyElement {
        components::interstitial(
            id,
            self.splash_backdrop_base(),
            self.interstitial_backdrop(),
            content,
            theme,
            crate::ui_scale::UiScale::from_percent(self.ui_scale_percent),
        )
    }

    fn git_unavailable_open_settings_button(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let settings_tooltip: SharedString = "Open settings".into();

        components::interstitial_cta_button(
            "git_unavailable_open_settings",
            "Open Settings",
            "icons/cog.svg",
            true,
            self.theme,
            crate::ui_scale::UiScale::from_percent(self.ui_scale_percent),
        )
        .gitcomet_tooltip(self.theme, settings_tooltip)
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|this, _e, _window, cx| {
                this.open_repo_panel = false;
                cx.defer(crate::view::open_settings_window);
                cx.notify();
            }),
        )
    }

    fn git_unavailable_panel_content(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let scaled_px = crate::ui_scale::scaler(self.ui_scale_percent);
        let detail_bg = with_alpha(
            theme.colors.surface.canvas,
            if theme.is_dark { 0.36 } else { 0.82 },
        );
        let detail_border = with_alpha(
            theme.colors.stroke.default,
            if theme.is_dark { 0.96 } else { 0.82 },
        );

        div()
            .id("git_unavailable_card")
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(Self::git_unavailable_status_icon(
                theme,
                self.ui_scale_percent,
            ))
            .child(
                div()
                    .text_size(theme.ui_text(18.0))
                    .font_weight(FontWeight::BOLD)
                    .text_center()
                    .child("Git executable unavailable"),
            )
            .child(
                div()
                    .max_w(scaled_px(SPLASH_BODY_MAX_WIDTH_PX))
                    .text_center()
                    .text_size(self.theme.ui_text(14.0))
                    .line_height(self.theme.ui_text(22.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(format!(
                        "{} cannot open, refresh, or run repository actions until a Git executable is configured.",
                        crate::view::product_name()
                    )),
            )
            .child(
                div()
                    .id("git_unavailable_detail")
                    .w_full()
                    .max_w(scaled_px(SPLASH_DETAIL_MAX_WIDTH_PX))
                    .rounded(px(theme.radii.panel))
                    .border_1()
                    .border_color(detail_border)
                    .bg(detail_bg)
                    .px_3()
                    .py_2()
                    .text_size(self.theme.ui_text(12.0))
                    .line_height(self.theme.ui_text(18.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(self.git_runtime_unavailable_detail_content()),
            )
            .child(
                div()
                    .pt_1()
                    .child(self.git_unavailable_open_settings_button(cx)),
            )
            .into_any_element()
    }

    pub(super) fn git_unavailable_splash(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        self.interstitial_shell(
            "git_unavailable_screen",
            self.git_unavailable_panel_content(theme, cx),
            theme,
        )
    }

    fn git_unavailable_overlay(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let scaled_px = crate::ui_scale::scaler(self.ui_scale_percent);
        let border_glow = with_alpha(
            theme.colors.stroke.default,
            if theme.is_dark { 0.86 } else { 0.74 },
        );

        div()
            .id("git_unavailable_overlay")
            .debug_selector(|| "git_unavailable_overlay".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .bg(with_alpha(
                theme.colors.surface.canvas,
                if theme.is_dark { 0.76 } else { 0.82 },
            ))
            .child(self.interstitial_backdrop())
            .child(
                div()
                    .relative()
                    .size_full()
                    .px_3()
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(scaled_px(SPLASH_CARD_MAX_WIDTH_PX))
                            .bg(with_alpha(
                                theme.colors.surface.panel,
                                if theme.is_dark { 0.96 } else { 0.98 },
                            ))
                            .border_1()
                            .border_color(border_glow)
                            .rounded(px(theme.radii.panel))
                            .shadow(vec![gpui::BoxShadow {
                                color: gpui::rgba(if theme.is_dark {
                                    0x00000052
                                } else {
                                    0x171a3b14
                                })
                                .into(),
                                offset: point(px(0.0), px(22.0)),
                                blur_radius: px(52.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }])
                            .p_4()
                            .child(self.git_unavailable_panel_content(theme, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn startup_repository_loading_screen(&mut self) -> AnyElement {
        let theme = self.theme;
        let ui_scale_percent = self.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

        self.interstitial_shell(
            "repository_loading_screen",
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(Self::interstitial_logo(theme, scaled_px(84.0)))
                .child(
                    div()
                        .text_size(theme.ui_text(18.0))
                        .font_weight(FontWeight::BOLD)
                        .child("Loading repository session"),
                )
                .child(
                    div()
                        .text_size(self.theme.ui_text(14.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(format!(
                            "{} is opening your workspace.",
                            crate::view::product_name()
                        )),
                )
                .child(
                    div()
                        .pt_1()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(self.theme.ui_text(14.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(svg_spinner(
                            ("repository_loading_spinner", 0u64),
                            theme.colors.accent.foreground,
                            scaled_px(16.0),
                        ))
                        .child("Please wait…"),
                ),
            theme,
        )
    }

    /// The vertical icon rail shown in place of the sidebar while it is collapsed.
    /// An expand affordance sits at the top; below it, one toggle per section that
    /// opens that section in a floating popover without expanding the sidebar.
    fn collapsed_sidebar_rail(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let active = self.sidebar_collapsed_popover;
        let icon_muted = theme.colors.foreground.secondary;
        let slot = scaled_px(28.0);

        let repo = self.active_repo();
        let sections: Vec<_> = CollapsedSidebarSection::ALL
            .into_iter()
            .filter(|section| section.is_available(repo))
            .collect();
        let icons = sections.into_iter().map(|section| {
            let is_active = active == Some(section);
            let icon_color = if is_active {
                theme.colors.foreground.primary
            } else {
                icon_muted
            };
            div()
                .id(section.element_id())
                .debug_selector(move || section.element_id().to_string())
                .flex()
                .items_center()
                .justify_center()
                .size(slot)
                .rounded(px(theme.radii.control))
                .cursor(CursorStyle::PointingHand)
                .control_interaction(
                    InteractionStyle::new(theme),
                    InteractionState::default().open(is_active),
                )
                .child(svg_icon(section.icon_path(), icon_color, scaled_px(16.0)))
                .on_pointer_click(
                    MouseButton::Left,
                    cx.listener(move |this, _e, _window, cx| {
                        this.toggle_sidebar_collapsed_popover(section, cx);
                    }),
                )
                .gitcomet_tooltip(theme, section.title().into())
        });

        div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .h_full()
            .pt(scaled_px(6.0))
            .gap(scaled_px(3.0))
            .children(icons)
            .into_any_element()
    }

    /// The floating panel next to the collapsed rail that hosts one section's
    /// content. Painted deferred so it sits above the main content card, which is
    /// a later sibling in the row.
    /// Transparent, occluding scrim covering the content area to the right of the
    /// rail; a mouse-down on it dismisses the popover. Must be added as a direct
    /// child of the (relative) content row — absolute children anchor to their
    /// direct parent — before the panel, so panel clicks never reach it. Starting
    /// at the rail's right edge keeps the rail icons clickable.
    fn collapsed_sidebar_popover_scrim(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        div()
            .id("collapsed_sidebar_popover_scrim")
            .absolute()
            .left(self.sidebar_render_width)
            .top_0()
            .bottom_0()
            .right_0()
            .occlude()
            .on_any_pointer_click(cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                this.close_sidebar_collapsed_popover(cx);
            }))
            .into_any_element()
    }

    /// The floating panel hosting one section's content. Added as a direct child of
    /// the content row (after the scrim), so it anchors to the row and paints above
    /// both the scrim and the main card, while staying below the context-menu layer.
    fn collapsed_sidebar_popover(
        &mut self,
        section: CollapsedSidebarSection,
        theme: AppTheme,
        fade_in: bool,
        anim_seq: u64,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let panel = div()
            .id("collapsed_sidebar_popover")
            .h_full()
            .debug_selector(|| "collapsed_sidebar_popover".to_string())
            .w_full()
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .rounded(px(theme.radii.panel))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .bg(theme.colors.surface.raised)
            .shadow_lg()
            // Claim clicks anywhere on the panel so its empty regions don't fall
            // through to the dismiss scrim underneath.
            .occlude()
            // `occlude` only hides the panes underneath from hitbox-driven
            // handlers; the history and diff canvases install window-level mouse
            // listeners that see every event regardless of what is painted over
            // them. Rows claim their own right-click (they stop propagation), so
            // this catches the gaps — the header, the padding, an empty section —
            // which would otherwise open a commit menu through the popover. It
            // opens the section's own menu instead, which is the only way to
            // reach the worktree/stash/submodule section actions while collapsed.
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let Some((invoker, kind)) = this
                        .active_repo_id()
                        .and_then(|repo_id| section.section_menu(repo_id))
                    else {
                        return;
                    };

                    this.open_popover_at(kind.invoked_by(invoker), e.position, window, cx);
                }),
            )
            .child(self.sidebar_pane.clone())
            // Use the same preferred-size bounds for every collapsed-sidebar
            // popover. Section content chooses the intrinsic height between them.
            .min_h(gpui::relative(1.0 / 3.0))
            .max_h(gpui::relative(1.0))
            .overflow_hidden();

        // Full-height reference box: the panel's relative min/max resolve against
        // its (definite) height, and the panel is anchored to its top. Positioned
        // to match the content card's frame (top flush, bottom margin) so the gaps
        // read symmetric.
        div()
            .absolute()
            .left(self.sidebar_render_width)
            .ml(scaled_px(6.0))
            .top(scaled_px(4.0))
            .bottom(scaled_px(4.0) + scaled_px(CONTENT_CARD_BOTTOM_MARGIN_PX))
            .w(scaled_px(COLLAPSED_POPOVER_WIDTH_PX))
            .flex()
            .flex_col()
            .child(panel)
            // Fade in on open, out on close. `anim_seq` changes each transition so
            // the animation restarts (and plays the opposite direction) each time.
            .with_animation(
                ("collapsed_sidebar_popover_fade", anim_seq),
                gpui::Animation::new(std::time::Duration::from_millis(
                    super::COLLAPSED_POPOVER_FADE_MS,
                ))
                .with_easing(gpui::quadratic),
                move |el, delta| el.opacity(if fade_in { delta } else { 1.0 - delta }),
            )
            .into_any_element()
    }

    pub(super) fn center_content(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        if let Some(pane) = &self.focused_diff_pane {
            return div()
                .flex_1()
                .min_h(px(0.0))
                .child(pane.view())
                .into_any_element();
        }

        let theme = self.theme;
        let ui_scale_percent = self.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

        // With a repository open the viewer takes only the main slot below, so
        // the sidebar, details and action bar stay put. Without one there is no
        // chrome to keep, and the viewer gets the whole content card.
        let repository_chrome = renders_full_chrome(self.view_mode)
            && !self.is_startup_repository_loading_screen_active()
            && !self.is_home_screen_active();
        if self.documents_active && !repository_chrome {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .bg(theme.colors.surface.chrome)
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .mx(scaled_px(6.0))
                        .mt(scaled_px(4.0))
                        .mb(scaled_px(CONTENT_CARD_BOTTOM_MARGIN_PX))
                        .rounded(px(main_content_card_radius(theme)))
                        .border_1()
                        .border_color(theme.colors.stroke.default)
                        .overflow_hidden()
                        .child(self.documents.clone()),
                )
                .child(stable_cached_fixed_height_view(
                    self.bottom_status_bar.clone(),
                    bottom_status_bar_height(cx),
                ))
                .into_any_element();
        }

        if self.is_startup_repository_loading_screen_active() {
            return self.startup_repository_loading_screen();
        }

        if self.is_home_screen_active() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(self.home_screen(cx))
                .child(self.bottom_status_bar.clone())
                .into_any_element();
        }

        if renders_full_chrome(self.view_mode) {
            // Terminal and/or reflog — see `render_bottom_panel` for which.
            let bottom_panel = self.render_bottom_panel(theme, window, cx);
            let bottom_panel_resize_handle = bottom_panel
                .is_some()
                .then(|| self.terminal_panel_resize_handle(theme, cx));
            let main_content = self.repository_main_content(cx);
            let details_tabs = self.details_tab_content(cx);
            let sidebar_sections = (!self.sidebar_collapsed)
                .then(|| self.sidebar_section_content(window, cx))
                .flatten();
            let content = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(self.open_repo_panel(cx))
                .child(stable_cached_fixed_height_view(
                    self.action_bar.clone(),
                    action_bar_height(cx),
                ))
                .child({
                    // While collapsed, the pane renders one section as a floating
                    // popover panel; otherwise it renders the full sidebar in place.
                    // `open` drives the scrim + fade direction; `render` also covers
                    // a section that is currently fading out.
                    let popover_open = self
                        .sidebar_collapsed
                        .then_some(self.sidebar_collapsed_popover)
                        .flatten();
                    let popover_render = self
                        .sidebar_collapsed
                        .then(|| {
                            self.sidebar_collapsed_popover
                                .or(self.sidebar_collapsed_popover_closing)
                        })
                        .flatten();
                    let popover_anim_seq = self.sidebar_collapsed_popover_anim_seq;
                    self.sidebar_pane.update(cx, |pane, cx| {
                        pane.set_collapsed_popover_section(popover_render, cx);
                    });

                    div()
                        // `relative` so the collapsed popover (a later, normal-flow
                        // child below) anchors to the row: it paints above the main
                        // card but below the overlay layer that hosts context menus.
                        .relative()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.0))
                        .bg(theme.colors.surface.chrome)
                        .child(
                            div()
                                .id("sidebar_pane")
                                .debug_selector(|| "sidebar_pane".to_string())
                                .relative()
                                .w(self.sidebar_render_width)
                                .min_h(px(0.0))
                                .bg(theme.colors.surface.chrome)
                                .when(!self.sidebar_collapsed, |d| {
                                    // Cached so frames driven by another view's
                                    // `notify` (a spinner tick in the title bar,
                                    // a diff update) reuse the sidebar's layout
                                    // and paint. The wrapper fills this div, so
                                    // the width animation still re-lays it out.
                                    match sidebar_sections {
                                        None => d.child(stable_cached_fill_view(
                                            self.sidebar_pane.clone(),
                                        )),
                                        // Extension sections sit below the
                                        // sidebar's own, which keeps the rest.
                                        Some(sections) => d
                                            .flex()
                                            .flex_col()
                                            .child(div().flex_1().min_h(px(0.0)).child(
                                                stable_cached_fill_view(self.sidebar_pane.clone()),
                                            ))
                                            .child(sections),
                                    }
                                })
                                .when(self.sidebar_collapsed, |d| {
                                    d.child(self.collapsed_sidebar_rail(theme, cx))
                                }),
                        )
                        .child(
                            // Main + details share one card silhouette; the panes stay
                            // independently resizable inside it. The card sits flush
                            // against the action bar, sidebar, and right window edge;
                            // the sidebar resize strip overlays its left edge below.
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .flex()
                                .flex_row()
                                // Kept minimal so the bottom bar's icons (pane
                                // toggles + zoom) read as one row hugging the card.
                                .mb(scaled_px(CONTENT_CARD_BOTTOM_MARGIN_PX))
                                .relative()
                                .rounded_tl(px(main_content_card_radius(theme)))
                                .rounded_bl(px(main_content_card_radius(theme)))
                                .border_1()
                                .border_color(theme.colors.stroke.default)
                                .overflow_hidden()
                                .bg(theme.colors.surface.canvas)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .min_h(px(0.0))
                                        .overflow_hidden()
                                        .map(|d| {
                                            match (bottom_panel, bottom_panel_resize_handle) {
                                                (Some(bottom_panel), Some(resize_handle)) => d
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_h(px(0.0))
                                                            .child(main_content),
                                                    )
                                                    .child(resize_handle)
                                                    .child(bottom_panel),
                                                _ => d.child(main_content),
                                            }
                                        }),
                                )
                                .child(
                                    div()
                                        .id("details_pane")
                                        .debug_selector(|| "details_pane".to_string())
                                        .relative()
                                        .w(self.details_render_width)
                                        .min_h(px(0.0))
                                        .flex()
                                        .flex_col()
                                        .overflow_hidden()
                                        .when(self.details_collapsed, |d| {
                                            // The resize handle is hidden while collapsed, so
                                            // keep a hairline between main and the strip.
                                            d.border_l_1().border_color(theme.colors.stroke.subtle)
                                        })
                                        .when(!self.details_collapsed, |d| match details_tabs {
                                            None => d.child(div().flex_1().min_h(px(0.0)).child(
                                                stable_cached_fill_view(self.details_pane.clone()),
                                            )),
                                            Some(content) => d.child(content),
                                        }),
                                )
                                .child(
                                    // Keep the full resize hit target without reserving a
                                    // visible strip between the main and details panes.
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .right(
                                            (self.details_render_width
                                                - self.pane_resize_handle_width() / 2.0)
                                                .max(px(0.0)),
                                        )
                                        .child(self.pane_resize_handle(
                                            theme,
                                            "pane_resize_details",
                                            PaneResizeHandle::Details,
                                            cx,
                                        )),
                                )
                                .child(card_left_corner_caps(
                                    px((main_content_card_radius(theme) - 1.0).max(0.0)),
                                    theme.colors.surface.chrome,
                                )),
                        )
                        .child(
                            // Sidebar resize grab strip, straddling the card's left
                            // edge the way the details strip straddles its boundary,
                            // so the grip centers on the rule instead of sitting
                            // beside it. It hangs off the row rather than the card
                            // because the card clips its overflow, and it matches the
                            // card's bottom margin so both strips end on the same
                            // line. Absolute, so the boundary still consumes no
                            // layout space of its own.
                            div()
                                .absolute()
                                .top_0()
                                .bottom(scaled_px(CONTENT_CARD_BOTTOM_MARGIN_PX))
                                .left(
                                    (self.sidebar_render_width
                                        - self.pane_resize_handle_width() / 2.0)
                                        .max(px(0.0)),
                                )
                                .child(self.pane_resize_handle(
                                    theme,
                                    "pane_resize_sidebar",
                                    PaneResizeHandle::Sidebar,
                                    cx,
                                )),
                        )
                        // Scrim only while open (not during fade-out).
                        .when(popover_open.is_some(), |d| {
                            d.child(self.collapsed_sidebar_popover_scrim(cx))
                        })
                        .when_some(popover_render, |d, section| {
                            let fade_in = popover_open.is_some();
                            d.child(self.collapsed_sidebar_popover(
                                section,
                                theme,
                                fade_in,
                                popover_anim_seq,
                                cx,
                            ))
                        })
                })
                .child(stable_cached_fixed_height_view(
                    self.bottom_status_bar.clone(),
                    bottom_status_bar_height(cx),
                ))
                .into_any_element();

            if self.should_show_git_unavailable_overlay() {
                return div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(content)
                    .child(self.git_unavailable_overlay(cx))
                    .into_any_element();
            }

            return content;
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(stable_cached_fill_view(self.main_pane.clone())),
            )
            .into_any_element()
    }
}
