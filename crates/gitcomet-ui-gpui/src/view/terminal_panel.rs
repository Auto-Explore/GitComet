use super::*;
use portable_pty::{CommandBuilder, PtySize};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::sync::Mutex;

const TERMINAL_INITIAL_ROWS: u16 = 24;
const TERMINAL_INITIAL_COLS: u16 = 80;
const TERMINAL_MIN_GRID_ROWS: u16 = 2;
const TERMINAL_MIN_GRID_COLS: u16 = 8;
const TERMINAL_SCROLLBACK_ROWS: usize = 10_000;
const TERMINAL_READ_CHUNK_BYTES: usize = 8192;
const TERMINAL_WRITE_QUEUE_CAPACITY: usize = 256;
const TERMINAL_READ_BATCH_DELAY_MS: u64 = 1;
const TERMINAL_READ_BATCH_MAX_BYTES: usize = TERMINAL_READ_CHUNK_BYTES * 8;
const TERMINAL_WRITE_BATCH_MAX_BYTES: usize = TERMINAL_READ_CHUNK_BYTES * 8;
const TERMINAL_FONT_SCALE: f32 = 0.92;
const TERMINAL_LINE_HEIGHT_SCALE: f32 = 1.0;
const TERMINAL_CELL_WIDTH_SAMPLE: &str = "0000000000";
const TERMINAL_CARET_WIDTH_RATIO: f32 = 0.12;
const TERMINAL_CARET_MIN_WIDTH_PX: f32 = 2.0;
const TERMINAL_CARET_MAX_WIDTH_PX: f32 = 3.0;
const TERMINAL_CARET_VERTICAL_INSET_PX: f32 = 1.0;
const TERMINAL_CARET_RADIUS_PX: f32 = 0.0;
const TERMINAL_CARET_BLINK_INTERVAL_MS: u64 = 530;
const TERMINAL_CARET_RESUME_DELAY_MS: u64 = 700;
const TERMINAL_SELECTION_ALPHA: f32 = 0.32;
const TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS: usize = 24;
const TERMINAL_DEFAULT_BG_HEX: u32 = 0x000000;
const TERMINAL_DEFAULT_FG_HEX: u32 = 0xffffff;
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

struct SpawnedTerminalSession {
    io: Arc<Mutex<TerminalIo>>,
    reader: Box<dyn Read + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

#[derive(Clone, Debug, PartialEq)]
struct TerminalCellStyle {
    fg: gpui::Rgba,
    bg: Option<gpui::Rgba>,
    bold: bool,
    italic: bool,
    underline: bool,
}

#[derive(Default)]
struct TerminalCanvasPaintState {
    selection_rects: Vec<Bounds<Pixels>>,
    lines: Vec<(ShapedLine, Point<Pixels>, Pixels)>,
    cursor: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalRowSignature {
    fingerprint: u64,
    paints: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TerminalReadCompletion {
    Eof,
    Error(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalReadBatchAction {
    None,
    ScheduleFlush,
    FlushNow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalShortcutAction {
    Copy,
    Paste,
    SelectAll,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalReadBatch {
    bytes: Vec<u8>,
    completion: Option<TerminalReadCompletion>,
}

#[derive(Debug, Default)]
struct TerminalReadBatchState {
    bytes: Vec<u8>,
    completion: Option<TerminalReadCompletion>,
    flush_scheduled: bool,
}

#[derive(Debug)]
struct TerminalDirtyRowRenderInput {
    row: u16,
    fingerprint: u64,
    text: Option<SharedString>,
    runs: Vec<TextRun>,
}

#[derive(Debug, Eq, PartialEq)]
enum EmbeddedTerminalSpawnFailureAction {
    OpenExternalWithWarning(String),
    ShowError(String),
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
            .map(|session| session.workdir.clone())
            .unwrap_or_else(|| repo.spec.workdir.clone()),
        repo_name: session
            .map(|session| session.repo_name.clone())
            .or_else(|| {
                repo.spec
                    .workdir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
            }),
    }
}

fn resolve_external_terminal_launch_context(
    state: &AppState,
    terminal_sessions: &HashMap<RepoId, RepoTerminalSession>,
    repo_id: RepoId,
) -> Result<ExternalTerminalLaunchContext, String> {
    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .ok_or_else(|| "Repository is no longer available.".to_string())?;
    Ok(terminal_launch_context_for_repo_state(
        repo,
        terminal_sessions.get(&repo_id),
    ))
}

fn resolve_embedded_terminal_spawn_failure<F>(
    preferences: &TerminalPreferences,
    context: &ExternalTerminalLaunchContext,
    err: &str,
    launch_external: F,
) -> EmbeddedTerminalSpawnFailureAction
where
    F: FnOnce(&TerminalPreferences, &ExternalTerminalLaunchContext) -> Result<(), String>,
{
    if preferences.external_terminal_fallback {
        match launch_external(preferences, context) {
            Ok(()) => {
                return EmbeddedTerminalSpawnFailureAction::OpenExternalWithWarning(format!(
                    "Embedded terminal failed to start ({err}); opened the configured external terminal instead."
                ));
            }
            Err(launch_err) => {
                return EmbeddedTerminalSpawnFailureAction::ShowError(format!(
                    "Failed to start embedded terminal: {err}. External terminal fallback also failed: {launch_err}"
                ));
            }
        }
    }

    EmbeddedTerminalSpawnFailureAction::ShowError(format!(
        "Failed to start embedded terminal: {err}"
    ))
}

fn next_terminal_panel_height(
    state: TerminalPanelResizeState,
    current_y: Pixels,
    window_height: Pixels,
) -> Pixels {
    let delta = state.start_y - current_y;
    let max_height = terminal_panel_height_for_window(window_height);
    (state.start_height + delta)
        .max(px(TERMINAL_PANEL_MIN_HEIGHT_PX))
        .min(max_height)
}

impl TerminalViewportView {
    fn new(
        theme: AppTheme,
        focus_handle: FocusHandle,
        session: TerminalSessionHandle,
        io: Arc<Mutex<TerminalIo>>,
    ) -> Self {
        Self {
            theme,
            focus_handle,
            session,
            io,
            layout_cache: None,
            render_cache: TerminalRenderCache::default(),
            cursor_blink_visible: true,
            cursor_blink_hold_until: Instant::now(),
            cursor_blink_active: false,
            cursor_blink_task_scheduled: false,
            cursor_blink_seq: 0,
        }
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.invalidate_layout(cx);
    }

    pub(super) fn invalidate_layout(&mut self, cx: &mut gpui::Context<Self>) {
        self.layout_cache = None;
        self.render_cache = TerminalRenderCache::default();
        cx.notify();
    }

    fn terminal_layout_snapshot(
        &mut self,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> TerminalLayoutCache {
        let rem_size = window.rem_size();
        if let Some(cache) = self.layout_cache.as_ref()
            && cache.rem_size == rem_size
        {
            return cache.clone();
        }

        let base_style = terminal_text_style(self.theme, window, cx);
        let cache = terminal_layout_cache(base_style, window);
        self.layout_cache = Some(cache.clone());
        cache
    }

    fn deactivate_cursor_blink(&mut self) {
        self.cursor_blink_active = false;
        self.cursor_blink_task_scheduled = false;
        self.cursor_blink_seq = self.cursor_blink_seq.wrapping_add(1);
        self.cursor_blink_visible = true;
        self.cursor_blink_hold_until = Instant::now();
    }

    fn schedule_cursor_blink_tick(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().uses_cursor_blink()
            || !self.cursor_blink_active
            || self.cursor_blink_task_scheduled
        {
            return;
        }

        self.cursor_blink_task_scheduled = true;
        let blink_seq = self.cursor_blink_seq;
        cx.spawn(
            async move |view: WeakEntity<TerminalViewportView>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(TERMINAL_CARET_BLINK_INTERVAL_MS)).await;
                let _ = view.update(cx, |this, cx| this.advance_cursor_blink(blink_seq, cx));
            },
        )
        .detach();
    }

    fn cursor_blink_should_run(&self, window: &Window) -> bool {
        let Ok(state) = self.session.state.lock() else {
            return false;
        };
        let screen = state.parser.screen();
        state.connected
            && !screen.hide_cursor()
            && screen.scrollback() == 0
            && self.focus_handle.is_focused(window)
    }

    fn sync_cursor_blink_activity(&mut self, window: &Window, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().uses_cursor_blink() {
            let was_hidden = !self.cursor_blink_visible;
            if self.cursor_blink_active || self.cursor_blink_task_scheduled || was_hidden {
                self.deactivate_cursor_blink();
                if was_hidden {
                    cx.notify();
                }
            }
            return;
        }

        if self.cursor_blink_should_run(window) {
            if !self.cursor_blink_active {
                self.cursor_blink_active = true;
                self.cursor_blink_seq = self.cursor_blink_seq.wrapping_add(1);
            }
            self.schedule_cursor_blink_tick(cx);
            return;
        }

        if self.cursor_blink_active || !self.cursor_blink_visible {
            self.deactivate_cursor_blink();
        }
    }

    fn advance_cursor_blink(&mut self, blink_seq: u64, cx: &mut gpui::Context<Self>) {
        if self.cursor_blink_seq != blink_seq {
            return;
        }

        self.cursor_blink_task_scheduled = false;
        if !self.cursor_blink_active {
            self.cursor_blink_visible = true;
            return;
        }

        let now = Instant::now();
        if now < self.cursor_blink_hold_until {
            if !self.cursor_blink_visible {
                self.cursor_blink_visible = true;
                cx.notify();
            }
            self.schedule_cursor_blink_tick(cx);
            return;
        }

        self.cursor_blink_visible = !self.cursor_blink_visible;
        cx.notify();
        self.schedule_cursor_blink_tick(cx);
    }

    fn reset_cursor_blink(&mut self, cx: &mut gpui::Context<Self>) {
        let was_visible = self.cursor_blink_visible;
        self.cursor_blink_visible = true;
        self.cursor_blink_hold_until =
            Instant::now() + Duration::from_millis(TERMINAL_CARET_RESUME_DELAY_MS);
        if !crate::ui_runtime::current().uses_cursor_blink() {
            self.cursor_blink_active = false;
            self.cursor_blink_task_scheduled = false;
        }
        self.schedule_cursor_blink_tick(cx);
        if !was_visible {
            cx.notify();
        }
    }

    fn queue_input(&mut self, bytes: Vec<u8>, cx: &mut gpui::Context<Self>) -> Result<(), String> {
        let reset_scrollback = {
            let mut state = self
                .session
                .state
                .lock()
                .map_err(|_| "Terminal state lock was poisoned.".to_string())?;
            if !state.connected {
                return Err("Terminal is no longer connected.".to_string());
            }
            reset_terminal_scrollback(&mut state)
        };

        match self
            .session
            .writer_tx
            .try_send(TerminalWriteRequest::Bytes(bytes))
        {
            Ok(()) => {
                self.reset_cursor_blink(cx);
                if reset_scrollback {
                    cx.notify();
                }
                Ok(())
            }
            Err(smol::channel::TrySendError::Closed(_)) => {
                mark_terminal_disconnected(
                    &self.session.state,
                    Some("Failed to send input: terminal input queue is closed.".to_string()),
                );
                cx.notify();
                Err("Failed to enqueue terminal input: terminal input queue is closed.".to_string())
            }
            Err(smol::channel::TrySendError::Full(_)) => {
                Err("Failed to enqueue terminal input: terminal input queue is full.".to_string())
            }
        }
    }

    fn handle_key_down(&mut self, event: &gpui::KeyDownEvent, cx: &mut gpui::Context<Self>) {
        if self.handle_scrollback_key(event, cx) {
            cx.stop_propagation();
            return;
        }

        let application_cursor = self
            .session
            .state
            .lock()
            .ok()
            .map(|state| state.parser.screen().application_cursor());
        if let Some(bytes) =
            application_cursor.and_then(|mode| encode_terminal_key_input(&event.keystroke, mode))
        {
            let _ = self.queue_input(bytes, cx);
        }
        cx.stop_propagation();
    }

    fn handle_scrollback_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if mods.control || mods.alt || mods.platform || mods.function || !mods.shift {
            return false;
        }

        let visible_rows = self
            .session
            .state
            .lock()
            .ok()
            .map(|state| state.parser.screen().size().0)
            .unwrap_or(TERMINAL_INITIAL_ROWS)
            .max(1);
        let page_rows = usize::from(visible_rows.saturating_sub(1)).max(1);

        let changed = match key {
            "pageup" => adjust_terminal_scrollback(&self.session.state, page_rows as isize),
            "pagedown" => adjust_terminal_scrollback(&self.session.state, -(page_rows as isize)),
            "home" => scroll_terminal_to_scrollback(&self.session.state, usize::MAX),
            "end" => scroll_terminal_to_scrollback(&self.session.state, 0),
            _ => false,
        };
        if changed {
            cx.notify();
        }
        changed
    }

    fn handle_scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let metrics = self.terminal_layout_snapshot(window, cx).metrics;
        let Some((delta_y, step_rows)) = terminal_scroll_wheel_delta(event, metrics.line_height)
        else {
            return;
        };
        cx.stop_propagation();

        let alternate_screen = self.session.state.lock().ok().map(|state| {
            (
                state.parser.screen().alternate_screen(),
                state.parser.screen().application_cursor(),
            )
        });
        let Some((alternate_screen, application_cursor)) = alternate_screen else {
            return;
        };

        if alternate_screen {
            let bytes =
                terminal_alternate_screen_scroll_bytes(delta_y, step_rows, application_cursor);
            let _ = self.queue_input(bytes, cx);
            return;
        }

        let changed = if delta_y > px(0.0) {
            adjust_terminal_scrollback(&self.session.state, step_rows as isize)
        } else {
            adjust_terminal_scrollback(&self.session.state, -(step_rows as isize))
        };
        if changed {
            cx.notify();
        }
    }

    fn sync_terminal_grid_size(&mut self, next_size: TerminalGridSize) {
        sync_terminal_grid_size_state(&self.session.state, &self.io, next_size);
    }

    fn build_terminal_canvas_paint_state(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> TerminalCanvasPaintState {
        let layout = self.terminal_layout_snapshot(window, cx);
        let next_size = terminal_grid_size(bounds, layout.metrics);
        self.sync_terminal_grid_size(next_size);
        self.sync_cursor_blink_activity(window, cx);

        let (rows, cols, viewport_key, dirty_rows, cursor_bounds) =
            collect_terminal_render_snapshot(
                &self.session.state,
                self.render_cache.viewport_key,
                &layout.base_style,
                self.theme,
                layout.key,
                layout.metrics,
                bounds,
                self.cursor_blink_visible,
                self.focus_handle.is_focused(window),
            );

        if self.render_cache.viewport_key != Some(viewport_key) {
            self.render_cache
                .rows
                .resize_with(usize::from(rows), TerminalCachedRow::default);
            for row in 0..rows {
                if !dirty_rows.iter().any(|input| input.row == row) {
                    // Force rebuild when the visible viewport shape changes.
                    let Some(slot) = self.render_cache.rows.get_mut(usize::from(row)) else {
                        continue;
                    };
                    slot.fingerprint = 0;
                }
            }
            self.render_cache.viewport_key = Some(viewport_key);
        } else {
            self.render_cache
                .rows
                .resize_with(usize::from(rows), TerminalCachedRow::default);
        }

        for input in dirty_rows {
            let cache_row = &mut self.render_cache.rows[usize::from(input.row)];
            cache_row.fingerprint = input.fingerprint;
            cache_row.layout_key = layout.key;
            cache_row.shaped = input.text.map(|text| {
                window
                    .text_system()
                    .shape_line(text, layout.metrics.font_size, &input.runs, None)
            });
            #[cfg(test)]
            {
                self.render_cache.rebuilt_rows += 1;
            }
        }

        let mut paint_state = TerminalCanvasPaintState {
            selection_rects: Vec::new(),
            lines: Vec::with_capacity(usize::from(rows)),
            cursor: cursor_bounds,
        };
        for (row, cache_row) in self
            .render_cache
            .rows
            .iter()
            .enumerate()
            .take(usize::from(rows))
        {
            let Some(shaped) = cache_row.shaped.clone() else {
                continue;
            };
            paint_state.lines.push((
                shaped,
                point(
                    bounds.left(),
                    bounds.top() + layout.metrics.line_height * row as f32,
                ),
                layout.metrics.line_height,
            ));
        }

        let _ = cols;
        paint_state
    }
}

impl Render for TerminalViewportView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        let theme = self.theme;
        div()
            .track_focus(&self.focus_handle)
            .w_full()
            .h_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.reset_cursor_blink(cx);
                }),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _window, cx| {
                this.handle_key_down(e, cx);
            }))
            .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, window, cx| {
                this.handle_scroll_wheel(e, window, cx);
            }))
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        view.update(cx, |this, cx| {
                            this.build_terminal_canvas_paint_state(bounds, window, cx)
                        })
                    },
                    move |_bounds, paint_state, window, cx| {
                        paint_terminal_canvas_state(paint_state, theme, window, cx);
                    },
                )
                .w_full()
                .h_full(),
            )
    }
}

