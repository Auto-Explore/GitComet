//! Holding a mouse selection past an input's edge keeps scrolling it.

use super::state::*;
use super::*;
use crate::test_support::refresh_and_draw as draw_frame;
use gpui::Modifiers;

/// Room around the viewport, so the pointer can leave it on every side
/// without leaving the window.
const MARGIN: f32 = 100.0;

#[derive(Clone, Copy)]
enum Host {
    /// Unwrapped multiline input in a fixed viewport (merge-tool output).
    Plain,
    /// Soft-wrapped input in a capped `ScrollContainer` (commit message box).
    Wrapped,
    /// Content-width input scrolling both ways (unwrapped file editor).
    Wide,
    /// A single-line field.
    SingleLine,
}

struct DragHostView {
    input: Entity<TextInput>,
    scroll: ScrollHandle,
    host: Host,
}

impl DragHostView {
    fn new(host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.activate();
        let scroll = ScrollHandle::new();
        let input = cx.new(|cx| {
            let multiline = !matches!(host, Host::SingleLine);
            let mut input = TextInput::new(
                TextInputOptions {
                    multiline,
                    soft_wrap: matches!(host, Host::Wrapped),
                    ..Default::default()
                },
                window,
                cx,
            );
            if multiline {
                input.set_vertical_scroll_handle(Some(scroll.clone()));
            }
            if matches!(host, Host::Wide) {
                input.set_content_width_layout(true);
            }
            input
        });
        Self {
            input,
            scroll,
            host,
        }
    }
}

impl Render for DragHostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = match self.host {
            Host::Plain => div()
                .id("drag_viewport")
                .w(px(600.0))
                .h(px(400.0))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(self.input.clone())
                .into_any_element(),
            Host::Wrapped => div()
                .w(px(280.0))
                .child(
                    crate::components::ScrollContainer::vertical(
                        "drag_surface",
                        "drag_scrollbar",
                        self.scroll.clone(),
                        px(140.0),
                    )
                    .render(AppTheme::gitcomet_dark(), self.input.clone()),
                )
                .into_any_element(),
            Host::Wide => div()
                .id("drag_viewport")
                .flex()
                .flex_col()
                .items_start()
                .w(px(400.0))
                .h(px(300.0))
                .overflow_scroll()
                .track_scroll(&self.scroll)
                .child(self.input.clone())
                .into_any_element(),
            Host::SingleLine => div()
                .w(px(200.0))
                .child(self.input.clone())
                .into_any_element(),
        };
        div()
            .size_full()
            .pt(px(MARGIN))
            .pl(px(MARGIN))
            .child(viewport)
    }
}

fn open<'a>(
    host: Host,
    text: &str,
    cx: &'a mut gpui::TestAppContext,
) -> (
    Entity<TextInput>,
    ScrollHandle,
    &'a mut gpui::VisualTestContext,
) {
    let (view, cx) = cx.add_window_view(move |window, cx| DragHostView::new(host, window, cx));
    cx.simulate_resize(size(px(1000.0), px(800.0)));
    let (input, scroll) = cx.update(|window, app| {
        let (input, scroll) = {
            let view = view.read(app);
            (view.input.clone(), view.scroll.clone())
        };
        input.update(app, |input, cx| {
            // The file editor's line height: two rows (40 px) shaped past the
            // viewport are less than one full autoscroll step.
            input.set_line_height(Some(px(20.0)), cx);
            input.set_text(text.to_owned(), cx);
            input.set_caret(0, cx);
            window.focus(&input.focus_handle(), cx);
        });
        (input, scroll)
    });
    for _ in 0..4 {
        draw_frame(cx);
    }
    (input, scroll, cx)
}

/// Fixed-width rows, so an offset maps straight to (row, column).
const ROW_LEN: usize = 36;

fn numbered_rows(count: usize) -> String {
    let text: String = (0..count)
        .map(|ix| format!("line {ix:03} lorem ipsum dolor sit amet\n"))
        .collect();
    debug_assert!(text.lines().all(|line| line.len() + 1 == ROW_LEN));
    text
}

fn text_bounds(input: &Entity<TextInput>, cx: &mut gpui::VisualTestContext) -> Bounds<Pixels> {
    cx.update(|_window, app| input.read(app).layout.bounds.expect("painted"))
}

