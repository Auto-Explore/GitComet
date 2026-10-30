//! An extension's own views in a hosted diff.

use super::*;
use gitcomet_extension_api::{DiffHostedView, DiffLayout, DiffLineRange, DiffScrollAnchor};
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

/// A view's frame, as pane `id` draws it.
fn thread_frame<V: 'static>(id: u64, thread: &gpui::Entity<V>) -> &'static str {
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
    let (thread, pane, renderer, id, cx) = mount_views(cx, layout, old, new, |app| {
        let thread = new_thread(app, 1);
        let view = DiffHostedView::block(
            DiffScrollAnchor {
                side: DiffLineSide::New,
                line,
            },
            thread.clone(),
        );
        (thread, vec![view])
    });
    Mounted {
        pane,
        thread,
        renderer,
        id,
        cx,
    }
}

fn new_thread(app: &mut App, replies: usize) -> gpui::Entity<Thread> {
    app.new(|cx| Thread {
        input: cx.new(|cx| {
            components::TextInput::new_inert(
                components::TextInputOptions {
                    placeholder: "Write a reply".into(),
                    ..Default::default()
                },
                cx,
            )
        }),
        replies,
        resolved: 0,
        renders: Rc::default(),
    })
}

/// A snapshot pane of `old` and `new` showing the views `views` makes, with
/// what else it returns.
fn mount_views<T>(
    cx: &mut gpui::TestAppContext,
    layout: DiffLayout,
    old: String,
    new: String,
    views: impl FnOnce(&mut App) -> (T, Vec<DiffHostedView>),
) -> (
    T,
    DiffPane,
    gpui::Entity<DiffRenderer>,
    u64,
    &mut gpui::VisualTestContext,
) {
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
        let (made, views) = views(app);
        pane.set_views(views, app);
        (pane, made)
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
    (thread, pane, renderer, id, cx)
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
fn block_views_draw_after_their_line_in_rows_that_follow_their_height(
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
fn block_view_rows_follow_the_ui_scale(cx: &mut gpui::TestAppContext) {
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
fn keys_and_clicks_in_a_hosted_view_stay_in_it(cx: &mut gpui::TestAppContext) {
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

/// A view stays put while its column scrolls sideways under a long line,
/// in a split's new-side column as inline.
#[gpui::test]
fn block_views_hold_still_while_the_lines_scroll_sideways(cx: &mut gpui::TestAppContext) {
    let long = "x".repeat(400);
    for layout in [DiffLayout::Inline, DiffLayout::Split] {
        let Mounted {
            thread,
            renderer,
            id,
            cx,
            ..
        } = mount(
            cx,
            layout,
            format!("one\n{long}\nthree\n"),
            format!("one\n{long} changed\nthree\n"),
            1,
        );
        let region = match layout {
            DiffLayout::Split => DiffTextRegion::SplitRight,
            _ => DiffTextRegion::Inline,
        };
        let view = thread_frame(id, &thread);
        let before = cx.debug_bounds(view).unwrap();
        let text = painted(cx, &renderer, region, "one");
        cx.update(|_, app| {
            renderer.update(app, |renderer, cx| {
                let scroll = match layout {
                    DiffLayout::Split => renderer.diff_split_right_scroll.0.borrow(),
                    _ => renderer.diff_scroll.0.borrow(),
                };
                let offset = scroll.base_handle.offset();
                scroll
                    .base_handle
                    .set_offset(gpui::point(px(-200.0), offset.y));
                cx.notify();
            })
        });
        draw(cx);
        let scrolled = painted(cx, &renderer, region, "one");
        assert!(
            scrolled.left() < text.left() - px(100.0),
            "{layout:?}: the lines scrolled: {text:?} to {scrolled:?}"
        );
        assert_eq!(
            cx.debug_bounds(view).unwrap().left(),
            before.left(),
            "{layout:?}"
        );
    }
}

/// Replacing a pane's views drops what it knew of those that went: their
/// rows collapse, their states go and their notifies no longer redraw the
/// pane. Removing them all leaves the document's rows alone.
#[gpui::test]
fn replaced_and_removed_hosted_views_leave_nothing_behind(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let Mounted {
        pane,
        thread,
        renderer,
        id,
        cx,
    } = mount(cx, DiffLayout::Inline, old, new, 2);
    let pane_view = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap();
    cx.update(|_, app| {
        thread.update(app, |thread, cx| {
            thread.replies = 8;
            cx.notify();
        })
    });
    draw(cx);
    let tall = reserved(cx, &renderer);

    // A short note on line 1 takes the thread's place.
    let note = cx.update(|_, app| {
        let note = app.new(|cx| Thread {
            input: cx.new(|cx| {
                components::TextInput::new_inert(components::TextInputOptions::default(), cx)
            }),
            replies: 0,
            resolved: 0,
            renders: Rc::default(),
        });
        pane.set_views(
            vec![DiffHostedView::block(
                DiffScrollAnchor {
                    side: DiffLineSide::New,
                    line: 1,
                },
                note.clone(),
            )],
            app,
        );
        note
    });
    draw(cx);
    assert!(cx.debug_bounds(thread_frame(id, &thread)).is_none());
    assert!(cx.debug_bounds(thread_frame(id, &note)).is_some());
    let short = reserved(cx, &renderer);
    let content = cx.debug_bounds("thread_root").unwrap().size.height;
    let height = row_height(cx, &renderer);
    assert_eq!(
        short,
        (f32::from(content) / f32::from(height)).ceil() as usize
    );
    assert!(short < tall, "the thread's rows went with it");
    assert_eq!(
        cx.update(|_, app| renderer.read(app).hosted_view_states()),
        1
    );

    // The thread is no longer watched: its notify redraws nothing.
    let renders = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| pane_view.read(app).rendered_statuses().len())
    };
    let before = renders(cx);
    cx.update(|_, app| thread.update(app, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(renders(cx), before, "the old view's notify reaches no pane");
    cx.update(|_, app| note.update(app, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert!(renders(cx) > before, "the new view's does");

    cx.update(|_, app| pane.set_views(Vec::new(), app));
    draw(cx);
    assert_eq!(reserved(cx, &renderer), 0);
    assert!(cx.debug_bounds(thread_frame(id, &note)).is_none());
    assert_eq!(
        cx.update(|_, app| renderer.read(app).hosted_view_states()),
        0
    );
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
fn search_copy_and_selection_skip_block_views(cx: &mut gpui::TestAppContext) {
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

/// A hint after a line: a button and a reply box centred in a row of a set
/// width, taller than a diff row so that its clipping shows.
struct Hint {
    input: gpui::Entity<components::TextInput>,
    width: Pixels,
    clicks: usize,
    renders: Rc<Cell<usize>>,
}

impl Render for Hint {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
            .debug_selector(|| "hint_root".into())
            .flex()
            .items_center()
            .w(self.width)
            .h(px(60.0))
            .child(
                div()
                    .id("hint_button")
                    .debug_selector(|| "hint_button".into())
                    .flex_none()
                    .w(px(16.0))
                    .h(px(8.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clicks += 1;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .debug_selector(|| "hint_input".into())
                    .flex_1()
                    .child(self.input.clone()),
            )
    }
}

fn new_hint(app: &mut App, width: f32) -> gpui::Entity<Hint> {
    app.new(|cx| Hint {
        input: cx.new(|cx| {
            components::TextInput::new_inert(components::TextInputOptions::default(), cx)
        }),
        width: px(width),
        clicks: 0,
        renders: Rc::default(),
    })
}

fn on_new(line: u32) -> DiffScrollAnchor {
    DiffScrollAnchor {
        side: DiffLineSide::New,
        line,
    }
}

/// A pane of `old` and `new` with a hint `width` wide after new line
/// `line`.
fn mount_hint(
    cx: &mut gpui::TestAppContext,
    layout: DiffLayout,
    old: String,
    new: String,
    line: u32,
    width: f32,
) -> (
    gpui::Entity<Hint>,
    DiffPane,
    gpui::Entity<DiffRenderer>,
    u64,
    &mut gpui::VisualTestContext,
) {
    mount_views(cx, layout, old, new, |app| {
        let hint = new_hint(app, width);
        let view = DiffHostedView::end_of_line(on_new(line), hint.clone());
        (hint, vec![view])
    })
}

/// The text bounds of the last row in `region` that painted `text`, and
/// where its text ends: its shaped line's width, or its monospace cells'
/// when the line was streamed.
fn text_end(
    cx: &mut gpui::VisualTestContext,
    renderer: &gpui::Entity<DiffRenderer>,
    region: DiffTextRegion,
    text: &str,
) -> (Bounds<Pixels>, Pixels) {
    cx.update(|_, app| {
        let renderer = renderer.read(app);
        let hit = renderer
            .diff_text_hitboxes
            .iter()
            .filter(|((_, at), hit)| *at == region && hit.painted_text.contains(text))
            .map(|(_, hit)| hit)
            .max_by(|a, b| f32::from(a.bounds.top()).total_cmp(&f32::from(b.bounds.top())))
            .unwrap_or_else(|| panic!("{text} is painted in {region:?}"));
        let width = match hit.streamed_ascii_monospace_cell_width {
            Some(cell) => cell * hit.text_len as f32,
            None => {
                renderer
                    .diff_text_layout_cache
                    .get(&hit.layout_key)
                    .expect("its layout is cached")
                    .layout
                    .width
            }
        };
        (hit.bounds, hit.bounds.left() + width)
    })
}

fn assert_near(a: Pixels, b: Pixels, what: &str) {
    assert!((a - b).abs() < px(0.5), "{what}: {a:?} against {b:?}");
}

/// A view at a line's end sits on its line's row right after the text, at
/// its own width and one row high, in its side's column only, and takes no
/// rows of the list.
#[gpui::test]
fn end_of_line_views_sit_after_their_lines_text_in_no_rows_of_their_own(
    cx: &mut gpui::TestAppContext,
) {
    for layout in [DiffLayout::Inline, DiffLayout::Split] {
        let (old, new) = three_lines();
        let (hint, _pane, renderer, id, cx) = mount_hint(cx, layout, old, new, 2, 50.0);
        let region = match layout {
            DiffLayout::Split => DiffTextRegion::SplitRight,
            _ => DiffTextRegion::Inline,
        };
        let frame = cx.debug_bounds(thread_frame(id, &hint)).expect("drawn");
        let (text, end) = text_end(cx, &renderer, region, "TWO");
        assert_near(frame.left(), end, &format!("{layout:?}: after the text"));
        assert_eq!(frame.top(), text.top(), "{layout:?}: on its line's row");
        let content = cx.debug_bounds("hint_root").unwrap();
        assert_near(
            content.center().y,
            frame.center().y,
            &format!("{layout:?}: centred on the row"),
        );
        assert_eq!(frame.size.width, px(50.0), "{layout:?}: its own width");
        assert_eq!(
            frame.size.height,
            row_height(cx, &renderer),
            "{layout:?}: one row high, though the hint is taller"
        );
        assert_eq!(reserved(cx, &renderer), 0, "{layout:?}: no rows");
        if layout == DiffLayout::Split {
            let left = painted(cx, &renderer, DiffTextRegion::SplitLeft, "two");
            assert!(
                frame.left() > left.right(),
                "in the new side's column: {frame:?}"
            );
        }
    }
}

/// A view at a line's end moves with the text when its column scrolls
/// sideways, and its column's scroll range makes room for it.
#[gpui::test]
fn end_of_line_views_scroll_sideways_with_the_text(cx: &mut gpui::TestAppContext) {
    let long = "x".repeat(600);
    for layout in [DiffLayout::Inline, DiffLayout::Split] {
        let (hint, _pane, renderer, id, cx) = mount_hint(
            cx,
            layout,
            format!("one\n{long}\nthree\n"),
            format!("one\n{long} changed\nthree\n"),
            2,
            400.0,
        );
        let (region, column) = match layout {
            DiffLayout::Split => (DiffTextRegion::SplitRight, 1),
            _ => (DiffTextRegion::Inline, 0),
        };
        let (text, end) = text_end(cx, &renderer, region, "changed");
        let scroll_width =
            cx.update(|_, app| renderer.read(app).diff_horizontal_scroll.content_widths[column]);
        assert!(
            scroll_width - (end - text.left()) >= px(400.0),
            "{layout:?}: the row's scroll width {scroll_width:?} holds the view"
        );
        // Two offsets that both show the line's end; the range ends where
        // the view does.
        let scroll_x = |cx: &mut gpui::VisualTestContext, x: Pixels| {
            cx.update(|_, app| {
                renderer.update(app, |renderer, cx| {
                    let scroll = match layout {
                        DiffLayout::Split => renderer.diff_split_right_scroll.0.borrow(),
                        _ => renderer.diff_scroll.0.borrow(),
                    };
                    let offset = scroll.base_handle.offset();
                    scroll.base_handle.set_offset(gpui::point(x, offset.y));
                    cx.notify();
                })
            });
            draw(cx);
        };
        scroll_x(cx, px(1750.0) - end);
        let before = cx.debug_bounds(thread_frame(id, &hint)).unwrap();
        scroll_x(cx, px(1600.0) - end);
        let after = cx.debug_bounds(thread_frame(id, &hint)).unwrap();
        assert_near(
            after.left(),
            before.left() - px(150.0),
            &format!("{layout:?}: it scrolled with the text"),
        );
        let (_, end) = text_end(cx, &renderer, region, "changed");
        assert_near(after.left(), end, &format!("{layout:?}: still after it"));
    }
}

/// A line long enough to stream in monospace cells ends where its cells do.
#[gpui::test]
fn end_of_line_views_follow_a_streamed_line(cx: &mut gpui::TestAppContext) {
    let long = "x".repeat(70_000);
    let (hint, _pane, renderer, id, cx) = mount_hint(
        cx,
        DiffLayout::Inline,
        "one\nthree\n".into(),
        format!("one\n{long}\nthree\n"),
        2,
        50.0,
    );
    let (_, end) = text_end(cx, &renderer, DiffTextRegion::Inline, "xxx");
    // To the line's end, where the view is.
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            let scroll = renderer.diff_scroll.0.borrow();
            let offset = scroll.base_handle.offset();
            scroll
                .base_handle
                .set_offset(gpui::point(offset.x - end + px(300.0), offset.y));
            cx.notify();
        })
    });
    draw(cx);
    let streamed = cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .values()
            .any(|hit| hit.streamed_ascii_monospace_cell_width.is_some())
    });
    assert!(streamed, "the line is streamed");
    let (_, end) = text_end(cx, &renderer, DiffTextRegion::Inline, "xxx");
    let frame = cx.debug_bounds(thread_frame(id, &hint)).expect("drawn");
    assert_near(frame.left(), end, "after the cells");
}

/// A wrapped line shows its view after the text of its last row.
#[gpui::test]
fn end_of_line_views_go_on_a_wrapped_lines_last_row(cx: &mut gpui::TestAppContext) {
    let words = (0..80)
        .map(|word| format!("w{word:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (hint, _pane, renderer, id, cx) = mount_hint(
        cx,
        DiffLayout::Inline,
        "one\nthree\n".into(),
        format!("one\n{words}\nthree\n"),
        2,
        50.0,
    );
    cx.update(|_, app| renderer.update(app, |pane, cx| pane.set_diff_word_wrap(true, cx)));
    draw(cx);
    let rows = cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .values()
            .filter(|hit| hit.painted_text.contains("w"))
            .count()
    });
    assert!(rows > 1, "the line wraps over {rows} rows");
    let (last, end) = text_end(cx, &renderer, DiffTextRegion::Inline, "w");
    let frame = cx.debug_bounds(thread_frame(id, &hint)).expect("drawn");
    assert_eq!(frame.top(), last.top(), "on the last row");
    assert_near(frame.left(), end, "after its text");
    assert_eq!(reserved(cx, &renderer), 0);
}

/// Keys typed in a line end view's input are the input's, and a click on
/// its button is the button's: neither reaches the diff.
#[gpui::test]
fn keys_and_clicks_in_an_end_of_line_view_stay_in_it(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let (hint, pane, renderer, _, cx) = mount_hint(cx, DiffLayout::Inline, old, new, 2, 200.0);
    let diff_focus = cx.update(|_, app| renderer.read(app).diff_panel_focus_handle.clone());
    click_debug_selector(cx, "diff_text_empty_space_Inline");
    cx.simulate_keystrokes("ctrl-f");
    draw(cx);
    assert!(cx.update(|_, app| renderer.read(app).diff_search_active));
    let selected = cx.update(|_, app| pane.selection(app));

    click_debug_selector(cx, "hint_button");
    draw(cx);
    assert_eq!(cx.update(|_, app| hint.read(app).clicks), 1);
    cx.update(|window, app| {
        assert!(!diff_focus.is_focused(window), "the diff took no focus");
        assert!(!renderer.read(app).diff_text_selecting);
        assert_eq!(pane.selection(app), selected, "the selection did not move");
    });

    cx.update(|_, app| {
        app.bind_keys([
            gpui::KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            gpui::KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
        ])
    });
    // Its middle, on the row's: the row clips the rest.
    let at = cx.debug_bounds("hint_input").unwrap().center();
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    draw(cx);
    let input = cx.update(|_, app| hint.read(app).input.clone());
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
    cx.update(|_, app| assert_eq!(pane.selection(app), selected));
}

/// Only a block view reserves rows: with one of each, the list has the
/// block view's rows; with the line end view alone, the document's only.
#[gpui::test]
fn only_block_views_reserve_rows(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let ((thread, hint), pane, renderer, id, cx) =
        mount_views(cx, DiffLayout::Inline, old, new, |app| {
            let thread = new_thread(app, 1);
            let hint = new_hint(app, 50.0);
            let views = vec![
                DiffHostedView::block(on_new(1), thread.clone()),
                DiffHostedView::end_of_line(on_new(2), hint.clone()),
            ];
            ((thread, hint), views)
        });
    assert!(cx.debug_bounds(thread_frame(id, &hint)).is_some());
    let content = cx.debug_bounds("thread_root").unwrap().size.height;
    let height = row_height(cx, &renderer);
    assert_eq!(
        reserved(cx, &renderer),
        (f32::from(content) / f32::from(height)).ceil() as usize,
        "the block view's rows only"
    );
    cx.update(|_, app| {
        pane.set_views(
            vec![DiffHostedView::end_of_line(on_new(2), hint.clone())],
            app,
        )
    });
    draw(cx);
    assert!(cx.debug_bounds(thread_frame(id, &thread)).is_none());
    assert!(cx.debug_bounds(thread_frame(id, &hint)).is_some());
    assert_eq!(reserved(cx, &renderer), 0);
}

/// A line end view is measured again when it notifies, but not when the
/// views are replaced around it or its line scrolls away and back; while
/// its line is away it is not drawn, and the pane keeps what it knew.
#[gpui::test]
fn an_end_of_line_view_is_measured_once_until_it_notifies(cx: &mut gpui::TestAppContext) {
    let (hint, pane, renderer, id, cx) = mount_hint(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        2,
        50.0,
    );
    let frame = thread_frame(id, &hint);
    let renders = cx.update(|_, app| hint.read(app).renders.clone());
    let one_frame = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    };

    renders.set(0);
    // The context draws the frame the replacement asks for.
    cx.update(|_, app| {
        let thread = new_thread(app, 1);
        pane.set_views(
            vec![
                DiffHostedView::block(on_new(1), thread),
                DiffHostedView::end_of_line(on_new(2), hint.clone()),
            ],
            app,
        )
    });
    assert_eq!(renders.get(), 1, "replaced around it: drawn, not measured");
    assert_eq!(
        cx.update(|_, app| renderer.read(app).hosted_view_states()),
        2
    );

    renders.set(0);
    cx.update(|_, app| {
        hint.update(app, |hint, cx| {
            hint.width = px(90.0);
            cx.notify();
        })
    });
    assert_eq!(renders.get(), 2, "measured, then drawn");
    draw(cx);
    assert_eq!(cx.debug_bounds(frame).unwrap().size.width, px(90.0));

    let height = row_height(cx, &renderer);
    let scroll_to = |cx: &mut gpui::VisualTestContext, y: Pixels| {
        cx.update(|_, app| {
            renderer.update(app, |renderer, cx| {
                renderer
                    .diff_scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(gpui::point(px(0.0), y));
                cx.notify();
            })
        });
        draw(cx);
    };
    scroll_to(cx, -height * 60.0);
    assert!(cx.debug_bounds(frame).is_none(), "its line is away");
    assert_eq!(
        cx.update(|_, app| renderer.read(app).hosted_view_states()),
        2
    );
    renders.set(0);
    scroll_to(cx, px(0.0));
    assert!(renders.get() > 0, "drawn again");
    assert_eq!(
        cx.debug_bounds(frame).unwrap().size.width,
        px(90.0),
        "at the width it was measured at"
    );
    renders.set(0);
    one_frame(cx);
    assert_eq!(renders.get(), 1, "back in view: drawn, not measured");
}
