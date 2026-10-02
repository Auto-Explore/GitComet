//! Repository and workspace routing between windows, and open requests.

use super::*;

#[cfg(target_os = "macos")]
pub(super) fn register_macos_open_request_handler(
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
                open_files_and_repositories(cx, backend, paths);
            });
        }
    })
    .detach();
}

#[cfg(target_os = "macos")]
fn open_files_and_repositories(cx: &mut App, backend: Arc<dyn GitBackend>, paths: Vec<PathBuf>) {
    cx.spawn(async move |cx| {
        let (directories, files): (Vec<_>, Vec<_>) =
            smol::unblock(move || paths.into_iter().partition(|path| path.is_dir())).await;
        cx.update(move |cx| {
            let directories: Vec<_> = directories
                .into_iter()
                .filter(|path| {
                    repository_entry_allowed(
                        cx,
                        path,
                        gitcomet_extension_api::EntryOrigin::CommandLine,
                        None,
                    )
                })
                .collect();
            if !directories.is_empty() {
                open_repositories_in_existing_or_new_window(cx, backend.clone(), directories);
            }
            if files.is_empty() {
                return;
            }
            if let Some(entry) = find_normal_gitcomet_window(cx) {
                let _ = entry
                    .view
                    .update(cx, |view, cx| view.open_document_paths(files, cx));
            } else {
                let handle = open_gitcomet_window(cx, backend, &normal_launch_config(None, None));
                let _ = handle.update(cx, |view, _, cx| view.open_document_paths(files, cx));
                activate_gitcomet_window(cx, handle.into());
            }
        });
    })
    .detach();
}

pub(super) fn register_browser_open_request_handler(
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

pub(super) fn activate_or_open_workspace(
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

pub(super) fn move_repository_to_workspace(
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

/// Runs the repository-entry gates for an app-level open. A denial is shown
/// in `window` (else any normal window) and the open stops there.
pub(super) fn repository_entry_allowed(
    cx: &mut App,
    path: &Path,
    origin: gitcomet_extension_api::EntryOrigin,
    window: Option<gpui::WindowId>,
) -> bool {
    let gitcomet_extension_api::GateDecision::Deny { reason } =
        crate::view::extension_host::entry_decision(path, origin, cx)
    else {
        return true;
    };
    let target = window
        .and_then(|id| normal_gitcomet_window_by_id(cx, id))
        .or_else(|| find_normal_gitcomet_window(cx));
    if let Some(target) = target {
        let _ = target.view.update(cx, |view, cx| {
            view.show_repository_entry_denial(reason, cx);
        });
    } else {
        eprintln!("{reason}");
    }
    false
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
        if !repository_entry_allowed(
            cx,
            &path,
            gitcomet_extension_api::EntryOrigin::Chooser,
            Some(source_window_id),
        ) {
            return;
        }
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
                // Already open here: a focus change, not an entry.
                focus_existing_repository_window(cx, &source, &path);
            } else if repository_entry_allowed(
                cx,
                &path,
                gitcomet_extension_api::EntryOrigin::Drop,
                Some(source_window_id),
            ) {
                let _ = source
                    .view
                    .update(cx, |view, cx| view.open_dropped_repo_locally(path, cx));
            }
        }
    });
}

pub(super) fn open_repository_in_window(cx: &mut App, window: &GitCometWindowEntry, path: PathBuf) {
    let _ = window.view.update(cx, |view, cx| {
        view.activate_or_open_repo_path(path, cx);
    });
    if cx.active_window().map(|active| active.window_id()) != Some(window.handle.window_id()) {
        activate_gitcomet_window(cx, window.handle);
    }
}

