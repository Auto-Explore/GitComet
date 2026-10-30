//! An extension's own views between a hosted diff's lines.

use super::*;
use gitcomet_extension_api::{DiffInlineView, DiffLayout, DiffLineRange, DiffScrollAnchor};
use std::cell::Cell;
use std::rc::Rc;

/// A review thread in small: its replies, a reply box and a button. Each
/// reply adds 20px.
struct Thread {
    input: gpui::Entity<components::TextInput>,
    replies: usize,
    resolved: usize,
    renders: Rc<Cell<usize>>,
}

impl Render for Thread {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
            .debug_selector(|| "thread_root".into())
            .flex()
            .flex_col()
            .w_full()
            .children(
                (0..self.replies).map(|reply| div().h(px(20.0)).child(format!("Reply {reply}"))),
            )
            .child(
                div()
                    .debug_selector(|| "thread_input".into())
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .id("thread_resolve")
                    .debug_selector(|| "thread_resolve".into())
                    .h(px(20.0))
                    .child("Resolve")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.resolved += 1;
                        cx.notify();
                    })),
            )
    }
}

struct Mounted<'a> {
    pane: DiffPane,
    thread: gpui::Entity<Thread>,
    renderer: gpui::Entity<DiffRenderer>,
    id: u64,
    cx: &'a mut gpui::VisualTestContext,
}

/// The thread's frame, as pane `id` draws it.
fn thread_frame(id: u64, thread: &gpui::Entity<Thread>) -> &'static str {
    selector(format!(
        "hosted_diff_{id}_view_{}",
        thread.entity_id().as_u64()
    ))
}

fn draw(cx: &mut gpui::VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
}

/// A snapshot pane of `old` and `new` with a thread after new line `line`.
fn mount<'a>(
    cx: &'a mut gpui::TestAppContext,
    layout: DiffLayout,
    old: String,
    new: String,
    line: u32,
) -> Mounted<'a> {
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let (pane, thread) = app_cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let pane = host
            .create_snapshot_pane(
                DiffSnapshot::new("notes.txt", old, new),
                DiffPaneOptions {
                    layout,
                    content_width: Some(700.0),
                    ..Default::default()
                },
                app,
            )
            .unwrap();
        let thread = app.new(|cx| Thread {
            input: cx.new(|cx| {
                components::TextInput::new_inert(
                    components::TextInputOptions {
                        placeholder: "Write a reply".into(),
                        ..Default::default()
                    },
                    cx,
                )
            }),
            replies: 1,
            resolved: 0,
            renders: Rc::default(),
        });
        pane.set_inline_views(
            vec![DiffInlineView::new(
                DiffScrollAnchor {
                    side: DiffLineSide::New,
                    line,
                },
                thread.clone(),
            )],
            app,
        );
        (pane, thread)
    });
    app_cx.run_until_parked();
    let renderer = hosted_renderer(app_cx, &pane);
    let id = app_cx.update(|_, app| {
        pane.view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap()
            .read(app)
            .view_id()
    });
    let mounted = pane.view();
    let (_, cx) = cx.add_window_view(move |_, _| PaneHolder(mounted));
    // An input takes typing only in the active window.
    cx.update(|window, _| window.activate());
    draw(cx);
    assert!(cx.update(|_, app| !pane.is_loading(app)));
    Mounted {
        pane,
        thread,
        renderer,
        id,
        cx,
    }
}

fn three_lines() -> (String, String) {
    ("one\ntwo\nthree\n".into(), "one\nTWO\nthree\n".into())
}

/// Where `text` is painted in `region`.
fn painted(
    cx: &mut gpui::VisualTestContext,
    renderer: &gpui::Entity<DiffRenderer>,
    region: DiffTextRegion,
    text: &str,
) -> Bounds<Pixels> {
    cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .iter()
            .find(|((_, at), hit)| *at == region && hit.painted_text.contains(text))
            .map(|(_, hit)| hit.bounds)
            .unwrap_or_else(|| panic!("{text} is painted in {region:?}"))
    })
}

fn row_height(cx: &mut gpui::VisualTestContext, renderer: &gpui::Entity<DiffRenderer>) -> Pixels {
    cx.update(|_, app| renderer.read(app).theme.editor_row_height(100))
}

