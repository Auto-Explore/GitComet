//! Launching: configuration, the application run loop, and opening main windows.

use super::*;

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
pub(super) struct WindowLaunchConfig {
    pub(super) title: String,
    pub(super) app_id: String,
    pub(super) view_config: GitCometViewConfig,
    /// How an initial browser request should be routed after saved workspaces have
    /// been restored. Constructors keep the historical existing-window
    /// behavior unless the executable explicitly carries a different choice.
    pub(super) browser_open_target: BrowserOpenTarget,
}

#[derive(Clone)]
pub(super) struct CleanShutdownTracker {
    pub(super) requested: Arc<AtomicBool>,
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
pub(super) struct GitCometBackendGlobal(pub(super) Arc<dyn GitBackend>);

impl gpui::Global for GitCometBackendGlobal {}

pub(super) type ShutdownCallback = Arc<dyn Fn() + Send + Sync>;

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

    pub fn detail(self) -> String {
        match self {
            Self::ExistingWindow => format!(
                "Add the repository to the active {} window.",
                identity::current().display_name()
            ),
            Self::NewWindow => "Create a separate window for the repository.".to_string(),
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

pub(super) fn main_window_default_size_for_percent(percent: u32) -> Size<Pixels> {
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

/// What a real launch tells the UI kit before its first window: run live, and
/// read custom themes from the user's theme folder. Tests skip this and stay
/// deterministic without touching the user's files.
pub(crate) fn install_live_kit_policy() {
    crate::ui_runtime::install(crate::ui_runtime::UiRuntime::live());
    crate::theme::set_user_themes_dir(session::user_themes_dir());
}

/// A browser-window launch: configure it, then [`run`](Self::run) it.
pub struct UiLaunch {
    pub(super) backend: Arc<dyn GitBackend>,
    pub(super) extensions: gitcomet_extension_api::Registry,
    pub(super) initial_request: BrowserOpenRequest,
    pub(super) startup_crash_report: Option<StartupCrashReport>,
    pub(super) on_shutdown: Option<ShutdownCallback>,
    pub(super) browser_requests: Option<crate::BrowserRequestReceiver>,
}

impl UiLaunch {
    pub fn new(backend: Arc<dyn GitBackend>) -> Self {
        Self {
            backend,
            extensions: gitcomet_extension_api::Registry::default(),
            initial_request: BrowserOpenRequest {
                path: None,
                target: BrowserOpenTarget::ExistingWindow,
            },
            startup_crash_report: None,
            on_shutdown: None,
            browser_requests: None,
        }
    }

    /// The compiled-in extensions, validated and frozen. Installed before the
    /// first window opens; an empty registry adds no work anywhere.
    pub fn extensions(mut self, registry: gitcomet_extension_api::Registry) -> Self {
        self.extensions = registry;
        self
    }

    /// The process's own open request. Its routing preference matters on a
    /// cold start: saved workspaces are restored before the path is routed,
    /// just like a forwarded request.
    pub fn initial_request(mut self, request: BrowserOpenRequest) -> Self {
        self.initial_request = request;
        self
    }

    pub fn startup_crash_report(mut self, report: Option<StartupCrashReport>) -> Self {
        self.startup_crash_report = report;
        self
    }

    /// Invoked from GPUI's graceful-shutdown callback. On Windows GPUI
    /// terminates with `ExitProcess`, so cleanup cannot wait for [`run`](Self::run)
    /// to return.
    pub fn on_shutdown(mut self, callback: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_shutdown = Some(Arc::new(callback));
        self
    }

    /// Repository-open requests forwarded by another process. The caller owns
    /// the single-instance transport, which keeps the wire protocol outside
    /// this crate and independently testable.
    pub fn browser_requests(mut self, requests: Option<crate::BrowserRequestReceiver>) -> Self {
        self.browser_requests = requests;
        self
    }

    pub fn run(self) -> Result<UiRunOutcome, UiLaunchError> {
        install_live_kit_policy();
        let Self {
            backend,
            extensions,
            initial_request,
            startup_crash_report,
            on_shutdown,
            browser_requests,
        } = self;
        let mut launch = normal_launch_config(initial_request.path, startup_crash_report);
        launch.browser_open_target = initial_request.target;
        ensure_graphics_device_available("main GPUI window launch")?;
        run_with_panic_guard("main GPUI window launch", move || {
            run_windowed_app(
                backend,
                extensions,
                launch,
                CleanShutdownTracker::default(),
                on_shutdown,
                browser_requests,
            )
        })?;
        // A native abort or forced process termination cannot return from the
        // GPUI event loop. Reaching this point is therefore a clean shutdown even
        // when a platform-specific close path did not set CleanShutdownTracker.
        Ok(UiRunOutcome::CleanShutdown)
    }
}

#[deprecated(note = "use `UiLaunch`")]
pub fn run(backend: Arc<dyn GitBackend>) -> Result<(), UiLaunchError> {
    UiLaunch::new(backend).run().map(|_| ())
}

#[deprecated(note = "use `UiLaunch`")]
pub fn run_with_startup_crash_report(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
) -> Result<UiRunOutcome, UiLaunchError> {
    UiLaunch::new(backend)
        .initial_request(BrowserOpenRequest {
            path: initial_path,
            target: BrowserOpenTarget::ExistingWindow,
        })
        .startup_crash_report(startup_crash_report)
        .run()
}

#[deprecated(note = "use `UiLaunch`")]
pub fn run_with_startup_crash_report_and_shutdown_callback(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
    on_shutdown: Option<impl Fn() + Send + Sync + 'static>,
) -> Result<UiRunOutcome, UiLaunchError> {
    #[allow(deprecated)]
    run_with_startup_crash_report_shutdown_callback_and_browser_requests(
        backend,
        initial_path,
        startup_crash_report,
        on_shutdown,
        None,
    )
}

#[deprecated(note = "use `UiLaunch`")]
pub fn run_with_startup_crash_report_shutdown_callback_and_browser_requests(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
    on_shutdown: Option<impl Fn() + Send + Sync + 'static>,
    browser_requests: Option<crate::BrowserRequestReceiver>,
) -> Result<UiRunOutcome, UiLaunchError> {
    #[allow(deprecated)]
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

#[deprecated(note = "use `UiLaunch`")]
pub fn run_with_startup_crash_report_shutdown_callback_and_initial_browser_request(
    backend: Arc<dyn GitBackend>,
    initial_request: BrowserOpenRequest,
    startup_crash_report: Option<StartupCrashReport>,
    on_shutdown: Option<impl Fn() + Send + Sync + 'static>,
    browser_requests: Option<crate::BrowserRequestReceiver>,
) -> Result<UiRunOutcome, UiLaunchError> {
    let mut launch = UiLaunch::new(backend)
        .initial_request(initial_request)
        .startup_crash_report(startup_crash_report)
        .browser_requests(browser_requests);
    launch.on_shutdown = on_shutdown.map(|callback| Arc::new(callback) as ShutdownCallback);
    launch.run()
}

/// Configuration for a standalone diff using the same pane as repository views.
#[derive(Clone, Debug)]
pub struct FocusedDiffConfig {
    pub label_left: String,
    pub label_right: String,
    pub display_path: Option<String>,
    pub diff_text: String,
}

impl UiLaunch {
    pub fn run_focused_diff(self, config: FocusedDiffConfig) -> i32 {
        install_live_kit_policy();
        if let Err(err) = ensure_graphics_device_available("focused diff GPUI launch") {
            eprintln!("Failed to launch focused diff window: {err}");
            return 2;
        }
        let launch = WindowLaunchConfig {
            title: format!("{} — Diff", identity::current().display_name()),
            app_id: identity::current().window_app_id(WindowKind::FocusedDiff),
            view_config: GitCometViewConfig {
                view_mode: GitCometViewMode::FocusedDiff,
                focused_diff: Some(config),
                workspace: WorkspaceBootstrap::Empty,
                ..Default::default()
            },
            browser_open_target: BrowserOpenTarget::ExistingWindow,
        };
        match run_with_panic_guard("focused diff GPUI launch", move || {
            run_windowed_app(
                self.backend,
                self.extensions,
                launch,
                CleanShutdownTracker::default(),
                self.on_shutdown,
                None,
            )
        }) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("Failed to launch focused diff window: {err}");
                2
            }
        }
    }

    pub fn run_focused_mergetool(self, config: FocusedMergetoolConfig) -> i32 {
        run_mergetool_with_extensions(self.backend, config, self.extensions)
    }
}

/// Compatibility entry point for a snapshot-only standalone diff.
pub fn run_focused_diff(config: FocusedDiffConfig) -> i32 {
    struct SnapshotBackend;
    impl GitBackend for SnapshotBackend {
        fn open(
            &self,
            _: &Path,
        ) -> gitcomet_core::services::Result<Arc<dyn gitcomet_core::services::GitRepository>>
        {
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Unsupported("snapshot window has no repository"),
            ))
        }
    }
    UiLaunch::new(Arc::new(SnapshotBackend)).run_focused_diff(config)
}

/// Launch the unified focused mergetool window using the shared `GitCometView`.
pub fn run_focused_mergetool(backend: Arc<dyn GitBackend>, config: FocusedMergetoolConfig) -> i32 {
    run_mergetool_with_extensions(backend, config, gitcomet_extension_api::Registry::default())
}

fn run_mergetool_with_extensions(
    backend: Arc<dyn GitBackend>,
    config: FocusedMergetoolConfig,
    extensions: gitcomet_extension_api::Registry,
) -> i32 {
    install_live_kit_policy();
    if let Err(err) = ensure_graphics_device_available("focused mergetool GPUI launch") {
        eprintln!("Failed to launch focused mergetool window: {err}");
        return FOCUSED_MERGETOOL_EXIT_ERROR;
    }

    let exit_code = Arc::new(AtomicI32::new(FOCUSED_MERGETOOL_EXIT_CANCELED));
    let launch = focused_mergetool_launch_config(&config, Some(exit_code.clone()));
    if let Err(err) = run_with_panic_guard("focused mergetool GPUI launch", move || {
        run_windowed_app(
            backend,
            extensions,
            launch,
            CleanShutdownTracker::default(),
            None,
            None,
        )
    }) {
        eprintln!("Failed to launch focused mergetool window: {err}");
        return FOCUSED_MERGETOOL_EXIT_ERROR;
    }
    exit_code.load(Ordering::SeqCst)
}

pub(super) fn normal_launch_config(
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    let mut view_config = GitCometViewConfig::normal(startup_crash_report);
    view_config.initial_path = initial_path;
    WindowLaunchConfig {
        title: identity::current().display_name().to_string(),
        app_id: identity::current().window_app_id(WindowKind::Main),
        view_config,
        browser_open_target: BrowserOpenTarget::ExistingWindow,
    }
}

pub(super) fn normal_launch_config_with_initial_repository(
    initial_path: PathBuf,
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    WindowLaunchConfig {
        title: identity::current().display_name().to_string(),
        app_id: identity::current().window_app_id(WindowKind::Main),
        view_config: GitCometViewConfig::normal_with_initial_repository(
            initial_path,
            startup_crash_report,
        ),
        browser_open_target: BrowserOpenTarget::ExistingWindow,
    }
}

pub(super) fn normal_empty_launch_config(
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    let mut launch = normal_launch_config(None, startup_crash_report);
    launch.view_config.workspace = WorkspaceBootstrap::Empty;
    launch
}

pub(super) fn launch_config_for_workspace(
    base: &WindowLaunchConfig,
    workspace: session::Workspace,
    startup_crash_report: Option<StartupCrashReport>,
) -> WindowLaunchConfig {
    let mut launch = base.clone();
    launch.title = format!(
        "{} — {}",
        workspace.display_name(),
        identity::current().display_name()
    );
    launch.view_config.initial_path = None;
    launch.view_config.initial_repository_launch_mode = InitialRepositoryLaunchMode::RestoreSession;
    launch.view_config.startup_crash_report = startup_crash_report;
    launch.view_config.workspace = WorkspaceBootstrap::Saved(Box::new(workspace));
    launch
}

pub(super) fn focused_mergetool_launch_config(
    config: &FocusedMergetoolConfig,
    exit_code: Option<Arc<AtomicI32>>,
) -> WindowLaunchConfig {
    WindowLaunchConfig {
        title: focused_mergetool_window_title(&config.conflicted_file_path),
        app_id: identity::current().window_app_id(WindowKind::FocusedMergetool),
        view_config: GitCometViewConfig {
            initial_path: Some(config.repo_path.clone()),
            initial_repository_launch_mode: InitialRepositoryLaunchMode::RestoreSession,
            view_mode: GitCometViewMode::FocusedMergetool,
            focused_diff: None,
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

pub(super) fn focused_mergetool_window_title(conflicted_file_path: &Path) -> String {
    let display = conflicted_file_path
        .file_name()
        .and_then(|name| name.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{conflicted_file_path:?}"));
    format!(
        "{} - Mergetool ({display})",
        identity::current().display_name()
    )
}

/// Keep undo/staging areas under the app state dir instead of repository
/// worktrees, and clear what crashed instances left there off the UI thread.
fn configure_filesystem_journal_storage() {
    let Some(root) = session::journal_storage_dir() else {
        return;
    };
    match gitcomet_core::filesystem::configure_journal_storage(&root) {
        Ok(_) => {
            smol::unblock(move || gitcomet_core::filesystem::sweep_leaked_journal_storage(&root))
                .detach();
        }
        Err(err) => eprintln!("Failed to configure filesystem journal storage: {err}"),
    }
}

pub(super) fn run_windowed_app(
    backend: Arc<dyn GitBackend>,
    extensions: gitcomet_extension_api::Registry,
    launch: WindowLaunchConfig,
    clean_shutdown_tracker: CleanShutdownTracker,
    on_shutdown: Option<ShutdownCallback>,
    browser_requests: Option<crate::BrowserRequestReceiver>,
) {
    let quit_when_all_windows_closed = should_quit_when_all_windows_closed(&launch);
    // Without this, `gpui` keeps its null client and every request — the
    // update check, and images a markdown preview points at — fails silently.
    let application = application()
        .with_assets(GitCometAssets::with_extensions(extensions.assets()))
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
        configure_filesystem_journal_storage();
        cx.on_app_quit(|_| {
            // GPUI only waits 200 ms for returned futures. Journal directories
            // can be large, so finish cleanup before that timeout starts.
            gitcomet_core::filesystem::cleanup_on_shutdown();
            async { smol::unblock(session::flush_recent_documents).await }
        })
        .detach();
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

        // Every window kind runs extension gates, guards, and window hooks;
        // only main windows offer extension commands.
        crate::view::extension_host::install_registry(extensions, cx);
        if launch.view_config.view_mode == GitCometViewMode::Normal {
            // Before the host's keys, so a chord both claim stays the host's.
            crate::view::extension_host::install_bindings(cx);
            bind_app_keys(cx);
            install_app_actions(cx, Arc::clone(&backend));
            if let Some(browser_requests) = browser_requests {
                register_browser_open_request_handler(cx, Arc::clone(&backend), browser_requests.0);
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

pub(super) fn open_initial_gitcomet_windows(
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
pub(super) fn open_initial_gitcomet_windows_after_workspace_initialization(
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

pub(super) fn should_quit_when_all_windows_closed(launch: &WindowLaunchConfig) -> bool {
    launch.view_config.view_mode != GitCometViewMode::Normal || !cfg!(target_os = "macos")
}

pub(super) fn open_gitcomet_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    launch: &WindowLaunchConfig,
) -> gpui::WindowHandle<GitCometView> {
    clear_clean_shutdown_request(cx);
    let ui_session = session::load();
    let ui_scale = crate::session_ui::ui_scale(&ui_session, cx);
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

    let window = crate::ui_probe::time_section("open main window", || {
        cx.open_window(
            with_main_window_background(WindowOptions {
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
                icon: crate::assets::window_icon(),
                is_movable: true,
                is_resizable: true,
                ..Default::default()
            }),
            move |window, cx| {
                ui_scale::apply_to_window(window, ui_scale_percent);
                window.on_window_should_close(cx, |window, cx| {
                    close_window_or_warn(window, cx);
                    false
                });
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
        let name = identity::current().display_name();
        panic!(
            "failed to open main {name} window: {err}\n\
             This is usually a GPU/display problem, not a {name} bug. \
             If you just updated your system (kernel, mesa, or vulkan drivers), reboot. \
             For diagnostics, include the crash report from the next launch."
        )
    });

    #[cfg(target_os = "macos")]
    refresh_macos_app_menus(cx);
    if let Ok(view) = window.update(cx, |_, _, cx| cx.entity()) {
        crate::view::extension_host::window_opened(&view, cx);
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
pub(crate) fn with_main_window_background(mut options: WindowOptions) -> WindowOptions {
    let transparent = match std::env::var("GITCOMET_WINDOW_BACKGROUND") {
        Ok(value) if value.trim().eq_ignore_ascii_case("opaque") => false,
        Ok(value) if value.trim().eq_ignore_ascii_case("transparent") => true,
        _ => !cfg!(target_os = "macos"),
    };
    #[cfg(target_os = "macos")]
    {
        options.macos_window_background = if transparent {
            gpui::MacosWindowBackground::Transparent
        } else {
            gpui::MacosWindowBackground::Opaque
        };
    }
    #[cfg(target_os = "windows")]
    {
        options.windows_window_background = if transparent {
            gpui::WindowsWindowBackground::Transparent
        } else {
            gpui::WindowsWindowBackground::Opaque
        };
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        options.linux_window_background = if transparent {
            gpui::LinuxWindowBackground::Transparent
        } else {
            gpui::LinuxWindowBackground::Opaque
        };
    }
    options
}

fn apply_ui_scale_to_window(cx: &mut App, handle: AnyWindowHandle, percent: u32) {
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

        // A pop-out: its views read the scale while rendering, so redraw.
        if window.rem_size() != ui_scale::rem_size_for_percent(percent) {
            ui_scale::apply_to_window(window, percent);
            window.refresh();
        }
    });
}

/// Moves every window to its scale: its own zoom, its main window's (for a
/// pop-out), or the default.
pub(crate) fn apply_ui_scale_to_windows(cx: &mut App) {
    for handle in cx.windows() {
        let percent = ui_scale::percent_for_window(cx, handle.window_id());
        apply_ui_scale_to_window(cx, handle, percent);
    }
}

/// Zooms one window; `None` returns it to the default UI scale. Call it
/// deferred: a window cannot be updated while it is mid-update.
pub(crate) fn set_window_ui_scale_percent(cx: &mut App, window_id: WindowId, percent: Option<u32>) {
    ui_scale::set_window_percent(cx, window_id, percent);
    apply_ui_scale_to_windows(cx);
    // Its footer shows whether it has its own zoom, which can change while
    // the percent does not.
    for handle in cx.windows() {
        let _ = handle.update(cx, |root_view, _, cx| {
            if let Ok(view) = root_view.downcast::<GitCometView>() {
                view.update(cx, |view, cx| view.refresh_zoom_indicator(cx));
            }
        });
    }
}

/// The window a zoom shortcut acts on: the one dispatching it, else the
/// focused one (the macOS menu bar dispatches outside any window).
pub(crate) fn zoom_target_window(cx: &App) -> Option<WindowId> {
    cx.current_window_id()
        .or_else(|| cx.active_window().map(|window| window.window_id()))
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
