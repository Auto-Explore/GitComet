use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

const SPLASH_BACKDROP_PNG_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/splash_backdrop.png"));
/// Gap (on the 8px grid) around the inset content/details cards so they read as
/// rounded surfaces floating on the shared window canvas, with the sidebar
/// blended into that canvas.
const CONTENT_CARD_GAP_PX: f32 = 8.0;
/// Bottom margin the main content card leaves for the bottom bar. The collapsed
/// section popover matches it so its top/bottom gaps read symmetric.
const CONTENT_CARD_BOTTOM_MARGIN_PX: f32 = 2.0;
static SPLASH_BACKDROP_IMAGE_CACHE: OnceLock<Arc<gpui::Image>> = OnceLock::new();

struct SplashInteractiveColors {
    base: gpui::Rgba,
    hover: gpui::Rgba,
    active: gpui::Rgba,
}

struct SplashCtaButtonColors {
    icon: gpui::Rgba,
    text: gpui::Rgba,
    background: SplashInteractiveColors,
    border: SplashInteractiveColors,
}

pub(in crate::view) fn load_splash_backdrop_image() -> Arc<gpui::Image> {
    SPLASH_BACKDROP_IMAGE_CACHE
        .get_or_init(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                SPLASH_BACKDROP_PNG_BYTES.to_vec(),
            ))
        })
        .clone()
}

