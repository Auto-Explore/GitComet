use crate::assets::GitCometAssets;
use crate::launch_guard::{UiLaunchError, run_with_panic_guard};
use crate::ui_scale;
use crate::view::{
    DiffNextFile, DiffNextSearchMatchOrChange, DiffPrevFile, DiffPrevSearchMatchOrChange,
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, InitialRepositoryLaunchMode, LocateFileInExplorer, MainPaneView,
    OpenActiveViewSearch, OpenRemoteInBrowser, PopoverPromptDismiss, PopoverPromptTabNext,
    PopoverPromptTabPrev, PushUpstreamRemoteClose, PushUpstreamRemoteNext,
    PushUpstreamRemoteOpenOrSelect, PushUpstreamRemotePrev, SettingsWindowView, StartupCrashReport,
    TerminalCopy, TerminalPaste, TerminalSelectAll, TextInputCommitSubmit, TextInputDiffNextChange,
    TextInputDiffNextFile, TextInputDiffNextSearchMatchOrChange, TextInputDiffPrevChange,
    TextInputDiffPrevFile, TextInputDiffPrevSearchMatchOrChange, ToggleCommandPalette,
    WorkspaceBootstrap, is_diff_shortcut_candidate,
};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitBackend;
use gitcomet_state::session;
use gitcomet_state::store::AppStore;

use gpui::{
    Action, App, AppContext, BorrowAppContext, Bounds, DisplayId, KeyBinding, Pixels, Point, Size,
    TitlebarOptions, Unbind, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowOptions, actions, point, px, size,
};
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem, OsAction, SystemMenuType};
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;
use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(target_os = "macos")]
use schemars::JsonSchema;
#[cfg(target_os = "macos")]
use serde::Deserialize;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

const WINDOW_MIN_WIDTH_PX: f32 = 820.0;
const WINDOW_MIN_HEIGHT_PX: f32 = 560.0;
const WINDOW_DEFAULT_WIDTH_PX: f32 = 1280.0;
const WINDOW_DEFAULT_HEIGHT_PX: f32 = 800.0;
const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
#[cfg(test)]
const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

actions!(
    app_menu,
    [
        NewWindow,
        OpenSettings,
        OpenInCodeEditor,
        OpenRepository,
        CloneRepository,
        InitializeRepository,
        SwitchRepository,
        OpenWorkspace,
        ApplyPatch,
        CheckForUpdates,
        ShowReflog,
        Close,
        CloseWindow,
        PreviousRepository,
        NextRepository,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        IncreaseUiScale,
        DecreaseUiScale,
        ResetUiScale,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
    ]
);

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, Action)]
#[action(namespace = app_menu)]
#[serde(deny_unknown_fields)]
struct OpenRecentRepository {
    storage_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusedMergetoolConfig {
    pub repo_path: PathBuf,
    pub conflicted_file_path: PathBuf,
    pub label_local: String,
    pub label_remote: String,
    pub label_base: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiRunOutcome {
    CleanShutdown,
    /// Retained for API compatibility. A successfully returned event loop is
    /// now classified as a clean shutdown.
    UnexpectedEventLoopExit,
}

#[derive(Clone, Debug)]
struct WindowLaunchConfig {
    title: String,
    app_id: String,
    view_config: GitCometViewConfig,
    /// How an initial browser request should be routed after saved workspaces have
    /// been restored. Constructors keep the historical existing-window
    /// behavior unless the executable explicitly carries a different choice.
    browser_open_target: BrowserOpenTarget,
}

#[derive(Clone)]
struct CleanShutdownTracker {
    requested: Arc<AtomicBool>,
}

impl Default for CleanShutdownTracker {
    fn default() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl gpui::Global for CleanShutdownTracker {}

#[derive(Clone)]
struct GitCometBackendGlobal(Arc<dyn GitBackend>);

impl gpui::Global for GitCometBackendGlobal {}

type ShutdownCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrowserOpenTarget {
    #[default]
    ExistingWindow,
    NewWindow,
}

impl BrowserOpenTarget {
    pub const ALL: [Self; 2] = [Self::ExistingWindow, Self::NewWindow];

    pub const fn key(self) -> &'static str {
        match self {
            Self::ExistingWindow => "existing_window",
            Self::NewWindow => "new_window",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "existing_window" => Some(Self::ExistingWindow),
            "new_window" => Some(Self::NewWindow),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ExistingWindow => "Active window",
            Self::NewWindow => "New window",
        }
    }

    pub const fn detail(self) -> &'static str {
        match self {
            Self::ExistingWindow => "Add the repository to the active GitComet window.",
            Self::NewWindow => "Create a separate window for the repository.",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserOpenRequest {
    pub path: Option<PathBuf>,
    pub target: BrowserOpenTarget,
}

pub(crate) fn main_window_min_size_for_percent(percent: u32) -> Size<Pixels> {
    ui_scale::design_size_from_percent(WINDOW_MIN_WIDTH_PX, WINDOW_MIN_HEIGHT_PX, percent)
}

fn main_window_default_size_for_percent(percent: u32) -> Size<Pixels> {
    ui_scale::design_size_from_percent(WINDOW_DEFAULT_WIDTH_PX, WINDOW_DEFAULT_HEIGHT_PX, percent)
}

/// Shrinks a default window size to the primary display, never below `min_size`.
pub(crate) fn fit_default_window_size(
    default_size: Size<Pixels>,
    min_size: Size<Pixels>,
    cx: &App,
) -> Size<Pixels> {
    let Some(visible) = cx
        .primary_display()
        .map(|display| display.visible_bounds().size)
    else {
        return default_size;
    };
    size(
        default_size.width.min(visible.width).max(min_size.width),
        default_size.height.min(visible.height).max(min_size.height),
    )
}

pub(crate) fn ensure_window_respects_min_size(window: &mut Window, min_size: Size<Pixels>) {
    let current = window.viewport_size();
    let next = size(
        current.width.max(min_size.width),
        current.height.max(min_size.height),
    );
    if next != current {
        window.resize(next);
    }
}

pub fn run(backend: Arc<dyn GitBackend>) -> Result<(), UiLaunchError> {
    run_with_startup_crash_report(backend, None, None).map(|_| ())
}

pub fn run_with_startup_crash_report(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
) -> Result<UiRunOutcome, UiLaunchError> {
    run_with_startup_crash_report_and_shutdown_callback(
        backend,
        initial_path,
        startup_crash_report,
        None::<fn()>,
    )
}

/// Runs the main window and invokes `on_shutdown` from GPUI's graceful-shutdown
/// callback. On Windows GPUI terminates with `ExitProcess`, so callers cannot
/// rely on [`run_with_startup_crash_report`] returning to perform cleanup.
pub fn run_with_startup_crash_report_and_shutdown_callback(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
    on_shutdown: Option<impl Fn() + Send + Sync + 'static>,
) -> Result<UiRunOutcome, UiLaunchError> {
    run_with_startup_crash_report_shutdown_callback_and_browser_requests(
        backend,
        initial_path,
        startup_crash_report,
        on_shutdown,
        None,
    )
}

/// Runs the browser and accepts repository-open requests forwarded by another
/// GitComet process. The caller owns the single-instance transport; keeping the
/// wire protocol outside the UI crate makes it independently testable.
pub fn run_with_startup_crash_report_shutdown_callback_and_browser_requests(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
    on_shutdown: Option<impl Fn() + Send + Sync + 'static>,
    browser_requests: Option<smol::channel::Receiver<BrowserOpenRequest>>,
) -> Result<UiRunOutcome, UiLaunchError> {
    run_with_startup_crash_report_shutdown_callback_and_initial_browser_request(
        backend,
        BrowserOpenRequest {
            path: initial_path,
            target: BrowserOpenTarget::ExistingWindow,
        },
        startup_crash_report,
        on_shutdown,
        browser_requests,
    )
}

/// Runs the browser while retaining the routing preference attached to the
/// process's own initial request. This matters on a cold start: saved workspaces
/// are restored before the path is routed, just like a forwarded request.
pub fn run_with_startup_crash_report_shutdown_callback_and_initial_browser_request(
    backend: Arc<dyn GitBackend>,
    initial_request: BrowserOpenRequest,
    startup_crash_report: Option<StartupCrashReport>,
    on_shutdown: Option<impl Fn() + Send + Sync + 'static>,
    browser_requests: Option<smol::channel::Receiver<BrowserOpenRequest>>,
) -> Result<UiRunOutcome, UiLaunchError> {
    let mut launch = normal_launch_config(initial_request.path, startup_crash_report);
    launch.browser_open_target = initial_request.target;
    ensure_graphics_device_available("main GPUI window launch")?;
    let on_shutdown = on_shutdown.map(|callback| Arc::new(callback) as ShutdownCallback);
    run_with_panic_guard("main GPUI window launch", move || {
        run_windowed_app(
            backend,
            launch,
            CleanShutdownTracker::default(),
            on_shutdown,
            browser_requests,
        )
    })?;
    // A native abort or forced process termination cannot return from the GPUI
    // event loop. Reaching this point is therefore a clean shutdown even when a
    // platform-specific close path did not set CleanShutdownTracker.
    Ok(UiRunOutcome::CleanShutdown)
}

/// Launch the unified focused mergetool window using the shared `GitCometView`.
pub fn run_focused_mergetool(backend: Arc<dyn GitBackend>, config: FocusedMergetoolConfig) -> i32 {
    if let Err(err) = ensure_graphics_device_available("focused mergetool GPUI launch") {
        eprintln!("Failed to launch focused mergetool window: {err}");
        return FOCUSED_MERGETOOL_EXIT_ERROR;
    }

    let exit_code = Arc::new(AtomicI32::new(FOCUSED_MERGETOOL_EXIT_CANCELED));
    let launch = focused_mergetool_launch_config(&config, Some(exit_code.clone()));
    if let Err(err) = run_with_panic_guard("focused mergetool GPUI launch", move || {
        run_windowed_app(backend, launch, CleanShutdownTracker::default(), None, None)
    }) {
        eprintln!("Failed to launch focused mergetool window: {err}");
        return FOCUSED_MERGETOOL_EXIT_ERROR;
    }
    exit_code.load(Ordering::SeqCst)
}

fn normal_launch_config(
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    let mut view_config = GitCometViewConfig::normal(startup_crash_report);
    view_config.initial_path = initial_path;
    WindowLaunchConfig {
        title: "GitComet".to_string(),
        app_id: "gitcomet".to_string(),
        view_config,
        browser_open_target: BrowserOpenTarget::ExistingWindow,
    }
}

fn normal_launch_config_with_initial_repository(
    initial_path: PathBuf,
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    WindowLaunchConfig {
        title: "GitComet".to_string(),
        app_id: "gitcomet".to_string(),
        view_config: GitCometViewConfig::normal_with_initial_repository(
            initial_path,
            startup_crash_report,
        ),
        browser_open_target: BrowserOpenTarget::ExistingWindow,
    }
}

fn normal_empty_launch_config(
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    let mut launch = normal_launch_config(None, startup_crash_report);
    launch.view_config.workspace = WorkspaceBootstrap::Empty;
    launch
}

fn launch_config_for_workspace(
    base: &WindowLaunchConfig,
    workspace: session::Workspace,
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    let mut launch = base.clone();
    launch.title = format!("{} — GitComet", workspace.display_name());
    launch.view_config.initial_path = None;
    launch.view_config.initial_repository_launch_mode = InitialRepositoryLaunchMode::RestoreSession;
    launch.view_config.startup_crash_report = startup_crash_report;
    launch.view_config.workspace = WorkspaceBootstrap::Saved(Box::new(workspace));
    launch
}

fn focused_mergetool_launch_config(
    config: &FocusedMergetoolConfig,
    exit_code: Option<Arc<AtomicI32>>,
) -> WindowLaunchConfig {
    WindowLaunchConfig {
        title: focused_mergetool_window_title(&config.conflicted_file_path),
        app_id: "gitcomet-mergetool".to_string(),
        view_config: GitCometViewConfig {
            initial_path: Some(config.repo_path.clone()),
            initial_repository_launch_mode: InitialRepositoryLaunchMode::RestoreSession,
            view_mode: GitCometViewMode::FocusedMergetool,
            focused_mergetool: Some(FocusedMergetoolViewConfig {
                repo_path: config.repo_path.clone(),
                conflicted_file_path: config.conflicted_file_path.clone(),
                labels: FocusedMergetoolLabels {
                    local: config.label_local.clone(),
                    remote: config.label_remote.clone(),
                    base: config.label_base.clone(),
                },
            }),
            focused_mergetool_exit_code: exit_code,
            startup_crash_report: None,
            workspace: WorkspaceBootstrap::LegacySession,
        },
        browser_open_target: BrowserOpenTarget::ExistingWindow,
    }
}

fn focused_mergetool_window_title(conflicted_file_path: &Path) -> String {
    let display = conflicted_file_path
        .file_name()
        .and_then(|name| name.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{conflicted_file_path:?}"));
    format!("GitComet - Mergetool ({display})")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowZoomAction {
    Zoom,
    Restore,
}

pub(crate) fn window_zoom_action(is_maximized: bool) -> WindowZoomAction {
    if cfg!(target_os = "windows") && is_maximized {
        WindowZoomAction::Restore
    } else {
        WindowZoomAction::Zoom
    }
}

pub(crate) fn toggle_window_zoom(window: &Window) {
    match window_zoom_action(window.is_maximized()) {
        WindowZoomAction::Zoom => window.zoom_window(),
        WindowZoomAction::Restore => {
            #[cfg(target_os = "windows")]
            if restore_maximized_window(window) {
                return;
            }

            window.zoom_window();
        }
    }
}

pub(crate) fn show_window_system_menu(window: &Window, position: Point<Pixels>) {
    #[cfg(target_os = "windows")]
    if show_windows_window_system_menu(window, position) {
        return;
    }

    window.show_window_menu(position);
}

pub(crate) fn application() -> gpui::Application {
    gpui_platform::application()
}

#[cfg(any(target_os = "windows", test))]
fn window_menu_position(position: Point<Pixels>, scale_factor: f32) -> (i32, i32) {
    (
        (f32::from(position.x) * scale_factor).round() as i32,
        (f32::from(position.y) * scale_factor).round() as i32,
    )
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowSystemMenuRequest {
    pub hwnd: isize,
    pub x: i32,
    pub y: i32,
}

#[cfg(target_os = "windows")]
fn window_hwnd(window: &Window) -> Option<isize> {
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return None;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };

    Some(handle.hwnd.get())
}

thread_local! {
    /// When we last asked the compositor/window manager for an interactive move
    /// or resize grab.
    static LAST_WINDOW_GRAB_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Record that we just requested an interactive move/resize grab.
///
/// The grab is executed by the compositor/WM, which takes the input focus for
/// the duration of the drag. GPUI surfaces that as a plain window deactivate →
/// activate pair — on Wayland via `wl_keyboard` Leave/Enter, on X11 via
/// FocusOut/FocusIn (which are not filtered on `mode`, so NotifyGrab and
/// NotifyUngrab arrive too) — indistinguishable from the user alt-tabbing away
/// and back. This marker lets the activation observer ignore its own echo
/// instead of treating it as a return to the app and refreshing the repo.
pub(crate) fn note_window_grab_started() {
    LAST_WINDOW_GRAB_AT.with(|cell| cell.set(Some(Instant::now())));
}

/// Whether a grab was requested no more than `max_age` ago. Always consumes the
/// marker, so a grab the compositor silently dropped cannot arm suppression for
/// an unrelated activation minutes later.
pub(crate) fn take_window_grab_started_within(now: Instant, max_age: Duration) -> bool {
    LAST_WINDOW_GRAB_AT.with(|cell| match cell.take() {
        Some(at) => now.saturating_duration_since(at) <= max_age,
        None => false,
    })
}

/// Hand the title-bar drag to the platform. On Windows this goes through GPUI's
/// `start_window_move` (a posted `WM_NCLBUTTONDOWN`/`HTCAPTION`) rather than the
/// `SC_MOVE` system command GitComet used to post itself: GPUI tracks that drag
/// and synthesizes the `WM_LBUTTONUP` the modal move loop swallows, so the app
/// sees a complete press/release pair after every move, and the native
/// restore-on-drag for maximized windows works the same either way.
pub(crate) fn begin_window_move(window: &Window) {
    note_window_grab_started();
    window.start_window_move();
}

pub(crate) fn begin_window_resize(window: &Window, edge: gpui::ResizeEdge) {
    note_window_grab_started();
    window.start_window_resize(edge);
}

#[cfg(target_os = "windows")]
fn restore_maximized_window(window: &Window) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };

    // GPUI's Windows zoom path currently maps directly to SW_MAXIMIZE, so
    // restore must go through the native Win32 API until upstream toggles.
    gitcomet_win32_window_utils::restore_window(hwnd)
}

#[cfg(target_os = "windows")]
fn show_windows_window_system_menu(window: &Window, position: Point<Pixels>) -> bool {
    let Some(request) = window_system_menu_request(window, position) else {
        return false;
    };

    gitcomet_win32_window_utils::show_window_system_menu(request.hwnd, request.x, request.y);
    true
}

#[cfg(target_os = "windows")]
pub(crate) fn window_system_menu_request(
    window: &Window,
    position: Point<Pixels>,
) -> Option<WindowSystemMenuRequest> {
    let (x, y) = window_menu_position(position, window.scale_factor());
    let hwnd = window_hwnd(window)?;
    Some(WindowSystemMenuRequest { hwnd, x, y })
}

fn run_windowed_app(
    backend: Arc<dyn GitBackend>,
    launch: WindowLaunchConfig,
    clean_shutdown_tracker: CleanShutdownTracker,
    on_shutdown: Option<ShutdownCallback>,
    browser_requests: Option<smol::channel::Receiver<BrowserOpenRequest>>,
) {
    let quit_when_all_windows_closed = should_quit_when_all_windows_closed(&launch);
    // Without this, `gpui` keeps its null client and every request — the
    // update check, and images a markdown preview points at — fails silently.
    let application = application()
        .with_assets(GitCometAssets)
        .with_http_client(crate::http::client());

    #[cfg(target_os = "macos")]
    let open_urls_rx = if launch.view_config.view_mode == GitCometViewMode::Normal {
        let (open_urls_tx, open_urls_rx) = smol::channel::unbounded::<Vec<String>>();
        application.on_open_urls(move |urls| {
            let _ = open_urls_tx.try_send(urls);
        });
        Some(open_urls_rx)
    } else {
        None
    };

    #[cfg(target_os = "macos")]
    if launch.view_config.view_mode == GitCometViewMode::Normal {
        let reopen_backend = Arc::clone(&backend);
        application.on_reopen(move |cx: &mut App| {
            if cx.windows().is_empty() {
                let reopen_launch = normal_empty_launch_config(None);
                open_gitcomet_window(cx, Arc::clone(&reopen_backend), &reopen_launch);
                cx.activate(true);
            }
        });
    }

    application.run(move |cx: &mut App| {
        cx.set_global(clean_shutdown_tracker);
        crate::ui_probe::start_if_enabled(cx);
        crate::environment::initialize(cx);
        cx.set_global(GitCometBackendGlobal(Arc::clone(&backend)));
        cx.on_app_quit(move |cx| {
            flush_open_workspace_environments(cx);
            crate::workspaces::flush_to_disk(cx);
            if let Some(on_shutdown) = on_shutdown.as_ref() {
                on_shutdown();
            }
            async {}
        })
        .detach();
        if let Err(err) = crate::bundled_fonts::register(cx) {
            eprintln!("Failed to register bundled fonts: {err:#}");
        }
        if quit_when_all_windows_closed {
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        }

        if launch.view_config.view_mode == GitCometViewMode::Normal {
            bind_app_keys(cx);
            install_app_actions(cx, Arc::clone(&backend));
            if let Some(browser_requests) = browser_requests {
                register_browser_open_request_handler(cx, Arc::clone(&backend), browser_requests);
            }

            #[cfg(target_os = "macos")]
            {
                install_macos_app_menu(cx, Arc::clone(&backend));
                cx.on_window_closed(|cx, _| {
                    cx.defer(refresh_macos_app_menus);
                })
                .detach();
                if let Some(open_urls_rx) = open_urls_rx {
                    register_macos_open_request_handler(cx, Arc::clone(&backend), open_urls_rx);
                }
            }
        }
        bind_text_input_keys(cx);
        bind_terminal_keys(cx);

        open_initial_gitcomet_windows(cx, Arc::clone(&backend), &launch);
        crate::view::scenario_driver::start_if_requested(cx);

        cx.activate(true);
    });
}

fn open_initial_gitcomet_windows(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    launch: &WindowLaunchConfig,
) {
    if launch.view_config.view_mode != GitCometViewMode::Normal {
        open_gitcomet_window(cx, backend, launch);
        return;
    }

    let ui_session = session::load();
    let all_workspaces = ui_session.workspaces.clone();
    crate::workspaces::initialize(cx, all_workspaces.clone());
    open_initial_gitcomet_windows_after_workspace_initialization(
        cx,
        backend,
        launch,
        all_workspaces,
    );
}

/// The startup sequence after the durable workspace manager has been initialized.
/// Split out so tests can provide an in-memory workspace manager and never touch a
/// developer's real session file.
fn open_initial_gitcomet_windows_after_workspace_initialization(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    launch: &WindowLaunchConfig,
    all_workspaces: Vec<session::Workspace>,
) {
    let mut restorable_workspaces: Vec<_> = all_workspaces
        .into_iter()
        .filter(|workspace| workspace.restore_on_launch)
        .collect();
    restorable_workspaces.sort_by_key(|workspace| workspace.last_activation_order);

    let requested_repository = launch.view_config.initial_path.clone();
    let restored_any_workspace = !restorable_workspaces.is_empty();
    // Empty customized workspaces open on Home; a command-line path still
    // belongs in one of those rather than a further window.
    let restored_any_repository = restorable_workspaces
        .iter()
        .any(|workspace| !workspace.repositories.is_empty());
    let startup_window = if !restored_any_workspace {
        let mut initial_launch = launch.clone();
        initial_launch.view_config.workspace = WorkspaceBootstrap::Empty;
        Some(open_gitcomet_window(
            cx,
            Arc::clone(&backend),
            &initial_launch,
        ))
    } else {
        let startup_crash_report = launch.view_config.startup_crash_report.clone();
        let final_workspace_index = restorable_workspaces.len().saturating_sub(1);
        let mut startup_window = None;
        for (index, workspace) in restorable_workspaces.into_iter().enumerate() {
            let report = (index == final_workspace_index)
                .then(|| startup_crash_report.clone())
                .flatten();
            let workspace_launch = launch_config_for_workspace(launch, workspace, report);
            startup_window = Some(open_gitcomet_window(
                cx,
                Arc::clone(&backend),
                &workspace_launch,
            ));
        }
        // Activation is asynchronous on Linux. Keep the last-activated
        // workspace's handle as the routing target until focus catches up.
        if let Some(window) = startup_window {
            activate_gitcomet_window(cx, window.into());
        }
        startup_window
    };

    if let Some(path) = requested_repository {
        handle_browser_open_request_with_window(
            cx,
            backend,
            BrowserOpenRequest {
                path: Some(path),
                // With no restored repository, the startup window is the
                // natural destination even when future forwarded opens are
                // configured to create new windows.
                target: if restored_any_repository {
                    launch.browser_open_target
                } else {
                    BrowserOpenTarget::ExistingWindow
                },
            },
            startup_window.map(|window| window.window_id()),
        );
    }

    run_process_startup_hooks_once(cx, startup_window.map(|window| window.window_id()));
}

fn should_quit_when_all_windows_closed(launch: &WindowLaunchConfig) -> bool {
    launch.view_config.view_mode == GitCometViewMode::FocusedMergetool || !cfg!(target_os = "macos")
}

fn frame_from_bounds(bounds: Bounds<Pixels>) -> session::SavedWindowFrame {
    let x: f32 = bounds.origin.x.into();
    let y: f32 = bounds.origin.y.into();
    let width: f32 = bounds.size.width.into();
    let height: f32 = bounds.size.height.into();
    session::SavedWindowFrame {
        x: x.round() as i32,
        y: y.round() as i32,
        width: width.round().max(1.0) as u32,
        height: height.round().max(1.0) as u32,
    }
}

fn bounds_from_frame(frame: session::SavedWindowFrame) -> Bounds<Pixels> {
    Bounds::new(
        point(px(frame.x as f32), px(frame.y as f32)),
        size(px(frame.width as f32), px(frame.height as f32)),
    )
}

fn captured_normal_frame(
    state: session::SavedWindowState,
    reported_frame: session::SavedWindowFrame,
    previous: Option<&session::PortableWindowPlacement>,
) -> session::SavedWindowFrame {
    match state {
        session::SavedWindowState::Windowed => reported_frame,
        session::SavedWindowState::Maximized | session::SavedWindowState::Fullscreen => previous
            .and_then(|placement| placement.normal_frame)
            .unwrap_or(reported_frame),
    }
}

fn display_uuid(display: &dyn gpui::PlatformDisplay) -> Option<String> {
    display.uuid().ok().map(|uuid| uuid.to_string())
}

fn restored_workspace_window_bounds(
    placement: &session::PortableWindowPlacement,
    fallback_size: Size<Pixels>,
    min_size: Size<Pixels>,
    cx: &mut App,
) -> (WindowBounds, Option<DisplayId>) {
    let displays = cx.displays();
    let display = placement
        .display_id
        .as_deref()
        .and_then(|wanted| {
            displays
                .iter()
                .find(|display| display_uuid(display.as_ref()).as_deref() == Some(wanted))
                .cloned()
        })
        .or_else(|| cx.primary_display())
        .or_else(|| displays.first().cloned());
    let display_id = display.as_ref().map(|display| display.id());

    let Some(saved_frame) = placement.normal_frame else {
        return (
            WindowBounds::Windowed(Bounds::centered(display_id, fallback_size, cx)),
            display_id,
        );
    };
    let current_visible = display
        .as_ref()
        .map(|display| frame_from_bounds(display.visible_bounds()))
        .unwrap_or(saved_frame);
    let minimum_width: f32 = min_size.width.into();
    let minimum_height: f32 = min_size.height.into();
    let frame = crate::workspaces::rebase_window_frame(
        saved_frame,
        placement.captured_visible_frame,
        current_visible,
        minimum_width.round().max(1.0) as u32,
        minimum_height.round().max(1.0) as u32,
    );
    let occupied_frames = cx
        .windows()
        .into_iter()
        .filter_map(|handle| {
            handle
                .update(cx, |_root, window, cx| {
                    let window_display = window.display(cx).map(|display| display.id());
                    (
                        window_display,
                        frame_from_bounds(window.window_bounds().get_bounds()),
                    )
                })
                .ok()
        })
        .filter_map(|(window_display, frame)| (window_display == display_id).then_some(frame))
        .collect::<Vec<_>>();
    let frame = cascade_colliding_window_frame(frame, current_visible, &occupied_frames);
    let bounds = bounds_from_frame(frame);
    let bounds = match placement.state {
        session::SavedWindowState::Windowed => WindowBounds::Windowed(bounds),
        session::SavedWindowState::Maximized => WindowBounds::Maximized(bounds),
        session::SavedWindowState::Fullscreen => WindowBounds::Fullscreen(bounds),
    };
    (bounds, display_id)
}

fn cascade_colliding_window_frame(
    frame: session::SavedWindowFrame,
    visible: session::SavedWindowFrame,
    occupied: &[session::SavedWindowFrame],
) -> session::SavedWindowFrame {
    let collides = |candidate: session::SavedWindowFrame| {
        occupied
            .iter()
            .any(|other| other.x == candidate.x && other.y == candidate.y)
    };
    if !collides(frame) {
        return frame;
    }

    let min_x = visible.x;
    let min_y = visible.y;
    let max_x = visible
        .x
        .saturating_add(visible.width.saturating_sub(frame.width) as i32);
    let max_y = visible
        .y
        .saturating_add(visible.height.saturating_sub(frame.height) as i32);
    const CASCADE_OFFSET: i32 = 28;

    for step in 1_i32..=64 {
        let delta = CASCADE_OFFSET.saturating_mul(step);
        let offsets = [
            (delta, delta),
            (-delta, delta),
            (delta, -delta),
            (-delta, -delta),
            (delta, 0),
            (0, delta),
            (-delta, 0),
            (0, -delta),
        ];
        for (dx, dy) in offsets {
            let candidate = session::SavedWindowFrame {
                x: frame.x.saturating_add(dx).clamp(min_x, max_x),
                y: frame.y.saturating_add(dy).clamp(min_y, max_y),
                ..frame
            };
            if candidate != frame && !collides(candidate) {
                return candidate;
            }
        }
    }

    // A display with no travel (for example a window as large as its usable
    // area) cannot be cascaded without violating the on-screen clamp.
    frame
}

pub(crate) fn capture_window_placement<C>(
    window: &Window,
    cx: &C,
    previous: Option<&session::PortableWindowPlacement>,
) -> session::PortableWindowPlacement
where
    C: std::borrow::Borrow<App>,
{
    let app = cx.borrow();
    let display = window.display(app);
    let state = if window.is_fullscreen() {
        session::SavedWindowState::Fullscreen
    } else if window.is_maximized() {
        session::SavedWindowState::Maximized
    } else {
        session::SavedWindowState::Windowed
    };
    let reported_frame = frame_from_bounds(window.window_bounds().get_bounds());
    let normal_frame = captured_normal_frame(state, reported_frame, previous);
    let tiled = match window.window_decorations() {
        gpui::Decorations::Client { tiling }
            if tiling.top || tiling.left || tiling.right || tiling.bottom =>
        {
            Some(session::SavedWindowTiling {
                top: tiling.top,
                left: tiling.left,
                right: tiling.right,
                bottom: tiling.bottom,
            })
        }
        _ => None,
    };
    session::PortableWindowPlacement {
        normal_frame: Some(normal_frame),
        captured_visible_frame: display
            .as_ref()
            .map(|display| frame_from_bounds(display.visible_bounds())),
        display_id: display
            .as_ref()
            .and_then(|display| display_uuid(display.as_ref())),
        state,
        tiled,
    }
}

fn open_gitcomet_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    launch: &WindowLaunchConfig,
) -> gpui::WindowHandle<GitCometView> {
    clear_clean_shutdown_request(cx);
    let ui_session = session::load();
    let ui_scale = ui_scale::current_or_initialize_from_session(&ui_session, cx);
    crate::window_controls::current_or_initialize_from_session(&ui_session, cx);
    let min_size = main_window_min_size_for_percent(ui_scale.percent);
    let default_size = fit_default_window_size(
        main_window_default_size_for_percent(ui_scale.percent),
        min_size,
        cx,
    );
    let workspace_placement = match &launch.view_config.workspace {
        WorkspaceBootstrap::Saved(workspace) => Some(workspace.placement.clone()),
        WorkspaceBootstrap::LegacySession | WorkspaceBootstrap::Empty => None,
    };
    // The focused mergetool keeps its own size; workspace frames never apply.
    let (saved_w, saved_h) = if launch.view_config.view_mode == GitCometViewMode::FocusedMergetool {
        (
            ui_session.mergetool_window_width,
            ui_session.mergetool_window_height,
        )
    } else {
        (ui_session.window_width, ui_session.window_height)
    };
    let restored_w = workspace_placement
        .as_ref()
        .and_then(|placement| placement.normal_frame)
        .map(|frame| frame.width)
        .or(saved_w)
        .map(|w| px(w as f32))
        .unwrap_or(default_size.width)
        .max(min_size.width);
    let restored_h = workspace_placement
        .as_ref()
        .and_then(|placement| placement.normal_frame)
        .map(|frame| frame.height)
        .or(saved_h)
        .map(|h| px(h as f32))
        .unwrap_or(default_size.height)
        .max(min_size.height);
    let fallback_size = size(restored_w, restored_h);
    let (window_bounds, display_id) = match workspace_placement.as_ref() {
        None => (
            WindowBounds::Windowed(Bounds::centered(None, fallback_size, cx)),
            None,
        ),
        Some(placement) => restored_workspace_window_bounds(placement, fallback_size, min_size, cx),
    };
    let window_title = launch.title.clone();
    let app_id = launch.app_id.clone();
    let view_config = launch.view_config.clone();
    let ui_scale_percent = ui_scale.percent;
    let intercept_native_close = view_config.view_mode == GitCometViewMode::Normal;

    let window = crate::ui_probe::time_section("open main window", || {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
                window_min_size: Some(min_size),
                titlebar: Some(TitlebarOptions {
                    title: Some(window_title.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(
                        crate::view::chrome::macos_traffic_light_position(),
                    ),
                }),
                app_id: Some(app_id),
                display_id,
                window_decorations: Some(WindowDecorations::Client),
                window_background: main_window_background_appearance(),
                is_movable: true,
                is_resizable: true,
                ..Default::default()
            },
            move |window, cx| {
                ui_scale::apply_to_window(window, ui_scale_percent);
                if intercept_native_close {
                    window.on_window_should_close(cx, |window, cx| {
                        close_window_or_warn(window, cx);
                        false
                    });
                }
                #[cfg(test)]
                let (store, events) = AppStore::new_test(Arc::clone(&backend));
                #[cfg(not(test))]
                let (store, events) = AppStore::new(Arc::clone(&backend));
                cx.new(|cx| {
                    GitCometView::new_with_config(store, events, view_config.clone(), window, cx)
                })
            },
        )
    })
    .unwrap_or_else(|err| {
        panic!(
            "failed to open main GitComet window: {err}\n\
             This is usually a GPU/display problem, not a GitComet bug. \
             If you just updated your system (kernel, mesa, or vulkan drivers), reboot. \
             For diagnostics, include the crash report from the next launch."
        )
    });

    #[cfg(target_os = "macos")]
    if intercept_native_close {
        refresh_macos_app_menus(cx);
    }

    window
}

/// Client-side decorations inset a rounded frame into the surface; the pixels
/// outside it must show the desktop, not a solid fill, so every platform but
/// macOS asks for a transparent surface.
///
/// `GITCOMET_WINDOW_BACKGROUND=opaque|transparent` overrides the choice. It is
/// a diagnostic knob: on Windows a transparent surface changes how the
/// compositor blends the window, so this is the quickest way to tell whether
/// that is what makes a build feel slow.
pub(crate) fn main_window_background_appearance() -> WindowBackgroundAppearance {
    let default = if cfg!(target_os = "macos") {
        WindowBackgroundAppearance::Opaque
    } else {
        WindowBackgroundAppearance::Transparent
    };
    match std::env::var("GITCOMET_WINDOW_BACKGROUND") {
        Ok(value) if value.trim().eq_ignore_ascii_case("opaque") => {
            WindowBackgroundAppearance::Opaque
        }
        Ok(value) if value.trim().eq_ignore_ascii_case("transparent") => {
            WindowBackgroundAppearance::Transparent
        }
        _ => default,
    }
}

fn current_or_default_ui_scale_percent(cx: &mut App) -> u32 {
    let current = ui_scale::current(cx);
    if current.initialized {
        current.percent
    } else {
        ui_scale::DEFAULT_UI_SCALE_PERCENT
    }
}

fn apply_ui_scale_to_open_windows(cx: &mut App, percent: u32) {
    for handle in cx.windows() {
        let _ = handle.update(cx, |root_view, window, cx| {
            let root_view = match root_view.downcast::<GitCometView>() {
                Ok(view) => {
                    view.update(cx, |view, cx| {
                        view.apply_ui_scale_percent(percent, window, cx);
                    });
                    return;
                }
                Err(root_view) => root_view,
            };

            if let Ok(view) = root_view.downcast::<SettingsWindowView>() {
                view.update(cx, |view, cx| {
                    view.apply_ui_scale_percent(percent, window, cx);
                });
                return;
            }

            ui_scale::apply_to_window(window, percent);
        });
    }
}

pub(crate) fn set_app_ui_scale_percent(cx: &mut App, percent: u32) {
    let current = ui_scale::current(cx);
    let next = ui_scale::set_current(cx, percent);
    if current.initialized && current.percent == next.percent {
        return;
    }

    apply_ui_scale_to_open_windows(cx, next.percent);
}

fn install_app_actions(cx: &mut App, backend: Arc<dyn GitBackend>) {
    crate::window_focus::observe_tab_navigation(cx).detach();
    install_global_diff_shortcut_fallback(cx);

    let new_window_backend = Arc::clone(&backend);
    cx.on_action(move |_: &NewWindow, cx| {
        let backend = Arc::clone(&new_window_backend);
        cx.defer(move |cx| {
            let launch = normal_empty_launch_config(None);
            open_gitcomet_window(cx, backend, &launch);
            cx.activate(true);
        });
    });

    cx.on_action(|_: &OpenSettings, cx| {
        cx.defer(|cx| {
            crate::view::open_settings_window(cx);
        });
    });

    cx.on_action(|_: &OpenInCodeEditor, cx| {
        cx.defer(|cx| {
            let _ = update_active_or_existing_normal_gitcomet_window(cx, |view, cx| {
                view.open_active_repo_in_external_code_editor(cx);
            });
        });
    });

    let repo_backend = Arc::clone(&backend);
    cx.on_action(move |_: &OpenRepository, cx| {
        let backend = Arc::clone(&repo_backend);
        cx.defer(move |cx| {
            if existing_normal_gitcomet_window_blocks_repository_management_actions(cx) {
                return;
            }
            prompt_open_repository(cx, backend);
        });
    });

    let recent_picker_backend = Arc::clone(&backend);
    cx.on_action(move |_: &SwitchRepository, cx| {
        let backend = Arc::clone(&recent_picker_backend);
        cx.defer(move |cx| {
            if existing_normal_gitcomet_window_blocks_repository_management_actions(cx) {
                return;
            }
            open_repository_switcher_in_existing_or_new_window(cx, backend);
        });
    });
    let workspace_picker_backend = Arc::clone(&backend);
    cx.on_action(move |_: &OpenWorkspace, cx| {
        let backend = Arc::clone(&workspace_picker_backend);
        cx.defer(move |cx| open_workspace_picker_in_existing_or_new_window(cx, backend));
    });
    let command_palette_backend = Arc::clone(&backend);
    cx.on_action(move |_: &ToggleCommandPalette, cx| {
        let backend = Arc::clone(&command_palette_backend);
        cx.defer(move |cx| toggle_command_palette_in_active_existing_or_new_window(cx, backend));
    });
    // Reaches the window even with nothing focused inside it — the same reason
    // the palette needs an app-level handler alongside its window one.
    cx.on_action(|_: &crate::view::ToggleRevealCommit, cx| {
        cx.defer(toggle_reveal_commit_in_active_window);
    });
    cx.on_action(|_: &LocateFileInExplorer, cx| {
        cx.defer(locate_file_in_active_or_existing_normal_window);
    });
    cx.on_action(|_: &OpenRemoteInBrowser, cx| {
        cx.defer(open_remote_in_browser_in_active_window);
    });
    cx.on_action(|_: &ShowReflog, cx| {
        cx.defer(|cx| {
            let _ = update_active_normal_gitcomet_window(cx, |view, cx| {
                view.open_reflog_panel_for_active_repo(cx);
            });
        });
    });

    cx.on_action(|_: &Close, cx| {
        cx.defer(|cx| {
            let handled =
                update_active_normal_gitcomet_window(cx, |view, cx| view.close_active_repo_tab(cx))
                    .unwrap_or(false);
            if !handled {
                close_active_window_or_warn(cx);
            }
        });
    });
    cx.on_action(|_: &CloseWindow, cx| {
        cx.defer(close_active_window_or_warn);
    });
    cx.on_action(|_: &PreviousRepository, cx| {
        cx.defer(|cx| {
            let _ = update_active_normal_gitcomet_window(cx, |view, cx| {
                view.activate_previous_repo_tab(cx)
            });
        });
    });
    cx.on_action(|_: &NextRepository, cx| {
        cx.defer(|cx| {
            let _ = update_active_normal_gitcomet_window(cx, |view, cx| {
                view.activate_next_repo_tab(cx)
            });
        });
    });
    cx.on_action(|_: &MinimizeWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_root, window, _cx| {
                    window.minimize_window();
                });
            }
        });
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_root, window, _cx| {
                    toggle_window_zoom(window);
                });
            }
        });
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_root, window, _cx| {
                    window.toggle_fullscreen();
                });
            }
        });
    });
    cx.on_action(|_: &IncreaseUiScale, cx| {
        cx.defer(|cx| {
            let next = ui_scale::step_up(current_or_default_ui_scale_percent(cx));
            set_app_ui_scale_percent(cx, next);
        });
    });
    cx.on_action(|_: &DecreaseUiScale, cx| {
        cx.defer(|cx| {
            let next = ui_scale::step_down(current_or_default_ui_scale_percent(cx));
            set_app_ui_scale_percent(cx, next);
        });
    });
    cx.on_action(|_: &ResetUiScale, cx| {
        cx.defer(|cx| {
            set_app_ui_scale_percent(cx, ui_scale::DEFAULT_UI_SCALE_PERCENT);
        });
    });
    cx.on_action(|_: &Hide, cx| cx.defer(|cx| cx.hide()));
    cx.on_action(|_: &HideOthers, cx| cx.defer(|cx| cx.hide_other_apps()));
    cx.on_action(|_: &ShowAll, cx| cx.defer(|cx| cx.unhide_other_apps()));
    cx.on_action(|_: &Quit, cx| cx.defer(quit_app_or_warn));
}

