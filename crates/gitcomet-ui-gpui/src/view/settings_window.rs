use super::*;
use gitcomet_core::process::configure_background_command;
use gpui::{Stateful, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions};

const SETTINGS_WINDOW_MIN_WIDTH_PX: f32 = 620.0;
const SETTINGS_WINDOW_MIN_HEIGHT_PX: f32 = 460.0;
const SETTINGS_WINDOW_DEFAULT_WIDTH_PX: f32 = 720.0;
const SETTINGS_WINDOW_DEFAULT_HEIGHT_PX: f32 = 620.0;
const SETTINGS_WINDOW_TITLE: &str = "Settings: GitComet";
const SETTINGS_TRAFFIC_LIGHTS_SAFE_INSET: Pixels = px(78.0);
const MIN_GIT_MAJOR: u32 = 2;
const MIN_GIT_MINOR: u32 = 50;
const GITHUB_URL: &str = "https://github.com/Auto-Explore/GitComet";
const LICENSE_URL: &str = "https://github.com/Auto-Explore/GitComet/blob/main/LICENSE-AGPL-3.0";
const LICENSE_NAME: &str = "AGPL-3.0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsSection {
    Theme,
    DateFormat,
    Timezone,
    ChangeTracking,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsView {
    Root,
    OpenSourceLicenses,
}

#[derive(Clone, Debug)]
struct SettingsRuntimeInfo {
    git: GitRuntimeInfo,
    app_version_display: SharedString,
    operating_system: SharedString,
}

#[derive(Clone, Debug)]
struct GitRuntimeInfo {
    version_display: SharedString,
    compatibility: GitCompatibility,
    detail: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GitCompatibility {
    Supported,
    TooOld,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GitVersion {
    major: u32,
    minor: u32,
}

pub(crate) struct SettingsWindowView {
    theme_mode: ThemeMode,
    theme: AppTheme,
    date_time_format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    change_tracking_view: ChangeTrackingView,
    current_view: SettingsView,
    open_source_licenses_scroll: UniformListScrollHandle,
    runtime_info: SettingsRuntimeInfo,
    expanded_section: Option<SettingsSection>,
    hover_resize_edge: Option<ResizeEdge>,
    title_drag_state: chrome::TitleBarDragState,
    _appearance_subscription: gpui::Subscription,
}

pub(crate) fn open_settings_window(cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<SettingsWindowView>())
    {
        let _ = window.update(cx, |_view, window, _cx| {
            window.activate_window();
        });
        return;
    }

    let bounds = Bounds::centered(
        None,
        size(
            px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX),
            px(SETTINGS_WINDOW_DEFAULT_HEIGHT_PX),
        ),
        cx,
    );
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(
                px(SETTINGS_WINDOW_MIN_WIDTH_PX),
                px(SETTINGS_WINDOW_MIN_HEIGHT_PX),
            )),
            titlebar: Some(settings_window_titlebar_options()),
            app_id: Some("gitcomet-settings".into()),
            window_decorations: Some(WindowDecorations::Client),
            is_movable: true,
            is_resizable: true,
            ..Default::default()
        },
        |window, cx| cx.new(|cx| SettingsWindowView::new(window, cx)),
    )
    .expect("failed to open settings window");

    cx.activate(true);
}

fn settings_window_titlebar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(SETTINGS_WINDOW_TITLE.into()),
        // Windows needs a transparent native titlebar to avoid rendering its own
        // caption on top of the custom settings header.
        appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
        traffic_light_position: cfg!(target_os = "macos").then_some(point(px(9.0), px(9.0))),
    }
}

fn settings_window_client_inset() -> Pixels {
    if cfg!(target_os = "windows") {
        px(0.0)
    } else {
        chrome::CLIENT_SIDE_DECORATION_INSET
    }
}

fn settings_window_frame(
    theme: AppTheme,
    decorations: Decorations,
    content: AnyElement,
) -> AnyElement {
    if cfg!(target_os = "windows") {
        content
    } else {
        window_frame(theme, decorations, content)
    }
}

impl SettingsWindowView {
    fn new(window: &mut Window, cx: &mut gpui::Context<Self>) -> Self {
        window.set_window_title(SETTINGS_WINDOW_TITLE);

        let ui_session = session::load();
        let theme_mode = ui_session
            .theme_mode
            .as_deref()
            .and_then(ThemeMode::from_key)
            .unwrap_or_default();
        let date_time_format = ui_session
            .date_time_format
            .as_deref()
            .and_then(DateTimeFormat::from_key)
            .unwrap_or(DateTimeFormat::YmdHm);
        let timezone = ui_session
            .timezone
            .as_deref()
            .and_then(Timezone::from_key)
            .unwrap_or_default();
        let show_timezone = ui_session.show_timezone.unwrap_or(true);
        let change_tracking_view = ui_session
            .change_tracking_view
            .as_deref()
            .and_then(ChangeTrackingView::from_key)
            .unwrap_or_default();
        let theme = theme_mode.resolve_theme(window.appearance());

        let appearance_subscription = {
            let view = cx.weak_entity();
            let mut first = true;
            window.observe_window_appearance(move |window, app| {
                if first {
                    first = false;
                    return;
                }

                let _ = view.update(app, |this, cx| {
                    if this.theme_mode != ThemeMode::Automatic {
                        return;
                    }
                    this.theme = this.theme_mode.resolve_theme(window.appearance());
                    cx.notify();
                });
            })
        };

        Self {
            theme_mode,
            theme,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            current_view: SettingsView::Root,
            open_source_licenses_scroll: UniformListScrollHandle::default(),
            runtime_info: SettingsRuntimeInfo::detect(),
            expanded_section: None,
            hover_resize_edge: None,
            title_drag_state: chrome::TitleBarDragState::default(),
            _appearance_subscription: appearance_subscription,
        }
    }