impl GitCometView {
    fn terminal_layout_snapshot(
        &mut self,
        theme: AppTheme,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> TerminalLayoutCache {
        let base_style = terminal_text_style(theme, window, cx);
        terminal_layout_cache(base_style, window)
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

    fn sync_terminal_cursor_blink_activity(
        &mut self,
        repo_id: RepoId,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !crate::ui_runtime::current().uses_cursor_blink() {
            let was_hidden = !self.terminal_cursor_blink_visible;
            if self.terminal_cursor_blink_active
                || self.terminal_cursor_blink_task_scheduled
                || was_hidden
            {
                self.deactivate_terminal_cursor_blink();
                if was_hidden {
                    cx.notify();
                }
            }
            return;
        }

        let should_run = self.terminal_cursor_blink_should_run(repo_id, window);
        if should_run {
            if !self.terminal_cursor_blink_active {
                self.terminal_cursor_blink_active = true;
                self.terminal_cursor_blink_seq = self.terminal_cursor_blink_seq.wrapping_add(1);
            }
            self.schedule_terminal_cursor_blink_tick(cx);
            return;
        }

        if self.terminal_cursor_blink_active || !self.terminal_cursor_blink_visible {
            self.deactivate_terminal_cursor_blink();
        }
    }

    fn terminal_cursor_blink_should_run(&self, repo_id: RepoId, window: &Window) -> bool {
        self.terminal_sessions.get(&repo_id).is_some_and(|session| {
            let Ok(state) = session.terminal.state.lock() else {
                return false;
            };
            let screen = state.parser.screen();
            state.connected
                && !screen.hide_cursor()
                && screen.scrollback() == 0
                && session.focus_handle.is_focused(window)
        })
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
        cx.notify();
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
            self.close_terminal_for_repo(repo_id, cx);
            return;
        }

        self.open_terminal_for_repo(repo_id, workdir, repo_name, window, cx);
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
            .collect::<HashSet<RepoId>>();
        let repo_tabs_bar = self.repo_tabs_bar.clone();
        let action_bar = self.action_bar.clone();
        cx.defer(move |cx| {
            repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_open_terminal_repo_ids(repo_ids.clone(), cx)
            });
            action_bar.update(cx, |bar, cx| bar.set_open_terminal_repo_ids(repo_ids, cx));
        });
    }

    pub(super) fn sync_terminal_sessions_with_state(&mut self, cx: &mut gpui::Context<Self>) {
        let active_repo_ids = self
            .state
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<HashSet<_>>();
        let removed_repo_ids = self
            .terminal_sessions
            .keys()
            .copied()
            .filter(|repo_id| !active_repo_ids.contains(repo_id))
            .collect::<Vec<_>>();
        if removed_repo_ids.is_empty() {
            return;
        }

        for repo_id in removed_repo_ids {
            if let Some(mut session) = self.terminal_sessions.remove(&repo_id) {
                terminate_terminal_session(&mut session);
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
        workdir: std::path::PathBuf,
        repo_name: String,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.terminal_sessions.contains_key(&repo_id) {
            self.focus_terminal_view(repo_id, window, cx);
            return;
        }

        let context = ExternalTerminalLaunchContext {
            cwd: workdir.clone(),
            repo_name: Some(repo_name.clone()),
        };
        let spawned = match spawn_terminal_session(&self.terminal_preferences, &workdir) {
            Ok(spawned) => spawned,
            Err(err) => {
                match resolve_embedded_terminal_spawn_failure(
                    &self.terminal_preferences,
                    &context,
                    &err,
                    launch_external_terminal_from_preferences,
                ) {
                    EmbeddedTerminalSpawnFailureAction::OpenExternalWithWarning(message) => {
                        self.push_toast(components::ToastKind::Warning, message, cx);
                    }
                    EmbeddedTerminalSpawnFailureAction::ShowError(message) => {
                        self.push_toast(components::ToastKind::Error, message, cx);
                    }
                }
                return;
            }
        };

        let session_seq = self.next_terminal_session_seq;
        self.next_terminal_session_seq = self.next_terminal_session_seq.wrapping_add(1).max(1);

        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let (writer_tx, writer_rx) =
            smol::channel::bounded::<TerminalWriteRequest>(TERMINAL_WRITE_QUEUE_CAPACITY);
        let terminal = TerminalSessionHandle {
            state: Arc::new(Mutex::new(initial_terminal_session_state())),
            writer_tx,
        };
        let viewport = cx.new(|_cx| {
            TerminalViewportView::new(
                self.theme,
                focus_handle.clone(),
                terminal.clone(),
                spawned.io.clone(),
            )
        });
        self.terminal_sessions.insert(
            repo_id,
            RepoTerminalSession {
                workdir,
                repo_name,
                focus_handle,
                io: spawned.io,
                parser: vt100::Parser::new(
                    TERMINAL_INITIAL_ROWS,
                    TERMINAL_INITIAL_COLS,
                    TERMINAL_SCROLLBACK_ROWS,
                ),
                grid_size: initial_terminal_grid_size(),
                content_epoch: 0,
                render_cache: TerminalRenderCache::default(),
                session_seq,
                connected: true,
                exit_status: None,
                terminal,
                viewport,
                selection: None,
                selection_drag_anchor: None,
                viewport_bounds: None,
            },
        );
        let writer = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|session| session.io.lock().ok()?.writer.take());
        if let Some(writer) = writer {
            self.spawn_terminal_writer_task(repo_id, session_seq, writer_rx, writer, cx);
        }
        self.spawn_terminal_reader_task(repo_id, session_seq, spawned.reader, cx);
        self.spawn_terminal_wait_task(repo_id, session_seq, spawned.child, cx);
        self.reset_terminal_cursor_blink(cx);
        self.sync_terminal_indicator_views(cx);
        self.focus_terminal_view(repo_id, window, cx);
        cx.notify();
    }

    fn close_terminal_for_repo(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        let Some(mut session) = self.terminal_sessions.remove(&repo_id) else {
            return;
        };
        terminate_terminal_session(&mut session);
        if !self.active_repo_has_open_terminal() {
            self.deactivate_terminal_cursor_blink();
        }
        self.sync_terminal_indicator_views(cx);
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
            .map(|session| session.focus_handle.clone())
        else {
            return;
        };
        window.focus(&focus_handle, cx);
        self.reset_terminal_cursor_blink(cx);
    }

    fn spawn_terminal_writer_task(
        &self,
        repo_id: RepoId,
        session_seq: u64,
        writer_rx: smol::channel::Receiver<TerminalWriteRequest>,
        writer: Box<dyn Write + Send>,
        cx: &mut gpui::Context<Self>,
    ) {
        let uses_background_compute = crate::ui_runtime::current().uses_background_compute();
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let mut writer = writer;
                loop {
                    let request = match writer_rx.recv().await {
                        Ok(request) => request,
                        Err(_) => break,
                    };

                    let mut bytes = Vec::new();
                    let mut shutdown = false;
                    match request {
                        TerminalWriteRequest::Bytes(chunk) => bytes.extend_from_slice(&chunk),
                        TerminalWriteRequest::Shutdown => break,
                    }

                    while bytes.len() < TERMINAL_WRITE_BATCH_MAX_BYTES {
                        match writer_rx.try_recv() {
                            Ok(TerminalWriteRequest::Bytes(chunk)) => {
                                bytes.extend_from_slice(&chunk);
                            }
                            Ok(TerminalWriteRequest::Shutdown) => {
                                shutdown = true;
                                break;
                            }
                            Err(smol::channel::TryRecvError::Empty)
                            | Err(smol::channel::TryRecvError::Closed) => break,
                        }
                    }

                    let (next_writer, result) = if uses_background_compute {
                        smol::unblock(move || write_terminal_bytes(writer, bytes)).await
                    } else {
                        write_terminal_bytes(writer, bytes)
                    };
                    writer = next_writer;
                    if let Err(err) = result {
                        let _ = view.update(cx, |this, cx| {
                            let Some(session) = this.terminal_sessions.get_mut(&repo_id) else {
                                return;
                            };
                            if session.session_seq != session_seq {
                                return;
                            }
                            session.connected = false;
                            if session.exit_status.is_none() {
                                session.exit_status = Some(format!("Failed to send input: {err}"));
                            }
                            mark_terminal_disconnected(
                                &session.terminal.state,
                                Some(format!("Failed to send input: {err}")),
                            );
                            cx.notify();
                        });
                        break;
                    }

                    if shutdown {
                        break;
                    }
                }
            },
        )
        .detach();
    }

    fn spawn_terminal_reader_task(
        &self,
        repo_id: RepoId,
        session_seq: u64,
        reader: Box<dyn Read + Send>,
        cx: &mut gpui::Context<Self>,
    ) {
        let batch_state = Arc::new(Mutex::new(TerminalReadBatchState::default()));
        let uses_background_compute = crate::ui_runtime::current().uses_background_compute();
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let mut reader = reader;
                loop {
                    let (next_reader, result) = if uses_background_compute {
                        smol::unblock(move || read_next_terminal_chunk(reader)).await
                    } else {
                        read_next_terminal_chunk(reader)
                    };
                    reader = next_reader;
                    match result {
                        Ok(Some(bytes)) => {
                            let action = match batch_state.lock() {
                                Ok(mut state) => push_terminal_read_bytes(&mut state, bytes),
                                Err(_) => TerminalReadBatchAction::FlushNow,
                            };
                            match action {
                                TerminalReadBatchAction::None => {}
                                TerminalReadBatchAction::ScheduleFlush => {
                                    if uses_background_compute {
                                        let batch_state = batch_state.clone();
                                        let _ = view.update(cx, |this, cx| {
                                            this.spawn_terminal_read_flush_task(
                                                repo_id,
                                                session_seq,
                                                batch_state,
                                                cx,
                                            );
                                        });
                                    }
                                }
                                TerminalReadBatchAction::FlushNow => {
                                    let batch_state = batch_state.clone();
                                    let _ = view.update(cx, |this, cx| {
                                        this.flush_terminal_read_batch(
                                            repo_id,
                                            session_seq,
                                            &batch_state,
                                            cx,
                                        );
                                    });
                                }
                            }
                        }
                        Ok(None) => {
                            let batch_state = batch_state.clone();
                            let _ = view.update(cx, |this, cx| {
                                if let Ok(mut state) = batch_state.lock() {
                                    state.completion = Some(TerminalReadCompletion::Eof);
                                }
                                this.flush_terminal_read_batch(
                                    repo_id,
                                    session_seq,
                                    &batch_state,
                                    cx,
                                );
                            });
                            break;
                        }
                        Err(err) => {
                            let err = err.to_string();
                            let batch_state = batch_state.clone();
                            let _ = view.update(cx, |this, cx| {
                                if let Ok(mut state) = batch_state.lock() {
                                    state.completion = Some(TerminalReadCompletion::Error(err));
                                }
                                this.flush_terminal_read_batch(
                                    repo_id,
                                    session_seq,
                                    &batch_state,
                                    cx,
                                );
                            });
                            break;
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn spawn_terminal_read_flush_task(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        batch_state: Arc<Mutex<TerminalReadBatchState>>,
        cx: &mut gpui::Context<Self>,
    ) {
        if !crate::ui_runtime::current().uses_background_compute() {
            self.flush_terminal_read_batch(repo_id, session_seq, &batch_state, cx);
            return;
        }

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(TERMINAL_READ_BATCH_DELAY_MS)).await;
                let _ = view.update(cx, |this, cx| {
                    this.flush_terminal_read_batch(repo_id, session_seq, &batch_state, cx);
                });
            },
        )
        .detach();
    }

    fn flush_terminal_read_batch(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        batch_state: &Arc<Mutex<TerminalReadBatchState>>,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(batch) = take_terminal_read_batch(batch_state) else {
            return;
        };
        let has_bytes = !batch.bytes.is_empty();
        let has_completion = batch.completion.is_some();

        {
            let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
                return;
            };
            if session.session_seq != session_seq {
                return;
            }

            if has_bytes {
                session.parser.process(&batch.bytes);
                session.content_epoch = session.content_epoch.wrapping_add(1);
                if terminal_clear_normal_selection(&mut session.selection) {
                    session.selection_drag_anchor = None;
                }
            }

            if let Some(completion) = batch.completion {
                session.connected = false;
                if let TerminalReadCompletion::Error(err) = completion
                    && session.exit_status.is_none()
                {
                    session.exit_status = Some(format!("Terminal I/O failed: {err}"));
                }
            }
        }

        if has_bytes {
            self.reset_terminal_cursor_blink(cx);
        }
        if has_bytes || has_completion {
            cx.notify();
        }
    }

    fn spawn_terminal_wait_task(
        &self,
        repo_id: RepoId,
        session_seq: u64,
        mut child: Box<dyn portable_pty::Child + Send + Sync>,
        cx: &mut gpui::Context<Self>,
    ) {
        let uses_background_compute = crate::ui_runtime::current().uses_background_compute();
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let result = if uses_background_compute {
                    smol::unblock(move || child.wait()).await
                } else {
                    child.wait()
                };
                let _ = view.update(cx, |this, cx| {
                    let Some(session) = this.terminal_sessions.get_mut(&repo_id) else {
                        return;
                    };
                    if session.session_seq != session_seq {
                        return;
                    }
                    session.connected = false;
                    session.exit_status = Some(match result {
                        Ok(status) => {
                            if let Some(signal) = status.signal() {
                                format!("Shell exited with signal {signal}.")
                            } else {
                                format!("Shell exited with code {}.", status.exit_code())
                            }
                        }
                        Err(err) => format!("Failed to wait for shell exit: {err}"),
                    });
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn sync_terminal_grid_size(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        next_size: TerminalGridSize,
    ) {
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return;
        };
        if session.session_seq != session_seq {
            return;
        }
        if session.grid_size == next_size {
            return;
        }

        let current_size = session.parser.screen().size();
        let mut size_changed = false;
        if current_size != (next_size.rows, next_size.cols) {
            session
                .parser
                .screen_mut()
                .set_size(next_size.rows, next_size.cols);
            session.content_epoch = session.content_epoch.wrapping_add(1);
            if terminal_clear_normal_selection(&mut session.selection) {
                session.selection_drag_anchor = None;
            }
            size_changed = true;
        }

        let mut pty_synced = false;
        if let Ok(mut io) = session.io.lock() {
            let pty_size = next_size.into_pty_size();
            if io.size != pty_size {
                if io.master.resize(pty_size).is_ok() {
                    io.size = pty_size;
                    pty_synced = true;
                }
            } else {
                pty_synced = true;
            }
        }

        if size_changed || pty_synced {
            session.grid_size = next_size;
        }
    }

    fn handle_terminal_key_down(
        &mut self,
        repo_id: RepoId,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(action) = terminal_clipboard_shortcut_action(&event.keystroke) {
            match action {
                TerminalShortcutAction::Copy => {
                    let _ = self.copy_terminal_selection_for_repo(repo_id, window, cx);
                }
                TerminalShortcutAction::Paste => {
                    let _ = self.paste_terminal_clipboard_for_repo(repo_id, window, cx);
                }
                TerminalShortcutAction::SelectAll => {
                    self.select_all_terminal_for_repo(repo_id, window, cx);
                }
            }
            cx.stop_propagation();
            return;
        }

        if self.handle_terminal_scrollback_key(repo_id, event, cx) {
            cx.stop_propagation();
            return;
        }

        let application_cursor = self
            .terminal_sessions
            .get(&repo_id)
            .map(|session| session.parser.screen().application_cursor());
        if let Some(bytes) =
            application_cursor.and_then(|mode| encode_terminal_key_input(&event.keystroke, mode))
        {
            let _ = self.send_terminal_bytes_for_repo(repo_id, &bytes, cx);
        }
        cx.stop_propagation();
    }

    fn handle_terminal_scrollback_key(
        &mut self,
        repo_id: RepoId,
        event: &gpui::KeyDownEvent,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if mods.control || mods.alt || mods.platform || mods.function || !mods.shift {
            return false;
        }

        let visible_rows = self
            .terminal_sessions
            .get(&repo_id)
            .map(|session| session.parser.screen().size().0)
            .unwrap_or(TERMINAL_INITIAL_ROWS)
            .max(1);
        let page_rows = usize::from(visible_rows.saturating_sub(1)).max(1);

        let changed = match key {
            "pageup" => self.adjust_terminal_scrollback(repo_id, page_rows as isize),
            "pagedown" => self.adjust_terminal_scrollback(repo_id, -(page_rows as isize)),
            "home" => self.scroll_terminal_to_scrollback(repo_id, usize::MAX),
            "end" => self.scroll_terminal_to_scrollback(repo_id, 0),
            _ => false,
        };

        if changed {
            cx.notify();
        }
        changed
    }

    fn handle_terminal_scroll_wheel(
        &mut self,
        repo_id: RepoId,
        event: &gpui::ScrollWheelEvent,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let metrics = self
            .terminal_layout_snapshot(self.theme, window, cx)
            .metrics;
        let Some((delta_y, step_rows)) = terminal_scroll_wheel_delta(event, metrics.line_height)
        else {
            return;
        };
        cx.stop_propagation();

        let Some((alternate_screen, application_cursor)) =
            self.terminal_sessions.get(&repo_id).map(|session| {
                (
                    session.parser.screen().alternate_screen(),
                    session.parser.screen().application_cursor(),
                )
            })
        else {
            return;
        };

        if alternate_screen {
            let bytes =
                terminal_alternate_screen_scroll_bytes(delta_y, step_rows, application_cursor);
            let _ = self.send_terminal_bytes_for_repo(repo_id, &bytes, cx);
            return;
        }

        let changed = if delta_y > px(0.0) {
            self.adjust_terminal_scrollback(repo_id, step_rows as isize)
        } else {
            self.adjust_terminal_scrollback(repo_id, -(step_rows as isize))
        };
        if changed {
            cx.notify();
        }
    }

    fn adjust_terminal_scrollback(&mut self, repo_id: RepoId, delta_rows: isize) -> bool {
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return false;
        };
        let current = session.parser.screen().scrollback();
        let candidate = if delta_rows >= 0 {
            current.saturating_add(delta_rows as usize)
        } else {
            current.saturating_sub(delta_rows.unsigned_abs())
        };
        session.parser.screen_mut().set_scrollback(candidate);
        let changed = session.parser.screen().scrollback() != current;
        if changed && terminal_clear_normal_selection(&mut session.selection) {
            session.selection_drag_anchor = None;
        }
        changed
    }

    fn scroll_terminal_to_scrollback(&mut self, repo_id: RepoId, scrollback: usize) -> bool {
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return false;
        };
        let current = session.parser.screen().scrollback();
        session.parser.screen_mut().set_scrollback(scrollback);
        let changed = session.parser.screen().scrollback() != current;
        if changed && terminal_clear_normal_selection(&mut session.selection) {
            session.selection_drag_anchor = None;
        }
        changed
    }

    fn send_terminal_bytes_for_repo(
        &mut self,
        repo_id: RepoId,
        bytes: &[u8],
        cx: &mut gpui::Context<Self>,
    ) -> Result<(), String> {
        let (result, state_changed) = {
            let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
                return Err("Repository terminal is no longer available.".to_string());
            };

            let reset_scrollback = session.parser.screen().scrollback() != 0;
            if reset_scrollback {
                session.parser.screen_mut().set_scrollback(0);
            }
            let selection_changed = terminal_clear_selection(&mut session.selection);
            if selection_changed {
                session.selection_drag_anchor = None;
            }
            if let Ok(mut state) = session.terminal.state.lock() {
                let _ = reset_terminal_scrollback(&mut state);
            }

            let result = match session.io.lock() {
                Ok(mut io) => match io.writer.as_mut() {
                    Some(writer) => writer
                        .write_all(bytes)
                        .and_then(|()| writer.flush())
                        .map_err(|err| err.to_string()),
                    None => match session
                        .terminal
                        .writer_tx
                        .try_send(TerminalWriteRequest::Bytes(bytes.to_vec()))
                    {
                        Ok(()) => Ok(()),
                        Err(smol::channel::TrySendError::Full(_)) => {
                            Err("Terminal input queue is full.".to_string())
                        }
                        Err(smol::channel::TrySendError::Closed(_)) => {
                            Err("Terminal input queue is closed.".to_string())
                        }
                    },
                },
                Err(_) => Err("Terminal state lock was poisoned.".to_string()),
            };

            if let Err(err) = &result {
                session.connected = false;
                if session.exit_status.is_none() {
                    session.exit_status = Some(format!("Failed to send input: {err}"));
                }
            }

            (result, reset_scrollback || selection_changed)
        };

        match result {
            Ok(()) => {
                self.reset_terminal_cursor_blink(cx);
                if state_changed {
                    cx.notify();
                }
                Ok(())
            }
            Err(err) => {
                cx.notify();
                Err(err)
            }
        }
    }

    fn open_external_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Result<(), String> {
        let context = resolve_external_terminal_launch_context(
            &self.state,
            &self.terminal_sessions,
            repo_id,
        )?;

        launch_external_terminal_from_preferences(&self.terminal_preferences, &context).map_err(
            |err| {
                self.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open external terminal: {err}"),
                    cx,
                );
                err
            },
        )
    }

    pub(in crate::view) fn terminal_session_exists(&self, repo_id: RepoId) -> bool {
        self.terminal_sessions.contains_key(&repo_id)
    }

    pub(in crate::view) fn terminal_is_connected(&self, repo_id: RepoId) -> bool {
        self.terminal_sessions
            .get(&repo_id)
            .is_some_and(|session| session.connected)
    }

    pub(in crate::view) fn terminal_has_copyable_selection(&self, repo_id: RepoId) -> bool {
        self.terminal_sessions
            .get(&repo_id)
            .and_then(|session| {
                session
                    .selection
                    .map(|selection| terminal_selection_text(session.parser.screen(), selection))
            })
            .is_some_and(|text| !text.is_empty())
    }

    pub(in crate::view) fn copy_terminal_selection_for_repo(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.focus_terminal_view(repo_id, window, cx);
        let Some(text) = self.terminal_sessions.get(&repo_id).and_then(|session| {
            session
                .selection
                .map(|selection| terminal_selection_text(session.parser.screen(), selection))
        }) else {
            return false;
        };
        if text.is_empty() {
            return false;
        }

        crate::clipboard::write_text(cx, text);
        true
    }

    pub(in crate::view) fn paste_terminal_clipboard_for_repo(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.focus_terminal_view(repo_id, window, cx);
        let Some(session) = self.terminal_sessions.get(&repo_id) else {
            return false;
        };
        if !session.connected {
            return false;
        }

        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        if text.is_empty() {
            return false;
        }

        let bracketed_paste = session.parser.screen().bracketed_paste();
        let bytes = terminal_paste_bytes(&text, bracketed_paste);
        self.send_terminal_bytes_for_repo(repo_id, &bytes, cx)
            .is_ok()
    }

    pub(in crate::view) fn select_all_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.focus_terminal_view(repo_id, window, cx);
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return;
        };
        session.selection = Some(TerminalSelection::AllBuffer);
        session.selection_drag_anchor = None;
        cx.notify();
    }

    pub(in crate::view) fn clear_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.focus_terminal_view(repo_id, window, cx);
        if !self.terminal_is_connected(repo_id) {
            return false;
        }
        self.send_terminal_bytes_for_repo(repo_id, b"\x0c", cx)
            .is_ok()
    }

    pub(in crate::view) fn open_external_terminal_from_menu(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Result<(), String> {
        self.focus_terminal_view(repo_id, window, cx);
        self.open_external_terminal_for_repo(repo_id, cx)
    }

    fn terminal_grid_point_at_position(
        &mut self,
        repo_id: RepoId,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<TerminalGridPoint> {
        let layout = self.terminal_layout_snapshot(self.theme, window, cx);
        let session = self.terminal_sessions.get(&repo_id)?;
        let bounds = session.viewport_bounds?;
        let (rows, cols) = session.parser.screen().size();
        Some(terminal_grid_point_for_position(
            bounds,
            layout.metrics,
            position,
            rows,
            cols,
        ))
    }

    fn handle_terminal_selection_mouse_down(
        &mut self,
        repo_id: RepoId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.focus_terminal_view(repo_id, window, cx);
        let Some(point) = self.terminal_grid_point_at_position(repo_id, position, window, cx)
        else {
            return;
        };
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return;
        };
        session.selection_drag_anchor = Some(point);
        session.selection = Some(TerminalSelection::visible(point, point));
        cx.notify();
    }

    fn handle_terminal_selection_mouse_move(
        &mut self,
        repo_id: RepoId,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self
            .terminal_sessions
            .get(&repo_id)
            .is_some_and(|session| session.selection_drag_anchor.is_some())
        {
            return;
        }
        let Some(point) = self.terminal_grid_point_at_position(repo_id, position, window, cx)
        else {
            return;
        };
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return;
        };
        let Some(anchor) = session.selection_drag_anchor else {
            return;
        };
        let selection = TerminalSelection::visible(anchor, point);
        if session.selection != Some(selection) {
            session.selection = Some(selection);
            cx.notify();
        }
    }

    fn handle_terminal_selection_mouse_up(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return;
        };
        let mut changed = session.selection_drag_anchor.take().is_some();
        if session.selection.is_some_and(TerminalSelection::is_empty) {
            session.selection = None;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn open_terminal_context_menu(
        &mut self,
        repo_id: RepoId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.focus_terminal_view(repo_id, window, cx);
        let context = TerminalMenuContext {
            has_session: self.terminal_session_exists(repo_id),
            has_selection: self.terminal_has_copyable_selection(repo_id),
            connected: self.terminal_is_connected(repo_id),
        };
        self.set_active_context_menu_invoker(
            Some(format!("terminal_menu_{}", repo_id.0).into()),
            cx,
        );
        self.open_popover_at(
            PopoverKind::TerminalMenu { repo_id, context },
            position,
            window,
            cx,
        );
    }

    pub(in crate::view) fn render_terminal_panel(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let repo_id = self.active_repo_id()?;
        let panel_height = terminal_panel_height_for_window(self.last_window_size.height).min(
            self.terminal_panel_height
                .max(px(TERMINAL_PANEL_MIN_HEIGHT_PX)),
        );
        let (session_seq, focus_handle, connected) = {
            let session = self.terminal_sessions.get(&repo_id)?;
            let focus_handle = session.focus_handle.clone();
            (session.session_seq, focus_handle.clone(), session.connected)
        };
        self.sync_terminal_cursor_blink_activity(repo_id, window, cx);
        let terminal_view = self.render_terminal_viewport(repo_id, session_seq, theme, window, cx);
        let action_icon = |path: &'static str| svg_icon(path, theme.colors.text_muted, px(12.0));

        let external_tooltip: SharedString = "Open in external terminal".into();
        let external_button = components::Button::new("terminal_open_external", "")
            .start_slot(action_icon("icons/open_external.svg"))
            .style(components::ButtonStyle::Transparent)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                let _ = this.open_external_terminal_for_repo(repo_id, cx);
            })
            .on_hover(cx.listener({
                let external_tooltip = external_tooltip.clone();
                move |this, hovering: &bool, _w, cx| {
                    if *hovering {
                        this.set_tooltip_text_if_changed(Some(external_tooltip.clone()), cx);
                    } else {
                        this.clear_tooltip_if_matches(&external_tooltip, cx);
                    }
                }
            }));
        let clear_tooltip: SharedString = "Clear terminal".into();
        let clear_button = components::Button::new("terminal_clear", "")
            .start_slot(action_icon("icons/broom.svg"))
            .style(components::ButtonStyle::Transparent)
            .disabled(!connected)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                let _ = this.clear_terminal_for_repo(repo_id, _w, cx);
            })
            .on_hover(cx.listener({
                let clear_tooltip = clear_tooltip.clone();
                move |this, hovering: &bool, _w, cx| {
                    if *hovering {
                        this.set_tooltip_text_if_changed(Some(clear_tooltip.clone()), cx);
                    } else {
                        this.clear_tooltip_if_matches(&clear_tooltip, cx);
                    }
                }
            }));
        let close_tooltip: SharedString = "Close terminal".into();
        let close_button = components::Button::new("terminal_close", "")
            .start_slot(action_icon("icons/generic_close.svg"))
            .style(components::ButtonStyle::Transparent)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                this.close_terminal_for_repo(repo_id, cx);
            })
            .on_hover(cx.listener({
                let close_tooltip = close_tooltip.clone();
                move |this, hovering: &bool, _w, cx| {
                    if *hovering {
                        this.set_tooltip_text_if_changed(Some(close_tooltip.clone()), cx);
                    } else {
                        this.clear_tooltip_if_matches(&close_tooltip, cx);
                    }
                }
            }));

        Some(
            div()
                .id("terminal_panel")
                .debug_selector(|| "terminal_panel".to_string())
                .h(panel_height)
                .min_h(px(TERMINAL_PANEL_MIN_HEIGHT_PX))
                .flex()
                .flex_col()
                .rounded_t(px(theme.radii.panel))
                .border_t_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.surface_bg_elevated)
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_1()
                        .border_b_1()
                        .border_color(theme.colors.border)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .font_family(crate::font_preferences::applied_ui_font_family(
                                    &crate::font_preferences::current(cx).ui_font_family,
                                ))
                                .child("Terminal"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(external_button)
                                .child(clear_button)
                                .child(close_button),
                        ),
                )
                .child(
                    div().flex_1().min_h(px(0.0)).child(
                        div()
                            .id("terminal_viewport")
                            .track_focus(&focus_handle)
                            .key_context("Terminal")
                            .w_full()
                            .h_full()
                            .bg(terminal_viewport_background(theme))
                            .overflow_hidden()
                            .on_action(cx.listener(move |this, _: &TerminalCopy, window, cx| {
                                cx.stop_propagation();
                                let _ = this.copy_terminal_selection_for_repo(repo_id, window, cx);
                            }))
                            .on_action(cx.listener(move |this, _: &TerminalPaste, window, cx| {
                                cx.stop_propagation();
                                let _ = this.paste_terminal_clipboard_for_repo(repo_id, window, cx);
                            }))
                            .on_action(cx.listener(
                                move |this, _: &TerminalSelectAll, window, cx| {
                                    cx.stop_propagation();
                                    this.select_all_terminal_for_repo(repo_id, window, cx);
                                },
                            ))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.handle_terminal_selection_mouse_down(
                                        repo_id, e.position, window, cx,
                                    );
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.open_terminal_context_menu(
                                        repo_id, e.position, window, cx,
                                    );
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, e: &MouseMoveEvent, window, cx| {
                                    this.handle_terminal_selection_mouse_move(
                                        repo_id, e.position, window, cx,
                                    );
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _e: &MouseUpEvent, _window, cx| {
                                    cx.stop_propagation();
                                    this.handle_terminal_selection_mouse_up(repo_id, cx);
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(move |this, _e: &MouseUpEvent, _window, cx| {
                                    cx.stop_propagation();
                                    this.handle_terminal_selection_mouse_up(repo_id, cx);
                                }),
                            )
                            .on_key_down(cx.listener(
                                move |this, e: &gpui::KeyDownEvent, window, cx| {
                                    this.handle_terminal_key_down(repo_id, e, window, cx);
                                },
                            ))
                            .on_scroll_wheel(cx.listener(
                                move |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                    this.handle_terminal_scroll_wheel(repo_id, e, window, cx);
                                },
                            ))
                            .child(terminal_view),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_terminal_viewport(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        theme: AppTheme,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity().clone();
        gpui::canvas(
            move |bounds, window, cx| {
                view.update(cx, |this, cx| {
                    let layout = this.terminal_layout_snapshot(theme, window, cx);
                    let next_size = terminal_grid_size(bounds, layout.metrics);
                    this.sync_terminal_grid_size(repo_id, session_seq, next_size);
                    this.build_terminal_canvas_paint_state(
                        repo_id,
                        session_seq,
                        theme,
                        bounds,
                        &layout,
                        window,
                    )
                })
            },
            move |_bounds, paint_state, window, cx| {
                paint_terminal_canvas_state(paint_state, theme, window, cx);
            },
        )
        .w_full()
        .h_full()
        .into_any_element()
    }

    fn build_terminal_canvas_paint_state(
        &mut self,
        repo_id: RepoId,
        session_seq: u64,
        theme: AppTheme,
        bounds: Bounds<Pixels>,
        layout: &TerminalLayoutCache,
        window: &mut Window,
    ) -> TerminalCanvasPaintState {
        let blink_visible = self.terminal_cursor_blink_visible;
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return TerminalCanvasPaintState::default();
        };
        if session.session_seq != session_seq {
            return TerminalCanvasPaintState::default();
        }

        session.viewport_bounds = Some(bounds);
        let selection = session.selection;
        let focus_is_focused = session.focus_handle.is_focused(window);
        let content_epoch = session.content_epoch;
        let parser = &session.parser;
        let render_cache = &mut session.render_cache;
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let viewport_key = TerminalViewportCacheKey {
            content_epoch,
            scrollback: screen.scrollback(),
            rows,
            cols,
            layout_key: layout.key,
        };

        if render_cache.viewport_key != Some(viewport_key) {
            render_cache
                .rows
                .resize_with(usize::from(rows), TerminalCachedRow::default);
            for row in 0..rows {
                let signature = terminal_row_signature(screen, row, cols);
                let cache_row = &mut render_cache.rows[usize::from(row)];
                if cache_row.fingerprint == signature.fingerprint
                    && cache_row.layout_key == layout.key
                {
                    continue;
                }

                cache_row.fingerprint = signature.fingerprint;
                cache_row.layout_key = layout.key;
                cache_row.shaped = if signature.paints {
                    let (text, runs) =
                        build_terminal_row(screen, row, cols, &layout.base_style, theme);
                    Some(window.text_system().shape_line(
                        text,
                        layout.metrics.font_size,
                        &runs,
                        None,
                    ))
                } else {
                    None
                };
                #[cfg(test)]
                {
                    render_cache.rebuilt_rows += 1;
                }
            }
            render_cache.viewport_key = Some(viewport_key);
        }

        let mut paint_state = TerminalCanvasPaintState {
            selection_rects: selection
                .map(|selection| {
                    terminal_selection_rects(selection, rows, cols, bounds, layout.metrics)
                })
                .unwrap_or_default(),
            lines: Vec::with_capacity(usize::from(rows)),
            cursor: None,
        };
        for (row, cache_row) in render_cache.rows.iter().enumerate() {
            let Some(shaped) = cache_row.shaped.clone() else {
                continue;
            };
            let origin = point(
                bounds.left(),
                bounds.top() + layout.metrics.line_height * row as f32,
            );
            paint_state
                .lines
                .push((shaped, origin, layout.metrics.line_height));
        }

        if blink_visible && !screen.hide_cursor() && screen.scrollback() == 0 && focus_is_focused {
            let (cursor_row, cursor_col) = screen.cursor_position();
            if cursor_row < rows && cols > 0 {
                let col = cursor_col.min(cols.saturating_sub(1));
                let width = match screen.cell(cursor_row, col) {
                    Some(cell) if cell.is_wide() => layout.metrics.cell_width * 2.0,
                    _ => layout.metrics.cell_width,
                };
                paint_state.cursor = Some(Bounds::new(
                    point(
                        bounds.left() + layout.metrics.cell_width * f32::from(col),
                        bounds.top() + layout.metrics.line_height * f32::from(cursor_row),
                    ),
                    size(width.max(px(1.0)), layout.metrics.line_height),
                ));
            }
        }

        paint_state
    }

    pub(in crate::view) fn terminal_panel_resize_handle(
        &self,
        theme: AppTheme,
        cx: &gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id("terminal_panel_resize_handle")
            .debug_selector(|| "terminal_panel_resize_handle".to_string())
            .w_full()
            .h(px(TERMINAL_PANEL_RESIZE_HANDLE_PX))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::ResizeUpDown)
            .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.65)))
            .active(move |s| s.bg(theme.colors.active))
            .child(div().h(px(1.0)).w_full().bg(theme.colors.border))
            .on_drag("terminal_panel_resize", |_drag, _offset, _window, cx| {
                cx.new(|_cx| PaneResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    this.terminal_panel_resize = Some(TerminalPanelResizeState {
                        start_y: e.position.y,
                        start_height: this.terminal_panel_height,
                    });
                    cx.notify();
                }),
            )
            .on_drag_move(
                cx.listener(|this, e: &gpui::DragMoveEvent<&'static str>, _w, cx| {
                    if *e.drag(cx) != "terminal_panel_resize" {
                        return;
                    }
                    let Some(state) = this.terminal_panel_resize else {
                        return;
                    };
                    let next_height = next_terminal_panel_height(
                        state,
                        e.event.position.y,
                        this.last_window_size.height,
                    );
                    if this.terminal_panel_height != next_height {
                        this.terminal_panel_height = next_height;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.terminal_panel_resize.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.terminal_panel_resize.take().is_some() {
                        cx.notify();
                    }
                }),
            )
    }
}