/// A view draws after its line, in its own side's column only and once a
/// frame, over as many rows as its measured height needs; when it grows,
/// the lines after it move down by the rows it gained.
#[gpui::test]
fn inline_views_draw_after_their_line_in_rows_that_follow_their_height(
    cx: &mut gpui::TestAppContext,
) {
    for layout in [DiffLayout::Inline, DiffLayout::Split] {
        let (old, new) = three_lines();
        let Mounted {
            renderer,
            thread,
            id,
            cx,
            ..
        } = mount(cx, layout, old, new, 2);
        let region = match layout {
            DiffLayout::Split => DiffTextRegion::SplitRight,
            _ => DiffTextRegion::Inline,
        };
        let view = thread_frame(id, &thread);
        let height = row_height(cx, &renderer);
        let reserved = |cx: &mut gpui::VisualTestContext| {
            let rows = cx.update(|_, app| renderer.update(app, |r, _| r.diff_list_len()));
            let document = cx.update(|_, app| renderer.read(app).diff_visible_len());
            rows - document
        };

        let bounds = cx.debug_bounds(view).expect("the view is drawn");
        let two = painted(cx, &renderer, region, "TWO");
        let three = painted(cx, &renderer, region, "three");
        assert!(
            bounds.top() >= two.bottom() - px(0.5),
            "{layout:?}: after TWO"
        );
        assert!(
            three.top() >= bounds.bottom() - px(0.5),
            "{layout:?}: before three"
        );
        let content = cx.debug_bounds("thread_root").unwrap();
        let rows = reserved(cx);
        assert_eq!(
            rows,
            (f32::from(content.size.height) / f32::from(height)).ceil() as usize,
            "{layout:?}: the rows its measured height needs"
        );
        assert_eq!(bounds.size.height, height * rows as f32);
        if layout == DiffLayout::Split {
            let left = painted(cx, &renderer, DiffTextRegion::SplitLeft, "three");
            assert!(
                bounds.left() > left.left() && bounds.left() <= three.left(),
                "drawn in the new side's column: {bounds:?}"
            );
        }
        // A frame with nothing new draws it once, in one column, and
        // measures nothing; after it notifies, it is measured once more.
        let renders = cx.update(|_, app| thread.read(app).renders.clone());
        renders.set(0);
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        assert_eq!(renders.get(), 1, "{layout:?}: drawn once, not measured");
        // The context draws the frame the notify asks for.
        renders.set(0);
        cx.update(|_, app| thread.update(app, |_, cx| cx.notify()));
        assert_eq!(renders.get(), 2, "{layout:?}: measured, then drawn");

        cx.update(|_, app| {
            thread.update(app, |thread, cx| {
                thread.replies += 5;
                cx.notify();
            })
        });
        draw(cx);
        let grown = reserved(cx);
        let content = cx.debug_bounds("thread_root").unwrap();
        assert_eq!(
            grown,
            (f32::from(content.size.height) / f32::from(height)).ceil() as usize
        );
        assert!(grown > rows, "{layout:?}: it grew");
        let moved = painted(cx, &renderer, region, "three");
        assert!(
            (moved.top() - three.top() - height * (grown - rows) as f32).abs() < px(0.5),
            "{layout:?}: three moved down by the rows gained"
        );
    }
}

/// The rows a mounted view reserves: the list's rows past the document's.
fn reserved(cx: &mut gpui::VisualTestContext, renderer: &gpui::Entity<DiffRenderer>) -> usize {
    let rows = cx.update(|_, app| renderer.update(app, |r, _| r.diff_list_len()));
    let document = cx.update(|_, app| renderer.read(app).diff_visible_len());
    rows - document
}

/// A view's rows are counted again in the new row height when the UI scale
/// changes, though the view itself did not notify.
#[gpui::test]
fn inline_view_rows_follow_the_ui_scale(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let Mounted {
        renderer,
        thread,
        cx,
        ..
    } = mount(cx, DiffLayout::Inline, old, new, 2);
    // Fixed-height replies, so the rows they need change with the row height.
    cx.update(|_, app| {
        thread.update(app, |thread, cx| {
            thread.replies = 8;
            cx.notify();
        })
    });
    draw(cx);
    let at_100 = reserved(cx, &renderer);
    for percent in [150, 200] {
        cx.update(|window, app| {
            ui_scale::set_default(app, percent);
            ui_scale::apply_to_window(window, percent);
            window.refresh();
        });
        draw(cx);
        let height = cx.update(|_, app| renderer.read(app).theme.editor_row_height(percent));
        let content = cx.debug_bounds("thread_root").unwrap().size.height;
        let rows = reserved(cx, &renderer);
        assert_eq!(
            rows,
            (f32::from(content) / f32::from(height)).ceil() as usize,
            "{percent}%: the rows {content:?} needs in rows of {height:?}"
        );
        assert!(rows < at_100, "{percent}%: taller rows, fewer of them");
    }
}