fn install_global_diff_shortcut_fallback(cx: &mut App) {
    cx.observe_keystrokes(|event, window, cx| {
        // Observers also run after a bound action handled the keystroke (gpui
        // passes that action here); acting again would run F3 twice and skip
        // every other change block.
        if event.action.is_some()
            || !is_diff_shortcut_candidate(&event.keystroke)
            || event.context_stack.iter().any(|context| {
                context.contains("TextInput")
                    || context.contains("Terminal")
                    || context.contains("ContextMenu")
                    || context.contains("PopoverPrompt")
                    || (context.contains("StatusSection")
                        && crate::view::is_status_section_shortcut(&event.keystroke))
            })
        {
            return;
        }

        let window_id = window.window_handle().window_id();
        let Some(entry) = gitcomet_window_entries(cx).into_iter().find(|entry| {
            entry.handle.window_id() == window_id && entry.view_mode == GitCometViewMode::Normal
        }) else {
            return;
        };

        let handled = entry
            .main_pane
            .update(cx, |pane, cx| {
                let handled = pane.handle_diff_shortcut(&event.keystroke, window, cx);
                if handled {
                    cx.notify();
                    window.refresh();
                }
                handled
            })
            .unwrap_or(false);
        if handled {
            cx.stop_propagation();
        }
    })
    .detach();
}

#[cfg(test)]
pub(crate) fn install_global_diff_shortcut_fallback_for_test(cx: &mut App) {
    install_global_diff_shortcut_fallback(cx);
}

#[cfg(target_os = "macos")]
fn install_macos_app_menu(cx: &mut App, backend: Arc<dyn GitBackend>) {
    let recent_repo_backend = Arc::clone(&backend);
    cx.on_action(move |recent: &OpenRecentRepository, cx| {
        let path = session::path_from_storage_key(&recent.storage_key);
        let backend = Arc::clone(&recent_repo_backend);
        cx.defer(move |cx| {
            open_repository_in_existing_or_new_window(cx, backend, path);
        });
    });

    cx.on_action(|_: &ApplyPatch, cx| {
        cx.defer(prompt_apply_patch);
    });

    cx.on_action(|_: &CheckForUpdates, cx| {
        let _ = check_for_updates_in_active_or_existing_normal_window(cx);
    });

    let clone_backend = Arc::clone(&backend);
    cx.on_action(move |_: &CloneRepository, cx| {
        let backend = Arc::clone(&clone_backend);
        cx.defer(move |cx| open_clone_repository_in_existing_or_new_window(cx, backend));
    });

    let initialize_backend = Arc::clone(&backend);
    cx.on_action(move |_: &InitializeRepository, cx| {
        let backend = Arc::clone(&initialize_backend);
        cx.defer(move |cx| prompt_initialize_repository_in_existing_or_new_window(cx, backend));
    });

    refresh_macos_app_menus(cx);
}

fn bind_app_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewWindow, None),
        KeyBinding::new("secondary-shift-n", NewWindow, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-o", OpenRepository, None),
        KeyBinding::new("secondary-shift-o", SwitchRepository, None),
        KeyBinding::new("secondary-shift-a", SwitchRepository, None),
        KeyBinding::new("secondary-shift-r", OpenWorkspace, None),
        KeyBinding::new("secondary-f", OpenActiveViewSearch, None),
        KeyBinding::new("secondary-p", ToggleCommandPalette, None),
        KeyBinding::new("secondary-g", crate::view::ToggleRevealCommit, None),
        KeyBinding::new("secondary-shift-l", LocateFileInExplorer, None),
        KeyBinding::new("secondary-k", OpenRemoteInBrowser, None),
        KeyBinding::new("secondary-w", Close, None),
        KeyBinding::new("secondary-shift-w", CloseWindow, None),
        KeyBinding::new("secondary-pageup", PreviousRepository, None),
        KeyBinding::new("secondary-pagedown", NextRepository, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-tab", PreviousRepository, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-tab", NextRepository, None),
        KeyBinding::new("secondary-+", IncreaseUiScale, None),
        KeyBinding::new("secondary-=", IncreaseUiScale, None),
        KeyBinding::new("secondary--", DecreaseUiScale, None),
        KeyBinding::new("secondary-0", ResetUiScale, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("f1", DiffPrevFile, None),
        KeyBinding::new("f4", DiffNextFile, None),
        KeyBinding::new("f2", DiffPrevSearchMatchOrChange, None),
        KeyBinding::new("f3", DiffNextSearchMatchOrChange, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-o", SwitchRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-{", PreviousRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-left", PreviousRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-}", NextRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-right", NextRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("f11", ToggleFullScreen, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-h", Hide, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-h", HideOthers, None),
    ]);
    refresh_external_editor_key_binding(cx);
}

fn refresh_external_editor_key_binding(cx: &mut App) {
    refresh_external_editor_key_binding_for_configured(
        cx,
        crate::external_editor::configured_setting().is_some(),
    );
}

fn refresh_external_editor_key_binding_for_configured(cx: &mut App, configured: bool) {
    if configured {
        cx.bind_keys([KeyBinding::new("secondary-shift-e", OpenInCodeEditor, None)]);
    } else {
        cx.bind_keys([KeyBinding::new(
            "secondary-shift-e",
            Unbind(OpenInCodeEditor.name().into()),
            None,
        )]);
    }
}

pub(crate) fn refresh_external_editor_app_surfaces_for_setting(
    setting: Option<&session::ExternalCodeEditorSetting>,
    cx: &mut App,
) {
    let configured = setting.is_some_and(crate::external_editor::setting_is_configured);
    refresh_external_editor_key_binding_for_configured(cx, configured);
    #[cfg(target_os = "macos")]
    refresh_macos_app_menus_for_external_editor(cx, configured);
}

#[cfg(target_os = "macos")]
fn macos_app_menus(cx: &mut App) -> Vec<Menu> {
    macos_app_menus_with_options(
        crate::external_editor::configured_setting().is_some(),
        find_normal_gitcomet_window(cx).is_some(),
    )
}

/// The menus as they look with a normal window open, so the tests that care
/// about the external-editor item do not have to restate that half.
#[cfg(all(test, target_os = "macos"))]
fn macos_app_menus_with_external_editor(external_editor_configured: bool) -> Vec<Menu> {
    macos_app_menus_with_options(external_editor_configured, true)
}

#[cfg(target_os = "macos")]
fn macos_app_menus_with_options(
    external_editor_configured: bool,
    normal_window_available: bool,
) -> Vec<Menu> {
    let mut file_items = vec![
        MenuItem::action("New Window", NewWindow),
        MenuItem::separator(),
        MenuItem::action(crate::menu_labels::OPEN_REPOSITORY, OpenRepository),
        MenuItem::action(crate::menu_labels::CLONE_REPOSITORY, CloneRepository),
        MenuItem::action(
            crate::menu_labels::INITIALIZE_REPOSITORY,
            InitializeRepository,
        ),
        MenuItem::action("Switch Repository…", SwitchRepository),
        MenuItem::action("Open Workspace…", OpenWorkspace),
    ];

    let recent_repo_items = recent_repo_menu_items();
    if !recent_repo_items.is_empty() {
        file_items.push(MenuItem::submenu(Menu {
            name: "Recent Repositories".into(),
            items: recent_repo_items,
            disabled: false,
        }));
    }
    file_items.push(MenuItem::separator());
    if external_editor_configured {
        file_items.push(MenuItem::action(
            crate::menu_labels::OPEN_IN_CODE_EDITOR,
            OpenInCodeEditor,
        ));
    }

    file_items.extend([
        MenuItem::action(
            crate::menu_labels::OPEN_IN_FILE_EXPLORER,
            LocateFileInExplorer,
        ),
        MenuItem::action(
            crate::menu_labels::OPEN_REMOTE_IN_BROWSER,
            OpenRemoteInBrowser,
        ),
        MenuItem::action(crate::menu_labels::APPLY_PATCH, ApplyPatch),
        MenuItem::action(crate::menu_labels::CHECK_FOR_UPDATES, CheckForUpdates).disabled(
            manual_update_check_menu_disabled(
                crate::view::update_checks_disabled_by_environment(),
                normal_window_available,
            ),
        ),
        MenuItem::separator(),
        MenuItem::action("Close", Close),
        MenuItem::action("Close Window", CloseWindow),
    ]);

    vec![
        Menu {
            name: "GitComet".into(),
            items: vec![
                MenuItem::action(crate::menu_labels::COMMAND_PALETTE, ToggleCommandPalette),
                MenuItem::action(crate::menu_labels::SETTINGS, OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide GitComet", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit GitComet", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: file_items,
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", crate::kit::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", crate::kit::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", crate::kit::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", crate::kit::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", crate::kit::Paste, OsAction::Paste),
                MenuItem::separator(),
                MenuItem::os_action("Select All", crate::kit::SelectAll, OsAction::SelectAll),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Reflog", ShowReflog)],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", MinimizeWindow),
                MenuItem::action("Zoom", ZoomWindow),
                MenuItem::separator(),
                MenuItem::action("Zoom In", IncreaseUiScale),
                MenuItem::action("Zoom Out", DecreaseUiScale),
                MenuItem::action("Actual Size", ResetUiScale),
                MenuItem::separator(),
                MenuItem::action("Previous Repository", PreviousRepository),
                MenuItem::action("Next Repository", NextRepository),
                MenuItem::separator(),
                MenuItem::action("Toggle Full Screen", ToggleFullScreen),
            ],
            disabled: false,
        },
    ]
}

