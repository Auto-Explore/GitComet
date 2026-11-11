//! Normal-buffer elastic scrollback regressions.

use super::*;
#[cfg(target_os = "macos")]
use alacritty_terminal::vte::ansi::Processor;
use gitcomet_ui_kit::elastic_scroll::ElasticScrollDriver;
#[cfg(target_os = "macos")]
use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase};
#[cfg(target_os = "macos")]
use std::{cell::Cell, rc::Rc};

#[cfg(target_os = "macos")]
fn wheel(position: Point<Pixels>, delta: ScrollDelta) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position,
        delta,
        touch_phase: TouchPhase::Started,
        modifiers: Default::default(),
    }
}

#[test]
fn precise_scrollback_accumulates_fractional_intent_without_rounding_it_into_bounce() {
    let term = test_term_with_lines(30);
    let elastic = TerminalElasticScroll::default();
    let driver = elastic.driver(Some(term.clone()), px(TEST_LINE_H), None);
    let (tail, max) = driver.metrics();
    assert!(max > 0.0);
    assert_eq!(tail, -max);

    // Rendering remains on Alacritty's whole-row grid: the first upward
    // packet exposes the preceding row, but later small packets accumulate
    // within that row rather than each rounding into another row or bounce.
    assert_eq!(driver.scroll_by(px(0.25)), px(0.25));
    assert_eq!(term.lock().grid().display_offset(), 1);
    for _ in 0..15 {
        assert_eq!(driver.scroll_by(px(0.25)), px(0.25));
    }
    assert_eq!(driver.metrics(), (tail + 4.0, max));
    assert_eq!(term.lock().grid().display_offset(), 1);
    assert_eq!(driver.scroll_by(px(-4.0)), px(-4.0));
    assert_eq!(driver.metrics(), (tail, max));
    assert_eq!(term.lock().grid().display_offset(), 0);
    assert_eq!(
        driver.scroll_by(px(-3.25)),
        px(0.0),
        "only true edge travel is unused"
    );

    term.lock().scroll_display(Scroll::Top);
    assert_eq!(driver.metrics(), (0.0, max));
    assert_eq!(driver.scroll_by(px(-0.25)), px(-0.25));
    assert_eq!(driver.metrics(), (-0.25, max));
    assert_eq!(driver.scroll_by(px(0.25)), px(0.25));
    assert_eq!(driver.scroll_by(px(10.0)), px(0.0));
}

#[test]
fn precise_scrollback_metrics_keep_fractional_progress_while_the_parser_is_busy() {
    let term = test_term_with_lines(30);
    let elastic = TerminalElasticScroll::default();
    let driver = elastic.driver(Some(term.clone()), px(TEST_LINE_H), None);
    driver.metrics();
    assert_eq!(driver.scroll_by(px(4.25)), px(4.25));
    let expected = driver.metrics();
    let parser = term.lock();
    assert_eq!(
        driver.metrics(),
        expected,
        "a paint-side read must not lock the parser again"
    );
    drop(parser);
    term.lock().scroll_display(Scroll::Top);
    assert_eq!(
        driver.metrics().0,
        0.0,
        "external scrollback actions replace fractional intent"
    );
}

#[test]
fn normal_scroll_attempt_owns_clamped_and_fractional_deltas() {
    let term = test_term_with_lines(30);
    let elastic = TerminalElasticScroll::default();
    let driver = elastic.driver(Some(term), px(TEST_LINE_H), None);
    assert_eq!(driver.scroll_by(px(-1.0)), px(0.0));
    assert_eq!(elastic.take_wheel_attempt(), WheelAttempt::Owned);
    assert_eq!(driver.scroll_by(px(0.25)), px(0.25));
    assert_eq!(elastic.take_wheel_attempt(), WheelAttempt::Owned);
    assert_eq!(
        elastic.take_wheel_attempt(),
        WheelAttempt::Declined,
        "an attempt result is consumed once"
    );
    assert_eq!(driver.scroll_by(px(f32::NAN)), px(0.0));
    assert_eq!(elastic.take_wheel_attempt(), WheelAttempt::Declined);
}