    fn toggle_section(&mut self, section: SettingsSection, cx: &mut gpui::Context<Self>) {
        self.expanded_section = if self.expanded_section == Some(section) {
            None
        } else {
            Some(section)
        };
        cx.notify();
    }

    fn persist_preferences(&self, cx: &mut gpui::Context<Self>) {
        let settings = session::UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: Some(self.theme_mode.key().to_string()),
            date_time_format: Some(self.date_time_format.key().to_string()),
            timezone: Some(self.timezone.key()),
            show_timezone: Some(self.show_timezone),
            change_tracking_view: Some(self.change_tracking_view.key().to_string()),
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
        };

        cx.background_spawn(async move {
            let _ = session::persist_ui_settings(settings);
        })
        .detach();
    }

    fn show_root(&mut self, cx: &mut gpui::Context<Self>) {
        if self.current_view == SettingsView::Root {
            return;
        }

        self.current_view = SettingsView::Root;
        cx.notify();
    }

    fn show_open_source_licenses(&mut self, cx: &mut gpui::Context<Self>) {
        if self.current_view == SettingsView::OpenSourceLicenses {
            return;
        }

        self.current_view = SettingsView::OpenSourceLicenses;
        self.expanded_section = None;
        cx.notify();
    }

    fn update_main_windows(
        &self,
        cx: &mut gpui::Context<Self>,
        f: impl FnMut(&mut GitCometView, &mut Window, &mut gpui::Context<GitCometView>) + 'static,
    ) {
        let handles: Vec<_> = cx
            .windows()
            .into_iter()
            .filter_map(|window| window.downcast::<GitCometView>())
            .collect();
        cx.spawn(
            async move |_view: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let _ = cx.update(move |cx| {
                    let mut f = f;
                    for handle in handles {
                        let _ = handle.update(cx, |view, window, cx| f(view, window, cx));
                    }
                });
            },
        )
        .detach();
    }

    fn set_theme_mode(
        &mut self,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == mode {
            return;
        }

        self.theme_mode = mode;
        self.theme = mode.resolve_theme(window.appearance());
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, root_window, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.set_theme_mode(mode, root_window.appearance(), cx);
            });
        });
        cx.notify();
    }

    fn set_date_time_format(&mut self, format: DateTimeFormat, cx: &mut gpui::Context<Self>) {
        if self.date_time_format == format {
            return;
        }

        self.date_time_format = format;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.set_date_time_format(format, cx);
            });
        });
        cx.notify();
    }

    fn set_timezone(&mut self, timezone: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == timezone {
            return;
        }

        self.timezone = timezone;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.set_timezone(timezone, cx);
            });
        });
        cx.notify();
    }

    fn set_show_timezone(&mut self, enabled: bool, cx: &mut gpui::Context<Self>) {
        if self.show_timezone == enabled {
            return;
        }

        self.show_timezone = enabled;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.set_show_timezone(enabled, cx);
            });
        });
        cx.notify();
    }

    fn set_change_tracking_view(&mut self, next: ChangeTrackingView, cx: &mut gpui::Context<Self>) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_change_tracking_view(next, cx);
        });
        cx.notify();
    }

    fn option_row(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        detail: Option<SharedString>,
        selected: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let id: SharedString = id.into();
        let debug_id = id.clone();
        let text_color = if selected {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };
        let selected_bg = with_alpha(theme.colors.accent, if theme.is_dark { 0.16 } else { 0.10 });
        let hover_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.08 } else { 0.05 });
        let active_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.12 } else { 0.08 });

        div()
            .id(id)
            .debug_selector(move || debug_id.to_string())
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_start()
            .gap_2()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .bg(if selected {
                selected_bg
            } else {
                gpui::rgba(0x00000000)
            })
            .hover(move |s| {
                if selected {
                    s.bg(selected_bg)
                } else {
                    s.bg(hover_bg)
                }
            })
            .active(move |s| {
                if selected {
                    s.bg(selected_bg)
                } else {
                    s.bg(active_bg)
                }
            })
            .child(
                div()
                    .w(px(16.0))
                    .text_sm()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .text_color(if selected {
                        theme.colors.accent
                    } else {
                        theme.colors.text_muted
                    })
                    .child(if selected { ">" } else { " " }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().text_sm().text_color(text_color).child(label.into()))
                    .when_some(detail, |this, detail| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child(detail),
                        )
                    }),
            )
    }

    fn summary_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: SharedString,
        expanded: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active))
            .child(div().text_sm().child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(value)
                    .child(
                        div()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .child(if expanded { "^" } else { "v" }),
                    ),
            )
    }

    fn toggle_row(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active))
            .child(div().text_sm().child(label))
            .child(
                div()
                    .text_sm()
                    .text_color(if enabled {
                        theme.colors.success
                    } else {
                        theme.colors.text_muted
                    })
                    .child(if enabled { "On" } else { "Off" }),
            )
    }

    fn info_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: SharedString,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .child(div().text_sm().child(label))
            .child(
                div()
                    .text_sm()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.text_muted)
                    .child(value),
            )
    }

    fn link_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: SharedString,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active))
            .child(div().text_sm().child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.colors.accent)
                    .child(value)
                    .child(div().font_family(UI_MONOSPACE_FONT_FAMILY).child("->")),
            )
    }

    fn open_source_license_row(
        &self,
        ix: usize,
        row: crate::view::open_source_licenses_data::OpenSourceLicenseRow,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(("settings_window_open_source_license_row", ix))
            .w_full()
            .px_2()
            .py_1()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(theme.colors.hover))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(200.0))
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(row.crate_name),
                    )
                    .child(
                        div()
                            .w(px(90.0))
                            .text_xs()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.text_muted)
                            .whitespace_nowrap()
                            .child(row.version),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.text_muted)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(row.license),
                    ),
            )
    }

    fn render_open_source_license_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let rows = crate::view::open_source_licenses_data::open_source_license_rows();
        let theme = this.theme;

        range
            .filter_map(|ix| rows.get(ix).copied().map(|row| (ix, row)))
            .map(|(ix, row)| {
                this.open_source_license_row(ix, row, theme)
                    .into_any_element()
            })
            .collect()
    }

    fn card(&self, id: &'static str, title: &'static str, theme: AppTheme) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(theme.radii.panel))
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_bg_elevated)
            .p_2()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.text_muted)
                    .child(title),
            )
    }
}