fn line_height(input: &Entity<TextInput>, cx: &mut gpui::VisualTestContext) -> Pixels {
    cx.update(|_window, app| input.read(app).layout.line_height)
}

fn head(input: &Entity<TextInput>, cx: &mut gpui::VisualTestContext) -> usize {
    cx.update(|_window, app| input.read(app).cursor_offset())
}

/// A window point that hits `offset` in the last painted frame.
fn point_for_offset(
    input: &Entity<TextInput>,
    cx: &mut gpui::VisualTestContext,
    offset: usize,
    y: Pixels,
) -> Point<Pixels> {
    cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("painted");
        (0..f32::from(bounds.size.width).ceil() as usize)
            .map(|x| point(bounds.left() + px(x as f32), y))
            .find(|at| input.index_for_mouse_position(*at) == offset)
            .unwrap_or_else(|| panic!("a hit position for offset {offset}"))
    })
}

fn press(cx: &mut gpui::VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    draw_frame(cx);
}

fn drag(cx: &mut gpui::VisualTestContext, to: Point<Pixels>) {
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    draw_frame(cx);
}

fn tick(cx: &mut gpui::VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(16));
    cx.run_until_parked();
    draw_frame(cx);
}

/// The window y of `row`'s top in the frame just painted.
fn row_top(input: &Entity<TextInput>, cx: &mut gpui::VisualTestContext, row: usize) -> Pixels {
    text_bounds(input, cx).top() + line_height(input, cx) * row as f32
}