#[cfg(target_os = "macos")]
fn live_viewport(
    term: AlacrittyTermLock,
    sender: Option<PtySender>,
    cx: &mut gpui::TestAppContext,
) -> (Entity<TerminalViewportView>, &mut gpui::VisualTestContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        let focus = cx.focus_handle();
        TerminalViewportView::with_backend(AppTheme::gitcomet_dark(), focus, Some(term), sender, cx)
    });
    cx.simulate_resize(size(px(200.0), px(120.0)));
    gitcomet_ui_kit::test_support::refresh_and_draw(cx);
    (view, cx)
}

#[cfg(target_os = "macos")]
fn dispatch_wheel(cx: &mut gpui::VisualTestContext, event: ScrollWheelEvent) {
    cx.update(|window, app| {
        window.dispatch_event(gpui::PlatformInput::ScrollWheel(event), app);
    });
    gitcomet_ui_kit::test_support::refresh_and_draw(cx);
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn normal_scrollback_bounces_under_a_fixed_scrollbar_with_translated_selection_and_ime(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::ui_runtime::with_override(
        gitcomet_ui_kit::ui_runtime::UiRuntime::live(),
        || {
            let term = test_term_with_lines(30);
            let (view, cx) = live_viewport(term.clone(), None, cx);
            let initial = view.read_with(cx, |view, _| view.viewport_bounds.unwrap());
            let scrollbar = cx.debug_bounds("terminal_scrollback_scrollbar").unwrap();
            let position = initial.center();
            dispatch_wheel(
                cx,
                wheel(position, ScrollDelta::Pixels(point(px(0.0), px(-100.0)))),
            );
            let painted = view.read_with(cx, |view, _| view.viewport_bounds.unwrap());
            assert!(painted.top() < initial.top() - px(20.0));
            assert_eq!(term.lock().grid().display_offset(), 0);
            assert_eq!(
                cx.debug_bounds("terminal_scrollback_scrollbar").unwrap(),
                scrollbar
            );
            cx.update(|window, app| {
                view.update(app, |view, cx| {
                    assert_eq!(view.scroll_elastic.fixed_bounds(painted), initial);
                    let metrics = view.layout_cache.as_ref().unwrap().metrics;
                    let pointer =
                        painted.origin + point(metrics.cell_width * 1.5, metrics.line_height * 2.5);
                    assert_eq!(
                        view.selection_grid_point(pointer),
                        Some(TerminalGridPoint::new(2, 1))
                    );
                    view.selection_start = Some(TerminalGridPoint::new(2, 0));
                    view.selection_end = Some(TerminalGridPoint::new(2, 3));
                    view.ime_state = Some(TerminalImeState {
                        marked_text: "preedit".into(),
                    });
                    window.focus(&view.focus_handle, cx);
                    let state = view.build_terminal_canvas_paint_state(painted, window, cx);
                    assert!(
                        (state.selection_rects[0].top()
                            - painted.top()
                            - metrics.line_height * 2.0)
                            .abs()
                            <= px(1.0)
                    );
                    let cursor = view.last_content.as_ref().unwrap().cursor.point;
                    let expected_ime_top =
                        painted.top() + metrics.line_height * cursor.line.0 as f32;
                    assert!((state.ime_bounds.unwrap().top() - expected_ime_top).abs() <= px(1.0));
                });
            });
            let mut ended = wheel(position, ScrollDelta::Pixels(point(px(0.0), px(0.0))));
            ended.touch_phase = TouchPhase::Ended;
            dispatch_wheel(cx, ended);
            cx.background_executor.advance_clock(Duration::from_secs(1));
            cx.update(|window, app| window.simulate_next_frame(app));
            gitcomet_ui_kit::test_support::refresh_and_draw(cx);
            assert_eq!(
                view.read_with(cx, |view, _| view.viewport_bounds.unwrap()),
                initial
            );
            assert_eq!(
                cx.debug_bounds("terminal_scrollback_scrollbar").unwrap(),
                scrollbar
            );

            term.lock().scroll_display(Scroll::Top);
            gitcomet_ui_kit::test_support::refresh_and_draw(cx);
            dispatch_wheel(
                cx,
                wheel(position, ScrollDelta::Pixels(point(px(0.0), px(100.0)))),
            );
            assert!(
                view.read_with(cx, |view, _| view.viewport_bounds.unwrap().top())
                    > initial.top() + px(20.0)
            );
            {
                let term = term.lock();
                assert_eq!(term.grid().display_offset(), term.grid().history_size());
            }
            let mut ended = wheel(position, ScrollDelta::Pixels(point(px(0.0), px(0.0))));
            ended.touch_phase = TouchPhase::Ended;
            dispatch_wheel(cx, ended);
            cx.background_executor.advance_clock(Duration::from_secs(1));
            cx.update(|window, app| window.simulate_next_frame(app));
            gitcomet_ui_kit::test_support::refresh_and_draw(cx);
            assert_eq!(
                view.read_with(cx, |view, _| view.viewport_bounds.unwrap()),
                initial
            );
        },
    );
}

#[cfg(target_os = "macos")]
fn written_bytes(
    receiver: &smol::channel::Receiver<alacritty_terminal::event_loop::Msg>,
) -> Vec<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        if let alacritty_terminal::event_loop::Msg::Input(input) = message {
            bytes.push(input.into_owned());
        }
    }
    bytes
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn newly_enabled_mouse_reporting_gets_the_original_packet_and_cancels_existing_bounce(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::ui_runtime::with_override(
        gitcomet_ui_kit::ui_runtime::UiRuntime::live(),
        || {
            let term = test_term_with_lines(30);
            let (sender, receiver) = PtySender::recording();
            let (view, cx) = live_viewport(term.clone(), Some(sender), cx);
            written_bytes(&receiver);
            let initial = view.read_with(cx, |view, _| view.viewport_bounds.unwrap());
            dispatch_wheel(
                cx,
                wheel(
                    initial.center(),
                    ScrollDelta::Pixels(point(px(0.0), px(-100.0))),
                ),
            );
            assert!(
                view.read_with(cx, |view, _| view.scroll_elastic.motion.displacement()) < px(-20.0)
            );
            let (painted, metrics) = view.read_with(cx, |view, _| {
                assert!(!view.last_content.as_ref().unwrap().mode.mouse_mode());
                (
                    view.viewport_bounds.unwrap(),
                    view.layout_cache.as_ref().unwrap().metrics,
                )
            });
            // No paint between the mode change and the packet: the live gate
            // must yield even though last_content still describes a shell.
            {
                let mut term = term.lock();
                let mut parser: Processor = Processor::new();
                parser.advance(&mut *term, b"\x1b[?1000h\x1b[?1006h");
            }
            let pointer =
                painted.origin + point(metrics.cell_width * 1.5, metrics.line_height * 2.5);
            dispatch_wheel(
                cx,
                wheel(pointer, ScrollDelta::Pixels(point(px(0.0), px(1.0)))),
            );
            assert_eq!(written_bytes(&receiver), vec![b"\x1b[<64;2;3M".to_vec()]);
            assert_eq!(
                view.read_with(cx, |view, _| view.scroll_elastic.motion.displacement()),
                px(0.0)
            );
            assert_eq!(
                view.read_with(cx, |view, _| view.viewport_bounds.unwrap()),
                initial
            );
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn reporting_enabled_after_a_successful_scroll_does_not_receive_the_owned_packet(
    cx: &mut gpui::TestAppContext,
) {
    let term = test_term_with_lines(80);
    let (sender, receiver) = PtySender::recording();
    let (view, cx) = live_viewport(term.clone(), Some(sender), cx);
    written_bytes(&receiver);
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            let metrics = view.layout_cache.as_ref().unwrap().metrics;
            let event = wheel(
                view.viewport_bounds.unwrap().center(),
                ScrollDelta::Pixels(point(px(0.0), metrics.line_height * 4.0)),
            );
            assert!(view.scroll_elastic.accepts_wheel(view.term_lock.as_ref(), &event));
            let driver = view.scroll_elastic.driver(
                view.term_lock.clone(),
                metrics.line_height,
                view.last_content.as_ref(),
            );
            assert_eq!(driver.scroll_by(metrics.line_height * 4.0), metrics.line_height * 4.0);
            let after = term.lock().grid().display_offset();
            assert_eq!(after, 4);
            // Deterministically model the parser acquiring the released lock
            // between the successful driver attempt and its on_scroll callback.
            {
                let mut term = term.lock();
                let mut parser: Processor = Processor::new();
                parser.advance(&mut *term, b"\x1b[?1000h\x1b[?1006h");
            }
            view.finish_elastic_scroll_wheel(&event, window, cx);
            assert_eq!(term.lock().grid().display_offset(), after);
            assert!(
                written_bytes(&receiver).is_empty(),
                "one owned packet must not also produce a mouse report after the parser changes modes"
            );
        });
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn a_mode_change_before_the_scroll_attempt_forwards_the_original_packet_once(
    cx: &mut gpui::TestAppContext,
) {
    let term = test_term_with_lines(80);
    let (sender, receiver) = PtySender::recording();
    let (view, cx) = live_viewport(term.clone(), Some(sender), cx);
    written_bytes(&receiver);
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            let metrics = view.layout_cache.as_ref().unwrap().metrics;
            let bounds = view.viewport_bounds.unwrap();
            let event = wheel(
                bounds.origin + point(metrics.cell_width * 1.5, metrics.line_height * 2.5),
                ScrollDelta::Pixels(point(px(0.0), px(1.0))),
            );
            assert!(
                view.scroll_elastic
                    .accepts_wheel(view.term_lock.as_ref(), &event)
            );
            let driver = view.scroll_elastic.driver(
                view.term_lock.clone(),
                metrics.line_height,
                view.last_content.as_ref(),
            );
            {
                let mut term = term.lock();
                let mut parser: Processor = Processor::new();
                parser.advance(&mut *term, b"\x1b[?1000h\x1b[?1006h");
            }
            let before = term.lock().grid().display_offset();
            assert_eq!(driver.scroll_by(px(1.0)), px(0.0));
            view.finish_elastic_scroll_wheel(&event, window, cx);
            assert_eq!(term.lock().grid().display_offset(), before);
            assert_eq!(written_bytes(&receiver), vec![b"\x1b[<64;2;3M".to_vec()]);
        });
    });
}

