use crate::assets::GitCometAssets;
use crate::launch_guard::{UiLaunchError, run_with_panic_guard};
use crate::view::{
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, InitialRepositoryLaunchMode, StartupCrashReport,
};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitBackend;
use gitcomet_state::session;
use gitcomet_state::store::AppStore;
#[cfg(target_os = "windows")]
use gpui::WindowsPlatform;
#[cfg(target_os = "macos")]
use gpui::{Action, Menu, MenuItem, OsAction, SystemMenuType};
use gpui::{
    App, AppContext, BorrowAppContext, Bounds, KeyBinding, Pixels, Point, TitlebarOptions, Window,
    WindowBounds, WindowDecorations, WindowOptions, actions, point, px, size,
};
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;
use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(target_os = "macos")]
use schemars::JsonSchema;
#[cfg(target_os = "macos")]
use serde::Deserialize;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

const WINDOW_MIN_WIDTH_PX: f32 = 820.0;
const WINDOW_MIN_HEIGHT_PX: f32 = 560.0;
const WINDOW_DEFAULT_WIDTH_PX: f32 = 1100.0;
const WINDOW_DEFAULT_HEIGHT_PX: f32 = 720.0;
const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
#[cfg(test)]
const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

actions!(
    app_menu,
    [
        NewWindow,
        OpenSettings,
        OpenRepository,
        OpenRecentPicker,
        ApplyPatch,
        Close,
        CloseWindow,
        PreviousRepository,
        NextRepository,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
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

#[derive(Clone, Debug)]
struct WindowLaunchConfig {
    title: String,
    app_id: String,
    view_config: GitCometViewConfig,
}

pub fn run(backend: Arc<dyn GitBackend>) -> Result<(), UiLaunchError> {
    run_with_startup_crash_report(backend, None, None)
}

pub fn run_with_startup_crash_report(
    backend: Arc<dyn GitBackend>,
    initial_path: Option<PathBuf>,
    startup_crash_report: Option<StartupCrashReport>,
) -> Result<(), UiLaunchError> {
    let launch = normal_launch_config(initial_path, startup_crash_report);
    ensure_graphics_device_available("main GPUI window launch")?;
    run_with_panic_guard("main GPUI window launch", move || {
        run_windowed_app(backend, launch)
    })
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
        run_windowed_app(backend, launch)
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
    }
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
        },
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

#[cfg(target_os = "windows")]
pub(crate) fn application() -> gpui::Application {
    gpui::Application::with_platform(Rc::new(
        WindowsPlatform::new(false).expect("failed to initialize Windows platform"),
    ))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn application() -> gpui::Application {
    gpui::application()
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
fn restore_maximized_window(window: &Window) -> bool {
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };

    // GPUI's Windows zoom path currently maps directly to SW_MAXIMIZE, so
    // restore must go through the native Win32 API until upstream toggles.
    gitcomet_win32_window_utils::restore_window(handle.hwnd.get())
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
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return None;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };

    let (x, y) = window_menu_position(position, window.scale_factor());
    Some(WindowSystemMenuRequest {
        hwnd: handle.hwnd.get(),
        x,
        y,
    })
}

fn run_windowed_app(backend: Arc<dyn GitBackend>, launch: WindowLaunchConfig) {
    let quit_when_all_windows_closed = should_quit_when_all_windows_closed(&launch);
    let application = application().with_assets(GitCometAssets);

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
        let reopen_launch = launch.clone();
        application.on_reopen(move |cx: &mut App| {
            if cx.windows().is_empty() {
                open_gitcomet_window(cx, Arc::clone(&reopen_backend), &reopen_launch);
                cx.activate(true);
            }
        });
    }

    application.run(move |cx: &mut App| {
        if let Err(err) = crate::bundled_fonts::register(cx) {
            eprintln!("Failed to register bundled fonts: {err:#}");
        }
        bind_text_input_keys(cx);
        if quit_when_all_windows_closed {
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        }

        if launch.view_config.view_mode == GitCometViewMode::Normal {
            bind_app_keys(cx);
            install_app_actions(cx, Arc::clone(&backend));

            #[cfg(target_os = "macos")]
            {
                install_macos_app_menu(cx, Arc::clone(&backend));
                if let Some(open_urls_rx) = open_urls_rx {
                    register_macos_open_request_handler(cx, Arc::clone(&backend), open_urls_rx);
                }
            }
        }

        open_gitcomet_window(cx, Arc::clone(&backend), &launch);

        cx.activate(true);
    });
}

