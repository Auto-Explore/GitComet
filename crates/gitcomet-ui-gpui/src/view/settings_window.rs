use super::*;
use gitcomet_core::process::configure_background_command;
use gpui::{Stateful, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions};
use std::sync::Arc;

const SETTINGS_WINDOW_MIN_WIDTH_PX: f32 = 620.0;
const SETTINGS_WINDOW_MIN_HEIGHT_PX: f32 = 460.0;
const SETTINGS_WINDOW_DEFAULT_WIDTH_PX: f32 = 720.0;
const SETTINGS_WINDOW_DEFAULT_HEIGHT_PX: f32 = 620.0;
const SETTINGS_DROPDOWN_LIST_MAX_HEIGHT_PX: f32 = 224.0;
const SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX: f32 = 28.0;
const SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX: f32 = 20.0;
const SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX: f32 = 42.0;
const SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX: f32 = 24.0;
const SETTINGS_DROPDOWN_DENSE_DETAIL_ROW_HEIGHT_PX: f32 = 28.0;
const SETTINGS_WINDOW_TITLE: &str = "Settings: GitComet";
const SETTINGS_TRAFFIC_LIGHTS_SAFE_INSET: Pixels = px(78.0);
const MIN_GIT_MAJOR: u32 = 2;
const MIN_GIT_MINOR: u32 = 50;
const GITHUB_URL: &str = "https://github.com/Auto-Explore/GitComet";
const LICENSE_URL: &str = "https://github.com/Auto-Explore/GitComet/blob/main/LICENSE-AGPL-3.0";
const LICENSE_NAME: &str = "AGPL-3.0";

const CHANGE_TRACKING_OPTIONS: &[(&str, ChangeTrackingView, &str)] = &[
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
];

