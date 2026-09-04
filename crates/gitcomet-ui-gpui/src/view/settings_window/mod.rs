use super::*;
use crate::ui_scale;
use gitcomet_core::domain::HistoryMode;
use gitcomet_core::process::{
    GitExecutablePreference, GitRuntimeState, install_git_executable_path, refresh_git_runtime,
};
use gitcomet_state::model::{DefaultTagType, GitLogTagFetchMode};
use gitcomet_state::session::ExternalCodeEditorSetting;
use gpui::{Stateful, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

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
const SETTINGS_TRAFFIC_LIGHTS_SAFE_INSET_PX: f32 = 78.0;
const MIN_GIT_MAJOR: u32 = 2;
const MIN_GIT_MINOR: u32 = 50;
const GITHUB_URL: &str = "https://github.com/Auto-Explore/GitComet";
const THEMES_GUIDE_URL: &str = "https://github.com/Auto-Explore/GitComet/blob/main/docs/themes.md";
const LICENSE_URL: &str = "https://github.com/Auto-Explore/GitComet/blob/main/LICENSE-AGPL-3.0";
const LICENSE_NAME: &str = "AGPL-3.0";

#[derive(Clone, Default)]
struct ExternalEditorPreferencePersistQueue {
    latest_sequence: Arc<AtomicU64>,
    write_lock: Arc<Mutex<()>>,
}

impl ExternalEditorPreferencePersistQueue {
    fn next_sequence(&self) -> u64 {
        self.latest_sequence
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    fn persist_if_latest(
        &self,
        sequence: u64,
        setting: Option<ExternalCodeEditorSetting>,
    ) -> std::io::Result<bool> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if self.latest_sequence.load(Ordering::Acquire) != sequence {
            return Ok(false);
        }
        session::persist_ui_settings(external_editor_preference_settings(setting))?;
        Ok(true)
    }

    #[cfg(test)]
    fn persist_to_path_if_latest(
        &self,
        sequence: u64,
        setting: Option<ExternalCodeEditorSetting>,
        path: &std::path::Path,
    ) -> std::io::Result<bool> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if self.latest_sequence.load(Ordering::Acquire) != sequence {
            return Ok(false);
        }
        session::persist_ui_settings_to_path(external_editor_preference_settings(setting), path)?;
        Ok(true)
    }
}

static EXTERNAL_EDITOR_PREFERENCE_PERSIST_QUEUE: OnceLock<ExternalEditorPreferencePersistQueue> =
    OnceLock::new();

fn external_editor_preference_persist_queue() -> &'static ExternalEditorPreferencePersistQueue {
    EXTERNAL_EDITOR_PREFERENCE_PERSIST_QUEUE.get_or_init(Default::default)
}

fn external_editor_preference_settings(
    setting: Option<ExternalCodeEditorSetting>,
) -> session::UiSettings {
    session::UiSettings {
        external_code_editor: Some(setting),
        ..session::UiSettings::default()
    }
}

fn custom_external_editor_path_prompt_options() -> gpui::PathPromptOptions {
    gpui::PathPromptOptions {
        files: true,
        directories: true,
        multiple: false,
        prompt: Some("Select external code editor".into()),
    }
}

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

const DIFF_CONTENT_MODE_OPTIONS: &[(&str, DiffContentMode, &str)] = &[
    (
        "settings_window_diff_content_mode_collapsed",
        DiffContentMode::Collapsed,
        "Hide unchanged sections, with hunk controls to reveal more context.",
    ),
    (
        "settings_window_diff_content_mode_full",
        DiffContentMode::Full,
        "Show the full file using the regular file diff view.",
    ),
];

const DIFF_VIEW_MODE_OPTIONS: &[(&str, DiffViewMode, &str)] = &[
    (
        "settings_window_diff_view_mode_inline",
        DiffViewMode::Inline,
        "Show changes inline.",
    ),
    (
        "settings_window_diff_view_mode_split",
        DiffViewMode::Split,
        "Show changes in split view.",
    ),
];