impl Render for SettingsWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let decorations = effective_window_decorations(window);
        let (tiling, client_inset) = match decorations {
            Decorations::Client { tiling } => (Some(tiling), settings_window_client_inset()),
            Decorations::Server => (None, px(0.0)),
        };
        window.set_client_inset(client_inset);

        let cursor = self
            .hover_resize_edge
            .map(chrome::cursor_style_for_resize_edge)
            .unwrap_or(CursorStyle::Arrow);
        let is_macos = cfg!(target_os = "macos");
        let header_bg = if window.is_window_active() {
            with_alpha(
                theme.colors.surface_bg,
                if theme.is_dark { 0.98 } else { 0.94 },
            )
        } else {
            theme.colors.surface_bg
        };
        let header_border = if window.is_window_active() {
            theme.colors.border
        } else {
            with_alpha(theme.colors.border, 0.7)
        };

        let drag_region = div()
            .id("settings_window_header_drag")
            .debug_selector(|| "settings_window_header_drag".to_string())
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .px_3()
            .window_control_area(WindowControlArea::Drag)
            .when(is_macos, |this| this.pl(SETTINGS_TRAFFIC_LIGHTS_SAFE_INSET))
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                if !chrome::should_handle_titlebar_double_click(e.click_count(), e.standard_click())
                {
                    return;
                }

                this.title_drag_state.clear();
                cx.stop_propagation();
                chrome::handle_titlebar_double_click(window);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _window, cx| {
                    this.title_drag_state.on_left_mouse_down(e.click_count);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, _e, window, _cx| {
                if this.title_drag_state.take_move_request() {
                    window.start_window_move();
                }
            }))
            .child(
                div()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .child(SETTINGS_WINDOW_TITLE),
            );

        let min_hover = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.08 });
        let min_active = with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.12 });
        let min = chrome::titlebar_control_button(
            theme,
            "settings_window_min_btn",
            chrome::titlebar_control_icon("icons/generic_minimize.svg", theme.colors.accent),
            min_hover,
            min_active,
        )
        .id("settings_window_min")
        .debug_selector(|| "settings_window_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.minimize_window();
        }));

        let max_icon = if window.is_maximized() {
            "icons/generic_restore.svg"
        } else {
            "icons/generic_maximize.svg"
        };
        let max_hover = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.08 });
        let max_active = with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.12 });
        let max = chrome::titlebar_control_button(
            theme,
            "settings_window_max_btn",
            chrome::titlebar_control_icon(max_icon, theme.colors.accent),
            max_hover,
            max_active,
        )
        .id("settings_window_max")
        .debug_selector(|| "settings_window_max".to_string())
        .window_control_area(WindowControlArea::Max)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.zoom_window();
            cx.notify();
        }));

        let close_hover = with_alpha(theme.colors.danger, if theme.is_dark { 0.45 } else { 0.28 });
        let close_active = with_alpha(theme.colors.danger, if theme.is_dark { 0.60 } else { 0.40 });
        let close = chrome::titlebar_control_button(
            theme,
            "settings_window_close_btn",
            chrome::titlebar_control_icon("icons/generic_close.svg", theme.colors.danger),
            close_hover,
            close_active,
        )
        .id("settings_window_close_btn")
        .debug_selector(|| "settings_window_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.remove_window();
        }));

        let header = div()
            .id("settings_window_header")
            .h(chrome::TITLE_BAR_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(header_border)
            .bg(header_bg)
            .child(drag_region)
            .when(!is_macos, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pr_2()
                        .child(min)
                        .child(max)
                        .child(close),
                )
            });

        let content = match self.current_view {
            SettingsView::Root => {
                let theme_row = self
                    .summary_row(
                        "settings_window_theme",
                        "Theme",
                        self.theme_mode.label().into(),
                        self.expanded_section == Some(SettingsSection::Theme),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::Theme, cx);
                    }));

                let date_format_row = self
                    .summary_row(
                        "settings_window_date_format",
                        "Date format",
                        self.date_time_format.label().into(),
                        self.expanded_section == Some(SettingsSection::DateFormat),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::DateFormat, cx);
                    }));

                let timezone_row = self
                    .summary_row(
                        "settings_window_timezone",
                        "Date timezone",
                        self.timezone.label().into(),
                        self.expanded_section == Some(SettingsSection::Timezone),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::Timezone, cx);
                    }));

                let show_timezone_row = self
                    .toggle_row(
                        "settings_window_show_timezone",
                        "Show timezone",
                        self.show_timezone,
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_show_timezone(!this.show_timezone, cx);
                    }));

                let change_tracking_row = self
                    .summary_row(
                        "settings_window_change_tracking",
                        "Untracked files",
                        self.change_tracking_view.settings_label().into(),
                        self.expanded_section == Some(SettingsSection::ChangeTracking),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::ChangeTracking, cx);
                    }));

                let mut general_card = self
                    .card("settings_window_general", "General", theme)
                    .child(theme_row);

                if self.expanded_section == Some(SettingsSection::Theme) {
                    for option in [ThemeMode::Automatic, ThemeMode::Light, ThemeMode::Dark] {
                        general_card = general_card.child(
                            self.option_row(
                                match option {
                                    ThemeMode::Automatic => "settings_window_theme_auto",
                                    ThemeMode::Light => "settings_window_theme_light",
                                    ThemeMode::Dark => "settings_window_theme_dark",
                                },
                                option.label(),
                                None,
                                self.theme_mode == option,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, window, cx| {
                                    this.set_theme_mode(option, window, cx);
                                },
                            )),
                        );
                    }
                }

                general_card = general_card.child(date_format_row);
                if self.expanded_section == Some(SettingsSection::DateFormat) {
                    for format in DateTimeFormat::all().iter().copied() {
                        general_card = general_card.child(
                            self.option_row(
                                match format {
                                    DateTimeFormat::YmdHm => "settings_window_date_format_ymd_hm",
                                    DateTimeFormat::YmdHms => "settings_window_date_format_ymd_hms",
                                    DateTimeFormat::DmyHm => "settings_window_date_format_dmy_hm",
                                    DateTimeFormat::MdyHm => "settings_window_date_format_mdy_hm",
                                },
                                format.label(),
                                None,
                                self.date_time_format == format,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, _window, cx| {
                                    this.set_date_time_format(format, cx);
                                },
                            )),
                        );
                    }
                }

                general_card = general_card.child(timezone_row);
                if self.expanded_section == Some(SettingsSection::Timezone) {
                    for timezone in Timezone::all().iter().copied() {
                        general_card = general_card.child(
                            self.option_row(
                                format!("settings_window_timezone_{}", timezone.key()),
                                timezone.label(),
                                Some(timezone.cities().into()),
                                self.timezone == timezone,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, _window, cx| {
                                    this.set_timezone(timezone, cx);
                                },
                            )),
                        );
                    }
                }

                general_card = general_card.child(show_timezone_row);

                let mut change_tracking_card = self
                    .card(
                        "settings_window_change_tracking_card",
                        "Change tracking",
                        theme,
                    )
                    .child(change_tracking_row);

                if self.expanded_section == Some(SettingsSection::ChangeTracking) {
                    for (id, option, detail) in [
                        (
                            "settings_window_change_tracking_combined",
                            ChangeTrackingView::Combined,
                            "Keep untracked files inside the Unstaged section",
                        ),
                        (
                            "settings_window_change_tracking_split_untracked",
                            ChangeTrackingView::SplitUntracked,
                            "Show an Untracked block above Unstaged",
                        ),
                    ] {
                        change_tracking_card = change_tracking_card.child(
                            self.option_row(
                                id,
                                option.label(),
                                Some(detail.into()),
                                self.change_tracking_view == option,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, _window, cx| {
                                    this.set_change_tracking_view(option, cx);
                                },
                            )),
                        );
                    }
                }

                let min_git_version = format!("{MIN_GIT_MAJOR}.{MIN_GIT_MINOR}");
                let (git_icon_path, git_icon_color, git_status_text): (
                    &'static str,
                    gpui::Rgba,
                    SharedString,
                ) = match self.runtime_info.git.compatibility {
                    GitCompatibility::Supported => (
                        "icons/check.svg",
                        theme.colors.success,
                        format!("Git >= {min_git_version}").into(),
                    ),
                    GitCompatibility::TooOld => (
                        "icons/warning.svg",
                        theme.colors.warning,
                        format!("Git < {min_git_version}").into(),
                    ),
                    GitCompatibility::Unknown => (
                        "icons/warning.svg",
                        theme.colors.warning,
                        "Git version unknown".into(),
                    ),
                };

                let mut environment_card = self
                    .card("settings_window_environment", "Environment", theme)
                    .child(
                        div()
                            .id("settings_window_git")
                            .w_full()
                            .px_2()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .rounded(px(theme.radii.row))
                            .child(div().text_sm().child("Git"))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(svg_icon(git_icon_path, git_icon_color, px(14.0)))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                                            .text_color(theme.colors.text_muted)
                                            .child(self.runtime_info.git.version_display.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(git_icon_color)
                                            .child(git_status_text),
                                    ),
                            ),
                    )
                    .child(self.info_row(
                        "settings_window_build",
                        "Build",
                        self.runtime_info.app_version_display.clone(),
                        theme,
                    ))
                    .child(self.info_row(
                        "settings_window_os",
                        "Operating system",
                        self.runtime_info.operating_system.clone(),
                        theme,
                    ));

                if let Some(detail) = self.runtime_info.git.detail.clone() {
                    environment_card = environment_card.child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child(detail),
                    );
                }

                let links_card = self
                    .card("settings_window_links", "Links", theme)
                    .child(
                        self.link_row("settings_window_github", "GitHub", GITHUB_URL.into(), theme)
                            .on_click(|_, _, cx| {
                                cx.open_url(GITHUB_URL);
                            }),
                    )
                    .child(
                        self.link_row(
                            "settings_window_license",
                            "License",
                            LICENSE_NAME.into(),
                            theme,
                        )
                        .on_click(|_, _, cx| {
                            cx.open_url(LICENSE_URL);
                        }),
                    )
                    .child(
                        self.link_row(
                            "settings_window_open_source_licenses",
                            "Open source licenses",
                            "Show".into(),
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, _e: &ClickEvent, _window, cx| {
                                this.show_open_source_licenses(cx);
                            },
                        )),
                    );

                div()
                    .id("settings_window_scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .child(general_card)
                    .child(change_tracking_card)
                    .child(environment_card)
                    .child(links_card)
            }
            SettingsView::OpenSourceLicenses => {
                let rows = crate::view::open_source_licenses_data::open_source_license_rows();
                let breadcrumb = div()
                    .id("settings_window_breadcrumb")
                    .w_full()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("settings_window_breadcrumb_settings")
                            .debug_selector(|| "settings_window_breadcrumb_settings".to_string())
                            .px_2()
                            .py_1()
                            .rounded(px(theme.radii.row))
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| s.bg(theme.colors.hover))
                            .active(move |s| s.bg(theme.colors.active))
                            .text_sm()
                            .text_color(theme.colors.accent)
                            .child("< Settings")
                            .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.show_root(cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.colors.text_muted)
                            .child("/"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("Open source licenses"),
                    );

                let list = if rows.is_empty() {
                    div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child("No dependency licenses found.")
                        .into_any_element()
                } else {
                    uniform_list(
                        "settings_window_open_source_licenses_list",
                        rows.len(),
                        cx.processor(Self::render_open_source_license_rows),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.open_source_licenses_scroll.clone())
                    .into_any_element()
                };

                let list_container = div()
                    .id("settings_window_open_source_licenses_list_container")
                    .relative()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_h(px(0.0))
                            .pr(components::Scrollbar::visible_gutter(
                                self.open_source_licenses_scroll.clone(),
                                components::ScrollbarAxis::Vertical,
                            ))
                            .child(list),
                    )
                    .child(
                        {
                            let scrollbar = components::Scrollbar::new(
                                "settings_window_open_source_licenses_scrollbar",
                                self.open_source_licenses_scroll.clone(),
                            )
                            .always_visible();
                            #[cfg(test)]
                            let scrollbar = scrollbar
                                .debug_selector("settings_window_open_source_licenses_scrollbar");
                            scrollbar
                        }
                        .render(theme),
                    );

                let licenses_card = self
                    .card(
                        "settings_window_open_source_licenses_card",
                        "Open source licenses",
                        theme,
                    )
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child(format!("{} third-party crates listed", rows.len())),
                    )
                    .child(
                        div()
                            .id("settings_window_open_source_licenses_columns")
                            .debug_selector(|| {
                                "settings_window_open_source_licenses_columns".to_string()
                            })
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(200.0)).child("Crate"))
                            .child(div().w(px(90.0)).child("Version"))
                            .child(div().flex_1().min_w(px(0.0)).child("License")),
                    )
                    .child(list_container);

                div()
                    .id("settings_window_open_source_licenses_view")
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .child(breadcrumb)
                    .child(licenses_card)
            }
        };

        let body = div()
            .id("settings_window_content")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.window_bg)
            .text_color(theme.colors.text)
            .child(header)
            .child(content);

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.text)
            .relative();

        root = root.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            let Decorations::Client { tiling } = effective_window_decorations(window) else {
                if this.hover_resize_edge.is_some() {
                    this.hover_resize_edge = None;
                    cx.notify();
                }
                return;
            };

            let size = window.viewport_size();
            let next = chrome::resize_edge(
                e.position,
                chrome::CLIENT_SIDE_DECORATION_INSET,
                size,
                tiling,
            );
            if next != this.hover_resize_edge {
                this.hover_resize_edge = next;
                cx.notify();
            }
        }));

        if tiling.is_some() {
            root = root.on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, e: &MouseDownEvent, window, cx| {
                    let Decorations::Client { tiling } = effective_window_decorations(window)
                    else {
                        return;
                    };

                    let size = window.viewport_size();
                    let edge = chrome::resize_edge(
                        e.position,
                        chrome::CLIENT_SIDE_DECORATION_INSET,
                        size,
                        tiling,
                    );
                    let Some(edge) = edge else {
                        return;
                    };

                    cx.stop_propagation();
                    window.start_window_resize(edge);
                }),
            );
        } else {
            self.hover_resize_edge = None;
        }

        root.child(settings_window_frame(
            theme,
            decorations,
            body.into_any_element(),
        ))
    }
}

