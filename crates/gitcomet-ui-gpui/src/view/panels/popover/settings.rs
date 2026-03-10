use super::*;

const MIN_GIT_MAJOR: u32 = 2;
const MIN_GIT_MINOR: u32 = 53;
const GITHUB_URL: &str = "https://github.com/Auto-Explore/GitComet";
const LICENSE_URL: &str = "https://github.com/Auto-Explore/GitComet/blob/main/LICENSE-AGPL-3.0";
const LICENSE_NAME: &str = "AGPL-3.0";

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

#[derive(Clone, Debug)]
pub(super) struct SettingsRuntimeInfo {
    pub(super) git: GitRuntimeInfo,
    pub(super) app_version_display: SharedString,
    pub(super) operating_system: SharedString,
    pub(super) github_url: SharedString,
    pub(super) license_url: SharedString,
}

#[derive(Clone, Debug)]
pub(super) struct GitRuntimeInfo {
    pub(super) version_display: SharedString,
    pub(super) compatibility: GitCompatibility,
    pub(super) detail: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GitCompatibility {
    Supported,
    TooOld,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GitVersion {
    major: u32,
    minor: u32,
    patch: Option<u32>,
}

impl SettingsRuntimeInfo {
    pub(super) fn detect() -> Self {
        Self {
            git: detect_git_runtime_info(),
            app_version_display: app_version_display(),
            operating_system: format!(
                "{} ({}, {})",
                std::env::consts::OS,
                std::env::consts::FAMILY,
                std::env::consts::ARCH
            )
            .into(),
            github_url: GITHUB_URL.into(),
            license_url: LICENSE_URL.into(),
        }
    }
}

fn app_version_display() -> SharedString {
    format!("GitComet v{}", env!("CARGO_PKG_VERSION")).into()
}

fn detect_git_runtime_info() -> GitRuntimeInfo {
    let tested_only_message = format!(
        "GitComet has been tested only with Git {MIN_GIT_MAJOR}.{MIN_GIT_MINOR}. \
         Please use Git {MIN_GIT_MAJOR}.{MIN_GIT_MINOR} or newer."
    );

    match std::process::Command::new("git").arg("--version").output() {
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
                    detail: Some(tested_only_message.into()),
                };
            }

            let compatibility = match parse_git_version(&version_output) {
                Some(version) if is_supported_git_version(version) => GitCompatibility::Supported,
                Some(_) => GitCompatibility::TooOld,
                None => GitCompatibility::Unknown,
            };

            let detail = match compatibility {
                GitCompatibility::Supported => None,
                GitCompatibility::TooOld | GitCompatibility::Unknown => {
                    Some(tested_only_message.into())
                }
            };

            GitRuntimeInfo {
                version_display: version_output.into(),
                compatibility,
                detail,
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
                detail: Some(tested_only_message.into()),
            }
        }
        Err(err) => GitRuntimeInfo {
            version_display: format!("Unavailable ({err})").into(),
            compatibility: GitCompatibility::Unknown,
            detail: Some(tested_only_message.into()),
        },
    }
}

fn parse_git_version(raw: &str) -> Option<GitVersion> {
    raw.split_whitespace().find_map(parse_git_version_token)
}