const REMOTE_MARKDOWN_IMAGE_OPTIONS: &[(&str, RemoteMarkdownImagePolicy, &str)] = &[
    (
        "settings_window_remote_markdown_images_always",
        RemoteMarkdownImagePolicy::AlwaysLoad,
        "Load HTTP and HTTPS images automatically.",
    ),
    (
        "settings_window_remote_markdown_images_ask",
        RemoteMarkdownImagePolicy::AskBeforeLoading,
        "Prompt before loading; approve one image or all images in the current preview.",
    ),
    (
        "settings_window_remote_markdown_images_never",
        RemoteMarkdownImagePolicy::NeverLoad,
        "Never request or display HTTP and HTTPS images.",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsSection {
    Theme,
    UiScale,
    UiFont,
    EditorFont,
    ExternalCodeEditor,
    DateFormat,
    Timezone,
    TerminalExternal,
    TerminalActionBar,
    ChangeTracking,
    DiffContentMode,
    Diff,
    DiffViewMode,
    GitLogDefaultMode,
    GitLogColumns,
    GitLogTagFetch,
    RemoteMarkdownImages,
}

impl SettingsSection {
    /// The left-nav category that owns this expandable section. Expanding a
    /// section always happens from within its owning category's page, so this
    /// mapping keeps the visible page and the expanded row in sync.
    fn category(self) -> SettingsCategory {
        match self {
            Self::Theme
            | Self::UiScale
            | Self::UiFont
            | Self::EditorFont
            | Self::ExternalCodeEditor
            | Self::DateFormat
            | Self::Timezone => SettingsCategory::General,
            Self::TerminalExternal | Self::TerminalActionBar => SettingsCategory::Terminal,
            Self::ChangeTracking => SettingsCategory::ChangeTracking,
            Self::DiffContentMode | Self::Diff | Self::DiffViewMode => SettingsCategory::Diff,
            Self::GitLogDefaultMode | Self::GitLogColumns | Self::GitLogTagFetch => {
                SettingsCategory::GitLog
            }
            Self::RemoteMarkdownImages => SettingsCategory::SecurityPrivacy,
        }
    }
}

/// A top-level settings grouping, shown as a row in the left-hand navigation.
/// Each category maps to one of the existing settings cards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsCategory {
    General,
    SecurityPrivacy,
    Terminal,
    ChangeTracking,
    Diff,
    FileEditing,
    GitLog,
    Remotes,
    Tags,
    GitExecutable,
    Environment,
    Links,
}

impl SettingsCategory {
    const ALL: &'static [SettingsCategory] = &[
        SettingsCategory::General,
        SettingsCategory::SecurityPrivacy,
        SettingsCategory::Terminal,
        SettingsCategory::ChangeTracking,
        SettingsCategory::Diff,
        SettingsCategory::FileEditing,
        SettingsCategory::GitLog,
        SettingsCategory::Remotes,
        SettingsCategory::Tags,
        SettingsCategory::GitExecutable,
        SettingsCategory::Environment,
        SettingsCategory::Links,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::SecurityPrivacy => "Security / Privacy",
            Self::Terminal => "Terminal",
            Self::ChangeTracking => "Change tracking",
            Self::Diff => "Diff",
            Self::FileEditing => "File editing",
            Self::GitLog => "Git log",
            Self::Remotes => "Remotes",
            Self::Tags => "Tags",
            Self::GitExecutable => "Git executable",
            Self::Environment => "Environment",
            Self::Links => "Links",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => "icons/cog.svg",
            Self::SecurityPrivacy => "icons/file_icons/lock.svg",
            Self::Terminal => "icons/terminal.svg",
            Self::ChangeTracking => "icons/file.svg",
            Self::Diff => "icons/swap.svg",
            Self::FileEditing => "icons/pencil.svg",
            Self::GitLog => "icons/history.svg",
            Self::Remotes => "icons/cloud.svg",
            Self::Tags => "icons/tag.svg",
            Self::GitExecutable => "icons/git_branch.svg",
            Self::Environment => "icons/computer.svg",
            Self::Links => "icons/link.svg",
        }
    }

    fn nav_id(self) -> &'static str {
        match self {
            Self::General => "settings_window_nav_general",
            Self::SecurityPrivacy => "settings_window_nav_security_privacy",
            Self::Terminal => "settings_window_nav_terminal",
            Self::ChangeTracking => "settings_window_nav_change_tracking",
            Self::Diff => "settings_window_nav_diff",
            Self::FileEditing => "settings_window_nav_file_editing",
            Self::GitLog => "settings_window_nav_git_log",
            Self::Remotes => "settings_window_nav_remotes",
            Self::Tags => "settings_window_nav_tags",
            Self::GitExecutable => "settings_window_nav_git_executable",
            Self::Environment => "settings_window_nav_environment",
            Self::Links => "settings_window_nav_links",
        }
    }

    /// Lowercase text (title plus the labels of the settings on the page) used
    /// to decide whether a category matches the nav search query.
    fn search_haystack(self) -> &'static str {
        match self {
            Self::General => {
                "general theme date format ui scale ui font editor font ligatures \
                 external code editor date timezone appearance"
            }
            Self::SecurityPrivacy => {
                "security privacy remote markdown images load image tracking pixels updates \
                 automatically check updates startup"
            }
            Self::Terminal => "terminal external terminal action bar terminal button opens",
            Self::ChangeTracking => "change tracking untracked files",
            Self::Diff => {
                "diff mode scroll sync show whitespace changes reveal whitespace characters \
                 word wrap show line numbers unified split"
            }
            Self::FileEditing => {
                "file editing edit file auto save autosave save automatically editor"
            }
            Self::GitLog => {
                "git log default history mode history columns relative dates show tags graph \
                 author sha"
            }
            Self::Remotes => "remotes remote fetch pull prune deleted branches automatically ghost",
            Self::Tags => "tags automatically fetch tags",
            Self::GitExecutable => "git executable custom path system path version",
            Self::Environment => "environment build operating system app version",
            Self::Links => {
                "links theme guide github license open source licenses professional edition \
                 waitlist"
            }
        }
    }

    fn matches_query(self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        self.search_haystack().contains(query.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsView {
    Root,
    OpenSourceLicenses,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GitExecutableMode {
    SystemPath,
    Custom,
}

impl GitExecutableMode {
    fn from_preference(preference: &GitExecutablePreference) -> Self {
        match preference {
            GitExecutablePreference::SystemPath => Self::SystemPath,
            GitExecutablePreference::Custom(_) => Self::Custom,
        }
    }
}

/// Where the External code editor dropdown's option list stands.
///
/// Detecting installed editors probes every PATH directory and walks IDE
/// install roots (on Windows `Program Files\JetBrains`), which measured 2.6 s
/// on the main thread when it ran in the constructor. The summary row only
/// needs the saved setting, so the scan is deferred until the row is expanded
/// and runs on a background thread; the dropdown shows a spinner until then.
enum ExternalEditorOptionsState {
    NotLoaded,
    /// Keeps the result-delivery future alive while this view exists. The
    /// blocking detector itself may finish after the view is dropped because
    /// `smol::unblock` jobs are not cancellable once running.
    Loading {
        _task: gpui::Task<()>,
    },
    Loaded,
}

pub(crate) struct SettingsWindowView {
    theme_mode: ThemeMode,
    theme: AppTheme,
    ui_scale_percent: u32,
    ui_font_family: String,
    editor_font_family: String,
    use_font_ligatures: bool,
    ui_font_options: Arc<[String]>,
    editor_font_options: Arc<[String]>,
    external_editor_options: Arc<[crate::external_editor::ExternalEditorOption]>,
    external_editor_options_state: ExternalEditorOptionsState,
    settings_window_scroll: ScrollHandle,
    theme_scroll: UniformListScrollHandle,
    ui_font_scroll: UniformListScrollHandle,
    editor_font_scroll: UniformListScrollHandle,
    external_editor_scroll: UniformListScrollHandle,
    date_format_scroll: UniformListScrollHandle,
    timezone_scroll: UniformListScrollHandle,
    change_tracking_scroll: UniformListScrollHandle,
    diff_content_mode_scroll: UniformListScrollHandle,
    diff_scroll_sync_scroll: UniformListScrollHandle,
    diff_view_mode_scroll: UniformListScrollHandle,
    remote_markdown_images_scroll: UniformListScrollHandle,
    date_time_format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    change_tracking_view: ChangeTrackingView,
    terminal_preferences: TerminalPreferences,
    terminal_external_program_input: Entity<components::TextInput>,
    terminal_external_args_input: Entity<components::TextInput>,
    terminal_status: Option<TerminalSettingsStatus>,
    diff_content_mode: DiffContentMode,
    diff_whitespace_mode: DiffWhitespaceMode,
    diff_view_mode: DiffViewMode,
    diff_reveal_whitespace_chars: bool,
    diff_word_wrap: bool,
    diff_show_line_numbers: bool,
    auto_save_file_edits: bool,
    remote_markdown_image_policy: RemoteMarkdownImagePolicy,
    check_for_updates_on_startup: bool,
    diff_scroll_sync: DiffScrollSync,
    history_show_graph: bool,
    history_show_author: bool,
    history_show_date: bool,
    history_show_sha: bool,
    history_relative_dates: bool,
    history_highlight_commit_chain: bool,
    history_show_tags: bool,
    history_tag_fetch_mode: GitLogTagFetchMode,
    default_history_mode: HistoryMode,
    default_tag_type: DefaultTagType,
    prune_deleted_remote_branches_on_fetch: bool,
    current_view: SettingsView,
    selected_category: SettingsCategory,
    search_query: String,
    search_input: Entity<components::TextInput>,
    nav_scroll: ScrollHandle,
    open_source_licenses_scroll: UniformListScrollHandle,
    runtime_info: SettingsRuntimeInfo,
    git_executable_mode: GitExecutableMode,
    git_custom_path_draft: String,
    git_executable_input: Entity<components::TextInput>,
    external_editor_setting: Option<ExternalCodeEditorSetting>,
    external_editor_custom_path_draft: String,
    external_editor_custom_arguments_draft: String,
    external_editor_custom_path_input: Entity<components::TextInput>,
    external_editor_custom_arguments_input: Entity<components::TextInput>,
    expanded_section: Option<SettingsSection>,
    hover_resize_edge: Option<ResizeEdge>,
    title_drag_state: chrome::TitleBarDragState,
    _git_executable_input_subscription: gpui::Subscription,
    _external_editor_custom_path_input_subscription: gpui::Subscription,
    _external_editor_custom_arguments_input_subscription: gpui::Subscription,
    _appearance_subscription: gpui::Subscription,
    _search_input_subscription: gpui::Subscription,
    #[cfg(test)]
    overflow_probe: bool,
    #[cfg(test)]
    external_editor_browse_notify_count: usize,
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

    let ui_session = session::load();
    let ui_scale = ui_scale::current_or_initialize_from_session(&ui_session, cx);
    let bounds = Bounds::centered(
        None,
        settings_window_default_size_for_percent(ui_scale.percent),
        cx,
    );
    let ui_scale_percent = ui_scale.percent;
    cx.open_window(
        settings_window_options_for_scale(bounds, ui_scale_percent),
        move |window, cx| {
            ui_scale::apply_to_window(window, ui_scale_percent);
            window.on_window_should_close(cx, |window, cx| {
                crate::app::mark_clean_shutdown_if_last_window(cx);
                window.remove_window();
                false
            });
            cx.new(|cx| SettingsWindowView::new(window, cx))
        },
    )
    .expect("failed to open settings window");

    cx.activate(true);
}

fn settings_window_min_size_for_percent(percent: u32) -> gpui::Size<Pixels> {
    ui_scale::design_size_from_percent(
        SETTINGS_WINDOW_MIN_WIDTH_PX,
        SETTINGS_WINDOW_MIN_HEIGHT_PX,
        percent,
    )
}

fn settings_window_default_size_for_percent(percent: u32) -> gpui::Size<Pixels> {
    ui_scale::design_size_from_percent(
        SETTINGS_WINDOW_DEFAULT_WIDTH_PX,
        SETTINGS_WINDOW_DEFAULT_HEIGHT_PX,
        percent,
    )
}

fn settings_window_traffic_light_position(_percent: u32) -> Point<Pixels> {
    point(px(9.0), px(9.0))
}

fn settings_window_traffic_lights_safe_inset(_percent: u32) -> Pixels {
    px(SETTINGS_TRAFFIC_LIGHTS_SAFE_INSET_PX)
}

#[cfg(test)]
fn settings_window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    settings_window_options_for_scale(bounds, ui_scale::DEFAULT_UI_SCALE_PERCENT)
}

fn settings_window_options_for_scale(
    bounds: Bounds<Pixels>,
    ui_scale_percent: u32,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(settings_window_min_size_for_percent(ui_scale_percent)),
        titlebar: Some(settings_window_titlebar_options_for_scale(ui_scale_percent)),
        app_id: Some("gitcomet-settings".into()),
        window_decorations: Some(WindowDecorations::Client),
        window_background: crate::app::main_window_background_appearance(),
        is_movable: true,
        is_resizable: true,
        ..Default::default()
    }
}

#[cfg(test)]
fn settings_window_titlebar_options() -> TitlebarOptions {
    settings_window_titlebar_options_for_scale(ui_scale::DEFAULT_UI_SCALE_PERCENT)
}

fn settings_window_titlebar_options_for_scale(ui_scale_percent: u32) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(SETTINGS_WINDOW_TITLE.into()),
        // Windows needs a transparent native titlebar to avoid rendering its own
        // caption on top of the custom settings header.
        appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
        traffic_light_position: cfg!(target_os = "macos")
            .then_some(settings_window_traffic_light_position(ui_scale_percent)),
    }
}