/// Children clip to rectangles, so full-bleed content inside the card can
/// square off its rounded corners. These caps repaint the four corner
/// notches (the area between the content rectangle's corner and the card's
/// inner arc) in the surrounding surface color, restoring the rounding over
/// anything the content paints. Canvas elements take no hitboxes, so the
/// overlay is invisible to the mouse.
fn card_corner_caps(radius: Pixels, color: gpui::Rgba) -> AnyElement {
    #[derive(Clone, Copy)]
    enum CapCorner {
        TopLeft,
        TopRight,
        BottomLeft,
        BottomRight,
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
                CapCorner::TopRight => (
                    point(bounds.right(), bounds.top()),
                    point(bounds.left(), bounds.top()),
                    point(bounds.right(), bounds.top() + r),
                    point(bounds.left() + k, bounds.top()),
                    point(bounds.right(), bounds.top() + r - k),
                ),
                CapCorner::BottomRight => (
                    point(bounds.right(), bounds.bottom()),
                    point(bounds.right(), bounds.top()),
                    point(bounds.left(), bounds.bottom()),
                    point(bounds.right(), bounds.top() + k),
                    point(bounds.left() + k, bounds.bottom()),
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
            CapCorner::TopRight => positioned.top_0().right_0(),
            CapCorner::BottomLeft => positioned.bottom_0().left_0(),
            CapCorner::BottomRight => positioned.bottom_0().right_0(),
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
        .child(cap(CapCorner::TopRight))
        .child(cap(CapCorner::BottomLeft))
        .child(cap(CapCorner::BottomRight))
        .into_any_element()
}

impl GitCometView {
    fn splash_backdrop_base() -> gpui::Background {
        gpui::linear_gradient(
            180.0,
            gpui::linear_color_stop(gpui::rgba(0x060a13ff), 0.0),
            gpui::linear_color_stop(gpui::rgba(0x02050fff), 1.0),
        )
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
                    .object_fit(gpui::ObjectFit::Fill),
            )
            .into_any_element()
    }

    fn has_repo_tabs(&self) -> bool {
        !self.state.repos.is_empty()
    }

    fn git_runtime_unavailable(&self) -> bool {
        !self.state.git_runtime.is_available()
    }

    fn git_runtime_unavailable_detail(&self) -> String {
        self.state
            .git_runtime
            .unavailable_detail()
            .unwrap_or("GitComet could not find a usable Git executable.")
            .to_string()
    }

    fn git_unavailable_status_icon(theme: AppTheme, ui_scale_percent: u32) -> AnyElement {
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        div()
            .id("git_unavailable_status_icon")
            .debug_selector(|| "git_unavailable_status_icon".to_string())
            .size(scaled_px(56.0))
            .flex()
            .items_center()
            .justify_center()
            .child(svg_icon(
                "icons/warning.svg",
                theme.colors.warning,
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

    pub(crate) fn blocks_non_repository_actions(&self) -> bool {
        repository_entry_interstitial_active(self.view_mode, self.has_repo_tabs())
            || matches!(self.view_mode, GitCometViewMode::Normal) && self.git_runtime_unavailable()
    }

    pub(crate) fn is_splash_screen_active(&self) -> bool {
        should_show_splash_screen(
            self.view_mode,
            self.has_repo_tabs(),
            self.startup_repo_bootstrap_pending,
        )
    }

    fn is_startup_repository_loading_screen_active(&self) -> bool {
        should_show_startup_repository_loading_screen(
            self.view_mode,
            self.has_repo_tabs(),
            self.startup_repo_bootstrap_pending,
        )
    }

    pub(super) fn sync_title_bar_workspace_actions(&mut self, cx: &mut gpui::Context<Self>) {
        let enabled = titlebar_workspace_actions_enabled(self.view_mode, self.has_repo_tabs());
        self.title_bar
            .update(cx, |bar, cx| bar.set_workspace_actions_enabled(enabled, cx));
    }

    fn interstitial_logo(_theme: AppTheme, size: Pixels) -> AnyElement {
        div()
            .id("repository_entry_logo")
            .size(size)
            .child(gpui::svg().path("gitcomet_logo.svg").w(size).h(size))
            .into_any_element()
    }

    fn interstitial_backdrop(&self) -> AnyElement {
        div()
            .id("splash_backdrop_native")
            .debug_selector(|| "splash_backdrop_native".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .bg(Self::splash_backdrop_base())
            .child(self.splash_backdrop_image_layer())
            .into_any_element()
    }

    fn splash_cta_button(
        id: &'static str,
        label: &'static str,
        icon_path: &'static str,
        colors: SplashCtaButtonColors,
        ui_scale_percent: u32,
    ) -> gpui::Stateful<gpui::Div> {
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let focus_ring = gpui::rgba(0x79d0ffeb);
        let SplashCtaButtonColors {
            icon: icon_color,
            text: text_color,
            background,
            border: border_colors,
        } = colors;
        let SplashInteractiveColors {
            base: bg,
            hover: hover_bg,
            active: active_bg,
        } = background;
        let SplashInteractiveColors {
            base: border,
            hover: hover_border,
            active: active_border,
        } = border_colors;

        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .tab_index(0)
            .h(scaled_px(36.0))
            .px(scaled_px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(scaled_px(6.0))
            .rounded(scaled_px(2.0))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_size(scaled_px(13.0))
            .font_weight(FontWeight::BOLD)
            .text_color(text_color)
            .cursor(CursorStyle::PointingHand)
            .whitespace_nowrap()
            .child(svg_icon(icon_path, icon_color, scaled_px(14.0)))
            .child(label)
            .focus(move |s| s.border_color(focus_ring))
            .hover(move |s| s.bg(hover_bg).border_color(hover_border))
            .active(move |s| s.bg(active_bg).border_color(active_border))
    }

    fn interstitial_shell(
        &self,
        id: &'static str,
        content: impl IntoElement,
        theme: AppTheme,
    ) -> AnyElement {
        let border_glow = with_alpha(theme.colors.border, if theme.is_dark { 0.86 } else { 0.74 });

        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .relative()
            .flex()
            .flex_1()
            .min_h(px(0.0))
            .items_center()
            .justify_center()
            .overflow_hidden()
            .px_3()
            .py_4()
            .bg(gpui::rgba(0x02050fff))
            .child(self.interstitial_backdrop())
            .child(
                div()
                    .relative()
                    .w_full()
                    .max_w(px(560.0))
                    .bg(with_alpha(
                        theme.colors.surface_bg,
                        if theme.is_dark { 0.96 } else { 0.98 },
                    ))
                    .border_1()
                    .border_color(border_glow)
                    .rounded(px(theme.radii.panel))
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::rgba(0x00000052).into(),
                        offset: point(px(0.0), px(22.0)),
                        blur_radius: px(52.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .p_4()
                    .child(content),
            )
            .into_any_element()
    }

    fn git_unavailable_open_settings_button(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let primary_bg = gpui::rgba(0x5ac1feff);
        let primary_hover = gpui::rgba(0x72c7ffff);
        let primary_active = gpui::rgba(0x48b6eeff);
        let primary_text = gpui::rgba(0x04172bff);
        let settings_tooltip: SharedString = "Open settings".into();

        Self::splash_cta_button(
            "git_unavailable_open_settings",
            "Open Settings",
            "icons/cog.svg",
            SplashCtaButtonColors {
                icon: primary_text,
                text: primary_text,
                background: SplashInteractiveColors {
                    base: primary_bg,
                    hover: primary_hover,
                    active: primary_active,
                },
                border: SplashInteractiveColors {
                    base: primary_bg,
                    hover: primary_hover,
                    active: primary_active,
                },
            },
            self.ui_scale_percent,
        )
        .gitcomet_tooltip(self.theme, settings_tooltip)
        .on_click(cx.listener(|this, _e, _window, cx| {
            this.open_repo_panel = false;
            cx.defer(crate::view::open_settings_window);
            cx.notify();
        }))
    }

    fn git_unavailable_panel_content(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let detail_bg = with_alpha(
            theme.colors.window_bg,
            if theme.is_dark { 0.36 } else { 0.82 },
        );
        let detail_border =
            with_alpha(theme.colors.border, if theme.is_dark { 0.96 } else { 0.82 });

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
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .text_center()
                    .child("Git executable unavailable"),
            )
            .child(
                div()
                    .max_w(px(440.0))
                    .text_center()
                    .text_sm()
                    .line_height(px(22.0))
                    .text_color(theme.colors.text_muted)
                    .child(
                        "GitComet cannot open, refresh, or run repository actions until a Git executable is configured.",
                    ),
            )
            .child(
                div()
                    .id("git_unavailable_detail")
                    .w_full()
                    .max_w(px(460.0))
                    .rounded(px(theme.radii.panel))
                    .border_1()
                    .border_color(detail_border)
                    .bg(detail_bg)
                    .px_3()
                    .py_2()
                    .text_xs()
                    .line_height(px(18.0))
                    .text_color(theme.colors.text_muted)
                    .child(self.git_runtime_unavailable_detail_content()),
            )
            .child(
                div()
                    .pt_1()
                    .child(self.git_unavailable_open_settings_button(cx)),
            )
            .into_any_element()
    }

    fn git_unavailable_splash(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        self.interstitial_shell(
            "git_unavailable_screen",
            self.git_unavailable_panel_content(theme, cx),
            theme,
        )
    }

    fn git_unavailable_overlay(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let border_glow = with_alpha(theme.colors.border, if theme.is_dark { 0.86 } else { 0.74 });

        div()
            .id("git_unavailable_overlay")
            .debug_selector(|| "git_unavailable_overlay".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .bg(with_alpha(
                theme.colors.window_bg,
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
                            .max_w(px(560.0))
                            .bg(with_alpha(
                                theme.colors.surface_bg,
                                if theme.is_dark { 0.96 } else { 0.98 },
                            ))
                            .border_1()
                            .border_color(border_glow)
                            .rounded(px(theme.radii.panel))
                            .shadow(vec![gpui::BoxShadow {
                                color: gpui::rgba(0x00000052).into(),
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
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);

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
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("Loading repository session"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child("GitComet is opening your workspace."),
                )
                .child(
                    div()
                        .pt_1()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child(svg_spinner(
                            ("repository_loading_spinner", 0u64),
                            theme.colors.accent,
                            scaled_px(16.0),
                        ))
                        .child("Please wait…"),
                ),
            theme,
        )
    }

    pub(super) fn splash_screen(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        if self.git_runtime_unavailable() {
            return self.git_unavailable_splash(cx);
        }

        let hero_text = gpui::rgba(0xf6f7fbff);
        let hero_muted = gpui::rgba(0xa8b1c6ff);
        let hero_proof = gpui::rgba(0xffffffbd);
        let panel_border = gpui::rgba(0xffffff1f);
        let guide_edge = gpui::rgba(0xffffff22);
        let guide_fade = gpui::rgba(0xffffff0a);
        let node_color = gpui::rgba(0xffffffff);
        let primary_bg = gpui::rgba(0x5ac1feff);
        let primary_hover = gpui::rgba(0x72c7ffff);
        let primary_active = gpui::rgba(0x48b6eeff);
        let primary_text = gpui::rgba(0x04172bff);
        let primary_button_colors = SplashCtaButtonColors {
            icon: primary_text,
            text: primary_text,
            background: SplashInteractiveColors {
                base: primary_bg,
                hover: primary_hover,
                active: primary_active,
            },
            border: SplashInteractiveColors {
                base: primary_bg,
                hover: primary_hover,
                active: primary_active,
            },
        };
        let secondary_bg = gpui::rgba(0xffffff26);
        let secondary_hover = gpui::rgba(0xffffff33);
        let secondary_active = gpui::rgba(0xffffff40);
        let secondary_border = gpui::rgba(0xffffff47);
        let secondary_hover_border = gpui::rgba(0xffffff66);
        let secondary_active_border = gpui::rgba(0xffffff80);
        let secondary_button_colors = SplashCtaButtonColors {
            icon: hero_text,
            text: hero_text,
            background: SplashInteractiveColors {
                base: secondary_bg,
                hover: secondary_hover,
                active: secondary_active,
            },
            border: SplashInteractiveColors {
                base: secondary_border,
                hover: secondary_hover_border,
                active: secondary_active_border,
            },
        };
        let panel_shadow = gpui::rgba(0x00000059);
        let open_tooltip: SharedString = "Open repository".into();
        let clone_tooltip: SharedString = "Clone repository".into();

        let open_button = Self::splash_cta_button(
            "splash_open_repo",
            "Open Repository",
            "icons/folder.svg",
            primary_button_colors,
            self.ui_scale_percent,
        )
        .gitcomet_tooltip(self.theme, open_tooltip)
        .on_click(cx.listener(|this, _e, window, cx| {
            this.prompt_open_repo(window, cx);
        }));

        let clone_button = {
            let last_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
            let last_bounds_for_prepaint = Rc::clone(&last_bounds);
            let last_bounds_for_click = Rc::clone(&last_bounds);

            let button = Self::splash_cta_button(
                "splash_clone_repo",
                "Clone Repository",
                "icons/cloud.svg",
                secondary_button_colors,
                self.ui_scale_percent,
            )
            .gitcomet_tooltip(self.theme, clone_tooltip)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                let bounds = (*last_bounds_for_click.borrow())
                    .unwrap_or_else(|| Bounds::new(e.position(), size(px(0.0), px(0.0))));
                this.open_popover_for_bounds(PopoverKind::CloneRepo, bounds, window, cx);
            }));

            div()
                .on_children_prepainted(move |children_bounds, _window, _cx| {
                    if let Some(bounds) = children_bounds.first() {
                        *last_bounds_for_prepaint.borrow_mut() = Some(*bounds);
                    }
                })
                .child(button)
        };

        let open_repo_fallback = if self.open_repo_panel {
            div()
                .w_full()
                .pt(px(12.0))
                .child(
                    div()
                        .pb(px(8.0))
                        .text_size(px(11.0))
                        .text_color(hero_muted)
                        .text_center()
                        .child(
                            "Native folder picker unavailable. Enter a repository path manually.",
                        ),
                )
                .child(self.open_repo_panel(cx))
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let headline_line = |text: &'static str| {
            div()
                .text_center()
                .font_family("Noto Serif")
                .font_weight(FontWeight::BOLD)
                .text_size(px(50.0))
                .line_height(px(44.0))
                .text_color(hero_text)
                .whitespace_nowrap()
                .child(text)
        };

        div()
            .id("repository_entry_screen")
            .debug_selector(|| "repository_entry_screen".to_string())
            .relative()
            .flex()
            .flex_1()
            .min_h(px(0.0))
            .items_center()
            .justify_start()
            .overflow_hidden()
            .bg(gpui::rgba(0x02050fff))
            .px_4()
            .pt(px(52.0))
            .pb(px(24.0))
            .child(self.interstitial_backdrop())
            .child(
                div().relative().w_full().flex().justify_center().child(
                    div()
                        .relative()
                        .w_full()
                        .max_w(px(700.0))
                        .px(px(48.0))
                        .py(px(36.0))
                        .border_1()
                        .border_color(panel_border)
                        .bg(gpui::linear_gradient(
                            180.0,
                            gpui::linear_color_stop(gpui::rgba(0x0f15258f), 0.0),
                            gpui::linear_color_stop(gpui::rgba(0x03081352), 1.0),
                        ))
                        .shadow(vec![gpui::BoxShadow {
                            color: panel_shadow.into(),
                            offset: point(px(0.0), px(40.0)),
                            blur_radius: px(80.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(12.0))
                                .child(
                                    div()
                                        .id("splash_headline")
                                        .debug_selector(|| "splash_headline".to_string())
                                        .max_w(px(560.0))
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .child(headline_line("Fastest Open"))
                                        .child(headline_line("Source Git GUI")),
                                )
                                .child(
                                    div()
                                        .max_w(px(500.0))
                                        .pt(px(2.0))
                                        .text_center()
                                        .text_size(px(14.0))
                                        .line_height(px(22.0))
                                        .text_color(hero_muted)
                                        .child(
                                            "GitComet is built for teams that want fast Git operations with local-first privacy, familiar workflows, and open source freedom.",
                                        ),
                                )
                                .child(
                                    div()
                                        .pt(px(4.0))
                                        .flex()
                                        .flex_wrap()
                                        .justify_center()
                                        .gap(px(10.0))
                                        .child(
                                            div()
                                                .id("splash_open_repo_action")
                                                .debug_selector(|| {
                                                    "splash_open_repo_action".to_string()
                                                })
                                                .flex()
                                                .justify_center()
                                                .child(open_button),
                                        )
                                        .child(
                                            div()
                                                .id("splash_clone_repo_action")
                                                .debug_selector(|| {
                                                    "splash_clone_repo_action".to_string()
                                                })
                                                .flex()
                                                .justify_center()
                                                .child(clone_button),
                                        ),
                                )
                                .child(open_repo_fallback)
                                .child(
                                    div()
                                        .pt(px(2.0))
                                        .text_size(px(12.0))
                                        .text_color(hero_proof)
                                        .text_center()
                                        .child("Available for Linux, Windows and macOS."),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(-224.0))
                                .left(px(-1.0))
                                .w(px(1.0))
                                .h(px(224.0))
                                .bg(gpui::linear_gradient(
                                    180.0,
                                    gpui::linear_color_stop(guide_fade, 0.0),
                                    gpui::linear_color_stop(guide_edge, 1.0),
                                )),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(-224.0))
                                .right(px(-1.0))
                                .w(px(1.0))
                                .h(px(224.0))
                                .bg(gpui::linear_gradient(
                                    180.0,
                                    gpui::linear_color_stop(guide_fade, 0.0),
                                    gpui::linear_color_stop(guide_edge, 1.0),
                                )),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(-214.0))
                                .left(px(-1.0))
                                .w(px(1.0))
                                .h(px(214.0))
                                .bg(gpui::linear_gradient(
                                    180.0,
                                    gpui::linear_color_stop(guide_edge, 0.0),
                                    gpui::linear_color_stop(guide_fade, 1.0),
                                )),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(-214.0))
                                .right(px(-1.0))
                                .w(px(1.0))
                                .h(px(214.0))
                                .bg(gpui::linear_gradient(
                                    180.0,
                                    gpui::linear_color_stop(guide_edge, 0.0),
                                    gpui::linear_color_stop(guide_fade, 1.0),
                                )),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(-4.0))
                                .left(px(-4.0))
                                .size(px(7.0))
                                .bg(node_color),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(-4.0))
                                .right(px(-4.0))
                                .size(px(7.0))
                                .bg(node_color),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(-4.0))
                                .left(px(-4.0))
                                .size(px(7.0))
                                .bg(node_color),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom(px(-4.0))
                                .right(px(-4.0))
                                .size(px(7.0))
                                .bg(node_color),
                        ),
                ),
            )
            .into_any_element()
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
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let active = self.sidebar_collapsed_popover;
        let icon_muted = theme.colors.text_muted;
        let active_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.06 });
        let hover_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.06 } else { 0.04 });
        let slot = scaled_px(28.0);

        let icons = CollapsedSidebarSection::ALL.into_iter().map(|section| {
            let is_active = active == Some(section);
            let icon_color = if is_active {
                theme.colors.text
            } else {
                icon_muted
            };
            div()
                .id(section.element_id())
                .flex()
                .items_center()
                .justify_center()
                .size(slot)
                .rounded(px(theme.radii.control))
                .cursor(CursorStyle::PointingHand)
                .when(is_active, |d| d.bg(active_bg))
                .hover(move |d| if is_active { d } else { d.bg(hover_bg) })
                .child(svg_icon(section.icon_path(), icon_color, scaled_px(16.0)))
                .on_mouse_down(
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
            .on_any_mouse_down(cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                this.close_sidebar_collapsed_popover(cx);
            }))
            .into_any_element()
    }

    /// The floating panel hosting one section's content. Added as a direct child of
    /// the content row (after the scrim), so it anchors to the row and paints above
    /// both the scrim and the main card, while staying below the context-menu layer.
    fn collapsed_sidebar_popover(
        &mut self,
        _section: CollapsedSidebarSection,
        theme: AppTheme,
        fade_in: bool,
        anim_seq: u64,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let panel = div()
            .id("collapsed_sidebar_popover")
            .debug_selector(|| "collapsed_sidebar_popover".to_string())
            .w_full()
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .rounded(px(theme.radii.panel))
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_bg_elevated)
            .shadow_lg()
            // Claim clicks anywhere on the panel so its empty regions don't fall
            // through to the dismiss scrim underneath.
            .occlude()
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
            .bottom(scaled_px(4.0) + px(CONTENT_CARD_BOTTOM_MARGIN_PX))
            .w(scaled_px(268.0))
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
        let theme = self.theme;

        if self.is_startup_repository_loading_screen_active() {
            return self.startup_repository_loading_screen();
        }

        if self.is_splash_screen_active() {
            return self.splash_screen(cx);
        }

        if renders_full_chrome(self.view_mode) {
            let terminal_panel = self.render_terminal_panel(theme, window, cx);
            let has_terminal_panel = terminal_panel.is_some();
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
                        .bg(theme.colors.sidebar_bg)
                        .child(
                            div()
                                .id("sidebar_pane")
                                .relative()
                                .w(self.sidebar_render_width)
                                .min_h(px(0.0))
                                .bg(theme.colors.sidebar_bg)
                                .when(!self.sidebar_collapsed, |d| {
                                    d.child(self.sidebar_pane.clone())
                                })
                                .when(self.sidebar_collapsed, |d| {
                                    d.child(self.collapsed_sidebar_rail(theme, cx))
                                }),
                        )
                        .child(
                            // Main + details share one card silhouette; the panes stay
                            // independently resizable inside it. The card sits flush
                            // against the action bar and sidebar (no top/left gap); the
                            // sidebar resize strip overlays the card's left edge below.
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .flex()
                                .flex_row()
                                // Kept minimal so the bottom bar's icons (pane
                                // toggles + zoom) read as one row hugging the card.
                                .mb(px(CONTENT_CARD_BOTTOM_MARGIN_PX))
                                .mr(px(CONTENT_CARD_GAP_PX))
                                .relative()
                                .rounded(px(theme.radii.panel))
                                .border_1()
                                .border_color(theme.colors.border)
                                .overflow_hidden()
                                .bg(theme.colors.window_bg)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .min_h(px(0.0))
                                        .overflow_hidden()
                                        .when_some(terminal_panel, |d, terminal_panel| {
                                            d.flex()
                                                .flex_col()
                                                .child(div().flex_1().min_h(px(0.0)).child(
                                                    stable_cached_fill_view(self.main_pane.clone()),
                                                ))
                                                .child(self.terminal_panel_resize_handle(theme, cx))
                                                .child(terminal_panel)
                                        })
                                        .when(!has_terminal_panel, |d| {
                                            d.child(stable_cached_fill_view(self.main_pane.clone()))
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
                                            d.border_l_1().border_color(theme.colors.border_variant)
                                        })
                                        .when(!self.details_collapsed, |d| {
                                            d.child(
                                                div()
                                                    .flex_1()
                                                    .min_h(px(0.0))
                                                    .child(self.details_pane.clone()),
                                            )
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
                                .child(card_corner_caps(
                                    px((theme.radii.panel - 1.0).max(0.0)),
                                    theme.colors.sidebar_bg,
                                ))
                                .child(
                                    // Sidebar resize grab strip overlaying the card's
                                    // left edge, so the boundary consumes no layout
                                    // space of its own.
                                    div().absolute().left_0().top_0().bottom_0().child(
                                        self.pane_resize_handle(
                                            theme,
                                            "pane_resize_sidebar",
                                            PaneResizeHandle::Sidebar,
                                            cx,
                                        ),
                                    ),
                                ),
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
                .child(
                    // Keep the bottom bar uncached. It paints after the details pane,
                    // so reusing its cached paint range can replay a stale input-handler
                    // index while a focused TextInput is temporarily detached during a
                    // Wayland text-input redraw.
                    self.bottom_status_bar.clone(),
                )
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
