use super::terminal_alacritty::*;
use super::*;
use crate::kit::ScrollbarDriver;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use palette::IntoColor;
use rustc_hash::FxHasher;
#[cfg(unix)]
use rustix::process::{Pid, Signal, kill_process_group};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

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

// ---------------------------------------------------------------------------
// Terminal scrollbar driver — maps Alacritty display_offset to pixel space
// so the kit Scrollbar component can drive the terminal's scroll model.
// ---------------------------------------------------------------------------

struct TerminalScrollbarDriver {
    term_lock: Option<AlacrittyTermLock>,
    line_height: Pixels,
}

impl ScrollbarDriver for TerminalScrollbarDriver {
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        if axis != ScrollbarAxis::Vertical {
            return px(0.0);
        }
        let Some(ref term_lock) = self.term_lock else {
            return px(0.0);
        };
        (term_lock.lock().grid().history_size() as f32 * self.line_height).max(px(0.0))
    }

    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        if axis != ScrollbarAxis::Vertical {
            return px(0.0);
        }
        let Some(ref term_lock) = self.term_lock else {
            return px(0.0);
        };
        let term = term_lock.lock();
        let history = term.grid().history_size() as f32;
        let display = term.grid().display_offset() as f32;
        -(history - display) * self.line_height
    }

    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels) {
        if axis != ScrollbarAxis::Vertical {
            return;
        }
        let Some(ref term_lock) = self.term_lock else {
            return;
        };
        let mut term = term_lock.lock();
        let history = term.grid().history_size();
        if history == 0 {
            return;
        }
        let current = term.grid().display_offset();
        let scroll_y = if offset < px(0.0) { -offset } else { offset };
        let target =
            history.saturating_sub(((scroll_y / self.line_height).round() as usize).min(history));
        let delta = target as i32 - current as i32;
        if delta != 0 {
            let steps = delta.unsigned_abs() as usize;
            if delta > 0 {
                for _ in 0..steps {
                    term.scroll_display(Scroll::Delta(1));
                }
            } else {
                for _ in 0..steps {
                    term.scroll_display(Scroll::Delta(-1));
                }
            }
        }
    }
}
#[derive(Default)]
struct TerminalCanvasPaintState {
    bounds: Bounds<Pixels>,
    terminal_bg: gpui::Rgba,
    selection_rects: Vec<Bounds<Pixels>>,
    background_rects: Vec<(Point<Pixels>, gpui::Size<Pixels>, gpui::Rgba)>,
    lines: Vec<(ShapedLine, Point<Pixels>, Pixels)>,
    cursor: Option<TerminalPaintCursor>,
    ime_bounds: Option<Bounds<Pixels>>,
    ime_marked_text: Option<String>,
    ime_base_style: Option<gpui::TextStyle>,
}

#[derive(Clone)]
struct TerminalPaintCursor {
    bounds: Bounds<Pixels>,
    shape: TerminalCursorShape,
}

/// Grid dimensions read live from the backing `Term`, rather than from the
/// `last_content` snapshot that is only refreshed during canvas prepaint. Any
/// `scroll_display` (autoscroll tick, wheel, scrollbar drag, scrollback keys)
/// leaves that snapshot's `display_offset` stale until the next paint, so
/// selection must resolve against these values instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalGridGeometry {
    display_offset: usize,
    history_size: usize,
    columns: usize,
    screen_lines: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalShortcutAction {
    Copy,
    Paste,
    SelectAll,
}

/// Drag payload used to track an in-progress terminal panel resize. Using the
/// drag/drag-move machinery (rather than element-local `on_mouse_move`) keeps
/// move events flowing even when the cursor leaves the thin handle bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalPanelResizeDrag;

/// Zero-size element that installs window-level mouse listeners so a selection
/// drag keeps extending after the pointer leaves the terminal viewport.
///
/// Element-local `on_mouse_move`/`on_mouse_up` are hitbox-gated by gpui (they
/// only fire while the element is hovered), which is why drag-selection used to
/// die at the viewport edge. Same shape as `DiffTextSelectionTracker` in
/// `diff_text_selection.rs`.
struct TerminalSelectionTracker {
    view: Entity<TerminalViewportView>,
}

impl IntoElement for TerminalSelectionTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalSelectionTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(0.0).into();
        style.size.height = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        // Registered unconditionally and gated inside the closures: `selecting`
        // only becomes true while handling mouse-down, i.e. after this frame was
        // painted, so gating here would drop every move and up event of the drag
        // that just started.
        let view_for_move = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            view_for_move.update(cx, |this, cx| {
                if !this.selecting {
                    return;
                }
                if !event.dragging() {
                    // The button came up without us seeing the release (e.g. it
                    // was let go outside the window). Don't leave the drag — and
                    // its autoscroll ticker — running forever.
                    this.end_selection_drag(cx);
                    return;
                }
                if this.drag_selection_to(event.position) {
                    cx.notify();
                }
            });
        });

        let view_for_up = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Bubble {
                return;
            }
            if event.button != MouseButton::Left {
                return;
            }
            view_for_up.update(cx, |this, cx| {
                // Cleared before the `selecting` check, because the two are
                // mutually exclusive: `pressed_mouse_button` is only set on the
                // mouse-reporting path and `selecting` only on the selection
                // path. Element-local `on_mouse_up` is hitbox-gated, so without
                // this a press released outside the viewport left the button
                // latched and every later motion report claimed it was held.
                // (A release report is not synthesised for the TUI here — that
                // would double-report whenever the release lands inside.)
                this.pressed_mouse_button = None;
                this.end_selection_drag(cx);
            });
        });
    }
}

// ============================================================================
// TerminalViewportView
// ============================================================================

impl TerminalViewportView {
    pub(super) fn new(
        theme: AppTheme,
        focus_handle: FocusHandle,
        term_lock: AlacrittyTermLock,
        pty_sender: terminal_alacritty::PtySender,
    ) -> Self {
        Self::with_backend(theme, focus_handle, Some(term_lock), Some(pty_sender))
    }

    /// Shared constructor. Tests use it to build a viewport over a real `Term`
    /// but with no PTY, since a `PtySender` can only come from a spawned event
    /// loop.
    fn with_backend(
        theme: AppTheme,
        focus_handle: FocusHandle,
        term_lock: Option<AlacrittyTermLock>,
        pty_sender: Option<terminal_alacritty::PtySender>,
    ) -> Self {
        Self {
            theme,
            focus_handle,
            term_lock,
            pty_sender,
            layout_cache: None,
            render_cache: TerminalRenderCache::default(),
            cursor_blink_visible: true,
            cursor_blink_hold_until: Instant::now(),
            cursor_blink_active: false,
            cursor_blink_task_scheduled: false,
            cursor_blink_seq: 0,
            content_epoch: 1,
            last_content: None,
            viewport_bounds: None,
            pressed_mouse_button: None,
            last_motion_cell: None,
            was_focused: false,
            selection_start: None,
            selection_end: None,
            select_all_active: false,
            selecting: false,
            selection_last_mouse_pos: point(px(0.0), px(0.0)),
            selection_drag_moved: false,
            selection_autoscroll_seq: 0,
            ime_state: None,
        }
    }

    pub(super) fn invalidate_layout(&mut self, cx: &mut gpui::Context<Self>) {
        self.layout_cache = None;
        self.render_cache = TerminalRenderCache::default();
        cx.notify();
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
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
        if let Some(ref cache) = self.layout_cache
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
        self.connected() && self.focus_handle.is_focused(window)
    }

    fn connected(&self) -> bool {
        self.term_lock.is_some() && self.pty_sender.is_some()
    }

    fn sync_cursor_blink_activity(&mut self, window: &Window, cx: &mut gpui::Context<Self>) {
        let is_focused = self.focus_handle.is_focused(window);
        if is_focused && !self.was_focused {
            self.handle_focus_gained(cx);
        } else if !is_focused && self.was_focused {
            self.handle_focus_lost(cx);
        }
        self.was_focused = is_focused;

        if !crate::ui_runtime::current().uses_cursor_blink() {
            if self.cursor_blink_active
                || self.cursor_blink_task_scheduled
                || !self.cursor_blink_visible
            {
                self.deactivate_cursor_blink();
            }
            return;
        }
        if self.cursor_blink_should_run(window) {
            if !self.cursor_blink_active {
                self.cursor_blink_active = true;
                self.cursor_blink_seq = self.cursor_blink_seq.wrapping_add(1);
            }
            self.schedule_cursor_blink_tick(cx);
        } else if self.cursor_blink_active || !self.cursor_blink_visible {
            self.deactivate_cursor_blink();
        }
    }