#[cfg(target_os = "macos")]
struct ModeChangeDuringWheel {
    viewport: Entity<TerminalViewportView>,
    term: AlacrittyTermLock,
    leave_mouse_mode: Rc<Cell<bool>>,
    capture_changed_mode: Rc<Cell<bool>>,
}

#[cfg(target_os = "macos")]
impl Render for ModeChangeDuringWheel {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let term = self.term.clone();
        let leave_mouse_mode = self.leave_mouse_mode.clone();
        let capture_changed_mode = self.capture_changed_mode.clone();
        div()
            .relative()
            .size_full()
            .child(self.viewport.clone())
            .child(
                // Register after the viewport's Capture listener, just as a
                // parser thread can change modes before Bubble reads them.
                gpui::canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let term = term.clone();
                        let leave_mouse_mode = leave_mouse_mode.clone();
                        let capture_changed_mode = capture_changed_mode.clone();
                        window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _, _| {
                            if phase == gpui::DispatchPhase::Capture
                                && leave_mouse_mode.replace(false)
                            {
                                let mut term = term.lock();
                                assert!(terminal_live_modes(&term).mouse_mode());
                                let mut parser: Processor = Processor::new();
                                parser.advance(&mut *term, b"\x1b[?1000l\x1b[?1006l");
                                capture_changed_mode.set(true);
                            }
                        });
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn leaving_mouse_reporting_during_dispatch_does_not_replay_an_old_scroll_packet(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::ui_runtime::with_override(
        gitcomet_ui_kit::ui_runtime::UiRuntime::live(),
        || {
            let term = test_term_with_lines(80);
            let leave_mouse_mode = Rc::new(Cell::new(false));
            let capture_changed_mode = Rc::new(Cell::new(false));
            let (surface, cx) = cx.add_window_view(|window, cx| {
                window.activate();
                let viewport = cx.new(|cx| {
                    let focus = cx.focus_handle();
                    TerminalViewportView::with_backend(
                        AppTheme::gitcomet_dark(),
                        focus,
                        Some(term.clone()),
                        None,
                        cx,
                    )
                });
                ModeChangeDuringWheel {
                    viewport,
                    term: term.clone(),
                    leave_mouse_mode: leave_mouse_mode.clone(),
                    capture_changed_mode: capture_changed_mode.clone(),
                }
            });
            cx.simulate_resize(size(px(200.0), px(120.0)));
            gitcomet_ui_kit::test_support::refresh_and_draw(cx);
            let view = surface.read_with(cx, |surface, _| surface.viewport.clone());
            let (bounds, line_height) = view.read_with(cx, |view, _| {
                (
                    view.viewport_bounds.unwrap(),
                    view.layout_cache.as_ref().unwrap().metrics.line_height,
                )
            });
            // Seed a prior claim and a deliberately different filtered delta.
            dispatch_wheel(
                cx,
                wheel(
                    bounds.center(),
                    ScrollDelta::Pixels(point(px(0.0), line_height * 4.0)),
                ),
            );
            let before = term.lock().grid().display_offset();
            assert_eq!(before, 4);
            {
                let mut term = term.lock();
                let mut parser: Processor = Processor::new();
                parser.advance(&mut *term, b"\x1b[?1000h\x1b[?1006h");
            }
            leave_mouse_mode.set(true);
            dispatch_wheel(
                cx,
                wheel(
                    bounds.center(),
                    ScrollDelta::Pixels(point(px(0.0), px(1.0))),
                ),
            );
            assert!(capture_changed_mode.get(), "mode changes inside dispatch");
            assert!(!terminal_live_modes(&term.lock()).mouse_mode());
            assert_eq!(
                term.lock().grid().display_offset(),
                before + 1,
                "the declined packet uses its original one-row fallback, not the old four-row delta"
            );
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn alternate_screen_and_line_wheels_retain_their_existing_input_policy(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::ui_runtime::with_override(
        gitcomet_ui_kit::ui_runtime::UiRuntime::live(),
        || {
            let term = test_term_with_lines(30);
            let (sender, receiver) = PtySender::recording();
            let (view, cx) = live_viewport(term.clone(), Some(sender), cx);
            written_bytes(&receiver);
            let bounds = view.read_with(cx, |view, _| view.viewport_bounds.unwrap());
            let event = wheel(bounds.center(), ScrollDelta::Lines(point(0.0, 2.0)));
            let steps = cx.update(|_window, app| {
                let line = view
                    .read(app)
                    .layout_cache
                    .as_ref()
                    .unwrap()
                    .metrics
                    .line_height;
                terminal_scroll_wheel_delta(&event, line)
                    .unwrap()
                    .1
                    .min(term.lock().grid().history_size())
            });
            dispatch_wheel(cx, event);
            assert_eq!(term.lock().grid().display_offset(), steps);
            assert!(written_bytes(&receiver).is_empty());
            assert_eq!(
                view.read_with(cx, |view, _| view.scroll_elastic.motion.displacement()),
                px(0.0)
            );

            {
                let mut term = term.lock();
                let mut parser: Processor = Processor::new();
                parser.advance(&mut *term, b"\x1b[?1049h\x1b[?1h");
            }
            // Both alternate-screen and application-cursor mode just changed;
            // forwarding must use them before the next content snapshot.
            dispatch_wheel(
                cx,
                wheel(
                    bounds.center(),
                    ScrollDelta::Pixels(point(px(0.0), px(1.0))),
                ),
            );
            assert_eq!(written_bytes(&receiver), vec![b"\x1bOA".to_vec()]);
            assert_eq!(
                view.read_with(cx, |view, _| view.scroll_elastic.motion.displacement()),
                px(0.0)
            );
        },
    );
}
