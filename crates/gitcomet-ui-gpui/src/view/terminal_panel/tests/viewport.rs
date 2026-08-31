use super::super::viewport::trim_terminal_copy;
use super::super::*;
use super::support::*;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::vte::ansi::Handler;

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
fn the_autoscroll_ticker_does_not_collapse_a_word_or_line_selection(cx: &mut gpui::TestAppContext) {
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