impl Drop for GitCometView {
    fn drop(&mut self) {
        for session in self.terminal_sessions.values_mut() {
            terminate_terminal_session(session);
        }
    }
}

fn terminal_panel_height_for_window(window_height: Pixels) -> Pixels {
    (window_height - px(260.0)).max(px(TERMINAL_PANEL_MIN_HEIGHT_PX))
}

fn spawn_terminal_session(
    preferences: &TerminalPreferences,
    workdir: &std::path::Path,
) -> Result<SpawnedTerminalSession, String> {
    let shell_program = resolve_embedded_shell_program(preferences)?;
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| err.to_string())?;

    let mut command = CommandBuilder::new(&shell_program);
    command.cwd(workdir);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "GitComet");
    command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|err| err.to_string())?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|err| err.to_string())?;
    let writer = pair.master.take_writer().map_err(|err| err.to_string())?;
    let killer = child.clone_killer();
    Ok(SpawnedTerminalSession {
        io: Arc::new(Mutex::new(TerminalIo {
            master: pair.master,
            writer: Some(writer),
            killer,
            size: PtySize {
                rows: TERMINAL_INITIAL_ROWS,
                cols: TERMINAL_INITIAL_COLS,
                pixel_width: 0,
                pixel_height: 0,
            },
        })),
        reader,
        child,
    })
}