fn parse_git_version_token(token: &str) -> Option<GitVersion> {
    let mut parts = token.split('.');
    let major = parse_u32_prefix(parts.next()?)?;
    let minor = parse_u32_prefix(parts.next()?)?;
    let patch = parts.next().and_then(parse_u32_prefix);
    Some(GitVersion {
        major,
        minor,
        patch,
    })
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

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let current_theme_mode = this.theme_mode;
    let current_format = this.date_time_format;
    let current_timezone = this.timezone;
    let show_timezone = this.show_timezone;
    let runtime = this.settings_runtime_info.clone();

    let row = |id: &'static str, label: &'static str, value: SharedString, open: bool| {
        div()
            .id(id)
            .w_full()
            .min_w_full()
            .max_w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active))
            .cursor(CursorStyle::PointingHand)
            .child(div().text_sm().child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(value)
                    .child(
                        div()
                            .font_family("monospace")
                            .child(if open { "▴" } else { "▾" }),
                    ),
            )
    };

    let toggle_row = |id: &'static str, label: &'static str, enabled: bool| {
        div()
            .id(id)
            .w_full()
            .min_w_full()
            .max_w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active))
            .cursor(CursorStyle::PointingHand)
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
    };

    let info_row = |id: &'static str, label: &'static str, value: SharedString| {
        div()
            .id(id)
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
                    .font_family("monospace")
                    .text_color(theme.colors.text_muted)
                    .child(value),
            )
    };

    let header = div()
        .px_2()
        .py_1()
        .text_sm()
        .font_weight(FontWeight::BOLD)
        .child("Settings");

    let section_label = div()
        .px_2()
        .pt(px(6.0))
        .pb(px(4.0))
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child("General");

    let content_bounds: std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let general_section_bounds: std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    let theme_anchor_bounds: std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let theme_anchor_bounds_for_prepaint = std::rc::Rc::clone(&theme_anchor_bounds);
    let theme_anchor_bounds_for_click = std::rc::Rc::clone(&theme_anchor_bounds);
    let content_bounds_for_theme_click = std::rc::Rc::clone(&content_bounds);
    let general_section_bounds_for_theme_click = std::rc::Rc::clone(&general_section_bounds);
    let theme_row = div()
        .flex()
        .w_full()
        .min_w_full()
        .max_w_full()
        .on_children_prepainted(move |children_bounds, _w, _cx| {
            if let Some(bounds) = children_bounds.first() {
                *theme_anchor_bounds_for_prepaint.borrow_mut() = Some(*bounds);
            }
        })
        .child(
            row(
                "settings_theme",
                "Theme",
                current_theme_mode.label().into(),
                this.settings_submenu == Some(SettingsSubmenu::Theme),
            )
            .flex_1()
            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                this.settings_submenu = if this.settings_submenu == Some(SettingsSubmenu::Theme) {
                    this.settings_submenu_top = None;
                    this.settings_submenu_left = None;
                    this.settings_submenu_width = None;
                    this.settings_submenu_max_h = None;
                    None
                } else {
                    if let (Some(row_bounds), Some(panel_bounds), Some(section_bounds)) = (
                        *theme_anchor_bounds_for_click.borrow(),
                        *content_bounds_for_theme_click.borrow(),
                        *general_section_bounds_for_theme_click.borrow(),
                    ) {
                        let submenu_width = section_bounds.size.width.min(px(240.0));
                        this.settings_submenu_top =
                            Some((row_bounds.bottom() - panel_bounds.top()) + px(1.0));
                        this.settings_submenu_left =
                            Some(section_bounds.right() - panel_bounds.left() - submenu_width);
                        this.settings_submenu_width = Some(submenu_width);
                        this.settings_submenu_max_h = Some(
                            ((panel_bounds.bottom() - row_bounds.bottom()) - px(12.0))
                                .max(px(120.0))
                                .min(px(280.0)),
                        );
                    } else {
                        this.settings_submenu_top = None;
                        this.settings_submenu_left = None;
                        this.settings_submenu_width = None;
                        this.settings_submenu_max_h = None;
                    }
                    Some(SettingsSubmenu::Theme)
                };
                cx.notify();
            })),
        );

    let date_format_anchor_bounds: std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let date_format_anchor_bounds_for_prepaint = std::rc::Rc::clone(&date_format_anchor_bounds);
    let date_format_anchor_bounds_for_click = std::rc::Rc::clone(&date_format_anchor_bounds);
    let content_bounds_for_date_click = std::rc::Rc::clone(&content_bounds);
    let general_section_bounds_for_date_click = std::rc::Rc::clone(&general_section_bounds);
    let date_row = div()
        .flex()
        .w_full()
        .min_w_full()
        .max_w_full()
        .on_children_prepainted(move |children_bounds, _w, _cx| {
            if let Some(bounds) = children_bounds.first() {
                *date_format_anchor_bounds_for_prepaint.borrow_mut() = Some(*bounds);
            }
        })
        .child(
            row(
                "settings_date_format",
                "Date format",
                current_format.label().into(),
                this.settings_submenu == Some(SettingsSubmenu::DateFormat),
            )
            .flex_1()
            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                this.settings_submenu =
                    if this.settings_submenu == Some(SettingsSubmenu::DateFormat) {
                        this.settings_submenu_top = None;
                        this.settings_submenu_left = None;
                        this.settings_submenu_width = None;
                        this.settings_submenu_max_h = None;
                        None
                    } else {
                        if let (Some(row_bounds), Some(panel_bounds), Some(section_bounds)) = (
                            *date_format_anchor_bounds_for_click.borrow(),
                            *content_bounds_for_date_click.borrow(),
                            *general_section_bounds_for_date_click.borrow(),
                        ) {
                            let submenu_width = section_bounds.size.width.min(px(320.0));
                            this.settings_submenu_top =
                                Some((row_bounds.bottom() - panel_bounds.top()) + px(1.0));
                            this.settings_submenu_left =
                                Some(section_bounds.right() - panel_bounds.left() - submenu_width);
                            this.settings_submenu_width = Some(submenu_width);
                            this.settings_submenu_max_h = Some(
                                ((panel_bounds.bottom() - row_bounds.bottom()) - px(12.0))
                                    .max(px(120.0))
                                    .min(px(280.0)),
                            );
                        } else {
                            this.settings_submenu_top = None;
                            this.settings_submenu_left = None;
                            this.settings_submenu_width = None;
                            this.settings_submenu_max_h = None;
                        }
                        Some(SettingsSubmenu::DateFormat)
                    };
                cx.notify();
            })),
        );

    let timezone_anchor_bounds: std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let timezone_anchor_bounds_for_prepaint = std::rc::Rc::clone(&timezone_anchor_bounds);
    let timezone_anchor_bounds_for_click = std::rc::Rc::clone(&timezone_anchor_bounds);
    let content_bounds_for_tz_click = std::rc::Rc::clone(&content_bounds);
    let general_section_bounds_for_tz_click = std::rc::Rc::clone(&general_section_bounds);
    let tz_row = div()
        .flex()
        .w_full()
        .min_w_full()
        .max_w_full()
        .on_children_prepainted(move |children_bounds, _w, _cx| {
            if let Some(bounds) = children_bounds.first() {
                *timezone_anchor_bounds_for_prepaint.borrow_mut() = Some(*bounds);
            }
        })
        .child(
            row(
                "settings_timezone",
                "Date timezone",
                current_timezone.label().into(),
                this.settings_submenu == Some(SettingsSubmenu::Timezone),
            )
            .flex_1()
            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                this.settings_submenu = if this.settings_submenu == Some(SettingsSubmenu::Timezone)
                {
                    this.settings_submenu_top = None;
                    this.settings_submenu_left = None;
                    this.settings_submenu_width = None;
                    this.settings_submenu_max_h = None;
                    None
                } else {
                    if let (Some(row_bounds), Some(panel_bounds), Some(section_bounds)) = (
                        *timezone_anchor_bounds_for_click.borrow(),
                        *content_bounds_for_tz_click.borrow(),
                        *general_section_bounds_for_tz_click.borrow(),
                    ) {
                        let submenu_width = section_bounds.size.width.min(px(420.0));
                        this.settings_submenu_top =
                            Some((row_bounds.bottom() - panel_bounds.top()) + px(1.0));
                        this.settings_submenu_left =
                            Some(section_bounds.right() - panel_bounds.left() - submenu_width);
                        this.settings_submenu_width = Some(submenu_width);
                        this.settings_submenu_max_h = Some(
                            ((panel_bounds.bottom() - row_bounds.bottom()) - px(12.0))
                                .max(px(120.0))
                                .min(px(280.0)),
                        );
                    } else {
                        this.settings_submenu_top = None;
                        this.settings_submenu_left = None;
                        this.settings_submenu_width = None;
                        this.settings_submenu_max_h = None;
                    }
                    Some(SettingsSubmenu::Timezone)
                };
                cx.notify();
            })),
        );

    let general_section_bounds_for_prepaint = std::rc::Rc::clone(&general_section_bounds);
    let general_section = div()
        .w_full()
        .min_w_full()
        .max_w_full()
        .on_children_prepainted(move |children_bounds, _w, _cx| {
            if let Some(bounds) = children_bounds.first() {
                *general_section_bounds_for_prepaint.borrow_mut() = Some(*bounds);
            }
        })
        .child(
            div()
                .w_full()
                .min_w_full()
                .max_w_full()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(theme_row)
                .child(date_row)
                .child(tz_row),
        );

    let show_timezone_row = toggle_row("settings_show_timezone", "Show timezone", show_timezone)
        .flex_1()
        .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            this.set_show_timezone(!this.show_timezone, cx);
            cx.notify();
        }));

    let mut content = div()
        .w_full()
        .min_w_full()
        .max_w_full()
        .w(px(760.0))
        .flex()
        .flex_col()
        .min_w(px(600.0))
        .max_w(px(760.0))
        .child(header)
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(section_label)
        .child(general_section)
        .child(
            div()
                .flex()
                .w_full()
                .min_w_full()
                .max_w_full()
                .pb_1()
                .child(show_timezone_row),
        );

    let environment_section_label = div()
        .px_2()
        .pt(px(6.0))
        .pb(px(4.0))
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child("Environment");

    let (git_icon, git_icon_color, git_status_text): (&'static str, gpui::Rgba, SharedString) =
        match runtime.git.compatibility {
            GitCompatibility::Supported => ("✓", theme.colors.success, "Git >= 2.53".into()),
            GitCompatibility::TooOld => ("⚠", theme.colors.warning, "Git < 2.53".into()),
            GitCompatibility::Unknown => ("⚠", theme.colors.warning, "Git version unknown".into()),
        };

    let git_row = div()
        .id("settings_git_version")
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
                .child(
                    div()
                        .font_family("monospace")
                        .text_sm()
                        .text_color(git_icon_color)
                        .child(git_icon),
                )
                .child(
                    div()
                        .font_family("monospace")
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child(runtime.git.version_display.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(git_icon_color)
                        .child(git_status_text),
                ),
        );

    let app_version_row = info_row(
        "settings_app_version",
        "Build",
        runtime.app_version_display.clone(),
    );
    let os_row = info_row(
        "settings_os_info",
        "Operating system",
        runtime.operating_system.clone(),
    );
    let github_row = div()
        .id("settings_github_link")
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .cursor(CursorStyle::PointingHand)
        .child(div().text_sm().child("GitHub"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(theme.colors.accent)
                .child(runtime.github_url.clone())
                .child(div().font_family("monospace").child("↗")),
        )
        .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            let url = this.settings_runtime_info.github_url.clone().to_string();
            match this.open_external_url(&url) {
                Ok(()) => this.push_toast(
                    components::ToastKind::Success,
                    "Opened GitHub repository in your browser.".to_string(),
                    cx,
                ),
                Err(err) => this.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open browser: {err}"),
                    cx,
                ),
            }
            cx.notify();
        }));

    let license_row = div()
        .id("settings_license_link")
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .cursor(CursorStyle::PointingHand)
        .child(div().text_sm().child("License"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(theme.colors.accent)
                .child(LICENSE_NAME)
                .child(div().font_family("monospace").child("↗")),
        )
        .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            let url = this.settings_runtime_info.license_url.clone().to_string();
            match this.open_external_url(&url) {
                Ok(()) => this.push_toast(
                    components::ToastKind::Success,
                    "Opened license in your browser.".to_string(),
                    cx,
                ),
                Err(err) => this.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open browser: {err}"),
                    cx,
                ),
            }
            cx.notify();
        }));

    let open_source_licenses_row = div()
        .id("settings_open_source_licenses")
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .cursor(CursorStyle::PointingHand)
        .child(div().text_sm().child("Open source licenses"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(theme.colors.accent)
                .child("Show")
                .child(div().font_family("monospace").child("↗")),
        )
        .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
            this.open_popover_at(
                PopoverKind::OpenSourceLicenses,
                crate::view::chrome::window_top_left_corner(window),
                window,
                cx,
            );
        }));

    content = content.child(environment_section_label).child(
        div()
            .px_2()
            .pb_1()
            .flex()
            .flex_col()
            .gap_1()
            .child(app_version_row)
            .child(git_row)
            .child(os_row)
            .child(github_row)
            .child(license_row)
            .child(open_source_licenses_row),
    );

    if let Some(detail) = runtime.git.detail.clone() {
        content = content.child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.warning)
                .child(detail),
        );
    }

    let content_bounds_for_prepaint = std::rc::Rc::clone(&content_bounds);
    let mut panel = div()
        .relative()
        .on_children_prepainted(move |children_bounds, _w, _cx| {
            if let Some(bounds) = children_bounds.first() {
                *content_bounds_for_prepaint.borrow_mut() = Some(*bounds);
            }
        })
        .child(content);

    if let (
        Some(menu),
        Some(overlay_top),
        Some(overlay_left),
        Some(overlay_width),
        Some(overlay_max_h),
    ) = (
        this.settings_submenu,
        this.settings_submenu_top,
        this.settings_submenu_left,
        this.settings_submenu_width,
        this.settings_submenu_max_h,
    ) {
        let menu_kind = match menu {
            SettingsSubmenu::Theme => PopoverKind::SettingsThemeMenu,
            SettingsSubmenu::DateFormat => PopoverKind::SettingsDateFormatMenu,
            SettingsSubmenu::Timezone => PopoverKind::SettingsTimezoneMenu,
        };

        panel = panel.child(
            div()
                .id("settings_submenu")
                .absolute()
                .top(overlay_top)
                .left(overlay_left)
                .w(overlay_width)
                .min_w(overlay_width)
                .max_w(overlay_width)
                .occlude()
                .bg(theme.colors.surface_bg_elevated)
                .border_1()
                .border_color(with_alpha(theme.colors.accent, 0.90))
                .rounded(px(theme.radii.panel))
                .shadow_lg()
                .overflow_hidden()
                .p_1()
                .child(
                    div()
                        .id("settings_submenu_scroll")
                        .min_h(px(0.0))
                        .max_h(overlay_max_h)
                        .overflow_y_scroll()
                        .child(
                            this.context_menu_view(menu_kind, cx)
                                .w_full()
                                .min_w_full()
                                .max_w_full(),
                        ),
                ),
        );
    }

    components::context_menu(theme, panel)
}