#[cfg(test)]
fn settings_window_client_inset() -> Pixels {
    settings_window_client_inset_for_scale(ui_scale::DEFAULT_UI_SCALE_PERCENT)
}

fn settings_window_client_inset_for_scale(ui_scale_percent: u32) -> Pixels {
    if cfg!(target_os = "windows") {
        px(0.0)
    } else {
        chrome::client_side_decoration_inset(ui_scale_percent)
    }
}

fn settings_window_frame(
    theme: AppTheme,
    decorations: Decorations,
    content: AnyElement,
    ui_scale_percent: u32,
) -> AnyElement {
    if cfg!(target_os = "windows") {
        content
    } else {
        chrome::window_frame(theme, decorations, content, None, ui_scale_percent)
    }
}

fn uniform_list_vertical_wheel_delta(event: &gpui::ScrollWheelEvent, window: &Window) -> Pixels {
    event.delta.pixel_delta(window.line_height()).y
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
        .unwrap_or_else(|| state.base_handle.max_offset().y.max(px(0.0)));
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
    gpui::Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}

fn settings_row_separator_color(theme: AppTheme) -> gpui::Rgba {
    mix_color(
        theme.colors.surface.canvas,
        theme.colors.stroke.subtle,
        if theme.is_dark { 0.14 } else { 0.10 },
    )
}