/// Keys typed in a view's input are the input's: none runs a diff shortcut,
/// even one the diff would take, such as Escape closing its search. A click
/// in the view runs the view's own handler, starts or moves no diff
/// selection and leaves focus out of the diff.
#[gpui::test]
fn keys_and_clicks_in_an_inline_view_stay_in_it(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let Mounted {
        pane,
        thread,
        renderer,
        cx,
        ..
    } = mount(cx, DiffLayout::Inline, old, new, 2);
    let input = cx.update(|_, app| thread.read(app).input.clone());
    let diff_focus = cx.update(|_, app| renderer.read(app).diff_panel_focus_handle.clone());

    // The diff's own search, which Escape would close.
    click_debug_selector(cx, "diff_text_empty_space_Inline");
    cx.simulate_keystrokes("ctrl-f");
    draw(cx);
    assert!(cx.update(|_, app| renderer.read(app).diff_search_active));
    // The click on the diff's empty space selected its last line.
    let selected = cx.update(|_, app| pane.selection(app));

    // The input's own arrows, as the app binds them.
    cx.update(|_, app| {
        app.bind_keys([
            gpui::KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            gpui::KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
        ])
    });
    click_debug_selector(cx, "thread_input");
    draw(cx);
    let input_focus = cx.update(|_, app| input.read(app).focus_handle());
    assert!(cx.update(|window, _| input_focus.is_focused(window)));
    cx.simulate_keystrokes("a space b left left c right right enter escape f2 f3");
    draw(cx);
    assert_eq!(
        cx.update(|_, app| input.read(app).text().to_string()),
        "ac b"
    );
    assert!(
        cx.update(|_, app| renderer.read(app).diff_search_active),
        "Escape stayed in the view"
    );

    click_debug_selector(cx, "thread_resolve");
    draw(cx);
    assert_eq!(cx.update(|_, app| thread.read(app).resolved), 1);
    cx.update(|window, app| {
        assert!(!diff_focus.is_focused(window), "the diff took no focus");
        assert!(!renderer.read(app).diff_text_selecting);
        assert_eq!(pane.selection(app), selected, "the selection did not move");
    });
}

/// A view stays put while its column scrolls sideways under a long line.
#[gpui::test]
fn inline_views_hold_still_while_the_lines_scroll_sideways(cx: &mut gpui::TestAppContext) {
    let long = "x".repeat(400);
    let Mounted {
        thread,
        renderer,
        id,
        cx,
        ..
    } = mount(
        cx,
        DiffLayout::Inline,
        format!("one\n{long}\nthree\n"),
        format!("one\n{long} changed\nthree\n"),
        1,
    );
    let view = thread_frame(id, &thread);
    let before = cx.debug_bounds(view).unwrap();
    let text = painted(cx, &renderer, DiffTextRegion::Inline, "one");
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            let scroll = renderer.diff_scroll.0.borrow();
            let offset = scroll.base_handle.offset();
            scroll
                .base_handle
                .set_offset(gpui::point(px(-200.0), offset.y));
            cx.notify();
        })
    });
    draw(cx);
    let scrolled = painted(cx, &renderer, DiffTextRegion::Inline, "one");
    assert!(
        scrolled.left() < text.left() - px(100.0),
        "the lines scrolled: {text:?} to {scrolled:?}"
    );
    assert_eq!(cx.debug_bounds(view).unwrap().left(), before.left());
}

/// A view growing above the top line keeps that line where it was.
#[gpui::test]
fn the_top_line_holds_when_a_view_above_it_grows(cx: &mut gpui::TestAppContext) {
    let Mounted {
        thread,
        renderer,
        cx,
        ..
    } = mount(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        2,
    );
    let height = row_height(cx, &renderer);
    let top = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            let renderer = renderer.read(app);
            let document = (0..renderer.diff_visible_len())
                .find(|row| renderer.hosted_line_numbers(*row).1 == Some(50))
                .unwrap();
            renderer.diff_list_index(document)
        })
    };
    let before = top(cx);
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer
                .diff_scroll
                .0
                .borrow()
                .base_handle
                .set_offset(gpui::point(px(0.0), -height * (before as f32 + 0.25)));
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|_, app| {
        thread.update(app, |thread, cx| {
            thread.replies += 6;
            cx.notify();
        })
    });
    draw(cx);
    let after = top(cx);
    assert!(after > before, "the view grew above the line");
    let offset = cx.update(|_, app| {
        renderer
            .read(app)
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y
    });
    assert_eq!(offset, -height * (after as f32 + 0.25));
}

/// Search, copy and a selection across a view read the diff's lines only.
#[gpui::test]
fn search_copy_and_selection_skip_inline_views(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let Mounted { pane, id, cx, .. } = mount(cx, DiffLayout::Inline, old, new, 2);
    assert_eq!(rows_with(cx, &pane, "Resolve"), 0);
    assert_eq!(rows_with(cx, &pane, "Reply"), 0);
    rows_with(cx, &pane, "");
    // Rows: one, two (removed), TWO, the view's rows, three.
    let three = cx.update(|_, app| {
        let renderer = hosted_renderer_of(&pane, app);
        let renderer = renderer.read(app);
        renderer.diff_list_index(3)
    });
    assert!(three > 3, "the view sits before three");
    click_debug_selector(cx, selector(format!("hosted_diff_{id}_row_2")));
    let center = cx
        .debug_bounds(selector(format!("hosted_diff_{id}_row_{three}")))
        .unwrap()
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::shift());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::shift());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::shift());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, app| pane.selection(app)),
        Some(DiffLineRange {
            side: DiffLineSide::New,
            start: 2,
            end: 3
        })
    );
    assert_eq!(
        cx.update(|_, app| pane.selected_text(app)).as_deref(),
        Some("TWO\nthree")
    );
}

fn hosted_renderer_of(pane: &DiffPane, app: &App) -> gpui::Entity<DiffRenderer> {
    pane.view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap()
        .read(app)
        .renderer()
        .cloned()
        .unwrap()
}
