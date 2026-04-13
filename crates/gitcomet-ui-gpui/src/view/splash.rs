use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

const SPLASH_BACKDROP_PNG_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/splash_backdrop.png"));
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

    fn git_unavailable_status_icon(theme: AppTheme) -> AnyElement {
        div()
            .id("git_unavailable_status_icon")
            .debug_selector(|| "git_unavailable_status_icon".to_string())
            .size(px(56.0))
            .flex()
            .items_center()
            .justify_center()
            .child(svg_icon(
                "icons/warning.svg",
                theme.colors.warning,
                px(36.0),
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

    fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
    }

    fn clear_tooltip_if_matches(&mut self, tooltip: &SharedString, cx: &mut gpui::Context<Self>) {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
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
    ) -> gpui::Stateful<gpui::Div> {
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
            .h(px(36.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .rounded(px(2.0))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_size(px(13.0))
            .font_weight(FontWeight::BOLD)
            .text_color(text_color)
            .cursor(CursorStyle::PointingHand)
            .whitespace_nowrap()
            .child(svg_icon(icon_path, icon_color, px(14.0)))
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
        )
        .on_click(cx.listener(|this, _e, _window, cx| {
            this.open_repo_panel = false;
            cx.defer(crate::view::open_settings_window);
            cx.notify();
        }))
        .on_hover(cx.listener({
            let settings_tooltip = settings_tooltip.clone();
            move |this, hovering: &bool, _w, cx| {
                if *hovering {
                    this.set_tooltip_text_if_changed(Some(settings_tooltip.clone()), cx);
                } else {
                    this.clear_tooltip_if_matches(&settings_tooltip, cx);
                }
            }
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
            .child(Self::git_unavailable_status_icon(theme))
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
                            }])
                            .p_4()
                            .child(self.git_unavailable_panel_content(theme, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn startup_repository_loading_screen(&mut self) -> AnyElement {
        let theme = self.theme;

        self.interstitial_shell(
            "repository_loading_screen",
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(Self::interstitial_logo(theme, px(84.0)))
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
                            px(16.0),
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
        )
        .on_click(cx.listener(|this, _e, window, cx| {
            this.prompt_open_repo(window, cx);
        }))
        .on_hover(cx.listener({
            let open_tooltip = open_tooltip.clone();
            move |this, hovering: &bool, _w, cx| {
                if *hovering {
                    this.set_tooltip_text_if_changed(Some(open_tooltip.clone()), cx);
                } else {
                    this.clear_tooltip_if_matches(&open_tooltip, cx);
                }
            }
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
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                let bounds = (*last_bounds_for_click.borrow())
                    .unwrap_or_else(|| Bounds::new(e.position(), size(px(0.0), px(0.0))));
                this.open_popover_for_bounds(PopoverKind::CloneRepo, bounds, window, cx);
            }))
            .on_hover(cx.listener({
                let clone_tooltip = clone_tooltip.clone();
                move |this, hovering: &bool, _w, cx| {
                    if *hovering {
                        this.set_tooltip_text_if_changed(Some(clone_tooltip.clone()), cx);
                    } else {
                        this.clear_tooltip_if_matches(&clone_tooltip, cx);
                    }
                }
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

    pub(super) fn center_content(
        &mut self,
        _window: &mut Window,
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
            let content = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(stable_cached_fixed_height_view(
                    self.repo_tabs_bar.clone(),
                    components::Tab::container_height(),
                ))
                .child(self.open_repo_panel(cx))
                .child(stable_cached_fixed_height_view(
                    self.action_bar.clone(),
                    ACTION_BAR_HEIGHT,
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .id("sidebar_pane")
                                .relative()
                                .w(self.sidebar_render_width)
                                .min_h(px(0.0))
                                .bg(theme.colors.surface_bg)
                                .when(self.sidebar_collapsed, |d| {
                                    d.border_r_1().border_color(theme.colors.border)
                                })
                                .when(!self.sidebar_collapsed, |d| {
                                    d.child(self.sidebar_pane.clone())
                                })
                                .child(
                                    div().absolute().bottom(px(6.0)).right(px(6.0)).child(
                                        components::Button::new("sidebar_toggle", "")
                                            .start_slot(svg_icon(
                                                if self.sidebar_collapsed {
                                                    "icons/arrow_right.svg"
                                                } else {
                                                    "icons/arrow_left.svg"
                                                },
                                                theme.colors.text_muted,
                                                px(12.0),
                                            ))
                                            .style(components::ButtonStyle::Transparent)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.set_sidebar_collapsed(
                                                    !this.sidebar_collapsed,
                                                    cx,
                                                );
                                            }),
                                    ),
                                ),
                        )
                        .child(self.pane_resize_handle(
                            theme,
                            "pane_resize_sidebar",
                            PaneResizeHandle::Sidebar,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .child(stable_cached_fill_view(self.main_pane.clone())),
                        )
                        .child(self.pane_resize_handle(
                            theme,
                            "pane_resize_details",
                            PaneResizeHandle::Details,
                            cx,
                        ))
                        .child(
                            div()
                                .id("details_pane")
                                .relative()
                                .w(self.details_render_width)
                                .min_h(px(0.0))
                                .flex()
                                .flex_col()
                                .when(self.details_collapsed, |d| {
                                    d.border_l_1().border_color(theme.colors.border)
                                })
                                .when(!self.details_collapsed, |d| {
                                    d.child(
                                        div()
                                            .flex_1()
                                            .min_h(px(0.0))
                                            .child(self.details_pane.clone()),
                                    )
                                })
                                .child(
                                    div().absolute().bottom(px(6.0)).left(px(6.0)).child(
                                        components::Button::new("details_toggle", "")
                                            .start_slot(svg_icon(
                                                if self.details_collapsed {
                                                    "icons/arrow_left.svg"
                                                } else {
                                                    "icons/arrow_right.svg"
                                                },
                                                theme.colors.text_muted,
                                                px(12.0),
                                            ))
                                            .style(components::ButtonStyle::Transparent)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.set_details_collapsed(
                                                    !this.details_collapsed,
                                                    cx,
                                                );
                                            }),
                                    ),
                                ),
                        ),
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