fn settings_dropdown_background(theme: AppTheme) -> gpui::Rgba {
    if theme.is_dark {
        mix_color(
            theme.colors.surface.raised,
            theme.colors.surface.canvas,
            0.58,
        )
    } else {
        mix_color(
            theme.colors.surface.raised,
            theme.colors.stroke.default,
            0.55,
        )
    }
}

fn settings_dropdown_border_color(theme: AppTheme) -> gpui::Rgba {
    if theme.is_dark {
        with_alpha(theme.colors.stroke.default, 0.98)
    } else {
        theme.colors.stroke.default
    }
}

fn settings_dropdown_height(
    item_count: usize,
    estimated_row_height_px: f32,
    extra_height_px: f32,
    ui_scale_percent: u32,
) -> Pixels {
    ui_scale::design_px_from_percent(
        (((item_count.max(1) as f32) * estimated_row_height_px) + extra_height_px)
            .min(SETTINGS_DROPDOWN_LIST_MAX_HEIGHT_PX),
        ui_scale_percent,
    )
}

/// The theme rows, labels included, from a single pass over the theme list.
///
/// `ThemeMode::label` resolves a key by re-reading the user theme directory --
/// a `create_dir_all`, a `read_dir`, and a `metadata` per file, all of it ahead
/// of the memo that is supposed to make it cheap -- and the row processor below
/// runs on every layout pass while the dropdown is open. Taking the label off
/// the same `ThemeOption` the mode is built from spends that once per render
/// instead of once per visible row per frame.
fn settings_theme_mode_options() -> Vec<(ThemeMode, SharedString)> {
    let themes = crate::theme::available_themes();
    let mut options = Vec::with_capacity(themes.len() + 1);
    options.push((
        ThemeMode::Automatic,
        SharedString::from(ThemeMode::Automatic.label()),
    ));
    options.extend(
        themes
            .into_iter()
            .map(|theme| (ThemeMode::Named(theme.key), SharedString::from(theme.label))),
    );
    options
}