#[cfg(target_os = "macos")]
pub(crate) fn refresh_macos_app_menus(cx: &mut App) {
    let menus = macos_app_menus(cx);
    cx.set_menus(menus);
}

#[cfg(target_os = "macos")]
fn refresh_macos_app_menus_for_external_editor(cx: &mut App, configured: bool) {
    let normal_window_available = find_normal_gitcomet_window(cx).is_some();
    cx.set_menus(macos_app_menus_with_options(
        configured,
        normal_window_available,
    ));
}

#[cfg(target_os = "macos")]
fn register_macos_open_request_handler(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    open_urls_rx: smol::channel::Receiver<Vec<String>>,
) {
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Ok(urls) = open_urls_rx.recv().await {
            let paths = repository_paths_from_open_urls(&urls);
            if paths.is_empty() {
                continue;
            }

            let backend = Arc::clone(&backend);
            cx.update(move |cx| {
                open_repositories_in_existing_or_new_window(cx, backend, paths);
            });
        }
    })
    .detach();
}

fn register_browser_open_request_handler(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    requests: smol::channel::Receiver<BrowserOpenRequest>,
) {
    // The listener must reject new requests as soon as the app stops consuming
    // them. A weak receiver does not keep this channel alive after the task exits.
    let requests_on_quit = requests.downgrade();
    cx.on_app_quit(move |_| {
        if let Some(requests) = requests_on_quit.upgrade() {
            requests.close();
        }
        async {}
    })
    .detach();
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Ok(request) = requests.recv().await {
            let backend = Arc::clone(&backend);
            cx.update(move |cx| handle_browser_open_request(cx, backend, request));
        }
    })
    .detach();
}

#[cfg(target_os = "macos")]
fn recent_repo_menu_items() -> Vec<MenuItem> {
    session::load()
        .recent_repos
        .into_iter()
        .map(|path| {
            MenuItem::action(
                recent_repository_label(&path),
                OpenRecentRepository {
                    storage_key: session::path_storage_key(&path),
                },
            )
        })
        .collect()
}

/// Labels the macOS "Recent Repositories" menu items. Other platforms only
/// reach the recents through the repository switcher, which builds its own rows.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn recent_repository_label(path: &Path) -> String {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.display().to_string();
    };
    let Some(parent) = path.parent() else {
        return name.to_string();
    };
    format!("{name} - {}", parent.display())
}

#[derive(Clone, PartialEq)]
struct GitCometWindowEntry {
    handle: gpui::AnyWindowHandle,
    view: gpui::WeakEntity<GitCometView>,
    main_pane: gpui::WeakEntity<MainPaneView>,
    view_mode: GitCometViewMode,
    workspace_id: Option<session::WorkspaceId>,
    repo_paths: std::sync::Arc<[PathBuf]>,
}

#[derive(Default)]
struct GitCometWindowRegistry {
    windows: FxHashMap<gpui::WindowId, GitCometWindowEntry>,
    last_focused_normal_window: Option<gpui::WindowId>,
}

impl gpui::Global for GitCometWindowRegistry {}

#[derive(Default)]
struct ProcessStartupHooksState {
    ran: bool,
}

impl gpui::Global for ProcessStartupHooksState {}

#[cfg(test)]
#[derive(Default)]
struct StartupHookInvocationCount(usize, Option<gpui::WindowId>);

#[cfg(test)]
impl gpui::Global for StartupHookInvocationCount {}

#[cfg(test)]
pub(crate) fn record_startup_hook_invocation_for_test<C>(cx: &mut C, window_id: gpui::WindowId)
where
    C: BorrowAppContext,
{
    cx.update_default_global::<StartupHookInvocationCount, _>(|count, _cx| {
        count.0 += 1;
        count.1 = Some(window_id);
    });
}

#[cfg(test)]
fn startup_hook_invocation_count_for_test(cx: &mut App) -> usize {
    cx.update_default_global::<StartupHookInvocationCount, _>(|count, _cx| count.0)
}

fn run_process_startup_hooks_once(cx: &mut App, startup_window: Option<gpui::WindowId>) {
    // Native activation can still be queued; route hooks to the chosen startup
    // window directly instead of depending on whichever window has focus now.
    let Some(window) = startup_window.and_then(|id| normal_gitcomet_window_by_id(cx, id)) else {
        return;
    };
    let should_run = cx.update_default_global::<ProcessStartupHooksState, _>(|state, _cx| {
        if state.ran {
            false
        } else {
            state.ran = true;
            true
        }
    });
    if !should_run {
        return;
    }
    let _ = window.view.update(cx, |view, cx| {
        view.run_process_startup_hooks(cx);
    });
}

pub(crate) fn sync_gitcomet_window_registry<C>(
    cx: &mut C,
    handle: gpui::AnyWindowHandle,
    view: gpui::WeakEntity<GitCometView>,
    main_pane: gpui::WeakEntity<MainPaneView>,
    view_mode: GitCometViewMode,
    workspace_id: Option<session::WorkspaceId>,
    repo_paths: Arc<[PathBuf]>,
) where
    C: std::borrow::BorrowMut<App>,
{
    let entry = GitCometWindowEntry {
        handle,
        view,
        main_pane,
        view_mode,
        workspace_id,
        repo_paths,
    };
    // Every store tick lands here; skip the lease when nothing moved.
    let unchanged = cx
        .borrow()
        .try_global::<GitCometWindowRegistry>()
        .and_then(|registry| registry.windows.get(&handle.window_id()))
        .is_some_and(|current| *current == entry);
    if !unchanged {
        cx.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
            registry.windows.insert(handle.window_id(), entry);
        });
    }
}

pub(crate) fn mark_gitcomet_window_focused<C>(cx: &mut C, window_id: gpui::WindowId)
where
    C: BorrowAppContext,
{
    cx.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
        if registry
            .windows
            .get(&window_id)
            .is_some_and(|entry| entry.view_mode == GitCometViewMode::Normal)
        {
            registry.last_focused_normal_window = Some(window_id);
        }
    });
}

fn gitcomet_window_entries(cx: &mut App) -> Vec<GitCometWindowEntry> {
    let live_window_ids: FxHashSet<_> = cx
        .windows()
        .into_iter()
        .map(|window| window.window_id())
        .collect();
    cx.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
        registry
            .windows
            .retain(|window_id, _| live_window_ids.contains(window_id));
        if registry
            .last_focused_normal_window
            .is_some_and(|window_id| !live_window_ids.contains(&window_id))
        {
            registry.last_focused_normal_window = None;
        }
        registry.windows.values().cloned().collect()
    })
}

fn flush_open_workspace_environments(cx: &mut App) {
    for entry in gitcomet_window_entries(cx) {
        if entry.view_mode != GitCometViewMode::Normal {
            continue;
        }
        let _ = entry.view.update(cx, |view, cx| {
            view.flush_workspace_environment(cx);
        });
    }
}

fn last_focused_normal_window_id(cx: &mut App) -> Option<gpui::WindowId> {
    cx.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
        registry.last_focused_normal_window
    })
}

fn active_gitcomet_window_entry(cx: &mut App) -> Option<GitCometWindowEntry> {
    let active_window_id = cx.active_window()?.window_id();
    gitcomet_window_entries(cx)
        .into_iter()
        .find(|entry| entry.handle.window_id() == active_window_id)
}

fn entry_contains_repo_path(entry: &GitCometWindowEntry, path: &Path) -> bool {
    entry.repo_paths.iter().any(|repo_path| repo_path == path)
}

/// Reject a missing destination or a move within the same workspace before
/// save/discard and terminal-shutdown guards can change editor or terminal state.
pub(crate) fn repository_move_target_is_noop<C>(
    cx: &mut C,
    source_window_id: gpui::WindowId,
    target_workspace: Option<session::WorkspaceId>,
) -> bool
where
    C: std::borrow::BorrowMut<App>,
{
    let Some(target_workspace) = target_workspace else {
        return false;
    };
    let live_windows = live_normal_windows(cx.borrow_mut());
    let Some(source) = live_windows
        .iter()
        .find(|entry| entry.handle.window_id() == source_window_id)
    else {
        return true;
    };
    if source.workspace_id == Some(target_workspace) {
        return true;
    }
    if let Some(target) = live_windows
        .iter()
        .find(|entry| entry.workspace_id == Some(target_workspace))
    {
        return target.handle.window_id() == source_window_id;
    }

    crate::workspaces::workspace(cx.borrow(), target_workspace).is_none()
}

fn active_normal_gitcomet_window(cx: &mut App) -> Option<GitCometWindowEntry> {
    let entry = active_gitcomet_window_entry(cx)?;
    (entry.view_mode == GitCometViewMode::Normal).then_some(entry)
}

fn normal_gitcomet_window_by_id(
    cx: &mut App,
    window_id: gpui::WindowId,
) -> Option<GitCometWindowEntry> {
    gitcomet_window_entries(cx).into_iter().find(|entry| {
        entry.handle.window_id() == window_id && entry.view_mode == GitCometViewMode::Normal
    })
}

fn update_active_normal_gitcomet_window<R>(
    cx: &mut App,
    f: impl FnOnce(&mut GitCometView, &mut gpui::Context<GitCometView>) -> R,
) -> Option<R> {
    let window = active_normal_gitcomet_window(cx)?;
    window.view.update(cx, f).ok()
}

fn update_active_or_existing_normal_gitcomet_window<R>(
    cx: &mut App,
    f: impl FnOnce(&mut GitCometView, &mut gpui::Context<GitCometView>) -> R,
) -> Option<R> {
    let window = find_normal_gitcomet_window(cx)?;
    window.view.update(cx, f).ok()
}

#[cfg(any(test, target_os = "macos"))]
fn check_for_updates_in_active_or_existing_normal_window(cx: &mut App) -> bool {
    let Some(window) = find_normal_gitcomet_window(cx) else {
        return false;
    };
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
    window
        .view
        .update(cx, |view, cx| view.check_for_updates_manually(cx))
        .is_ok()
}

#[cfg(any(test, target_os = "macos"))]
fn manual_update_check_menu_disabled(
    disabled_by_environment: bool,
    normal_window_available: bool,
) -> bool {
    disabled_by_environment || !normal_window_available
}

fn normal_gitcomet_window_blocks_repository_management_actions(
    cx: &mut App,
    window: &GitCometWindowEntry,
) -> bool {
    window
        .view
        .update(cx, |view, _cx| view.blocks_repository_management_actions())
        .unwrap_or(false)
}

fn existing_normal_gitcomet_window_blocks_repository_management_actions(cx: &mut App) -> bool {
    let Some(window) = find_normal_gitcomet_window(cx) else {
        return false;
    };
    normal_gitcomet_window_blocks_repository_management_actions(cx, &window)
}

