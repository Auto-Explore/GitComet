use super::terminal_alacritty::*;
use super::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
const TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS: usize = 24;
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Default)]
struct TerminalCanvasPaintState {
    bounds: Bounds<Pixels>,
    terminal_bg: gpui::Rgba,
    selection_rects: Vec<Bounds<Pixels>>,
    background_rects: Vec<(Point<Pixels>, gpui::Size<Pixels>, gpui::Rgba)>,
    lines: Vec<(ShapedLine, Point<Pixels>, Pixels)>,
    cursor: Option<Bounds<Pixels>>,
    ime_bounds: Option<Bounds<Pixels>>,
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
        Self {
            theme,
            focus_handle,
            term_lock: Some(term_lock),
            pty_sender: Some(pty_sender),
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
            mouse_mode_active: false,
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
        }
    }

    fn handle_key_down(&mut self, event: &gpui::KeyDownEvent, cx: &mut gpui::Context<Self>) {
        if self.handle_scrollback_key(event, cx) {
            cx.stop_propagation();
            return;
        }
        let app_cursor = self
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::APP_CURSOR))
            .unwrap_or(false);
        let option_as_meta = true;
        if let Some(bytes) =
            encode_alacritty_key_input(&event.keystroke, app_cursor, option_as_meta)
        {
            self.queue_input(bytes, cx);
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

        if self
            .last_content
            .as_ref()
            .map(|c| c.mode.mouse_mode())
            .unwrap_or(false)
        {
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
                let mode = self
                    .last_content
                    .as_ref()
                    .map(|c| c.mode)
                    .unwrap_or_default();
                let reports = terminal_scroll_report(
                    grid_row,
                    grid_col,
                    event.modifiers,
                    delta_y,
                    step_rows,
                    mode,
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

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        button: gpui::MouseButton,
    ) {
        window.focus(&self.focus_handle, cx);
        self.reset_cursor_blink(cx);

        if self.mouse_mode_active {
            self.queue_mouse_event(event.position, button, event.modifiers, true, cx);
            self.pressed_mouse_button = Some(button);
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
        button: gpui::MouseButton,
    ) {
        if self.mouse_mode_active {
            self.queue_mouse_event(event.position, button, event.modifiers, false, cx);
        }
        self.pressed_mouse_button = None;
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.mouse_mode_active {
            return;
        }
        let mode = self
            .last_content
            .as_ref()
            .map(|c| c.mode)
            .unwrap_or_default();
        if !mode.intersects(TerminalModes::MOUSE_MOTION | TerminalModes::MOUSE_DRAG) {
            return;
        }
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

    fn queue_mouse_event(
        &mut self,
        position: Point<Pixels>,
        button: gpui::MouseButton,
        modifiers: gpui::Modifiers,
        pressed: bool,
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
            term.resize(TerminalResizeDims {
                columns: next_size.cols as usize,
                screen_lines: next_size.rows as usize,
                total_lines: 10_000,
            });
            drop(term);
            pty_sender.resize(next_size.cols as usize, next_size.rows as usize);
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
            self.mouse_mode_active = content.mode.mouse_mode();
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

            let mut paint_state = TerminalCanvasPaintState::default();
            paint_state.bounds = bounds;
            paint_state.terminal_bg =
                TerminalAnsiPalette::from_theme(self.theme).background;
            self.viewport_bounds = Some(bounds);

            for row in 0..rows {
                let cache_row = &mut self.render_cache.rows[usize::from(row)];

                if rebuild || cache_row.shaped.is_none() {
                    let (text, runs, bg_rects) = build_alacritty_row(
                        &content.cells,
                        row as i32,
                        cols,
                        &layout.base_style,
                        self.theme,
                    );
                    let shaped = window.text_system().shape_line(
                        text,
                        layout.metrics.font_size,
                        &runs,
                        None,
                    );
                    cache_row.shaped = Some(shaped);
                    cache_row.background_rects = bg_rects;
                }

                if let Some(ref shaped) = cache_row.shaped {
                    paint_state.lines.push((
                        shaped.clone(),
                        point(
                            bounds.left(),
                            bounds.top() + layout.metrics.line_height * row as f32,
                        ),
                        layout.metrics.line_height,
                    ));
                }

                for rect in &cache_row.background_rects {
                    let origin = point(
                        bounds.left() + layout.metrics.cell_width * rect.col as f32,
                        bounds.top() + layout.metrics.line_height * rect.row as f32,
                    );
                    let rect_size = size(
                        layout.metrics.cell_width * rect.num_cells as f32,
                        layout.metrics.line_height,
                    );
                    paint_state
                        .background_rects
                        .push((origin, rect_size, rect.color));
                }
            }

            // Cursor
            if self.cursor_blink_visible {
                let cursor_row = content.cursor.point.line.0 as f32;
                let cursor_col = content.cursor.point.column.0 as f32;
                if cursor_row >= 0.0
                    && cursor_row < rows as f32
                    && cursor_col >= 0.0
                    && cursor_col < cols as f32
                {
                    let cursor_bounds = Bounds::new(
                        point(
                            bounds.left() + layout.metrics.cell_width * cursor_col,
                            bounds.top() + layout.metrics.line_height * cursor_row,
                        ),
                        size(layout.metrics.cell_width, layout.metrics.line_height),
                    );
                    paint_state.cursor = Some(cursor_bounds);
                }
            }

            paint_state
        } else {
            TerminalCanvasPaintState::default()
        }
    }
}

struct TerminalResizeDims {
    columns: usize,
    screen_lines: usize,
    total_lines: usize,
}

impl alacritty_terminal::grid::Dimensions for TerminalResizeDims {
    fn columns(&self) -> usize {
        self.columns
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn total_lines(&self) -> usize {
        self.total_lines
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
            .on_mouse_move(cx.listener(
                |this, e: &MouseMoveEvent, window, cx| {
                    this.handle_mouse_move(e, window, cx);
                },
            ))
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

// ============================================================================
// GitCometView terminal methods
// ============================================================================

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
            if self.terminal_cursor_blink_active
                || self.terminal_cursor_blink_task_scheduled
                || !self.terminal_cursor_blink_visible
            {
                self.deactivate_terminal_cursor_blink();
            }
            return;
        }
        if self.terminal_cursor_blink_should_run(repo_id, window) {
            if !self.terminal_cursor_blink_active {
                self.terminal_cursor_blink_active = true;
                self.terminal_cursor_blink_seq = self.terminal_cursor_blink_seq.wrapping_add(1);
            }
            self.schedule_terminal_cursor_blink_tick(cx);
        } else if self.terminal_cursor_blink_active || !self.terminal_cursor_blink_visible {
            self.deactivate_terminal_cursor_blink();
        }
    }

    fn terminal_cursor_blink_should_run(&self, repo_id: RepoId, window: &Window) -> bool {
        self.terminal_sessions
            .get(&repo_id)
            .is_some_and(|session| session.connected && session.focus_handle.is_focused(window))
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
            self.close_terminal_for_repo(repo_id, cx);
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
            .collect::<HashSet<_>>();
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
            if let Some(session) = self.terminal_sessions.remove(&repo_id)
                && let Some(pty) = &session.pty_sender {
                    pty.shutdown();
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

        let workdir2 = workdir.clone();
        let window_id = 0u64;

        // Do PTY spawning synchronously (non-blocking on Linux - openpty is fast)
        let spawned = match spawn_alacritty_terminal(&workdir2, window_id) {
            Ok(spawned) => spawned,
            Err(err) => {
                self.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to start embedded terminal: {err}"),
                    cx,
                );
                return;
            }
        };

        self.finish_open_terminal_for_repo(repo_id, workdir, repo_name, spawned, window, cx);
    }

    fn finish_open_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        workdir: PathBuf,
        repo_name: String,
        spawned: SpawnedAlacTerminal,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let session_seq = self.next_terminal_session_seq;
        self.next_terminal_session_seq = self.next_terminal_session_seq.wrapping_add(1).max(1);
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let theme = self.theme;

        let term_lock = spawned.term_lock.clone();
        let pty_sender = spawned.pty_sender.clone();
        let events_rx = spawned.events_rx;

        let viewport = cx.new(|_cx| {
            TerminalViewportView::new(
                theme,
                focus_handle.clone(),
                term_lock.clone(),
                pty_sender.clone(),
            )
        });

        self.terminal_sessions.insert(
            repo_id,
            RepoTerminalSession {
                workdir,
                repo_name,
                focus_handle,
                io: Arc::new(Mutex::new(TerminalIo {
                    pty_sender: Some(pty_sender.clone()),
                    events_rx: None,
                })),
                term_lock: Some(term_lock),
                last_content: None,
                pty_sender: Some(pty_sender),
                events_rx: Some(events_rx),
                grid_size: TerminalGridSize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                content_epoch: 0,
                render_cache: TerminalRenderCache::default(),
                connected: true,
                exit_status: None,
                viewport,
                session_seq,
                selection: None,
                selection_drag_anchor: None,
                viewport_bounds: None,
                ime_state: None,
            },
        );

        // Spawn event processing task
        self.spawn_terminal_event_task(repo_id, session_seq, cx);
        self.reset_terminal_cursor_blink(cx);
        self.sync_terminal_indicator_views(cx);
        self.focus_terminal_view(repo_id, window, cx);
        cx.notify();
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
            .and_then(|s| s.events_rx.take());
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
                        if session.session_seq != session_seq {
                            return;
                        }

                        match event {
                            TerminalBackendEvent::Title(_title) => {}
                            TerminalBackendEvent::ClipboardStore(data) => {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(data));
                            }
                            TerminalBackendEvent::ClipboardLoad => {
                                if let Some(item) = cx.read_from_clipboard()
                                    && let Some(text) = item.text()
                                        && let Some(ref pty) = session.pty_sender {
                                            pty.write(text.into_bytes());
                                        }
                            }
                            TerminalBackendEvent::Bell => {}
                            TerminalBackendEvent::Exit | TerminalBackendEvent::ChildExit(_) => {
                                session.connected = false;
                                session.exit_status = Some("Shell exited.".to_string());
                                cx.notify();
                            }
                            TerminalBackendEvent::Wakeup
                            | TerminalBackendEvent::CursorBlinkingChange => {
                                session.viewport.update(cx, |viewport, _cx| {
                                    viewport.content_epoch = viewport.content_epoch.wrapping_add(1);
                                });
                                cx.notify();
                            }
                            TerminalBackendEvent::PtyWrite(_) => {}
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

    fn close_terminal_for_repo(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        if let Some(session) = self.terminal_sessions.remove(&repo_id)
            && let Some(ref pty) = session.pty_sender {
                pty.shutdown();
            }
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
            .map(|s| s.focus_handle.clone())
        else {
            return;
        };
        window.focus(&focus_handle, cx);
        self.reset_terminal_cursor_blink(cx);
    }

    pub(super) fn send_terminal_bytes_for_repo(&mut self, repo_id: RepoId, bytes: Vec<u8>) {
        if let Some(session) = self.terminal_sessions.get(&repo_id)
            && let Some(ref pty) = session.pty_sender {
                pty.write(bytes);
            }
    }

    pub(super) fn copy_terminal_selection_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(session) = self.terminal_sessions.get_mut(&repo_id) else {
            return false;
        };
        let selection = match session.selection {
            Some(sel) => sel,
            None => return false,
        };

        let text = if let Some(term_lock) = &session.term_lock {
            let term = term_lock.lock();
            match selection {
                TerminalSelection::AllBuffer => terminal_full_buffer_text(&term),
                TerminalSelection::Visible { start, end } => {
                    let content = make_terminal_content(&term);
                    let (s, e) = TerminalSelection::visible(start, end)
                        .normalized_visible()
                        .unwrap_or((start, end));
                    let mut result = String::new();
                    for row in s.row..=e.row.min(content.terminal_bounds.screen_lines as u16 - 1) {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&row_cell_text(
                            &content.cells,
                            row as i32,
                            content.terminal_bounds.columns,
                        ));
                    }
                    result
                }
            }
        } else {
            return false;
        };

        crate::clipboard::write_text(cx, text);
        true
    }

    pub(super) fn paste_terminal_clipboard_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        let Some(session) = self.terminal_sessions.get(&repo_id) else {
            return false;
        };
        let bracketed = session
            .last_content
            .as_ref()
            .map(|c| c.mode.contains(TerminalModes::BRACKETED_PASTE))
            .unwrap_or(false);
        let bytes = terminal_paste_bytes(&text, bracketed);
        if let Some(ref pty) = session.pty_sender {
            pty.write(bytes);
        }
        true
    }

    pub(super) fn select_all_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(session) = self.terminal_sessions.get_mut(&repo_id) {
            session.selection = Some(TerminalSelection::AllBuffer);
            cx.notify();
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
        if !self.terminal_sessions.contains_key(&active_repo) {
            return None;
        }

        let has_selection = self
            .terminal_sessions
            .get(&active_repo)
            .map(|s| s.selection.is_some())
            .unwrap_or(false);
        let connected = self
            .terminal_sessions
            .get(&active_repo)
            .map(|s| s.connected)
            .unwrap_or(false);
        let viewport_entity = self
            .terminal_sessions
            .get(&active_repo)
            .map(|s| s.viewport.clone());
        let exit_status = self
            .terminal_sessions
            .get(&active_repo)
            .and_then(|s| s.exit_status.clone());

        let header =
            self.render_terminal_header(theme, active_repo, has_selection, connected, window, cx);
        let viewport_element = viewport_entity
            .map(|e| {
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .key_context("Terminal")
                    .child(e)
                    .into_any_element()
            })
            .unwrap_or_else(|| div().flex_1().into_any_element());

        let panel = div()
            .flex()
            .flex_col()
            .h(self.terminal_panel_height)
            .min_h(px(TERMINAL_PANEL_MIN_HEIGHT_PX))
            .bg(terminal_default_background(theme))
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
                            .text_color(gpui::rgb(0x888888))
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
        _has_selection: bool,
        _connected: bool,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let close_repo = active_repo;
        let external_repo = active_repo;
        let clear_repo = active_repo;
        div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.0))
            .py(px(4.0))
            .bg(theme.colors.surface_bg)
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .flex_1()
                    .text_color(theme.colors.text)
                    .text_size(px(12.0))
                    .child("Terminal"),
            )
            .child(
                div()
                    .px(px(4.0))
                    .py(px(2.0))
                    .cursor(CursorStyle::PointingHand)
                    .text_color(theme.colors.text)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.open_external_terminal_for_repo(external_repo, cx);
                        }),
                    )
                    .child("\u{26A1}"),
            )
            .child(
                div()
                    .px(px(4.0))
                    .py(px(2.0))
                    .cursor(CursorStyle::PointingHand)
                    .text_color(theme.colors.text)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, _cx| {
                            this.clear_terminal_for_repo(clear_repo, _window, _cx);
                        }),
                    )
                    .child("Clear"),
            )
            .child(
                div()
                    .px(px(4.0))
                    .py(px(2.0))
                    .cursor(CursorStyle::PointingHand)
                    .text_color(theme.colors.text)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.close_terminal_for_repo(close_repo, cx);
                        }),
                    )
                    .child("\u{2715}"),
            )
            .into_any()
    }

    fn terminal_header_external_button(
        &self,
        repo_id: RepoId,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let repo_id = repo_id;
        gpui::div()
            .px(px(4.0))
            .py(px(2.0))
            .cursor(CursorStyle::PointingHand)
            .text_color(theme.colors.text)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                    this.open_external_terminal_for_repo(repo_id, cx);
                }),
            )
            .child("\u{26A1}")
            .into_any()
    }

    fn terminal_header_clear_button(
        &self,
        repo_id: RepoId,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let repo_id = repo_id;
        gpui::div()
            .px(px(4.0))
            .py(px(2.0))
            .cursor(CursorStyle::PointingHand)
            .text_color(theme.colors.text)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, _window, _cx| {
                    this.clear_terminal_for_repo(repo_id, _window, _cx);
                }),
            )
            .child("Clear")
            .into_any()
    }

    fn terminal_header_close_button(
        &self,
        repo_id: RepoId,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let repo_id = repo_id;
        gpui::div()
            .px(px(4.0))
            .py(px(2.0))
            .cursor(CursorStyle::PointingHand)
            .text_color(theme.colors.text)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                    this.close_terminal_for_repo(repo_id, cx);
                }),
            )
            .child("\u{2715}")
            .into_any()
    }

    pub(super) fn open_external_terminal_for_repo(
        &mut self,
        repo_id: RepoId,
        _cx: &mut gpui::Context<Self>,
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
            let _ = launch_external_terminal_from_preferences(&self.terminal_preferences, &context);
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
            .h(px(TERMINAL_PANEL_RESIZE_HANDLE_PX))
            .w_full()
            .cursor(CursorStyle::ResizeUpDown)
            .bg(theme.colors.border)
            .on_drag(TerminalPanelResizeDrag, |_payload, _offset, _window, cx| {
                cx.new(|_cx| super::mod_helpers::ResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
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
        for session in self.terminal_sessions.values() {
            if let Some(ref pty) = session.pty_sender {
                pty.shutdown();
            }
        }
    }
}

// ============================================================================
// Helper functions
// ============================================================================

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
    style.color = terminal_default_foreground(theme).into();
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
    for rect in paint_state.selection_rects {
        window.paint_quad(fill(
            rect,
            with_alpha(theme.colors.accent, TERMINAL_SELECTION_ALPHA),
        ));
    }
    for (origin, rect_size, color) in paint_state.background_rects {
        window.paint_quad(fill(Bounds::new(origin, rect_size), color));
    }
    for (line, origin, line_height) in paint_state.lines {
        let _ = line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
    }
    if let Some(cursor) = paint_state.cursor {
        let caret = terminal_caret_bounds(cursor);
        window.paint_quad(
            fill(caret, terminal_default_foreground(theme)).corner_radii(px(TERMINAL_CARET_RADIUS_PX)),
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