fn terminate_terminal_session(session: &mut RepoTerminalSession) {
    let _ = session
        .terminal
        .writer_tx
        .try_send(TerminalWriteRequest::Shutdown);
    if let Ok(mut io) = session.io.lock() {
        let _ = io.killer.kill();
    }
}

fn read_next_terminal_chunk(
    mut reader: Box<dyn Read + Send>,
) -> (Box<dyn Read + Send>, std::io::Result<Option<Vec<u8>>>) {
    loop {
        let mut buffer = vec![0u8; TERMINAL_READ_CHUNK_BYTES];
        match reader.read(&mut buffer) {
            Ok(0) => return (reader, Ok(None)),
            Ok(len) => {
                buffer.truncate(len);
                return (reader, Ok(Some(buffer)));
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return (reader, Err(err)),
        }
    }
}

fn write_terminal_bytes(
    mut writer: Box<dyn Write + Send>,
    bytes: Vec<u8>,
) -> (Box<dyn Write + Send>, Result<(), String>) {
    let result = writer
        .write_all(&bytes)
        .and_then(|()| writer.flush())
        .map_err(|err| err.to_string());
    (writer, result)
}

fn initial_terminal_grid_size() -> TerminalGridSize {
    TerminalGridSize {
        rows: TERMINAL_INITIAL_ROWS,
        cols: TERMINAL_INITIAL_COLS,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn push_terminal_read_bytes(
    state: &mut TerminalReadBatchState,
    bytes: Vec<u8>,
) -> TerminalReadBatchAction {
    if bytes.is_empty() {
        return TerminalReadBatchAction::None;
    }

    state.bytes.extend_from_slice(&bytes);
    if state.bytes.len() >= TERMINAL_READ_BATCH_MAX_BYTES {
        state.flush_scheduled = false;
        return TerminalReadBatchAction::FlushNow;
    }

    if state.flush_scheduled {
        TerminalReadBatchAction::None
    } else {
        state.flush_scheduled = true;
        TerminalReadBatchAction::ScheduleFlush
    }
}

fn take_terminal_read_batch(
    batch_state: &Arc<Mutex<TerminalReadBatchState>>,
) -> Option<TerminalReadBatch> {
    let mut state = batch_state.lock().ok()?;
    state.flush_scheduled = false;
    if state.bytes.is_empty() && state.completion.is_none() {
        return None;
    }

    Some(TerminalReadBatch {
        bytes: std::mem::take(&mut state.bytes),
        completion: state.completion.take(),
    })
}

fn initial_terminal_session_state() -> TerminalSessionState {
    TerminalSessionState {
        parser: vt100::Parser::new(
            TERMINAL_INITIAL_ROWS,
            TERMINAL_INITIAL_COLS,
            TERMINAL_SCROLLBACK_ROWS,
        ),
        grid_size: initial_terminal_grid_size(),
        connected: true,
        exit_status: None,
        row_fingerprints: Vec::new(),
        dirty_rows: Vec::new(),
        selection: None,
        selection_drag_anchor: None,
    }
}

fn reset_terminal_scrollback(state: &mut TerminalSessionState) -> bool {
    let current = state.parser.screen().scrollback();
    state.parser.screen_mut().set_scrollback(0);
    let scrollback_changed = state.parser.screen().scrollback() != current;
    let selection_changed = terminal_clear_selection(&mut state.selection);
    if selection_changed {
        state.selection_drag_anchor = None;
    }
    scrollback_changed || selection_changed
}

fn terminal_clear_selection(selection: &mut Option<TerminalSelection>) -> bool {
    if selection.is_some() {
        *selection = None;
        true
    } else {
        false
    }
}

fn terminal_clear_normal_selection(selection: &mut Option<TerminalSelection>) -> bool {
    if matches!(selection, Some(TerminalSelection::Visible { .. })) {
        *selection = None;
        true
    } else {
        false
    }
}

fn terminal_selection_text(screen: &vt100::Screen, selection: TerminalSelection) -> String {
    match selection {
        TerminalSelection::Visible { start, end } => {
            terminal_visible_selection_text(screen, start, end)
        }
        TerminalSelection::AllBuffer => terminal_full_buffer_text(screen),
    }
}

fn terminal_visible_selection_text(
    screen: &vt100::Screen,
    start: TerminalGridPoint,
    end: TerminalGridPoint,
) -> String {
    let (rows, cols) = screen.size();
    if rows == 0 || cols == 0 {
        return String::new();
    }

    let Some((mut start, mut end)) = TerminalSelection::visible(start, end).normalized_visible()
    else {
        return String::new();
    };
    let max_row = rows.saturating_sub(1);
    start.row = start.row.min(max_row);
    end.row = end.row.min(max_row);
    start.col = start.col.min(cols);
    end.col = end.col.min(cols);
    if start == end {
        return String::new();
    }

    screen.contents_between(start.row, start.col, end.row, end.col)
}

fn terminal_full_buffer_text(screen: &vt100::Screen) -> String {
    let (rows, cols) = screen.size();
    if rows == 0 || cols == 0 {
        return String::new();
    }

    let visible_rows = usize::from(rows);
    let mut probe = screen.clone();
    probe.set_scrollback(usize::MAX);
    let max_scrollback = probe.scrollback();
    let total_rows = max_scrollback + visible_rows;
    let mut contents = String::new();
    let mut absolute_row = 0;

    // vt100 exposes text through the visible window, so walk the buffer in
    // screen-height chunks while reusing one probe screen.
    while absolute_row < max_scrollback {
        probe.set_scrollback(max_scrollback - absolute_row);
        let row_count = visible_rows.min(total_rows - absolute_row);
        terminal_append_visible_rows_text(&probe, 0, row_count, cols, &mut contents);
        absolute_row += row_count;
    }

    if absolute_row < total_rows {
        probe.set_scrollback(0);
        let start_row = absolute_row.saturating_sub(max_scrollback);
        terminal_append_visible_rows_text(
            &probe,
            start_row,
            total_rows - absolute_row,
            cols,
            &mut contents,
        );
    }

    while contents.ends_with('\n') {
        contents.pop();
    }
    contents
}

fn terminal_append_visible_rows_text(
    screen: &vt100::Screen,
    start_row: usize,
    row_count: usize,
    cols: u16,
    contents: &mut String,
) {
    for (row, text) in screen
        .rows(0, cols)
        .enumerate()
        .skip(start_row)
        .take(row_count)
    {
        contents.push_str(&text);
        if !screen.row_wrapped(row as u16) {
            contents.push('\n');
        }
    }
}

fn terminal_paste_bytes(text: &str, bracketed_paste: bool) -> Vec<u8> {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            normalized.push('\n');
        } else {
            normalized.push(ch);
        }
    }

    if !bracketed_paste {
        return normalized.into_bytes();
    }

    let sanitized = terminal_sanitize_bracketed_paste_payload(&normalized);
    let mut bytes = Vec::with_capacity(
        BRACKETED_PASTE_START.len() + sanitized.len() + BRACKETED_PASTE_END.len(),
    );
    bytes.extend_from_slice(BRACKETED_PASTE_START);
    bytes.extend_from_slice(sanitized.as_bytes());
    bytes.extend_from_slice(BRACKETED_PASTE_END);
    bytes
}

fn terminal_sanitize_bracketed_paste_payload(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect()
}

fn terminal_grid_point_for_position(
    bounds: Bounds<Pixels>,
    metrics: TerminalTextMetrics,
    position: Point<Pixels>,
    rows: u16,
    cols: u16,
) -> TerminalGridPoint {
    let row_count = rows.max(1);
    let rel_x = if position.x <= bounds.left() {
        px(0.0)
    } else {
        position.x - bounds.left()
    };
    let rel_y = if position.y <= bounds.top() {
        px(0.0)
    } else {
        position.y - bounds.top()
    };
    let col = ((rel_x / metrics.cell_width).floor() as u16).min(cols);
    let row = ((rel_y / metrics.line_height).floor() as u16).min(row_count.saturating_sub(1));
    TerminalGridPoint::new(row, col)
}

fn terminal_selection_rects(
    selection: TerminalSelection,
    rows: u16,
    cols: u16,
    bounds: Bounds<Pixels>,
    metrics: TerminalTextMetrics,
) -> Vec<Bounds<Pixels>> {
    if rows == 0 || cols == 0 {
        return Vec::new();
    }

    match selection {
        TerminalSelection::AllBuffer => (0..rows)
            .map(|row| {
                Bounds::new(
                    point(
                        bounds.left(),
                        bounds.top() + metrics.line_height * f32::from(row),
                    ),
                    size(metrics.cell_width * f32::from(cols), metrics.line_height),
                )
            })
            .collect(),
        TerminalSelection::Visible { start, end } => {
            let Some((mut start, mut end)) =
                TerminalSelection::visible(start, end).normalized_visible()
            else {
                return Vec::new();
            };
            let max_row = rows.saturating_sub(1);
            start.row = start.row.min(max_row);
            end.row = end.row.min(max_row);
            start.col = start.col.min(cols);
            end.col = end.col.min(cols);
            if start == end {
                return Vec::new();
            }

            let mut rects = Vec::new();
            for row in start.row..=end.row {
                let start_col = if row == start.row { start.col } else { 0 };
                let end_col = if row == end.row { end.col } else { cols };
                if end_col <= start_col {
                    continue;
                }
                rects.push(Bounds::new(
                    point(
                        bounds.left() + metrics.cell_width * f32::from(start_col),
                        bounds.top() + metrics.line_height * f32::from(row),
                    ),
                    size(
                        metrics.cell_width * f32::from(end_col - start_col),
                        metrics.line_height,
                    ),
                ));
            }
            rects
        }
    }
}

fn terminal_scroll_wheel_delta(
    event: &gpui::ScrollWheelEvent,
    line_height: Pixels,
) -> Option<(Pixels, usize)> {
    let pixel_delta = event.delta.pixel_delta(line_height);
    let delta_y = if !pixel_delta.y.is_zero() {
        pixel_delta.y
    } else {
        pixel_delta.x
    };
    if delta_y.is_zero() {
        return None;
    }

    let step_rows = (((delta_y.abs()) / line_height).ceil() as usize).max(1);
    Some((delta_y, step_rows))
}

fn terminal_alternate_screen_scroll_bytes(
    delta_y: Pixels,
    step_rows: usize,
    application_cursor: bool,
) -> Vec<u8> {
    let sequence = if delta_y > px(0.0) {
        if application_cursor {
            b"\x1bOA".as_slice()
        } else {
            b"\x1b[A".as_slice()
        }
    } else if application_cursor {
        b"\x1bOB".as_slice()
    } else {
        b"\x1b[B".as_slice()
    };
    let repeats = step_rows
        .max(1)
        .min(TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS);
    let mut bytes = Vec::with_capacity(sequence.len() * repeats);
    for _ in 0..repeats {
        bytes.extend_from_slice(sequence);
    }
    bytes
}

fn terminal_clipboard_shortcut_action(
    keystroke: &gpui::Keystroke,
) -> Option<TerminalShortcutAction> {
    let action = match keystroke.key.as_str() {
        "c" => TerminalShortcutAction::Copy,
        "v" => TerminalShortcutAction::Paste,
        "a" => TerminalShortcutAction::SelectAll,
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

fn mark_terminal_disconnected(
    state: &Arc<Mutex<TerminalSessionState>>,
    exit_status: Option<String>,
) {
    if let Ok(mut state) = state.lock() {
        state.connected = false;
        if let Some(exit_status) = exit_status {
            state.exit_status = Some(exit_status);
        }
    }
}

fn adjust_terminal_scrollback(state: &Arc<Mutex<TerminalSessionState>>, delta_rows: isize) -> bool {
    let Ok(mut state) = state.lock() else {
        return false;
    };
    let current = state.parser.screen().scrollback();
    let candidate = if delta_rows >= 0 {
        current.saturating_add(delta_rows as usize)
    } else {
        current.saturating_sub(delta_rows.unsigned_abs())
    };
    state.parser.screen_mut().set_scrollback(candidate);
    let changed = state.parser.screen().scrollback() != current;
    if changed && terminal_clear_normal_selection(&mut state.selection) {
        state.selection_drag_anchor = None;
    }
    changed
}

fn scroll_terminal_to_scrollback(
    state: &Arc<Mutex<TerminalSessionState>>,
    scrollback: usize,
) -> bool {
    let Ok(mut state) = state.lock() else {
        return false;
    };
    let current = state.parser.screen().scrollback();
    state.parser.screen_mut().set_scrollback(scrollback);
    let changed = state.parser.screen().scrollback() != current;
    if changed && terminal_clear_normal_selection(&mut state.selection) {
        state.selection_drag_anchor = None;
    }
    changed
}

fn sync_terminal_grid_size_state(
    state: &Arc<Mutex<TerminalSessionState>>,
    io: &Arc<Mutex<TerminalIo>>,
    next_size: TerminalGridSize,
) {
    if let Ok(mut state) = state.lock() {
        if state.grid_size == next_size {
            return;
        }
        if state.parser.screen().size() != (next_size.rows, next_size.cols) {
            state
                .parser
                .screen_mut()
                .set_size(next_size.rows, next_size.cols);
            state.row_fingerprints.clear();
            state.dirty_rows = (0..next_size.rows).collect();
            if terminal_clear_normal_selection(&mut state.selection) {
                state.selection_drag_anchor = None;
            }
        }
        state.grid_size = next_size;
    } else {
        return;
    }

    if let Ok(mut io) = io.lock() {
        let pty_size = next_size.into_pty_size();
        if io.size != pty_size && io.master.resize(pty_size).is_ok() {
            io.size = pty_size;
        }
    }
}

fn collect_terminal_render_snapshot(
    state: &Arc<Mutex<TerminalSessionState>>,
    _previous_viewport_key: Option<TerminalViewportCacheKey>,
    base_style: &gpui::TextStyle,
    theme: AppTheme,
    layout_key: TerminalLayoutKey,
    metrics: TerminalTextMetrics,
    bounds: Bounds<Pixels>,
    cursor_blink_visible: bool,
    focus_is_focused: bool,
) -> (
    u16,
    u16,
    TerminalViewportCacheKey,
    Vec<TerminalDirtyRowRenderInput>,
    Option<Bounds<Pixels>>,
) {
    let Ok(state) = state.lock() else {
        return (
            0,
            0,
            TerminalViewportCacheKey {
                content_epoch: 0,
                scrollback: 0,
                rows: 0,
                cols: 0,
                layout_key,
            },
            Vec::new(),
            None,
        );
    };

    let screen = state.parser.screen();
    let (rows, cols) = screen.size();
    let viewport_key = TerminalViewportCacheKey {
        content_epoch: 0,
        scrollback: screen.scrollback(),
        rows,
        cols,
        layout_key,
    };
    let mut dirty_rows = Vec::with_capacity(usize::from(rows));
    for row in 0..rows {
        let signature = terminal_row_signature(screen, row, cols);
        if signature.paints {
            let (text, runs) = build_terminal_row(screen, row, cols, base_style, theme);
            dirty_rows.push(TerminalDirtyRowRenderInput {
                row,
                fingerprint: signature.fingerprint,
                text: Some(text),
                runs,
            });
        } else {
            dirty_rows.push(TerminalDirtyRowRenderInput {
                row,
                fingerprint: signature.fingerprint,
                text: None,
                runs: Vec::new(),
            });
        }
    }

    let cursor = if cursor_blink_visible
        && state.connected
        && !screen.hide_cursor()
        && screen.scrollback() == 0
        && focus_is_focused
    {
        let (cursor_row, cursor_col) = screen.cursor_position();
        if cursor_row < rows && cols > 0 {
            let col = cursor_col.min(cols.saturating_sub(1));
            let width = match screen.cell(cursor_row, col) {
                Some(cell) if cell.is_wide() => metrics.cell_width * 2.0,
                _ => metrics.cell_width,
            };
            Some(Bounds::new(
                point(
                    bounds.left() + metrics.cell_width * f32::from(col),
                    bounds.top() + metrics.line_height * f32::from(cursor_row),
                ),
                size(width.max(px(1.0)), metrics.line_height),
            ))
        } else {
            None
        }
    } else {
        None
    };

    (rows, cols, viewport_key, dirty_rows, cursor)
}

fn terminal_layout_key(metrics: TerminalTextMetrics) -> TerminalLayoutKey {
    TerminalLayoutKey {
        font_size_bits: pixels_bits(metrics.font_size),
        line_height_bits: pixels_bits(metrics.line_height),
        cell_width_bits: pixels_bits(metrics.cell_width),
    }
}

fn terminal_layout_cache(mut base_style: gpui::TextStyle, window: &Window) -> TerminalLayoutCache {
    let rem_size = window.rem_size();
    let font_size = base_style.font_size.to_pixels(rem_size) * TERMINAL_FONT_SCALE;
    let line_height = terminal_line_height(font_size);
    base_style.line_height = line_height.into();
    let sample = window.text_system().shape_line(
        TERMINAL_CELL_WIDTH_SAMPLE.into(),
        font_size,
        &[base_style.to_run(TERMINAL_CELL_WIDTH_SAMPLE.len())],
        None,
    );
    let cell_width = if TERMINAL_CELL_WIDTH_SAMPLE.is_empty() {
        px(8.0)
    } else {
        (sample.width / TERMINAL_CELL_WIDTH_SAMPLE.len() as f32).max(px(1.0))
    };
    let metrics = TerminalTextMetrics {
        font_size,
        line_height,
        cell_width,
    };
    TerminalLayoutCache {
        rem_size,
        key: terminal_layout_key(metrics),
        base_style,
        metrics,
    }
}

fn terminal_line_height(font_size: Pixels) -> Pixels {
    let font_size_px: f32 = font_size.into();
    px((font_size_px * TERMINAL_LINE_HEIGHT_SCALE).ceil().max(1.0))
}

fn pixels_bits(value: Pixels) -> u32 {
    let raw: f32 = value.into();
    raw.to_bits()
}

fn terminal_row_signature(screen: &vt100::Screen, row: u16, cols: u16) -> TerminalRowSignature {
    let default_fg = terminal_default_foreground();
    let default_bg = terminal_default_background();
    let mut hasher = FxHasher::default();
    let mut paints = false;

    row.hash(&mut hasher);
    cols.hash(&mut hasher);
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }

        let contents = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        let style = terminal_cell_style(cell, default_fg, default_bg);
        contents.hash(&mut hasher);
        hash_terminal_cell_style(&style, &mut hasher);
        paints |= contents != " " || style.bg.is_some();
    }

    TerminalRowSignature {
        fingerprint: hasher.finish(),
        paints,
    }
}

fn hash_terminal_cell_style(style: &TerminalCellStyle, hasher: &mut FxHasher) {
    hash_rgba(style.fg, hasher);
    style.bg.is_some().hash(hasher);
    if let Some(bg) = style.bg {
        hash_rgba(bg, hasher);
    }
    style.bold.hash(hasher);
    style.italic.hash(hasher);
    style.underline.hash(hasher);
}

fn hash_rgba(color: gpui::Rgba, hasher: &mut FxHasher) {
    color.r.to_bits().hash(hasher);
    color.g.to_bits().hash(hasher);
    color.b.to_bits().hash(hasher);
    color.a.to_bits().hash(hasher);
}

fn paint_terminal_canvas_state(
    paint_state: TerminalCanvasPaintState,
    theme: AppTheme,
    window: &mut Window,
    cx: &mut App,
) {
    for rect in paint_state.selection_rects {
        window.paint_quad(fill(rect, terminal_selection_color(theme)));
    }

    for (line, origin, line_height) in paint_state.lines {
        let _ = line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
    }

    if let Some(cursor) = paint_state.cursor {
        window.paint_quad(
            fill(terminal_caret_bounds(cursor), terminal_caret_color(theme))
                .corner_radii(px(TERMINAL_CARET_RADIUS_PX)),
        );
    }
}

fn terminal_caret_bounds(cell_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = (cell_bounds.size.width * TERMINAL_CARET_WIDTH_RATIO)
        .max(px(TERMINAL_CARET_MIN_WIDTH_PX))
        .min(px(TERMINAL_CARET_MAX_WIDTH_PX))
        .min(cell_bounds.size.width.max(px(1.0)));
    let inset_y = (cell_bounds.size.height * 0.08)
        .max(px(TERMINAL_CARET_VERTICAL_INSET_PX))
        .min((cell_bounds.size.height / 2.0).max(px(0.0)));
    let height = (cell_bounds.size.height - inset_y * 2.0).max(px(1.0));

    Bounds::new(
        point(cell_bounds.left(), cell_bounds.top() + inset_y),
        size(width, height),
    )
}

fn terminal_caret_color(theme: AppTheme) -> gpui::Rgba {
    let _ = theme;
    terminal_default_foreground()
}

fn terminal_selection_color(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.accent, TERMINAL_SELECTION_ALPHA)
}