const DIFF_SCROLL_SYNC_OPTIONS: &[(&str, DiffScrollSync, &str)] = &[
    (
        "settings_window_diff_scroll_sync_vertical",
        DiffScrollSync::Vertical,
        "Lock vertical scrolling only.",
    ),
    (
        "settings_window_diff_scroll_sync_horizontal",
        DiffScrollSync::Horizontal,
        "Lock horizontal scrolling only.",
    ),
    (
        "settings_window_diff_scroll_sync_none",
        DiffScrollSync::None,
        "Keep split and merge panes independent.",
    ),
    (
        "settings_window_diff_scroll_sync_both",
        DiffScrollSync::Both,
        "Lock both vertical and horizontal scrolling.",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsSection {
    Theme,
    UiFont,
    EditorFont,
    DateFormat,
    Timezone,
    ChangeTracking,
    Diff,
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
    ui_font_family: String,
    editor_font_family: String,
    use_font_ligatures: bool,
    ui_font_options: Arc<[String]>,
    editor_font_options: Arc<[String]>,
    settings_window_scroll: ScrollHandle,
    theme_scroll: UniformListScrollHandle,
    ui_font_scroll: UniformListScrollHandle,
    editor_font_scroll: UniformListScrollHandle,
    date_format_scroll: UniformListScrollHandle,
    timezone_scroll: UniformListScrollHandle,
    change_tracking_scroll: UniformListScrollHandle,
    diff_scroll_sync_scroll: UniformListScrollHandle,
    date_time_format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    change_tracking_view: ChangeTrackingView,
    diff_scroll_sync: DiffScrollSync,
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
        cx.activate(true);
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
        settings_window_options(bounds),
        |window, cx| cx.new(|cx| SettingsWindowView::new(window, cx)),
    )
    .expect("failed to open settings window");

    cx.activate(true);
}

fn settings_window_options(bounds: Bounds<Pixels>) -> WindowOptions {
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
    }
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

fn uniform_list_vertical_wheel_delta(event: &gpui::ScrollWheelEvent, window: &Window) -> Pixels {
    let pixel_delta = event.delta.pixel_delta(window.line_height());
    if !pixel_delta.y.is_zero() {
        pixel_delta.y
    } else {
        pixel_delta.x
    }
}

fn normalize_scroll_offset(raw_offset: Pixels, max_offset: Pixels) -> Pixels {
    if max_offset <= px(0.0) {
        return px(0.0);
    }

    if raw_offset < px(0.0) {
        (-raw_offset).max(px(0.0)).min(max_offset)
    } else {
        raw_offset.max(px(0.0)).min(max_offset)
    }
}

fn uniform_list_vertical_scroll_metrics(
    handle: &UniformListScrollHandle,
) -> (Pixels, Pixels, Pixels) {
    let state = handle.0.borrow();
    let max_offset = state
        .last_item_size
        .map(|size| (size.contents.height - size.item.height).max(px(0.0)))
        .unwrap_or_else(|| state.base_handle.max_offset().height.max(px(0.0)));
    let raw_offset = state.base_handle.offset().y;
    let scroll_offset = normalize_scroll_offset(raw_offset, max_offset);
    (raw_offset, scroll_offset, max_offset)
}

fn uniform_list_should_stop_scroll_propagation(
    handle: &UniformListScrollHandle,
    event: &gpui::ScrollWheelEvent,
    window: &Window,
) -> bool {
    let delta_y = uniform_list_vertical_wheel_delta(event, window);
    if delta_y.is_zero() {
        return false;
    }

    let (raw_offset_after, _scroll_offset_after, max_offset) =
        uniform_list_vertical_scroll_metrics(handle);
    if max_offset <= px(0.0) {
        return false;
    }

    // This runs after the list's built-in wheel scroll listener, so reconstruct the pre-scroll
    // position before deciding whether to keep the event inside the dropdown.
    let raw_offset_before = raw_offset_after - delta_y;
    let scroll_offset_before = normalize_scroll_offset(raw_offset_before, max_offset);
    if delta_y < px(0.0) {
        scroll_offset_before < max_offset
    } else {
        scroll_offset_before > px(0.0)
    }
}

fn mix_color(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    gpui::Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

fn settings_dropdown_background(theme: AppTheme) -> gpui::Rgba {
    if theme.is_dark {
        mix_color(
            theme.colors.surface_bg_elevated,
            theme.colors.window_bg,
            0.58,
        )
    } else {
        mix_color(theme.colors.surface_bg_elevated, theme.colors.border, 0.55)
    }
}

fn settings_dropdown_height(
    item_count: usize,
    estimated_row_height_px: f32,
    extra_height_px: f32,
) -> Pixels {
    px(
        (((item_count.max(1) as f32) * estimated_row_height_px) + extra_height_px)
            .min(SETTINGS_DROPDOWN_LIST_MAX_HEIGHT_PX),
    )
}

fn settings_theme_modes() -> Vec<ThemeMode> {
    let mut modes = Vec::with_capacity(crate::theme::available_themes().len() + 1);
    modes.push(ThemeMode::Automatic);
    modes.extend(
        crate::theme::available_themes()
            .into_iter()
            .map(|theme| ThemeMode::Named(theme.key.to_string())),
    );
    modes
}

impl SettingsWindowView {
    fn new(window: &mut Window, cx: &mut gpui::Context<Self>) -> Self {
        window.set_window_title(SETTINGS_WINDOW_TITLE);

        let ui_session = session::load();
        let font_preferences =
            crate::font_preferences::current_or_initialize_from_session(window, &ui_session, cx);
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
        let diff_scroll_sync = ui_session
            .diff_scroll_sync
            .as_deref()
            .and_then(DiffScrollSync::from_key)
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
                    if !this.theme_mode.is_automatic() {
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
            ui_font_family: font_preferences.ui_font_family,
            editor_font_family: font_preferences.editor_font_family,
            use_font_ligatures: font_preferences.use_font_ligatures,
            ui_font_options: crate::font_preferences::ui_font_options(window),
            editor_font_options: crate::font_preferences::editor_font_options(window),
            settings_window_scroll: ScrollHandle::default(),
            theme_scroll: UniformListScrollHandle::default(),
            ui_font_scroll: UniformListScrollHandle::default(),
            editor_font_scroll: UniformListScrollHandle::default(),
            date_format_scroll: UniformListScrollHandle::default(),
            timezone_scroll: UniformListScrollHandle::default(),
            change_tracking_scroll: UniformListScrollHandle::default(),
            diff_scroll_sync_scroll: UniformListScrollHandle::default(),
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            diff_scroll_sync,
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
            ui_font_family: Some(self.ui_font_family.clone()),
            editor_font_family: Some(self.editor_font_family.clone()),
            use_font_ligatures: Some(self.use_font_ligatures),
            date_time_format: Some(self.date_time_format.key().to_string()),
            timezone: Some(self.timezone.key()),
            show_timezone: Some(self.show_timezone),
            change_tracking_view: Some(self.change_tracking_view.key().to_string()),
            diff_scroll_sync: Some(self.diff_scroll_sync.key().to_string()),
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

    fn font_option_detail(&self, family: &str) -> Option<SharedString> {
        match family {
            crate::font_preferences::UI_SYSTEM_FONT_FAMILY => {
                Some("Use GitComet's best match for the operating system UI font stack".into())
            }
            _ => None,
        }
    }

    fn font_options_hint(&self, family: &str) -> SharedString {
        self.font_option_detail(family)
            .unwrap_or_else(|| "Choose from installed system fonts".into())
    }

    fn font_option_row_for_family(
        &self,
        id_prefix: &'static str,
        ix: usize,
        family: &str,
        selected: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        self.option_row(
            format!("{id_prefix}_{ix}"),
            crate::font_preferences::display_label(family),
            None,
            selected,
            theme,
        )
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

        self.theme_mode = mode.clone();
        self.theme = mode.resolve_theme(window.appearance());
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, root_window, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.set_theme_mode(mode.clone(), root_window.appearance(), cx);
            });
        });
        cx.notify();
    }

    fn set_ui_font_family(&mut self, family: String, cx: &mut gpui::Context<Self>) {
        if self.ui_font_family == family {
            return;
        }

        self.ui_font_family = family;
        self.expanded_section = None;
        crate::font_preferences::set_current(
            cx,
            self.ui_font_family.clone(),
            self.editor_font_family.clone(),
            self.use_font_ligatures,
        );
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.notify_font_preferences_changed(cx);
        });
        cx.notify();
    }

    fn set_editor_font_family(&mut self, family: String, cx: &mut gpui::Context<Self>) {
        if self.editor_font_family == family {
            return;
        }

        self.editor_font_family = family;
        self.expanded_section = None;
        crate::font_preferences::set_current(
            cx,
            self.ui_font_family.clone(),
            self.editor_font_family.clone(),
            self.use_font_ligatures,
        );
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.notify_font_preferences_changed(cx);
        });
        cx.notify();
    }

    fn set_use_font_ligatures(&mut self, enabled: bool, cx: &mut gpui::Context<Self>) {
        if self.use_font_ligatures == enabled {
            return;
        }

        self.use_font_ligatures = enabled;
        crate::font_preferences::set_current(
            cx,
            self.ui_font_family.clone(),
            self.editor_font_family.clone(),
            self.use_font_ligatures,
        );
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.notify_font_preferences_changed(cx);
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

    fn set_diff_scroll_sync(&mut self, next: DiffScrollSync, cx: &mut gpui::Context<Self>) {
        if self.diff_scroll_sync == next {
            return;
        }

        self.diff_scroll_sync = next;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_scroll_sync(next, cx);
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
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(detail),
                        )
                    }),
            )
    }

    fn dense_detail_option_row(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
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
            .min_h(px(SETTINGS_DROPDOWN_DENSE_DETAIL_ROW_HEIGHT_PX))
            .px_2()
            .py(px(2.0))
            .flex()
            .items_center()
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
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(text_color)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(label.into()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(detail.into()),
                    ),
            )
    }

    fn empty_dropdown_list(&self, message: &'static str, theme: AppTheme) -> AnyElement {
        div()
            .h_full()
            .min_h(px(0.0))
            .px_2()
            .py_1()
            .text_sm()
            .text_color(theme.colors.text_muted)
            .child(message)
            .into_any_element()
    }

    fn dropdown_list_container(
        &self,
        container_id: &'static str,
        scrollbar_id: &'static str,
        scroll: UniformListScrollHandle,
        item_count: usize,
        estimated_row_height_px: f32,
        extra_height_px: f32,
        list: AnyElement,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let height = settings_dropdown_height(item_count, estimated_row_height_px, extra_height_px);

        div()
            .id(container_id)
            .debug_selector(move || container_id.to_string())
            .relative()
            .h(height)
            .min_h(height)
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(if theme.is_dark {
                with_alpha(theme.colors.border, 0.98)
            } else {
                theme.colors.border
            })
            .bg(settings_dropdown_background(theme))
            .overflow_hidden()
            .child(
                div()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list),
            )
            .child(
                components::Scrollbar::new(scrollbar_id, scroll)
                    .always_visible()
                    .render(theme),
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

    fn render_ui_font_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                this.ui_font_options
                    .get(ix)
                    .cloned()
                    .map(|family| (ix, family))
            })
            .map(|(ix, family)| {
                this.font_option_row_for_family(
                    "settings_window_ui_font",
                    ix,
                    family.as_str(),
                    this.ui_font_family == family,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_ui_font_family(family.clone(), cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn render_theme_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let modes = settings_theme_modes();
        range
            .filter_map(|ix| modes.get(ix).cloned())
            .map(|mode| {
                this.option_row(
                    format!("settings_window_theme_{}", mode.key()),
                    mode.label(),
                    None,
                    this.theme_mode == mode,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
                    this.set_theme_mode(mode.clone(), window, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn render_editor_font_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                this.editor_font_options
                    .get(ix)
                    .cloned()
                    .map(|family| (ix, family))
            })
            .map(|(ix, family)| {
                this.font_option_row_for_family(
                    "settings_window_editor_font",
                    ix,
                    family.as_str(),
                    this.editor_font_family == family,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_editor_font_family(family.clone(), cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn render_date_format_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                DateTimeFormat::all()
                    .get(ix)
                    .copied()
                    .map(|format| (ix, format))
            })
            .map(|(_ix, format)| {
                this.option_row(
                    match format {
                        DateTimeFormat::YmdHm => "settings_window_date_format_ymd_hm",
                        DateTimeFormat::YmdHms => "settings_window_date_format_ymd_hms",
                        DateTimeFormat::DmyHm => "settings_window_date_format_dmy_hm",
                        DateTimeFormat::MdyHm => "settings_window_date_format_mdy_hm",
                    },
                    format.label(),
                    None,
                    this.date_time_format == format,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_date_time_format(format, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn render_timezone_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                Timezone::all()
                    .get(ix)
                    .copied()
                    .map(|timezone| (ix, timezone))
            })
            .map(|(_ix, timezone)| {
                this.dense_detail_option_row(
                    format!("settings_window_timezone_{}", timezone.key()),
                    timezone.label(),
                    timezone.cities(),
                    this.timezone == timezone,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_timezone(timezone, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn render_change_tracking_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| CHANGE_TRACKING_OPTIONS.get(ix).copied())
            .map(|(id, option, detail)| {
                this.option_row(
                    id,
                    option.label(),
                    Some(detail.into()),
                    this.change_tracking_view == option,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_change_tracking_view(option, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    fn render_diff_scroll_sync_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| DIFF_SCROLL_SYNC_OPTIONS.get(ix).copied())
            .map(|(id, option, detail)| {
                this.option_row(
                    id,
                    option.label(),
                    Some(detail.into()),
                    this.diff_scroll_sync == option,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_scroll_sync(option, cx);
                }))
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
        let decorations = window.window_decorations();
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
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|_this, e: &MouseUpEvent, window, cx| {
                    chrome::show_titlebar_secondary_menu(e.position, window, cx);
                }),
            )
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
            crate::app::toggle_window_zoom(window);
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

                let ui_font_row = self
                    .summary_row(
                        "settings_window_ui_font",
                        "UI Font",
                        crate::font_preferences::display_label(&self.ui_font_family).into(),
                        self.expanded_section == Some(SettingsSection::UiFont),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::UiFont, cx);
                    }));

                let editor_font_row = self
                    .summary_row(
                        "settings_window_editor_font",
                        "Editor Font",
                        crate::font_preferences::display_label(&self.editor_font_family).into(),
                        self.expanded_section == Some(SettingsSection::EditorFont),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::EditorFont, cx);
                    }));

                let font_ligatures_row = self
                    .toggle_row(
                        "settings_window_use_font_ligatures",
                        "Use font ligatures",
                        self.use_font_ligatures,
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_use_font_ligatures(!this.use_font_ligatures, cx);
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

                let diff_scroll_sync_row = self
                    .summary_row(
                        "settings_window_diff_scroll_sync",
                        "Scroll sync",
                        self.diff_scroll_sync.label().into(),
                        self.expanded_section == Some(SettingsSection::Diff),
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.toggle_section(SettingsSection::Diff, cx);
                    }));

                let mut general_card = self
                    .card("settings_window_general", "General", theme)
                    .child(theme_row);

                if self.expanded_section == Some(SettingsSection::Theme) {
                    let theme_mode_count = settings_theme_modes().len();
                    let list = uniform_list(
                        "settings_window_theme_list",
                        theme_mode_count,
                        cx.processor(Self::render_theme_option_rows),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.theme_scroll.clone())
                    .on_scroll_wheel({
                        let scroll = self.theme_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .into_any_element();
                    general_card = general_card.child(self.dropdown_list_container(
                        "settings_window_theme_list_container",
                        "settings_window_theme_scrollbar",
                        self.theme_scroll.clone(),
                        theme_mode_count,
                        SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                        SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                        list,
                        theme,
                    ));
                }

                general_card = general_card.child(ui_font_row);
                if self.expanded_section == Some(SettingsSection::UiFont) {
                    let list = if self.ui_font_options.is_empty() {
                        self.empty_dropdown_list("No fonts available.", theme)
                    } else {
                        uniform_list(
                            "settings_window_ui_font_list",
                            self.ui_font_options.len(),
                            cx.processor(Self::render_ui_font_option_rows),
                        )
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(self.ui_font_scroll.clone())
                        .on_scroll_wheel({
                            let scroll = self.ui_font_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        })
                        .into_any_element()
                    };
                    general_card = general_card
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child(self.font_options_hint(self.ui_font_family.as_str())),
                        )
                        .child(self.dropdown_list_container(
                            "settings_window_ui_font_list_container",
                            "settings_window_ui_font_scrollbar",
                            self.ui_font_scroll.clone(),
                            self.ui_font_options.len(),
                            SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                            0.0,
                            list,
                            theme,
                        ));
                }

                general_card = general_card.child(editor_font_row);
                if self.expanded_section == Some(SettingsSection::EditorFont) {
                    let list = if self.editor_font_options.is_empty() {
                        self.empty_dropdown_list("No fonts available.", theme)
                    } else {
                        uniform_list(
                            "settings_window_editor_font_list",
                            self.editor_font_options.len(),
                            cx.processor(Self::render_editor_font_option_rows),
                        )
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(self.editor_font_scroll.clone())
                        .on_scroll_wheel({
                            let scroll = self.editor_font_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        })
                        .into_any_element()
                    };
                    general_card = general_card
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child(self.font_options_hint(self.editor_font_family.as_str())),
                        )
                        .child(self.dropdown_list_container(
                            "settings_window_editor_font_list_container",
                            "settings_window_editor_font_scrollbar",
                            self.editor_font_scroll.clone(),
                            self.editor_font_options.len(),
                            SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                            0.0,
                            list,
                            theme,
                        ));
                }

                general_card = general_card.child(font_ligatures_row);

                general_card = general_card.child(date_format_row);
                if self.expanded_section == Some(SettingsSection::DateFormat) {
                    let list = uniform_list(
                        "settings_window_date_format_list",
                        DateTimeFormat::all().len(),
                        cx.processor(Self::render_date_format_option_rows),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.date_format_scroll.clone())
                    .on_scroll_wheel({
                        let scroll = self.date_format_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .into_any_element();
                    general_card = general_card.child(self.dropdown_list_container(
                        "settings_window_date_format_list_container",
                        "settings_window_date_format_scrollbar",
                        self.date_format_scroll.clone(),
                        DateTimeFormat::all().len(),
                        SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                        SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                        list,
                        theme,
                    ));
                }

                general_card = general_card.child(timezone_row);
                if self.expanded_section == Some(SettingsSection::Timezone) {
                    let list = uniform_list(
                        "settings_window_timezone_list",
                        Timezone::all().len(),
                        cx.processor(Self::render_timezone_option_rows),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.timezone_scroll.clone())
                    .on_scroll_wheel({
                        let scroll = self.timezone_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .into_any_element();
                    general_card = general_card.child(self.dropdown_list_container(
                        "settings_window_timezone_list_container",
                        "settings_window_timezone_scrollbar",
                        self.timezone_scroll.clone(),
                        Timezone::all().len(),
                        SETTINGS_DROPDOWN_DENSE_DETAIL_ROW_HEIGHT_PX,
                        0.0,
                        list,
                        theme,
                    ));
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
                    let list = uniform_list(
                        "settings_window_change_tracking_list",
                        CHANGE_TRACKING_OPTIONS.len(),
                        cx.processor(Self::render_change_tracking_option_rows),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.change_tracking_scroll.clone())
                    .on_scroll_wheel({
                        let scroll = self.change_tracking_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .into_any_element();
                    change_tracking_card =
                        change_tracking_card.child(self.dropdown_list_container(
                            "settings_window_change_tracking_list_container",
                            "settings_window_change_tracking_scrollbar",
                            self.change_tracking_scroll.clone(),
                            CHANGE_TRACKING_OPTIONS.len(),
                            SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                            list,
                            theme,
                        ));
                }

                let mut diff_card = self
                    .card("settings_window_diff_card", "Diff", theme)
                    .child(diff_scroll_sync_row);

                if self.expanded_section == Some(SettingsSection::Diff) {
                    let list = uniform_list(
                        "settings_window_diff_scroll_sync_list",
                        DIFF_SCROLL_SYNC_OPTIONS.len(),
                        cx.processor(Self::render_diff_scroll_sync_option_rows),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.diff_scroll_sync_scroll.clone())
                    .on_scroll_wheel({
                        let scroll = self.diff_scroll_sync_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .into_any_element();
                    diff_card = diff_card.child(self.dropdown_list_container(
                        "settings_window_diff_scroll_sync_list_container",
                        "settings_window_diff_scroll_sync_scrollbar",
                        self.diff_scroll_sync_scroll.clone(),
                        DIFF_SCROLL_SYNC_OPTIONS.len(),
                        SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                        SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX + 18.0,
                        list,
                        theme,
                    ));
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
                    .track_scroll(&self.settings_window_scroll)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .child(general_card)
                    .child(change_tracking_card)
                    .child(diff_card)
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
            .font(gpui::Font {
                family: crate::font_preferences::applied_ui_font_family(&self.ui_font_family)
                    .into(),
                features: crate::font_preferences::applied_font_features(self.use_font_ligatures),
                fallbacks: None,
                weight: gpui::FontWeight::default(),
                style: gpui::FontStyle::default(),
            })
            .text_color(theme.colors.text)
            .child(header)
            .child(content);

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.text)
            .relative();

        root = root.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            let Decorations::Client { tiling } = window.window_decorations() else {
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
                    let Decorations::Client { tiling } = window.window_decorations() else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock_visual_test;
    use gitcomet_core::error::{Error, ErrorKind};
    use gitcomet_core::services::{GitBackend, GitRepository, Result};
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent};
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

    #[test]
    fn settings_window_options_request_client_chrome_and_resize_behavior() {
        let bounds = Bounds::new(
            point(px(12.0), px(24.0)),
            size(
                px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX),
                px(SETTINGS_WINDOW_DEFAULT_HEIGHT_PX),
            ),
        );
        let options = settings_window_options(bounds);

        assert_eq!(
            options.window_bounds,
            Some(WindowBounds::Windowed(bounds)),
            "settings window should open at the requested bounds"
        );
        assert_eq!(
            options.window_min_size,
            Some(size(
                px(SETTINGS_WINDOW_MIN_WIDTH_PX),
                px(SETTINGS_WINDOW_MIN_HEIGHT_PX),
            )),
            "settings window should enforce its minimum size"
        );
        assert_eq!(
            options.window_decorations,
            Some(WindowDecorations::Client),
            "settings window should request client-side decorations"
        );
        assert!(
            options.is_movable,
            "settings window should remain movable with custom chrome"
        );
        assert!(
            options.is_resizable,
            "settings window should remain resizable with custom chrome"
        );
    }

    #[test]
    fn settings_dropdown_background_is_darker_than_card_surface() {
        fn brightness(color: gpui::Rgba) -> f32 {
            color.r + color.g + color.b
        }

        let dark = AppTheme::gitcomet_dark();
        assert!(
            brightness(settings_dropdown_background(dark))
                < brightness(dark.colors.surface_bg_elevated),
            "dark dropdown surface should be darker than the card surface"
        );

        let light = AppTheme::gitcomet_light();
        assert!(
            brightness(settings_dropdown_background(light))
                < brightness(light.colors.surface_bg_elevated),
            "light dropdown surface should still read darker than the card surface"
        );
    }

    #[test]
    fn settings_theme_modes_include_automatic_and_all_available_named_themes() {
        let modes = settings_theme_modes();
        assert_eq!(modes.first(), Some(&ThemeMode::Automatic));

        let named_modes = modes.iter().skip(1).map(ThemeMode::key).collect::<Vec<_>>();
        let available_themes = crate::theme::available_themes()
            .into_iter()
            .map(|theme| theme.key.to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            named_modes,
            available_themes
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
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
    fn expanded_settings_sections_render_scrollable_list_containers(cx: &mut gpui::TestAppContext) {
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

        for (section, selector) in [
            (
                SettingsSection::Theme,
                "settings_window_theme_list_container",
            ),
            (
                SettingsSection::DateFormat,
                "settings_window_date_format_list_container",
            ),
            (
                SettingsSection::UiFont,
                "settings_window_ui_font_list_container",
            ),
            (
                SettingsSection::EditorFont,
                "settings_window_editor_font_list_container",
            ),
            (
                SettingsSection::Timezone,
                "settings_window_timezone_list_container",
            ),
            (
                SettingsSection::ChangeTracking,
                "settings_window_change_tracking_list_container",
            ),
            (
                SettingsSection::Diff,
                "settings_window_diff_scroll_sync_list_container",
            ),
        ] {
            let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
                settings.expanded_section = Some(section);
                cx.notify();
            });
            settings_cx.run_until_parked();
            settings_cx.update(|window, app| {
                let _ = window.draw(app);
            });

            assert!(
                settings_cx.debug_bounds(selector).is_some(),
                "expected `{selector}` to be rendered for the expanded section"
            );
        }
    }

    #[gpui::test]
    fn settings_dropdowns_fit_without_inner_scroll(cx: &mut gpui::TestAppContext) {
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

        for (section, label) in [
            (SettingsSection::Theme, "Theme"),
            (SettingsSection::DateFormat, "Date time format"),
            (SettingsSection::ChangeTracking, "Untracked files"),
            (SettingsSection::Diff, "Diff scroll sync"),
        ] {
            let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
                settings.expanded_section = Some(section);
                cx.notify();
            });
            settings_cx.run_until_parked();
            settings_cx.update(|window, app| {
                let _ = window.draw(app);
            });

            let max_offset = settings_window
                .update(&mut settings_cx, |settings, _window, _cx| match section {
                    SettingsSection::Theme => {
                        uniform_list_vertical_scroll_metrics(&settings.theme_scroll).2
                    }
                    SettingsSection::DateFormat => {
                        uniform_list_vertical_scroll_metrics(&settings.date_format_scroll).2
                    }
                    SettingsSection::ChangeTracking => {
                        uniform_list_vertical_scroll_metrics(&settings.change_tracking_scroll).2
                    }
                    SettingsSection::Diff => {
                        uniform_list_vertical_scroll_metrics(&settings.diff_scroll_sync_scroll).2
                    }
                    _ => px(0.0),
                })
                .expect("settings window should remain readable");

            assert_eq!(
                max_offset,
                px(0.0),
                "expected the {label} dropdown to fit without inner scroll"
            );
        }
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
    fn non_macos_settings_window_renders_custom_chrome_controls(
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

    #[gpui::test]
    fn diff_scroll_sync_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
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

        let next_mode = cx.update(|_window, app| {
            let current = settings_window
                .read_with(app, |settings, _cx| settings.diff_scroll_sync)
                .expect("settings window should be readable");
            match current {
                DiffScrollSync::Both => DiffScrollSync::Vertical,
                DiffScrollSync::Vertical => DiffScrollSync::Horizontal,
                DiffScrollSync::Horizontal => DiffScrollSync::None,
                DiffScrollSync::None => DiffScrollSync::Both,
            }
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cx.update(|_window, app| {
                main_view.update(app, |_view, cx| {
                    let _ = settings_window.update(cx, |settings, _window, cx| {
                        settings.set_diff_scroll_sync(next_mode, cx);
                    });
                });
            });
        }));
        assert!(
            result.is_ok(),
            "diff scroll sync update should not re-enter GitCometView updates"
        );

        cx.run_until_parked();

        cx.update(|_window, app| {
            assert_eq!(main_view.read(app).diff_scroll_sync_for_test(), next_mode);
            assert_eq!(
                settings_window
                    .read_with(app, |settings, _cx| settings.diff_scroll_sync)
                    .expect("settings window should remain readable"),
                next_mode
            );
        });
    }

    #[gpui::test]
    fn ui_font_dropdown_wheel_scrolls_inner_list_before_outer_window(
        cx: &mut gpui::TestAppContext,
    ) {
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

        let synthetic_fonts: Arc<[String]> = (0..200)
            .map(|ix| format!("Test UI Font {ix:03}"))
            .collect::<Vec<_>>()
            .into();

        cx.update(|_window, app| {
            let _ = settings_window.update(app, |settings, _window, cx| {
                settings.ui_font_options = synthetic_fonts.clone();
                settings.ui_font_family = synthetic_fonts[0].clone();
                settings.expanded_section = Some(SettingsSection::UiFont);
                settings.settings_window_scroll = ScrollHandle::default();
                settings.ui_font_scroll = UniformListScrollHandle::default();
                cx.notify();
            });
        });

        let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
        settings_cx.run_until_parked();
        settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(460.0)));
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let list_bounds = settings_cx
            .debug_bounds("settings_window_ui_font_list_container")
            .expect("expected UI font list bounds");

        let (outer_before, inner_before, outer_max, inner_max) = settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                (
                    absolute_scroll_y(&settings.settings_window_scroll),
                    uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
                    settings
                        .settings_window_scroll
                        .max_offset()
                        .height
                        .max(px(0.0)),
                    uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).2,
                )
            })
            .expect("settings window should remain readable");
        assert!(
            outer_max > px(0.0),
            "expected the settings page to be scrollable during the test"
        );
        assert!(
            inner_max > px(0.0),
            "expected the UI font list to be scrollable during the test"
        );

        settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
        settings_cx.simulate_event(ScrollWheelEvent {
            position: list_bounds.center(),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
            ..Default::default()
        });
        settings_cx.run_until_parked();

        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let (outer_after_inner_scroll, inner_after_inner_scroll) = settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                (
                    absolute_scroll_y(&settings.settings_window_scroll),
                    uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
                )
            })
            .expect("settings window should remain readable");

        assert!(
            inner_after_inner_scroll > inner_before + px(0.5),
            "expected the UI font list to consume wheel scroll first"
        );
        assert!(
            (outer_after_inner_scroll - outer_before).abs() <= px(0.5),
            "expected the outer settings page to stay still while the UI font list can still scroll"
        );

        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            let (raw_offset, _scroll_offset, max_offset) =
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll);
            let current_x = settings.ui_font_scroll.0.borrow().base_handle.offset().x;
            let target_y = if raw_offset > px(0.0) {
                max_offset
            } else {
                -max_offset
            };
            settings
                .ui_font_scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(current_x, target_y));
            cx.notify();
        });
        settings_cx.run_until_parked();

        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let outer_before_boundary_handoff = settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                absolute_scroll_y(&settings.settings_window_scroll)
            })
            .expect("settings window should remain readable");

        settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
        settings_cx.simulate_event(ScrollWheelEvent {
            position: list_bounds.center(),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
            ..Default::default()
        });
        settings_cx.run_until_parked();

        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let outer_after_boundary_handoff = settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                absolute_scroll_y(&settings.settings_window_scroll)
            })
            .expect("settings window should remain readable");

        assert!(
            outer_after_boundary_handoff > outer_before_boundary_handoff + px(0.5),
            "expected wheel scrolling to bubble to the outer settings page once the UI font list reaches its boundary"
        );
    }
}