pub(super) fn focus_existing_repository_window(
    cx: &mut App,
    window: &GitCometWindowEntry,
    path: &Path,
) {
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

pub(super) fn normalize_repository_open_path(path: PathBuf) -> PathBuf {
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
pub(super) fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    let path = url.to_file_path().ok()?;
    (!path.as_os_str().is_empty()).then_some(path)
}

#[cfg(target_os = "macos")]
pub(super) fn repository_paths_from_open_urls(urls: &[String]) -> Vec<PathBuf> {
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

pub(super) fn open_repository_switcher_in_window(cx: &mut App, window: &GitCometWindowEntry) {
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

pub(super) fn open_workspace_picker_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
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

pub(super) fn open_repository_switcher_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
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
pub(super) fn open_clone_repository_in_window(cx: &mut App, window: &GitCometWindowEntry) {
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
pub(super) fn open_clone_repository_in_existing_or_new_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
) {
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
pub(super) fn prompt_initialize_repository_in_window(cx: &mut App, window: &GitCometWindowEntry) {
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
pub(super) fn prompt_initialize_repository_in_existing_or_new_window(
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

pub(super) fn toggle_command_palette_in_window(cx: &mut App, window: &GitCometWindowEntry) {
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
pub(super) fn toggle_reveal_commit_in_active_window(cx: &mut App) {
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
pub(super) fn open_remote_in_browser_in_active_window(cx: &mut App) {
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

pub(super) fn toggle_command_palette_in_active_existing_or_new_window(
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

pub(super) fn show_open_repository_manual_entry_in_window(
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

pub(super) fn show_open_repository_manual_entry_in_existing_or_new_window(
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

pub(super) fn open_repositories_in_existing_or_new_window(
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

pub(super) fn open_repository_in_existing_or_new_window(
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

pub(super) fn handle_browser_open_request(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    request: BrowserOpenRequest,
) {
    handle_browser_open_request_with_window(cx, backend, request, None);
}

pub(super) fn handle_browser_open_request_with_window(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    request: BrowserOpenRequest,
    preferred_window: Option<gpui::WindowId>,
) {
    handle_browser_open_request_and_activate(cx, backend, request, preferred_window, App::activate);
}

pub(super) fn handle_browser_open_request_and_activate(
    cx: &mut App,
    backend: Arc<dyn GitBackend>,
    request: BrowserOpenRequest,
    preferred_window: Option<gpui::WindowId>,
    // GPUI's headless platform ignores application activation; inject the
    // platform call so tests can verify it independently of window focus.
    activate: impl FnOnce(&App, bool),
) {
    // A denied path still brings a window forward, as an empty request does,
    // and that window shows why.
    let mut denial = None;
    let path = request
        .path
        .map(normalize_repository_open_path)
        .filter(|path| {
            match crate::view::extension_host::entry_decision(
                path,
                gitcomet_extension_api::EntryOrigin::CommandLine,
                cx,
            ) {
                gitcomet_extension_api::GateDecision::Allow => true,
                gitcomet_extension_api::GateDecision::Deny { reason } => {
                    denial = Some(reason);
                    false
                }
            }
        });
    match path {
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
    if let Some(reason) = denial
        && let Some(window) = preferred_window
            .and_then(|id| normal_gitcomet_window_by_id(cx, id))
            .or_else(|| find_normal_gitcomet_window(cx))
    {
        let _ = window.view.update(cx, |view, cx| {
            view.show_repository_entry_denial(reason, cx);
        });
    }
    // On macOS, making a window key does not unhide or foreground the app.
    activate(cx, true);
}

pub(super) fn prompt_open_repository(cx: &mut App, backend: Arc<dyn GitBackend>) {
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
            let path = normalize_repository_open_path(path);
            if !repository_entry_allowed(
                cx,
                &path,
                gitcomet_extension_api::EntryOrigin::Chooser,
                source_window_id,
            ) {
                return;
            }
            if let Some(window) =
                source_window_id.and_then(|id| normal_gitcomet_window_by_id(cx, id))
            {
                open_repository_in_window(cx, &window, path);
            } else {
                open_repository_in_existing_or_new_window(cx, Arc::clone(&backend), path);
            }
        });
    })
    .detach();
}

#[cfg(target_os = "macos")]
pub(super) fn prompt_apply_patch(cx: &mut App) {
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