fn should_quit_when_all_windows_closed(launch: &WindowLaunchConfig) -> bool {
    launch.view_config.view_mode == GitCometViewMode::FocusedMergetool || !cfg!(target_os = "macos")
}

fn open_gitcomet_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    launch: &WindowLaunchConfig,
) -> gpui::WindowHandle<GitCometView> {
    let ui_session = session::load();
    let restored_w = ui_session
        .window_width
        .map(|w| px(w as f32))
        .unwrap_or(px(WINDOW_DEFAULT_WIDTH_PX))
        .max(px(WINDOW_MIN_WIDTH_PX));
    let restored_h = ui_session
        .window_height
        .map(|h| px(h as f32))
        .unwrap_or(px(WINDOW_DEFAULT_HEIGHT_PX))
        .max(px(WINDOW_MIN_HEIGHT_PX));
    let bounds = Bounds::centered(None, size(restored_w, restored_h), cx);
    let window_title = launch.title.clone();
    let app_id = launch.app_id.clone();
    let view_config = launch.view_config.clone();

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(WINDOW_MIN_WIDTH_PX), px(WINDOW_MIN_HEIGHT_PX))),
            titlebar: Some(TitlebarOptions {
                title: Some(window_title.into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(9.0), px(9.0))),
            }),
            app_id: Some(app_id),
            window_decorations: Some(WindowDecorations::Client),
            is_movable: true,
            is_resizable: true,
            ..Default::default()
        },
        move |window, cx| {
            let (store, events) = AppStore::new(Arc::clone(&backend));
            cx.new(|cx| {
                GitCometView::new_with_config(store, events, view_config.clone(), window, cx)
            })
        },
    )
    .expect("failed to open main GitComet window")
}