impl SettingsRuntimeInfo {
    fn detect() -> Self {
        Self {
            git: detect_git_runtime_info(),
            app_version_display: format!("GitComet v{}", env!("CARGO_PKG_VERSION")).into(),
            operating_system: format!(
                "{} ({}, {})",
                std::env::consts::OS,
                std::env::consts::FAMILY,
                std::env::consts::ARCH
            )
            .into(),
        }
    }
}

fn detect_git_runtime_info() -> GitRuntimeInfo {
    let compatibility_message =
        format!("GitComet has been tested only with Git {MIN_GIT_MAJOR}.{MIN_GIT_MINOR} or newer.");

    let mut command = std::process::Command::new("git");
    configure_background_command(&mut command);
    match command.arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version_output = if !output.stdout.is_empty() {
                bytes_to_text_preserving_utf8(&output.stdout)
                    .trim()
                    .to_string()
            } else {
                bytes_to_text_preserving_utf8(&output.stderr)
                    .trim()
                    .to_string()
            };

            if version_output.is_empty() {
                return GitRuntimeInfo {
                    version_display: "Unavailable".into(),
                    compatibility: GitCompatibility::Unknown,
                    detail: Some(compatibility_message.into()),
                };
            }

            let compatibility = match parse_git_version(&version_output) {
                Some(version) if is_supported_git_version(version) => GitCompatibility::Supported,
                Some(_) => GitCompatibility::TooOld,
                None => GitCompatibility::Unknown,
            };

            GitRuntimeInfo {
                version_display: version_output.into(),
                compatibility,
                detail: match compatibility {
                    GitCompatibility::Supported => None,
                    GitCompatibility::TooOld | GitCompatibility::Unknown => {
                        Some(compatibility_message.into())
                    }
                },
            }
        }
        Ok(output) => {
            let stderr = bytes_to_text_preserving_utf8(&output.stderr)
                .trim()
                .to_string();
            let display = if stderr.is_empty() {
                format!("Unavailable (exit code: {})", output.status)
            } else {
                format!("Unavailable ({stderr})")
            };
            GitRuntimeInfo {
                version_display: display.into(),
                compatibility: GitCompatibility::Unknown,
                detail: Some(compatibility_message.into()),
            }
        }
        Err(err) => GitRuntimeInfo {
            version_display: format!("Unavailable ({err})").into(),
            compatibility: GitCompatibility::Unknown,
            detail: Some(compatibility_message.into()),
        },
    }
}