fn terminal_default_background() -> gpui::Rgba {
    rgba_from_hex(TERMINAL_DEFAULT_BG_HEX)
}

fn terminal_default_foreground() -> gpui::Rgba {
    rgba_from_hex(TERMINAL_DEFAULT_FG_HEX)
}

fn build_terminal_row(
    screen: &vt100::Screen,
    row: u16,
    cols: u16,
    base_style: &gpui::TextStyle,
    theme: AppTheme,
) -> (SharedString, Vec<TextRun>) {
    let terminal_bg = terminal_viewport_background(theme);
    let terminal_fg = terminal_default_foreground();
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut active_style: Option<TerminalCellStyle> = None;
    let mut active_len = 0usize;

    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }

        let contents = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        let style = terminal_cell_style(cell, terminal_fg, terminal_bg);

        if active_style
            .as_ref()
            .is_some_and(|current| current == &style)
        {
            active_len += contents.len();
        } else {
            if let Some(previous) = active_style.take() {
                runs.push(terminal_text_run(base_style, &previous, active_len));
            }
            active_style = Some(style);
            active_len = contents.len();
        }

        text.push_str(contents);
    }

    if let Some(previous) = active_style {
        runs.push(terminal_text_run(base_style, &previous, active_len));
    }

    (text.into(), runs)
}

fn terminal_text_run(
    base_style: &gpui::TextStyle,
    cell_style: &TerminalCellStyle,
    len: usize,
) -> TextRun {
    let mut style = base_style.clone();
    style.color = cell_style.fg.into();
    style.background_color = cell_style.bg.map(Into::into);
    style.font_weight = if cell_style.bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    };
    style.font_style = if cell_style.italic {
        gpui::FontStyle::Italic
    } else {
        gpui::FontStyle::Normal
    };
    style.underline = cell_style.underline.then_some(gpui::UnderlineStyle {
        thickness: px(1.0),
        color: Some(cell_style.fg.into()),
        wavy: false,
    });
    style.to_run(len)
}

fn terminal_text_style<C>(_theme: AppTheme, window: &Window, cx: &mut C) -> gpui::TextStyle
where
    C: gpui::BorrowAppContext,
{
    let mut style = window.text_style();
    style.font_family = crate::font_preferences::current_editor_font_family(cx).into();
    style.font_features = crate::font_preferences::current_font_features(cx);
    style.font_weight = FontWeight::NORMAL;
    style.font_style = gpui::FontStyle::Normal;
    style.color = terminal_default_foreground().into();
    style.white_space = gpui::WhiteSpace::Nowrap;
    style.text_overflow = None;
    style
}

fn terminal_grid_size(bounds: Bounds<Pixels>, metrics: TerminalTextMetrics) -> TerminalGridSize {
    let cols =
        ((bounds.size.width / metrics.cell_width).floor() as u16).max(TERMINAL_MIN_GRID_COLS);
    let rows =
        ((bounds.size.height / metrics.line_height).floor() as u16).max(TERMINAL_MIN_GRID_ROWS);
    TerminalGridSize {
        rows,
        cols,
        pixel_width: pixels_to_u16(metrics.cell_width),
        pixel_height: pixels_to_u16(metrics.line_height),
    }
}

fn terminal_cell_style(
    cell: &vt100::Cell,
    default_fg: gpui::Rgba,
    default_bg: gpui::Rgba,
) -> TerminalCellStyle {
    let mut fg = vt100_color_to_rgba(cell.fgcolor(), default_fg);
    let mut bg = vt100_color_to_rgba(cell.bgcolor(), default_bg);
    if cell.inverse() {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.dim() {
        fg = mix_rgba(fg, bg, 0.35);
    }

    TerminalCellStyle {
        fg,
        bg: (bg != default_bg).then_some(bg),
        bold: cell.bold(),
        italic: cell.italic(),
        underline: cell.underline(),
    }
}

fn vt100_color_to_rgba(color: vt100::Color, default: gpui::Rgba) -> gpui::Rgba {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => rgba_from_rgb(r, g, b),
        vt100::Color::Idx(index) => terminal_palette_color(index),
    }
}

fn terminal_palette_color(index: u8) -> gpui::Rgba {
    const ANSI: [u32; 16] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
        0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];

    match index {
        0..=15 => rgba_from_hex(ANSI[usize::from(index)]),
        16..=231 => {
            let i = index - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            rgba_from_rgb(
                color_cube_level(r),
                color_cube_level(g),
                color_cube_level(b),
            )
        }
        232..=255 => {
            let value = 8u8.saturating_add((index - 232) * 10);
            rgba_from_rgb(value, value, value)
        }
    }
}

fn color_cube_level(level: u8) -> u8 {
    if level == 0 { 0 } else { 55 + level * 40 }
}

fn terminal_viewport_background(_theme: AppTheme) -> gpui::Rgba {
    terminal_default_background()
}

fn encode_terminal_key_input(
    keystroke: &gpui::Keystroke,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let key = keystroke.key.as_str();
    let mods = keystroke.modifiers;

    if mods.platform || mods.function {
        return None;
    }

    if mods.shift && !mods.control && !mods.alt {
        match key {
            "tab" => return Some(b"\x1b[Z".to_vec()),
            "up" => return Some(b"\x1b[1;2A".to_vec()),
            "down" => return Some(b"\x1b[1;2B".to_vec()),
            "right" => return Some(b"\x1b[1;2C".to_vec()),
            "left" => return Some(b"\x1b[1;2D".to_vec()),
            _ => {}
        }
    }

    if mods.control
        && !mods.alt
        && let Some(control) = encode_control_key(key)
    {
        return Some(vec![control]);
    }

    match key {
        "enter" => return Some(vec![b'\r']),
        "tab" => return Some(vec![b'\t']),
        "backspace" => return Some(vec![0x7f]),
        "escape" => return Some(vec![0x1b]),
        "insert" => return Some(b"\x1b[2~".to_vec()),
        "delete" => return Some(b"\x1b[3~".to_vec()),
        "home" => {
            return Some(if application_cursor {
                b"\x1bOH".to_vec()
            } else {
                b"\x1b[H".to_vec()
            });
        }
        "end" => {
            return Some(if application_cursor {
                b"\x1bOF".to_vec()
            } else {
                b"\x1b[F".to_vec()
            });
        }
        "pageup" => return Some(b"\x1b[5~".to_vec()),
        "pagedown" => return Some(b"\x1b[6~".to_vec()),
        "up" => {
            return Some(if application_cursor {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            });
        }
        "down" => {
            return Some(if application_cursor {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            });
        }
        "right" => {
            return Some(if application_cursor {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            });
        }
        "left" => {
            return Some(if application_cursor {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            });
        }
        _ => {}
    }

    let key_char = keystroke.key_char.as_ref()?;
    if mods.control {
        return None;
    }

    let mut bytes = Vec::new();
    if mods.alt {
        bytes.push(0x1b);
    }
    bytes.extend_from_slice(key_char.as_bytes());
    Some(bytes)
}

fn encode_control_key(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return match key {
            "space" => Some(0),
            "[" => Some(27),
            "\\" => Some(28),
            "]" => Some(29),
            "^" | "6" => Some(30),
            "_" | "-" => Some(31),
            _ => None,
        };
    }

    let lower = first.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        Some((lower as u8 - b'a') + 1)
    } else {
        match lower {
            '2' | '@' => Some(0),
            '3' => Some(27),
            '4' => Some(28),
            '5' => Some(29),
            '6' => Some(30),
            '7' | '/' => Some(31),
            '8' | '?' => Some(127),
            _ => None,
        }
    }
}

fn rgba_from_hex(rgb: u32) -> gpui::Rgba {
    rgba_from_rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

fn rgba_from_rgb(r: u8, g: u8, b: u8) -> gpui::Rgba {
    gpui::Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
}

fn mix_rgba(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    gpui::Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

fn pixels_to_u16(value: Pixels) -> u16 {
    let raw: f32 = value.into();
    raw.max(0.0).min(u16::MAX as f32).round() as u16
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}

impl TerminalGridSize {
    fn into_pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: self.pixel_width,
            pixel_height: self.pixel_height,
        }
    }
}

