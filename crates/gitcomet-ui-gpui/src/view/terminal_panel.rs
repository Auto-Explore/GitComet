use super::terminal_alacritty::*;
use super::*;
#[cfg(unix)]
use rustix::process::{Pid, Signal, kill_process_group};
use std::path::PathBuf;

mod painting;
mod viewport;

#[cfg(test)]
mod tests;

/// How long a save-and-close waits for the dispatched writes to land before
/// closing anyway. A wedged command must not leave the user unable to quit.
const UNSAVED_FILE_EDITS_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const UNSAVED_FILE_EDITS_FLUSH_POLL: std::time::Duration = std::time::Duration::from_millis(25);
/// Minimum time to wait before believing an in-flight count of zero. A
/// `dispatch` is a channel send; the worker needs a turn to reduce it into a
/// running command, and until it has, "nothing in flight" means "not started".
const UNSAVED_FILE_EDITS_FLUSH_GRACE: std::time::Duration = std::time::Duration::from_millis(150);

/// Re-run whatever the unsaved-edits prompt interrupted.
fn retry_close_action(action: UnsavedFileEditsAction, cx: &mut gpui::App) {
    match action {
        UnsavedFileEditsAction::CloseWindow(window_id) => {
            crate::app::close_window_by_id_or_warn(cx, window_id)
        }
        UnsavedFileEditsAction::QuitApp => crate::app::quit_app_or_warn(cx),
    }
}

const TERMINAL_PANEL_MIN_HEIGHT_PX: f32 = 120.0;
const TERMINAL_FONT_SCALE: f32 = 0.92;
const TERMINAL_LINE_HEIGHT_SCALE: f32 = 1.15;
const TERMINAL_MIN_GRID_ROWS: u16 = 2;
const TERMINAL_MIN_GRID_COLS: u16 = 8;
const TERMINAL_CARET_WIDTH_RATIO: f32 = 0.12;
const TERMINAL_CARET_MIN_WIDTH_PX: f32 = 2.0;
const TERMINAL_CARET_MAX_WIDTH_PX: f32 = 3.0;
const TERMINAL_CARET_VERTICAL_INSET_PX: f32 = 1.0;
const TERMINAL_CARET_RADIUS_PX: f32 = 0.0;
const TERMINAL_CARET_BLINK_INTERVAL_MS: u64 = 530;
const TERMINAL_CARET_RESUME_DELAY_MS: u64 = 700;
const TERMINAL_SELECTION_ALPHA: f32 = 0.32;
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalShortcutAction {
    Copy,
    Paste,
    SelectAll,
}

/// Which surviving terminal receives focus after a stable-sequence close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalSurvivorFocusPolicy {
    /// Only refocus when the closed tab held keyboard focus (async exits).
    IfClosedTabWasFocused,
    /// Always focus the surviving tab (user-initiated closes).
    Always,
}

/// Drag payload used to track an in-progress terminal panel resize. Using the
/// drag/drag-move machinery (rather than element-local `on_mouse_move`) keeps
/// move events flowing even when the cursor leaves the thin handle bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalPanelResizeDrag;

impl GitCometView {
    /// Returns the viewport of the embedded terminal that currently holds
    /// keyboard focus in `window`, if any.
    pub(super) fn focused_terminal_viewport(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<Entity<TerminalViewportView>> {
        self.terminal_sessions
            .values()
            .flat_map(|session| session.instances.iter())
            .find(|instance| instance.viewport.read(cx).focus_handle.is_focused(window))
            .map(|instance| instance.viewport.clone())
    }

    /// Routes a keystroke to the focused embedded terminal before the app's
    /// global key bindings get a chance to run. A focused terminal must take
    /// priority over app shortcuts (e.g. `Ctrl+P`) so that the TUI running
    /// inside it receives its own shortcuts. Installed as a keystroke
    /// interceptor (see [`GitCometView::install_terminal_keystroke_interceptor`]),
    /// which fires before binding/action dispatch; when the terminal consumes
    /// the keystroke we stop propagation so no app action is triggered.
    pub(super) fn forward_keystroke_to_focused_terminal(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(viewport) = self.focused_terminal_viewport(window, cx) else {
            return;
        };
        viewport.update(cx, |viewport, cx| {
            viewport.handle_key_down(keystroke, cx);
        });
    }

    /// Installs an app-level keystroke interceptor that forwards keystrokes to a
    /// focused embedded terminal. Interceptors run before key bindings resolve
    /// to actions, so this is what lets the terminal swallow shortcuts that the
    /// app would otherwise claim (Ctrl+P, function keys, etc.). The returned
    /// [`gpui::Subscription`] must be stored for the interceptor to stay active.
    pub(super) fn install_terminal_keystroke_interceptor(
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Subscription {
        let view = cx.weak_entity();
        cx.intercept_keystrokes(move |event, window, cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| {
                this.forward_keystroke_to_focused_terminal(&event.keystroke, window, cx);
            });
        })
    }

    fn deactivate_terminal_cursor_blink(&mut self) {
        self.terminal_cursor_blink_active = false;
        self.terminal_cursor_blink_task_scheduled = false;
        self.terminal_cursor_blink_seq = self.terminal_cursor_blink_seq.wrapping_add(1);
        self.terminal_cursor_blink_visible = true;
        self.terminal_cursor_blink_hold_until = Instant::now();
    }