fn bytes_to_text_preserving_utf8(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match std::str::from_utf8(&bytes[cursor..]) {
            Ok(valid) => {
                out.push_str(valid);
                break;
            }
            Err(err) => {
                let valid_len = err.valid_up_to();
                if valid_len > 0 {
                    let valid = &bytes[cursor..cursor + valid_len];
                    out.push_str(
                        std::str::from_utf8(valid)
                            .expect("slice identified by valid_up_to must be valid UTF-8"),
                    );
                    cursor += valid_len;
                }

                let invalid_len = err.error_len().unwrap_or(1);
                let invalid_end = cursor.saturating_add(invalid_len).min(bytes.len());
                for byte in &bytes[cursor..invalid_end] {
                    let _ = write!(out, "\\x{byte:02x}");
                }
                cursor = invalid_end;
            }
        }
    }

    out
}

fn parse_git_version(raw: &str) -> Option<GitVersion> {
    raw.split_whitespace().find_map(parse_git_version_token)
}

fn parse_git_version_token(token: &str) -> Option<GitVersion> {
    let mut parts = token.split('.');
    let major = parse_u32_prefix(parts.next()?)?;
    let minor = parse_u32_prefix(parts.next()?)?;
    Some(GitVersion { major, minor })
}

fn parse_u32_prefix(part: &str) -> Option<u32> {
    let end = part
        .char_indices()
        .find_map(|(ix, ch)| (!ch.is_ascii_digit()).then_some(ix))
        .unwrap_or(part.len());
    if end == 0 {
        return None;
    }
    part[..end].parse::<u32>().ok()
}

