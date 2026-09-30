//! The window registry: finding, focusing, closing, and quitting main windows.

use super::*;

#[derive(Clone, PartialEq)]
pub(super) struct GitCometWindowEntry {
    pub(super) handle: gpui::AnyWindowHandle,
    pub(super) view: gpui::WeakEntity<GitCometView>,
    pub(super) main_pane: gpui::WeakEntity<MainPaneView>,
    pub(super) view_mode: GitCometViewMode,
    pub(super) workspace_id: Option<session::WorkspaceId>,
    pub(super) repo_paths: std::sync::Arc<[PathBuf]>,
}

#[derive(Default)]
pub(super) struct GitCometWindowRegistry {
    pub(super) windows: FxHashMap<gpui::WindowId, GitCometWindowEntry>,
    pub(super) last_focused_normal_window: Option<gpui::WindowId>,
}

impl gpui::Global for GitCometWindowRegistry {}

#[derive(Default)]
pub(super) struct ProcessStartupHooksState {
    pub(super) ran: bool,
}

impl gpui::Global for ProcessStartupHooksState {}

#[cfg(test)]
#[derive(Default)]
pub(super) struct StartupHookInvocationCount(pub(super) usize, pub(super) Option<gpui::WindowId>);

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
pub(super) fn startup_hook_invocation_count_for_test(cx: &mut App) -> usize {
    cx.update_default_global::<StartupHookInvocationCount, _>(|count, _cx| count.0)
}

pub(super) fn run_process_startup_hooks_once(cx: &mut App, startup_window: Option<gpui::WindowId>) {
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

pub(super) fn gitcomet_window_entries(cx: &mut App) -> Vec<GitCometWindowEntry> {
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

pub(super) fn flush_open_workspace_environments(cx: &mut App) {
    for entry in gitcomet_window_entries(cx) {
        if entry.view_mode != GitCometViewMode::Normal {
            continue;
        }
        let _ = entry.view.update(cx, |view, cx| {
            view.flush_workspace_environment(cx);
        });
    }
}

pub(super) fn last_focused_normal_window_id(cx: &mut App) -> Option<gpui::WindowId> {
    cx.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
        registry.last_focused_normal_window
    })
}

pub(super) fn active_gitcomet_window_entry(cx: &mut App) -> Option<GitCometWindowEntry> {
    let active_window_id = cx.active_window()?.window_id();
    gitcomet_window_entries(cx)
        .into_iter()
        .find(|entry| entry.handle.window_id() == active_window_id)
}

pub(super) fn entry_contains_repo_path(entry: &GitCometWindowEntry, path: &Path) -> bool {
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

pub(super) fn active_normal_gitcomet_window(cx: &mut App) -> Option<GitCometWindowEntry> {
    let entry = active_gitcomet_window_entry(cx)?;
    (entry.view_mode == GitCometViewMode::Normal).then_some(entry)
}

pub(super) fn normal_gitcomet_window_by_id(
    cx: &mut App,
    window_id: gpui::WindowId,
) -> Option<GitCometWindowEntry> {
    gitcomet_window_entries(cx).into_iter().find(|entry| {
        entry.handle.window_id() == window_id && entry.view_mode == GitCometViewMode::Normal
    })
}

pub(super) fn update_active_normal_gitcomet_window<R>(
    cx: &mut App,
    f: impl FnOnce(&mut GitCometView, &mut gpui::Context<GitCometView>) -> R,
) -> Option<R> {
    let window = active_normal_gitcomet_window(cx)?;
    window.view.update(cx, f).ok()
}

pub(crate) fn update_active_or_existing_normal_gitcomet_window<R>(
    cx: &mut App,
    f: impl FnOnce(&mut GitCometView, &mut gpui::Context<GitCometView>) -> R,
) -> Option<R> {
    let window = find_normal_gitcomet_window(cx)?;
    window.view.update(cx, f).ok()
}

#[cfg(any(test, target_os = "macos"))]
pub(super) fn check_for_updates_in_active_or_existing_normal_window(cx: &mut App) -> bool {
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
pub(super) fn manual_update_check_menu_disabled(
    disabled_by_environment: bool,
    normal_window_available: bool,
) -> bool {
    disabled_by_environment || !normal_window_available
}

pub(super) fn normal_gitcomet_window_blocks_repository_management_actions(
    cx: &mut App,
    window: &GitCometWindowEntry,
) -> bool {
    window
        .view
        .update(cx, |view, _cx| view.blocks_repository_management_actions())
        .unwrap_or(false)
}

pub(super) fn existing_normal_gitcomet_window_blocks_repository_management_actions(
    cx: &mut App,
) -> bool {
    let Some(window) = find_normal_gitcomet_window(cx) else {
        return false;
    };
    normal_gitcomet_window_blocks_repository_management_actions(cx, &window)
}

pub(super) fn locate_file_in_active_or_existing_normal_window(cx: &mut App) {
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

pub(super) fn mark_clean_shutdown<C>(cx: &mut C)
where
    C: BorrowAppContext,
{
    cx.update_default_global::<CleanShutdownTracker, _>(|tracker, _cx| {
        tracker.requested.store(true, Ordering::SeqCst);
    });
}

pub(super) fn clear_clean_shutdown_request(cx: &mut App) {
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
pub(super) fn flush_workspace_environment_for(cx: &mut App, window_id: gpui::WindowId) {
    if let Some(entry) = normal_gitcomet_window_by_id(cx, window_id) {
        let _ = entry
            .view
            .update(cx, |view, cx| view.flush_workspace_environment(cx));
    }
}

pub(super) fn close_active_window(cx: &mut App) {
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

    // Running Git operations and extension guards, gathered from every
    // window; asked after the terminal prompt when there is one.
    let mut late_reasons: Vec<gpui::SharedString> = Vec::new();
    if running_command_count == 0 {
        for entry in &entries {
            if let Ok(reasons) = entry
                .view
                .read_with(cx, |view, cx| view.quit_close_guard_reasons(cx))
            {
                for reason in reasons {
                    if !late_reasons.contains(&reason) {
                        late_reasons.push(reason);
                    }
                }
            }
        }
        if late_reasons.is_empty() {
            mark_clean_shutdown(cx);
            cx.quit();
            return;
        }
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
        if late_reasons.is_empty() {
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
            let _ = entry.view.update(cx, |view, cx| {
                view.request_quit_close_guards(late_reasons, all_views, cx);
            });
            activate_gitcomet_window(cx, entry.handle);
        }
    } else {
        mark_clean_shutdown(cx);
        cx.quit();
    }
}

pub(super) fn find_normal_gitcomet_window(cx: &mut App) -> Option<GitCometWindowEntry> {
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
pub(super) fn find_normal_gitcomet_window_for_repo(
    cx: &mut App,
    path: &Path,
) -> Option<GitCometWindowEntry> {
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

pub(super) fn activate_gitcomet_window(cx: &mut App, window: gpui::AnyWindowHandle) {
    let _ = window.update(cx, |_view, window, _cx| {
        window.activate_window();
    });
}

pub(super) fn live_normal_windows(cx: &mut App) -> Vec<GitCometWindowEntry> {
    gitcomet_window_entries(cx)
        .into_iter()
        .filter(|entry| entry.view_mode == GitCometViewMode::Normal)
        .collect()
}