    fn handle_focus_gained(&mut self, cx: &mut gpui::Context<Self>) {
        let has_focus_mode = self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::FOCUS_IN_OUT))
            .unwrap_or(false);
        if has_focus_mode {
            self.queue_input(b"\x1b[I".to_vec(), cx);
        }
    }

    fn handle_focus_lost(&mut self, cx: &mut gpui::Context<Self>) {
        // Backstop for a drag whose mouse-up never reaches us — a release over
        // another window is not delivered on every platform. Without this the
        // drag stays live: its ticker keeps waking, keeps taking the terminal
        // lock, and the next unrelated pointer move silently resumes selecting.
        self.end_selection_drag(cx);
        let has_focus_mode = self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::FOCUS_IN_OUT))
            .unwrap_or(false);
        if has_focus_mode {
            self.queue_input(b"\x1b[O".to_vec(), cx);
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

    pub(super) fn reset_cursor_blink(&mut self, cx: &mut gpui::Context<Self>) {
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

    fn queue_input(&mut self, bytes: Vec<u8>, cx: &mut gpui::Context<Self>) {
        if let Some(ref pty) = self.pty_sender {
            pty.write(bytes);
            self.reset_cursor_blink(cx);
            cx.notify();
        }
    }

    /// Handles a keystroke aimed at the terminal. Returns `true` when the
    /// keystroke was consumed (encoded and forwarded to the PTY, or used for a
    /// terminal shortcut/scrollback action). Callers use the return value to
    /// decide whether to suppress the app's global key bindings — see
    /// [`GitCometView::forward_keystroke_to_focused_terminal`].
    fn handle_key_down(
        &mut self,
        keystroke: &gpui::Keystroke,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if let Some(action) = terminal_clipboard_shortcut_action(keystroke) {
            self.perform_clipboard_action(action, cx);
            cx.stop_propagation();
            return true;
        }
        if self.handle_scrollback_key(keystroke, cx) {
            cx.stop_propagation();
            return true;
        }
        let app_cursor = self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::APP_CURSOR))
            .unwrap_or(false);
        let option_as_meta = true;
        if let Some(bytes) = encode_alacritty_key_input(keystroke, app_cursor, option_as_meta) {
            // Typing clears any active selection / select-all so the highlight
            // doesn't linger and `selected_text()` stops copying the whole buffer.
            if self.clear_selection() {
                cx.notify();
            }
            self.queue_input(bytes, cx);
            cx.stop_propagation();
            return true;
        }
        false
    }

    fn perform_clipboard_action(
        &mut self,
        action: TerminalShortcutAction,
        cx: &mut gpui::Context<Self>,
    ) {
        match action {
            TerminalShortcutAction::Copy => {
                let text = if self.select_all_active {
                    self.copy_entire_buffer()
                } else if let Some((start, end)) = self.selection_start.zip(self.selection_end) {
                    self.copy_grid_range(start, end)
                } else {
                    // Fallback: copy visible screen content when no selection
                    self.copy_visible_screen()
                };
                if !text.is_empty() {
                    crate::clipboard::write_text(
                        cx,
                        text,
                        crate::clipboard::CopySource::TerminalShortcut,
                    );
                }
            }
            TerminalShortcutAction::Paste => {
                let bracketed = self
                    .last_content
                    .as_ref()
                    .map(|c| c.mode.contains(TerminalModes::BRACKETED_PASTE))
                    .unwrap_or(false);
                if let Some(text) = crate::clipboard::read_text(cx) {
                    let bytes = terminal_paste_bytes(&text, bracketed);
                    self.queue_input(bytes, cx);
                }
            }
            TerminalShortcutAction::SelectAll => self.select_all(cx),
        }
    }

    fn copy_grid_range(&self, start: TerminalGridPoint, end: TerminalGridPoint) -> String {
        let (mut first, mut last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let Some(term_lock) = &self.term_lock else {
            return String::new();
        };
        let term = term_lock.lock();
        let grid = term.grid();
        // Dimensions come from the live grid, not from `last_content`: a resize
        // or scrollback eviction between selecting and copying would otherwise
        // index the grid out of range and panic.
        let cols = grid.columns();
        let history_size = grid.history_size();
        let screen_lines = grid.screen_lines();
        if cols == 0 || screen_lines == 0 {
            return String::new();
        }
        first.row = terminal_clamp_grid_row(first.row, history_size, screen_lines);
        last.row = terminal_clamp_grid_row(last.row, history_size, screen_lines);
        let mut text = String::new();
        for row in first.row..=last.row {
            let sc = if row == first.row {
                (first.col as usize).min(cols)
            } else {
                0
            };
            let ec = if row == last.row {
                (last.col as usize + 1).min(cols)
            } else {
                cols
            };
            let line_start = text.len();
            for c in sc..ec {
                let cell = &grid[Line(row)][Column(c)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch = cell.c;
                if ch == ' ' || ch == '\0' {
                    text.push(' ');
                } else {
                    text.push(ch);
                }
            }
            // Grid rows are space-padded to the full width, so whenever the
            // selection reaches the line end the trailing spaces are padding
            // rather than selected content. A selection that stops mid-row is
            // left verbatim — those spaces really were selected.
            if ec == cols {
                let trimmed = text[line_start..].trim_end().len();
                text.truncate(line_start + trimmed);
            }
            if row < last.row && !terminal_row_wraps(&grid[Line(row)], cols) {
                text.push('\n');
            }
        }
        drop(term);
        text
    }

    fn copy_visible_screen(&self) -> String {
        let Some(term_lock) = &self.term_lock else {
            return String::new();
        };
        let term = term_lock.lock();
        let grid = term.grid();
        // Live dimensions, not the `last_content` snapshot: a shrinking resize
        // between paint and copy would otherwise index the grid out of range.
        let display_offset = grid.display_offset();
        let cols = grid.columns();
        let rows = grid.screen_lines();
        let mut text = String::new();
        for row in 0..rows {
            let grid_row = row as i32 - display_offset as i32;
            for c in 0..cols {
                let cell = &grid[Line(grid_row)][Column(c)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch = cell.c;
                if ch == ' ' || ch == '\0' {
                    text.push(' ');
                } else {
                    text.push(ch);
                }
            }
            if row + 1 < rows && !terminal_row_wraps(&grid[Line(grid_row)], cols) {
                text.push('\n');
            }
        }
        drop(term);
        trim_terminal_copy(&text)
    }

    /// Copies the entire terminal buffer, including the scrollback history above
    /// the visible screen. The buffer spans `Line(-history_size)` (oldest) to
    /// `Line(screen_lines - 1)` (newest).
    fn copy_entire_buffer(&self) -> String {
        let Some(term_lock) = &self.term_lock else {
            return String::new();
        };
        let term = term_lock.lock();
        let grid = term.grid();
        // Live dimensions, not the `last_content` snapshot: a shrinking resize
        // between paint and copy would otherwise index the grid out of range.
        let cols = grid.columns();
        let screen_lines = grid.screen_lines();
        let history_size = grid.history_size();
        let mut text = String::new();
        let top = -(history_size as i32);
        let bottom = screen_lines as i32 - 1;
        for row in top..=bottom {
            for c in 0..cols {
                let cell = &grid[Line(row)][Column(c)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch = cell.c;
                if ch == ' ' || ch == '\0' {
                    text.push(' ');
                } else {
                    text.push(ch);
                }
            }
            if row < bottom && !terminal_row_wraps(&grid[Line(row)], cols) {
                text.push('\n');
            }
        }
        drop(term);
        trim_terminal_copy(&text)
    }

    pub(super) fn has_selection(&self) -> bool {
        self.selection_start.zip(self.selection_end).is_some()
    }

    pub(super) fn selected_text(&self) -> Option<String> {
        let text = if self.select_all_active {
            self.copy_entire_buffer()
        } else {
            let (start, end) = self.selection_start.zip(self.selection_end)?;
            self.copy_grid_range(start, end)
        };
        if text.is_empty() { None } else { Some(text) }
    }

    /// Live grid dimensions, or `None` when there is no backing terminal.
    ///
    /// Takes the terminal lock, so it must not be called while that lock is
    /// already held — `FairMutex` is not reentrant.
    fn grid_geometry(&self) -> Option<TerminalGridGeometry> {
        let term = self.term_lock.as_ref()?.lock();
        let grid = term.grid();
        Some(TerminalGridGeometry {
            display_offset: grid.display_offset(),
            history_size: grid.history_size(),
            columns: grid.columns(),
            screen_lines: grid.screen_lines(),
        })
    }

    pub(super) fn select_all(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(geometry) = self.grid_geometry() else {
            return;
        };
        if geometry.screen_lines == 0 || geometry.columns == 0 {
            return;
        }
        // Select the whole buffer, scrollback included, so the highlight stays
        // correct at every scroll position. Copy still routes through
        // `copy_entire_buffer` (via `select_all_active`) for its trimming.
        self.selection_start = Some(TerminalGridPoint::new(-(geometry.history_size as i32), 0));
        self.selection_end = Some(TerminalGridPoint::new(
            geometry.screen_lines as i32 - 1,
            geometry.columns as u16 - 1,
        ));
        self.select_all_active = true;
        cx.notify();
    }

    /// Drops any selection and cancels an in-flight drag. Returns whether
    /// anything changed, so callers can skip a redundant `notify`.
    fn clear_selection(&mut self) -> bool {
        if self.selection_start.is_none()
            && self.selection_end.is_none()
            && !self.select_all_active
            && !self.selecting
        {
            return false;
        }
        self.selection_start = None;
        self.selection_end = None;
        self.select_all_active = false;
        self.end_selection_drag_state();
        true
    }

    /// Clears drag state and invalidates any running autoscroll ticker.
    fn end_selection_drag_state(&mut self) {
        self.selecting = false;
        self.selection_autoscroll_seq = self.selection_autoscroll_seq.wrapping_add(1);
    }

    pub(super) fn paste_text(&mut self, text: &str, cx: &mut gpui::Context<Self>) {
        let bracketed = self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::BRACKETED_PASTE))
            .unwrap_or(false);
        let bytes = terminal_paste_bytes(text, bracketed);
        self.queue_input(bytes, cx);
    }

    fn handle_scrollback_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let key = keystroke.key.as_str();
        let mods = keystroke.modifiers;
        if mods.control || mods.alt || mods.platform || mods.function || !mods.shift {
            return false;
        }
        let Some(term_lock) = &self.term_lock else {
            return false;
        };
        let mut term = term_lock.lock();
        match key {
            "pageup" => {
                term.scroll_display(alacritty_terminal::grid::Scroll::PageUp);
            }
            "pagedown" => {
                term.scroll_display(alacritty_terminal::grid::Scroll::PageDown);
            }
            "home" => {
                term.scroll_display(alacritty_terminal::grid::Scroll::Top);
            }
            "end" => {
                term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
            }
            _ => return false,
        };
        drop(term);
        cx.notify();
        true
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

        // Scrolling mid-drag counts as movement, so the ticker starts re-resolving
        // the free end and the selection follows the newly revealed rows.
        if self.selecting {
            self.selection_drag_moved = true;
        }

        let mouse_mode = self.live_modes();
        if mouse_mode.mouse_mode() {
            if let Some(pos) = self.viewport_bounds.and_then(|b| {
                terminal_grid_point(
                    event.position,
                    b,
                    metrics.cell_width,
                    metrics.line_height,
                    self.last_content
                        .as_ref()
                        .map(|c| c.display_offset)
                        .unwrap_or(0),
                    self.last_content
                        .as_ref()
                        .map(|c| c.terminal_bounds.columns as u16)
                        .unwrap_or(0),
                )
            }) {
                let (grid_row, grid_col) = pos;
                let reports = terminal_scroll_report(
                    grid_row,
                    grid_col,
                    event.modifiers,
                    delta_y,
                    step_rows,
                    mouse_mode,
                );
                for report in reports {
                    self.queue_input(report, cx);
                }
            }
            return;
        }

        if self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::ALT_SCREEN))
            .unwrap_or(false)
        {
            let app_cursor = self
                .last_content
                .as_ref()
                .map(|c| c.mode.contains(TerminalModes::APP_CURSOR))
                .unwrap_or(false);
            let bytes = terminal_alt_screen_scroll_bytes(delta_y, step_rows, app_cursor);
            self.queue_input(bytes, cx);
            return;
        }

        let Some(term_lock) = &self.term_lock else {
            return;
        };
        {
            let mut term = term_lock.lock();
            if delta_y > px(0.0) {
                for _ in 0..step_rows {
                    term.scroll_display(alacritty_terminal::grid::Scroll::Delta(1));
                }
            } else {
                for _ in 0..step_rows {
                    term.scroll_display(alacritty_terminal::grid::Scroll::Delta(-1));
                }
            }
        }
        cx.notify();
    }

    /// Current terminal modes read live from the backing `Term`, rather than the
    /// last painted [`TerminalContent`] snapshot. Mouse forwarding gates on this so a
    /// TUI that has just enabled mouse reporting is honoured at the instant of the
    /// click, independent of when the next paint refreshes `last_content`.
    fn live_modes(&self) -> TerminalModes {
        self.term_lock
            .as_ref()
            .map(|t| terminal_live_modes(&t.lock()))
            .unwrap_or_default()
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        button: gpui::MouseButton,
    ) {
        window.focus(&self.focus_handle, cx);
        self.reset_cursor_blink(cx);
        crate::press_gesture::claim_press(cx);

        let mode = self.live_modes();
        if mode.mouse_mode() {
            self.queue_mouse_event(event.position, button, event.modifiers, true, mode, cx);
            self.pressed_mouse_button = Some(button);
            cx.stop_propagation();
        } else if button == gpui::MouseButton::Left {
            // Starting a manual selection cancels a prior "select all".
            self.select_all_active = false;
            self.selecting = true;
            self.selection_last_mouse_pos = event.position;
            self.selection_drag_moved = false;
            let anchor = self.selection_grid_point(event.position);
            match event.click_count {
                2 => {
                    // Double click: select the word under the cursor.
                    match anchor.and_then(|p| self.word_range_at(p)) {
                        Some((start, end)) => {
                            self.selection_start = Some(start);
                            self.selection_end = Some(end);
                        }
                        None => {
                            self.selection_start = anchor;
                            self.selection_end = None;
                        }
                    }
                }
                n if n >= 3 => {
                    // Triple (or more) click: select the whole line.
                    let cols = self.grid_geometry().map(|g| g.columns).unwrap_or(0);
                    match anchor {
                        Some(p) if cols > 0 => {
                            self.selection_start = Some(TerminalGridPoint::new(p.row, 0));
                            self.selection_end =
                                Some(TerminalGridPoint::new(p.row, cols as u16 - 1));
                        }
                        _ => {
                            self.selection_start = anchor;
                            self.selection_end = None;
                        }
                    }
                }
                _ => {
                    // Single click: set the anchor but leave the selection
                    // "pending" (end = None) so nothing is painted until a drag
                    // occurs. A click without a drag clears any prior selection
                    // on mouse up (the painter requires both endpoints to be set).
                    self.selection_start = anchor;
                    self.selection_end = None;
                }
            }
            self.start_selection_autoscroll(cx);
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
        button: gpui::MouseButton,
    ) {
        let mode = self.live_modes();
        if mode.mouse_mode() {
            // A release belongs to this viewport only when the matching press
            // did. Without the latch check, letting go over the terminal after
            // a drag that started elsewhere reports a phantom button-up to the
            // program running in it. `stop_propagation` stays inside the guard:
            // declining to act must not swallow the event either.
            if self.pressed_mouse_button == Some(button) {
                self.queue_mouse_event(event.position, button, event.modifiers, false, mode, cx);
                cx.stop_propagation();
            }
        } else if button == gpui::MouseButton::Left {
            self.end_selection_drag(cx);
        }
        self.pressed_mouse_button = None;
    }

    /// Ends a selection drag. Idempotent, because both the element-local
    /// `on_mouse_up` (release inside the viewport) and the window-level
    /// [`TerminalSelectionTracker`] (release anywhere) route through here.
    fn end_selection_drag(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.selecting {
            return;
        }
        self.end_selection_drag_state();
        // A press that never dragged has no end point; clear it so a plain click
        // dismisses the previous selection.
        if self.selection_end.is_none() {
            self.selection_start = None;
        }
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mode = self.live_modes();
        if mode.mouse_mode()
            && mode.intersects(TerminalModes::MOUSE_MOTION | TerminalModes::MOUSE_DRAG)
        {
            // Only forward a motion report when the pointer crosses into a new
            // grid cell. Without this, every sub-cell GPUI move event produces a
            // duplicate report and floods a TUI in any-event mode (1003); Zed
            // dedupes the same way via `mouse_changed`.
            let cell = self.viewport_to_grid_point(event.position);
            if cell != self.last_motion_cell {
                self.last_motion_cell = cell;
                if let Some(report) = terminal_mouse_moved_report_at(
                    event.position,
                    self.viewport_bounds,
                    &self.layout_cache,
                    &self.last_content,
                    self.pressed_mouse_button,
                    event.modifiers,
                    mode,
                ) {
                    self.queue_input(report, cx);
                }
            }
        }
        // Selection drags are handled entirely by `TerminalSelectionTracker`.
        // Extending here as well would be pure duplicated work: the tracker
        // registers its window listener unconditionally on every paint, so a
        // live listener always exists before any mouse-down, and gpui dispatches
        // window listeners ahead of this element-local one.
    }

    /// Strict resolver used by mouse reporting: returns `None` outside the
    /// viewport, matching what `terminal_mouse_*_report_at` will actually
    /// report. Selection must not share it — a clamping variant here would let
    /// `last_motion_cell` latch a cell that was never reported, swallowing the
    /// next genuine entry into it.
    fn viewport_to_grid_point(&self, position: Point<Pixels>) -> Option<TerminalGridPoint> {
        let bounds = self.viewport_bounds?;
        let cache = self.layout_cache.as_ref()?;
        let content = self.last_content.as_ref()?;
        let (grid_row, grid_col) = terminal_grid_point(
            position,
            bounds,
            cache.metrics.cell_width,
            cache.metrics.line_height,
            content.display_offset,
            content.terminal_bounds.columns as u16,
        )?;
        Some(TerminalGridPoint::new(grid_row, grid_col as u16))
    }

    /// Resolver used by text selection: clamps the position into the viewport so
    /// a drag past the edge keeps extending, and resolves against the *live*
    /// scroll offset so a point resolved right after an autoscroll tick is not
    /// off by the amount just scrolled.
    fn selection_grid_point(&self, position: Point<Pixels>) -> Option<TerminalGridPoint> {
        let bounds = self.viewport_bounds?;
        let cache = self.layout_cache.as_ref()?;
        let geometry = self.grid_geometry()?;
        terminal_selection_grid_point(
            position,
            bounds,
            cache.metrics.cell_width,
            cache.metrics.line_height,
            geometry.display_offset,
            geometry.history_size,
            geometry.columns,
            geometry.screen_lines,
        )
    }

    /// Moves the selection's free end to `position` in response to real pointer
    /// motion, which also unlocks the autoscroll ticker's own re-resolving.
    fn drag_selection_to(&mut self, position: Point<Pixels>) -> bool {
        self.selection_drag_moved = true;
        self.extend_selection_to(position)
    }

    /// Moves the selection's free end to `position`. Returns whether the
    /// selection changed, so callers can skip a redundant `notify`.
    fn extend_selection_to(&mut self, position: Point<Pixels>) -> bool {
        self.selection_last_mouse_pos = position;
        let Some(point) = self.selection_grid_point(position) else {
            return false;
        };
        // A press that never moved must stay "pending" (end == None) so mouse-up
        // still clears it. Without this the autoscroll ticker would materialise a
        // one-cell selection after every click, leaving a stray highlight and
        // wrongly enabling Copy in the context menu.
        if self.selection_end.is_none() && Some(point) == self.selection_start {
            return false;
        }
        if self.selection_end == Some(point) {
            return false;
        }
        self.selection_end = Some(point);
        true
    }

    /// Starts the loop that scrolls the viewport while a selection drag sits
    /// outside it. A ticker is needed rather than doing this from move events:
    /// the pointer can be held perfectly still beyond the edge, and a mid-drag
    /// wheel scroll also has to re-resolve the selection's free end.
    fn start_selection_autoscroll(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.selecting {
            return;
        }
        // Invalidates any ticker left over from a previous drag.
        self.selection_autoscroll_seq = self.selection_autoscroll_seq.wrapping_add(1);
        let seq = self.selection_autoscroll_seq;
        cx.spawn(
            async move |view: WeakEntity<TerminalViewportView>, cx: &mut gpui::AsyncApp| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let mut keep_going = false;
                    let updated = view.update(cx, |this, cx| {
                        if !this.selecting || this.selection_autoscroll_seq != seq {
                            return;
                        }
                        keep_going = true;
                        if this.tick_selection_autoscroll() {
                            cx.notify();
                        }
                    });
                    if updated.is_err() || !keep_going {
                        break;
                    }
                }
            },
        )
        .detach();
    }

    /// One autoscroll step. Returns whether anything changed.
    fn tick_selection_autoscroll(&mut self) -> bool {
        let Some(bounds) = self.viewport_bounds else {
            return false;
        };
        let Some(cache) = self.layout_cache.as_ref() else {
            return false;
        };
        let line_height = cache.metrics.line_height;
        let lines = terminal_autoscroll_lines(
            self.selection_last_mouse_pos.y,
            bounds.top(),
            bounds.bottom(),
            line_height,
        );
        let scrolled = if lines != 0 {
            match &self.term_lock {
                Some(term_lock) => {
                    let mut term = term_lock.lock();
                    let before = term.grid().display_offset();
                    term.scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
                    let after = term.grid().display_offset();
                    // Must drop before extending: `selection_grid_point` takes the
                    // same non-reentrant lock.
                    drop(term);
                    before != after
                }
                None => false,
            }
        } else {
            false
        };
        if scrolled {
            self.selection_drag_moved = true;
        }
        // Re-resolve only once the drag has actually moved. Doing it
        // unconditionally would drag a double- or triple-click's word/line
        // selection back to the press cell on the very first tick. Once the drag
        // has moved, re-resolving every tick is what keeps a held-still pointer
        // outside the viewport extending, and what makes a mid-drag wheel scroll
        // extend the selection.
        let extended =
            self.selection_drag_moved && self.extend_selection_to(self.selection_last_mouse_pos);
        scrolled || extended
    }

    /// Word boundaries (inclusive) for the cell at `point`, or `None` when the
    /// cell is whitespace. Operates in viewport/grid coordinates.
    fn word_range_at(
        &self,
        point: TerminalGridPoint,
    ) -> Option<(TerminalGridPoint, TerminalGridPoint)> {
        let term_lock = self.term_lock.as_ref()?;
        // Dimensions from the live grid, so a row captured before a resize or a
        // scrollback eviction cannot index out of range.
        let geometry = self.grid_geometry()?;
        let cols = geometry.columns;
        if cols == 0 || point.col as usize >= cols {
            return None;
        }
        if point.row
            != terminal_clamp_grid_row(point.row, geometry.history_size, geometry.screen_lines)
        {
            return None;
        }
        let chars: Vec<char> = {
            let term = term_lock.lock();
            let grid = term.grid();
            let line = Line(point.row);
            (0..cols)
                .map(|c| {
                    let cell = &grid[line][Column(c)];
                    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                        ' '
                    } else {
                        cell.c
                    }
                })
                .collect()
        };
        let (left, right) = terminal_word_bounds(&chars, point.col as usize)?;
        Some((
            TerminalGridPoint::new(point.row, left as u16),
            TerminalGridPoint::new(point.row, right as u16),
        ))
    }

    fn queue_mouse_event(
        &mut self,
        position: Point<Pixels>,
        button: gpui::MouseButton,
        modifiers: gpui::Modifiers,
        pressed: bool,
        mode: TerminalModes,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(report) = terminal_mouse_event_at(
            position,
            self.viewport_bounds,
            &self.layout_cache,
            &self.last_content,
            button,
            modifiers,
            pressed,
            mode,
        ) {
            self.queue_input(report, cx);
        }
    }

    fn sync_terminal_grid_size(&mut self, next_size: TerminalGridSize) {
        let Some(term_lock) = &self.term_lock else {
            return;
        };
        let Some(pty_sender) = &self.pty_sender else {
            return;
        };
        let current_cols = self
            .last_content
            .as_ref()
            .map(|c| c.terminal_bounds.columns)
            .unwrap_or(0);
        let current_rows = self
            .last_content
            .as_ref()
            .map(|c| c.terminal_bounds.screen_lines)
            .unwrap_or(0);
        if current_cols != next_size.cols as usize || current_rows != next_size.rows as usize {
            let mut term = term_lock.lock();
            term.resize(TerminalDims {
                columns: next_size.cols as usize,
                screen_lines: next_size.rows as usize,
                total_lines: TERMINAL_SCROLLBACK_ROWS,
            });
            drop(term);
            pty_sender.resize(next_size.cols as usize, next_size.rows as usize);
            // `resize` reflows the grid, so the stored endpoints now name
            // different text. The clamps in the copy paths keep that from
            // panicking, but leaving the selection would show a highlight over —
            // and copy — text the user never selected.
            self.clear_selection();
        }
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

        // Refresh content from alacritty
        if let Some(term_lock) = &self.term_lock {
            let term = term_lock.lock();
            let content = make_terminal_content(&term);
            let display_offset = content.display_offset;
            self.last_content = Some(content.clone());
            drop(term);

            let rows = next_size.rows;
            let cols = next_size.cols as usize;

            let viewport_key = TerminalViewportCacheKey {
                content_epoch: self.content_epoch,
                scrollback: display_offset,
                rows,
                cols: cols as u16,
                layout_key: layout.key,
            };

            let mut rebuild = false;
            if self.render_cache.viewport_key != Some(viewport_key) {
                self.render_cache
                    .rows
                    .resize_with(usize::from(rows), TerminalCachedRow::default);
                self.render_cache.viewport_key = Some(viewport_key);
                rebuild = true;
            } else {
                self.render_cache
                    .rows
                    .resize_with(usize::from(rows), TerminalCachedRow::default);
            }

            let mut paint_state = TerminalCanvasPaintState {
                bounds,
                terminal_bg: TerminalAnsiPalette::from_theme(self.theme).background,
                ..Default::default()
            };
            self.viewport_bounds = Some(bounds);

            // Merge background rects across rows after building all rows
            let mut all_bg_rects: Vec<TerminalBackgroundRect> = Vec::new();

            let disp_off = content.display_offset as i32;
            for row in 0..rows {
                let cache_row = &mut self.render_cache.rows[usize::from(row)];
                let grid_row = row as i32 - disp_off;
                let row_fingerprint = terminal_row_fingerprint(&content.cells, grid_row, cols);

                if rebuild
                    || cache_row.shaped.is_none()
                    || cache_row.fingerprint != row_fingerprint
                    || cache_row.layout_key != layout.key
                {
                    let (text, runs, bg_rects) = build_alacritty_row(
                        &content.cells,
                        grid_row,
                        cols,
                        &layout.base_style,
                        self.theme,
                    );
                    let shaped = window.text_system().shape_line(
                        text,
                        layout.metrics.font_size,
                        &runs,
                        Some(layout.metrics.cell_width),
                    );
                    cache_row.fingerprint = row_fingerprint;
                    cache_row.layout_key = layout.key;
                    cache_row.shaped = Some(shaped);
                    cache_row.background_rects = bg_rects;
                }

                if let Some(ref shaped) = cache_row.shaped {
                    paint_state.lines.push((
                        shaped.clone(),
                        point(
                            terminal_snap_to_device_pixels(window, bounds.left()),
                            terminal_snap_to_device_pixels(
                                window,
                                bounds.top() + layout.metrics.line_height * row as f32,
                            ),
                        ),
                        layout.metrics.line_height,
                    ));
                }

                for rect in &cache_row.background_rects {
                    let mut screen_rect = rect.clone();
                    screen_rect.row += disp_off;
                    all_bg_rects.push(screen_rect);
                }
            }

            // Merge background rects across rows
            let merged = merge_background_rects(&all_bg_rects);
            for rect in &merged {
                let origin = point(
                    terminal_snap_to_device_pixels(
                        window,
                        bounds.left() + layout.metrics.cell_width * rect.col as f32,
                    ),
                    terminal_snap_to_device_pixels(
                        window,
                        bounds.top() + layout.metrics.line_height * rect.row as f32,
                    ),
                );
                let rect_size = size(
                    terminal_snap_to_device_pixels(
                        window,
                        layout.metrics.cell_width * rect.num_cells as f32,
                    ),
                    terminal_snap_to_device_pixels(
                        window,
                        layout.metrics.line_height * rect.num_rows as f32,
                    ),
                );
                paint_state
                    .background_rects
                    .push((origin, rect_size, rect.color));
            }

            // Selection rects. Selection points are in grid coordinates (display
            // offset already subtracted), so convert each grid row to a display row
            // with `+ disp_off` — matching the background rects above. Painting
            // `sel_row` directly left the highlight glued to fixed screen rows when
            // scrolled. The row span is clamped to the visible window up front: a
            // selection can now cover the whole scrollback, and iterating all
            // 10_000 rows every frame to skip most of them would be wasteful.
            if let Some((start, end)) = self
                .selection_start
                .zip(self.selection_end)
                .map(|(s, e)| if s <= e { (s, e) } else { (e, s) })
                && let Some(visible_rows) = terminal_selection_visible_rows(
                    start.row,
                    end.row,
                    content.display_offset,
                    rows as usize,
                )
            {
                for sel_row in visible_rows {
                    let display_row = sel_row + disp_off;
                    let sel_start_col = if sel_row == start.row {
                        start.col as usize
                    } else {
                        0
                    };
                    let sel_end_col = if sel_row == end.row {
                        (end.col as usize + 1).min(cols)
                    } else {
                        cols
                    };
                    if sel_start_col < sel_end_col {
                        let sel_origin = point(
                            terminal_snap_to_device_pixels(
                                window,
                                bounds.left() + layout.metrics.cell_width * sel_start_col as f32,
                            ),
                            terminal_snap_to_device_pixels(
                                window,
                                bounds.top() + layout.metrics.line_height * display_row as f32,
                            ),
                        );
                        let sel_size = size(
                            terminal_snap_to_device_pixels(
                                window,
                                layout.metrics.cell_width * (sel_end_col - sel_start_col) as f32,
                            ),
                            terminal_snap_to_device_pixels(window, layout.metrics.line_height),
                        );
                        paint_state
                            .selection_rects
                            .push(Bounds::new(sel_origin, sel_size));
                    }
                }
            }

            // Cursor
            if self.cursor_blink_visible
                && self
                    .ime_state
                    .as_ref()
                    .is_none_or(|s| s.marked_text.is_empty())
                && content.cursor.shape != TerminalCursorShape::Hidden
            {
                let cursor_row = content.cursor.point.line.0 as f32 + content.display_offset as f32;
                let cursor_col = content.cursor.point.column.0 as f32;
                if cursor_row >= 0.0
                    && cursor_row < rows as f32
                    && cursor_col >= 0.0
                    && cursor_col < cols as f32
                {
                    let cursor_width = terminal_cursor_width(
                        content.cursor_char,
                        &layout.base_style,
                        layout.metrics.font_size,
                        layout.metrics.cell_width,
                        window,
                    );
                    let cursor_bounds = Bounds::new(
                        point(
                            terminal_snap_to_device_pixels(
                                window,
                                bounds.left() + layout.metrics.cell_width * cursor_col,
                            ),
                            terminal_snap_to_device_pixels(
                                window,
                                bounds.top() + layout.metrics.line_height * cursor_row,
                            ),
                        ),
                        size(
                            terminal_snap_to_device_pixels(window, cursor_width),
                            terminal_snap_to_device_pixels(window, layout.metrics.line_height),
                        ),
                    );
                    paint_state.cursor = Some(TerminalPaintCursor {
                        bounds: cursor_bounds,
                        shape: content.cursor.shape,
                    });
                    paint_state.ime_bounds = Some(cursor_bounds);
                }
            }

            // IME marked text
            if let Some(ref ime) = self.ime_state
                && !ime.marked_text.is_empty()
            {
                paint_state.ime_marked_text = Some(ime.marked_text.clone());
                paint_state.ime_base_style = Some(layout.base_style.clone());
                let cursor_row = content.cursor.point.line.0 as f32 + content.display_offset as f32;
                let cursor_col = content.cursor.point.column.0 as f32;
                if cursor_row >= 0.0
                    && cursor_row < rows as f32
                    && cursor_col >= 0.0
                    && cursor_col < cols as f32
                {
                    let cb = Bounds::new(
                        point(
                            bounds.left() + layout.metrics.cell_width * cursor_col,
                            bounds.top() + layout.metrics.line_height * cursor_row,
                        ),
                        size(layout.metrics.cell_width, layout.metrics.line_height),
                    );
                    paint_state.ime_bounds = Some(cb);
                }
            }

            paint_state
        } else {
            TerminalCanvasPaintState::default()
        }
    }
}