    fn schedule_terminal_cursor_blink_tick(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().uses_cursor_blink()
            || !self.terminal_cursor_blink_active
            || self.terminal_cursor_blink_task_scheduled
        {
            return;
        }
        self.terminal_cursor_blink_task_scheduled = true;
        let blink_seq = self.terminal_cursor_blink_seq;
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(TERMINAL_CARET_BLINK_INTERVAL_MS)).await;
                let _ = view.update(cx, |this, cx| {
                    this.advance_terminal_cursor_blink(blink_seq, cx)
                });
            },
        )
        .detach();
    }

    fn advance_terminal_cursor_blink(&mut self, blink_seq: u64, cx: &mut gpui::Context<Self>) {
        if self.terminal_cursor_blink_seq != blink_seq {
            return;
        }
        self.terminal_cursor_blink_task_scheduled = false;
        if !self.terminal_cursor_blink_active {
            self.terminal_cursor_blink_visible = true;
            return;
        }
        let now = Instant::now();
        if now < self.terminal_cursor_blink_hold_until {
            if !self.terminal_cursor_blink_visible {
                self.terminal_cursor_blink_visible = true;
                cx.notify();
            }
            self.schedule_terminal_cursor_blink_tick(cx);
            return;
        }
        self.terminal_cursor_blink_visible = !self.terminal_cursor_blink_visible;
        cx.notify();
        self.schedule_terminal_cursor_blink_tick(cx);
    }

    pub(in crate::view) fn apply_terminal_preferences(
        &mut self,
        next: TerminalPreferences,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.terminal_preferences == next {
            return;
        }
        self.terminal_preferences = next;
        self.sync_action_bar_terminal_target(cx);
        cx.notify();
    }

    pub(in crate::view) fn toggle_terminal_for_active_repo(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let workdir = repo.spec.workdir.clone();
        let repo_name = terminal_repo_name(&repo.spec.workdir);

        if self.terminal_sessions.contains_key(&repo_id) {
            if !self.request_close_terminal_for_repo(repo_id, cx) {
                self.close_terminal_for_repo(repo_id, cx);
            }
            return;
        }
        self.open_terminal_for_repo(repo_id, workdir, repo_name, window, cx);
    }

    pub(in crate::view) fn activate_terminal_button_for_active_repo(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match self.terminal_preferences.action_bar_terminal_target {
            ActionBarTerminalTarget::Embedded => self.toggle_terminal_for_active_repo(window, cx),
            ActionBarTerminalTarget::External => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.open_external_terminal_for_repo(repo_id, cx);
                }
            }
        }
    }

    fn reset_terminal_cursor_blink(&mut self, cx: &mut gpui::Context<Self>) {
        let was_visible = self.terminal_cursor_blink_visible;
        self.terminal_cursor_blink_visible = true;
        self.terminal_cursor_blink_hold_until =
            Instant::now() + Duration::from_millis(TERMINAL_CARET_RESUME_DELAY_MS);
        if !crate::ui_runtime::current().uses_cursor_blink() {
            self.terminal_cursor_blink_active = false;
            self.terminal_cursor_blink_task_scheduled = false;
        }
        self.schedule_terminal_cursor_blink_tick(cx);
        if !was_visible {
            cx.notify();
        }
    }

    fn active_repo_has_open_terminal(&self) -> bool {
        self.active_repo_id()
            .is_some_and(|repo_id| self.terminal_sessions.contains_key(&repo_id))
    }

    fn sync_terminal_indicator_views(&mut self, cx: &mut gpui::Context<Self>) {
        let repo_ids = self
            .terminal_sessions
            .keys()
            .copied()
            .collect::<FxHashSet<RepoId>>();
        let repo_tabs_bar = self.repo_tabs_bar.clone();
        let action_bar = self.action_bar.clone();
        cx.defer(move |cx| {
            repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_open_terminal_repo_ids(repo_ids.clone(), cx)
            });
            action_bar.update(cx, |bar, cx| bar.set_open_terminal_repo_ids(repo_ids, cx));
        });
    }

    pub(in crate::view) fn sync_action_bar_terminal_target(&self, cx: &mut gpui::Context<Self>) {
        let target = self.terminal_preferences.action_bar_terminal_target;
        let action_bar = self.action_bar.clone();
        cx.defer(move |cx| {
            action_bar.update(cx, |bar, cx| bar.set_action_bar_terminal_target(target, cx));
        });
    }

    pub(super) fn sync_terminal_sessions_with_state(&mut self, cx: &mut gpui::Context<Self>) {
        let active_repo_ids = self
            .state
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<FxHashSet<_>>();
        let removed_repo_ids: Vec<_> = self
            .terminal_sessions
            .keys()
            .copied()
            .filter(|repo_id| !active_repo_ids.contains(repo_id))
            .collect();
        if removed_repo_ids.is_empty() {
            return;
        }
        for repo_id in removed_repo_ids {
            if let Some(session) = self.terminal_sessions.remove(&repo_id) {
                for instance in &session.instances {
                    if let Some(pty) = &instance.pty_sender {
                        pty.shutdown();
                    }
                }
            }
        }
        if !self.active_repo_has_open_terminal() {
            self.deactivate_terminal_cursor_blink();
        }
        self.sync_terminal_indicator_views(cx);
        cx.notify();
    }

    fn open_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        workdir: PathBuf,
        repo_name: String,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.terminal_sessions.contains_key(&repo_id) {
            self.focus_terminal_view(repo_id, window, cx);
            return;
        }

        // Do PTY spawning synchronously (non-blocking on Linux - openpty is fast)
        let Some(instance) = self.spawn_terminal_instance(&workdir, cx) else {
            return;
        };
        let session_seq = instance.session_seq;
        self.terminal_sessions.insert(
            repo_id,
            RepoTerminalSession {
                workdir,
                repo_name,
                instances: vec![instance],
                active_index: 0,
            },
        );

        self.spawn_terminal_event_task(repo_id, session_seq, cx);
        self.reset_terminal_cursor_blink(cx);
        self.sync_terminal_indicator_views(cx);
        self.focus_terminal_view(repo_id, window, cx);
        cx.notify();
    }

    /// Spawn a new terminal tab in the existing session for `repo_id`.
    pub(in crate::view) fn add_terminal_tab_for_repo(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(workdir) = self
            .terminal_sessions
            .get(&repo_id)
            .map(|s| s.workdir.clone())
        else {
            return;
        };
        let Some(instance) = self.spawn_terminal_instance(&workdir, cx) else {
            return;
        };
        let session_seq = instance.session_seq;
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return;
        };
        session.instances.push(instance);
        session.active_index = session.instances.len() - 1;

        self.spawn_terminal_event_task(repo_id, session_seq, cx);
        self.reset_terminal_cursor_blink(cx);
        self.sync_terminal_indicator_views(cx);
        self.focus_terminal_view(repo_id, window, cx);
        cx.notify();
    }

    /// Spawn a PTY + alacritty terminal and wrap it in a `TerminalInstance`.
    /// Returns `None` (after surfacing a toast) when spawning fails.
    fn spawn_terminal_instance(
        &mut self,
        workdir: &std::path::Path,
        cx: &mut gpui::Context<Self>,
    ) -> Option<TerminalInstance> {
        let window_id = 0u64;
        let spawned = match spawn_alacritty_terminal(workdir, window_id) {
            Ok(spawned) => spawned,
            Err(err) => {
                self.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to start embedded terminal: {err}"),
                    cx,
                );
                return None;
            }
        };

        let session_seq = self.next_terminal_session_seq;
        self.next_terminal_session_seq = self.next_terminal_session_seq.wrapping_add(1).max(1);
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let theme = self.theme;

        let term_lock = spawned.term_lock;
        let pty_sender = spawned.pty_sender.clone();
        let events_rx = spawned.events_rx;

        let viewport = cx.new(|_cx| {
            TerminalViewportView::new(theme, focus_handle.clone(), term_lock, pty_sender.clone())
        });

        Some(TerminalInstance {
            focus_handle,
            pty_sender: Some(pty_sender),
            child_pid: spawned.child_pid,
            events_rx: Some(events_rx),
            connected: true,
            viewport,
            session_seq,
            title: terminal_tab_default_title(),
        })
    }

    fn spawn_terminal_event_task(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        cx: &mut gpui::Context<Self>,
    ) {
        let events_rx = self
            .terminal_sessions
            .get_mut(&repo_id)
            .and_then(|s| s.instance_by_seq_mut(session_seq))
            .and_then(|i| i.events_rx.take());
        let Some(events_rx) = events_rx else {
            return;
        };
        let window_handle = self.window_handle;

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let rx = events_rx;
                while let Ok(event) = rx.recv().await {
                    if matches!(&event, TerminalBackendEvent::Exit) {
                        // The shell is already gone, so close this tab without a
                        // running-command prompt. Resolve the tab from its stable
                        // session sequence at removal time: another tab may have
                        // closed and shifted every index while this event waited.
                        let _ = window_handle.update(cx, |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.close_terminal_tab_by_session_seq(
                                    repo_id,
                                    session_seq,
                                    TerminalSurvivorFocusPolicy::IfClosedTabWasFocused,
                                    window,
                                    cx,
                                );
                            });
                        });
                        break;
                    }

                    let result = view.update(cx, |this, cx| {
                        let Some(session) = this.terminal_sessions.get_mut(&repo_id) else {
                            return;
                        };
                        let Some(instance) = session.instance_by_seq_mut(session_seq) else {
                            return;
                        };

                        match event {
                            TerminalBackendEvent::Title(title) => {
                                if !title.is_empty() {
                                    instance.title = friendly_terminal_title(title);
                                    cx.notify();
                                }
                            }
                            TerminalBackendEvent::ClipboardStore(data) => {
                                crate::clipboard::write_text(
                                    cx,
                                    data,
                                    crate::clipboard::CopySource::TerminalProtocol,
                                );
                            }
                            TerminalBackendEvent::ClipboardLoad => {
                                if let Some(text) = crate::clipboard::read_text(cx)
                                    && let Some(ref pty) = instance.pty_sender
                                {
                                    pty.write(text.into_bytes());
                                }
                            }
                            TerminalBackendEvent::Bell => {}
                            TerminalBackendEvent::Exit => unreachable!(
                                "terminal exit events are handled before instance updates"
                            ),
                            TerminalBackendEvent::ChildExit(Some(0)) => {}
                            TerminalBackendEvent::ChildExit(code) => {
                                let msg = match code {
                                    Some(c) => format!("Child process exited with code {c}"),
                                    None => "Child process exited".to_string(),
                                };
                                gitcomet_core::process::write_stderr_line(format_args!(
                                    "terminal child process: {msg}"
                                ));
                            }
                            TerminalBackendEvent::Wakeup
                            | TerminalBackendEvent::CursorBlinkingChange => {
                                instance.viewport.update(cx, |viewport, cx| {
                                    viewport.content_epoch = viewport.content_epoch.wrapping_add(1);
                                    cx.notify();
                                });
                                cx.notify();
                            }
                            TerminalBackendEvent::PtyWrite(data) => {
                                // Terminal query responses (DSR, Device Attributes,
                                // etc.) must be written back to the PTY, or programs
                                // that probe the terminal hang waiting for a reply.
                                if let Some(ref pty) = instance.pty_sender {
                                    pty.write(data.into_bytes());
                                }
                            }
                        }
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();
    }

    pub(super) fn close_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(session) = self.terminal_sessions.remove(&repo_id) {
            for instance in &session.instances {
                shutdown_terminal_instance(instance, false);
            }
        }
        if !self.active_repo_has_open_terminal() {
            self.deactivate_terminal_cursor_blink();
        }
        self.sync_terminal_indicator_views(cx);
        cx.notify();
    }

    /// Close a single terminal tab. Closing the last tab closes the panel.
    pub(in crate::view) fn close_terminal_tab(
        &mut self,
        repo_id: RepoId,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.close_terminal_tab_inner(repo_id, index, true, window, cx);
    }

    fn close_terminal_tab_inner(
        &mut self,
        repo_id: RepoId,
        index: usize,
        focus_surviving_tab: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut session_emptied = false;
        match self.terminal_sessions.get_mut(&repo_id) {
            Some(session) => {
                if index >= session.instances.len() {
                    return;
                }
                let instance = session.instances.remove(index);
                shutdown_terminal_instance(&instance, false);
                if session.instances.is_empty() {
                    session_emptied = true;
                } else {
                    if session.active_index > index {
                        session.active_index -= 1;
                    }
                    if session.active_index >= session.instances.len() {
                        session.active_index = session.instances.len() - 1;
                    }
                }
            }
            None => return,
        }

        if session_emptied {
            self.terminal_sessions.remove(&repo_id);
        }
        if !self.active_repo_has_open_terminal() {
            self.deactivate_terminal_cursor_blink();
        } else if focus_surviving_tab && self.active_repo_id() == Some(repo_id) {
            self.focus_terminal_view(repo_id, window, cx);
        }
        self.sync_terminal_indicator_views(cx);
        cx.notify();
    }

    /// Close the terminal identified by its stable session sequence.
    ///
    /// Backend events are asynchronous and shutdown confirmations are delayed,
    /// so a tab's index at spawn time may no longer identify it; both close
    /// paths resolve through the sequence at close time instead. A missing
    /// sequence means the tab was already closed (or its repository went
    /// away), in which case the late event is intentionally ignored.
    ///
    /// `policy` picks the surviving tab's focus treatment: an asynchronous
    /// shell exit only refocuses when the exited tab held keyboard focus,
    /// while a user-initiated close always focuses the survivor.
    fn close_terminal_tab_by_session_seq(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        policy: TerminalSurvivorFocusPolicy,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(index) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|session| session.index_by_seq(session_seq))
        else {
            return;
        };
        let focus_surviving_tab = match policy {
            TerminalSurvivorFocusPolicy::Always => true,
            TerminalSurvivorFocusPolicy::IfClosedTabWasFocused => {
                let session = self
                    .terminal_sessions
                    .get(&repo_id)
                    .expect("session was present for the sequence lookup");
                self.active_repo_id() == Some(repo_id)
                    && session.instances[index].focus_handle.is_focused(window)
            }
        };
        self.close_terminal_tab_inner(repo_id, index, focus_surviving_tab, window, cx);
    }

    pub(in crate::view) fn select_terminal_tab(
        &mut self,
        repo_id: RepoId,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match self.terminal_sessions.get_mut(&repo_id) {
            Some(session) if index < session.instances.len() => {
                session.active_index = index;
            }
            _ => return,
        }
        self.focus_terminal_view(repo_id, window, cx);
        cx.notify();
    }

    fn focus_terminal_view(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(focus_handle) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .map(|i| i.focus_handle.clone())
        else {
            return;
        };
        window.focus(&focus_handle, cx);
        self.reset_terminal_cursor_blink(cx);
    }

    pub(crate) fn running_terminal_summary(&self) -> TerminalShutdownSummary {
        let mut summary = terminal_shutdown_summary_for_instances(
            self.terminal_sessions
                .values()
                .flat_map(|session| session.instances.iter()),
        );
        summary.repo_names = self.repo_names_with_running_terminals();
        summary
    }

    fn repo_names_with_running_terminals(&self) -> Vec<String> {
        self.terminal_sessions
            .iter()
            .filter(|(_, session)| {
                session
                    .instances
                    .iter()
                    .any(|i| i.connected && terminal_instance_has_running_command(i))
            })
            .map(|(_, session)| session.repo_name.clone())
            .collect()
    }

    fn terminal_shutdown_summary_for_action(
        &self,
        action: &TerminalShutdownAction,
    ) -> TerminalShutdownSummary {
        match action {
            TerminalShutdownAction::CloseRepo { repo_id }
            | TerminalShutdownAction::CloseTerminalForRepo { repo_id } => {
                let mut summary = self
                    .terminal_sessions
                    .get(repo_id)
                    .map(|session| {
                        terminal_shutdown_summary_for_instances(session.instances.iter())
                    })
                    .unwrap_or_default();
                if summary.running_command_count > 0
                    && let Some(session) = self.terminal_sessions.get(repo_id)
                {
                    summary.repo_names = vec![session.repo_name.clone()];
                }
                summary
            }
            TerminalShutdownAction::CloseTerminalTab {
                repo_id,
                session_seq,
            } => {
                let mut summary = self
                    .terminal_sessions
                    .get(repo_id)
                    .and_then(|session| session.instance_by_seq(*session_seq))
                    .map(|instance| {
                        terminal_shutdown_summary_for_instances(std::iter::once(instance))
                    })
                    .unwrap_or_default();
                if summary.running_command_count > 0
                    && let Some(session) = self.terminal_sessions.get(repo_id)
                {
                    summary.repo_names = vec![session.repo_name.clone()];
                }
                summary
            }
            TerminalShutdownAction::CloseWindow | TerminalShutdownAction::QuitApp => {
                self.running_terminal_summary()
            }
        }
    }

    fn queue_terminal_shutdown_prompt(
        &mut self,
        action: TerminalShutdownAction,
        summary: TerminalShutdownSummary,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_terminal_shutdown_prompt = Some(TerminalShutdownPrompt { action, summary });
        cx.notify();
    }

    pub(in crate::view) fn request_terminal_shutdown_action(
        &mut self,
        action: TerminalShutdownAction,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let summary = self.terminal_shutdown_summary_for_action(&action);
        if summary.running_command_count == 0 {
            return false;
        }
        self.queue_terminal_shutdown_prompt(action, summary, cx);
        true
    }

    pub(crate) fn request_close_window_or_warn(
        &mut self,
        window_id: gpui::WindowId,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self
            .request_unsaved_file_edits_prompt(UnsavedFileEditsAction::CloseWindow(window_id), cx)
        {
            return true;
        }
        self.request_terminal_shutdown_action(TerminalShutdownAction::CloseWindow, cx)
    }

    /// [`Self::request_unsaved_file_edits_prompt`] for a quit, callable from
    /// the app-level shutdown path (which cannot name the action enum).
    pub(crate) fn request_quit_unsaved_file_edits_prompt(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.request_unsaved_file_edits_prompt(UnsavedFileEditsAction::QuitApp, cx)
    }

    /// Queue the unsaved-edits dialog if the editor is holding writes that
    /// closing would throw away. Returns whether it took over the action.
    ///
    /// Resolving it re-runs the original request rather than closing directly,
    /// so a window with both unsaved edits and a running command still gets the
    /// terminal warning afterwards.
    pub(in crate::view) fn request_unsaved_file_edits_prompt(
        &mut self,
        action: UnsavedFileEditsAction,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        // `pending_*_prompt` is `take()`n by `Render` when it opens the popover,
        // so it is `None` for as long as the dialog is actually on screen. Ask
        // the popover host whether the dialog is up rather than mirroring that
        // into a bool: a mirror only stays true, and every way the popover can
        // go away without being closed — `open_popover` replacing it, say —
        // would leave it stuck and the window permanently unclosable.
        if self.pending_unsaved_file_edits_prompt.is_some()
            || self.unsaved_file_edits_dialog_open(cx)
        {
            return true;
        }
        // With auto-save on, a buffer inside its 800 ms quiet period is not an
        // unsaved edit — it is a write that has not fired yet, so the user is
        // asked nothing. But flushing only *dispatches* the write, and returning
        // `false` here let the caller quit out from under it: the store never
        // reduced the message and the edits were lost. Take over the close and
        // let it through once the write has actually drained.
        let flushed_a_pending_write = self.main_pane.update(cx, |pane, cx| {
            let pending = pane.auto_save_file_edits && !pane.unsaved_file_edit_labels().is_empty();
            pane.flush_file_editor_buffer(cx);
            pending
        });
        if flushed_a_pending_write {
            self.retry_once_file_edit_writes_drain(action, cx);
            return true;
        }
        let files = self.main_pane.read(cx).unsaved_file_edit_labels();
        if files.is_empty() {
            return false;
        }
        self.pending_unsaved_file_edits_prompt = Some(UnsavedFileEditsPrompt { action, files });
        cx.notify();
        true
    }

    /// Whether the unsaved-edits dialog is the popover currently on screen.
    fn unsaved_file_edits_dialog_open(&self, cx: &gpui::App) -> bool {
        self.popover_host
            .read(cx)
            .showing_unsaved_file_edits_prompt()
    }

    pub(in crate::view) fn clear_pending_unsaved_file_edits_prompt(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_unsaved_file_edits_prompt = None;
        cx.notify();
    }

    /// Save or discard the unsaved buffers, then retry what the user asked for.
    ///
    /// Discarding can retry immediately, but saving cannot: the writes go
    /// through the store's command executor, and `cx.quit()` on the next flush
    /// would race them — the app would exit with some files still unwritten.
    /// `local_actions_in_flight` is the store's own count of exactly those
    /// commands, so the retry waits for it to drain (bounded, so a wedged
    /// command cannot trap the user in an app that will not close).
    pub(in crate::view) fn resolve_unsaved_file_edits(
        &mut self,
        action: UnsavedFileEditsAction,
        save: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_unsaved_file_edits_prompt = None;
        self.main_pane.update(cx, |pane, cx| {
            if save {
                pane.save_all_file_edits(cx);
            } else {
                pane.discard_all_file_edits(cx);
            }
        });

        if !save {
            // Ordering note: the caller's `close_popover` defers a clear of
            // `pending_unsaved_file_edits_prompt`, and it runs *after* this
            // retry. If the retry finds edits still outstanding and queues a
            // fresh prompt, that clear would silently swallow it and the close
            // would do nothing — so the retry is deferred behind the clear.
            cx.defer(move |cx| cx.defer(move |cx| retry_close_action(action, cx)));
            return;
        }
        self.retry_once_file_edit_writes_drain(action, cx);
    }

    /// Re-run `action` once the dispatched worktree writes have landed.
    ///
    /// `dispatch` is a channel send, so the store worker needs a turn before
    /// `local_actions_in_flight` means anything — quitting on the count it reads
    /// immediately would exit with the writes still queued.
    fn retry_once_file_edit_writes_drain(
        &mut self,
        action: UnsavedFileEditsAction,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_unsaved_file_edits_flush = Some(cx.spawn(async move |view, cx| {
            let started = std::time::Instant::now();
            let deadline = started + UNSAVED_FILE_EDITS_FLUSH_TIMEOUT;
            loop {
                cx.background_executor()
                    .timer(UNSAVED_FILE_EDITS_FLUSH_POLL)
                    .await;
                let now = std::time::Instant::now();
                if now >= deadline {
                    break;
                }
                if now.duration_since(started) < UNSAVED_FILE_EDITS_FLUSH_GRACE {
                    continue;
                }
                let drained = view
                    .read_with(cx, |view, _cx| {
                        !view
                            .state
                            .repos
                            .iter()
                            .any(|repo| repo.local_actions_in_flight > 0)
                    })
                    .unwrap_or(true);
                if drained {
                    break;
                }
            }
            cx.update(move |cx| cx.defer(move |cx| retry_close_action(action, cx)));
        }));
    }

    pub(crate) fn request_quit_or_warn(
        &mut self,
        terminal_count: usize,
        running_command_count: usize,
        repo_names: Vec<String>,
        other_window_views: Vec<gpui::WeakEntity<Self>>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let summary = TerminalShutdownSummary {
            terminal_count,
            running_command_count,
            repo_names,
        };
        if summary.running_command_count == 0 {
            return false;
        }
        self.pending_quit_other_views = other_window_views;
        self.queue_terminal_shutdown_prompt(TerminalShutdownAction::QuitApp, summary, cx);
        true
    }

    pub(in crate::view) fn clear_pending_terminal_shutdown_prompt(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_terminal_shutdown_prompt = None;
        cx.notify();
    }

    pub(in crate::view) fn confirm_terminal_shutdown(
        &mut self,
        prompt: TerminalShutdownPrompt,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_terminal_shutdown_prompt = None;
        terminate_terminals_for_action(self, &prompt.action);
        match prompt.action {
            TerminalShutdownAction::CloseRepo { repo_id } => {
                self.store.dispatch(Msg::CloseRepo { repo_id });
                cx.notify();
            }
            TerminalShutdownAction::CloseTerminalForRepo { repo_id } => {
                self.close_terminal_for_repo(repo_id, cx);
            }
            TerminalShutdownAction::CloseTerminalTab {
                repo_id,
                session_seq,
            } => {
                self.close_terminal_tab_by_session_seq(
                    repo_id,
                    session_seq,
                    TerminalSurvivorFocusPolicy::Always,
                    window,
                    cx,
                );
            }
            TerminalShutdownAction::CloseWindow => {
                crate::app::mark_clean_shutdown_if_last_window_from_view(cx);
                window.remove_window();
            }
            TerminalShutdownAction::QuitApp => {
                for weak in self.pending_quit_other_views.drain(..) {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |v, _cx| {
                            for session in v.terminal_sessions.values() {
                                for instance in &session.instances {
                                    shutdown_terminal_instance(instance, true);
                                }
                            }
                        });
                    }
                }
                crate::app::mark_clean_shutdown_from_view(cx);
                cx.quit();
            }
        }
    }

    pub(super) fn request_close_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.request_terminal_shutdown_action(
            TerminalShutdownAction::CloseTerminalForRepo { repo_id },
            cx,
        )
    }

    fn request_close_terminal_tab(
        &mut self,
        repo_id: RepoId,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(session_seq) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|session| session.instances.get(index))
            .map(|instance| instance.session_seq)
        else {
            return;
        };
        if !self.request_terminal_shutdown_action(
            TerminalShutdownAction::CloseTerminalTab {
                repo_id,
                session_seq,
            },
            cx,
        ) {
            self.close_terminal_tab(repo_id, index, window, cx);
        }
    }

    pub(super) fn send_terminal_bytes_for_repo(&mut self, repo_id: RepoId, bytes: Vec<u8>) {
        if let Some(pty) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .and_then(|i| i.pty_sender.as_ref())
        {
            pty.write(bytes);
        }
    }

    pub(super) fn copy_terminal_selection_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(viewport) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .map(|i| i.viewport.clone())
        else {
            return false;
        };
        let Some(text) = viewport.read(cx).selected_text() else {
            return false;
        };
        crate::clipboard::write_text(cx, text, crate::clipboard::CopySource::TerminalContextMenu);
        true
    }

    pub(super) fn paste_terminal_clipboard_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(text) = crate::clipboard::read_text(cx) else {
            return false;
        };
        let Some(viewport) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .map(|i| i.viewport.clone())
        else {
            return false;
        };
        viewport.update(cx, |v, cx| v.paste_text(&text, cx));
        true
    }

    pub(super) fn select_all_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(viewport) = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .map(|i| i.viewport.clone())
        {
            viewport.update(cx, |v, cx| v.select_all(cx));
        }
    }

    pub(super) fn clear_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) {
        self.send_terminal_bytes_for_repo(repo_id, vec![0x0c]);
    }

    pub(in crate::view) fn terminal_launch_context_for_active_repo(
        &self,
    ) -> Option<ExternalTerminalLaunchContext> {
        let repo = self.active_repo()?;
        Some(terminal_launch_context_for_repo_state(
            repo,
            self.terminal_sessions.get(&repo.id),
        ))
    }

    // -- Panel rendering --

    pub(super) fn render_terminal_panel(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let active_repo = self.active_repo_id()?;

        let (viewport_entity, connected, tabs, active_index) = {
            let session = self.terminal_sessions.get(&active_repo)?;
            let active = session.active_instance()?;
            let tabs: Vec<SharedString> = session
                .instances
                .iter()
                .map(|inst| SharedString::from(inst.title.clone()))
                .collect();
            (
                active.viewport.clone(),
                active.connected,
                tabs,
                session.active_index,
            )
        };
        let has_selection = viewport_entity.read(cx).has_selection();
        let mouse_mode = viewport_entity
            .read(cx)
            .last_content
            .as_ref()
            .map(|c| c.mode.mouse_mode())
            .unwrap_or(false);
        // When the terminal holds keyboard focus, app shortcuts are routed to
        // the embedded TUI instead of the app. Surface that state so the user
        // understands why their usual shortcuts behave differently.
        let terminal_focused = viewport_entity.read(cx).focus_handle.is_focused(window);

        let header = self.render_terminal_header(
            theme,
            active_repo,
            &tabs,
            active_index,
            terminal_focused,
            cx,
        );
        let viewport_element = div()
            .flex_1()
            .min_h(px(0.0))
            // Breathing room so the first/last column doesn't touch the panel
            // edge; the viewport measures its own bounds, so the grid adapts.
            .px(crate::ui_scale::design_px_from_percent(
                6.0,
                self.ui_scale_percent,
            ))
            .key_context("Terminal")
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    // When the running program has requested mouse reporting
                    // (e.g. a full-screen TUI), forward the click instead of
                    // showing our context menu.
                    if mouse_mode {
                        return;
                    }
                    cx.stop_propagation();
                    let context = TerminalMenuContext {
                        has_session: true,
                        has_selection,
                        connected,
                    };
                    let invoker: SharedString = format!("terminal_menu_{}", active_repo.0).into();
                    this.set_active_context_menu_invoker(Some(invoker), cx);
                    this.open_popover_at(
                        PopoverKind::TerminalMenu {
                            repo_id: active_repo,
                            context,
                        },
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .child(viewport_entity)
            .into_any_element();

        Some(
            div()
                .flex()
                .flex_col()
                .h(self.terminal_panel_height)
                .min_h(px(TERMINAL_PANEL_MIN_HEIGHT_PX))
                .bg(terminal_default_background(theme))
                // A focus ring along the top edge reinforces that the terminal is
                // capturing keyboard input. The border is always present (kept
                // transparent when unfocused) so toggling focus never shifts layout.
                .border_t_2()
                .border_color(if terminal_focused {
                    theme.colors.interaction.focus_ring
                } else {
                    with_alpha(theme.colors.interaction.focus_ring, 0.0)
                })
                .child(header)
                .child(viewport_element)
                .into_any(),
        )
    }

    fn render_terminal_header(
        &mut self,
        theme: AppTheme,
        active_repo: RepoId,
        tabs: &[SharedString],
        active_index: usize,
        focused: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let external_repo = active_repo;
        let clear_repo = active_repo;
        let close_repo = active_repo;
        let repo_id = active_repo;

        let icon_btn = move |id: &'static str, icon: &'static str, tip: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded(px(theme.radii.row))
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(theme.colors.interaction.hover_background))
                .child(svg_icon(icon, theme.colors.foreground.primary, px(14.0)))
                .gitcomet_tooltip(theme, tip.into())
        };

        let mut tabs_row = div()
            .id("terminal_tabs_scroll")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .flex_1()
            .min_w(px(0.0))
            .overflow_x_scroll()
            .scrollbar_width(px(0.0));

        for (i, title) in tabs.iter().enumerate() {
            let is_active = i == active_index;
            let tab_bg = if is_active {
                theme.colors.interaction.selected_background
            } else {
                theme.colors.surface.panel
            };
            let text_color = if is_active {
                theme.colors.interaction.selected_foreground
            } else {
                theme.colors.foreground.secondary
            };

            let close = div()
                .id(("terminal_tab_close", i))
                .flex()
                .items_center()
                .justify_center()
                .size(px(14.0))
                .rounded(px(theme.radii.row))
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(with_alpha(theme.colors.status.danger.foreground, 0.18)))
                .child(svg_icon("icons/generic_close.svg", text_color, px(10.0)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.request_close_terminal_tab(repo_id, i, window, cx);
                    }),
                );

            let tab = div()
                .id(("terminal_tab", i))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(theme.radii.row))
                .bg(tab_bg)
                .text_color(text_color)
                .text_size(px(12.0))
                .flex_none()
                .cursor(CursorStyle::PointingHand)
                .when(!is_active, |d| {
                    d.hover(move |s| s.bg(theme.colors.interaction.hover_background))
                })
                .child(svg_icon("icons/terminal.svg", text_color, px(12.0)))
                .child(title.clone())
                .child(close)
                .gitcomet_tooltip(theme, title.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                        this.select_terminal_tab(repo_id, i, window, cx);
                    }),
                );

            tabs_row = tabs_row.child(tab);
        }

        let new_tab = div()
            .id("terminal_new_tab")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(20.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.colors.interaction.hover_background))
            .child(svg_icon(
                "icons/plus.svg",
                theme.colors.foreground.primary,
                px(12.0),
            ))
            .gitcomet_tooltip(theme, "New terminal".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                    this.add_terminal_tab_for_repo(repo_id, window, cx);
                }),
            );

        tabs_row = tabs_row.child(new_tab);

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(4.0))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(tabs_row)
            .when(focused, |row| {
                // Badge that explains why the usual app shortcuts (Ctrl+P, etc.)
                // are being swallowed: the terminal currently owns the keyboard.
                row.child(
                    div()
                        .id("terminal_focus_badge")
                        .flex()
                        .flex_none()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(theme.radii.row))
                        .bg(with_alpha(theme.colors.accent.foreground, 0.15))
                        .child(
                            div()
                                .size(px(6.0))
                                .rounded(px(3.0))
                                .bg(theme.colors.accent.foreground),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme.colors.foreground.primary)
                                .child("Keyboard captured"),
                        )
                        .gitcomet_tooltip(
                            theme,
                            "Terminal has keyboard focus — app shortcuts are sent to the \
                             terminal. Click outside the terminal to release."
                                .into(),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        icon_btn(
                            "terminal_open_external",
                            "icons/open_external.svg",
                            "Open in external terminal",
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                this.open_external_terminal_for_repo(external_repo, cx);
                            }),
                        ),
                    )
                    .child(
                        icon_btn(
                            "terminal_clear",
                            "icons/broom.svg",
                            "Clear terminal (Ctrl+L)",
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                                this.clear_terminal_for_repo(clear_repo, window, cx);
                            }),
                        ),
                    )
                    .child(
                        icon_btn(
                            "terminal_close",
                            "icons/generic_close.svg",
                            "Close terminal",
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                cx.stop_propagation();
                                if !this.request_close_terminal_for_repo(close_repo, cx) {
                                    this.close_terminal_for_repo(close_repo, cx);
                                }
                            }),
                        ),
                    ),
            )
            .into_any()
    }

    pub(super) fn open_external_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let workdir = self
            .terminal_sessions
            .get(&repo_id)
            .map(|s| s.workdir.clone())
            .or_else(|| {
                self.state
                    .repos
                    .iter()
                    .find(|r| r.id == repo_id)
                    .map(|r| r.spec.workdir.clone())
            });
        if let Some(wd) = workdir {
            let context = ExternalTerminalLaunchContext {
                cwd: wd,
                repo_name: self
                    .terminal_sessions
                    .get(&repo_id)
                    .map(|s| s.repo_name.clone()),
            };
            match resolve_external_terminal_launch_spec(&self.terminal_preferences, &context) {
                Ok(spec) => super::platform_open::spawn_launch(
                    cx,
                    move || spec.launch(),
                    |this, result, cx| {
                        if let Err(err) = result {
                            this.push_toast(
                                components::ToastKind::Error,
                                format!("Failed to open external terminal: {err}"),
                                cx,
                            );
                        }
                    },
                ),
                Err(err) => self.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open external terminal: {err}"),
                    cx,
                ),
            }
        }
    }

    pub(super) fn open_external_terminal_from_menu(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_external_terminal_for_repo(repo_id, cx);
    }

    pub(super) fn terminal_panel_resize_handle(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        gpui::div()
            .id("terminal_panel_resize")
            .group("terminal_panel_resize")
            .h(px(TERMINAL_PANEL_RESIZE_HANDLE_PX))
            .w_full()
            .cursor(CursorStyle::ResizeUpDown)
            .child(components::resize_grip(
                theme,
                self.ui_scale_percent,
                "terminal_panel_resize",
                components::ResizeGripAxis::Horizontal,
                self.terminal_panel_resize.is_some(),
                Some(theme.colors.stroke.subtle),
            ))
            .on_drag(TerminalPanelResizeDrag, |_payload, _offset, _window, cx| {
                cx.new(|_cx| super::mod_helpers::ResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    crate::press_gesture::claim_press(cx);
                    this.terminal_panel_resize = Some(TerminalPanelResizeState {
                        start_y: e.position.y,
                        start_height: this.terminal_panel_height,
                    });
                    cx.notify();
                }),
            )
            .on_drag_move(cx.listener(
                move |this, e: &gpui::DragMoveEvent<TerminalPanelResizeDrag>, _w, cx| {
                    let Some(state) = this.terminal_panel_resize else {
                        return;
                    };
                    let new_height = (state.start_height + (state.start_y - e.event.position.y))
                        .max(px(TERMINAL_PANEL_MIN_HEIGHT_PX));
                    if this.terminal_panel_height != new_height {
                        this.terminal_panel_height = new_height;
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.terminal_panel_resize.take().is_some() {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.terminal_panel_resize.take().is_some() {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            )
            .into_any()
    }
}

impl Drop for GitCometView {
    fn drop(&mut self) {
        // Fallback teardown for OS-level window close / unwind that bypasses the
        // explicit shutdown flow. SIGTERM the child process group (a no-op on a
        // group already terminating) so commands aren't left as orphans, then
        // close the PTY. A repeated shutdown is safe.
        for session in self.terminal_sessions.values() {
            for instance in &session.instances {
                terminate_terminal_process_group(instance.child_pid);
                if let Some(ref pty) = instance.pty_sender {
                    pty.shutdown();
                }
            }
        }
    }
}

/// Default tab title for a freshly-spawned terminal: the shell program's base
/// name (e.g. "zsh"), falling back to "Terminal".
fn terminal_tab_default_title() -> String {
    resolve_embedded_shell_program()
        .ok()
        .and_then(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Terminal".to_string())
}

/// Console titles that are just the shell executable's path (conhost's
/// default on Windows, e.g. `C:\Program Files\PowerShell\7\pwsh.exe`)
/// collapse to the program stem ("pwsh"); anything else is a deliberate
/// application-set title and passes through untouched.
fn friendly_terminal_title(title: String) -> String {
    let Some(program) = title
        .contains(['\\', '/'])
        .then(|| title.rsplit(['\\', '/']).next())
        .flatten()
    else {
        return title;
    };
    let Some((stem, extension)) = program.rsplit_once('.') else {
        return title;
    };
    if stem.is_empty() || !extension.eq_ignore_ascii_case("exe") {
        return title;
    }
    stem.to_owned()
}

fn terminal_repo_name(workdir: &std::path::Path) -> String {
    workdir
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path_display::path_display_string(workdir))
}

fn terminal_launch_context_for_repo_state(
    repo: &RepoState,
    session: Option<&RepoTerminalSession>,
) -> ExternalTerminalLaunchContext {
    ExternalTerminalLaunchContext {
        cwd: session
            .map(|s| s.workdir.clone())
            .unwrap_or_else(|| repo.spec.workdir.clone()),
        repo_name: session.map(|s| s.repo_name.clone()).or_else(|| {
            repo.spec
                .workdir
                .file_name()
                .and_then(|n| n.to_str())
                .map(ToOwned::to_owned)
        }),
    }
}

fn terminal_shutdown_summary_for_instances<'a>(
    instances: impl IntoIterator<Item = &'a TerminalInstance>,
) -> TerminalShutdownSummary {
    let mut summary = TerminalShutdownSummary::default();
    for instance in instances {
        if !instance.connected {
            continue;
        }
        summary.terminal_count += 1;
        if terminal_instance_has_running_command(instance) {
            summary.running_command_count += 1;
        }
    }
    summary
}

fn terminal_instance_has_running_command(instance: &TerminalInstance) -> bool {
    if !instance.connected {
        return false;
    }
    instance
        .child_pid
        .is_some_and(terminal_process_has_running_child_command)
}

fn terminate_terminals_for_action(view: &mut GitCometView, action: &TerminalShutdownAction) {
    match action {
        TerminalShutdownAction::CloseRepo { repo_id }
        | TerminalShutdownAction::CloseTerminalForRepo { repo_id } => {
            if let Some(session) = view.terminal_sessions.get(repo_id) {
                for instance in &session.instances {
                    terminate_terminal_process_group(instance.child_pid);
                }
            }
        }
        TerminalShutdownAction::CloseTerminalTab {
            repo_id,
            session_seq,
        } => {
            if let Some(instance) = view
                .terminal_sessions
                .get(repo_id)
                .and_then(|session| session.instance_by_seq(*session_seq))
            {
                terminate_terminal_process_group(instance.child_pid);
            }
        }
        TerminalShutdownAction::CloseWindow | TerminalShutdownAction::QuitApp => {
            for session in view.terminal_sessions.values() {
                for instance in &session.instances {
                    shutdown_terminal_instance(instance, true);
                }
            }
        }
    }
}

fn shutdown_terminal_instance(instance: &TerminalInstance, terminate: bool) {
    if terminate {
        terminate_terminal_process_group(instance.child_pid);
    }
    if let Some(ref pty) = instance.pty_sender {
        pty.shutdown();
    }
}

/// Returns whether the shell process `pid` has at least one child process, which
/// indicates a command is currently running (an idle interactive shell has none).
/// Works uniformly across platforms via `sysinfo`. Called only on user-initiated
/// close, so a one-shot process snapshot is acceptable.
fn terminal_process_has_running_child_command(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    // We must enumerate all processes to find any whose *parent* is `pid` (a
    // child-of-pid query can't be narrowed to a single PID), but we only read
    // `parent()`, which is base info — so skip the expensive cmd/environ/exe/cwd
    // field collection that `everything()` would do for every process.
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    let target = sysinfo::Pid::from_u32(pid);
    system
        .processes()
        .values()
        .any(|process| process.parent() == Some(target))
}

#[cfg(unix)]
fn terminate_terminal_process_group(child_pid: Option<u32>) {
    let Some(child_pid) = child_pid else {
        return;
    };
    let Some(pid) = Pid::from_raw(child_pid as i32) else {
        return;
    };
    let _ = kill_process_group(pid, Signal::TERM);
}

#[cfg(not(unix))]
fn terminate_terminal_process_group(_child_pid: Option<u32>) {}

fn terminal_clipboard_shortcut_action(
    keystroke: &gpui::Keystroke,
) -> Option<TerminalShortcutAction> {
    let action = match keystroke.key.as_str() {
        "c" | "C" => TerminalShortcutAction::Copy,
        "v" | "V" => TerminalShortcutAction::Paste,
        "a" | "A" => TerminalShortcutAction::SelectAll,
        _ => return None,
    };
    let mods = keystroke.modifiers;
    if cfg!(target_os = "macos") {
        if mods.platform && !mods.control && !mods.alt && !mods.function && !mods.shift {
            Some(action)
        } else {
            None
        }
    } else if mods.control && mods.shift && !mods.platform && !mods.alt && !mods.function {
        Some(action)
    } else {
        None
    }
}