fn install_app_actions(cx: &mut App, backend: Arc<dyn GitBackend>) {
    let new_window_backend = Arc::clone(&backend);
    cx.on_action(move |_: &NewWindow, cx| {
        let backend = Arc::clone(&new_window_backend);
        cx.defer(move |cx| {
            let launch = normal_launch_config(None, None);
            open_gitcomet_window(cx, backend, &launch);
            cx.activate(true);
        });
    });

    cx.on_action(|_: &OpenSettings, cx| {
        cx.defer(|cx| {
            crate::view::open_settings_window(cx);
        });
    });

    let repo_backend = Arc::clone(&backend);
    cx.on_action(move |_: &OpenRepository, cx| {
        let backend = Arc::clone(&repo_backend);
        cx.defer(move |cx| {
            prompt_open_repository(cx, backend);
        });
    });

    let recent_picker_backend = Arc::clone(&backend);
    cx.on_action(move |_: &OpenRecentPicker, cx| {
        let backend = Arc::clone(&recent_picker_backend);
        cx.defer(move |cx| {
            if active_normal_gitcomet_window_blocks_non_repository_actions(cx) {
                return;
            }
            open_recent_repository_picker_in_existing_or_new_window(cx, backend);
        });
    });

    cx.on_action(|_: &Close, cx| {
        cx.defer(|cx| {
            let handled =
                update_active_normal_gitcomet_window(cx, |view, cx| view.close_active_repo_tab(cx))
                    .unwrap_or(false);
            if !handled {
                close_active_window(cx);
            }
        });
    });
    cx.on_action(|_: &CloseWindow, cx| {
        cx.defer(close_active_window);
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
    cx.on_action(|_: &Hide, cx| cx.defer(|cx| cx.hide()));
    cx.on_action(|_: &HideOthers, cx| cx.defer(|cx| cx.hide_other_apps()));
    cx.on_action(|_: &ShowAll, cx| cx.defer(|cx| cx.unhide_other_apps()));
    cx.on_action(|_: &Quit, cx| cx.defer(|cx| cx.quit()));
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

    refresh_macos_app_menus(cx);
}

fn bind_app_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewWindow, None),
        KeyBinding::new("secondary-shift-n", NewWindow, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-o", OpenRepository, None),
        KeyBinding::new("secondary-shift-o", OpenRecentPicker, None),
        KeyBinding::new("secondary-w", Close, None),
        KeyBinding::new("secondary-shift-w", CloseWindow, None),
        KeyBinding::new("secondary-pageup", PreviousRepository, None),
        KeyBinding::new("secondary-pagedown", NextRepository, None),
        KeyBinding::new("secondary-q", Quit, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-o", OpenRecentPicker, None),
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
}

#[cfg(target_os = "macos")]
fn macos_app_menus() -> Vec<Menu> {
    let mut file_items = vec![
        MenuItem::action("New Window", NewWindow),
        MenuItem::separator(),
        MenuItem::action("Open…", OpenRepository),
        MenuItem::action("Open Recent…", OpenRecentPicker),
    ];

    let recent_repo_items = recent_repo_menu_items();
    if !recent_repo_items.is_empty() {
        file_items.push(MenuItem::submenu(Menu {
            name: "Recent Repositories".into(),
            items: recent_repo_items,
            disabled: false,
        }));
    }

    file_items.extend([
        MenuItem::action("Apply Patch…", ApplyPatch),
        MenuItem::separator(),
        MenuItem::action("Close", Close),
        MenuItem::action("Close Window", CloseWindow),
    ]);

    vec![
        Menu {
            name: "GitComet".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
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
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", MinimizeWindow),
                MenuItem::action("Zoom", ZoomWindow),
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
    cx.set_menus(macos_app_menus());
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
            let _ = cx.update(move |cx| {
                open_repositories_in_existing_or_new_window(cx, backend, paths);
            });
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

pub(crate) fn recent_repository_label(path: &Path) -> String {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.display().to_string();
    };
    let Some(parent) = path.parent() else {
        return name.to_string();
    };
    format!("{name} - {}", parent.display())
}

#[derive(Clone)]
struct GitCometWindowEntry {
    handle: gpui::AnyWindowHandle,
    view: gpui::WeakEntity<GitCometView>,
    view_mode: GitCometViewMode,
    repo_paths: Vec<PathBuf>,
}

#[derive(Default)]
struct GitCometWindowRegistry {
    windows: FxHashMap<gpui::WindowId, GitCometWindowEntry>,
}

impl gpui::Global for GitCometWindowRegistry {}

pub(crate) fn sync_gitcomet_window_state<C>(
    cx: &mut C,
    handle: gpui::AnyWindowHandle,
    view: gpui::WeakEntity<GitCometView>,
    view_mode: GitCometViewMode,
    repo_paths: Vec<PathBuf>,
) where
    C: BorrowAppContext,
{
    cx.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
        registry.windows.insert(
            handle.window_id(),
            GitCometWindowEntry {
                handle,
                view,
                view_mode,
                repo_paths,
            },
        );
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
        registry.windows.values().cloned().collect()
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

fn active_normal_gitcomet_window(cx: &mut App) -> Option<GitCometWindowEntry> {
    let entry = active_gitcomet_window_entry(cx)?;
    (entry.view_mode == GitCometViewMode::Normal).then_some(entry)
}

fn update_active_normal_gitcomet_window<R>(
    cx: &mut App,
    f: impl FnOnce(&mut GitCometView, &mut gpui::Context<GitCometView>) -> R,
) -> Option<R> {
    let window = active_normal_gitcomet_window(cx)?;
    window.view.update(cx, f).ok()
}

fn active_normal_gitcomet_window_blocks_non_repository_actions(cx: &mut App) -> bool {
    update_active_normal_gitcomet_window(cx, |view, _cx| view.blocks_non_repository_actions())
        .unwrap_or(false)
}

fn close_active_window(cx: &mut App) {
    if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_root, window, _cx| {
            window.remove_window();
        });
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
    entries
        .into_iter()
        .find(|entry| entry.view_mode == GitCometViewMode::Normal)
}

fn find_normal_gitcomet_window_for_repo(cx: &mut App, path: &Path) -> Option<GitCometWindowEntry> {
    let entries = gitcomet_window_entries(cx);
    let active_window_id = cx.active_window().map(|window| window.window_id());
    if let Some(active_window_id) = active_window_id
        && let Some(entry) = entries
            .iter()
            .find(|entry| {
                entry.handle.window_id() == active_window_id
                    && entry.view_mode == GitCometViewMode::Normal
                    && entry_contains_repo_path(entry, path)
            })
            .cloned()
    {
        return Some(entry);
    }
    entries.into_iter().find(|entry| {
        entry.view_mode == GitCometViewMode::Normal && entry_contains_repo_path(entry, path)
    })
}

fn activate_gitcomet_window(cx: &mut App, window: gpui::AnyWindowHandle) {
    let _ = window.update(cx, |_view, window, _cx| {
        window.activate_window();
    });
}

fn open_repository_in_window(cx: &mut App, window: &GitCometWindowEntry, path: PathBuf) {
    let path_for_window = path.clone();
    let _ = window.view.update(cx, |view, cx| {
        view.open_repo_path(path_for_window, cx);
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

fn focus_existing_repository_window(cx: &mut App, window: &GitCometWindowEntry, path: &Path) {
    let path_for_window = path.to_path_buf();
    let _ = window.view.update(cx, |view, cx| {
        view.activate_repo_path(path_for_window.as_path(), cx);
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

fn open_recent_repository_picker_in_window(cx: &mut App, window: &GitCometWindowEntry) {
    let _ = window.handle.update(cx, |root_view, window, cx| {
        let Ok(view) = root_view.downcast::<GitCometView>() else {
            return;
        };
        view.update(cx, |view, cx| {
            view.open_recent_repository_picker(window, cx);
        });
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

fn open_recent_repository_picker_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
    if let Some(window) = find_normal_gitcomet_window(cx) {
        open_recent_repository_picker_in_window(cx, &window);
        return;
    }

    let launch = normal_launch_config(None, None);
    let window = open_gitcomet_window(cx, backend, &launch);
    let _ = window.update(cx, |view, window, cx| {
        view.open_recent_repository_picker(window, cx);
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

    let launch = normal_launch_config(None, None);
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
        if let Some(window) = find_normal_gitcomet_window_for_repo(cx, path.as_path()) {
            focus_existing_repository_window(cx, &window, path.as_path());
            target_window = Some(window);
            continue;
        }

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

fn prompt_open_repository(cx: &mut App, backend: Arc<dyn GitBackend>) {
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
                    show_open_repository_manual_entry_in_existing_or_new_window(
                        cx,
                        Arc::clone(&backend),
                    );
                });
                return;
            }
        };
        let Some(path) = paths.into_iter().next() else {
            return;
        };

        cx.update(move |cx| {
            open_repository_in_existing_or_new_window(cx, Arc::clone(&backend), path);
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

        let _ = cx.update(move |cx| {
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
        KeyBinding::new("enter", crate::kit::Enter, Some("TextInput")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Action, Context, FocusHandle, InteractiveElement, IntoElement, Render, Styled, Window, div,
    };

    use crate::test_support::lock_visual_test;
    use gitcomet_core::error::{Error, ErrorKind};
    use gitcomet_core::services::{GitRepository, Result};
    use gitcomet_state::msg::Msg;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct TestBackend;

    impl GitBackend for TestBackend {
        fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::Unsupported(
                "test backend does not open repositories",
            )))
        }
    }

    fn seed_workspace_repo(
        cx: &mut gpui::VisualTestContext,
        store: &AppStore,
        view: gpui::Entity<GitCometView>,
    ) {
        cx.update(|_window, app| {
            view.update(app, |this, _cx| this.disable_poller_for_tests());
        });

        store.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/gitcomet-app-test-repo")));

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|window, app| {
                view.update(app, |this, cx| this.sync_store_snapshot_for_tests(cx));
                let _ = window.draw(app);
            });
            cx.run_until_parked();

            let ready = cx.update(|_window, app| !view.read(app).blocks_non_repository_actions());
            if ready {
                return;
            }

            if Instant::now() >= deadline {
                panic!("timed out waiting for the workspace view to leave the splash state");
            }

            std::thread::sleep(Duration::from_millis(10));
        }
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
                .on_action(record_action_listener!(crate::kit::Enter))
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
                .on_action(record_action_listener!(NewWindow))
                .on_action(record_action_listener!(OpenSettings))
                .on_action(record_action_listener!(OpenRepository))
                .on_action(record_action_listener!(OpenRecentPicker))
                .on_action(record_action_listener!(Close))
                .on_action(record_action_listener!(CloseWindow))
                .on_action(record_action_listener!(PreviousRepository))
                .on_action(record_action_listener!(NextRepository))
                .on_action(record_action_listener!(MinimizeWindow))
                .on_action(record_action_listener!(ZoomWindow))
                .on_action(record_action_listener!(ToggleFullScreen))
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
            ("enter", crate::kit::Enter.name()),
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
    fn text_input_command_shortcuts_trigger_undo_and_redo(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            crate::kit::TextInput::new(
                crate::kit::TextInputOptions {
                    multiline: false,
                    ..Default::default()
                },
                window,
                cx,
            )
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
            crate::kit::TextInput::new(
                crate::kit::TextInputOptions {
                    multiline: false,
                    ..Default::default()
                },
                window,
                cx,
            )
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

    fn install_app_shortcuts_for_test(app: &mut App, backend: Arc<dyn GitBackend>) {
        bind_app_keys(app);
        install_app_actions(app, backend);
    }

    #[gpui::test]
    fn app_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
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
            ("secondary-shift-o", OpenRecentPicker.name()),
            ("secondary-w", Close.name()),
            ("secondary-shift-w", CloseWindow.name()),
            ("secondary-pageup", PreviousRepository.name()),
            ("secondary-pagedown", NextRepository.name()),
            ("secondary-q", Quit.name()),
        ];

        #[cfg(target_os = "macos")]
        cases.extend([
            ("alt-cmd-o", OpenRecentPicker.name()),
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
        cases.push(("f11", ToggleFullScreen.name()));

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
    fn settings_shortcut_opens_a_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        seed_workspace_repo(cx, &store, view);

        assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
        cx.simulate_keystrokes("secondary-,");
        cx.run_until_parked();
        assert_eq!(cx.update(|_window, app| app.windows().len()), 2);
    }

    #[gpui::test]
    fn settings_shortcut_reuses_existing_window_and_activates_it(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new(Arc::clone(&backend));
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
    fn recent_picker_shortcut_opens_the_popover(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new(Arc::clone(&backend));
        let store_for_view = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store_for_view, events, None, window, cx)
        });

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });
        seed_workspace_repo(cx, &store, view);

        cx.simulate_keystrokes("secondary-shift-o");
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(cx.debug_bounds("app_popover").is_some());
    }

    #[gpui::test]
    fn new_window_shortcuts_open_new_windows(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new(Arc::clone(&backend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            install_app_shortcuts_for_test(app, Arc::clone(&backend));
            let _ = window.draw(app);
            window.activate_window();
        });

        assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
        cx.simulate_keystrokes("secondary-n");
        cx.run_until_parked();
        assert_eq!(cx.update(|_window, app| app.windows().len()), 2);

        cx.simulate_keystrokes("secondary-shift-n");
        cx.run_until_parked();
        assert_eq!(cx.update(|_window, app| app.windows().len()), 3);
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
        let (store, events) = AppStore::new(Arc::clone(&backend));
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
        let (store, events) = AppStore::new(Arc::clone(&backend));
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
        let (store, events) = AppStore::new(Arc::clone(&backend));
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

    #[gpui::test]
    fn repository_picker_fallback_reuses_existing_normal_window(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new(Arc::clone(&backend));
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
            assert!(view.read(app).open_repo_panel_visible_for_test());
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
                .update(app, |view, _cx| view.open_repo_panel_visible_for_test())
                .expect("expected to inspect the new GitComet window")
        });
        assert_eq!(cx.update(|app| app.windows().len()), 1);
        assert!(panel_visible);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn focus_existing_repository_window_for_path_avoids_reading_the_active_window_on_stack(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
        let (store, events) = AppStore::new(Arc::clone(&backend));
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
        let (store, events) = AppStore::new(Arc::clone(&backend));
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