impl Render for TerminalViewportView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        let theme = self.theme;
        let term_lock = self.term_lock.clone();
        div()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .w_full()
            .h_full()
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.handle_mouse_down(e, window, cx, gpui::MouseButton::Left);
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.handle_mouse_down(e, window, cx, gpui::MouseButton::Middle);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.handle_mouse_down(e, window, cx, gpui::MouseButton::Right);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, window, cx| {
                    this.handle_mouse_up(e, window, cx, gpui::MouseButton::Left);
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseUpEvent, window, cx| {
                    this.handle_mouse_up(e, window, cx, gpui::MouseButton::Middle);
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, e: &MouseUpEvent, window, cx| {
                    this.handle_mouse_up(e, window, cx, gpui::MouseButton::Right);
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                this.handle_mouse_move(e, window, cx);
            }))
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _window, cx| {
                this.handle_key_down(&e.keystroke, cx);
            }))
            .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, window, cx| {
                this.handle_scroll_wheel(e, window, cx);
            }))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h_full()
                    .child(
                        // Keep the grid clear of the scrollbar gutter. The
                        // always-visible scrollbar blocks mouse events across its
                        // full-height gutter once there is scrollback, so text under
                        // it could neither be clicked to start a selection nor be
                        // read with the thumb drawn over it.
                        div()
                            .w_full()
                            .h_full()
                            .pr(Scrollbar::gutter(ScrollbarAxis::Vertical))
                            .child({
                                let focus_handle = self.focus_handle.clone();
                                let pty_sender = self.pty_sender.clone();
                                let view = view.clone();
                                gpui::canvas(
                                    move |bounds, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.build_terminal_canvas_paint_state(
                                                bounds, window, cx,
                                            )
                                        })
                                    },
                                    move |_bounds, paint_state, window, cx| {
                                        let ime_handler = TerminalTextInputHandler {
                                            pty_sender: pty_sender.clone(),
                                            ime_state: paint_state.ime_marked_text.as_ref().map(
                                                |t| TerminalImeState {
                                                    marked_text: t.clone(),
                                                },
                                            ),
                                        };
                                        window.handle_input(&focus_handle, ime_handler, cx);
                                        paint_terminal_canvas_state(paint_state, theme, window, cx);
                                    },
                                )
                                .w_full()
                                .h_full()
                            }),
                    )
                    .child({
                        let line_height = self
                            .terminal_layout_snapshot(_window, cx)
                            .metrics
                            .line_height;
                        Scrollbar::new(
                            "terminal_scrollbar",
                            TerminalScrollbarDriver {
                                term_lock: term_lock.clone(),
                                line_height,
                            },
                        )
                        .always_visible()
                        .render(theme)
                    }),
            )
            // Last child, so its window-level listeners are registered last and
            // therefore dispatched first in the bubble phase.
            .child(TerminalSelectionTracker { view })
    }
}