fn settings_theme_modes() -> Vec<ThemeMode> {
    settings_theme_mode_options()
        .into_iter()
        .map(|(mode, _)| mode)
        .collect()
}

fn history_columns_settings_label(
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
) -> SharedString {
    let mut columns = Vec::new();
    if show_graph {
        columns.push("Graph");
    }
    if show_author {
        columns.push("Author");
    }
    if show_date {
        columns.push("Commit date");
    }
    if show_sha {
        columns.push("SHA");
    }

    if columns.is_empty() {
        "None".into()
    } else {
        columns.join(", ").into()
    }
}

fn git_log_tag_fetch_mode_label(mode: GitLogTagFetchMode) -> &'static str {
    match mode {
        GitLogTagFetchMode::OnRepositoryActivation => "On repository activation",
        GitLogTagFetchMode::Disabled => "Disabled",
    }
}

fn applied_git_executable_path(runtime: &GitRuntimeState) -> Option<PathBuf> {
    match &runtime.preference {
        GitExecutablePreference::SystemPath => None,
        GitExecutablePreference::Custom(path) => Some(path.clone()),
    }
}

fn git_executable_scope_note() -> &'static str {
    "Applies to the main GitComet browser window. Git-invoked command modes keep using git from System PATH. Helper tools such as gpg are resolved by Git from the app environment unless configured in Git."
}