fn locate_file_in_active_or_existing_normal_window(cx: &mut App) {
    let Some(window) = find_normal_gitcomet_window(cx) else {
        return;
    };
    if window
        .view
        .update(cx, |view, cx| view.locate_open_file_in_explorer(cx))
        .is_err()
    {
        return;
    }
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

fn mark_clean_shutdown<C>(cx: &mut C)
where
    C: BorrowAppContext,
{
    cx.update_default_global::<CleanShutdownTracker, _>(|tracker, _cx| {
        tracker.requested.store(true, Ordering::SeqCst);
    });
}

fn clear_clean_shutdown_request(cx: &mut App) {
    if let Some(tracker) = cx.try_global::<CleanShutdownTracker>() {
        tracker.requested.store(false, Ordering::SeqCst);
    }
}

pub(crate) fn mark_clean_shutdown_if_last_window(cx: &mut App) {
    if cx.windows().len() == 1 {
        mark_clean_shutdown(cx);
    }
}

pub(crate) fn mark_clean_shutdown_requested(cx: &mut App) {
    mark_clean_shutdown(cx);
}

pub(crate) fn mark_clean_shutdown_if_last_window_from_view<T>(cx: &mut gpui::Context<T>)
where
    T: 'static,
{
    if cx.windows().len() == 1 {
        mark_clean_shutdown(cx);
    }
}

pub(crate) fn mark_clean_shutdown_from_view<T>(cx: &mut gpui::Context<T>)
where
    T: 'static,
{
    mark_clean_shutdown(cx);
}

/// Keep the last main workspace restorable off macOS, including when an
/// auxiliary Settings window remains open during shutdown.
pub(crate) fn mark_window_closing(cx: &mut App, window_id: gpui::WindowId) {
    let last_window = cx.windows().len() == 1;
    let normal_windows = live_normal_windows(cx);
    let last_main_window =
        normal_windows.len() == 1 && normal_windows[0].handle.window_id() == window_id;
    let preserve_workspace = (last_window || last_main_window) && !cfg!(target_os = "macos");
    if !preserve_workspace {
        crate::workspaces::mark_window_closed(cx, window_id);
    }
    if last_window || last_main_window {
        mark_clean_shutdown(cx);
    }
}

/// Record the window's latest pane layout before it closes; the debounced
/// write may still be pending.
fn flush_workspace_environment_for(cx: &mut App, window_id: gpui::WindowId) {
    if let Some(entry) = normal_gitcomet_window_by_id(cx, window_id) {
        let _ = entry
            .view
            .update(cx, |view, cx| view.flush_workspace_environment(cx));
    }
}

fn close_active_window(cx: &mut App) {
    if let Some(window) = cx.active_window() {
        flush_workspace_environment_for(cx, window.window_id());
        mark_window_closing(cx, window.window_id());
        let _ = window.update(cx, |_root, window, _cx| {
            window.remove_window();
        });
    }
}

pub(crate) fn close_window_or_warn(window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    let handled = normal_gitcomet_window_by_id(cx, window_id)
        .and_then(|entry| {
            entry
                .view
                .update(cx, |view, cx| {
                    view.request_close_window_or_warn(window_id, cx)
                })
                .ok()
        })
        .unwrap_or(false);
    if !handled {
        flush_workspace_environment_for(cx, window_id);
        mark_window_closing(cx, window_id);
        window.remove_window();
    }
}

/// Close a specific window, honouring its own warnings.
///
/// The unsaved-edits retry needs this rather than "the active window": it can
/// run seconds after the prompt (a slow write drains first), by which time the
/// user may have clicked into another window — and closing that one instead is
/// not what they asked for.
pub(crate) fn close_window_by_id_or_warn(cx: &mut App, window_id: gpui::WindowId) {
    let handled = normal_gitcomet_window_by_id(cx, window_id)
        .and_then(|entry| {
            entry
                .view
                .update(cx, |view, cx| {
                    view.request_close_window_or_warn(window_id, cx)
                })
                .ok()
        })
        .unwrap_or(false);
    if handled {
        return;
    }
    mark_window_closing(cx, window_id);
    if let Some(entry) = normal_gitcomet_window_by_id(cx, window_id) {
        let _ = entry
            .handle
            .update(cx, |_, window, _| window.remove_window());
    }
}

pub(crate) fn close_active_window_or_warn(cx: &mut App) {
    let active_window_id = cx.active_window().map(|window| window.window_id());
    let handled = active_window_id
        .and_then(|window_id| {
            update_active_normal_gitcomet_window(cx, move |view, cx| {
                view.request_close_window_or_warn(window_id, cx)
            })
        })
        .unwrap_or(false);
    if !handled {
        close_active_window(cx);
    }
}

pub(crate) fn quit_app_or_warn(cx: &mut App) {
    let entries: Vec<_> = gitcomet_window_entries(cx)
        .into_iter()
        .filter(|entry| entry.view_mode == GitCometViewMode::Normal)
        .collect();
    if entries.is_empty() {
        mark_clean_shutdown(cx);
        cx.quit();
        return;
    }

    // Unsaved editor buffers are asked about before the terminal summary, and
    // before the no-running-commands shortcut below: a quit with nothing running
    // is the common case and used to exit straight past them. Every window is
    // offered the prompt, because each holds its own buffers.
    for entry in &entries {
        let queued = entry
            .view
            .update(cx, |view, cx| {
                view.request_quit_unsaved_file_edits_prompt(cx)
            })
            .unwrap_or(false);
        if queued {
            entry
                .handle
                .update(cx, |_, window, _| window.activate_window())
                .ok();
            return;
        }
    }

    let mut terminal_count = 0usize;
    let mut running_command_count = 0usize;
    let mut repo_names: Vec<String> = Vec::new();
    for entry in &entries {
        if let Ok(summary) = entry
            .view
            .read_with(cx, |view, _cx| view.running_terminal_summary())
        {
            terminal_count += summary.terminal_count;
            running_command_count += summary.running_command_count;
            repo_names.extend(summary.repo_names);
        }
    }

    if running_command_count == 0 {
        mark_clean_shutdown(cx);
        cx.quit();
        return;
    }

    let active_window_id = cx.active_window().map(|window| window.window_id());
    let prompt_entry = active_window_id
        .and_then(|active_window_id| {
            entries
                .iter()
                .find(|entry| entry.handle.window_id() == active_window_id)
                .cloned()
        })
        .or_else(|| entries.first().cloned());

    if let Some(entry) = prompt_entry {
        let all_views: Vec<_> = entries.iter().map(|e| e.view.clone()).collect();
        let _ = entry.view.update(cx, |view, cx| {
            view.request_quit_or_warn(
                terminal_count,
                running_command_count,
                repo_names,
                all_views,
                cx,
            );
        });
    } else {
        mark_clean_shutdown(cx);
        cx.quit();
    }
}

fn find_normal_gitcomet_window(cx: &mut App) -> Option<GitCometWindowEntry> {
    let entries = gitcomet_window_entries(cx);
    let active_window_id = cx.active_window().map(|window| window.window_id());
    if let Some(active_window_id) = active_window_id
        && let Some(entry) = entries
            .iter()
            .find(|entry| {
                entry.handle.window_id() == active_window_id
                    && entry.view_mode == GitCometViewMode::Normal
            })
            .cloned()
    {
        return Some(entry);
    }
    if let Some(last_focused) = last_focused_normal_window_id(cx)
        && let Some(entry) = entries
            .iter()
            .find(|entry| {
                entry.handle.window_id() == last_focused
                    && entry.view_mode == GitCometViewMode::Normal
            })
            .cloned()
    {
        return Some(entry);
    }
    entries
        .into_iter()
        .find(|entry| entry.view_mode == GitCometViewMode::Normal)
}

#[cfg(test)]
fn find_normal_gitcomet_window_for_repo(cx: &mut App, path: &Path) -> Option<GitCometWindowEntry> {
    let active_window_id = cx.active_window().map(|window| window.window_id());
    let last_focused = last_focused_normal_window_id(cx);
    live_normal_windows(cx)
        .into_iter()
        .filter(|entry| entry_contains_repo_path(entry, path))
        .min_by_key(|entry| {
            let id = Some(entry.handle.window_id());
            (id != active_window_id, id != last_focused)
        })
}

fn activate_gitcomet_window(cx: &mut App, window: gpui::AnyWindowHandle) {
    let _ = window.update(cx, |_view, window, _cx| {
        window.activate_window();
    });
}

fn live_normal_windows(cx: &mut App) -> Vec<GitCometWindowEntry> {
    gitcomet_window_entries(cx)
        .into_iter()
        .filter(|entry| entry.view_mode == GitCometViewMode::Normal)
        .collect()
}

/// Open a workspace from a window: an empty window adopts it in place (no
/// second window); otherwise focus its owner or open a new window.
pub(crate) fn open_workspace_in_window(
    cx: &mut App,
    window_id: gpui::WindowId,
    workspace_id: session::WorkspaceId,
) {
    let entries = gitcomet_window_entries(cx);
    if let Some(owner) = entries.iter().find(|entry| {
        entry.view_mode == GitCometViewMode::Normal && entry.workspace_id == Some(workspace_id)
    }) {
        activate_gitcomet_window(cx, owner.handle);
        return;
    }
    let Some(target) =
        normal_gitcomet_window_by_id(cx, window_id).filter(|target| target.repo_paths.is_empty())
    else {
        let _ = activate_or_open_workspace(cx, workspace_id);
        return;
    };
    let Some(workspace) = crate::workspaces::workspace(cx, workspace_id) else {
        return;
    };
    if target
        .workspace_id
        .is_some_and(|current| current != workspace_id)
    {
        crate::workspaces::release_window_workspace(cx, window_id);
    }
    let _ = target
        .view
        .update(cx, |view, cx| view.adopt_workspace(workspace, cx));
    activate_gitcomet_window(cx, target.handle);
    cx.activate(true);
}

fn activate_or_open_workspace(
    cx: &mut App,
    workspace_id: session::WorkspaceId,
) -> Option<GitCometWindowEntry> {
    if let Some(window) = gitcomet_window_entries(cx).into_iter().find(|entry| {
        entry.view_mode == GitCometViewMode::Normal && entry.workspace_id == Some(workspace_id)
    }) {
        activate_gitcomet_window(cx, window.handle);
        return Some(window);
    }

    let workspace = crate::workspaces::workspace(cx, workspace_id)?;

    let backend = cx
        .try_global::<GitCometBackendGlobal>()
        .map(|backend| Arc::clone(&backend.0))?;
    let base = normal_launch_config(None, None);
    let launch = launch_config_for_workspace(&base, workspace, None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let window_id = window.window_id();
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
    normal_gitcomet_window_by_id(cx, window_id)
}

pub(crate) fn activate_workspace_from_view<T>(
    cx: &mut gpui::Context<T>,
    workspace_id: session::WorkspaceId,
) where
    T: 'static,
{
    cx.defer(move |cx| {
        let _ = activate_or_open_workspace(cx, workspace_id);
    });
}

/// Forget a workspace. Its window closes, or returns to Home when it is the
/// last one, once that window's unsaved-edit and terminal guards agree.
pub(crate) fn delete_workspace(cx: &mut App, workspace_id: session::WorkspaceId) {
    let Some(owner) = live_normal_windows(cx)
        .into_iter()
        .find(|entry| entry.workspace_id == Some(workspace_id))
    else {
        crate::workspaces::discard_workspace(cx, workspace_id);
        return;
    };
    let window_id = owner.handle.window_id();
    let prompted = owner
        .view
        .update(cx, |view, cx| {
            view.request_delete_workspace_or_warn(window_id, workspace_id, cx)
        })
        .unwrap_or(false);
    if prompted {
        // The prompt lives in that window; the request may come from another.
        activate_gitcomet_window(cx, owner.handle);
    } else {
        finish_workspace_delete(cx, window_id, workspace_id);
    }
}

pub(crate) fn delete_workspace_from_view<T>(
    cx: &mut gpui::Context<T>,
    workspace_id: session::WorkspaceId,
) where
    T: 'static,
{
    cx.defer(move |cx| delete_workspace(cx, workspace_id));
}

/// The guards passed. Decided now rather than at request time, since a prompt
/// can sit open while other windows come and go.
pub(crate) fn finish_workspace_delete(
    cx: &mut App,
    window_id: gpui::WindowId,
    workspace_id: session::WorkspaceId,
) {
    let live = live_normal_windows(cx);
    let last_window = live.len() == 1;
    let Some(owner) = live.into_iter().find(|entry| {
        entry.handle.window_id() == window_id && entry.workspace_id == Some(workspace_id)
    }) else {
        crate::workspaces::discard_workspace(cx, workspace_id);
        return;
    };
    if last_window {
        let _ = owner
            .view
            .update(cx, |view, cx| view.reset_to_home_after_workspace_delete(cx));
    } else {
        crate::workspaces::discard_workspace_for_window(cx, window_id);
        let _ = owner
            .handle
            .update(cx, |_, window, _| window.remove_window());
    }
}

fn move_repository_to_workspace(
    cx: &mut App,
    source_window_id: gpui::WindowId,
    repo_id: gitcomet_state::model::RepoId,
    path: PathBuf,
    target_workspace: Option<session::WorkspaceId>,
) {
    let Some(source) = normal_gitcomet_window_by_id(cx, source_window_id) else {
        return;
    };
    if !entry_contains_repo_path(&source, &path)
        || target_workspace.is_some() && source.workspace_id == target_workspace
    {
        return;
    }

    if !source
        .view
        .update(cx, |view, cx| {
            view.prepare_repo_move(repo_id, &path, target_workspace, cx)
        })
        .unwrap_or(false)
    {
        return;
    }

    let target = match target_workspace {
        Some(workspace_id) => activate_or_open_workspace(cx, workspace_id),
        None => {
            let backend = cx
                .try_global::<GitCometBackendGlobal>()
                .map(|backend| Arc::clone(&backend.0));
            backend.and_then(|backend| {
                let launch = normal_empty_launch_config(None);
                let window = open_gitcomet_window(cx, backend, &launch);
                let window_id = window.window_id();
                activate_gitcomet_window(cx, window.into());
                cx.activate(true);
                normal_gitcomet_window_by_id(cx, window_id)
            })
        }
    };
    let Some(target) = target else {
        return;
    };
    if target.handle.window_id() == source_window_id {
        return;
    }

    if entry_contains_repo_path(&target, &path) {
        focus_existing_repository_window(cx, &target, &path);
    } else {
        open_repository_in_window(cx, &target, path);
    }

    let _ = source.view.update(cx, |view, cx| {
        view.detach_repo_for_move(repo_id, cx);
    });
    let source_is_customized = crate::workspaces::workspace_for_window(cx, source_window_id)
        .is_some_and(|workspace| workspace.is_customized());
    if source.repo_paths.len() == 1 && !source_is_customized {
        crate::workspaces::discard_workspace_for_window(cx, source_window_id);
        let _ = source.handle.update(cx, |_root, window, _cx| {
            window.remove_window();
        });
    }
}

/// Re-enter the view-level move workflow for a specific source window. This is
/// used after an unsaved-edits save/discard completes so the terminal guard is
/// still honored and focus changes cannot redirect the move to another window.
pub(crate) fn request_move_repository_to_workspace_by_id(
    cx: &mut App,
    source_window_id: gpui::WindowId,
    repo_id: gitcomet_state::model::RepoId,
    path: PathBuf,
    target_workspace: Option<session::WorkspaceId>,
) {
    let Some(source) = normal_gitcomet_window_by_id(cx, source_window_id) else {
        return;
    };
    let _ = source.view.update(cx, |view, cx| {
        view.request_move_repo_to_workspace(repo_id, path, target_workspace, cx);
    });
}

pub(crate) fn move_repository_to_workspace_from_view<T>(
    cx: &mut gpui::Context<T>,
    source_window_id: gpui::WindowId,
    repo_id: gitcomet_state::model::RepoId,
    path: PathBuf,
    target_workspace: Option<session::WorkspaceId>,
) where
    T: 'static,
{
    cx.defer(move |cx| {
        move_repository_to_workspace(cx, source_window_id, repo_id, path, target_workspace);
    });
}

/// Refresh every live window that owns a workspace whose name, colour or theme
/// changed. Requests come from a `PopoverHost` or the settings window, so defer
/// before touching a root view that may own that same host.
pub(crate) fn notify_workspace_changed_from_view<T>(
    cx: &mut gpui::Context<T>,
    workspace_id: session::WorkspaceId,
) where
    T: 'static,
{
    cx.defer(move |cx| {
        for entry in gitcomet_window_entries(cx) {
            if entry.workspace_id != Some(workspace_id) {
                continue;
            }
            let _ = entry.view.update(cx, |view, cx| {
                view.workspace_changed(cx);
            });
        }
    });
}

pub(crate) fn open_repository_from_view<T>(
    cx: &mut gpui::Context<T>,
    source_window_id: gpui::WindowId,
    path: PathBuf,
) where
    T: 'static,
{
    cx.defer(move |cx| {
        let path = normalize_repository_open_path(path);
        let source = normal_gitcomet_window_by_id(cx, source_window_id)
            .or_else(|| find_normal_gitcomet_window(cx));
        if let Some(source) = source {
            open_repository_in_window(cx, &source, path);
        }
    });
}

/// Reuse a tab in the drop's destination window, or open the folder there
/// provisionally until the backend validates it.
pub(crate) fn open_dropped_repository_from_view<T>(
    cx: &mut gpui::Context<T>,
    source_window_id: gpui::WindowId,
    path: PathBuf,
) where
    T: 'static,
{
    cx.defer(move |cx| {
        let path = normalize_repository_open_path(path);
        if let Some(source) = normal_gitcomet_window_by_id(cx, source_window_id) {
            if entry_contains_repo_path(&source, &path) {
                focus_existing_repository_window(cx, &source, &path);
            } else {
                let _ = source
                    .view
                    .update(cx, |view, cx| view.open_dropped_repo_locally(path, cx));
            }
        }
    });
}

fn open_repository_in_window(cx: &mut App, window: &GitCometWindowEntry, path: PathBuf) {
    let _ = window.view.update(cx, |view, cx| {
        view.activate_or_open_repo_path(path, cx);
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

fn focus_existing_repository_window(cx: &mut App, window: &GitCometWindowEntry, path: &Path) {
    let path_for_window = path.to_path_buf();
    let _ = window.view.update(cx, |view, cx| {
        view.activate_or_open_repo_path(path_for_window, cx);
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
    cx.add_recent_document(path);
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn focus_existing_repository_window_for_path(cx: &mut App, path: &Path) -> bool {
    let Some(window) = find_normal_gitcomet_window_for_repo(cx, path) else {
        return false;
    };
    focus_existing_repository_window(cx, &window, path);
    true
}

fn normalize_repository_open_path(path: PathBuf) -> PathBuf {
    let path = if path.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    } else {
        path
    };
    canonicalize_or_original(path)
}

#[cfg(target_os = "macos")]
fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    let path = url.to_file_path().ok()?;
    (!path.as_os_str().is_empty()).then_some(path)
}

#[cfg(target_os = "macos")]
fn repository_paths_from_open_urls(urls: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for url in urls {
        let Some(path) = file_url_to_path(url) else {
            continue;
        };
        let path = normalize_repository_open_path(path);
        if paths.iter().any(|existing| existing == &path) {
            continue;
        }
        paths.push(path);
    }
    paths
}

fn open_repository_switcher_in_window(cx: &mut App, window: &GitCometWindowEntry) {
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.toggle_repository_switcher(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

/// Open Workspace in the active window, or in a new (Home) window.
/// Open a new empty window (a workspace-to-be) on Home. Used from Settings,
/// which has no window of its own to route a `NewWindow` action through.
pub(crate) fn open_new_empty_window(cx: &mut App) {
    let Some(backend) = cx
        .try_global::<GitCometBackendGlobal>()
        .map(|backend| Arc::clone(&backend.0))
    else {
        return;
    };
    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

fn open_workspace_picker_in_existing_or_new_window(cx: &mut App, backend: Arc<dyn GitBackend>) {
    let toggle =
        |view: &mut GitCometView, window: &mut Window, cx: &mut gpui::Context<GitCometView>| {
            view.toggle_workspace_picker(window, cx);
        };
    if let Some(entry) = find_normal_gitcomet_window(cx) {
        let _ = entry.handle.update(cx, |root_view, window, cx| {
            if let Ok(view) = root_view.downcast::<GitCometView>() {
                view.update(cx, |view, cx| toggle(view, window, cx));
            }
        });
        if cx.active_window().map(|active| active.window_id()) != Some(entry.handle.window_id()) {
            activate_gitcomet_window(cx, entry.handle);
        }
        return;
    }
    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| toggle(view, window, cx));
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

fn open_repository_switcher_in_existing_or_new_window(cx: &mut App, backend: Arc<dyn GitBackend>) {
    if let Some(window) = find_normal_gitcomet_window(cx) {
        open_repository_switcher_in_window(cx, &window);
        return;
    }

    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| {
        view.toggle_repository_switcher(window, cx);
    });
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

#[cfg(target_os = "macos")]
fn open_clone_repository_in_window(cx: &mut App, window: &GitCometWindowEntry) {
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.open_clone_repository_prompt(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

#[cfg(target_os = "macos")]
fn open_clone_repository_in_existing_or_new_window(cx: &mut App, backend: Arc<dyn GitBackend>) {
    if let Some(window) = find_normal_gitcomet_window(cx) {
        if normal_gitcomet_window_blocks_repository_management_actions(cx, &window) {
            return;
        }
        open_clone_repository_in_window(cx, &window);
        return;
    }

    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| {
        view.open_clone_repository_prompt(window, cx);
    });
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

#[cfg(target_os = "macos")]
fn prompt_initialize_repository_in_window(cx: &mut App, window: &GitCometWindowEntry) {
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.prompt_init_repo(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

#[cfg(target_os = "macos")]
fn prompt_initialize_repository_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
    if let Some(window) = find_normal_gitcomet_window(cx) {
        if normal_gitcomet_window_blocks_repository_management_actions(cx, &window) {
            return;
        }
        prompt_initialize_repository_in_window(cx, &window);
        return;
    }

    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| {
        view.prompt_init_repo(window, cx);
    });
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

fn toggle_command_palette_in_window(cx: &mut App, window: &GitCometWindowEntry) {
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.toggle_command_palette(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

/// Toggle the Reveal Commit dialog in whichever normal window is in front.
///
/// Unlike the command palette this never opens a window: there is nothing to
/// reveal without a repository, so with no normal window the chord is a no-op.
fn toggle_reveal_commit_in_active_window(cx: &mut App) {
    let Some(window) =
        active_normal_gitcomet_window(cx).or_else(|| find_normal_gitcomet_window(cx))
    else {
        return;
    };
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.toggle_reveal_commit(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

/// Open the front normal window's remote in the browser. With no normal window
/// there is no repository, so the chord is a no-op.
fn open_remote_in_browser_in_active_window(cx: &mut App) {
    let Some(window) =
        active_normal_gitcomet_window(cx).or_else(|| find_normal_gitcomet_window(cx))
    else {
        return;
    };
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.open_remote_in_browser(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

fn toggle_command_palette_in_active_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
    if let Some(window) =
        active_normal_gitcomet_window(cx).or_else(|| find_normal_gitcomet_window(cx))
    {
        toggle_command_palette_in_window(cx, &window);
        return;
    }

    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| {
        view.toggle_command_palette(window, cx);
    });
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

fn show_open_repository_manual_entry_in_window(
    cx: &mut App,
    window: &GitCometWindowEntry,
    show_notice: bool,
) {
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.show_open_repo_panel_fallback(Some(window), show_notice, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

fn show_open_repository_manual_entry_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
    if let Some(window) = find_normal_gitcomet_window(cx) {
        show_open_repository_manual_entry_in_window(cx, &window, true);
        return;
    }

    let launch = normal_empty_launch_config(None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| {
        view.show_open_repo_panel_fallback(Some(window), true, cx);
    });
    activate_gitcomet_window(cx, window.into());
    cx.activate(true);
}

fn open_repositories_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    paths: Vec<PathBuf>,
) {
    let mut target_window = find_normal_gitcomet_window(cx);

    for path in paths {
        if let Some(window) = target_window.as_ref() {
            open_repository_in_window(cx, window, path);
            continue;
        }

        let launch = normal_launch_config_with_initial_repository(path, None);
        let window = open_gitcomet_window(cx, Arc::clone(&backend), &launch);
        activate_gitcomet_window(cx, window.into());
        target_window = find_normal_gitcomet_window(cx);
        cx.activate(true);
    }
}

fn open_repository_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    path: PathBuf,
) {
    open_repositories_in_existing_or_new_window(
        cx,
        backend,
        vec![normalize_repository_open_path(path)],
    );
}

fn handle_browser_open_request(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    request: BrowserOpenRequest,
) {
    handle_browser_open_request_with_window(cx, backend, request, None);
}

fn handle_browser_open_request_with_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    request: BrowserOpenRequest,
    preferred_window: Option<gpui::WindowId>,
) {
    handle_browser_open_request_and_activate(cx, backend, request, preferred_window, App::activate);
}

fn handle_browser_open_request_and_activate(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    request: BrowserOpenRequest,
    preferred_window: Option<gpui::WindowId>,
    // GPUI's headless platform ignores application activation; inject the
    // platform call so tests can verify it independently of window focus.
    activate: impl FnOnce(&App, bool),
) {
    match request.path.map(normalize_repository_open_path) {
        None => {
            if let Some(window) = find_normal_gitcomet_window(cx) {
                activate_gitcomet_window(cx, window.handle);
            } else {
                let launch = normal_empty_launch_config(None);
                let window = open_gitcomet_window(cx, backend, &launch);
                activate_gitcomet_window(cx, window.into());
            }
        }
        Some(path) => match request.target {
            BrowserOpenTarget::ExistingWindow => {
                if let Some(window) =
                    preferred_window.and_then(|id| normal_gitcomet_window_by_id(cx, id))
                {
                    open_repository_in_window(cx, &window, path);
                } else {
                    open_repository_in_existing_or_new_window(cx, backend, path);
                }
            }
            BrowserOpenTarget::NewWindow => {
                let launch = normal_launch_config_with_initial_repository(path, None);
                let window = open_gitcomet_window(cx, backend, &launch);
                activate_gitcomet_window(cx, window.into());
            }
        },
    }
    // On macOS, making a window key does not unhide or foreground the app.
    activate(cx, true);
}

fn prompt_open_repository(cx: &mut App, backend: Arc<dyn GitBackend>) {
    let source_window_id = find_normal_gitcomet_window(cx).map(|entry| entry.handle.window_id());
    let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open Git Repository".into()),
    });

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let result = rx.await;
        let paths = match result {
            Ok(Ok(Some(paths))) => paths,
            Ok(Ok(None)) => return,
            Ok(Err(_)) | Err(_) => {
                cx.update(move |cx| {
                    if let Some(window) =
                        source_window_id.and_then(|id| normal_gitcomet_window_by_id(cx, id))
                    {
                        show_open_repository_manual_entry_in_window(cx, &window, true);
                    } else {
                        show_open_repository_manual_entry_in_existing_or_new_window(
                            cx,
                            Arc::clone(&backend),
                        );
                    }
                });
                return;
            }
        };
        let Some(path) = paths.into_iter().next() else {
            return;
        };

        cx.update(move |cx| {
            if let Some(window) =
                source_window_id.and_then(|id| normal_gitcomet_window_by_id(cx, id))
            {
                open_repository_in_window(cx, &window, normalize_repository_open_path(path));
            } else {
                open_repository_in_existing_or_new_window(cx, Arc::clone(&backend), path);
            }
        });
    })
    .detach();
}

#[cfg(target_os = "macos")]
fn prompt_apply_patch(cx: &mut App) {
    if find_normal_gitcomet_window(cx).is_none() {
        return;
    }

    let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Select patch file".into()),
    });

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let result = rx.await;
        let paths = match result {
            Ok(Ok(Some(paths))) => paths,
            Ok(Ok(None)) => return,
            Ok(Err(_)) | Err(_) => return,
        };
        let Some(patch) = paths.into_iter().next() else {
            return;
        };

        cx.update(move |cx| {
            let Some(window) = find_normal_gitcomet_window(cx) else {
                return;
            };
            let patch_for_window = patch.clone();
            let _ = window.view.update(cx, |view, cx| {
                view.apply_patch_from_file(patch_for_window, cx);
            });
            if cx.active_window().map(|active| active.window_id())
                != Some(window.handle.window_id())
            {
                activate_gitcomet_window(cx, window.handle);
            }
        });
    })
    .detach();
}

#[cfg(target_os = "macos")]
pub(crate) fn ensure_graphics_device_available(context: &'static str) -> Result<(), UiLaunchError> {
    if metal::Device::all().is_empty() {
        return Err(UiLaunchError::from_launch_failure(
            context,
            "no compatible Metal graphics device is available in this macOS session. \
             GPUI requires Metal to open windows; launch from an active local GUI session.",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn ensure_graphics_device_available(context: &'static str) -> Result<(), UiLaunchError> {
    let env = crate::linux_gui_env::LinuxGuiEnvironment::detect();
    if env.session_is_gui_capable() {
        return Ok(());
    }

    Err(UiLaunchError::from_launch_failure(
        context,
        env.launch_failure_message(),
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn ensure_graphics_device_available(
    _context: &'static str,
) -> Result<(), UiLaunchError> {
    Ok(())
}

fn bind_text_input_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "enter",
            PushUpstreamRemoteOpenOrSelect,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "space",
            PushUpstreamRemoteOpenOrSelect,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "up",
            PushUpstreamRemotePrev,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "down",
            PushUpstreamRemoteNext,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "escape",
            PushUpstreamRemoteClose,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new("escape", PopoverPromptDismiss, Some("PopoverPrompt")),
        KeyBinding::new("tab", PopoverPromptTabNext, Some("PopoverPrompt")),
        KeyBinding::new("shift-tab", PopoverPromptTabPrev, Some("PopoverPrompt")),
        KeyBinding::new("backspace", crate::kit::Backspace, Some("TextInput")),
        KeyBinding::new("shift-backspace", crate::kit::Backspace, Some("TextInput")),
        KeyBinding::new("delete", crate::kit::Delete, Some("TextInput")),
        KeyBinding::new(
            "ctrl-backspace",
            crate::kit::DeleteWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "ctrl-delete",
            crate::kit::DeleteWordRight,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "alt-backspace",
            crate::kit::DeleteWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new("alt-delete", crate::kit::DeleteWordRight, Some("TextInput")),
        KeyBinding::new(
            "cmd-backspace",
            crate::kit::DeleteToLineStart,
            Some("TextInput"),
        ),
        KeyBinding::new("cmd-delete", crate::kit::DeleteToLineEnd, Some("TextInput")),
        // The Windows/Linux counterpart, as in GTK text views. Plain
        // Ctrl-Backspace/Delete already delete a word there.
        KeyBinding::new(
            "ctrl-shift-backspace",
            crate::kit::DeleteToLineStart,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "ctrl-shift-delete",
            crate::kit::DeleteToLineEnd,
            Some("TextInput"),
        ),
        KeyBinding::new("enter", crate::kit::Enter, Some("TextInput")),
        KeyBinding::new("shift-enter", crate::kit::ShiftEnter, Some("TextInput")),
        KeyBinding::new("secondary-enter", TextInputCommitSubmit, Some("TextInput")),
        KeyBinding::new("f1", TextInputDiffPrevFile, Some("TextInput")),
        KeyBinding::new("f4", TextInputDiffNextFile, Some("TextInput")),
        KeyBinding::new(
            "f2",
            TextInputDiffPrevSearchMatchOrChange,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "f3",
            TextInputDiffNextSearchMatchOrChange,
            Some("TextInput"),
        ),
        KeyBinding::new("shift-f7", TextInputDiffPrevChange, Some("TextInput")),
        KeyBinding::new("f7", TextInputDiffNextChange, Some("TextInput")),
        KeyBinding::new("alt-up", TextInputDiffPrevChange, Some("TextInput")),
        KeyBinding::new("alt-down", TextInputDiffNextChange, Some("TextInput")),
        KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
        KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
        KeyBinding::new("up", crate::kit::Up, Some("TextInput")),
        KeyBinding::new("down", crate::kit::Down, Some("TextInput")),
        // Word navigation (Ctrl on Windows/Linux, Option on macOS)
        KeyBinding::new("ctrl-left", crate::kit::WordLeft, Some("TextInput")),
        KeyBinding::new("ctrl-right", crate::kit::WordRight, Some("TextInput")),
        KeyBinding::new(
            "ctrl-shift-left",
            crate::kit::SelectWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "ctrl-shift-right",
            crate::kit::SelectWordRight,
            Some("TextInput"),
        ),
        KeyBinding::new("alt-left", crate::kit::WordLeft, Some("TextInput")),
        KeyBinding::new("alt-right", crate::kit::WordRight, Some("TextInput")),
        KeyBinding::new(
            "alt-shift-left",
            crate::kit::SelectWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "alt-shift-right",
            crate::kit::SelectWordRight,
            Some("TextInput"),
        ),
        KeyBinding::new("shift-left", crate::kit::SelectLeft, Some("TextInput")),
        KeyBinding::new("shift-right", crate::kit::SelectRight, Some("TextInput")),
        KeyBinding::new("shift-up", crate::kit::SelectUp, Some("TextInput")),
        KeyBinding::new("shift-down", crate::kit::SelectDown, Some("TextInput")),
        KeyBinding::new("home", crate::kit::Home, Some("TextInput")),
        KeyBinding::new("ctrl-home", crate::kit::DocumentHome, Some("TextInput")),
        KeyBinding::new("ctrl-end", crate::kit::DocumentEnd, Some("TextInput")),
        KeyBinding::new("cmd-home", crate::kit::DocumentHome, Some("TextInput")),
        KeyBinding::new("cmd-end", crate::kit::DocumentEnd, Some("TextInput")),
        KeyBinding::new("shift-home", crate::kit::SelectHome, Some("TextInput")),
        KeyBinding::new("end", crate::kit::End, Some("TextInput")),
        KeyBinding::new("shift-end", crate::kit::SelectEnd, Some("TextInput")),
        KeyBinding::new("cmd-left", crate::kit::Home, Some("TextInput")),
        KeyBinding::new("cmd-shift-left", crate::kit::SelectHome, Some("TextInput")),
        KeyBinding::new("cmd-right", crate::kit::End, Some("TextInput")),
        KeyBinding::new("cmd-shift-right", crate::kit::SelectEnd, Some("TextInput")),
        KeyBinding::new("pageup", crate::kit::PageUp, Some("TextInput")),
        KeyBinding::new("shift-pageup", crate::kit::SelectPageUp, Some("TextInput")),
        KeyBinding::new("pagedown", crate::kit::PageDown, Some("TextInput")),
        KeyBinding::new(
            "shift-pagedown",
            crate::kit::SelectPageDown,
            Some("TextInput"),
        ),
        KeyBinding::new("cmd-a", crate::kit::SelectAll, Some("TextInput")),
        KeyBinding::new("ctrl-a", crate::kit::SelectAll, Some("TextInput")),
        KeyBinding::new("cmd-v", crate::kit::Paste, Some("TextInput")),
        KeyBinding::new("ctrl-v", crate::kit::Paste, Some("TextInput")),
        KeyBinding::new("cmd-c", crate::kit::Copy, Some("TextInput")),
        KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        KeyBinding::new("cmd-x", crate::kit::Cut, Some("TextInput")),
        KeyBinding::new("ctrl-x", crate::kit::Cut, Some("TextInput")),
        KeyBinding::new("cmd-z", crate::kit::Undo, Some("TextInput")),
        KeyBinding::new("ctrl-z", crate::kit::Undo, Some("TextInput")),
        KeyBinding::new("cmd-shift-z", crate::kit::Redo, Some("TextInput")),
        KeyBinding::new("ctrl-shift-z", crate::kit::Redo, Some("TextInput")),
        #[cfg(target_os = "macos")]
        KeyBinding::new(
            "ctrl-cmd-space",
            crate::kit::ShowCharacterPalette,
            Some("TextInput"),
        ),
    ]);
}

fn bind_terminal_keys(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", TerminalCopy, Some("Terminal")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", TerminalPaste, Some("Terminal")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", TerminalSelectAll, Some("Terminal")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("secondary-shift-a", TerminalSelectAll, Some("Terminal")),
    ]);
}

#[cfg(test)]
pub(crate) fn bind_text_input_keys_for_test(cx: &mut App) {
    bind_text_input_keys(cx);
}

#[cfg(test)]
pub(crate) fn bind_app_keys_for_test(cx: &mut App) {
    bind_app_keys(cx);
}

#[cfg(test)]
pub(crate) fn bind_terminal_keys_for_test(cx: &mut App) {
    bind_terminal_keys(cx);
}

#[cfg(test)]
pub(crate) fn install_app_shortcuts_for_test(app: &mut App, backend: Arc<dyn GitBackend>) {
    bind_app_keys(app);
    app.set_global(GitCometBackendGlobal(Arc::clone(&backend)));
    install_app_actions(app, backend);
}

#[cfg(test)]
pub(crate) fn windows_owning_repo_for_test(cx: &mut App, path: &Path) -> Vec<gpui::WindowId> {
    gitcomet_window_entries(cx)
        .into_iter()
        .filter(|entry| entry_contains_repo_path(entry, path))
        .map(|entry| entry.handle.window_id())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::point;
    use gpui::{
        Action, Context, FocusHandle, InteractiveElement, IntoElement, Render, Styled, Window, div,
    };

    use crate::test_support::lock_visual_test;
    use gitcomet_core::error::{Error, ErrorKind};
    use gitcomet_core::services::{GitRepository, Result};
    use gitcomet_state::msg::Msg;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    struct TestBackend;

    impl GitBackend for TestBackend {
        fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::Unsupported(
                "test backend does not open repositories",
            )))
        }
    }

    struct NotARepositoryBackend;

    impl GitBackend for NotARepositoryBackend {
        fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::NotARepository))
        }
    }

    struct RecordingOpenBackend {
        opened: mpsc::Sender<PathBuf>,
    }

    impl GitBackend for RecordingOpenBackend {
        fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            let _ = self.opened.send(workdir.to_path_buf());
            Err(Error::new(ErrorKind::Unsupported(
                "recording backend does not open repositories",
            )))
        }
    }

    struct ControlledOpenBackend {
        opened: mpsc::Sender<PathBuf>,
        result: Mutex<mpsc::Receiver<bool>>,
    }

    impl GitBackend for ControlledOpenBackend {
        fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            let _ = self.opened.send(workdir.to_path_buf());
            // Dropping the sender also unblocks the worker if an assertion fails.
            if self.result.lock().unwrap().recv().unwrap_or(false) {
                Ok(Arc::new(
                    gitcomet_core::test_support::UnconfiguredRepository::new(workdir),
                ))
            } else {
                Err(Error::new(ErrorKind::NotARepository))
            }
        }
    }

    #[test]
    fn manual_update_menu_is_disabled_without_a_feedback_window() {
        assert!(manual_update_check_menu_disabled(false, false));
        assert!(manual_update_check_menu_disabled(true, true));
        assert!(!manual_update_check_menu_disabled(false, true));
    }

    #[gpui::test]
    fn manual_update_check_activates_its_feedback_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let main_window_id = cx.update(|window, app| {
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });

        cx.cx.update(crate::view::open_settings_window);
        cx.run_until_parked();
        let settings_window = cx.cx.update(|app| {
            app.windows()
                .into_iter()
                .find(|window| window.window_id() != main_window_id)
                .expect("Settings window should open")
        });
        cx.cx.update(|app| {
            let _ = settings_window.update(app, |_view, window, _cx| window.activate_window());
            assert_ne!(
                app.active_window().map(|window| window.window_id()),
                Some(main_window_id),
                "Settings should own focus before the manual update check"
            );
            assert!(check_for_updates_in_active_or_existing_normal_window(app));
            assert_eq!(
                app.active_window().map(|window| window.window_id()),
                Some(main_window_id),
                "manual feedback must be shown in the window brought to the foreground"
            );
        });
    }

    #[cfg(target_os = "macos")]
    fn menu_action_entries(menu: &Menu) -> Vec<(String, String)> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, action, .. } => {
                    Some((name.to_string(), action.name().to_string()))
                }
                _ => None,
            })
            .collect()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_app_menus_use_gitcomet_terminology_and_actions() {
        let menus = macos_app_menus_with_external_editor(true);
        let app_menu = menus
            .iter()
            .find(|menu| menu.name.as_ref() == "GitComet")
            .expect("GitComet menu");
        let file_menu = menus
            .iter()
            .find(|menu| menu.name.as_ref() == "File")
            .expect("File menu");

        let app_entries = menu_action_entries(app_menu);
        assert_eq!(
            &app_entries[..2],
            &[
                (
                    crate::menu_labels::COMMAND_PALETTE.to_string(),
                    ToggleCommandPalette.name().to_string(),
                ),
                (
                    crate::menu_labels::SETTINGS.to_string(),
                    OpenSettings.name().to_string(),
                ),
            ]
        );

        let file_entries = menu_action_entries(file_menu);
        assert_eq!(
            file_entries,
            vec![
                ("New Window".to_string(), NewWindow.name().to_string()),
                (
                    crate::menu_labels::OPEN_REPOSITORY.to_string(),
                    OpenRepository.name().to_string(),
                ),
                (
                    crate::menu_labels::CLONE_REPOSITORY.to_string(),
                    CloneRepository.name().to_string(),
                ),
                (
                    crate::menu_labels::INITIALIZE_REPOSITORY.to_string(),
                    InitializeRepository.name().to_string(),
                ),
                (
                    "Switch Repository…".to_string(),
                    SwitchRepository.name().to_string(),
                ),
                (
                    "Open Workspace…".to_string(),
                    OpenWorkspace.name().to_string(),
                ),
                (
                    crate::menu_labels::OPEN_IN_CODE_EDITOR.to_string(),
                    OpenInCodeEditor.name().to_string(),
                ),
                (
                    crate::menu_labels::OPEN_IN_FILE_EXPLORER.to_string(),
                    LocateFileInExplorer.name().to_string(),
                ),
                (
                    crate::menu_labels::OPEN_REMOTE_IN_BROWSER.to_string(),
                    OpenRemoteInBrowser.name().to_string(),
                ),
                (
                    crate::menu_labels::APPLY_PATCH.to_string(),
                    ApplyPatch.name().to_string(),
                ),
                (
                    crate::menu_labels::CHECK_FOR_UPDATES.to_string(),
                    CheckForUpdates.name().to_string(),
                ),
                ("Close".to_string(), Close.name().to_string()),
                ("Close Window".to_string(), CloseWindow.name().to_string(),),
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_file_menu_hides_unconfigured_external_editor_action() {
        let menus = macos_app_menus_with_external_editor(false);
        let file_menu = menus
            .iter()
            .find(|menu| menu.name.as_ref() == "File")
            .expect("File menu");
        let entries = menu_action_entries(file_menu);

        assert!(
            entries
                .iter()
                .all(|(_, action)| action != OpenInCodeEditor.name())
        );
        assert!(entries.iter().any(|(label, action)| {
            label == crate::menu_labels::OPEN_IN_FILE_EXPLORER
                && action == LocateFileInExplorer.name()
        }));
    }

    fn seed_worktree_repo(
        cx: &mut gpui::VisualTestContext,
        store: &AppStore,
        view: gpui::Entity<GitCometView>,
    ) {
        store.dispatch(Msg::OpenRepo(
            std::env::temp_dir().join("gitcomet-app-test-repo"),
        ));

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx)
                });
                let _ = window.draw(app);
            });
            cx.run_until_parked();

            let ready = cx.update(|_window, app| !view.read(app).blocks_non_repository_actions());
            if ready {
                return;
            }

            if Instant::now() >= deadline {
                panic!("timed out waiting for the window to leave the Home screen");
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[gpui::test]
    fn review_regression_cold_start_honors_new_window_browser_target_when_workspaces_restore(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let restored_path = std::env::temp_dir().join("gitcomet-cold-start-restored");
        let requested_path = restored_path.clone();
        let mut saved = session::Workspace::new(vec![restored_path]);
        saved.restore_on_launch = true;
        saved.last_activation_order = 7;
        let workspaces = vec![saved];
        let mut launch = normal_launch_config(Some(requested_path), None);
        launch.browser_open_target = BrowserOpenTarget::NewWindow;

        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, workspaces.clone());
            open_initial_gitcomet_windows_after_workspace_initialization(
                app,
                Arc::clone(&backend),
                &launch,
                workspaces,
            );
        });

        assert_eq!(
            cx.update(|app| app.windows().len()),
            2,
            "the requested repository needs its own window beside the restored group"
        );
    }

    #[gpui::test]
    fn review_regression_startup_path_uses_the_last_activated_restored_workspace(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let mut older = session::Workspace::new(vec![
            std::env::temp_dir().join("gitcomet-startup-older-workspace"),
        ]);
        older.restore_on_launch = true;
        older.last_activation_order = 1;
        let mut latest = session::Workspace::new(vec![
            std::env::temp_dir().join("gitcomet-startup-latest-workspace"),
        ]);
        latest.restore_on_launch = true;
        latest.last_activation_order = 9;
        let latest_id = latest.id;
        // Saved order differs from activation order.
        let workspaces = vec![latest, older];
        let requested = std::env::temp_dir().join("gitcomet-startup-requested-repository");
        let mut launch = normal_launch_config(Some(requested.clone()), None);
        launch.browser_open_target = BrowserOpenTarget::ExistingWindow;

        crate::ui_runtime::with_override(
            crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
            || {
                cx.update(|app| {
                    crate::workspaces::initialize_for_test(app, workspaces.clone());
                    open_initial_gitcomet_windows_after_workspace_initialization(
                        app, backend, &launch, workspaces,
                    );
                    assert_eq!(app.windows().len(), 2);
                    let owner = find_normal_gitcomet_window_for_repo(app, &requested).expect(
                        "the startup request must have a window owner before focus events run",
                    );
                    assert_eq!(owner.workspace_id, Some(latest_id));
                });
            },
        );
    }

    #[gpui::test]
    fn review_regression_startup_routing_does_not_depend_on_native_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
        let launch = normal_empty_launch_config(None);
        let first = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
        let second = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
        let mut window_cx = gpui::VisualTestContext::from_window(first.into(), cx);
        window_cx.deactivate_window();

        cx.update(|app| {
            // Reproduce startup before Linux delivers either a native active
            // window or the view's focus notification. Pick the window the
            // arbitrary fallback would not choose, so the test is deterministic.
            app.update_default_global::<GitCometWindowRegistry, _>(|registry, _| {
                registry.last_focused_normal_window = None;
            });
            assert!(app.active_window().is_none());
            let fallback = find_normal_gitcomet_window(app)
                .expect("normal window")
                .handle;
            let preferred = if fallback.window_id() == first.window_id() {
                second
            } else {
                first
            };

            for stale_focus in [false, true] {
                if stale_focus {
                    // A compositor can also still report the previous window
                    // as active while the requested activation is pending.
                    activate_gitcomet_window(app, fallback);
                    mark_gitcomet_window_focused(app, fallback.window_id());
                }
                let path = std::env::temp_dir()
                    .join(format!("gitcomet-startup-pending-focus-{stale_focus}"));
                handle_browser_open_request_with_window(
                    app,
                    Arc::clone(&backend),
                    BrowserOpenRequest {
                        path: Some(path.clone()),
                        target: BrowserOpenTarget::ExistingWindow,
                    },
                    Some(preferred.window_id()),
                );
                let owner =
                    find_normal_gitcomet_window_for_repo(app, &path).expect("repository owner");
                assert_eq!(owner.handle.window_id(), preferred.window_id());

                // The destination wins even when another window has the path.
                handle_browser_open_request_with_window(
                    app,
                    Arc::clone(&backend),
                    BrowserOpenRequest {
                        path: Some(path.clone()),
                        target: BrowserOpenTarget::ExistingWindow,
                    },
                    Some(fallback.window_id()),
                );
                let owners: Vec<_> = gitcomet_window_entries(app)
                    .into_iter()
                    .filter(|entry| entry_contains_repo_path(entry, &path))
                    .map(|entry| entry.handle.window_id())
                    .collect();
                assert_eq!(owners.len(), 2);
                assert!(owners.contains(&preferred.window_id()));
                assert!(owners.contains(&fallback.window_id()));
            }
            assert_eq!(app.windows().len(), 2);
        });
    }

    #[gpui::test]
    fn review_regression_followup_startup_path_is_routed_when_no_workspace_restores(
        cx: &mut gpui::TestAppContext,
    ) {
        let (opened_tx, opened_rx) = mpsc::channel();
        let backend: Arc<dyn GitBackend> = Arc::new(RecordingOpenBackend { opened: opened_tx });
        let requested = std::env::temp_dir().join("gitcomet-fresh-start-requested-repository");
        let launch = normal_launch_config(Some(requested.clone()), None);

        crate::ui_runtime::with_override(
            crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
            || {
                cx.update(|app| {
                    crate::workspaces::initialize_for_test(app, Vec::new());
                    open_initial_gitcomet_windows_after_workspace_initialization(
                        app,
                        Arc::clone(&backend),
                        &launch,
                        Vec::new(),
                    );
                });
            },
        );

        let opened = opened_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("a fresh launch must dispatch its requested repository");
        assert_eq!(opened, requested);
    }

    #[gpui::test]
    fn review_regression_manual_and_picker_opens_use_the_initiating_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let (opened_tx, opened_rx) = mpsc::channel();
        let backend: Arc<dyn GitBackend> = Arc::new(RecordingOpenBackend { opened: opened_tx });
        let path = std::env::temp_dir().join("gitcomet-app-wide-owned-repository");
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

        let (first_store, first_events) = AppStore::new_test(Arc::clone(&backend));
        let first = cx.add_window(|window, cx| {
            GitCometView::new(first_store, first_events, None, window, cx)
        });
        let (second_store, second_events) = AppStore::new_test(Arc::clone(&backend));
        let second_store_for_assertions = second_store.clone();
        let second = cx.add_window(|window, cx| {
            GitCometView::new(second_store, second_events, None, window, cx)
        });

        let repo_id = gitcomet_state::model::RepoId(41);
        let owner_state = gitcomet_state::model::AppState {
            repos: vec![gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: path.clone(),
                },
            )],
            active_repo: Some(repo_id),
            ..gitcomet_state::model::AppState::test_default()
        };
        first
            .update(cx, |view, _window, cx| {
                crate::view::test_support::apply_state_snapshot_for_test(
                    view,
                    Arc::new(owner_state),
                    cx,
                );
            })
            .expect("install the first window's live repository state");

        second
            .update(cx, |view, _window, cx| {
                view.open_repo_path(path.clone(), cx);
            })
            .expect("manually open from the second window");
        second
            .update(cx, |view, _window, cx| {
                crate::view::test_support::activate_closed_repo_picker_entry_for_test(
                    view,
                    path.clone(),
                    cx,
                );
            })
            .expect("activate a closed repository-picker row");
        cx.background_executor.run_until_parked();

        assert_eq!(
            opened_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("open in the second window"),
            path
        );
        let owners = cx.update(|app| {
            gitcomet_window_entries(app)
                .into_iter()
                .filter(|entry| entry_contains_repo_path(entry, &path))
                .map(|entry| entry.handle.window_id())
                .collect::<Vec<_>>()
        });
        assert_eq!(owners.len(), 2);
        assert!(owners.contains(&first.window_id()));
        assert!(owners.contains(&second.window_id()));
        assert_eq!(second_store_for_assertions.snapshot().repos.len(), 1);
        assert!(
            opened_rx.try_recv().is_err(),
            "reopening in the same window must reuse its tab"
        );
    }

    #[gpui::test]
    fn review_regression_followup_restoring_multiple_workspaces_runs_startup_hooks_once(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let workspaces = (0..3)
            .map(|index| {
                let mut workspace = session::Workspace::new(vec![
                    std::env::temp_dir().join(format!("gitcomet-startup-hook-group-{index}")),
                ]);
                // Test runtime disables automatic repo restore. Keep the named
                // workspace mapped even while its window is empty.
                workspace.custom_name = Some(format!("Startup {index}"));
                workspace.restore_on_launch = true;
                workspace.last_activation_order = index;
                workspace
            })
            .collect::<Vec<_>>();
        let expected_workspace = workspaces.last().unwrap().id;
        let launch = normal_empty_launch_config(None);

        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, workspaces.clone());
            open_initial_gitcomet_windows_after_workspace_initialization(
                app,
                Arc::clone(&backend),
                &launch,
                workspaces,
            );
        });

        assert_eq!(
            cx.update(startup_hook_invocation_count_for_test),
            1,
            "survey and update startup hooks are process work, not per-window work"
        );
        cx.update(|app| {
            let window = app.global::<StartupHookInvocationCount>().1.unwrap();
            assert_eq!(
                crate::workspaces::workspace_for_window(app, window)
                    .unwrap()
                    .id,
                expected_workspace,
                "startup hooks belong to the most recently active restored workspace"
            );
        });
    }

    #[gpui::test]
    fn pr530_browser_request_handler_closes_its_receiver_on_shutdown(
        cx: &mut gpui::TestAppContext,
    ) {
        let app = cx.new_app();
        let (requests, received) = smol::channel::unbounded();
        app.update(|app| {
            register_browser_open_request_handler(app, Arc::new(TestBackend), received)
        });
        app.background_executor.run_until_parked();
        assert!(!requests.is_closed());
        app.quit();
        assert!(
            requests.is_closed(),
            "shutdown must stop accepting forwarded requests"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[gpui::test]
    fn pr530_closing_last_main_window_with_settings_keeps_workspace_restorable(
        cx: &mut gpui::TestAppContext,
    ) {
        let _guard = lock_visual_test();
        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, Vec::new());
            let main = open_gitcomet_window(
                app,
                Arc::new(TestBackend),
                &normal_empty_launch_config(None),
            );
            let workspace = crate::workspaces::sync_window(
                app,
                main.window_id(),
                None,
                vec![PathBuf::from("/tmp/pr530-last-main")],
                None,
            )
            .unwrap();
            crate::view::open_settings_window(app);
            assert_eq!(app.windows().len(), 2);
            mark_window_closing(app, main.window_id());
            assert!(
                crate::workspaces::workspace(app, workspace)
                    .unwrap()
                    .restore_on_launch,
                "Settings must not turn last-main-window close into workspace removal"
            );
        });
    }

    #[gpui::test]
    fn review_regression_restoring_overlapping_workspaces_keeps_both_windows(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let shared = std::env::temp_dir().join("gitcomet-restored-shared");
        let other = std::env::temp_dir().join("gitcomet-restored-other");
        let mut first = session::Workspace::new(vec![shared.clone(), other.clone()]);
        first.active_repository = Some(shared.clone());
        first.last_activation_order = 1;
        let mut second = session::Workspace::new(vec![shared.clone(), other.clone()]);
        second.active_repository = Some(other);
        second.last_activation_order = 2;
        let workspaces = vec![first, second];
        crate::ui_runtime::with_override(
            crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
            || {
                cx.update(|app| {
                    crate::workspaces::initialize_for_test(app, workspaces.clone());
                    open_initial_gitcomet_windows_after_workspace_initialization(
                        app,
                        backend,
                        &normal_empty_launch_config(None),
                        workspaces.clone(),
                    );
                })
            },
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.run_until_parked();
            let restored = cx.update(|app| {
                let windows = gitcomet_window_entries(app);
                assert_eq!(windows.len(), 2);
                windows.iter().all(|entry| {
                    let saved = workspaces
                        .iter()
                        .find(|workspace| Some(workspace.id) == entry.workspace_id)
                        .unwrap();
                    entry
                        .view
                        .update(app, |view, cx| {
                            crate::view::test_support::sync_store_snapshot(view, cx);
                            let (pending, count, active) =
                                crate::view::test_support::startup_repository_state_for_test(view);
                            !pending && count == 2 && active == saved.active_repository
                        })
                        .unwrap()
                })
            });
            if restored {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "restore each workspace's independent active tab"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        cx.update(|app| {
            assert_eq!(windows_owning_repo_for_test(app, &shared).len(), 2);
            for saved in workspaces {
                let restored = crate::workspaces::workspace(app, saved.id).unwrap();
                assert_eq!(restored.repositories, saved.repositories);
                assert_eq!(restored.active_repository, saved.active_repository);
            }
        });
    }

    #[gpui::test]
    fn review_regression_native_picker_keeps_its_initiating_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
        let launch = normal_empty_launch_config(None);
        let first = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
        let second = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
        let path = std::env::temp_dir().join("gitcomet-picker-shared");
        cx.update(|app| {
            let first_entry = normal_gitcomet_window_by_id(app, first.window_id()).unwrap();
            open_repository_in_window(app, &first_entry, path.clone());
            activate_gitcomet_window(app, second.into());
            prompt_open_repository(app, Arc::clone(&backend));
            // Focus can change while the native dialog is open.
            activate_gitcomet_window(app, first.into());
        });
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| Some(vec![path.clone()]));
        cx.run_until_parked();
        cx.update(|app| {
            let owners = windows_owning_repo_for_test(app, &path);
            assert_eq!(owners.len(), 2);
            assert!(owners.contains(&first.window_id()));
            assert!(owners.contains(&second.window_id()));
            assert_eq!(app.active_window().unwrap().window_id(), second.window_id());
        });
    }

    #[gpui::test]
    fn review_regression_folder_drop_reuses_only_the_destination_windows_tab(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let (opened_tx, opened_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let backend: Arc<dyn GitBackend> = Arc::new(ControlledOpenBackend {
            opened: opened_tx,
            result: Mutex::new(result_rx),
        });
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
        let folder = tempfile::tempdir().unwrap();
        let path = normalize_repository_open_path(folder.path().to_path_buf());
        let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
        let first = cx.add_window(|window, cx| {
            GitCometView::new(first_store, first_events, None, window, cx)
        });
        first
            .update(cx, |view, _, cx| view.open_repo_path(path.clone(), cx))
            .unwrap();
        cx.run_until_parked();
        let (store, events) = AppStore::new_test(backend);
        let store_for_view = store.clone();
        let second =
            cx.add_window(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
        for dropped_path in [path.clone(), path.join(".")] {
            second
                .update(cx, |_, _, cx| {
                    open_dropped_repository_from_view(cx, second.window_id(), dropped_path);
                })
                .unwrap();
            cx.run_until_parked();
        }
        assert_eq!(
            opened_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            path
        );
        assert_eq!(store.snapshot().repos.len(), 1);
        cx.update(|app| {
            let windows = windows_owning_repo_for_test(app, &path);
            assert_eq!(windows.len(), 2);
            assert!(windows.contains(&first.window_id()));
            assert!(windows.contains(&second.window_id()));
            assert!(
                crate::workspaces::workspace_for_window(app, second.window_id()).is_none(),
                "the dropped copy must validate before it is saved"
            );
        });
        result_tx.send(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            second
                .update(cx, |view, _, cx| {
                    crate::view::test_support::sync_store_snapshot(view, cx)
                })
                .unwrap();
            cx.run_until_parked();
            if cx.update(|app| {
                crate::workspaces::workspace_for_window(app, second.window_id()).is_some()
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "save the validated drop");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            opened_rx.try_recv().is_err(),
            "same-window drops must share one validation"
        );
        first
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        cx.update(|app| {
            assert_eq!(
                windows_owning_repo_for_test(app, &path),
                vec![second.window_id()]
            );
            assert_eq!(
                crate::workspaces::workspace_for_window(app, second.window_id())
                    .unwrap()
                    .repositories,
                vec![path]
            );
        });
        assert_eq!(store.snapshot().repos.len(), 1);
    }

    #[gpui::test]
    fn review_regression_forwarded_open_falls_back_to_the_last_focused_normal_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

        let (first_store, first_events) = AppStore::new_test(Arc::clone(&backend));
        let first = cx.add_window(|window, cx| {
            GitCometView::new(first_store, first_events, None, window, cx)
        });
        let (second_store, second_events) = AppStore::new_test(Arc::clone(&backend));
        let second = cx.add_window(|window, cx| {
            GitCometView::new(second_store, second_events, None, window, cx)
        });

        first
            .update(cx, |_view, window, _cx| window.activate_window())
            .expect("activate a window before simulating app blur");
        cx.background_executor.run_until_parked();
        let mut first_context = gpui::VisualTestContext::from_window(first.into(), cx);
        first_context.deactivate_window();
        let arbitrary_fallback = cx
            .update(find_normal_gitcomet_window)
            .expect("normal window")
            .handle
            .window_id();
        let expected = if arbitrary_fallback == first.window_id() {
            gpui::AnyWindowHandle::from(second)
        } else {
            gpui::AnyWindowHandle::from(first)
        };
        expected
            .update(cx, |_view, window, _cx| window.activate_window())
            .expect("activate the non-fallback window");
        cx.background_executor.run_until_parked();
        let mut expected_context = gpui::VisualTestContext::from_window(expected, cx);
        expected_context.deactivate_window();

        let selected = cx
            .update(find_normal_gitcomet_window)
            .expect("normal window after app blur");
        assert_eq!(
            selected.handle.window_id(),
            expected.window_id(),
            "an external terminal leaves no active GPUI window, so the remembered focus must win"
        );
    }

    #[gpui::test]
    fn review_regression_recovering_a_saved_workspace_keeps_overlapping_repositories(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let duplicate_path = std::env::temp_dir().join("gitcomet-app-test-repo");
        let mut saved = session::Workspace::new(vec![duplicate_path.clone()]);
        saved.restore_on_launch = false;
        let saved_id = saved.id;
        cx.update(|app| crate::workspaces::initialize_for_test(app, vec![saved]));

        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        seed_worktree_repo(cx, &store, view);
        assert!(cx.cx.update(|app| {
            find_normal_gitcomet_window_for_repo(app, &duplicate_path).is_some()
        }));

        let recovered = crate::ui_runtime::with_override(
            crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
            || {
                cx.cx.update(|app| {
                    activate_or_open_workspace(app, saved_id).expect("open the saved workspace")
                })
            },
        );
        cx.run_until_parked();

        assert_eq!(
            cx.cx.update(|app| app.windows().len()),
            2,
            "each workspace needs its own window even when its repositories overlap"
        );
        cx.cx.update(|app| {
            let saved =
                crate::workspaces::workspace(app, saved_id).expect("retain saved workspace");
            assert_eq!(saved.repositories, vec![duplicate_path.clone()]);
            assert_eq!(recovered.workspace_id, Some(saved_id));
            let again = activate_or_open_workspace(app, saved_id).expect("focus the workspace");
            assert_eq!(again.handle.window_id(), recovered.handle.window_id());
            assert_eq!(app.windows().len(), 2);
        });
    }

    #[gpui::test]
    fn review_regression_move_to_a_saved_workspace_with_the_same_repository(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let path = std::env::temp_dir().join("gitcomet-app-test-repo");
        let mut saved = session::Workspace::new(vec![path.clone()]);
        saved.restore_on_launch = false;
        let saved_id = saved.id;
        cx.update(|app| crate::workspaces::initialize_for_test(app, vec![saved]));

        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let source_window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view);
        let repo_id = store.snapshot().repos[0].id;

        crate::ui_runtime::with_override(
            crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
            || {
                cx.cx.update(|app| {
                    move_repository_to_workspace(
                        app,
                        source_window_id,
                        repo_id,
                        path.clone(),
                        Some(saved_id),
                    );
                })
            },
        );
        cx.run_until_parked();

        let (window_count, owners) = cx.cx.update(|app| {
            (
                app.windows().len(),
                gitcomet_window_entries(app)
                    .into_iter()
                    .filter(|entry| entry_contains_repo_path(entry, &path))
                    .map(|entry| entry.handle.window_id())
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(window_count, 1, "moving the only tab closes the source");
        assert_eq!(owners.len(), 1);
        assert_ne!(owners[0], source_window_id);
        cx.cx.update(|app| {
            let destination = normal_gitcomet_window_by_id(app, owners[0]).unwrap();
            assert_eq!(destination.workspace_id, Some(saved_id));
            assert_eq!(destination.repo_paths.as_ref(), &[path]);
        });
    }

    /// Closing the last window quits on Linux/Windows, so it must keep the
    /// workspace restorable; closing one of several windows still hides it.
    #[cfg(not(target_os = "macos"))]
    #[gpui::test]
    fn review_regression_closing_the_last_window_keeps_its_workspace_restorable(
        cx: &mut gpui::TestAppContext,
    ) {
        let first = cx.add_window(|_, _| gpui::Empty);
        let last = cx.add_window(|_, _| gpui::Empty);
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
        let (first_id, last_id) = cx.update(|app| {
            let sync = |app: &mut App, window: gpui::WindowId, repo: &str| {
                crate::workspaces::sync_window(app, window, None, vec![PathBuf::from(repo)], None)
                    .expect("durable workspace")
            };
            (
                sync(app, first.window_id(), "/repos/a"),
                sync(app, last.window_id(), "/repos/b"),
            )
        });
        let restores = |cx: &mut gpui::TestAppContext, id| {
            cx.update(|app| crate::workspaces::workspace(app, id))
                .expect("workspace kept")
                .restore_on_launch
        };

        first
            .update(cx, |_, window, app| close_window_or_warn(window, app))
            .unwrap();
        assert!(
            !restores(cx, first_id),
            "a closed non-final window is hidden"
        );

        last.update(cx, |_, window, app| close_window_or_warn(window, app))
            .unwrap();
        assert!(
            restores(cx, last_id),
            "closing the final window quits, so its workspace must restore"
        );
    }

    /// A stale registry entry must not prevent reopening a distinct workspace,
    /// even when that workspace contains the same repository as the source.
    #[gpui::test]
    fn review_regression_move_noop_check_ignores_closed_windows(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let path = std::env::temp_dir().join("gitcomet-app-test-repo");
        let mut saved = session::Workspace::new(vec![path.clone()]);
        saved.restore_on_launch = false;
        let saved_id = saved.id;
        cx.update(|app| crate::workspaces::initialize_for_test(app, vec![saved]));
        let closed = cx.add_window(|_, _| gpui::Empty);
        let closed_handle: gpui::AnyWindowHandle = closed.into();

        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let source_window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view);

        closed
            .update(cx, |_, window, _| window.remove_window())
            .expect("close the stale window");
        cx.run_until_parked();
        cx.cx.update(|app| {
            app.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
                registry.windows.insert(
                    closed_handle.window_id(),
                    GitCometWindowEntry {
                        handle: closed_handle,
                        view: gpui::WeakEntity::new_invalid(),
                        main_pane: gpui::WeakEntity::new_invalid(),
                        view_mode: GitCometViewMode::Normal,
                        workspace_id: Some(saved_id),
                        repo_paths: Arc::from(Vec::new()),
                    },
                );
            });
        });

        assert!(
            !cx.cx.update(|app| repository_move_target_is_noop(
                app,
                source_window_id,
                Some(saved_id)
            )),
            "a closed workspace with the same repository is a valid move destination"
        );
    }

    #[gpui::test]
    fn review_regression_forwarded_opens_activate_the_application_after_routing(
        cx: &mut gpui::TestAppContext,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, Vec::new());
            let window =
                open_gitcomet_window(app, Arc::clone(&backend), &normal_empty_launch_config(None));
            let existing = std::env::temp_dir().join("gitcomet-forwarded-existing");
            let preferred = std::env::temp_dir().join("gitcomet-forwarded-preferred");
            let new_window = std::env::temp_dir().join("gitcomet-forwarded-new");
            let scenarios = [
                (None, BrowserOpenTarget::ExistingWindow, None),
                (
                    Some(existing.clone()),
                    BrowserOpenTarget::ExistingWindow,
                    None,
                ),
                (
                    Some(preferred),
                    BrowserOpenTarget::ExistingWindow,
                    Some(window.window_id()),
                ),
                (Some(existing), BrowserOpenTarget::NewWindow, None),
                (Some(new_window), BrowserOpenTarget::NewWindow, None),
            ];
            for (path, target, preferred_window) in scenarios {
                let activated = Cell::new(false);
                handle_browser_open_request_and_activate(
                    app,
                    Arc::clone(&backend),
                    BrowserOpenRequest {
                        path: path.clone(),
                        target,
                    },
                    preferred_window,
                    |app, ignoring_other_apps| {
                        assert!(
                            ignoring_other_apps,
                            "a forwarded open must foreground a backgrounded app"
                        );
                        let active = app
                            .active_window()
                            .expect("route to a window before activating the app");
                        if let Some(path) = path.as_ref() {
                            let registry = app.global::<GitCometWindowRegistry>();
                            let owner = registry
                                .windows
                                .get(&active.window_id())
                                .expect("active repository window");
                            assert!(
                                entry_contains_repo_path(owner, path),
                                "open the repository before activating the app"
                            );
                        }
                        activated.set(true);
                    },
                );
                assert!(
                    activated.get(),
                    "every forwarded-open route must activate the application"
                );
            }
            assert_eq!(
                app.windows().len(),
                3,
                "every request targeting a new window creates one"
            );
        });
    }

    fn check_provisional_drop_window_locality(cx: &mut gpui::TestAppContext, succeeds: bool) {
        let _visual_guard = lock_visual_test();
        let (opened_tx, opened_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let backend: Arc<dyn GitBackend> = Arc::new(ControlledOpenBackend {
            opened: opened_tx,
            result: Mutex::new(result_rx),
        });
        let mut move_target = session::Workspace::new(Vec::new());
        move_target.custom_name = Some("Move target".to_string());
        let move_target_id = move_target.id;
        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, vec![move_target]);
            app.set_global(GitCometBackendGlobal(Arc::clone(&backend)));
        });
        let directory = tempfile::tempdir().expect("create repository directories");
        let root = normalize_repository_open_path(directory.path().to_path_buf());
        let base = root.join("base");
        let dropped = root.join("dropped");
        std::fs::create_dir(&base).unwrap();
        std::fs::create_dir(&dropped).unwrap();
        let repo_id = gitcomet_state::model::RepoId(51);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let mut repo = gitcomet_state::model::RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: base.clone(),
            },
        );
        repo.open = gitcomet_state::model::Loadable::Ready(());
        store.insert_repo_for_test(
            repo_id,
            Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                base.clone(),
            )),
        );
        store.replace_snapshot_for_test(Arc::new(gitcomet_state::model::AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..gitcomet_state::model::AppState::test_default()
        }));
        let store_for_view = store.clone();
        let source =
            cx.add_window(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
        let (other_store, other_events) = AppStore::new_test(Arc::new(TestBackend));
        let other = cx.add_window(|window, cx| {
            GitCometView::new(other_store, other_events, None, window, cx)
        });

        // Check routing before the view receives the store's provisional tab,
        // then check it again while backend validation is deliberately blocked.
        for validating in [false, true] {
            if validating {
                assert_eq!(
                    opened_rx
                        .recv_timeout(Duration::from_secs(3))
                        .expect("start validation"),
                    dropped
                );
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    let snapshot = store.snapshot();
                    if snapshot
                        .repos
                        .iter()
                        .any(|repo| repo.spec.workdir == dropped)
                    {
                        let dropped_id = snapshot
                            .repos
                            .iter()
                            .find(|repo| repo.spec.workdir == dropped)
                            .unwrap()
                            .id;
                        assert!(
                            !session::snapshot_repos_from_state(&snapshot)
                                .open_repos
                                .iter()
                                .any(|path| session::path_from_storage_key(path) == dropped)
                        );
                        source
                            .update(cx, |view, _, cx| {
                                crate::view::test_support::apply_state_snapshot_for_test(
                                    view, snapshot, cx,
                                );
                            })
                            .unwrap();
                        for target in [None, Some(move_target_id)] {
                            source
                                .update(cx, |view, _, cx| {
                                    view.request_move_repo_to_workspace(
                                        dropped_id,
                                        dropped.clone(),
                                        target,
                                        cx,
                                    );
                                })
                                .unwrap();
                            cx.run_until_parked();
                            cx.update(|app| {
                                // The deferred execution boundary also rejects a
                                // stale menu/confirmation trying to bypass the UI.
                                move_repository_to_workspace(
                                    app,
                                    source.window_id(),
                                    dropped_id,
                                    dropped.clone(),
                                    target,
                                );
                                assert_eq!(
                                    app.windows().len(),
                                    2,
                                    "validation must finish before moving a drop"
                                );
                                assert!(
                                    crate::workspaces::workspace(app, move_target_id)
                                        .unwrap()
                                        .repositories
                                        .is_empty()
                                );
                            });
                        }
                        break;
                    }
                    assert!(Instant::now() < deadline, "publish the provisional tab");
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                source
                    .update(cx, |view, _, cx| {
                        view.open_dropped_repo_locally(dropped.clone(), cx)
                    })
                    .unwrap();
            }

            other
                .update(cx, |view, _, cx| view.open_repo_path(dropped.clone(), cx))
                .unwrap();
            source
                .update(cx, |view, _, cx| {
                    view.open_repo_path(dropped.clone(), cx);
                    open_dropped_repository_from_view(cx, source.window_id(), dropped.clone());
                })
                .unwrap();
            cx.run_until_parked();
            cx.update(|app| {
                for window in [source.window_id(), other.window_id()] {
                    handle_browser_open_request_with_window(
                        app,
                        Arc::clone(&backend),
                        BrowserOpenRequest {
                            path: Some(dropped.clone()),
                            target: BrowserOpenTarget::ExistingWindow,
                        },
                        Some(window),
                    );
                }
                let owners: Vec<_> = gitcomet_window_entries(app)
                    .into_iter()
                    .filter(|entry| entry_contains_repo_path(entry, &dropped))
                    .map(|entry| entry.handle.window_id())
                    .collect();
                assert_eq!(owners.len(), 2);
                assert!(owners.contains(&source.window_id()));
                assert!(owners.contains(&other.window_id()));
                assert_eq!(
                    app.windows().len(),
                    2,
                    "existing-window requests must reuse their destination window"
                );
                let workspace = crate::workspaces::workspace_for_window(app, source.window_id())
                    .expect("original workspace");
                assert_eq!(
                    workspace.repositories,
                    vec![base.clone()],
                    "unvalidated paths must stay out of saved workspaces"
                );
                assert_eq!(workspace.active_repository, Some(base.clone()));
                assert_eq!(
                    crate::workspaces::workspace_for_window(app, other.window_id())
                        .expect("independent explicit open")
                        .repositories,
                    vec![dropped.clone()]
                );
            });
        }

        result_tx.send(succeeds).expect("finish validation");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let snapshot = store.snapshot();
            let finished = if succeeds {
                snapshot.repos.iter().any(|repo| {
                    repo.spec.workdir == dropped
                        && matches!(repo.open, gitcomet_state::model::Loadable::Ready(()))
                })
            } else {
                snapshot
                    .repo_open_failures
                    .get(&dropped)
                    .copied()
                    .unwrap_or_default()
                    > 0
            };
            if finished {
                source
                    .update(cx, |view, _, cx| {
                        crate::view::test_support::apply_state_snapshot_for_test(
                            view, snapshot, cx,
                        );
                    })
                    .unwrap();
                break;
            }
            assert!(Instant::now() < deadline, "finish the provisional open");
            std::thread::sleep(Duration::from_millis(10));
        }
        cx.update(|app| {
            let source_entry = normal_gitcomet_window_by_id(app, source.window_id()).unwrap();
            assert_eq!(entry_contains_repo_path(&source_entry, &dropped), succeeds);
            let other_entry = normal_gitcomet_window_by_id(app, other.window_id()).unwrap();
            assert!(entry_contains_repo_path(&other_entry, &dropped));
            let workspace = crate::workspaces::workspace_for_window(app, source.window_id())
                .expect("original workspace");
            assert_eq!(
                workspace.repositories,
                if succeeds {
                    vec![base.clone(), dropped.clone()]
                } else {
                    vec![base.clone()]
                }
            );
            assert_eq!(app.windows().len(), 2);
        });
        assert!(
            opened_rx.try_recv().is_err(),
            "reopening a provisional tab in the same window must not restart validation"
        );
    }

    #[gpui::test]
    fn review_regression_provisional_drop_stays_window_local_until_validation_succeeds(
        cx: &mut gpui::TestAppContext,
    ) {
        check_provisional_drop_window_locality(cx, true);
    }

    #[gpui::test]
    fn review_regression_failed_provisional_drop_does_not_remove_another_windows_copy(
        cx: &mut gpui::TestAppContext,
    ) {
        check_provisional_drop_window_locality(cx, false);
    }

    #[gpui::test]
    fn review_regression_pending_new_window_opens_allow_the_same_repository(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let path = std::env::temp_dir().join("gitcomet-pending-new-window-owner");
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

        cx.update(|app| {
            for _ in 0..2 {
                handle_browser_open_request(
                    app,
                    Arc::clone(&backend),
                    BrowserOpenRequest {
                        path: Some(path.clone()),
                        target: BrowserOpenTarget::NewWindow,
                    },
                );
            }

            assert_eq!(
                app.windows().len(),
                2,
                "new-window requests must open separate windows even while loading"
            );
            let owners = gitcomet_window_entries(app)
                .into_iter()
                .filter(|entry| entry_contains_repo_path(entry, &path))
                .count();
            assert_eq!(owners, 2, "each destination reserves its own pending tab");
        });
    }

    #[gpui::test]
    fn review_regression_confirmed_failed_open_releases_pending_path(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(NotARepositoryBackend);
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let path = std::env::temp_dir().join("gitcomet-failed-pending-open");
        cx.cx.update(|app| {
            handle_browser_open_request(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: Some(path.clone()),
                    target: BrowserOpenTarget::ExistingWindow,
                },
            );
            assert_eq!(
                gitcomet_window_entries(app)
                    .iter()
                    .filter(|entry| entry_contains_repo_path(entry, &path))
                    .count(),
                1,
                "the path must be reserved while its open attempt is pending"
            );
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx);
                });
            });
            cx.run_until_parked();
            let released = cx.cx.update(|app| {
                gitcomet_window_entries(app)
                    .iter()
                    .all(|entry| !entry_contains_repo_path(entry, &path))
                    && crate::workspaces::workspaces(app)
                        .iter()
                        .all(|workspace| !workspace.repositories.contains(&path))
            });
            if released && store.snapshot().repo_open_failure_revision > 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a failed open must release its pending app-wide ownership"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[gpui::test]
    fn review_regression_confirmed_existing_window_open_waits_for_git_recovery(
        cx: &mut gpui::TestAppContext,
    ) {
        use gitcomet_core::process::{
            GitExecutableAvailability, GitExecutablePreference, GitRuntimeState,
        };
        use gitcomet_state::model::AppState;

        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        store.replace_snapshot_for_test(Arc::new(AppState {
            git_runtime: GitRuntimeState {
                preference: GitExecutablePreference::Custom(PathBuf::new()),
                availability: GitExecutableAvailability::Unavailable {
                    detail: "Git unavailable for broker routing test".to_string(),
                },
            },
            ..AppState::test_default()
        }));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let path = std::env::temp_dir().join("gitcomet-broker-open-after-git-recovery");
        cx.cx.update(|app| {
            handle_browser_open_request(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: Some(path.clone()),
                    target: BrowserOpenTarget::ExistingWindow,
                },
            );
        });
        cx.run_until_parked();
        assert!(
            store.snapshot().repos.is_empty(),
            "the unavailable reducer must not start the repository open yet"
        );

        store.dispatch(Msg::SetGitRuntimeState(GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability: GitExecutableAvailability::Available {
                version_output: "git version test".to_string(),
            },
        }));

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx);
                });
            });
            cx.run_until_parked();
            if store
                .snapshot()
                .repos
                .iter()
                .any(|repo| repo.spec.workdir == path)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the acknowledged broker request must resume once Git is available"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            store
                .snapshot()
                .repos
                .iter()
                .any(|repo| repo.spec.workdir == path),
            "the acknowledged broker request must resume once Git is available"
        );
    }

    #[gpui::test]
    fn review_regression_lifecycle_cold_start_activates_a_requested_inactive_saved_tab(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let initially_active = std::env::temp_dir().join("gitcomet-cold-start-initially-active");
        let requested = std::env::temp_dir().join("gitcomet-cold-start-requested-inactive");
        let mut saved = session::Workspace::new(vec![initially_active.clone(), requested.clone()]);
        saved.active_repository = Some(initially_active.clone());
        saved.restore_on_launch = true;
        saved.last_activation_order = 9;
        let saved_id = saved.id;
        let saved_paths = saved.repositories.clone();
        let launch = normal_empty_launch_config(None);

        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, vec![saved]);
            let window = open_gitcomet_window(app, Arc::clone(&backend), &launch);
            let owner = normal_gitcomet_window_by_id(app, window.window_id())
                .expect("normal window registry entry");
            sync_gitcomet_window_registry(
                app,
                owner.handle,
                owner.view.clone(),
                owner.main_pane.clone(),
                GitCometViewMode::Normal,
                Some(saved_id),
                saved_paths.into(),
            );
            owner
                .view
                .update(app, |view, _cx| {
                    crate::view::test_support::dispatch_restore_session_for_test(
                        view,
                        vec![initially_active.clone(), requested.clone()],
                        Some(initially_active.clone()),
                    );
                })
                .expect("queue saved tab restoration");
            handle_browser_open_request(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: Some(requested.clone()),
                    target: BrowserOpenTarget::ExistingWindow,
                },
            );
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        let active = loop {
            cx.update(|app| {
                for entry in gitcomet_window_entries(app) {
                    let _ = entry.view.update(app, |view, cx| {
                        crate::view::test_support::sync_store_snapshot(view, cx);
                    });
                }
            });
            cx.run_until_parked();

            let startup = cx.update(|app| {
                find_normal_gitcomet_window_for_repo(app, &requested).and_then(|entry| {
                    entry
                        .view
                        .read_with(app, |view, _cx| {
                            crate::view::test_support::startup_repository_state_for_test(view)
                        })
                        .ok()
                })
            });
            if let Some((false, 2, active)) = startup {
                break active;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for the saved repository tabs to restore");
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(
            active,
            Some(requested),
            "the startup request must win over the group's previously active tab"
        );
    }

    #[gpui::test]
    fn review_regression_closing_immediately_flushes_the_current_workspace_layout(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        cx.update(|app| {
            crate::workspaces::initialize_for_test(app, Vec::new());
            ui_scale::set_current(app, 100);
        });
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view.clone());
        let workspace_id = cx
            .cx
            .update(|app| crate::workspaces::workspace_for_window(app, window_id))
            .expect("durable live group")
            .id;
        let width = px(517.0);
        let expected_width = cx.update(|_window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::set_sidebar_width_for_test(this, width, cx);
                ui_scale::stored_design_units(Some(
                    ui_scale::UiScale::current(cx).design_units_from_pixels(width),
                ))
            })
        });

        cx.cx
            .update(|app| close_window_by_id_or_warn(app, window_id));
        cx.run_until_parked();

        let saved = cx
            .cx
            .update(|app| crate::workspaces::workspace(app, workspace_id))
            .expect("closed group remains recoverable");
        assert_eq!(saved.layout.sidebar_width, expected_width);
    }

    #[gpui::test]
    fn review_regression_colliding_restored_window_frames_are_cascaded(
        cx: &mut gpui::TestAppContext,
    ) {
        let placement = session::PortableWindowPlacement {
            normal_frame: Some(session::SavedWindowFrame {
                x: 120,
                y: 90,
                width: 900,
                height: 650,
            }),
            captured_visible_frame: None,
            display_id: None,
            state: session::SavedWindowState::Windowed,
            tiled: None,
        };
        let fallback_size = size(px(900.0), px(650.0));
        let min_size = size(px(820.0), px(560.0));

        let first = cx.update(|app| {
            restored_workspace_window_bounds(&placement, fallback_size, min_size, app).0
        });
        let _first_window = cx.update(|app| {
            app.open_window(
                WindowOptions {
                    window_bounds: Some(first),
                    ..Default::default()
                },
                |_window, cx| cx.new(|_| gpui::Empty),
            )
            .expect("open the first restored frame")
        });
        let second = cx.update(|app| {
            restored_workspace_window_bounds(&placement, fallback_size, min_size, app).0
        });

        assert_ne!(
            first.get_bounds(),
            second.get_bounds(),
            "identical saved frames must not restore directly on top of one another"
        );
    }

    #[test]
    fn window_zoom_action_restores_only_on_windows_when_already_maximized() {
        assert_eq!(window_zoom_action(false), WindowZoomAction::Zoom);

        let expected = if cfg!(target_os = "windows") {
            WindowZoomAction::Restore
        } else {
            WindowZoomAction::Zoom
        };
        assert_eq!(window_zoom_action(true), expected);
    }

    #[test]
    fn window_menu_position_scales_logical_pixels_to_device_pixels() {
        assert_eq!(
            window_menu_position(point(px(12.4), px(7.6)), 1.25),
            (16, 10)
        );
    }

    #[test]
    fn maximized_and_fullscreen_captures_keep_the_last_windowed_frame() {
        let previous_frame = session::SavedWindowFrame {
            x: 100,
            y: 80,
            width: 1100,
            height: 720,
        };
        let reported_screen_frame = session::SavedWindowFrame {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let previous = session::PortableWindowPlacement {
            normal_frame: Some(previous_frame),
            ..Default::default()
        };

        for state in [
            session::SavedWindowState::Maximized,
            session::SavedWindowState::Fullscreen,
        ] {
            assert_eq!(
                captured_normal_frame(state, reported_screen_frame, Some(&previous)),
                previous_frame
            );
        }
        assert_eq!(
            captured_normal_frame(
                session::SavedWindowState::Windowed,
                reported_screen_frame,
                Some(&previous)
            ),
            reported_screen_frame
        );
    }

    struct KeyBindingProbe {
        focus_handle: FocusHandle,
        key_context: Option<&'static str>,
        observed_actions: Arc<Mutex<Vec<String>>>,
    }

    impl KeyBindingProbe {
        fn new(
            key_context: Option<&'static str>,
            observed_actions: Arc<Mutex<Vec<String>>>,
            cx: &mut Context<Self>,
        ) -> Self {
            Self {
                focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
                key_context,
                observed_actions,
            }
        }

        fn focus_handle(&self) -> FocusHandle {
            self.focus_handle.clone()
        }

        fn record_action(&self, action_name: &str) {
            self.observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(action_name.to_string());
        }
    }

    impl Render for KeyBindingProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            macro_rules! record_action_listener {
                ($action:path) => {
                    cx.listener(|this, _: &$action, _window, _cx| {
                        this.record_action($action.name());
                    })
                };
            }

            let root = div()
                .size_full()
                .track_focus(&self.focus_handle)
                .on_action(record_action_listener!(crate::kit::Backspace))
                .on_action(record_action_listener!(crate::kit::Delete))
                .on_action(record_action_listener!(crate::kit::DeleteWordLeft))
                .on_action(record_action_listener!(crate::kit::DeleteWordRight))
                .on_action(record_action_listener!(crate::kit::DeleteToLineStart))
                .on_action(record_action_listener!(crate::kit::DeleteToLineEnd))
                .on_action(record_action_listener!(crate::kit::Enter))
                .on_action(record_action_listener!(crate::kit::ShiftEnter))
                .on_action(record_action_listener!(crate::kit::Left))
                .on_action(record_action_listener!(crate::kit::Right))
                .on_action(record_action_listener!(crate::kit::Up))
                .on_action(record_action_listener!(crate::kit::Down))
                .on_action(record_action_listener!(crate::kit::WordLeft))
                .on_action(record_action_listener!(crate::kit::WordRight))
                .on_action(record_action_listener!(crate::kit::SelectLeft))
                .on_action(record_action_listener!(crate::kit::SelectRight))
                .on_action(record_action_listener!(crate::kit::SelectUp))
                .on_action(record_action_listener!(crate::kit::SelectDown))
                .on_action(record_action_listener!(crate::kit::SelectWordLeft))
                .on_action(record_action_listener!(crate::kit::SelectWordRight))
                .on_action(record_action_listener!(crate::kit::SelectAll))
                .on_action(record_action_listener!(crate::kit::Home))
                .on_action(record_action_listener!(crate::kit::DocumentHome))
                .on_action(record_action_listener!(crate::kit::DocumentEnd))
                .on_action(record_action_listener!(crate::kit::SelectHome))
                .on_action(record_action_listener!(crate::kit::End))
                .on_action(record_action_listener!(crate::kit::SelectEnd))
                .on_action(record_action_listener!(crate::kit::PageUp))
                .on_action(record_action_listener!(crate::kit::SelectPageUp))
                .on_action(record_action_listener!(crate::kit::PageDown))
                .on_action(record_action_listener!(crate::kit::SelectPageDown))
                .on_action(record_action_listener!(crate::kit::Paste))
                .on_action(record_action_listener!(crate::kit::Cut))
                .on_action(record_action_listener!(crate::kit::Copy))
                .on_action(record_action_listener!(crate::kit::Undo))
                .on_action(record_action_listener!(crate::kit::Redo))
                .on_action(record_action_listener!(crate::view::DiffPrevFile))
                .on_action(record_action_listener!(crate::view::DiffNextFile))
                .on_action(record_action_listener!(
                    crate::view::DiffPrevSearchMatchOrChange
                ))
                .on_action(record_action_listener!(
                    crate::view::DiffNextSearchMatchOrChange
                ))
                .on_action(record_action_listener!(crate::view::TextInputCommitSubmit))
                .on_action(record_action_listener!(crate::view::TextInputDiffPrevFile))
                .on_action(record_action_listener!(crate::view::TextInputDiffNextFile))
                .on_action(record_action_listener!(
                    crate::view::TextInputDiffPrevSearchMatchOrChange
                ))
                .on_action(record_action_listener!(
                    crate::view::TextInputDiffNextSearchMatchOrChange
                ))
                .on_action(record_action_listener!(
                    crate::view::TextInputDiffPrevChange
                ))
                .on_action(record_action_listener!(
                    crate::view::TextInputDiffNextChange
                ))
                .on_action(record_action_listener!(crate::view::OpenActiveViewSearch))
                .on_action(record_action_listener!(crate::view::ToggleCommandPalette))
                .on_action(record_action_listener!(crate::view::ToggleRevealCommit))
                .on_action(record_action_listener!(crate::view::LocateFileInExplorer))
                .on_action(record_action_listener!(crate::view::OpenRemoteInBrowser))
                .on_action(record_action_listener!(NewWindow))
                .on_action(record_action_listener!(OpenSettings))
                .on_action(record_action_listener!(OpenInCodeEditor))
                .on_action(record_action_listener!(OpenRepository))
                .on_action(record_action_listener!(SwitchRepository))
                .on_action(record_action_listener!(ShowReflog))
                .on_action(record_action_listener!(Close))
                .on_action(record_action_listener!(CloseWindow))
                .on_action(record_action_listener!(PreviousRepository))
                .on_action(record_action_listener!(NextRepository))
                .on_action(record_action_listener!(TerminalCopy))
                .on_action(record_action_listener!(TerminalPaste))
                .on_action(record_action_listener!(TerminalSelectAll))
                .on_action(record_action_listener!(MinimizeWindow))
                .on_action(record_action_listener!(ZoomWindow))
                .on_action(record_action_listener!(ToggleFullScreen))
                .on_action(record_action_listener!(IncreaseUiScale))
                .on_action(record_action_listener!(DecreaseUiScale))
                .on_action(record_action_listener!(ResetUiScale))
                .on_action(record_action_listener!(Hide))
                .on_action(record_action_listener!(HideOthers))
                .on_action(record_action_listener!(ShowAll))
                .on_action(record_action_listener!(Quit));

            #[cfg(target_os = "macos")]
            let root = root.on_action(record_action_listener!(crate::kit::ShowCharacterPalette));

            if let Some(key_context) = self.key_context {
                root.key_context(key_context)
            } else {
                root
            }
        }
    }

    #[test]
    fn focused_mergetool_title_uses_file_name_when_available() {
        let title = focused_mergetool_window_title(Path::new("/repo/src/conflict.txt"));
        assert_eq!(title, "GitComet - Mergetool (conflict.txt)");
    }

    #[test]
    fn focused_mergetool_launch_config_sets_focused_view_mode_and_repo() {
        let config = FocusedMergetoolConfig {
            repo_path: PathBuf::from("/repo"),
            conflicted_file_path: PathBuf::from("/repo/src/conflict.txt"),
            label_local: "LOCAL".to_string(),
            label_remote: "REMOTE".to_string(),
            label_base: "BASE".to_string(),
        };

        let launch = focused_mergetool_launch_config(&config, None);
        assert_eq!(launch.app_id, "gitcomet-mergetool");
        assert_eq!(launch.title, "GitComet - Mergetool (conflict.txt)");
        assert_eq!(launch.view_config.initial_path, Some(config.repo_path));
        assert_eq!(
            launch.view_config.view_mode,
            GitCometViewMode::FocusedMergetool
        );
        assert_eq!(
            launch.view_config.focused_mergetool,
            Some(FocusedMergetoolViewConfig {
                repo_path: PathBuf::from("/repo"),
                conflicted_file_path: PathBuf::from("/repo/src/conflict.txt"),
                labels: FocusedMergetoolLabels {
                    local: "LOCAL".to_string(),
                    remote: "REMOTE".to_string(),
                    base: "BASE".to_string(),
                },
            })
        );
        assert!(launch.view_config.focused_mergetool_exit_code.is_none());
    }

    #[test]
    fn focused_mergetool_exit_codes_match_mergetool_contract() {
        assert_eq!(FOCUSED_MERGETOOL_EXIT_SUCCESS, 0);
        assert_eq!(FOCUSED_MERGETOOL_EXIT_CANCELED, 1);
        assert_eq!(FOCUSED_MERGETOOL_EXIT_ERROR, 2);
    }

    #[gpui::test]
    fn text_input_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            KeyBindingProbe::new(Some("TextInput"), Arc::clone(&observed_actions), cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_text_input_keys(app);
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        let cases: Vec<(&str, &'static str)> = vec![
            ("backspace", crate::kit::Backspace.name()),
            ("shift-backspace", crate::kit::Backspace.name()),
            ("delete", crate::kit::Delete.name()),
            ("ctrl-backspace", crate::kit::DeleteWordLeft.name()),
            ("ctrl-delete", crate::kit::DeleteWordRight.name()),
            ("alt-backspace", crate::kit::DeleteWordLeft.name()),
            ("alt-delete", crate::kit::DeleteWordRight.name()),
            ("cmd-backspace", crate::kit::DeleteToLineStart.name()),
            ("cmd-delete", crate::kit::DeleteToLineEnd.name()),
            ("ctrl-shift-backspace", crate::kit::DeleteToLineStart.name()),
            ("ctrl-shift-delete", crate::kit::DeleteToLineEnd.name()),
            ("enter", crate::kit::Enter.name()),
            ("shift-enter", crate::kit::ShiftEnter.name()),
            ("secondary-enter", crate::view::TextInputCommitSubmit.name()),
            ("f1", crate::view::TextInputDiffPrevFile.name()),
            ("f4", crate::view::TextInputDiffNextFile.name()),
            (
                "f2",
                crate::view::TextInputDiffPrevSearchMatchOrChange.name(),
            ),
            (
                "f3",
                crate::view::TextInputDiffNextSearchMatchOrChange.name(),
            ),
            ("shift-f7", crate::view::TextInputDiffPrevChange.name()),
            ("f7", crate::view::TextInputDiffNextChange.name()),
            ("alt-up", crate::view::TextInputDiffPrevChange.name()),
            ("alt-down", crate::view::TextInputDiffNextChange.name()),
            ("left", crate::kit::Left.name()),
            ("right", crate::kit::Right.name()),
            ("up", crate::kit::Up.name()),
            ("down", crate::kit::Down.name()),
            ("ctrl-left", crate::kit::WordLeft.name()),
            ("ctrl-right", crate::kit::WordRight.name()),
            ("ctrl-shift-left", crate::kit::SelectWordLeft.name()),
            ("ctrl-shift-right", crate::kit::SelectWordRight.name()),
            ("alt-left", crate::kit::WordLeft.name()),
            ("alt-right", crate::kit::WordRight.name()),
            ("alt-shift-left", crate::kit::SelectWordLeft.name()),
            ("alt-shift-right", crate::kit::SelectWordRight.name()),
            ("shift-left", crate::kit::SelectLeft.name()),
            ("shift-right", crate::kit::SelectRight.name()),
            ("shift-up", crate::kit::SelectUp.name()),
            ("shift-down", crate::kit::SelectDown.name()),
            ("home", crate::kit::Home.name()),
            ("ctrl-home", crate::kit::DocumentHome.name()),
            ("ctrl-end", crate::kit::DocumentEnd.name()),
            ("cmd-home", crate::kit::DocumentHome.name()),
            ("cmd-end", crate::kit::DocumentEnd.name()),
            ("shift-home", crate::kit::SelectHome.name()),
            ("end", crate::kit::End.name()),
            ("shift-end", crate::kit::SelectEnd.name()),
            ("cmd-left", crate::kit::Home.name()),
            ("cmd-shift-left", crate::kit::SelectHome.name()),
            ("cmd-right", crate::kit::End.name()),
            ("cmd-shift-right", crate::kit::SelectEnd.name()),
            ("pageup", crate::kit::PageUp.name()),
            ("shift-pageup", crate::kit::SelectPageUp.name()),
            ("pagedown", crate::kit::PageDown.name()),
            ("shift-pagedown", crate::kit::SelectPageDown.name()),
            ("cmd-a", crate::kit::SelectAll.name()),
            ("ctrl-a", crate::kit::SelectAll.name()),
            ("cmd-v", crate::kit::Paste.name()),
            ("ctrl-v", crate::kit::Paste.name()),
            ("cmd-c", crate::kit::Copy.name()),
            ("ctrl-c", crate::kit::Copy.name()),
            ("cmd-x", crate::kit::Cut.name()),
            ("ctrl-x", crate::kit::Cut.name()),
            ("cmd-z", crate::kit::Undo.name()),
            ("ctrl-z", crate::kit::Undo.name()),
            ("cmd-shift-z", crate::kit::Redo.name()),
            ("ctrl-shift-z", crate::kit::Redo.name()),
        ];

        #[cfg(target_os = "macos")]
        let cases = {
            let mut cases = cases;
            cases.push(("ctrl-cmd-space", crate::kit::ShowCharacterPalette.name()));
            cases
        };

        for (keystroke, expected_action) in cases {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_action = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .cloned();
            assert_eq!(
                actual_action.as_deref(),
                Some(expected_action),
                "expected `{keystroke}` to resolve to `{expected_action}`"
            );
        }
    }

    #[gpui::test]
    fn text_input_diff_keybindings_stay_scoped_when_app_keys_are_installed(
        cx: &mut gpui::TestAppContext,
    ) {
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            KeyBindingProbe::new(Some("TextInput"), Arc::clone(&observed_actions), cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_app_keys(app);
            bind_text_input_keys(app);
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        let cases = [
            ("f1", crate::view::TextInputDiffPrevFile.name()),
            ("f4", crate::view::TextInputDiffNextFile.name()),
            (
                "f2",
                crate::view::TextInputDiffPrevSearchMatchOrChange.name(),
            ),
            (
                "f3",
                crate::view::TextInputDiffNextSearchMatchOrChange.name(),
            ),
            ("secondary-shift-a", SwitchRepository.name()),
            // No text-input binding (an Emacs kill-line, say) may shadow it.
            ("secondary-k", crate::view::OpenRemoteInBrowser.name()),
        ];

        for (keystroke, expected_action) in cases {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_actions = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            assert_eq!(
                actual_actions,
                vec![expected_action.to_string()],
                "expected `{keystroke}` to resolve only to the TextInput-scoped diff action"
            );
        }
    }

    #[gpui::test]
    fn terminal_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            KeyBindingProbe::new(Some("Terminal"), Arc::clone(&observed_actions), cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_app_keys(app);
            bind_terminal_keys_for_test(app);
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        #[cfg(target_os = "macos")]
        let cases = [
            ("cmd-c", TerminalCopy.name()),
            ("cmd-v", TerminalPaste.name()),
            ("cmd-a", TerminalSelectAll.name()),
        ];

        #[cfg(not(target_os = "macos"))]
        let cases = [
            ("ctrl-shift-c", TerminalCopy.name()),
            ("ctrl-shift-v", TerminalPaste.name()),
            ("secondary-shift-a", TerminalSelectAll.name()),
        ];

        for (keystroke, expected_action) in cases {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_action = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .cloned();
            assert_eq!(
                actual_action.as_deref(),
                Some(expected_action),
                "expected `{keystroke}` to resolve to `{expected_action}`"
            );
        }

        for keystroke in ["ctrl-c", "ctrl-v", "ctrl-a"] {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_actions = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            assert!(
                actual_actions.is_empty(),
                "expected `{keystroke}` to remain shell input, got {actual_actions:?}"
            );
        }
    }

    #[gpui::test]
    fn text_input_command_shortcuts_trigger_undo_and_redo(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            crate::kit::TextInput::new(crate::kit::TextInputOptions::default(), window, cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_text_input_keys(app);
            let focus = input.read(app).focus_handle();
            window.focus(&focus, app);

            input.update(app, |input, cx| {
                input.set_text("alpha", cx);
                let inserted = input.replace_utf8_range(0..5, "beta", cx);
                assert_eq!(inserted, 0..4);
            });
            let _ = window.draw(app);
        });

        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            cx.update(|_window, app| input.read(app).text().to_string()),
            "alpha"
        );

        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(
            cx.update(|_window, app| input.read(app).text().to_string()),
            "beta"
        );
    }

    #[gpui::test]
    fn text_input_control_redo_shortcut_triggers_redo(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            crate::kit::TextInput::new(crate::kit::TextInputOptions::default(), window, cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_text_input_keys(app);
            let focus = input.read(app).focus_handle();
            window.focus(&focus, app);

            input.update(app, |input, cx| {
                input.set_text("alpha", cx);
                let inserted = input.replace_utf8_range(0..5, "beta", cx);
                assert_eq!(inserted, 0..4);
            });
            let _ = window.draw(app);
        });

        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(
            cx.update(|_window, app| input.read(app).text().to_string()),
            "alpha"
        );

        cx.simulate_keystrokes("ctrl-shift-z");
        assert_eq!(
            cx.update(|_window, app| input.read(app).text().to_string()),
            "beta"
        );
    }

    #[test]
    fn should_quit_when_all_windows_closed_depends_on_launch_mode() {
        let normal = normal_launch_config(None, None);
        let focused = focused_mergetool_launch_config(
            &FocusedMergetoolConfig {
                repo_path: PathBuf::from("/repo"),
                conflicted_file_path: PathBuf::from("/repo/conflict.txt"),
                label_local: "LOCAL".to_string(),
                label_remote: "REMOTE".to_string(),
                label_base: "BASE".to_string(),
            },
            None,
        );

        #[cfg(target_os = "macos")]
        assert!(!should_quit_when_all_windows_closed(&normal));
        #[cfg(not(target_os = "macos"))]
        assert!(should_quit_when_all_windows_closed(&normal));
        assert!(should_quit_when_all_windows_closed(&focused));
    }

    #[test]
    fn normal_launch_config_keeps_startup_paths_in_restore_session_mode() {
        let launch = normal_launch_config(Some(PathBuf::from("/repo")), None);

        assert_eq!(
            launch.view_config.initial_path,
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(
            launch.view_config.initial_repository_launch_mode,
            InitialRepositoryLaunchMode::RestoreSession
        );
    }

    #[test]
    fn explicit_repository_launch_config_marks_initial_path_as_explicit() {
        let launch = normal_launch_config_with_initial_repository(PathBuf::from("/repo"), None);

        assert_eq!(
            launch.view_config.initial_path,
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(
            launch.view_config.initial_repository_launch_mode,
            InitialRepositoryLaunchMode::OpenExplicitly
        );
    }

    #[test]
    fn recent_repository_label_formats_repo_name_and_parent() {
        let label = recent_repository_label(Path::new("/Users/sampo/projects/gitcomet"));
        assert_eq!(label, "gitcomet - /Users/sampo/projects");
    }

    #[test]
    fn recent_repository_label_falls_back_to_display_when_file_name_is_missing() {
        let path = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
        assert_eq!(recent_repository_label(&path), path.display().to_string());
    }

    #[gpui::test]
    fn app_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
        let _external_editor_guard =
            crate::external_editor::configured_setting_override_test_guard();
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            KeyBindingProbe::new(None, Arc::clone(&observed_actions), cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_app_keys(app);
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        let mut cases = vec![
            ("secondary-n", NewWindow.name()),
            ("secondary-shift-n", NewWindow.name()),
            ("secondary-,", OpenSettings.name()),
            ("secondary-o", OpenRepository.name()),
            ("secondary-shift-o", SwitchRepository.name()),
            ("secondary-shift-a", SwitchRepository.name()),
            ("secondary-f", crate::view::OpenActiveViewSearch.name()),
            ("secondary-p", crate::view::ToggleCommandPalette.name()),
            ("secondary-g", crate::view::ToggleRevealCommit.name()),
            (
                "secondary-shift-l",
                crate::view::LocateFileInExplorer.name(),
            ),
            ("secondary-k", crate::view::OpenRemoteInBrowser.name()),
            ("secondary-w", Close.name()),
            ("secondary-shift-w", CloseWindow.name()),
            ("secondary-pageup", PreviousRepository.name()),
            ("secondary-pagedown", NextRepository.name()),
            ("secondary-+", IncreaseUiScale.name()),
            ("secondary-=", IncreaseUiScale.name()),
            ("secondary--", DecreaseUiScale.name()),
            ("secondary-0", ResetUiScale.name()),
            ("secondary-q", Quit.name()),
            ("f1", crate::view::DiffPrevFile.name()),
            ("f4", crate::view::DiffNextFile.name()),
            ("f2", crate::view::DiffPrevSearchMatchOrChange.name()),
            ("f3", crate::view::DiffNextSearchMatchOrChange.name()),
        ];

        #[cfg(target_os = "macos")]
        cases.extend([
            ("alt-cmd-o", SwitchRepository.name()),
            ("cmd-{", PreviousRepository.name()),
            ("alt-cmd-left", PreviousRepository.name()),
            ("cmd-}", NextRepository.name()),
            ("alt-cmd-right", NextRepository.name()),
            ("cmd-m", MinimizeWindow.name()),
            ("ctrl-cmd-f", ToggleFullScreen.name()),
            ("cmd-h", Hide.name()),
            ("alt-cmd-h", HideOthers.name()),
        ]);

        #[cfg(not(target_os = "macos"))]
        cases.extend([
            ("ctrl-shift-tab", PreviousRepository.name()),
            ("ctrl-tab", NextRepository.name()),
            ("f11", ToggleFullScreen.name()),
        ]);

        for (keystroke, expected_action) in cases {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_action = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .cloned();
            assert_eq!(
                actual_action.as_deref(),
                Some(expected_action),
                "expected `{keystroke}` to resolve to `{expected_action}`"
            );
        }

        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.simulate_keystrokes("secondary-shift-e");
        assert!(
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty(),
            "expected external editor shortcut to be unbound by default"
        );
    }

    #[gpui::test]
    fn external_editor_shortcut_respects_configured_setting_override(
        cx: &mut gpui::TestAppContext,
    ) {
        let _external_editor_guard =
            crate::external_editor::configured_setting_override_test_guard();
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            KeyBindingProbe::new(None, Arc::clone(&observed_actions), cx)
        });

        cx.update(|window, app| {
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        crate::external_editor::set_configured_setting_override(Some(
            session::ExternalCodeEditorSetting::Custom {
                executable: PathBuf::from("/usr/bin/editor"),
                arguments: None,
            },
        ));
        cx.update(|_window, app| {
            app.clear_key_bindings();
            bind_app_keys(app);
        });

        cx.simulate_keystrokes("secondary-shift-e");
        let actual_action = observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned();
        assert_eq!(actual_action.as_deref(), Some(OpenInCodeEditor.name()));

        crate::external_editor::set_configured_setting_override(None);
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.update(|_window, app| {
            bind_app_keys(app);
        });

        cx.simulate_keystrokes("secondary-shift-e");
        assert!(
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty(),
            "expected external editor shortcut to be unbound after the configured override is cleared"
        );
    }

    #[gpui::test]
    fn settings_shortcut_opens_a_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        seed_worktree_repo(cx, &store, view);

        assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
        cx.simulate_keystrokes("secondary-,");
        cx.run_until_parked();
        assert_eq!(cx.update(|_window, app| app.windows().len()), 2);
    }

    #[gpui::test]
    fn settings_shortcut_reuses_existing_window_and_activates_it(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let main_window = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle()
        });
        let main_window_id = main_window.window_id();

        cx.simulate_keystrokes("secondary-,");
        cx.run_until_parked();

        let settings_window_id = cx.cx.update(|app| {
            assert_eq!(app.windows().len(), 2);
            app.windows()
                .into_iter()
                .map(|window| window.window_id())
                .find(|window_id| *window_id != main_window_id)
                .expect("expected the settings window to be open")
        });

        cx.cx.update(|app| {
            let _ = main_window.update(app, |_, window, _cx| {
                window.activate_window();
            });
            assert_eq!(
                app.active_window().map(|window| window.window_id()),
                Some(main_window_id),
                "expected the main window to become active before reopening settings"
            );
        });

        cx.simulate_keystrokes("secondary-,");
        cx.run_until_parked();

        cx.cx.update(|app| {
            assert_eq!(app.windows().len(), 2);
            assert_eq!(
                app.active_window().map(|window| window.window_id()),
                Some(settings_window_id),
                "expected reopening settings to activate the existing settings window"
            );
        });
    }

    #[gpui::test]
    fn recent_picker_shortcut_toggles_the_popover(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        seed_worktree_repo(cx, &store, view);

        cx.simulate_keystrokes("secondary-shift-a");
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(cx.debug_bounds("app_popover").is_some());

        cx.simulate_keystrokes("secondary-shift-a");
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(
            cx.debug_bounds("app_popover").is_none(),
            "pressing the shortcut again should close the repository picker"
        );
    }

    #[gpui::test]
    fn new_window_shortcuts_open_new_windows(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        seed_worktree_repo(cx, &store, view);

        assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
        cx.simulate_keystrokes("secondary-n");
        cx.run_until_parked();
        assert_eq!(cx.update(|_window, app| app.windows().len()), 2);
        let mut repo_counts = cx.cx.update(|app| {
            gitcomet_window_entries(app)
                .into_iter()
                .map(|entry| entry.repo_paths.len())
                .collect::<Vec<_>>()
        });
        repo_counts.sort_unstable();
        assert_eq!(
            repo_counts,
            vec![0, 1],
            "a new window must not copy repositories from the active window"
        );

        cx.simulate_keystrokes("secondary-shift-n");
        cx.run_until_parked();
        assert_eq!(cx.update(|_window, app| app.windows().len()), 3);
        let mut repo_counts = cx.cx.update(|app| {
            gitcomet_window_entries(app)
                .into_iter()
                .map(|entry| entry.repo_paths.len())
                .collect::<Vec<_>>()
        });
        repo_counts.sort_unstable();
        assert_eq!(repo_counts, vec![0, 0, 1]);
    }

    fn empty_window_for_adoption(
        cx: &mut gpui::TestAppContext,
        workspaces: Vec<session::Workspace>,
    ) -> (
        gpui::Entity<GitCometView>,
        &mut gpui::VisualTestContext,
        gpui::WindowId,
    ) {
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let window_id = cx.update(|window, app| {
            crate::workspaces::initialize_for_test(app, workspaces);
            install_app_shortcuts_for_test(app, backend);
            let _ = window.draw(app);
            window.window_handle().window_id()
        });
        cx.update(|_window, app| {
            view.update(app, |view, cx| {
                crate::view::test_support::sync_store_snapshot(view, cx)
            });
        });
        (view, cx, window_id)
    }

    #[gpui::test]
    fn adopting_an_overlapping_workspace_reuses_the_empty_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let repositories = vec![PathBuf::from("/tmp/adopt-a"), PathBuf::from("/tmp/adopt-b")];
        let mut workspace = session::Workspace::new(repositories.clone());
        workspace.restore_on_launch = false;
        let id = workspace.id;
        let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace]);
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let other = cx
            .cx
            .add_window(|window, cx| GitCometView::new(store, events, None, window, cx));
        other
            .update(&mut cx.cx, |view, _, cx| {
                view.open_repo_path_locally(repositories[0].clone(), cx)
            })
            .unwrap();

        cx.update(|_window, app| open_workspace_in_window(app, window_id, id));

        cx.cx.update(|app| {
            assert_eq!(
                app.windows().len(),
                2,
                "adoption must reuse the empty window"
            );
            let adopted = crate::workspaces::workspace_for_window(app, window_id)
                .expect("the window now belongs to the workspace");
            assert_eq!(adopted.id, id);
            // The store has not reduced the restore yet; the snapshot taken
            // during adoption must not have emptied (and so deleted) it.
            assert_eq!(adopted.repositories, repositories);
            assert!(adopted.restore_on_launch);
            assert_eq!(windows_owning_repo_for_test(app, &repositories[0]).len(), 2);
        });
    }

    #[gpui::test]
    fn opening_a_workspace_owned_by_another_window_leaves_this_window_empty(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let mut workspace = session::Workspace::new(vec![PathBuf::from("/tmp/adopt-owned")]);
        // Test views do not auto-restore repositories; a name keeps the owner's
        // workspace alive while its window stays empty.
        workspace.custom_name = Some("Owned".to_string());
        let id = workspace.id;
        let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace.clone()]);
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let config = crate::view::GitCometViewConfig {
            workspace: WorkspaceBootstrap::Saved(Box::new(workspace)),
            ..crate::view::GitCometViewConfig::normal(None)
        };
        let owner = cx.cx.add_window(|window, cx| {
            GitCometView::new_with_config(store, events, config, window, cx)
        });
        cx.cx.update(|app| {
            let any_owner: gpui::AnyWindowHandle = owner.into();
            let _ = any_owner.update(app, |_root, window, cx| {
                let _ = window.draw(cx);
            });
            let _ = owner.update(app, |view, _window, cx| {
                crate::view::test_support::sync_store_snapshot(view, cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(
            cx.cx.update(|app| {
                normal_gitcomet_window_by_id(app, owner.window_id()).and_then(|e| e.workspace_id)
            }),
            Some(id),
            "the owner window registers the workspace"
        );

        cx.update(|_window, app| open_workspace_in_window(app, window_id, id));

        cx.cx.update(|app| {
            assert!(crate::workspaces::workspace_for_window(app, window_id).is_none());
            assert_eq!(
                crate::workspaces::workspace_for_window(app, owner.window_id()).map(|w| w.id),
                Some(id)
            );
        });
    }

    #[gpui::test]
    fn adopting_an_empty_customized_workspace_keeps_the_window_on_home(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let mut workspace = session::Workspace::new(Vec::new());
        workspace.custom_name = Some("Later".to_string());
        workspace.restore_on_launch = false;
        let id = workspace.id;
        let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace]);

        cx.update(|_window, app| open_workspace_in_window(app, window_id, id));

        cx.cx.update(|app| {
            assert_eq!(app.windows().len(), 1);
            let adopted = crate::workspaces::workspace_for_window(app, window_id).expect("adopted");
            assert_eq!(adopted.id, id);
            assert!(adopted.repositories.is_empty());
        });
    }

    #[gpui::test]
    fn closing_the_active_window_saves_a_layout_change_the_debounce_has_not_written(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let mut workspace = session::Workspace::new(Vec::new());
        workspace.custom_name = Some("Layout".to_string());
        let id = workspace.id;
        let (view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace]);
        cx.update(|window, app| {
            open_workspace_in_window(app, window_id, id);
            window.activate_window();
        });
        cx.update(|_window, app| {
            view.update(app, |view, cx| {
                crate::view::test_support::set_sidebar_width_for_test(view, px(333.0), cx);
            });
        });

        cx.update(|_window, app| close_active_window(app));

        let saved = cx
            .cx
            .update(|app| crate::workspaces::workspace(app, id))
            .expect("a customized workspace outlives its window");
        assert_eq!(saved.layout.sidebar_width, Some(333));
        assert_eq!(
            saved.restore_on_launch,
            !cfg!(target_os = "macos"),
            "closing the only window quits off macOS, so only there is it restored"
        );
    }

    #[gpui::test]
    fn deleting_a_workspace_closes_its_window_when_another_window_remains(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let mut alpha = session::Workspace::new(Vec::new());
        alpha.custom_name = Some("Alpha".into());
        let alpha_id = alpha.id;
        let (view, cx, window_id) = empty_window_for_adoption(cx, vec![alpha.clone()]);
        cx.update(|_window, app| view.update(app, |view, cx| view.adopt_workspace(alpha, cx)));
        cx.run_until_parked();
        cx.cx.update(open_new_empty_window);
        cx.run_until_parked();
        let other = cx.cx.update(|app| {
            app.windows()
                .into_iter()
                .map(|window| window.window_id())
                .find(|id| *id != window_id)
                .expect("a second window")
        });

        cx.cx.update(|app| delete_workspace(app, alpha_id));
        cx.run_until_parked();

        cx.cx.update(|app| {
            let open: Vec<_> = app
                .windows()
                .iter()
                .map(|window| window.window_id())
                .collect();
            assert_eq!(
                open,
                vec![other],
                "only the deleted workspace's window closes"
            );
            assert!(crate::workspaces::workspace(app, alpha_id).is_none());
        });
    }

    #[gpui::test]
    fn deleting_the_last_windows_workspace_returns_it_to_home(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let window_id = cx.update(|window, app| {
            crate::workspaces::initialize_for_test(app, Vec::new());
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view.clone());
        let workspace_id = cx.cx.update(|app| {
            crate::workspaces::workspace_for_window(app, window_id)
                .expect("the seeded repository makes the window durable")
                .id
        });
        cx.cx
            .update(|app| crate::workspaces::set_workspace_name(app, workspace_id, "Alpha"));

        cx.cx.update(|app| delete_workspace(app, workspace_id));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|window, app| {
                view.update(app, |view, cx| {
                    crate::view::test_support::sync_store_snapshot(view, cx)
                });
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            let emptied = cx.cx.update(|app| {
                normal_gitcomet_window_by_id(app, window_id)
                    .is_some_and(|entry| entry.repo_paths.is_empty())
            });
            if emptied {
                break;
            }
            assert!(Instant::now() < deadline, "the window's repositories close");
            std::thread::sleep(Duration::from_millis(10));
        }

        cx.cx.update(|app| {
            assert!(
                app.windows()
                    .iter()
                    .any(|window| window.window_id() == window_id),
                "the last window stays open on Home"
            );
            // Still gone once the repositories have closed, even customized.
            assert!(crate::workspaces::workspace(app, workspace_id).is_none());
            assert!(crate::workspaces::workspace_for_window(app, window_id).is_none());
            assert!(crate::workspaces::workspaces(app).is_empty());
            assert_eq!(
                normal_gitcomet_window_by_id(app, window_id).and_then(|entry| entry.workspace_id),
                None
            );
        });
    }

    #[gpui::test]
    fn deleting_a_saved_workspace_only_forgets_it(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let mut saved = session::Workspace::new(vec![PathBuf::from("/tmp/saved-workspace")]);
        saved.restore_on_launch = false;
        let saved_id = saved.id;
        let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![saved]);

        cx.cx.update(|app| delete_workspace(app, saved_id));
        cx.run_until_parked();

        cx.cx.update(|app| {
            assert!(crate::workspaces::workspace(app, saved_id).is_none());
            let open: Vec<_> = app
                .windows()
                .iter()
                .map(|window| window.window_id())
                .collect();
            assert_eq!(open, vec![window_id], "no window owned it");
        });
    }

    #[gpui::test]
    fn moving_the_last_repository_out_of_a_customized_workspace_keeps_the_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let source_window_id = cx.update(|window, app| {
            crate::workspaces::initialize_for_test(app, Vec::new());
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view.clone());
        let snapshot = store.snapshot();
        let repo = snapshot.repos.first().expect("source repo");
        let (repo_id, path) = (repo.id, repo.spec.workdir.clone());
        let source_workspace = cx.cx.update(|app| {
            crate::workspaces::workspace_for_window(app, source_window_id)
                .expect("the seeded repository makes the window durable")
                .id
        });
        cx.cx
            .update(|app| crate::workspaces::set_workspace_name(app, source_workspace, "Alpha"));

        cx.update(|_window, app| {
            view.update(app, |_view, cx| {
                move_repository_to_workspace_from_view(
                    cx,
                    source_window_id,
                    repo_id,
                    path.clone(),
                    None,
                );
            });
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.cx.update(|app| {
                for entry in gitcomet_window_entries(app) {
                    let _ = entry.view.update(app, |view, cx| {
                        crate::view::test_support::sync_store_snapshot(view, cx);
                    });
                }
            });
            cx.run_until_parked();
            let settled = cx.cx.update(|app| {
                let entries = gitcomet_window_entries(app);
                entries.iter().any(|entry| {
                    entry.handle.window_id() != source_window_id
                        && entry.repo_paths.as_ref() == [path.clone()]
                }) && entries.iter().any(|entry| {
                    entry.handle.window_id() == source_window_id && entry.repo_paths.is_empty()
                })
            });
            if settled {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the repository to leave the customized workspace"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let (source_open, kept) = cx.cx.update(|app| {
            (
                app.windows()
                    .iter()
                    .any(|window| window.window_id() == source_window_id),
                crate::workspaces::workspace_for_window(app, source_window_id),
            )
        });
        assert!(
            source_open,
            "a customized workspace keeps its window on Home"
        );
        let kept = kept.expect("the window still belongs to its workspace");
        assert_eq!(kept.id, source_workspace);
        assert_eq!(kept.custom_name.as_deref(), Some("Alpha"));
        assert!(kept.repositories.is_empty());
    }

    #[gpui::test]
    fn moving_the_only_repository_to_a_new_window_closes_the_source(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let source_window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view.clone());
        let snapshot = store.snapshot();
        let repo = snapshot.repos.first().expect("source repo");
        let (repo_id, path) = (repo.id, repo.spec.workdir.clone());

        cx.update(|_window, app| {
            view.update(app, |_view, cx| {
                move_repository_to_workspace_from_view(
                    cx,
                    source_window_id,
                    repo_id,
                    path.clone(),
                    None,
                );
            });
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.cx.update(|app| {
                for entry in gitcomet_window_entries(app) {
                    let _ = entry.view.update(app, |view, cx| {
                        crate::view::test_support::sync_store_snapshot(view, cx);
                    });
                }
            });
            cx.run_until_parked();
            let moved = cx.cx.update(|app| {
                app.windows().len() == 1
                    && gitcomet_window_entries(app)
                        .iter()
                        .any(|entry| entry.repo_paths.as_ref() == [path.clone()])
            });
            if moved {
                break;
            }
            if Instant::now() >= deadline {
                let diagnostics = cx.cx.update(|app| {
                    let entries = gitcomet_window_entries(app)
                        .into_iter()
                        .map(|entry| {
                            format!("{:?}: {:?}", entry.handle.window_id(), entry.repo_paths)
                        })
                        .collect::<Vec<_>>();
                    format!("{} windows; registry: {entries:?}", app.windows().len())
                });
                panic!(
                    "timed out waiting for the repository to move to its new window ({diagnostics})"
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            cx.cx.update(|app| {
                app.windows()
                    .iter()
                    .all(|window| window.window_id() != source_window_id)
            }),
            "moving the final tab should remove the now-empty source window"
        );
    }

    #[gpui::test]
    fn review_regression_confirmed_final_tab_move_reserves_durable_target_first(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });
        let source_window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.window_handle().window_id()
        });
        seed_worktree_repo(cx, &store, view);
        let snapshot = store.snapshot();
        let repo = snapshot.repos.first().expect("source repo");
        let (repo_id, path) = (repo.id, repo.spec.workdir.clone());

        cx.cx.update(|app| {
            move_repository_to_workspace(app, source_window_id, repo_id, path.clone(), None);

            assert!(
                crate::workspaces::workspaces(app)
                    .iter()
                    .any(|workspace| workspace.repositories.contains(&path)),
                "durable target ownership must exist before the source group is discarded"
            );
            assert_eq!(
                gitcomet_window_entries(app)
                    .into_iter()
                    .filter(|entry| entry_contains_repo_path(entry, &path))
                    .count(),
                1,
                "the target reservation must replace the source as the sole owner"
            );
        });
    }

    #[gpui::test]
    fn close_button_closes_only_the_clicked_window_after_opening_a_new_window(
        cx: &mut gpui::TestAppContext,
    ) {
        if cfg!(target_os = "macos") {
            // The custom Min/Max/Close controls are only rendered on non-macOS.
            return;
        }

        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let first_window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });

        cx.simulate_keystrokes("secondary-n");
        cx.run_until_parked();

        let second_window_id = cx.cx.update(|app| {
            assert_eq!(app.windows().len(), 2, "expected two GitComet windows");
            app.windows()
                .into_iter()
                .map(|window| window.window_id())
                .find(|window_id| *window_id != first_window_id)
                .expect("expected the new window to remain open")
        });

        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let close_bounds = cx
            .debug_bounds("titlebar_win_close")
            .expect("expected titlebar close control bounds");
        cx.simulate_mouse_move(close_bounds.center(), None, gpui::Modifiers::default());
        cx.simulate_mouse_down(
            close_bounds.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            close_bounds.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();

        cx.cx.update(|app| {
            let remaining_windows = app.windows();
            assert_eq!(
                remaining_windows.len(),
                1,
                "expected the close control to remove only the clicked window"
            );
            assert_eq!(
                remaining_windows[0].window_id(),
                second_window_id,
                "expected the new window to remain open after closing the original window"
            );
        });
    }

    #[gpui::test]
    fn close_shortcut_closes_the_active_window_when_no_repo_tab_can_close(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });

        assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
        cx.simulate_keystrokes("secondary-w");
        cx.run_until_parked();
        assert_eq!(cx.cx.update(|app| app.windows().len()), 0);
    }

    #[gpui::test]
    fn close_window_shortcut_closes_the_active_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });

        assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
        cx.simulate_keystrokes("secondary-shift-w");
        cx.run_until_parked();
        assert_eq!(cx.cx.update(|app| app.windows().len()), 0);
    }

    #[cfg(not(target_os = "macos"))]
    #[gpui::test]
    fn ctrl_tab_shortcuts_cycle_repository_tabs_in_the_main_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        let repos = vec![
            PathBuf::from("/tmp/gitcomet-app-test-repo-1"),
            PathBuf::from("/tmp/gitcomet-app-test-repo-2"),
            PathBuf::from("/tmp/gitcomet-app-test-repo-3"),
        ];

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });

        store.dispatch(Msg::RestoreSession {
            open_repos: repos.clone(),
            active_repo: repos.first().cloned(),
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        let repo_ids = loop {
            cx.update(|window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx)
                });
                let _ = window.draw(app);
            });
            cx.run_until_parked();

            let snapshot = store.snapshot();
            if snapshot.repos.len() == repos.len()
                && !view.read_with(&cx.cx, |view, _| view.blocks_non_repository_actions())
            {
                break snapshot
                    .repos
                    .iter()
                    .map(|repo| repo.id)
                    .collect::<Vec<_>>();
            }

            if Instant::now() >= deadline {
                panic!("timed out waiting for restored repository tabs to become interactive");
            }

            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(store.snapshot().active_repo, Some(repo_ids[0]));

        cx.simulate_keystrokes("ctrl-tab");
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        assert_eq!(store.snapshot().active_repo, Some(repo_ids[1]));

        cx.simulate_keystrokes("ctrl-shift-tab");
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        assert_eq!(store.snapshot().active_repo, Some(repo_ids[0]));

        cx.simulate_keystrokes("ctrl-shift-tab");
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        assert_eq!(store.snapshot().active_repo, Some(repo_ids[2]));
    }

    #[gpui::test]
    fn repository_picker_fallback_reuses_existing_normal_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
            window.activate_window();
        });

        assert_eq!(cx.cx.update(|app| app.windows().len()), 1);
        cx.cx.update(|app| {
            show_open_repository_manual_entry_in_existing_or_new_window(app, Arc::clone(&backend));
        });
        cx.run_until_parked();

        assert_eq!(cx.cx.update(|app| app.windows().len()), 1);
        cx.update(|_window, app| {
            assert!(crate::view::test_support::open_repo_panel_visible(
                view.read(app)
            ));
        });
    }

    #[gpui::test]
    fn repository_picker_fallback_opens_new_normal_window_when_none_exist(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);

        assert_eq!(cx.update(|app| app.windows().len()), 0);
        cx.update(|app| {
            show_open_repository_manual_entry_in_existing_or_new_window(app, Arc::clone(&backend));
        });
        cx.run_until_parked();

        let panel_visible = cx.update(|app| {
            let entry = find_normal_gitcomet_window(app)
                .expect("expected a normal GitComet window for manual repository entry");
            entry
                .view
                .update(app, |view, _cx| {
                    crate::view::test_support::open_repo_panel_visible(view)
                })
                .expect("expected to inspect the new GitComet window")
        });
        assert_eq!(cx.update(|app| app.windows().len()), 1);
        assert!(panel_visible);
    }

    #[gpui::test]
    fn command_palette_opens_new_normal_window_when_none_exist(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);

        assert_eq!(cx.update(|app| app.windows().len()), 0);
        cx.update(|app| {
            toggle_command_palette_in_active_existing_or_new_window(app, Arc::clone(&backend));
        });
        cx.run_until_parked();

        let palette_open = cx.update(|app| {
            let entry = find_normal_gitcomet_window(app)
                .expect("expected a normal GitComet window for Command Palette");
            entry
                .view
                .read_with(app, |view, _cx| {
                    crate::view::test_support::command_palette_is_open(view)
                })
                .expect("expected to inspect the new GitComet window")
        });
        assert_eq!(cx.update(|app| app.windows().len()), 1);
        assert!(palette_open);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn macos_clone_repository_action_opens_native_clone_prompt(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            install_macos_app_menu(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        cx.cx.update(|app| app.dispatch_action(&CloneRepository));
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(
            cx.debug_bounds("clone_repo_go_hint").is_some(),
            "Clone repository should open GitComet's native clone prompt"
        );
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn macos_initialize_repository_action_requests_folder_picker(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            install_macos_app_menu(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        cx.cx
            .update(|app| app.dispatch_action(&InitializeRepository));
        cx.run_until_parked();

        assert!(
            cx.did_prompt_for_paths(),
            "Initialize repository should request a destination folder"
        );
        cx.simulate_path_prompt_response(|options| {
            assert!(options.directories);
            assert!(!options.files);
            assert!(!options.multiple);
            assert_eq!(options.prompt.as_deref(), Some("Initialize Git Repository"));
            None
        });
        cx.run_until_parked();
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn macos_repository_actions_validate_background_normal_window(cx: &mut gpui::TestAppContext) {
        use gitcomet_core::process::{
            GitExecutableAvailability, GitExecutablePreference, GitRuntimeState,
        };
        use gitcomet_state::model::AppState;

        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let main_window_id = cx.update(|window, app| {
            install_macos_app_menu(app, Arc::clone(&backend));
            view.update(app, |view, cx| {
                crate::view::test_support::apply_state_snapshot_for_test(
                    view,
                    Arc::new(AppState {
                        git_runtime: GitRuntimeState {
                            preference: GitExecutablePreference::Custom(PathBuf::new()),
                            availability: GitExecutableAvailability::Unavailable {
                                detail: "Git unavailable for menu routing test".to_string(),
                            },
                        },
                        ..AppState::test_default()
                    }),
                    cx,
                );
            });
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });

        cx.cx.update(crate::view::open_settings_window);
        cx.run_until_parked();
        let settings_window = cx.cx.update(|app| {
            app.windows()
                .into_iter()
                .find(|window| window.window_id() != main_window_id)
                .expect("Settings window should open")
        });
        let settings_window_id = settings_window.window_id();
        cx.cx.update(|app| {
            let _ = settings_window.update(app, |_view, window, _cx| window.activate_window());
            assert_eq!(
                app.active_window().map(|window| window.window_id()),
                Some(settings_window_id),
                "Settings should become active"
            );
        });
        assert_ne!(settings_window_id, main_window_id);

        cx.cx.update(|app| app.dispatch_action(&CloneRepository));
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        assert!(
            cx.debug_bounds("clone_repo_go_hint").is_none(),
            "Clone repository must stay blocked by the target window's unavailable runtime"
        );

        cx.cx
            .update(|app| app.dispatch_action(&InitializeRepository));
        cx.run_until_parked();
        assert!(
            !cx.did_prompt_for_paths(),
            "Initialize repository must stay blocked by the target window's unavailable runtime"
        );
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn locate_file_action_activates_background_normal_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let main_window_id = cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle().window_id()
        });
        cx.cx.update(crate::view::open_settings_window);
        cx.run_until_parked();
        let settings_window = cx.cx.update(|app| {
            app.windows()
                .into_iter()
                .find(|window| window.window_id() != main_window_id)
                .expect("Settings window should open")
        });
        let settings_window_id = settings_window.window_id();
        cx.cx.update(|app| {
            let _ = settings_window.update(app, |_view, window, _cx| window.activate_window());
            assert_eq!(
                app.active_window().map(|window| window.window_id()),
                Some(settings_window_id),
                "Settings should become active"
            );
        });
        assert_ne!(settings_window_id, main_window_id);

        cx.cx
            .update(|app| app.dispatch_action(&LocateFileInExplorer));
        cx.run_until_parked();

        cx.cx.update(|app| {
            assert_eq!(
                app.active_window().map(|window| window.window_id()),
                Some(main_window_id),
                "locating a file should bring the main GitComet window forward"
            );
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn focus_existing_repository_window_for_path_avoids_reading_the_active_window_on_stack(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let window_handle = cx.update(|window, app| {
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle()
        });

        let repo_path = PathBuf::from("/tmp/gitcomet-not-open");
        cx.cx.update(|app| {
            let result = window_handle.update(app, |_root_view, _window, app| {
                focus_existing_repository_window_for_path(app, repo_path.as_path())
            });
            assert_eq!(result.ok(), Some(false));
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn update_active_normal_gitcomet_window_avoids_reading_the_active_window_on_stack(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new_test(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let window_handle = cx.update(|window, app| {
            let _ = window.draw(app);
            window.activate_window();
            window.window_handle()
        });

        cx.cx.update(|app| {
            let result = window_handle.update(app, |_root_view, _window, app| {
                update_active_normal_gitcomet_window(app, |view, cx| view.close_active_repo_tab(cx))
            });
            assert_eq!(result.ok().flatten(), Some(false));
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn file_url_to_path_decodes_spaces() {
        let path = file_url_to_path("file:///Users/sampo/Repo%20Name").expect("valid file url");
        assert_eq!(path, PathBuf::from("/Users/sampo/Repo Name"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn file_url_to_path_accepts_localhost_urls() {
        let path =
            file_url_to_path("file://localhost/Users/sampo/repo").expect("localhost file url");
        assert_eq!(path, PathBuf::from("/Users/sampo/repo"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn repository_paths_from_open_urls_filters_non_file_urls_and_dedups() {
        let urls = vec![
            "file:///tmp/repo".to_string(),
            "https://example.com/repo".to_string(),
            "file:///tmp/repo".to_string(),
        ];

        let paths = repository_paths_from_open_urls(&urls);

        assert_eq!(
            paths,
            vec![normalize_repository_open_path(PathBuf::from("/tmp/repo"))]
        );
    }
}