#[cfg(test)]
impl GitCometView {
    fn disable_poller_for_tests(&mut self) {
        // Test views now start under the deterministic UI runtime, which already
        // disables the live store poller and requires explicit snapshot pushes.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{lock_clipboard_test, lock_visual_test};
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::error::{Error, ErrorKind};
    use gitcomet_core::services::{GitBackend, GitRepository, Result as GitResult};
    use gitcomet_state::store::AppStore;
    use gpui::{App, Bounds, Keystroke, Modifiers, WindowAppearance, point, px, size};
    use portable_pty::{Child, ChildKiller, ExitStatus, MasterPty, PtySize};
    use std::collections::VecDeque;
    use std::io::{self, Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct TestBackend;

    impl GitBackend for TestBackend {
        fn open(&self, _workdir: &Path) -> GitResult<Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::Unsupported(
                "Test backend does not open repositories",
            )))
        }
    }

    #[derive(Debug, Default)]
    struct FakeMasterState {
        size: Mutex<PtySize>,
        resize_calls: Mutex<Vec<PtySize>>,
        resize_error: Mutex<Option<io::ErrorKind>>,
    }

    #[derive(Clone)]
    struct FakeMasterPty {
        state: Arc<FakeMasterState>,
    }

    impl FakeMasterPty {
        fn new(state: Arc<FakeMasterState>) -> Self {
            Self { state }
        }
    }

    impl MasterPty for FakeMasterPty {
        fn resize(&self, size: PtySize) -> std::result::Result<(), anyhow::Error> {
            self.state.resize_calls.lock().unwrap().push(size);
            if let Some(kind) = *self.state.resize_error.lock().unwrap() {
                return Err(io::Error::new(kind, "resize failed").into());
            }
            *self.state.size.lock().unwrap() = size;
            Ok(())
        }

        fn get_size(&self) -> std::result::Result<PtySize, anyhow::Error> {
            Ok(*self.state.size.lock().unwrap())
        }

        fn try_clone_reader(&self) -> std::result::Result<Box<dyn Read + Send>, anyhow::Error> {
            Ok(Box::new(io::Cursor::new(Vec::<u8>::new())))
        }

        fn take_writer(&self) -> std::result::Result<Box<dyn Write + Send>, anyhow::Error> {
            Ok(Box::new(FakeWriter::new(Arc::new(
                FakeWriterState::default(),
            ))))
        }

        #[cfg(unix)]
        fn process_group_leader(&self) -> Option<std::ffi::c_int> {
            None
        }

        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<portable_pty::unix::RawFd> {
            None
        }

        #[cfg(unix)]
        fn tty_name(&self) -> Option<PathBuf> {
            None
        }
    }

    #[derive(Debug, Default)]
    struct FakeWriterState {
        bytes: Mutex<Vec<u8>>,
        write_error: Mutex<Option<io::ErrorKind>>,
        flush_error: Mutex<Option<io::ErrorKind>>,
        flush_count: AtomicUsize,
    }

    #[derive(Clone, Debug)]
    struct FakeWriter {
        state: Arc<FakeWriterState>,
    }

    impl FakeWriter {
        fn new(state: Arc<FakeWriterState>) -> Self {
            Self { state }
        }
    }

    impl Write for FakeWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Some(kind) = *self.state.write_error.lock().unwrap() {
                return Err(io::Error::new(kind, "write failed"));
            }
            self.state.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.state.flush_count.fetch_add(1, Ordering::SeqCst);
            if let Some(kind) = *self.state.flush_error.lock().unwrap() {
                return Err(io::Error::new(kind, "flush failed"));
            }
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeKillerState {
        kill_count: AtomicUsize,
    }

    #[derive(Clone, Debug)]
    struct FakeKiller {
        state: Arc<FakeKillerState>,
    }

    impl FakeKiller {
        fn new(state: Arc<FakeKillerState>) -> Self {
            Self { state }
        }
    }

    impl ChildKiller for FakeKiller {
        fn kill(&mut self) -> io::Result<()> {
            self.state.kill_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(self.clone())
        }
    }

    #[derive(Clone, Debug)]
    enum FakeWaitResult {
        Exit(ExitStatus),
    }

    #[derive(Debug)]
    struct FakeChild {
        wait_result: Arc<Mutex<Option<FakeWaitResult>>>,
        killer: FakeKiller,
    }

    impl FakeChild {
        fn new(wait_result: FakeWaitResult, killer_state: Arc<FakeKillerState>) -> Self {
            Self {
                wait_result: Arc::new(Mutex::new(Some(wait_result))),
                killer: FakeKiller::new(killer_state),
            }
        }
    }

    impl ChildKiller for FakeChild {
        fn kill(&mut self) -> io::Result<()> {
            self.killer.kill()
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            self.killer.clone_killer()
        }
    }

    impl Child for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            match self.wait_result.lock().unwrap().as_ref() {
                Some(FakeWaitResult::Exit(status)) => Ok(Some(status.clone())),
                None => Ok(Some(ExitStatus::with_exit_code(0))),
            }
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            match self.wait_result.lock().unwrap().take() {
                Some(FakeWaitResult::Exit(status)) => Ok(status),
                None => Ok(ExitStatus::with_exit_code(0)),
            }
        }

        fn process_id(&self) -> Option<u32> {
            Some(42)
        }
    }

    #[derive(Debug)]
    enum ReadStep {
        Bytes(Vec<u8>),
        Eof,
        Error(io::ErrorKind, &'static str),
    }

    #[derive(Debug)]
    struct ScriptedReader {
        steps: VecDeque<ReadStep>,
    }

    impl ScriptedReader {
        fn new(steps: impl IntoIterator<Item = ReadStep>) -> Self {
            Self {
                steps: steps.into_iter().collect(),
            }
        }
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front().unwrap_or(ReadStep::Eof) {
                ReadStep::Bytes(bytes) => {
                    let len = bytes.len().min(buf.len());
                    buf[..len].copy_from_slice(&bytes[..len]);
                    Ok(len)
                }
                ReadStep::Eof => Ok(0),
                ReadStep::Error(kind, message) => Err(io::Error::new(kind, message)),
            }
        }
    }

    fn test_theme() -> AppTheme {
        AppTheme::default_for_window_appearance(WindowAppearance::Dark)
    }

    fn test_repo_state(repo_id: RepoId, workdir: impl Into<PathBuf>) -> RepoState {
        RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: workdir.into(),
            },
        )
    }

    fn app_state_with_repos(repos: Vec<RepoState>, active_repo: Option<RepoId>) -> Arc<AppState> {
        Arc::new(AppState {
            repos,
            active_repo,
            ..Default::default()
        })
    }

    fn push_test_state(this: &GitCometView, state: Arc<AppState>, cx: &mut impl gpui::AppContext) {
        this._ui_model
            .update(cx, |model, cx| model.set_state(state, cx));
    }

    fn fake_session_io(
        initial_size: PtySize,
    ) -> (
        Arc<Mutex<TerminalIo>>,
        Arc<FakeMasterState>,
        Arc<FakeWriterState>,
        Arc<FakeKillerState>,
    ) {
        let master_state = Arc::new(FakeMasterState {
            size: Mutex::new(initial_size),
            ..Default::default()
        });
        let writer_state = Arc::new(FakeWriterState::default());
        let killer_state = Arc::new(FakeKillerState::default());
        let io = Arc::new(Mutex::new(TerminalIo {
            master: Box::new(FakeMasterPty::new(master_state.clone())),
            writer: Some(Box::new(FakeWriter::new(writer_state.clone()))),
            killer: Box::new(FakeKiller::new(killer_state.clone())),
            size: initial_size,
        }));
        (io, master_state, writer_state, killer_state)
    }

    fn make_test_terminal_session(
        workdir: impl Into<PathBuf>,
        focus_handle: gpui::FocusHandle,
        io: Arc<Mutex<TerminalIo>>,
        session_seq: u64,
        cx: &mut gpui::Context<GitCometView>,
    ) -> RepoTerminalSession {
        let workdir = workdir.into();
        let (writer_tx, _writer_rx) =
            smol::channel::bounded::<TerminalWriteRequest>(TERMINAL_WRITE_QUEUE_CAPACITY);
        let terminal = TerminalSessionHandle {
            state: Arc::new(Mutex::new(initial_terminal_session_state())),
            writer_tx,
        };
        let viewport = cx.new(|_cx| {
            TerminalViewportView::new(
                test_theme(),
                focus_handle.clone(),
                terminal.clone(),
                io.clone(),
            )
        });
        RepoTerminalSession {
            repo_name: terminal_repo_name(&workdir),
            workdir,
            focus_handle,
            io,
            parser: vt100::Parser::new(
                TERMINAL_INITIAL_ROWS,
                TERMINAL_INITIAL_COLS,
                TERMINAL_SCROLLBACK_ROWS,
            ),
            grid_size: initial_terminal_grid_size(),
            content_epoch: 0,
            render_cache: TerminalRenderCache::default(),
            session_seq,
            connected: true,
            exit_status: None,
            terminal,
            viewport,
            selection: None,
            selection_drag_anchor: None,
            viewport_bounds: None,
        }
    }

    fn indicator_repo_ids(
        view: &Entity<GitCometView>,
        app: &App,
    ) -> (HashSet<RepoId>, HashSet<RepoId>) {
        let (repo_tabs_bar, action_bar) = {
            let root = view.read(app);
            (root.repo_tabs_bar.clone(), root.action_bar.clone())
        };
        (
            repo_tabs_bar
                .read(app)
                .open_terminal_repo_ids_for_test()
                .clone(),
            action_bar
                .read(app)
                .open_terminal_repo_ids_for_test()
                .clone(),
        )
    }

    fn wait_for_view_condition<T, Ready, Snapshot>(
        cx: &mut gpui::VisualTestContext,
        view: &Entity<GitCometView>,
        description: &str,
        is_ready: Ready,
        snapshot: Snapshot,
    ) where
        T: std::fmt::Debug,
        Ready: Fn(&GitCometView) -> bool,
        Snapshot: Fn(&GitCometView) -> T,
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();

            let ready = cx.update(|_window, app| {
                let root = view.read(app);
                is_ready(&root)
            });
            if ready {
                return;
            }

            if Instant::now() >= deadline {
                let snapshot = cx.update(|_window, app| {
                    let root = view.read(app);
                    snapshot(&root)
                });
                panic!("timed out waiting for {description}: {snapshot:?}");
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn encode_terminal_key_input_supports_application_cursor_keys() {
        let up = encode_terminal_key_input(
            &Keystroke {
                modifiers: Modifiers::default(),
                key: "up".to_string(),
                key_char: None,
            },
            true,
        )
        .expect("expected cursor bytes");
        assert_eq!(up, b"\x1bOA");

        let home = encode_terminal_key_input(
            &Keystroke {
                modifiers: Modifiers::default(),
                key: "home".to_string(),
                key_char: None,
            },
            false,
        )
        .expect("expected home bytes");
        assert_eq!(home, b"\x1b[H");
    }

    #[test]
    fn encode_control_key_covers_common_shell_shortcuts() {
        assert_eq!(encode_control_key("c"), Some(3));
        assert_eq!(encode_control_key("d"), Some(4));
        assert_eq!(encode_control_key("space"), Some(0));
        assert_eq!(encode_control_key("7"), Some(31));
        assert_eq!(encode_control_key("left"), None);
    }

    #[test]
    fn terminal_grid_size_clamps_small_bounds() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(12.0), px(8.0)));
        let metrics = super::TerminalTextMetrics {
            font_size: px(12.0),
            line_height: px(18.0),
            cell_width: px(9.0),
        };
        let size = super::terminal_grid_size(bounds, metrics);
        assert_eq!(
            size,
            TerminalGridSize {
                rows: 2,
                cols: 8,
                pixel_width: 9,
                pixel_height: 18,
            }
        );
    }

    #[test]
    fn terminal_line_height_tracks_font_size_without_extra_leading() {
        assert_eq!(terminal_line_height(px(12.0)), px(12.0));
        assert_eq!(terminal_line_height(px(12.1)), px(13.0));
    }

    #[test]
    fn terminal_grid_point_for_position_clamps_to_visible_grid() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(80.0), px(30.0)));
        let metrics = super::TerminalTextMetrics {
            font_size: px(10.0),
            line_height: px(10.0),
            cell_width: px(10.0),
        };

        assert_eq!(
            terminal_grid_point_for_position(bounds, metrics, point(px(0.0), px(0.0)), 3, 8),
            TerminalGridPoint::new(0, 0)
        );
        assert_eq!(
            terminal_grid_point_for_position(bounds, metrics, point(px(99.0), px(99.0)), 3, 8),
            TerminalGridPoint::new(2, 8)
        );
    }

    #[test]
    fn terminal_selection_normalizes_and_extracts_visible_text() {
        let mut parser = vt100::Parser::new(3, 20, 10);
        parser.process(b"alpha\r\nbeta\r\ngamma");

        let selection =
            TerminalSelection::visible(TerminalGridPoint::new(1, 2), TerminalGridPoint::new(0, 1));

        assert_eq!(
            terminal_selection_text(parser.screen(), selection),
            "lpha\nbe"
        );
    }

    #[test]
    fn terminal_full_buffer_text_includes_scrollback() {
        let mut parser = vt100::Parser::new(2, 20, 10);
        parser.process(b"one\r\ntwo\r\nthree\r\nfour");

        assert_eq!(
            terminal_selection_text(parser.screen(), TerminalSelection::AllBuffer),
            "one\ntwo\nthree\nfour"
        );
        parser.screen_mut().set_scrollback(1);
        assert_eq!(
            terminal_selection_text(parser.screen(), TerminalSelection::AllBuffer),
            "one\ntwo\nthree\nfour"
        );
    }

    #[test]
    fn terminal_full_buffer_text_preserves_soft_wraps() {
        let mut parser = vt100::Parser::new(2, 4, 10);
        parser.process(b"abcdef\r\nxy");

        assert_eq!(
            terminal_selection_text(parser.screen(), TerminalSelection::AllBuffer),
            "abcdef\nxy"
        );
        parser.screen_mut().set_scrollback(1);
        assert!(parser.screen().scrollback() > 0);
        assert_eq!(
            terminal_selection_text(parser.screen(), TerminalSelection::AllBuffer),
            "abcdef\nxy"
        );
    }

    #[test]
    fn terminal_paste_bytes_normalizes_newlines_and_wraps_bracketed_paste() {
        assert_eq!(
            terminal_paste_bytes("alpha\r\nbeta\rgamma", false),
            b"alpha\nbeta\ngamma"
        );
        assert_eq!(
            terminal_paste_bytes("alpha\r\n", true),
            b"\x1b[200~alpha\n\x1b[201~"
        );
    }

    #[test]
    fn terminal_paste_bytes_sanitizes_bracketed_payload_controls() {
        let bytes = terminal_paste_bytes("alpha\x1b[201~\nrm\x07\u{009b}31m\x7fbeta", true);

        assert_eq!(bytes, b"\x1b[200~alpha[201~\nrm31mbeta\x1b[201~");
        assert!(
            bytes[BRACKETED_PASTE_START.len()..bytes.len() - BRACKETED_PASTE_END.len()]
                .iter()
                .all(|byte| *byte != b'\x1b')
        );
    }

    #[test]
    fn terminal_alternate_screen_scroll_bytes_repeat_cursor_keys() {
        assert_eq!(
            terminal_alternate_screen_scroll_bytes(px(120.0), 3, false),
            b"\x1b[A\x1b[A\x1b[A"
        );
        assert_eq!(
            terminal_alternate_screen_scroll_bytes(px(-120.0), 2, true),
            b"\x1bOB\x1bOB"
        );
        assert_eq!(
            terminal_alternate_screen_scroll_bytes(px(120.0), 0, false),
            b"\x1b[A"
        );
    }

    #[test]
    fn terminal_clipboard_shortcut_preserves_plain_control_keys() {
        let keystroke = |key: &str, modifiers: Modifiers| Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        };

        let mut plain_control = Modifiers::default();
        plain_control.control = true;
        assert_eq!(
            terminal_clipboard_shortcut_action(&keystroke("c", plain_control)),
            None
        );

        #[cfg(target_os = "macos")]
        {
            let mut command = Modifiers::default();
            command.platform = true;
            assert_eq!(
                terminal_clipboard_shortcut_action(&keystroke("c", command)),
                Some(TerminalShortcutAction::Copy)
            );

            let mut command_shift = command;
            command_shift.shift = true;
            assert_eq!(
                terminal_clipboard_shortcut_action(&keystroke("c", command_shift)),
                None
            );
        }

        #[cfg(not(target_os = "macos"))]
        {
            let mut terminal_copy = Modifiers::default();
            terminal_copy.control = true;
            terminal_copy.shift = true;
            assert_eq!(
                terminal_clipboard_shortcut_action(&keystroke("c", terminal_copy)),
                Some(TerminalShortcutAction::Copy)
            );
        }
    }

    #[test]
    fn terminal_caret_bounds_render_as_a_beam() {
        let cell = Bounds::new(point(px(4.0), px(8.0)), size(px(8.0), px(10.0)));
        let caret = terminal_caret_bounds(cell);
        assert_eq!(caret.left(), px(4.0));
        assert_eq!(caret.top(), px(9.0));
        assert_eq!(caret.size.width, px(2.0));
        assert_eq!(caret.size.height, px(8.0));
    }

    #[test]
    fn terminal_caret_bounds_clamp_wide_cells() {
        let cell = Bounds::new(point(px(0.0), px(0.0)), size(px(40.0), px(20.0)));
        let caret = terminal_caret_bounds(cell);
        assert_eq!(caret.size.width, px(3.0));
    }

    #[test]
    fn ansi_256_color_cube_uses_xterm_steps() {
        assert_eq!(color_cube_level(0), 0);
        assert_eq!(color_cube_level(1), 95);
        assert_eq!(color_cube_level(5), 255);
    }

    #[test]
    fn terminal_defaults_to_white_text_on_black_background() {
        assert_eq!(
            terminal_default_foreground(),
            super::rgba_from_hex(0xffffff)
        );
        assert_eq!(
            terminal_default_background(),
            super::rgba_from_hex(0x000000)
        );
    }

    #[test]
    fn terminal_repo_name_falls_back_to_path_display_when_file_name_is_missing() {
        let workdir = Path::new("/");
        assert_eq!(
            terminal_repo_name(workdir),
            super::super::path_display::path_display_string(workdir)
        );
    }

    #[test]
    fn resolve_external_terminal_launch_context_uses_repo_workdir_without_session() {
        let repo_id = RepoId(7);
        let workdir = PathBuf::from("/tmp/example-repo");
        let state = AppState {
            repos: vec![test_repo_state(repo_id, workdir.clone())],
            active_repo: Some(repo_id),
            ..Default::default()
        };

        let context =
            resolve_external_terminal_launch_context(&state, &HashMap::default(), repo_id)
                .expect("expected terminal launch context");

        assert_eq!(context.cwd, workdir);
        assert_eq!(context.repo_name.as_deref(), Some("example-repo"));
    }

    #[test]
    fn resolve_external_terminal_launch_context_errors_for_missing_repo() {
        let err = resolve_external_terminal_launch_context(
            &AppState::default(),
            &HashMap::default(),
            RepoId(99),
        )
        .expect_err("missing repo should fail");

        assert_eq!(err, "Repository is no longer available.");
    }

    #[test]
    fn resolve_embedded_terminal_spawn_failure_without_fallback_shows_error() {
        let preferences = TerminalPreferences {
            external_terminal_fallback: false,
            ..Default::default()
        };
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/example"),
            repo_name: Some("example".to_string()),
        };

        let action = resolve_embedded_terminal_spawn_failure(
            &preferences,
            &context,
            "spawn failed",
            |_preferences, _context| Ok(()),
        );

        assert_eq!(
            action,
            EmbeddedTerminalSpawnFailureAction::ShowError(
                "Failed to start embedded terminal: spawn failed".to_string()
            )
        );
    }

    #[test]
    fn resolve_embedded_terminal_spawn_failure_warns_when_fallback_succeeds() {
        let preferences = TerminalPreferences {
            external_terminal_fallback: true,
            ..Default::default()
        };
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/example"),
            repo_name: Some("example".to_string()),
        };

        let action = resolve_embedded_terminal_spawn_failure(
            &preferences,
            &context,
            "spawn failed",
            |_preferences, launch_context| {
                assert_eq!(launch_context.repo_name.as_deref(), Some("example"));
                Ok(())
            },
        );

        assert_eq!(
            action,
            EmbeddedTerminalSpawnFailureAction::OpenExternalWithWarning(
                "Embedded terminal failed to start (spawn failed); opened the configured external terminal instead.".to_string()
            )
        );
    }

    #[test]
    fn resolve_embedded_terminal_spawn_failure_combines_fallback_error() {
        let preferences = TerminalPreferences {
            external_terminal_fallback: true,
            ..Default::default()
        };

        let action = resolve_embedded_terminal_spawn_failure(
            &preferences,
            &ExternalTerminalLaunchContext {
                cwd: PathBuf::from("/tmp/example"),
                repo_name: None,
            },
            "spawn failed",
            |_preferences, _context| Err("launcher failed".to_string()),
        );

        assert_eq!(
            action,
            EmbeddedTerminalSpawnFailureAction::ShowError(
                "Failed to start embedded terminal: spawn failed. External terminal fallback also failed: launcher failed".to_string()
            )
        );
    }

    #[test]
    fn next_terminal_panel_height_clamps_to_min_and_window_limit() {
        let state = TerminalPanelResizeState {
            start_y: px(400.0),
            start_height: px(220.0),
        };

        assert_eq!(
            next_terminal_panel_height(state, px(1000.0), px(900.0)),
            px(TERMINAL_PANEL_MIN_HEIGHT_PX)
        );
        assert_eq!(
            next_terminal_panel_height(state, px(0.0), px(500.0)),
            terminal_panel_height_for_window(px(500.0))
        );
    }

    #[test]
    fn terminal_panel_height_for_window_respects_minimum_height() {
        assert_eq!(
            terminal_panel_height_for_window(px(100.0)),
            px(TERMINAL_PANEL_MIN_HEIGHT_PX)
        );
        assert_eq!(terminal_panel_height_for_window(px(540.0)), px(280.0));
    }

    #[test]
    fn read_next_terminal_chunk_retries_interrupted_reads() {
        let reader: Box<dyn Read + Send> = Box::new(ScriptedReader::new([
            ReadStep::Error(io::ErrorKind::Interrupted, "interrupted"),
            ReadStep::Bytes(b"hello".to_vec()),
        ]));

        let (_reader, result) = read_next_terminal_chunk(reader);

        assert_eq!(result.unwrap(), Some(b"hello".to_vec()));
    }

    #[test]
    fn read_next_terminal_chunk_returns_none_on_eof() {
        let reader: Box<dyn Read + Send> = Box::new(ScriptedReader::new([ReadStep::Eof]));

        let (_reader, result) = read_next_terminal_chunk(reader);

        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn read_next_terminal_chunk_propagates_non_interrupt_errors() {
        let reader: Box<dyn Read + Send> = Box::new(ScriptedReader::new([ReadStep::Error(
            io::ErrorKind::BrokenPipe,
            "broken",
        )]));

        let (_reader, result) = read_next_terminal_chunk(reader);

        let err = result.expect_err("expected read failure");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn terminal_read_batches_coalesce_bytes_and_completion() {
        let state = Arc::new(Mutex::new(TerminalReadBatchState::default()));
        {
            let mut batch = state.lock().unwrap();
            assert_eq!(
                push_terminal_read_bytes(&mut batch, b"hel".to_vec()),
                TerminalReadBatchAction::ScheduleFlush
            );
            assert_eq!(
                push_terminal_read_bytes(&mut batch, b"lo".to_vec()),
                TerminalReadBatchAction::None
            );
            batch.completion = Some(TerminalReadCompletion::Eof);
        }

        let batch = take_terminal_read_batch(&state).expect("expected batched terminal bytes");
        assert_eq!(batch.bytes, b"hello".to_vec());
        assert_eq!(batch.completion, Some(TerminalReadCompletion::Eof));
        assert!(take_terminal_read_batch(&state).is_none());
    }

    #[test]
    fn terminal_read_batches_flush_immediately_when_pending_bytes_get_large() {
        let mut state = TerminalReadBatchState::default();
        let action =
            push_terminal_read_bytes(&mut state, vec![b'x'; TERMINAL_READ_BATCH_MAX_BYTES]);

        assert_eq!(action, TerminalReadBatchAction::FlushNow);
        assert_eq!(state.bytes.len(), TERMINAL_READ_BATCH_MAX_BYTES);
        assert!(!state.flush_scheduled);
    }

    #[test]
    fn terminal_palette_color_supports_grayscale_ramp() {
        assert_eq!(terminal_palette_color(232), rgba_from_rgb(8, 8, 8));
        assert_eq!(terminal_palette_color(255), rgba_from_rgb(238, 238, 238));
    }

    #[test]
    fn terminal_cell_style_applies_inverse_dim_and_text_attributes() {
        let mut parser = vt100::Parser::new(1, 1, 0);
        parser.process(b"\x1b[31;44;1;3;4;2;7mX");

        let cell = parser
            .screen()
            .cell(0, 0)
            .expect("expected styled terminal cell");
        let red = terminal_palette_color(1);
        let blue = terminal_palette_color(4);
        let style = terminal_cell_style(
            cell,
            terminal_default_foreground(),
            terminal_default_background(),
        );

        assert_eq!(style.fg, mix_rgba(blue, red, 0.35));
        assert_eq!(style.bg, Some(red));
        assert!(style.italic);
        assert!(style.underline);
    }

    #[test]
    fn build_terminal_row_groups_runs_for_style_changes() {
        let mut parser = vt100::Parser::new(1, 3, 0);
        parser.process(b"A\x1b[31mB\x1b[0mC");

        let (text, runs) = build_terminal_row(
            parser.screen(),
            0,
            3,
            &gpui::TextStyle::default(),
            test_theme(),
        );

        assert_eq!(text.as_ref(), "ABC");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].len, 1);
        assert_eq!(runs[1].len, 1);
        assert_eq!(runs[2].len, 1);
        assert_eq!(runs[1].color, terminal_palette_color(1).into());
    }

    #[test]
    fn build_terminal_row_skips_wide_continuation_cells() {
        let mut parser = vt100::Parser::new(1, 2, 0);
        parser.process("\u{754c}".as_bytes());

        let (text, runs) = build_terminal_row(
            parser.screen(),
            0,
            2,
            &gpui::TextStyle::default(),
            test_theme(),
        );

        assert_eq!(text.as_ref(), "\u{754c}");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, "\u{754c}".len());
    }

    #[test]
    fn encode_terminal_key_input_supports_shift_navigation_and_alt_text() {
        let shifted_up = encode_terminal_key_input(
            &Keystroke {
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
                key: "up".to_string(),
                key_char: None,
            },
            false,
        )
        .expect("expected shifted-up escape sequence");
        assert_eq!(shifted_up, b"\x1b[1;2A");

        let alt_x = encode_terminal_key_input(
            &Keystroke {
                modifiers: Modifiers {
                    alt: true,
                    ..Default::default()
                },
                key: "x".to_string(),
                key_char: Some("x".to_string()),
            },
            false,
        )
        .expect("expected alt-modified text bytes");
        assert_eq!(alt_x, b"\x1bx");
    }

    #[test]
    fn encode_terminal_key_input_ignores_platform_shortcuts() {
        let bytes = encode_terminal_key_input(
            &Keystroke {
                modifiers: Modifiers {
                    platform: true,
                    ..Default::default()
                },
                key: "k".to_string(),
                key_char: Some("k".to_string()),
            },
            false,
        );

        assert!(bytes.is_none());
    }

    #[gpui::test]
    fn terminal_canvas_reuses_cached_rows_until_visible_content_changes(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-canvas-cache");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);

                let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
                    rows: 2,
                    cols: 4,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(2, 4);
                session.grid_size = TerminalGridSize {
                    rows: 2,
                    cols: 4,
                    pixel_width: 0,
                    pixel_height: 0,
                };
                session.parser.process(b"ABCD");
                session.content_epoch = 1;
                this.terminal_sessions.insert(repo_id, session);

                let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(80.0), px(40.0)));
                let layout = this.terminal_layout_snapshot(test_theme(), window, cx);
                let paint_state = this.build_terminal_canvas_paint_state(
                    repo_id,
                    1,
                    test_theme(),
                    bounds,
                    &layout,
                    window,
                );
                assert_eq!(paint_state.lines.len(), 1);
                assert_eq!(
                    this.terminal_sessions
                        .get(&repo_id)
                        .expect("expected cached terminal session")
                        .render_cache
                        .rebuilt_rows,
                    2
                );

                this.terminal_sessions
                    .get_mut(&repo_id)
                    .expect("expected cached terminal session")
                    .render_cache
                    .rebuilt_rows = 0;

                let layout = this.terminal_layout_snapshot(test_theme(), window, cx);
                let paint_state = this.build_terminal_canvas_paint_state(
                    repo_id,
                    1,
                    test_theme(),
                    bounds,
                    &layout,
                    window,
                );
                assert_eq!(paint_state.lines.len(), 1);
                assert_eq!(
                    this.terminal_sessions
                        .get(&repo_id)
                        .expect("expected reused terminal cache")
                        .render_cache
                        .rebuilt_rows,
                    0
                );

                {
                    let session = this
                        .terminal_sessions
                        .get_mut(&repo_id)
                        .expect("expected terminal session to mutate");
                    session.parser.process(b"\rZ");
                    session.content_epoch += 1;
                    session.render_cache.rebuilt_rows = 0;
                }

                let layout = this.terminal_layout_snapshot(test_theme(), window, cx);
                let paint_state = this.build_terminal_canvas_paint_state(
                    repo_id,
                    1,
                    test_theme(),
                    bounds,
                    &layout,
                    window,
                );
                assert_eq!(paint_state.lines.len(), 1);
                assert_eq!(
                    this.terminal_sessions
                        .get(&repo_id)
                        .expect("expected refreshed terminal cache")
                        .render_cache
                        .rebuilt_rows,
                    1
                );
            });
        });
    }

    #[gpui::test]
    fn terminal_cursor_blink_stays_visible_without_scheduling_in_deterministic_runtime(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-cursor-blink");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);

                let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
                    rows: TERMINAL_INITIAL_ROWS,
                    cols: TERMINAL_INITIAL_COLS,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let other_focus = cx.focus_handle().tab_index(1).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus.clone(), io, 1, cx),
                );

                window.focus(&other_focus, cx);
                this.sync_terminal_cursor_blink_activity(repo_id, window, cx);
                assert!(!this.terminal_cursor_blink_active);
                assert!(!this.terminal_cursor_blink_task_scheduled);
                assert!(this.terminal_cursor_blink_visible);

                window.focus(&focus, cx);
                this.sync_terminal_cursor_blink_activity(repo_id, window, cx);
                assert!(!this.terminal_cursor_blink_active);
                assert!(!this.terminal_cursor_blink_task_scheduled);
                assert!(this.terminal_cursor_blink_visible);

                this.terminal_cursor_blink_visible = false;
                window.focus(&other_focus, cx);
                this.sync_terminal_cursor_blink_activity(repo_id, window, cx);
                assert!(!this.terminal_cursor_blink_active);
                assert!(!this.terminal_cursor_blink_task_scheduled);
                assert!(this.terminal_cursor_blink_visible);
            });
        });
    }

    #[gpui::test]
    fn spawn_terminal_writer_task_writes_bytes_and_flushes_in_deterministic_runtime(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, mut cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/writer-task");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let writer_state = Arc::new(FakeWriterState::default());
        let (writer_tx, writer_rx) =
            smol::channel::bounded::<TerminalWriteRequest>(TERMINAL_WRITE_QUEUE_CAPACITY);

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
                    rows: TERMINAL_INITIAL_ROWS,
                    cols: TERMINAL_INITIAL_COLS,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 13, cx),
                );
                this.spawn_terminal_writer_task(
                    repo_id,
                    13,
                    writer_rx,
                    Box::new(FakeWriter::new(writer_state.clone())),
                    cx,
                );
            });
        });

        writer_tx
            .try_send(TerminalWriteRequest::Bytes(b"echo ".to_vec()))
            .expect("first terminal write should enqueue");
        writer_tx
            .try_send(TerminalWriteRequest::Bytes(b"hi\r".to_vec()))
            .expect("second terminal write should enqueue");
        writer_tx
            .try_send(TerminalWriteRequest::Shutdown)
            .expect("writer shutdown should enqueue");

        let writer_state_for_ready = writer_state.clone();
        let writer_state_for_snapshot = writer_state.clone();
        wait_for_view_condition(
            &mut cx,
            &view,
            "terminal writer task to flush queued bytes",
            move |_root| writer_state_for_ready.flush_count.load(Ordering::SeqCst) >= 1,
            move |root| {
                (
                    writer_state_for_snapshot.flush_count.load(Ordering::SeqCst),
                    writer_state_for_snapshot.bytes.lock().unwrap().clone(),
                    root.terminal_sessions
                        .get(&repo_id)
                        .map(|session| (session.connected, session.exit_status.clone())),
                )
            },
        );

        assert_eq!(writer_state.bytes.lock().unwrap().as_slice(), b"echo hi\r");
        assert_eq!(writer_state.flush_count.load(Ordering::SeqCst), 1);
        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("expected session to still be present");
            assert!(session.connected);
            assert_eq!(session.exit_status, None);
        });
    }

    #[gpui::test]
    fn terminal_launch_context_for_active_repo_prefers_session_workdir(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let repo_workdir = PathBuf::from("/tmp/repo-root");
        let session_workdir = PathBuf::from("/tmp/repo-root/subdir");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, repo_workdir.clone())],
            Some(repo_id),
        );

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);

                let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
                    rows: TERMINAL_INITIAL_ROWS,
                    cols: TERMINAL_INITIAL_COLS,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session =
                    make_test_terminal_session(session_workdir.clone(), focus, io, 1, cx);
                session.repo_name = terminal_repo_name(&repo_workdir);
                this.terminal_sessions.insert(repo_id, session);
            });
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let context = view
                .read(app)
                .terminal_launch_context_for_active_repo()
                .expect("expected active terminal context");
            assert_eq!(context.cwd, session_workdir);
            assert_eq!(context.repo_name.as_deref(), Some("repo-root"));
        });
    }

    #[gpui::test]
    fn toggle_terminal_for_active_repo_closes_existing_session_and_removes_indicator_icon(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/repo-with-terminal");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let killer_state = Arc::new(FakeKillerState::default());

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);

                let master_state = Arc::new(FakeMasterState {
                    size: Mutex::new(PtySize {
                        rows: TERMINAL_INITIAL_ROWS,
                        cols: TERMINAL_INITIAL_COLS,
                        pixel_width: 0,
                        pixel_height: 0,
                    }),
                    ..Default::default()
                });
                let writer_state = Arc::new(FakeWriterState::default());
                let io = Arc::new(Mutex::new(TerminalIo {
                    master: Box::new(FakeMasterPty::new(master_state)),
                    writer: Some(Box::new(FakeWriter::new(writer_state))),
                    killer: Box::new(FakeKiller::new(killer_state.clone())),
                    size: PtySize {
                        rows: TERMINAL_INITIAL_ROWS,
                        cols: TERMINAL_INITIAL_COLS,
                        pixel_width: 0,
                        pixel_height: 0,
                    },
                }));
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 1, cx),
                );
                this.sync_terminal_indicator_views(cx);
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(
            cx.debug_bounds("repo_tab_terminal_1").is_some(),
            "expected repo tab terminal icon while session is open"
        );

        cx.update(|window, app| {
            let _ = view.update(app, |this, cx| {
                this.toggle_terminal_for_active_repo(window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        cx.update(|_window, app| {
            assert!(
                !view.read(app).terminal_sessions.contains_key(&repo_id),
                "expected active repo terminal to close"
            );
            let (repo_tabs_ids, action_ids) = indicator_repo_ids(&view, app);
            assert!(repo_tabs_ids.is_empty());
            assert!(action_ids.is_empty());
        });
        assert_eq!(killer_state.kill_count.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn apply_state_snapshot_prunes_removed_terminal_sessions_and_updates_indicators(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo1 = RepoId(1);
        let repo2 = RepoId(2);
        let workdir1 = PathBuf::from("/tmp/repo-one");
        let workdir2 = PathBuf::from("/tmp/repo-two");
        let initial_state = app_state_with_repos(
            vec![
                test_repo_state(repo1, workdir1.clone()),
                test_repo_state(repo2, workdir2.clone()),
            ],
            Some(repo1),
        );
        let next_state =
            app_state_with_repos(vec![test_repo_state(repo1, workdir1.clone())], Some(repo1));
        let (io1, _master1, _writer1, killer1) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        });
        let (io2, _master2, _writer2, killer2) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, initial_state, cx);
                let focus1 = cx.focus_handle().tab_index(0).tab_stop(false);
                let focus2 = cx.focus_handle().tab_index(1).tab_stop(false);
                this.terminal_sessions.insert(
                    repo1,
                    make_test_terminal_session(workdir1.clone(), focus1, io1, 1, cx),
                );
                this.terminal_sessions.insert(
                    repo2,
                    make_test_terminal_session(workdir2.clone(), focus2, io2, 2, cx),
                );
                this.sync_terminal_indicator_views(cx);
            });
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let (repo_tabs_ids, action_ids) = indicator_repo_ids(&view, app);
            assert_eq!(repo_tabs_ids.len(), 2);
            assert_eq!(action_ids.len(), 2);
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                push_test_state(this, next_state, cx);
            });
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let root = view.read(app);
            assert!(root.terminal_sessions.contains_key(&repo1));
            assert!(!root.terminal_sessions.contains_key(&repo2));
            let (repo_tabs_ids, action_ids) = indicator_repo_ids(&view, app);
            assert_eq!(repo_tabs_ids, HashSet::from_iter([repo1]));
            assert_eq!(action_ids, HashSet::from_iter([repo1]));
        });
        assert_eq!(killer1.kill_count.load(Ordering::SeqCst), 0);
        assert_eq!(killer2.kill_count.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn terminal_mouse_drag_selection_copies_visible_text(cx: &mut gpui::TestAppContext) {
        let _clipboard_guard = lock_clipboard_test();
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-mouse-selection");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: 3,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(3, 20);
                session.parser.process(b"alpha\r\nbeta");
                session.viewport_bounds = Some(Bounds::new(
                    point(px(0.0), px(0.0)),
                    size(px(600.0), px(200.0)),
                ));
                this.terminal_sessions.insert(repo_id, session);

                let layout = this.terminal_layout_snapshot(test_theme(), window, cx);
                let start = point(
                    layout.metrics.cell_width * 0.1,
                    layout.metrics.line_height * 0.5,
                );
                let end = point(
                    layout.metrics.cell_width * 5.2,
                    layout.metrics.line_height * 0.5,
                );
                this.handle_terminal_selection_mouse_down(repo_id, start, window, cx);
                this.handle_terminal_selection_mouse_move(repo_id, end, window, cx);
                this.handle_terminal_selection_mouse_up(repo_id, cx);
                assert!(this.copy_terminal_selection_for_repo(repo_id, window, cx));
            });
        });

        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("alpha".into())
        );
    }

    #[gpui::test]
    fn terminal_output_clears_visible_selection_before_copy(cx: &mut gpui::TestAppContext) {
        let _clipboard_guard = lock_clipboard_test();
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-output-clears-selection");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: 3,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        });
        let batch_state = Arc::new(Mutex::new(TerminalReadBatchState::default()));
        {
            let mut batch = batch_state.lock().unwrap();
            batch.bytes = b"\romega".to_vec();
        }

        cx.write_to_clipboard(gpui::ClipboardItem::new_string("unchanged".to_string()));
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(3, 20);
                session.parser.process(b"alpha");
                session.selection = Some(TerminalSelection::visible(
                    TerminalGridPoint::new(0, 0),
                    TerminalGridPoint::new(0, 5),
                ));
                session.selection_drag_anchor = Some(TerminalGridPoint::new(0, 0));
                this.terminal_sessions.insert(repo_id, session);

                this.flush_terminal_read_batch(repo_id, 1, &batch_state, cx);

                let session = this
                    .terminal_sessions
                    .get(&repo_id)
                    .expect("session should remain available");
                assert_eq!(
                    session.parser.screen().contents_between(0, 0, 0, 5),
                    "omega"
                );
                assert_eq!(session.selection, None);
                assert_eq!(session.selection_drag_anchor, None);
                assert!(!this.terminal_has_copyable_selection(repo_id));
                assert!(!this.copy_terminal_selection_for_repo(repo_id, window, cx));
            });
        });

        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("unchanged".into())
        );
    }

    #[gpui::test]
    fn terminal_context_menu_opens_from_root_update(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-context-menu");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: 3,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(3, 20);
                session.parser.process(b"alpha\r\nbeta");
                session.selection = Some(TerminalSelection::AllBuffer);
                this.terminal_sessions.insert(repo_id, session);

                this.open_terminal_context_menu(repo_id, point(px(12.0), px(24.0)), window, cx);
            });
        });

        let popover = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert_eq!(
            popover,
            Some(PopoverKind::TerminalMenu {
                repo_id,
                context: TerminalMenuContext {
                    has_session: true,
                    has_selection: true,
                    connected: true,
                },
            })
        );
    }

    #[gpui::test]
    fn terminal_wheel_scroll_over_panel_moves_scrollback(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-wheel-scroll");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: 6,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                this.terminal_panel_height = px(160.0);
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(6, 20);
                for ix in 0..80 {
                    session
                        .parser
                        .process(format!("line {ix:02}\r\n").as_bytes());
                }
                this.terminal_sessions.insert(repo_id, session);
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let panel_bounds = cx
            .debug_bounds("terminal_panel")
            .expect("expected terminal panel to be rendered");
        let position = panel_bounds.center();
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(120.0))),
            ..Default::default()
        });
        cx.run_until_parked();

        let scrollback = cx.update(|_window, app| {
            view.read(app)
                .terminal_sessions
                .get(&repo_id)
                .map(|session| session.parser.screen().scrollback())
                .unwrap_or_default()
        });
        assert!(
            scrollback > 0,
            "expected wheel-up over terminal panel to move into scrollback"
        );
    }

    #[gpui::test]
    fn terminal_wheel_scroll_in_alternate_screen_sends_scroll_input(cx: &mut gpui::TestAppContext) {
        let _visual_guard = lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-wheel-alt-screen");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, writer_state, _killer_state) = fake_session_io(PtySize {
            rows: 6,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                this.terminal_panel_height = px(160.0);
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(6, 20);
                session.parser.process(b"\x1b[?1049h");
                assert!(session.parser.screen().alternate_screen());
                this.terminal_sessions.insert(repo_id, session);
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let panel_bounds = cx
            .debug_bounds("terminal_panel")
            .expect("expected terminal panel to be rendered");
        let position = panel_bounds.center();
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(120.0))),
            ..Default::default()
        });
        cx.run_until_parked();

        let bytes = writer_state.bytes.lock().unwrap().clone();
        assert!(
            bytes.starts_with(b"\x1b[A"),
            "expected alternate-screen wheel-up to send cursor-up input, got {bytes:?}"
        );
    }

    #[gpui::test]
    fn terminal_menu_actions_paste_clear_and_select_all(cx: &mut gpui::TestAppContext) {
        let _clipboard_guard = lock_clipboard_test();
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/terminal-menu-actions");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, writer_state, _killer_state) = fake_session_io(PtySize {
            rows: 3,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.write_to_clipboard(gpui::ClipboardItem::new_string("one\r\ntwo".to_string()));
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_size(3, 20);
                session.parser.process(b"alpha\r\nbeta");
                this.terminal_sessions.insert(repo_id, session);

                assert!(this.paste_terminal_clipboard_for_repo(repo_id, window, cx));
                assert!(this.clear_terminal_for_repo(repo_id, window, cx));
                this.select_all_terminal_for_repo(repo_id, window, cx);
                assert!(this.copy_terminal_selection_for_repo(repo_id, window, cx));
            });
        });

        assert_eq!(
            writer_state.bytes.lock().unwrap().as_slice(),
            b"one\ntwo\x0c"
        );
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("alpha\nbeta".into())
        );
    }

    #[gpui::test]
    fn send_terminal_bytes_for_repo_writes_bytes_and_resets_scrollback_and_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/send-terminal-success");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, writer_state, _killer_state) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                let mut session = make_test_terminal_session(workdir.clone(), focus, io, 1, cx);
                session.parser.screen_mut().set_scrollback(12);
                session.selection = Some(TerminalSelection::AllBuffer);
                {
                    let mut state = session.terminal.state.lock().unwrap();
                    state.parser.screen_mut().set_scrollback(12);
                    state.selection = Some(TerminalSelection::AllBuffer);
                }
                this.terminal_sessions.insert(repo_id, session);
                this.send_terminal_bytes_for_repo(repo_id, b"ls\r", cx)
                    .expect("expected terminal bytes to be written");
            });
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("session should remain available");
            assert_eq!(session.parser.screen().scrollback(), 0);
            assert_eq!(session.selection, None);
            let state = session.terminal.state.lock().unwrap();
            assert_eq!(state.parser.screen().scrollback(), 0);
            assert_eq!(state.selection, None);
            assert!(session.connected);
            assert_eq!(session.exit_status, None);
        });
        assert_eq!(writer_state.bytes.lock().unwrap().as_slice(), b"ls\r");
        assert_eq!(writer_state.flush_count.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn send_terminal_bytes_for_repo_marks_terminal_disconnected_on_write_error(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/send-terminal-error");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, writer_state, _killer_state) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        });
        *writer_state.write_error.lock().unwrap() = Some(io::ErrorKind::BrokenPipe);

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 1, cx),
                );
                let err = this
                    .send_terminal_bytes_for_repo(repo_id, b"pwd\r", cx)
                    .expect_err("write failure should surface");
                assert!(err.contains("write failed"));
            });
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("session should remain readable after write error");
            assert!(!session.connected);
            assert_eq!(
                session.exit_status.as_deref(),
                Some("Failed to send input: write failed")
            );
        });
    }

    #[gpui::test]
    fn send_terminal_bytes_for_repo_handles_poisoned_lock(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/send-terminal-poison");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        });
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let io = io.clone();
            move || {
                let _guard = io.lock().unwrap();
                panic!("poison terminal lock");
            }
        }));

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 1, cx),
                );
                let err = this
                    .send_terminal_bytes_for_repo(repo_id, b"pwd\r", cx)
                    .expect_err("poisoned lock should return an error");
                assert_eq!(err, "Terminal state lock was poisoned.");
            });
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("session should remain readable after lock poison");
            assert!(!session.connected);
            assert_eq!(
                session.exit_status.as_deref(),
                Some("Failed to send input: Terminal state lock was poisoned.")
            );
        });
    }

    #[gpui::test]
    fn sync_terminal_grid_size_updates_parser_and_pty(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/sync-grid-size");
        let state = app_state_with_repos(vec![test_repo_state(repo_id, workdir.clone())], None);
        let (io, master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 9,
            pixel_height: 18,
        });
        let next_size = TerminalGridSize {
            rows: 40,
            cols: 120,
            pixel_width: 11,
            pixel_height: 21,
        };

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io.clone(), 9, cx),
                );
                this.sync_terminal_grid_size(repo_id, 9, next_size);
            });
        });

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("expected session to be present");
            assert_eq!(session.parser.screen().size(), (40, 120));
            assert_eq!(session.grid_size, next_size);
            assert_eq!(session.content_epoch, 1);
            assert_eq!(session.io.lock().unwrap().size, next_size.into_pty_size());
        });
        assert_eq!(
            master_state.resize_calls.lock().unwrap().as_slice(),
            &[next_size.into_pty_size()]
        );
    }

    #[gpui::test]
    fn sync_terminal_grid_size_ignores_stale_session_seq(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/sync-grid-size-stale");
        let state = app_state_with_repos(vec![test_repo_state(repo_id, workdir.clone())], None);
        let (io, master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
            rows: TERMINAL_INITIAL_ROWS,
            cols: TERMINAL_INITIAL_COLS,
            pixel_width: 9,
            pixel_height: 18,
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 5, cx),
                );
                this.sync_terminal_grid_size(
                    repo_id,
                    99,
                    TerminalGridSize {
                        rows: 40,
                        cols: 120,
                        pixel_width: 11,
                        pixel_height: 21,
                    },
                );
            });
        });

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("expected session to be present");
            assert_eq!(
                session.parser.screen().size(),
                (TERMINAL_INITIAL_ROWS, TERMINAL_INITIAL_COLS)
            );
            assert_eq!(session.grid_size, initial_terminal_grid_size());
            assert_eq!(session.content_epoch, 0);
        });
        assert!(master_state.resize_calls.lock().unwrap().is_empty());
    }

    #[gpui::test]
    fn spawn_terminal_reader_task_batches_chunks_and_marks_disconnect_on_eof(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/reader-task");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
                    rows: TERMINAL_INITIAL_ROWS,
                    cols: TERMINAL_INITIAL_COLS,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 11, cx),
                );
                this.spawn_terminal_reader_task(
                    repo_id,
                    11,
                    Box::new(ScriptedReader::new([
                        ReadStep::Bytes(b"hel".to_vec()),
                        ReadStep::Bytes(b"lo".to_vec()),
                        ReadStep::Eof,
                    ])),
                    cx,
                );
            });
        });

        wait_for_view_condition(
            cx,
            &view,
            "terminal reader task to reach EOF",
            |root| {
                root.terminal_sessions
                    .get(&repo_id)
                    .is_some_and(|session| !session.connected)
            },
            |root| {
                root.terminal_sessions.get(&repo_id).map(|session| {
                    (
                        session.connected,
                        session.exit_status.clone(),
                        session.parser.screen().contents(),
                    )
                })
            },
        );

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("expected session to still be present");
            assert!(!session.connected);
            assert_eq!(session.exit_status, None);
            assert!(session.content_epoch >= 1);
            assert!(session.parser.screen().contents().contains("hello"));
        });
    }

    #[gpui::test]
    fn spawn_terminal_wait_task_records_exit_code(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let workdir = PathBuf::from("/tmp/wait-task");
        let state = app_state_with_repos(
            vec![test_repo_state(repo_id, workdir.clone())],
            Some(repo_id),
        );
        let killer_state = Arc::new(FakeKillerState::default());

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.disable_poller_for_tests();
                push_test_state(this, state, cx);
                let (io, _master_state, _writer_state, _killer_state) = fake_session_io(PtySize {
                    rows: TERMINAL_INITIAL_ROWS,
                    cols: TERMINAL_INITIAL_COLS,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let focus = cx.focus_handle().tab_index(0).tab_stop(false);
                this.terminal_sessions.insert(
                    repo_id,
                    make_test_terminal_session(workdir.clone(), focus, io, 12, cx),
                );
                this.spawn_terminal_wait_task(
                    repo_id,
                    12,
                    Box::new(FakeChild::new(
                        FakeWaitResult::Exit(ExitStatus::with_exit_code(7)),
                        killer_state,
                    )),
                    cx,
                );
            });
        });

        wait_for_view_condition(
            cx,
            &view,
            "terminal wait task to record exit status",
            |root| {
                root.terminal_sessions
                    .get(&repo_id)
                    .and_then(|session| session.exit_status.as_ref())
                    .is_some()
            },
            |root| {
                root.terminal_sessions
                    .get(&repo_id)
                    .map(|session| (session.connected, session.exit_status.clone()))
            },
        );

        cx.update(|_window, app| {
            let root = view.read(app);
            let session = root
                .terminal_sessions
                .get(&repo_id)
                .expect("expected session to still be present");
            assert!(!session.connected);
            assert_eq!(
                session.exit_status.as_deref(),
                Some("Shell exited with code 7.")
            );
        });
    }
}