fn initial_external_editor_setting(
    ui_session: &session::UiSession,
) -> Option<ExternalCodeEditorSetting> {
    crate::external_editor::configured_setting_preference_override()
        .unwrap_or_else(|| ui_session.external_code_editor.clone())
}

impl SettingsWindowView {
    fn new(window: &mut Window, cx: &mut gpui::Context<Self>) -> Self {
        window.set_window_title(SETTINGS_WINDOW_TITLE);

        let ui_session = session::load();
        let ui_preferences = UiPreferences::from_session(&ui_session);
        let ui_scale = ui_scale::current_or_initialize_from_session(&ui_session, cx);
        let font_preferences =
            crate::font_preferences::current_or_initialize_from_session(window, &ui_session, cx);
        let theme_mode = ui_preferences.appearance.theme_mode.clone();
        let date_time_format = ui_preferences.appearance.date_time_format;
        let timezone = ui_preferences.appearance.timezone;
        let show_timezone = ui_preferences.appearance.show_timezone;
        let change_tracking_view = ui_preferences.change_tracking.view;
        let terminal_preferences = ui_preferences.terminal.clone();
        let diff_scroll_sync = ui_preferences.diff.scroll_sync;
        let diff_content_mode = ui_preferences.diff.content_mode;
        let diff_whitespace_mode = ui_preferences.diff.whitespace_mode;
        let diff_view_mode = ui_preferences.diff.view_mode;
        let diff_reveal_whitespace_chars = ui_preferences.diff.reveal_whitespace_chars;
        let diff_word_wrap = ui_preferences.diff.word_wrap;
        let diff_show_line_numbers = ui_preferences.diff.show_line_numbers;
        let auto_save_file_edits = ui_preferences.file_editing.auto_save;
        let remote_markdown_image_policy = ui_preferences.security.remote_markdown_images;
        let check_for_updates_on_startup = ui_preferences.security.check_for_updates_on_startup;
        let history_show_graph = ui_preferences.history.show_graph;
        let history_show_author = ui_preferences.history.show_author;
        let history_show_date = ui_preferences.history.show_date;
        let history_show_sha = ui_preferences.history.show_sha;
        let history_relative_dates = ui_preferences.history.relative_dates;
        let history_highlight_commit_chain = ui_preferences.history.highlight_commit_chain;
        let history_show_tags = ui_preferences.history.show_tags;
        let history_tag_fetch_mode = ui_preferences.history.tag_fetch_mode;
        let default_history_mode = ui_preferences.history.default_mode;
        let default_tag_type = ui_preferences.repository.default_tag_type;
        let prune_deleted_remote_branches_on_fetch = ui_preferences
            .remotes
            .prune_deleted_remote_branches_on_fetch;
        let external_editor_setting = initial_external_editor_setting(&ui_session);
        // Only the saved editor's entry is needed to render the summary row;
        // installed editors are detected once the row is expanded, see
        // `ensure_external_editor_options_loaded`.
        let external_editor_options: Arc<[crate::external_editor::ExternalEditorOption]> =
            crate::external_editor::external_editor_options_from_detected(
                external_editor_setting.as_ref(),
                Vec::new(),
            )
            .into();
        let (external_editor_custom_path_draft, external_editor_custom_arguments_draft) =
            match &external_editor_setting {
                Some(ExternalCodeEditorSetting::Custom {
                    executable,
                    arguments,
                }) => (
                    executable.display().to_string(),
                    arguments.clone().unwrap_or_default(),
                ),
                _ => (String::new(), String::new()),
            };
        let theme = theme_mode.resolve_theme(window.appearance());
        let runtime_info = SettingsRuntimeInfo::detect();
        let git_executable_mode =
            GitExecutableMode::from_preference(&runtime_info.git.runtime.preference);
        let git_custom_path_draft = match &runtime_info.git.runtime.preference {
            GitExecutablePreference::Custom(path) if !path.as_os_str().is_empty() => {
                path.display().to_string()
            }
            _ => String::new(),
        };

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

        let terminal_external_program_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "wezterm".into(),
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_theme(theme, cx);
            input.set_text(terminal_preferences.external_terminal_program.clone(), cx);
            input
        });

        let terminal_external_args_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "One argument per line".into(),
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_theme(theme, cx);
            input.set_line_height(Some(px(20.0)), cx);
            input.set_text(terminal_preferences.external_args_multiline(), cx);
            input
        });

        let git_executable_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        git_executable_input.update(cx, |input, cx| {
            input.set_text(git_custom_path_draft.clone(), cx);
        });
        let git_executable_input_subscription =
            cx.observe(&git_executable_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let next = input.read(cx).text().to_string();
                if this.git_custom_path_draft != next {
                    this.git_custom_path_draft = next;
                    cx.notify();
                }
                if enter_pressed && this.git_executable_mode == GitExecutableMode::Custom {
                    this.apply_git_executable_settings(cx);
                }
            });

        let external_editor_custom_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/editor".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        external_editor_custom_path_input.update(cx, |input, cx| {
            input.set_text(external_editor_custom_path_draft.clone(), cx);
        });
        let external_editor_custom_arguments_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "--reuse-window {path}".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        external_editor_custom_arguments_input.update(cx, |input, cx| {
            input.set_text(external_editor_custom_arguments_draft.clone(), cx);
        });
        let external_editor_custom_path_input_subscription =
            cx.observe(&external_editor_custom_path_input, |this, input, cx| {
                let next = input.read(cx).text().to_string();
                if this.external_editor_custom_path_draft == next {
                    return;
                }
                this.external_editor_custom_path_draft = next;
                if this.external_editor_is_custom() {
                    this.persist_external_editor_from_custom_drafts(cx);
                }
                cx.notify();
            });
        let search_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Search".into(),
                    leading_icon: Some("icons/zoom.svg"),
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_theme(theme, cx);
            input
        });
        let search_input_subscription = cx.observe(&search_input, |this, input, cx| {
            let next = input.read(cx).text().to_string();
            if this.search_query == next {
                return;
            }
            this.search_query = next;
            // Keep the visible page in the filtered set: if the current
            // category no longer matches, jump to the first one that does.
            if !this.selected_category.matches_query(&this.search_query)
                && let Some(first) = SettingsCategory::ALL
                    .iter()
                    .copied()
                    .find(|category| category.matches_query(&this.search_query))
            {
                this.selected_category = first;
                this.expanded_section = None;
            }
            cx.notify();
        });

        let external_editor_custom_arguments_input_subscription = cx.observe(
            &external_editor_custom_arguments_input,
            |this, input, cx| {
                let next = input.read(cx).text().to_string();
                if this.external_editor_custom_arguments_draft == next {
                    return;
                }
                this.external_editor_custom_arguments_draft = next;
                if this.external_editor_is_custom() {
                    this.persist_external_editor_from_custom_drafts(cx);
                }
                cx.notify();
            },
        );

        Self {
            theme_mode,
            theme,
            ui_scale_percent: ui_scale.percent,
            ui_font_family: font_preferences.ui_font_family,
            editor_font_family: font_preferences.editor_font_family,
            use_font_ligatures: font_preferences.use_font_ligatures,
            ui_font_options: crate::font_preferences::ui_font_options(window),
            editor_font_options: crate::font_preferences::editor_font_options(window),
            external_editor_options,
            external_editor_options_state: ExternalEditorOptionsState::NotLoaded,
            settings_window_scroll: ScrollHandle::default(),
            theme_scroll: UniformListScrollHandle::default(),
            ui_font_scroll: UniformListScrollHandle::default(),
            editor_font_scroll: UniformListScrollHandle::default(),
            external_editor_scroll: UniformListScrollHandle::default(),
            date_format_scroll: UniformListScrollHandle::default(),
            timezone_scroll: UniformListScrollHandle::default(),
            change_tracking_scroll: UniformListScrollHandle::default(),
            diff_content_mode_scroll: UniformListScrollHandle::default(),
            diff_scroll_sync_scroll: UniformListScrollHandle::default(),
            diff_view_mode_scroll: UniformListScrollHandle::default(),
            remote_markdown_images_scroll: UniformListScrollHandle::default(),
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            terminal_preferences,
            terminal_external_program_input,
            terminal_external_args_input,
            terminal_status: None,
            diff_content_mode,
            diff_whitespace_mode,
            diff_view_mode,
            diff_reveal_whitespace_chars,
            diff_word_wrap,
            diff_show_line_numbers,
            auto_save_file_edits,
            remote_markdown_image_policy,
            check_for_updates_on_startup,
            diff_scroll_sync,
            history_show_graph,
            history_show_author,
            history_show_date,
            history_show_sha,
            history_relative_dates,
            history_highlight_commit_chain,
            history_show_tags,
            history_tag_fetch_mode,
            default_history_mode,
            default_tag_type,
            prune_deleted_remote_branches_on_fetch,
            current_view: SettingsView::Root,
            selected_category: SettingsCategory::General,
            search_query: String::new(),
            search_input,
            nav_scroll: ScrollHandle::default(),
            open_source_licenses_scroll: UniformListScrollHandle::default(),
            runtime_info,
            git_executable_mode,
            git_custom_path_draft,
            git_executable_input,
            external_editor_setting,
            external_editor_custom_path_draft,
            external_editor_custom_arguments_draft,
            external_editor_custom_path_input,
            external_editor_custom_arguments_input,
            expanded_section: None,
            hover_resize_edge: None,
            title_drag_state: chrome::TitleBarDragState::default(),
            _git_executable_input_subscription: git_executable_input_subscription,
            _external_editor_custom_path_input_subscription:
                external_editor_custom_path_input_subscription,
            _external_editor_custom_arguments_input_subscription:
                external_editor_custom_arguments_input_subscription,
            _appearance_subscription: appearance_subscription,
            _search_input_subscription: search_input_subscription,
            #[cfg(test)]
            overflow_probe: false,
            #[cfg(test)]
            external_editor_browse_notify_count: 0,
        }
    }

    fn select_category(&mut self, category: SettingsCategory, cx: &mut gpui::Context<Self>) {
        if self.selected_category == category {
            return;
        }
        self.selected_category = category;
        // Collapse any expanded row so the new page starts clean, and scroll
        // the content pane back to the top.
        self.set_expanded_section(None, cx);
        self.settings_window_scroll
            .set_offset(gpui::point(px(0.0), px(0.0)));
        cx.notify();
    }

    fn toggle_section(&mut self, section: SettingsSection, cx: &mut gpui::Context<Self>) {
        let next = if self.expanded_section == Some(section) {
            None
        } else {
            Some(section)
        };
        self.set_expanded_section(next, cx);
    }

    fn set_expanded_section(
        &mut self,
        section: Option<SettingsSection>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.expanded_section == section {
            return;
        }
        self.expanded_section = section;
        if section == Some(SettingsSection::ExternalCodeEditor) {
            self.ensure_external_editor_options_loaded(cx);
        }
        cx.notify();
    }

    /// Whether the External code editor dropdown is still waiting for the
    /// installed-editor scan; it shows a static placeholder meanwhile.
    pub(super) fn external_editor_options_loading(&self) -> bool {
        !matches!(
            self.external_editor_options_state,
            ExternalEditorOptionsState::Loaded
        )
    }

    /// Start the installed-editor scan the first time the dropdown needs it.
    /// The scan runs on a background thread, and the option list is rebuilt
    /// against the setting current at completion, so a change made in the
    /// meantime is not clobbered.
    fn ensure_external_editor_options_loaded(&mut self, cx: &mut gpui::Context<Self>) {
        if !matches!(
            self.external_editor_options_state,
            ExternalEditorOptionsState::NotLoaded
        ) {
            return;
        }
        // The deterministic runtime (tests) has no background thread to wait
        // for, so the list is complete by the time the expanded row draws.
        if !crate::ui_runtime::current().uses_background_compute() {
            let detected = crate::external_editor::detect_external_editors();
            self.apply_detected_external_editors(detected, cx);
            return;
        }
        let task = crate::ui_runtime::run_background_compute(
            cx,
            crate::external_editor::detect_external_editors,
            |this, cx, detected| this.apply_detected_external_editors(detected, cx),
        );
        self.external_editor_options_state = ExternalEditorOptionsState::Loading { _task: task };
    }

    fn apply_detected_external_editors(
        &mut self,
        detected: Vec<crate::external_editor::DetectedExternalEditor>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.external_editor_options =
            crate::external_editor::external_editor_options_from_detected(
                self.external_editor_setting.as_ref(),
                detected,
            )
            .into();
        self.external_editor_options_state = ExternalEditorOptionsState::Loaded;
        cx.notify();
    }
}

mod prefs;
mod render;
mod rows;
mod runtime;

use runtime::*;

#[cfg(test)]
mod tests;