fn is_supported_git_version(version: GitVersion) -> bool {
    version.major > MIN_GIT_MAJOR
        || (version.major == MIN_GIT_MAJOR && version.minor >= MIN_GIT_MINOR)
}

fn effective_window_decorations(window: &Window) -> Decorations {
    match window.window_decorations() {
        Decorations::Client { tiling } => Decorations::Client { tiling },
        Decorations::Server if !cfg!(target_os = "macos") => Decorations::Client {
            tiling: Tiling::default(),
        },
        Decorations::Server => Decorations::Server,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock_visual_test;
    use gitcomet_core::error::{Error, ErrorKind};
    use gitcomet_core::services::{GitBackend, GitRepository, Result};
    use gpui::Modifiers;
    use std::ops::Deref;
    use std::path::Path;

    struct TestBackend;

    impl GitBackend for TestBackend {
        fn open(&self, _workdir: &Path) -> Result<std::sync::Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::Unsupported(
                "Test backend does not open repositories",
            )))
        }
    }

    #[test]
    fn bytes_to_text_preserving_utf8_escapes_invalid_bytes() {
        assert_eq!(
            bytes_to_text_preserving_utf8(b"ok\xff\xfeend"),
            "ok\\xff\\xfeend"
        );
    }

    #[test]
    fn parse_git_version_extracts_first_version_token() {
        assert_eq!(
            parse_git_version("git version 2.50.7"),
            Some(GitVersion {
                major: 2,
                minor: 50
            })
        );
    }

    #[test]
    fn parse_git_version_token_accepts_numeric_prefixes_and_rejects_non_numeric_prefixes() {
        assert_eq!(
            parse_git_version_token("2.45.1.windows.1"),
            Some(GitVersion {
                major: 2,
                minor: 45
            })
        );
        assert_eq!(parse_git_version_token("v2.45.1"), None);
        assert_eq!(parse_u32_prefix("53rc1"), Some(53));
        assert_eq!(parse_u32_prefix("rc53"), None);
    }

    #[test]
    fn supported_version_requires_minimum_2_50() {
        assert!(is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR,
            minor: MIN_GIT_MINOR,
        }));
        assert!(is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR,
            minor: MIN_GIT_MINOR + 1,
        }));
        assert!(!is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR,
            minor: MIN_GIT_MINOR - 1,
        }));
        assert!(is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR + 1,
            minor: 0,
        }));
    }

    #[test]
    fn settings_window_titlebar_options_match_platform_chrome_strategy() {
        let options = settings_window_titlebar_options();
        assert_eq!(
            options.appears_transparent,
            cfg!(any(target_os = "macos", target_os = "windows")),
            "settings window titlebar transparency should match the platform chrome strategy"
        );
        assert_eq!(
            options.title.as_ref().map(ToString::to_string),
            Some(SETTINGS_WINDOW_TITLE.to_string()),
            "settings window titlebar should keep the OS-visible title"
        );
    }

    #[test]
    fn settings_window_frame_strategy_matches_platform_chrome() {
        #[cfg(target_os = "windows")]
        {
            assert_eq!(settings_window_client_inset(), px(0.0));
        }

        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(
                settings_window_client_inset(),
                chrome::CLIENT_SIDE_DECORATION_INSET
            );
        }
    }

    #[gpui::test]
    fn settings_window_sets_platform_title(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
        let (_main_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            open_settings_window(app);
        });
        cx.run_until_parked();

        let settings_window = cx.update(|_window, app| {
            app.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window should be open")
        });

        let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
        settings_cx.run_until_parked();

        assert_eq!(
            settings_cx.window_title().as_deref(),
            Some(SETTINGS_WINDOW_TITLE),
            "expected settings window to expose the native OS title"
        );
    }

    #[gpui::test]
    fn settings_window_open_source_licenses_row_switches_content(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
        let (_main_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            open_settings_window(app);
        });
        cx.run_until_parked();

        let settings_window = cx.update(|_window, app| {
            app.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window should be open")
        });

        let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
        settings_cx.run_until_parked();
        settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let row_bounds = settings_cx
            .debug_bounds("settings_window_open_source_licenses")
            .expect("expected open source licenses row bounds");
        settings_cx.simulate_click(row_bounds.center(), Modifiers::default());
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        cx.update(|_window, app| {
            assert_eq!(
                app.windows().len(),
                2,
                "expected the settings window to reuse the existing window"
            );
            assert_eq!(
                settings_window
                    .read_with(app, |settings, _cx| settings.current_view)
                    .expect("settings window should remain readable"),
                SettingsView::OpenSourceLicenses,
                "expected the settings window to switch to open source licenses content"
            );
        });

        assert_eq!(
            settings_cx.window_title().as_deref(),
            Some(SETTINGS_WINDOW_TITLE),
            "expected the settings window to keep its OS title"
        );
        assert!(
            settings_cx
                .debug_bounds("settings_window_breadcrumb_settings")
                .is_some(),
            "expected a breadcrumb back control in the licenses view"
        );
        assert!(
            settings_cx
                .debug_bounds("settings_window_open_source_licenses_columns")
                .is_some(),
            "expected open source licenses columns in debug bounds"
        );
        assert!(
            settings_cx
                .debug_bounds("settings_window_open_source_licenses_scrollbar")
                .is_some(),
            "expected a visible scrollbar in the open source licenses view"
        );

        let back_bounds = settings_cx
            .debug_bounds("settings_window_breadcrumb_settings")
            .expect("expected breadcrumb back control bounds");
        settings_cx.simulate_click(back_bounds.center(), Modifiers::default());
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        cx.update(|_window, app| {
            assert_eq!(
                settings_window
                    .read_with(app, |settings, _cx| settings.current_view)
                    .expect("settings window should remain readable"),
                SettingsView::Root,
                "expected the breadcrumb back control to return to the root settings view"
            );
        });
    }

    #[gpui::test]
    fn non_macos_settings_window_uses_client_chrome_and_resize_edges(
        cx: &mut gpui::TestAppContext,
    ) {
        if cfg!(target_os = "macos") {
            return;
        }

        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
        let (_main_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            open_settings_window(app);
        });
        cx.run_until_parked();

        let settings_window = cx.update(|_window, app| {
            app.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window should be open")
        });

        let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        for selector in [
            "settings_window_header_drag",
            "settings_window_min",
            "settings_window_max",
            "settings_window_close",
        ] {
            assert!(
                settings_cx.debug_bounds(selector).is_some(),
                "expected `{selector}` in debug bounds"
            );
        }

        settings_cx.simulate_mouse_move(point(px(1.0), px(1.0)), None, Modifiers::default());
        settings_cx.run_until_parked();

        cx.update(|_window, app| {
            assert_eq!(
                settings_window
                    .read_with(app, |settings, _cx| settings.hover_resize_edge)
                    .expect("settings window should remain readable"),
                Some(ResizeEdge::TopLeft),
                "expected top-left corner hover to expose a resize edge"
            );
        });
    }

    #[gpui::test]
    fn linux_settings_window_close_button_closes_only_the_settings_window(
        cx: &mut gpui::TestAppContext,
    ) {
        if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return;
        }

        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
        let (_main_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            open_settings_window(app);
        });
        cx.run_until_parked();

        let settings_window = cx.update(|_window, app| {
            assert_eq!(app.windows().len(), 2, "expected main + settings windows");
            app.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window should be open")
        });

        let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let close_bounds = settings_cx
            .debug_bounds("settings_window_close")
            .expect("expected settings window close control bounds");
        settings_cx.simulate_mouse_move(close_bounds.center(), None, Modifiers::default());
        settings_cx.simulate_mouse_down(
            close_bounds.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        settings_cx.simulate_mouse_up(
            close_bounds.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        settings_cx.run_until_parked();

        cx.update(|_window, app| {
            assert_eq!(
                app.windows().len(),
                1,
                "expected the settings close control to close only the settings window"
            );
            assert!(
                app.windows()
                    .into_iter()
                    .all(|window| window.downcast::<SettingsWindowView>().is_none()),
                "expected the settings window to be removed"
            );
        });
    }

    #[gpui::test]
    fn show_timezone_toggle_defers_main_window_update(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
        let (main_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            open_settings_window(app);
        });
        cx.run_until_parked();

        let settings_window = cx.update(|_window, app| {
            app.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window should be open")
        });

        let next_show_timezone = cx.update(|_window, app| {
            !settings_window
                .read_with(app, |settings, _cx| settings.show_timezone)
                .expect("settings window should be readable")
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cx.update(|_window, app| {
                main_view.update(app, |_view, cx| {
                    let _ = settings_window.update(cx, |settings, _window, cx| {
                        settings.set_show_timezone(next_show_timezone, cx);
                    });
                });
            });
        }));
        assert!(
            result.is_ok(),
            "settings window toggle should not re-enter GitCometView updates"
        );

        cx.run_until_parked();

        cx.update(|_window, app| {
            assert_eq!(
                main_view.read(app).show_timezone_for_test(),
                next_show_timezone
            );
            assert_eq!(
                settings_window
                    .read_with(app, |settings, _cx| settings.show_timezone)
                    .expect("settings window should remain readable"),
                next_show_timezone
            );
        });
    }

    #[gpui::test]
    fn change_tracking_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
        let (main_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            open_settings_window(app);
        });
        cx.run_until_parked();

        let settings_window = cx.update(|_window, app| {
            app.windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window should be open")
        });

        let next_view = cx.update(|_window, app| {
            let current = settings_window
                .read_with(app, |settings, _cx| settings.change_tracking_view)
                .expect("settings window should be readable");
            match current {
                ChangeTrackingView::Combined => ChangeTrackingView::SplitUntracked,
                ChangeTrackingView::SplitUntracked => ChangeTrackingView::Combined,
            }
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cx.update(|_window, app| {
                main_view.update(app, |_view, cx| {
                    let _ = settings_window.update(cx, |settings, _window, cx| {
                        settings.set_change_tracking_view(next_view, cx);
                    });
                });
            });
        }));
        assert!(
            result.is_ok(),
            "change tracking update should not re-enter GitCometView updates"
        );

        cx.run_until_parked();

        cx.update(|_window, app| {
            assert_eq!(
                main_view.read(app).change_tracking_view_for_test(),
                next_view
            );
            assert_eq!(
                settings_window
                    .read_with(app, |settings, _cx| settings.change_tracking_view)
                    .expect("settings window should remain readable"),
                next_view
            );
        });
    }
}