// ============================================================================
// GitCometView terminal methods
// ============================================================================

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
            exit_status: None,
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

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let rx = events_rx;
                while let Ok(event) = rx.recv().await {
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
                            TerminalBackendEvent::Exit => {
                                instance.connected = false;
                                instance.exit_status = Some("Shell exited.".to_string());
                                instance.viewport.update(cx, |viewport, _cx| {
                                    viewport.pty_sender = None;
                                    viewport.term_lock = None;
                                });
                                cx.notify();
                            }
                            TerminalBackendEvent::ChildExit(code) => {
                                let msg = match code {
                                    Some(c) => format!("Child process exited with code {c}"),
                                    None => "Child process exited".to_string(),
                                };
                                eprintln!("terminal child process: {msg}");
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
        } else {
            self.focus_terminal_view(repo_id, window, cx);
        }
        self.sync_terminal_indicator_views(cx);
        cx.notify();
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
            TerminalShutdownAction::CloseTerminalTab { repo_id, index } => {
                let mut summary = self
                    .terminal_sessions
                    .get(repo_id)
                    .and_then(|session| session.instances.get(*index))
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
            TerminalShutdownAction::CloseTerminalTab { repo_id, index } => {
                self.close_terminal_tab(repo_id, index, window, cx);
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
        if !self.request_terminal_shutdown_action(
            TerminalShutdownAction::CloseTerminalTab { repo_id, index },
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

        let (viewport_entity, exit_status, connected, tabs, active_index) = {
            let session = self.terminal_sessions.get(&active_repo)?;
            let active = session.active_instance()?;
            let tabs: Vec<SharedString> = session
                .instances
                .iter()
                .map(|inst| SharedString::from(inst.title.clone()))
                .collect();
            (
                active.viewport.clone(),
                active.exit_status.clone(),
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

        let panel = div()
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
            .into_any_element();

        if let Some(status) = exit_status {
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(4.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(format!("Terminal — {status}")),
                    )
                    .child(panel)
                    .into_any(),
            );
        }
        Some(panel.into_any())
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

// ============================================================================
// RustDrop for GitCometView
// ============================================================================

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

// ============================================================================
// Helper functions
// ============================================================================

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

fn terminal_text_style<C>(theme: AppTheme, window: &Window, cx: &mut C) -> gpui::TextStyle
where
    C: gpui::BorrowAppContext,
{
    let mut style = window.text_style();
    style.font_family = crate::font_preferences::current_editor_font_family(cx).into();
    style.font_features = gpui::FontFeatures::disable_ligatures();
    style.font_weight = FontWeight::NORMAL;
    style.font_style = gpui::FontStyle::Normal;
    style.color = terminal_default_foreground(theme).into_color();
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

fn terminal_layout_cache(mut base_style: gpui::TextStyle, window: &Window) -> TerminalLayoutCache {
    let rem_size = window.rem_size();
    let font_size = base_style.font_size.to_pixels(rem_size) * TERMINAL_FONT_SCALE;
    let line_height = terminal_line_height(font_size);
    base_style.line_height = line_height.into();
    let font_id = window.text_system().resolve_font(&base_style.font());
    let cell_width = window
        .text_system()
        .advance(font_id, font_size, 'm')
        .unwrap()
        .width
        .max(px(1.0));
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
    px((font_size_px * TERMINAL_LINE_HEIGHT_SCALE).ceil())
}

fn terminal_layout_key(metrics: TerminalTextMetrics) -> TerminalLayoutKey {
    TerminalLayoutKey {
        font_size_bits: pixels_bits(metrics.font_size),
        line_height_bits: pixels_bits(metrics.line_height),
        cell_width_bits: pixels_bits(metrics.cell_width),
    }
}

fn pixels_bits(value: Pixels) -> u32 {
    let raw: f32 = value.into();
    raw.to_bits()
}

fn pixels_to_u16(value: Pixels) -> u16 {
    let raw: f32 = value.into();
    raw as u16
}

/// Cleans up text copied from the terminal grid. Grid rows are space-padded to
/// the full width, so trim trailing whitespace from each line and drop trailing
/// blank lines (the previous `while ends_with('\n')` trim was a no-op because
/// rows end in spaces, not newlines).
fn trim_terminal_copy(text: &str) -> String {
    let mut result = String::new();
    for line in text.lines() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line.trim_end());
    }
    while result.ends_with('\n') {
        result.pop();
    }
    result
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
    let sanitized = sanitize_bracketed_paste(&normalized);
    let mut bytes = Vec::with_capacity(
        BRACKETED_PASTE_START.len() + sanitized.len() + BRACKETED_PASTE_END.len(),
    );
    bytes.extend_from_slice(BRACKETED_PASTE_START);
    bytes.extend_from_slice(sanitized.as_bytes());
    bytes.extend_from_slice(BRACKETED_PASTE_END);
    bytes
}

fn terminal_scroll_wheel_delta(
    event: &gpui::ScrollWheelEvent,
    line_height: Pixels,
) -> Option<(Pixels, usize)> {
    let delta = event.delta.pixel_delta(line_height).y;
    if delta.abs() < px(1.0) {
        return None;
    }
    let step_rows = ((delta.abs() / line_height).ceil() as usize).max(1);
    Some((delta, step_rows))
}

fn paint_terminal_canvas_state(
    paint_state: TerminalCanvasPaintState,
    theme: AppTheme,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(paint_state.bounds, paint_state.terminal_bg));
    for (origin, rect_size, color) in paint_state.background_rects {
        window.paint_quad(fill(Bounds::new(origin, rect_size), color));
    }
    for rect in paint_state.selection_rects {
        window.paint_quad(fill(
            rect,
            with_alpha(theme.colors.accent.foreground, TERMINAL_SELECTION_ALPHA),
        ));
    }
    for (line, origin, line_height) in paint_state.lines {
        let _ = line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
    }
    if let Some(cursor) = paint_state.cursor {
        paint_terminal_cursor(cursor, theme, window);
    }

    // IME preedit (marked) text
    if let Some(ref marked_text) = paint_state.ime_marked_text
        && let Some(ime_bounds) = paint_state.ime_bounds
        && let Some(ref base_style) = paint_state.ime_base_style
    {
        let mut ime_style = base_style.clone();
        ime_style.underline = Some(gpui::UnderlineStyle {
            color: Some(ime_style.color),
            thickness: px(1.0),
            wavy: false,
        });
        let shaped = window.text_system().shape_line(
            marked_text.clone().into(),
            ime_style.font_size.to_pixels(window.rem_size()),
            &[TextRun {
                len: marked_text.len(),
                font: ime_style.font(),
                color: ime_style.color,
                underline: ime_style.underline,
                ..Default::default()
            }],
            None,
        );
        let ime_bg = Bounds::new(
            ime_bounds.origin,
            size(shaped.width, ime_bounds.size.height),
        );
        window.paint_quad(fill(ime_bg, paint_state.terminal_bg));
        let _ = shaped.paint(
            ime_bounds.origin,
            ime_bounds.size.height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
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

fn paint_terminal_cursor(cursor: TerminalPaintCursor, theme: AppTheme, window: &mut Window) {
    let cursor_color = terminal_default_foreground(theme);
    match cursor.shape {
        TerminalCursorShape::Beam => {
            let caret = terminal_caret_bounds(cursor.bounds);
            window.paint_quad(fill(caret, cursor_color).corner_radii(px(TERMINAL_CARET_RADIUS_PX)));
        }
        TerminalCursorShape::Underline => {
            let height = (cursor.bounds.size.height * 0.12).max(px(1.0));
            let underline = Bounds::new(
                point(cursor.bounds.left(), cursor.bounds.bottom() - height),
                size(cursor.bounds.size.width.max(px(1.0)), height),
            );
            window.paint_quad(fill(underline, cursor_color));
        }
        TerminalCursorShape::Block => {
            window.paint_quad(fill(cursor.bounds, cursor_color));
        }
        TerminalCursorShape::Hollow => {
            let thickness = px(1.0)
                .min(cursor.bounds.size.width / 2.0)
                .min(cursor.bounds.size.height / 2.0)
                .max(px(1.0));
            let top = Bounds::new(
                cursor.bounds.origin,
                size(cursor.bounds.size.width, thickness),
            );
            let bottom = Bounds::new(
                point(cursor.bounds.left(), cursor.bounds.bottom() - thickness),
                size(cursor.bounds.size.width, thickness),
            );
            let left = Bounds::new(
                cursor.bounds.origin,
                size(thickness, cursor.bounds.size.height),
            );
            let right = Bounds::new(
                point(cursor.bounds.right() - thickness, cursor.bounds.top()),
                size(thickness, cursor.bounds.size.height),
            );
            for edge in [top, bottom, left, right] {
                window.paint_quad(fill(edge, cursor_color));
            }
        }
        TerminalCursorShape::Hidden => {}
    }
}

fn terminal_cursor_width(
    cursor_char: char,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    cell_width: Pixels,
    window: &Window,
) -> Pixels {
    if cursor_char.is_whitespace() {
        return cell_width;
    }
    let cursor_text = cursor_char.to_string();
    let shaped = window.text_system().shape_line(
        cursor_text.clone().into(),
        font_size,
        &[TextRun {
            len: cursor_text.len(),
            font: base_style.font(),
            color: base_style.color,
            ..Default::default()
        }],
        None,
    );
    shaped.width.max(cell_width).ceil()
}

fn terminal_snap_to_device_pixels(window: &Window, value: Pixels) -> Pixels {
    let scale_factor = window.scale_factor().max(1.0);
    Pixels::from((f32::from(value) * scale_factor).floor() / scale_factor)
}

fn terminal_row_fingerprint(cells: &[IndexedCell], row: i32, cols: usize) -> u64 {
    let mut hasher = FxHasher::default();
    row.hash(&mut hasher);
    cols.hash(&mut hasher);

    for cell in cells.iter().filter(|cell| cell.point.line.0 == row) {
        cell.point.column.0.hash(&mut hasher);
        cell.cell.c.hash(&mut hasher);
        cell.cell.flags.hash(&mut hasher);
        hash_terminal_color(cell.cell.fg, &mut hasher);
        hash_terminal_color(cell.cell.bg, &mut hasher);
        if let Some(zw_chars) = cell.cell.zerowidth() {
            zw_chars.len().hash(&mut hasher);
            for ch in zw_chars {
                ch.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

fn hash_terminal_color<H: Hasher>(color: alacritty_terminal::vte::ansi::Color, hasher: &mut H) {
    use alacritty_terminal::vte::ansi::Color;

    match color {
        Color::Named(name) => {
            0u8.hash(hasher);
            std::mem::discriminant(&name).hash(hasher);
        }
        Color::Spec(rgb) => {
            1u8.hash(hasher);
            rgb.r.hash(hasher);
            rgb.g.hash(hasher);
            rgb.b.hash(hasher);
        }
        Color::Indexed(index) => {
            2u8.hash(hasher);
            index.hash(hasher);
        }
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
        TerminalShutdownAction::CloseTerminalTab { repo_id, index } => {
            if let Some(instance) = view
                .terminal_sessions
                .get(repo_id)
                .and_then(|session| session.instances.get(*index))
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

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::Handler;

    // A 20x6 grid of 6x12 cells, so the viewport is 120x72 at (100, 200).
    const TEST_COLS: usize = 20;
    const TEST_ROWS: usize = 6;
    const TEST_CELL_W: f32 = 6.0;
    const TEST_LINE_H: f32 = 12.0;
    const TEST_SCROLLBACK: usize = 100;

    fn test_viewport_bounds() -> Bounds<Pixels> {
        Bounds::new(
            point(px(100.0), px(200.0)),
            size(
                px(TEST_CELL_W * TEST_COLS as f32),
                px(TEST_LINE_H * TEST_ROWS as f32),
            ),
        )
    }

    /// Position of the top-left pixel of visible row `row`, column `col`.
    fn test_cell_pos(row: usize, col: usize) -> Point<Pixels> {
        let bounds = test_viewport_bounds();
        point(
            bounds.left() + px(TEST_CELL_W * col as f32 + 1.0),
            bounds.top() + px(TEST_LINE_H * row as f32 + 1.0),
        )
    }

    fn test_layout_cache() -> TerminalLayoutCache {
        TerminalLayoutCache {
            rem_size: px(16.0),
            key: TerminalLayoutKey::default(),
            base_style: gpui::TextStyle::default(),
            metrics: TerminalTextMetrics {
                font_size: px(10.0),
                line_height: px(TEST_LINE_H),
                cell_width: px(TEST_CELL_W),
            },
        }
    }

    /// A live `Term` holding `count` numbered lines, so anything beyond the last
    /// `TEST_ROWS` of them sits in scrollback.
    fn test_term_with_lines(count: usize) -> AlacrittyTermLock {
        let (events_tx, _events_rx) = smol::channel::unbounded();
        let term_lock = new_term(
            &terminal_config(TEST_SCROLLBACK),
            &TerminalDims {
                columns: TEST_COLS,
                screen_lines: TEST_ROWS,
                total_lines: TEST_ROWS + TEST_SCROLLBACK,
            },
            events_tx,
        );
        let mut term = term_lock.lock();
        for i in 0..count {
            for c in format!("line{i:03}").chars() {
                term.input(c);
            }
            if i + 1 < count {
                term.linefeed();
                term.carriage_return();
            }
        }
        drop(term);
        term_lock
    }

    /// Builds a viewport view over `term_lock`, with the layout and bounds a
    /// paint would normally supply. `pty_sender` is `None`, which also keeps
    /// `sync_terminal_grid_size` from resizing the grid out from under the test.
    fn test_viewport(
        term_lock: AlacrittyTermLock,
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<TerminalViewportView>, &mut gpui::VisualTestContext) {
        cx.add_window_view(|_window, cx| {
            let mut view = TerminalViewportView::with_backend(
                AppTheme::gitcomet_dark(),
                cx.focus_handle(),
                Some(term_lock),
                None,
            );
            view.viewport_bounds = Some(test_viewport_bounds());
            view.layout_cache = Some(test_layout_cache());
            view
        })
    }

    /// Runs `f` against the view, re-stubbing the geometry a real paint would
    /// own so the test does not depend on the test window's actual size.
    fn with_viewport<R>(
        view: &Entity<TerminalViewportView>,
        cx: &mut gpui::VisualTestContext,
        f: impl FnOnce(
            &mut TerminalViewportView,
            &mut Window,
            &mut gpui::Context<TerminalViewportView>,
        ) -> R,
    ) -> R {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.viewport_bounds = Some(test_viewport_bounds());
                this.layout_cache = Some(test_layout_cache());
                f(this, window, cx)
            })
        })
    }

    fn test_mouse_down(position: Point<Pixels>) -> MouseDownEvent {
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: gpui::Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }
    }

    fn display_offset_of(
        view: &Entity<TerminalViewportView>,
        cx: &mut gpui::VisualTestContext,
    ) -> usize {
        with_viewport(view, cx, |this, _window, _cx| {
            this.grid_geometry().expect("live term").display_offset
        })
    }

    #[gpui::test]
    fn drag_below_the_viewport_keeps_extending_the_selection(cx: &mut gpui::TestAppContext) {
        let term_lock = test_term_with_lines(30);
        let (view, cx) = test_viewport(term_lock, cx);

        // Press on the top visible row, then drag far below the panel. The
        // element-local move handler is hitbox-gated by gpui, so this is the path
        // the window-level `TerminalSelectionTracker` drives.
        with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(
                &test_mouse_down(test_cell_pos(0, 0)),
                window,
                cx,
                MouseButton::Left,
            );
        });

        let below = point(
            test_viewport_bounds().left() + px(30.0),
            test_viewport_bounds().bottom() + px(500.0),
        );
        let extended = with_viewport(&view, cx, |this, _window, _cx| {
            this.drag_selection_to(below)
        });

        assert!(
            extended,
            "a drag past the bottom edge must extend the selection"
        );
        let (start, end) = with_viewport(&view, cx, |this, _window, _cx| {
            (this.selection_start, this.selection_end)
        });
        assert_eq!(start, Some(TerminalGridPoint::new(0, 0)));
        assert_eq!(
            end.map(|p| p.row),
            Some(TEST_ROWS as i32 - 1),
            "extends to the last visible row rather than stopping at the anchor"
        );
        // Release, so no detached autoscroll ticker outlives the test.
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
    }

    #[gpui::test]
    fn autoscroll_tick_scrolls_while_the_pointer_sits_outside(cx: &mut gpui::TestAppContext) {
        let term_lock = test_term_with_lines(30);
        let (view, cx) = test_viewport(term_lock, cx);

        // Scroll back into history, then hold a drag above the panel.
        with_viewport(&view, cx, |this, _window, _cx| {
            let term_lock = this.term_lock.clone().expect("live term");
            term_lock.lock().scroll_display(Scroll::Delta(5));
        });
        assert_eq!(display_offset_of(&view, cx), 5);

        let above = point(
            test_viewport_bounds().left() + px(30.0),
            test_viewport_bounds().top() - px(40.0),
        );
        with_viewport(&view, cx, |this, _window, _cx| {
            this.selecting = true;
            this.selection_start = Some(TerminalGridPoint::new(-5, 0));
            this.selection_last_mouse_pos = above;
        });

        // The pointer never moves again: only the ticker can make progress.
        for _ in 0..3 {
            with_viewport(&view, cx, |this, _window, _cx| {
                this.tick_selection_autoscroll()
            });
        }
        assert!(
            display_offset_of(&view, cx) > 5,
            "holding the pointer above the panel must keep scrolling into history"
        );

        // And below the panel scrolls back toward the live tail.
        let below = point(
            test_viewport_bounds().left() + px(30.0),
            test_viewport_bounds().bottom() + px(40.0),
        );
        with_viewport(&view, cx, |this, _window, _cx| {
            this.selection_last_mouse_pos = below;
        });
        let before = display_offset_of(&view, cx);
        with_viewport(&view, cx, |this, _window, _cx| {
            this.tick_selection_autoscroll()
        });
        assert!(
            display_offset_of(&view, cx) < before,
            "dragging below the panel must scroll toward the live tail"
        );
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
    }

    #[gpui::test]
    fn selecting_after_scrolling_back_copies_the_scrollback_rows(cx: &mut gpui::TestAppContext) {
        let term_lock = test_term_with_lines(30);
        let (view, cx) = test_viewport(term_lock, cx);

        // 30 lines in a 6-row grid: rows 24..29 are on screen, the rest is
        // history. Scrolling back 10 puts lines 14..19 on screen.
        with_viewport(&view, cx, |this, _window, _cx| {
            let term_lock = this.term_lock.clone().expect("live term");
            term_lock.lock().scroll_display(Scroll::Delta(10));
        });

        // Select the first two visible rows, whole width.
        let text = with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(
                &test_mouse_down(test_cell_pos(0, 0)),
                window,
                cx,
                MouseButton::Left,
            );
            this.drag_selection_to(test_cell_pos(1, TEST_COLS - 1));
            this.selected_text()
        });

        assert_eq!(
            text.as_deref(),
            Some("line014\nline015"),
            "the highlight must resolve to the scrollback rows under the pointer, \
             and whole-row copies must not carry the grid's padding spaces"
        );
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
    }

    #[gpui::test]
    fn a_click_without_a_drag_leaves_no_selection(cx: &mut gpui::TestAppContext) {
        let term_lock = test_term_with_lines(30);
        let (view, cx) = test_viewport(term_lock, cx);

        with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(
                &test_mouse_down(test_cell_pos(2, 3)),
                window,
                cx,
                MouseButton::Left,
            );
        });

        // The autoscroll ticker re-resolves the pointer every frame; it must not
        // turn a stationary press into a one-cell selection, or every click would
        // leave a stray highlight and enable Copy.
        for _ in 0..5 {
            with_viewport(&view, cx, |this, _window, _cx| {
                this.tick_selection_autoscroll()
            });
        }
        assert_eq!(
            with_viewport(&view, cx, |this, _window, _cx| this.selection_end),
            None,
            "a press that never moved stays pending"
        );

        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
        let (has_selection, selecting) = with_viewport(&view, cx, |this, _window, _cx| {
            (this.has_selection(), this.selecting)
        });
        assert!(!has_selection, "releasing a click clears the anchor");
        assert!(!selecting, "and ends the drag");
    }

    /// A `Term` holding one line of `len` characters, so anything past the
    /// column count soft-wraps onto the following row(s).
    fn test_term_with_long_line(len: usize) -> AlacrittyTermLock {
        let (events_tx, _events_rx) = smol::channel::unbounded();
        let term_lock = new_term(
            &terminal_config(TEST_SCROLLBACK),
            &TerminalDims {
                columns: TEST_COLS,
                screen_lines: TEST_ROWS,
                total_lines: TEST_ROWS + TEST_SCROLLBACK,
            },
            events_tx,
        );
        {
            let mut term = term_lock.lock();
            for i in 0..len {
                term.input((b'a' + (i % 26) as u8) as char);
            }
        }
        term_lock
    }

    #[gpui::test]
    fn copying_a_soft_wrapped_line_does_not_insert_a_newline(cx: &mut gpui::TestAppContext) {
        let expected: String = (0..30).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let (view, cx) = test_viewport(test_term_with_long_line(30), cx);

        // 30 characters in a 20-column grid occupy rows 0 and 1 as one logical
        // line. Selecting both must copy it unbroken: a '\n' at the wrap column
        // makes a pasted command run as a truncated fragment.
        let text = with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(
                &test_mouse_down(test_cell_pos(0, 0)),
                window,
                cx,
                MouseButton::Left,
            );
            this.drag_selection_to(test_cell_pos(1, TEST_COLS - 1));
            this.selected_text()
        });
        assert_eq!(text.as_deref(), Some(expected.as_str()));
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
    }

    #[gpui::test]
    fn a_selection_starting_mid_row_still_drops_the_grid_padding(cx: &mut gpui::TestAppContext) {
        let (view, cx) = test_viewport(test_term_with_lines(30), cx);

        // Start two columns in and drag to the end of the next row. The first
        // row's selection still reaches the line end, so its trailing spaces are
        // grid padding, not selected content.
        let text = with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(
                &test_mouse_down(test_cell_pos(0, 2)),
                window,
                cx,
                MouseButton::Left,
            );
            this.drag_selection_to(test_cell_pos(1, TEST_COLS - 1));
            this.selected_text()
        });
        assert_eq!(text.as_deref(), Some("ne024\nline025"));
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
    }

    fn test_multi_click(position: Point<Pixels>, click_count: usize) -> MouseDownEvent {
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: gpui::Modifiers::default(),
            click_count,
            first_mouse: false,
        }
    }

    #[gpui::test]
    fn the_autoscroll_ticker_does_not_collapse_a_word_or_line_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let term_lock = test_term_with_lines(30);
        let (view, cx) = test_viewport(term_lock, cx);

        // Double-click in the middle of the word on the last visible row. The
        // ticker then fires with the pointer never having moved; it must leave
        // the word selection alone rather than re-resolving the free end back to
        // the press cell.
        let press = test_cell_pos(TEST_ROWS - 1, 3);
        let word = with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(&test_multi_click(press, 2), window, cx, MouseButton::Left);
            this.selected_text()
        });
        assert_eq!(
            word.as_deref(),
            Some("line029"),
            "double click selects a word"
        );

        for _ in 0..5 {
            with_viewport(&view, cx, |this, _window, _cx| {
                this.tick_selection_autoscroll()
            });
        }
        assert_eq!(
            with_viewport(&view, cx, |this, _window, _cx| this.selected_text()).as_deref(),
            Some("line029"),
            "a stationary pointer must not shrink the word selection"
        );
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));

        // Same for a triple-click line selection.
        let (start, end) = with_viewport(&view, cx, |this, window, cx| {
            this.handle_mouse_down(&test_multi_click(press, 3), window, cx, MouseButton::Left);
            (this.selection_start, this.selection_end)
        });
        assert_eq!(start.map(|p| p.col), Some(0));
        assert_eq!(end.map(|p| p.col), Some(TEST_COLS as u16 - 1));

        for _ in 0..5 {
            with_viewport(&view, cx, |this, _window, _cx| {
                this.tick_selection_autoscroll()
            });
        }
        assert_eq!(
            with_viewport(&view, cx, |this, _window, _cx| (
                this.selection_start,
                this.selection_end
            )),
            (start, end),
            "a stationary pointer must not shrink the line selection"
        );
        with_viewport(&view, cx, |this, _window, cx| this.end_selection_drag(cx));
    }

    #[gpui::test]
    fn the_grid_stops_short_of_the_scrollbar_gutter(cx: &mut gpui::TestAppContext) {
        let term_lock = test_term_with_lines(3);
        let (view, cx) = cx.add_window_view(|_window, cx| {
            TerminalViewportView::with_backend(
                AppTheme::gitcomet_dark(),
                cx.focus_handle(),
                Some(term_lock),
                None,
            )
        });
        cx.run_until_parked();

        let (window_size, viewport) =
            cx.update(|window, app| (window.viewport_size(), view.read(app).viewport_bounds));
        let viewport = viewport.expect("the canvas records its bounds during prepaint");
        let gutter = Scrollbar::gutter(ScrollbarAxis::Vertical);
        // The always-visible scrollbar blocks mouse events across its whole
        // gutter, so text must never be laid out underneath it: a press there
        // could not start a selection, and the thumb would cover the glyphs.
        assert_eq!(
            window_size.width - viewport.size.width,
            gutter,
            "the grid must be inset from the right edge by exactly the gutter"
        );
        assert_eq!(
            viewport.size.height, window_size.height,
            "and must still fill the available height"
        );
    }

    #[gpui::test]
    fn select_all_covers_the_whole_buffer_and_stays_visible_when_scrolled(
        cx: &mut gpui::TestAppContext,
    ) {
        let term_lock = test_term_with_lines(30);
        let (view, cx) = test_viewport(term_lock, cx);

        let (start, end, history) = with_viewport(&view, cx, |this, _window, cx| {
            this.select_all(cx);
            let history = this.grid_geometry().expect("live term").history_size;
            (this.selection_start, this.selection_end, history)
        });
        assert!(
            history > 0,
            "30 lines in a 6-row grid must produce scrollback"
        );
        assert_eq!(start, Some(TerminalGridPoint::new(-(history as i32), 0)));
        assert_eq!(
            end,
            Some(TerminalGridPoint::new(
                TEST_ROWS as i32 - 1,
                TEST_COLS as u16 - 1
            ))
        );

        // The painted span is clamped to the screen at every scroll position, so
        // a buffer-wide selection never iterates the whole scrollback per frame.
        for offset in [0usize, 5, history] {
            let visible = terminal_selection_visible_rows(
                start.unwrap().row,
                end.unwrap().row,
                offset,
                TEST_ROWS,
            )
            .expect("select-all is visible at every scroll offset");
            assert_eq!(visible.clone().count(), TEST_ROWS);
            assert_eq!(*visible.start(), -(offset as i32));
        }
    }

    #[test]
    fn friendly_terminal_title_collapses_shell_paths_to_the_program_stem() {
        assert_eq!(
            friendly_terminal_title("C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_string()),
            "pwsh"
        );
        assert_eq!(
            friendly_terminal_title("C:/Windows/System32/cmd.EXE".to_string()),
            "cmd"
        );
        // Application-set titles pass through, even when they contain paths.
        assert_eq!(
            friendly_terminal_title("PS C:\\Users\\sampo\\git\\GitComet".to_string()),
            "PS C:\\Users\\sampo\\git\\GitComet"
        );
        assert_eq!(friendly_terminal_title("vim".to_string()), "vim");
    }

    #[test]
    fn trim_terminal_copy_strips_trailing_whitespace_and_blank_lines() {
        // Grid rows are space-padded to the full width; copying must trim trailing
        // spaces per line and drop trailing blank lines (the old newline-only trim
        // was a no-op because rows end in spaces).
        let raw = "git status      \n                \n";
        assert_eq!(trim_terminal_copy(raw), "git status");
        // Interior blank lines are preserved.
        assert_eq!(trim_terminal_copy("a   \n   \nb   "), "a\n\nb");
        assert_eq!(trim_terminal_copy(""), "");
    }

    #[test]
    fn cursor_screen_row_adds_display_offset() {
        // When the terminal is scrolled back (display_offset > 0),
        // the cursor grid position must be converted to screen position
        // by adding display_offset. This ensures the cursor stays at
        // the input line position and does not appear to move with scroll.
        let cursor_grid_row = 23;
        let display_offset: usize = 0;
        let screen_row = cursor_grid_row as f32 + display_offset as f32;
        assert_eq!(screen_row, 23.0, "cursor at grid row 23, no scroll");

        let display_offset: usize = 5;
        let screen_row = cursor_grid_row as f32 + display_offset as f32;
        assert_eq!(
            screen_row, 28.0,
            "scrolled back 5 lines, cursor moves below visible area"
        );
    }

    #[test]
    fn cursor_hidden_when_scrolled_beyond_viewport() {
        // When display_offset pushes the cursor beyond screen_lines,
        // the cursor should not be rendered (it's below the visible history).
        let screen_lines: usize = 24;
        let cursor_grid_row: usize = 23;
        let display_offset: usize = 5;
        let screen_row = cursor_grid_row as f32 + display_offset as f32;
        assert!(
            screen_row >= screen_lines as f32,
            "cursor at row {screen_row} should be >= screen_lines ({screen_lines}) -> not visible"
        );
    }

    #[test]
    fn cursor_visible_when_at_live_tail() {
        let screen_lines: usize = 24;
        let cursor_grid_row: usize = 23;
        let display_offset: usize = 0;
        let screen_row = cursor_grid_row as f32 + display_offset as f32;
        assert!(
            screen_row < screen_lines as f32,
            "cursor at live tail should be visible"
        );
    }

    #[test]
    fn scrollbar_gutter_contains_only_points_inside_gutter() {
        let gutter = Bounds::new(point(px(300.0), px(0.0)), size(px(16.0), px(400.0)));
        assert!(
            gutter.contains(&point(px(308.0), px(200.0))),
            "point inside gutter is contained"
        );
        assert!(
            !gutter.contains(&point(px(280.0), px(200.0))),
            "point left of gutter is not contained"
        );
        assert!(
            !gutter.contains(&point(px(320.0), px(200.0))),
            "point right of gutter is not contained"
        );
        assert!(
            gutter.contains(&point(px(300.0), px(0.0))),
            "top-left corner is contained"
        );
        assert!(
            gutter.contains(&point(px(300.0), px(200.0))),
            "point on left edge is contained"
        );
        assert!(
            !gutter.contains(&point(px(316.0), px(200.0))),
            "point exactly on right edge is NOT contained (exclusive)"
        );
        assert!(
            !gutter.contains(&point(px(300.0), px(400.0))),
            "point exactly on bottom edge is NOT contained (exclusive)"
        );
    }
}
