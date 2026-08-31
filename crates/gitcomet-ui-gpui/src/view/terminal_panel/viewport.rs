use super::painting::{
    TerminalCanvasPaintState, TerminalGridGeometry, TerminalPaintCursor,
    paint_terminal_canvas_state, terminal_cursor_width, terminal_row_fingerprint,
    terminal_snap_to_device_pixels,
};
use super::*;
use crate::kit::ScrollbarDriver;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use palette::IntoColor;

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
    pub(super) fn with_backend(
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

    pub(crate) fn invalidate_layout(&mut self, cx: &mut gpui::Context<Self>) {
        self.layout_cache = None;
        self.render_cache = TerminalRenderCache::default();
        cx.notify();
    }

    pub(crate) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.layout_cache = None;
        self.render_cache = TerminalRenderCache::default();
        cx.notify();
    }

    pub(super) fn terminal_layout_snapshot(
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

    pub(super) fn connected(&self) -> bool {
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

    pub(super) fn handle_focus_gained(&mut self, cx: &mut gpui::Context<Self>) {
        let has_focus_mode = self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::FOCUS_IN_OUT))
            .unwrap_or(false);
        if has_focus_mode {
            self.queue_input(b"\x1b[I".to_vec(), cx);
        }
    }

    pub(super) fn handle_focus_lost(&mut self, cx: &mut gpui::Context<Self>) {
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

    pub(super) fn advance_cursor_blink(&mut self, blink_seq: u64, cx: &mut gpui::Context<Self>) {
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

    pub(super) fn queue_input(&mut self, bytes: Vec<u8>, cx: &mut gpui::Context<Self>) {
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
    pub(super) fn handle_key_down(
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

    pub(super) fn perform_clipboard_action(
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

    pub(super) fn copy_grid_range(
        &self,
        start: TerminalGridPoint,
        end: TerminalGridPoint,
    ) -> String {
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

    pub(super) fn copy_visible_screen(&self) -> String {
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
    pub(super) fn copy_entire_buffer(&self) -> String {
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
    pub(super) fn grid_geometry(&self) -> Option<TerminalGridGeometry> {
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
    pub(super) fn clear_selection(&mut self) -> bool {
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

    pub(super) fn handle_scrollback_key(
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

    pub(super) fn handle_scroll_wheel(
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
    pub(super) fn live_modes(&self) -> TerminalModes {
        self.term_lock
            .as_ref()
            .map(|t| terminal_live_modes(&t.lock()))
            .unwrap_or_default()
    }

    pub(super) fn handle_mouse_down(
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

    pub(super) fn handle_mouse_up(
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
    pub(super) fn end_selection_drag(&mut self, cx: &mut gpui::Context<Self>) {
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

    pub(super) fn handle_mouse_move(
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
    pub(super) fn viewport_to_grid_point(
        &self,
        position: Point<Pixels>,
    ) -> Option<TerminalGridPoint> {
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
    pub(super) fn selection_grid_point(
        &self,
        position: Point<Pixels>,
    ) -> Option<TerminalGridPoint> {
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
    pub(super) fn drag_selection_to(&mut self, position: Point<Pixels>) -> bool {
        self.selection_drag_moved = true;
        self.extend_selection_to(position)
    }

    /// Moves the selection's free end to `position`. Returns whether the
    /// selection changed, so callers can skip a redundant `notify`.
    pub(super) fn extend_selection_to(&mut self, position: Point<Pixels>) -> bool {
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
    pub(super) fn start_selection_autoscroll(&mut self, cx: &mut gpui::Context<Self>) {
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
    pub(super) fn tick_selection_autoscroll(&mut self) -> bool {
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
    pub(super) fn word_range_at(
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

    pub(super) fn queue_mouse_event(
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

    pub(super) fn sync_terminal_grid_size(&mut self, next_size: TerminalGridSize) {
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

    pub(super) fn build_terminal_canvas_paint_state(
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
pub(super) fn trim_terminal_copy(text: &str) -> String {
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