#[gpui::test]
fn a_drag_held_below_a_multiline_input_keeps_scrolling_it(cx: &mut gpui::TestAppContext) {
    let text = numbered_rows(200);
    let (input, scroll, cx) = open(Host::Plain, &text, cx);
    let viewport = scroll.bounds();
    let line_height = line_height(&input, cx);
    let y = row_top(&input, cx, 2) + line_height / 2.0;
    let at = point_for_offset(&input, cx, 2 * ROW_LEN + 6, y);
    press(cx, at);

    drag(cx, point(at.x, viewport.bottom() + px(30.0)));
    for _ in 0..5 {
        tick(cx);
    }
    let after_five = scroll.offset().y;
    assert!(after_five < px(0.0), "held below, the input scrolls");
    for _ in 0..5 {
        tick(cx);
    }
    assert!(
        scroll.offset().y < after_five,
        "with the pointer still, it keeps scrolling"
    );

    // The head follows the visible bottom edge at the pointer's column.
    let head = head(&input, cx);
    let row = head / ROW_LEN;
    assert_eq!(head % ROW_LEN, 6, "the head keeps the pointer's column");
    let top = row_top(&input, cx, row);
    assert!(
        top < viewport.bottom() && top + line_height >= viewport.bottom() - px(1.0),
        "the head sits on the row at the bottom edge (row {row}, top {top:?}, edge {:?})",
        viewport.bottom()
    );
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn a_fast_drag_keeps_the_head_on_a_visible_row_at_the_pointers_column(
    cx: &mut gpui::TestAppContext,
) {
    // Wrapped layouts shape the fewest rows past the viewport, so the commit
    // box is the case a full step outruns first. Rows short enough not to wrap.
    let short: String = (0..120).map(|ix| format!("w{ix:03} abcdefgh\n")).collect();
    for (host, text, row_len) in [
        (Host::Plain, numbered_rows(200), ROW_LEN),
        (Host::Wrapped, short, 14),
    ] {
        let (input, scroll, cx) = open(host, &text, cx);
        let viewport = scroll.bounds();
        let line_height = line_height(&input, cx);
        let y = row_top(&input, cx, 1) + line_height / 2.0;
        let at = point_for_offset(&input, cx, row_len + 6, y);
        press(cx, at);

        // Far enough out for the largest step.
        drag(cx, point(at.x, viewport.bottom() + px(150.0)));
        let mut previous = scroll.offset().y;
        for ix in 0..20 {
            tick(cx);
            let offset = scroll.offset().y;
            assert!(offset < previous, "tick {ix} scrolled");
            previous = offset;
            let head = head(&input, cx);
            assert_eq!(head % row_len, 6, "tick {ix}: head at the pointer's column");
            let top = row_top(&input, cx, head / row_len);
            assert!(
                top < viewport.bottom() && top + line_height >= viewport.bottom() - px(1.0),
                "tick {ix}: head on the bottom row"
            );
        }

        // Held until the end, the selection reaches the end of the text.
        for _ in 0..200 {
            tick(cx);
        }
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
        cx.update(|_window, app| {
            assert_eq!(input.read(app).selection.range, row_len + 6..text.len());
        });
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    }
}

#[gpui::test]
fn a_drag_held_above_scrolls_back_up(cx: &mut gpui::TestAppContext) {
    let text = numbered_rows(200);
    let (input, scroll, cx) = open(Host::Plain, &text, cx);
    scroll.set_offset(point(px(0.0), -scroll.max_offset().y));
    draw_frame(cx);
    let viewport = scroll.bounds();
    let line_height = line_height(&input, cx);
    let bounds = text_bounds(&input, cx);
    let row = ((viewport.center().y - bounds.top()) / line_height).floor() as usize;
    let y = row_top(&input, cx, row) + line_height / 2.0;
    let at = point_for_offset(&input, cx, row * ROW_LEN + 6, y);
    press(cx, at);

    drag(cx, point(at.x, viewport.top() - px(30.0)));
    let before = scroll.offset().y;
    for _ in 0..10 {
        tick(cx);
    }
    assert!(scroll.offset().y > before, "held above, it scrolls up");
    let head = head(&input, cx);
    assert_eq!(head % ROW_LEN, 6);
    let top = row_top(&input, cx, head / ROW_LEN);
    assert!(
        top <= viewport.top() && top + line_height > viewport.top(),
        "the head sits on the row at the top edge"
    );
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn a_drag_held_below_a_wrapped_commit_box_scrolls_to_the_end(cx: &mut gpui::TestAppContext) {
    let text: String = (0..30)
        .map(|ix| {
            if ix % 5 == 0 {
                format!("paragraph {ix} that is long enough to wrap onto a second row here\n")
            } else {
                format!("short line {ix}\n")
            }
        })
        .collect();
    let (input, scroll, cx) = open(Host::Wrapped, &text, cx);
    let viewport = scroll.bounds();
    assert!(
        scroll.max_offset().y > px(0.0),
        "the text overflows the box"
    );
    let line_height = line_height(&input, cx);
    let y = row_top(&input, cx, 0) + line_height / 2.0;
    let at = point_for_offset(&input, cx, 3, y);
    press(cx, at);

    drag(cx, point(at.x, viewport.bottom() + px(20.0)));
    tick(cx);
    tick(cx);
    let early = scroll.offset().y;
    assert!(early < px(0.0), "held below, the box scrolls");
    for _ in 0..300 {
        tick(cx);
    }
    assert_eq!(scroll.offset().y, -scroll.max_offset().y);
    cx.update(|_window, app| {
        assert_eq!(input.read(app).selection.range, 3..text.len());
    });
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn releasing_or_losing_the_button_stops_the_scroll(cx: &mut gpui::TestAppContext) {
    let text = numbered_rows(200);
    for lose_button in [false, true] {
        let (input, scroll, cx) = open(Host::Plain, &text, cx);
        let viewport = scroll.bounds();
        let line_height = line_height(&input, cx);
        let y = row_top(&input, cx, 1) + line_height / 2.0;
        let at = point_for_offset(&input, cx, ROW_LEN + 6, y);
        press(cx, at);
        let below = point(at.x, viewport.bottom() + px(30.0));
        drag(cx, below);
        for _ in 0..3 {
            tick(cx);
        }
        assert!(scroll.offset().y < px(0.0));
        if lose_button {
            // A release the window never saw: the next move has no button.
            cx.simulate_mouse_move(below, None, Modifiers::default());
        } else {
            cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::default());
        }
        draw_frame(cx);
        let stopped = scroll.offset().y;
        for _ in 0..5 {
            tick(cx);
        }
        assert_eq!(scroll.offset().y, stopped, "lose_button={lose_button}");
        cx.update(|_window, app| assert!(!input.read(app).interaction.is_selecting));
    }
}

#[gpui::test]
fn a_drag_held_right_of_an_unwrapped_editor_scrolls_it_sideways(cx: &mut gpui::TestAppContext) {
    let text: String = (0..20)
        .map(|ix| format!("{ix:02} {}\n", "x".repeat(300)))
        .collect();
    let (input, scroll, cx) = open(Host::Wide, &text, cx);
    let viewport = scroll.bounds();
    assert!(scroll.max_offset().x > px(0.0), "lines overflow sideways");
    let line_height = line_height(&input, cx);
    let y = row_top(&input, cx, 1) + line_height / 2.0;
    let at = point_for_offset(&input, cx, 304 + 2, y);
    press(cx, at);

    drag(cx, point(viewport.right() + px(40.0), y));
    for _ in 0..5 {
        tick(cx);
    }
    let after_five = scroll.offset().x;
    assert!(
        after_five < px(0.0),
        "held right, the editor scrolls sideways"
    );
    for _ in 0..5 {
        tick(cx);
    }
    assert!(scroll.offset().x < after_five, "and keeps going");
    assert_eq!(scroll.offset().y, px(0.0), "without scrolling down");
    let head = head(&input, cx);
    assert_eq!(head / 304, 1, "the head stays on the pressed row");
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn a_drag_held_past_a_single_line_field_scrolls_it_one_step_at_a_time(
    cx: &mut gpui::TestAppContext,
) {
    let text = format!("start {} end", "word ".repeat(80));
    let (input, _scroll, cx) = open(Host::SingleLine, &text, cx);
    let bounds = text_bounds(&input, cx);
    let at = point_for_offset(&input, cx, 2, bounds.center().y);
    press(cx, at);

    // Out past the right edge and a little below the field: the field is one
    // line, so the drift must not read as "select to the end".
    drag(
        cx,
        point(bounds.right() + px(20.0), bounds.bottom() + px(12.0)),
    );
    let scroll_x = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| input.read(app).layout.scroll_x)
    };
    let mut previous = scroll_x(cx);
    for ix in 0..10 {
        tick(cx);
        let current = scroll_x(cx);
        assert!(current > previous, "tick {ix} scrolled right");
        assert!(
            current - previous <= px(24.0),
            "tick {ix} moved {:?}, not one step",
            current - previous
        );
        previous = current;
    }
    cx.update(|_window, app| {
        let input = input.read(app);
        assert!(input.selection.range.end < text.len(), "not yet at the end");
        assert!(input.selection.range.start == 2);
    });

    // And back the other way.
    drag(cx, point(bounds.left() - px(20.0), bounds.center().y));
    let turned = scroll_x(cx);
    for _ in 0..3 {
        tick(cx);
    }
    assert!(scroll_x(cx) < turned, "held left, it scrolls back");
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn a_hit_test_after_a_scroll_lands_on_the_row_now_under_the_pointer(cx: &mut gpui::TestAppContext) {
    let text = numbered_rows(200);
    let (input, scroll, cx) = open(Host::Plain, &text, cx);
    let viewport = scroll.bounds();
    let line_height = line_height(&input, cx);
    let at = point(
        viewport.left() + px(40.0),
        viewport.top() + line_height * 5.5,
    );
    let before = cx.update(|_window, app| input.read(app).index_for_mouse_position(at));
    // Scrolled by three rows, before the next paint.
    scroll.set_offset(point(px(0.0), scroll.offset().y - line_height * 3.0));
    let after = cx.update(|_window, app| input.read(app).index_for_mouse_position(at));
    assert_eq!(after / ROW_LEN, before / ROW_LEN + 3);
}

#[gpui::test]
fn a_second_tick_before_the_next_paint_does_not_step_again(cx: &mut gpui::TestAppContext) {
    let text = numbered_rows(200);
    let (input, scroll, cx) = open(Host::Plain, &text, cx);
    let viewport = scroll.bounds();
    let line_height = line_height(&input, cx);
    let y = row_top(&input, cx, 1) + line_height / 2.0;
    let at = point_for_offset(&input, cx, ROW_LEN + 6, y);
    press(cx, at);
    drag(cx, point(at.x, viewport.bottom() + px(150.0)));

    let one_step = cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let start = scroll.offset().y;
            assert!(input.tick_drag_autoscroll(cx));
            let one_step = scroll.offset().y;
            assert!(one_step < start);
            input.tick_drag_autoscroll(cx);
            assert_eq!(scroll.offset().y, one_step, "no second step before a paint");
            one_step
        })
    });
    draw_frame(cx);
    cx.update(|_window, app| input.update(app, |input, cx| input.tick_drag_autoscroll(cx)));
    assert!(scroll.offset().y < one_step, "after a paint it steps again");
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
}