#[cfg(test)]
mod tests {
    use super::{
        GitVersion, MIN_GIT_MAJOR, MIN_GIT_MINOR, app_version_display, is_supported_git_version,
        parse_git_version,
    };

    #[test]
    fn parse_git_version_extracts_semver_from_standard_output() {
        let parsed = parse_git_version("git version 2.53.1").expect("parsed");
        assert_eq!(
            parsed,
            GitVersion {
                major: 2,
                minor: 53,
                patch: Some(1)
            }
        );
    }

    #[test]
    fn parse_git_version_handles_windows_suffix_output() {
        let parsed = parse_git_version("git version 2.45.1.windows.1").expect("parsed");
        assert_eq!(
            parsed,
            GitVersion {
                major: 2,
                minor: 45,
                patch: Some(1)
            }
        );
    }

    #[test]
    fn supported_version_requires_minimum_2_53() {
        assert!(is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR,
            minor: MIN_GIT_MINOR,
            patch: Some(0)
        }));
        assert!(!is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR,
            minor: MIN_GIT_MINOR - 1,
            patch: Some(9)
        }));
        assert!(is_supported_git_version(GitVersion {
            major: MIN_GIT_MAJOR + 1,
            minor: 0,
            patch: Some(0)
        }));
    }

    #[test]
    fn app_version_display_uses_package_version() {
        let expected = format!("GitComet v{}", env!("CARGO_PKG_VERSION"));
        assert_eq!(app_version_display().as_ref(), expected);
    }
}
