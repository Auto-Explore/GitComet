//! An extension's own views in a hosted diff.

use super::*;
use gitcomet_extension_api::{
    DiffAlign, DiffHostedView, DiffLayout, DiffLineRange, DiffScrollAnchor,
};
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
fn view_frame<V: 'static>(id: u64, thread: &gpui::Entity<V>) -> &'static str {
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
    mount_views_in(cx, layout, old, new, views, false)
}

/// A view that holds a pane, cached as the app's views are: it draws again
/// only when something in it notifies or the window refreshes.
struct CachedPaneHolder(gpui::AnyView);

impl Render for CachedPaneHolder {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.0
                .clone()
                .cached(gpui::StyleRefinement::default().size_full()),
        )
    }
}

/// [`mount_views`], the pane `cached` as the app's views are or not.
fn mount_views_in<T>(
    cx: &mut gpui::TestAppContext,
    layout: DiffLayout,
    old: String,
    new: String,
    views: impl FnOnce(&mut App) -> (T, Vec<DiffHostedView>),
    cached: bool,
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
    let cx = if cached {
        cx.add_window_view(move |_, _| CachedPaneHolder(mounted)).1
    } else {
        cx.add_window_view(move |_, _| PaneHolder(mounted)).1
    };
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
        let view = view_frame(id, &thread);
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
        let view = view_frame(id, &thread);
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
    assert!(cx.debug_bounds(view_frame(id, &thread)).is_none());
    assert!(cx.debug_bounds(view_frame(id, &note)).is_some());
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
    assert!(cx.debug_bounds(view_frame(id, &note)).is_none());
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
    height: Pixels,
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
            .h(self.height)
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
        height: px(60.0),
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
        let frame = cx.debug_bounds(view_frame(id, &hint)).expect("drawn");
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
        let before = cx.debug_bounds(view_frame(id, &hint)).unwrap();
        scroll_x(cx, px(1600.0) - end);
        let after = cx.debug_bounds(view_frame(id, &hint)).unwrap();
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
    let frame = cx.debug_bounds(view_frame(id, &hint)).expect("drawn");
    assert_near(frame.left(), end, "after the cells");
}

/// A streamed CRLF line does not draw its CR, so its view sits after the
/// last drawn cell, not one cell further.
#[gpui::test]
fn end_of_line_views_follow_a_streamed_crlf_line(cx: &mut gpui::TestAppContext) {
    let long = "x".repeat(70_000);
    let (hint, _pane, renderer, id, cx) = mount_hint(
        cx,
        DiffLayout::Inline,
        "one\r\nthree\r\n".into(),
        format!("one\r\n{long}\r\nthree\r\n"),
        2,
        50.0,
    );
    let (_, end) = text_end(cx, &renderer, DiffTextRegion::Inline, "xxx");
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
    let (_, end) = text_end(cx, &renderer, DiffTextRegion::Inline, "xxx");
    let frame = cx.debug_bounds(view_frame(id, &hint)).expect("drawn");
    assert_near(frame.left(), end, "after the last drawn cell");
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
    let frame = cx.debug_bounds(view_frame(id, &hint)).expect("drawn");
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
    assert!(cx.debug_bounds(view_frame(id, &hint)).is_some());
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
    assert!(cx.debug_bounds(view_frame(id, &thread)).is_none());
    assert!(cx.debug_bounds(view_frame(id, &hint)).is_some());
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
    let frame = view_frame(id, &hint);
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

/// A view moved between placements is measured again for its new one: at
/// its column's width between lines, at its own after the text, and back.
#[gpui::test]
fn a_view_moved_between_placements_is_measured_for_its_new_one(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let (hint, pane, renderer, id, cx) = mount_views(cx, DiffLayout::Inline, old, new, |app| {
        let hint = new_hint(app, 50.0);
        (hint.clone(), vec![DiffHostedView::block(on_new(2), hint)])
    });
    let frame = view_frame(id, &hint);
    let block = cx.debug_bounds(frame).unwrap();
    let rows = reserved(cx, &renderer);
    assert!(
        block.size.width > px(400.0),
        "its column's width: {block:?}"
    );
    assert!(rows > 0);

    cx.update(|_, app| {
        pane.set_views(
            vec![DiffHostedView::end_of_line(on_new(2), hint.clone())],
            app,
        )
    });
    draw(cx);
    let end = cx.debug_bounds(frame).unwrap();
    assert_eq!(end.size.width, px(50.0), "its own width");
    assert_eq!(end.size.height, row_height(cx, &renderer));
    assert_eq!(reserved(cx, &renderer), 0);

    cx.update(|_, app| pane.set_views(vec![DiffHostedView::block(on_new(2), hint.clone())], app));
    draw(cx);
    assert_eq!(cx.debug_bounds(frame).unwrap().size, block.size);
    assert_eq!(reserved(cx, &renderer), rows);
}

/// Views after the same line sit side by side after its text, in the
/// order they were given.
#[gpui::test]
fn end_of_line_views_after_one_line_sit_side_by_side_in_order(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let ((first, second), _pane, renderer, id, cx) =
        mount_views(cx, DiffLayout::Inline, old, new, |app| {
            let first = new_hint(app, 60.0);
            let second = new_hint(app, 40.0);
            let views = vec![
                DiffHostedView::end_of_line(on_new(2), first.clone()),
                DiffHostedView::end_of_line(on_new(2), second.clone()),
            ];
            ((first, second), views)
        });
    let (_, end) = text_end(cx, &renderer, DiffTextRegion::Inline, "TWO");
    let first = cx.debug_bounds(view_frame(id, &first)).unwrap();
    let second = cx.debug_bounds(view_frame(id, &second)).unwrap();
    assert_near(first.left(), end, "the first right after the text");
    assert_near(second.left(), first.right(), "the second right after it");
    assert_eq!(first.size.width, px(60.0));
    assert_eq!(second.size.width, px(40.0));
    assert_eq!(first.top(), second.top());
}

/// A view anchored to an old line shows on its row: a removed line's, and a
/// context line's beside one anchored to the same row's new line, in an
/// inline diff; in a split, in the old side's column.
#[gpui::test]
fn end_of_line_views_follow_old_lines_too(cx: &mut gpui::TestAppContext) {
    let on_old = |line| DiffScrollAnchor {
        side: DiffLineSide::Old,
        line,
    };
    for layout in [DiffLayout::Inline, DiffLayout::Split] {
        let (old, new) = three_lines();
        let ((removed, new_one, old_one), _pane, renderer, id, cx) =
            mount_views(cx, layout, old, new, |app| {
                let removed = new_hint(app, 30.0);
                let new_one = new_hint(app, 40.0);
                let old_one = new_hint(app, 50.0);
                let views = vec![
                    DiffHostedView::end_of_line(on_old(2), removed.clone()),
                    DiffHostedView::end_of_line(on_new(1), new_one.clone()),
                    DiffHostedView::end_of_line(on_old(1), old_one.clone()),
                ];
                ((removed, new_one, old_one), views)
            });
        let (old_region, new_region) = match layout {
            DiffLayout::Split => (DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight),
            _ => (DiffTextRegion::Inline, DiffTextRegion::Inline),
        };
        let frame = |cx: &mut gpui::VisualTestContext, view: &gpui::Entity<Hint>| {
            cx.debug_bounds(view_frame(id, view)).unwrap()
        };
        let (two, end) = text_end(cx, &renderer, old_region, "two");
        let removed = frame(cx, &removed);
        assert_near(removed.left(), end, &format!("{layout:?}: after two"));
        assert_eq!(removed.top(), two.top(), "{layout:?}");

        let (old_row, old_end) = text_end(cx, &renderer, old_region, "one");
        let old_one = frame(cx, &old_one);
        let new_one = frame(cx, &new_one);
        assert_eq!(old_one.top(), old_row.top(), "{layout:?}: on one's row");
        assert_eq!(new_one.top(), old_row.top(), "{layout:?}: on one's row");
        match layout {
            DiffLayout::Split => {
                let (_, new_end) = text_end(cx, &renderer, new_region, "one");
                assert_near(old_one.left(), old_end, "after the old side's one");
                assert_near(new_one.left(), new_end, "after the new side's one");
                assert!(old_one.right() < new_one.left(), "each in its column");
            }
            _ => {
                assert_near(new_one.left(), old_end, "the first given first");
                assert_near(old_one.left(), new_one.right(), "then the other");
            }
        }
    }
}

/// In a split with word wrap, each side's view goes on the last row of its
/// own side's text, though the two sides wrap over different rows.
#[gpui::test]
fn end_of_line_views_in_a_wrapped_split_follow_each_sides_last_row(cx: &mut gpui::TestAppContext) {
    let words = |count: usize| {
        (0..count)
            .map(|word| format!("w{word:02}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let on_old = DiffScrollAnchor {
        side: DiffLineSide::Old,
        line: 2,
    };
    let ((left, right), _pane, renderer, id, cx) = mount_views(
        cx,
        DiffLayout::Split,
        format!("one\n{}\nthree\n", words(20)),
        format!("one\n{}\nthree\n", words(80)),
        |app| {
            let left = new_hint(app, 30.0);
            let right = new_hint(app, 30.0);
            let views = vec![
                DiffHostedView::end_of_line(on_old, left.clone()),
                DiffHostedView::end_of_line(on_new(2), right.clone()),
            ];
            ((left, right), views)
        },
    );
    cx.update(|_, app| renderer.update(app, |pane, cx| pane.set_diff_word_wrap(true, cx)));
    draw(cx);
    let (left_row, left_end) = text_end(cx, &renderer, DiffTextRegion::SplitLeft, "w");
    let (right_row, right_end) = text_end(cx, &renderer, DiffTextRegion::SplitRight, "w");
    assert!(
        left_row.top() < right_row.top(),
        "the new side wraps further"
    );
    let left = cx.debug_bounds(view_frame(id, &left)).unwrap();
    let right = cx.debug_bounds(view_frame(id, &right)).unwrap();
    assert_eq!(left.top(), left_row.top(), "on the old side's last row");
    assert_near(left.left(), left_end, "after the old side's text");
    assert_eq!(right.top(), right_row.top(), "on the new side's last row");
    assert_near(right.left(), right_end, "after the new side's text");
}

/// A pane of `old` and `new` with a hint `width` wide over the text below
/// new line `line`, lined up with `align`.
fn mount_over(
    cx: &mut gpui::TestAppContext,
    layout: DiffLayout,
    old: String,
    new: String,
    line: u32,
    align: DiffAlign,
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
        let view = DiffHostedView::over(on_new(line), align, hint.clone());
        (hint, vec![view])
    })
}

/// The layer over `region`'s rows that pane `id` draws its over views in.
fn over_layer(id: u64, region: DiffTextRegion) -> &'static str {
    selector(format!("hosted_diff_{id}_over_{region:?}"))
}

/// Scrolls the inline list to `y`, and draws.
fn scroll_to(cx: &mut gpui::VisualTestContext, renderer: &gpui::Entity<DiffRenderer>, y: Pixels) {
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            let scroll = renderer.diff_scroll.0.borrow();
            let x = scroll.base_handle.offset().x;
            scroll.base_handle.set_offset(gpui::point(x, y));
            cx.notify();
        })
    });
    draw(cx);
}

/// Where new line `line`'s row is in the inline list, counted in rows.
fn list_row(
    cx: &mut gpui::VisualTestContext,
    renderer: &gpui::Entity<DiffRenderer>,
    line: u32,
) -> usize {
    cx.update(|_, app| {
        let renderer = renderer.read(app);
        let document = (0..renderer.diff_visible_len())
            .find(|row| renderer.hosted_line_numbers(*row).1 == Some(line))
            .unwrap();
        renderer.diff_list_index(document)
    })
}

/// A view over the text floats below its line at its own size, at its
/// column's text start or end, in its side's column only, and takes no rows.
#[gpui::test]
fn over_views_float_below_their_line(cx: &mut gpui::TestAppContext) {
    for layout in [DiffLayout::Inline, DiffLayout::Split] {
        for align in [DiffAlign::Start, DiffAlign::End] {
            let (old, new) = three_lines();
            let (hint, _pane, renderer, id, cx) = mount_over(cx, layout, old, new, 1, align, 120.0);
            let region = match layout {
                DiffLayout::Split => DiffTextRegion::SplitRight,
                _ => DiffTextRegion::Inline,
            };
            let what = format!("{layout:?} {align:?}");
            let frame = cx.debug_bounds(view_frame(id, &hint)).expect("drawn");
            let layer = cx.debug_bounds(over_layer(id, region)).unwrap();
            let one = painted(cx, &renderer, region, "one");
            assert_eq!(frame.top(), one.bottom(), "{what}: below its line");
            assert_eq!(frame.size, gpui::size(px(120.0), px(60.0)), "{what}");
            match align {
                DiffAlign::Start => assert_near(frame.left(), one.left(), &what),
                DiffAlign::End => assert_near(frame.right(), layer.right(), &what),
            }
            assert!(frame.left() >= layer.left(), "{what}: in its column");
            assert_eq!(reserved(cx, &renderer), 0, "{what}: no rows");
            if layout == DiffLayout::Split {
                assert!(
                    cx.debug_bounds(over_layer(id, DiffTextRegion::SplitLeft))
                        .is_none()
                );
            }
        }
    }
}

/// A view that would run past the pane's bottom goes above its line; one
/// that fits neither way stays below it, clipped.
#[gpui::test]
fn over_views_move_above_their_line_near_the_bottom(cx: &mut gpui::TestAppContext) {
    let (hint, _pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        71,
        DiffAlign::Start,
        120.0,
    );
    let frame = view_frame(id, &hint);
    let layer = cx
        .debug_bounds(over_layer(id, DiffTextRegion::Inline))
        .unwrap();
    let height = row_height(cx, &renderer);
    let row = list_row(cx, &renderer, 71);
    // Its line's bottom a little above the pane's.
    scroll_to(
        cx,
        &renderer,
        layer.size.height - px(10.0) - height * (row + 1) as f32,
    );
    let line = painted(cx, &renderer, DiffTextRegion::Inline, "line 70");
    assert_near(
        line.bottom(),
        layer.bottom() - px(10.0),
        "the line near the bottom",
    );
    let above = cx.debug_bounds(frame).unwrap();
    assert_eq!(above.bottom(), line.top(), "above its line");

    cx.update(|_, app| {
        hint.update(app, |hint, cx| {
            hint.height = layer.size.height;
            cx.notify();
        })
    });
    draw(cx);
    let line = painted(cx, &renderer, DiffTextRegion::Inline, "line 70");
    assert_eq!(
        cx.debug_bounds(frame).unwrap().top(),
        line.bottom(),
        "fits neither way: below"
    );
}

/// A view over the text holds still while its column scrolls sideways, and
/// moves with its line when the rows scroll.
#[gpui::test]
fn over_views_follow_their_line_but_not_sideways_scrolling(cx: &mut gpui::TestAppContext) {
    let long = "x".repeat(400);
    let (hint, _pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        format!("{long}\n{}", numbered("line", 60, None)),
        format!("{long} changed\n{}", numbered("line", 60, None)),
        1,
        DiffAlign::Start,
        120.0,
    );
    let frame = view_frame(id, &hint);
    let before = cx.debug_bounds(frame).unwrap();
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            let scroll = renderer.diff_scroll.0.borrow();
            let y = scroll.base_handle.offset().y;
            scroll.base_handle.set_offset(gpui::point(px(-200.0), y));
            cx.notify();
        })
    });
    draw(cx);
    let text = painted(cx, &renderer, DiffTextRegion::Inline, "changed");
    assert!(
        text.left() < before.left() - px(100.0),
        "the lines scrolled"
    );
    assert_eq!(cx.debug_bounds(frame).unwrap().origin, before.origin);

    let height = row_height(cx, &renderer);
    scroll_to(cx, &renderer, -height);
    assert_near(
        cx.debug_bounds(frame).unwrap().top(),
        before.top() - height,
        "a row up with its line",
    );
}

/// A view whose line is above the rows on screen still shows while it
/// reaches into them; wholly out of view it is not drawn, and the pane
/// keeps what it knew of it, so it is drawn again without measuring.
#[gpui::test]
fn over_views_show_while_they_reach_into_view(cx: &mut gpui::TestAppContext) {
    let (hint, _pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        31,
        DiffAlign::Start,
        120.0,
    );
    let frame = view_frame(id, &hint);
    let layer = cx
        .debug_bounds(over_layer(id, DiffTextRegion::Inline))
        .unwrap();
    let height = row_height(cx, &renderer);
    let row = list_row(cx, &renderer, 31);
    // Its line's bottom 10px above the rows on screen.
    scroll_to(cx, &renderer, -height * (row + 1) as f32 - px(10.0));
    let shown = cx.debug_bounds(frame).expect("it reaches into view");
    assert_near(shown.top(), layer.top() - px(10.0), "below its line");

    scroll_to(cx, &renderer, -height * (row + 10) as f32);
    assert!(cx.debug_bounds(frame).is_none(), "out of view");
    assert_eq!(
        cx.update(|_, app| renderer.read(app).hosted_view_states()),
        1
    );
    let renders = cx.update(|_, app| hint.read(app).renders.clone());
    renders.set(0);
    scroll_to(cx, &renderer, px(0.0) - height * (row as f32 - 2.0));
    assert!(cx.debug_bounds(frame).is_some(), "back in view");
    assert!(renders.get() > 0);
    renders.set(0);
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    assert_eq!(renders.get(), 1, "drawn, not measured");
}

/// Keys typed in a view over the text are its input's, and a click on its
/// button is the button's, though it floats over the diff's rows.
#[gpui::test]
fn keys_and_clicks_in_an_over_view_stay_in_it(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let (hint, pane, renderer, _, cx) =
        mount_over(cx, DiffLayout::Inline, old, new, 1, DiffAlign::Start, 200.0);
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
    click_debug_selector(cx, "hint_input");
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

/// A view over the text is measured again when it notifies, and no wider
/// than its column.
#[gpui::test]
fn over_views_are_measured_again_when_they_notify(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let (hint, _pane, _renderer, id, cx) =
        mount_over(cx, DiffLayout::Inline, old, new, 1, DiffAlign::End, 120.0);
    let frame = view_frame(id, &hint);
    let renders = cx.update(|_, app| hint.read(app).renders.clone());
    renders.set(0);
    cx.update(|_, app| {
        hint.update(app, |hint, cx| {
            hint.width = px(160.0);
            cx.notify();
        })
    });
    assert_eq!(renders.get(), 2, "measured, then drawn");
    draw(cx);
    assert_eq!(cx.debug_bounds(frame).unwrap().size.width, px(160.0));

    cx.update(|_, app| {
        hint.update(app, |hint, cx| {
            hint.width = px(5000.0);
            cx.notify();
        })
    });
    draw(cx);
    let layer = cx
        .debug_bounds(over_layer(id, DiffTextRegion::Inline))
        .unwrap();
    let wide = cx.debug_bounds(frame).unwrap();
    assert!(
        wide.size.width <= layer.size.width,
        "up to its column's width: {wide:?} in {layer:?}"
    );
    assert_near(wide.right(), layer.right(), "still at the end");
}

/// Where `region`'s text starts, as a row painted it.
fn text_left(
    cx: &mut gpui::VisualTestContext,
    renderer: &gpui::Entity<DiffRenderer>,
    region: DiffTextRegion,
) -> Pixels {
    cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .iter()
            .find(|((_, at), _)| *at == region)
            .map(|(_, hit)| hit.bounds.left())
            .unwrap_or_else(|| panic!("a row is painted in {region:?}"))
    })
}

/// At a row height the list rounds to device pixels, a view far down the
/// list still sits right below its line: it follows the rows as the list
/// lays them out, not as many row heights down.
#[gpui::test]
fn over_views_follow_the_lists_rows_at_a_fractional_row_height(cx: &mut gpui::TestAppContext) {
    let (hint, _pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        numbered("line", 400, None),
        numbered("line", 400, Some(1)),
        301,
        DiffAlign::Start,
        120.0,
    );
    cx.update(|window, app| {
        ui_scale::set_default(app, 115);
        ui_scale::apply_to_window(window, 115);
        window.refresh();
    });
    draw(cx);
    let height = cx.update(|_, app| renderer.read(app).theme.editor_row_height(115));
    assert!(
        f32::from(height).fract() != 0.0,
        "{height:?}: a row height the list rounds"
    );
    let row = list_row(cx, &renderer, 301);
    assert!(row > 300);
    scroll_to(cx, &renderer, -height * (row as f32 - 5.0));
    let line = painted(cx, &renderer, DiffTextRegion::Inline, "line 300");
    assert_near(
        cx.debug_bounds(view_frame(id, &hint)).unwrap().top(),
        line.bottom(),
        "right below its line",
    );
}

/// Rows the pane places above a view's line, an inset's and a block
/// view's, move it down with its line.
#[gpui::test]
fn over_views_sit_below_their_line_past_rows_placed_above_it(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let (hint, pane, renderer, id, cx) = mount_views(cx, DiffLayout::Inline, old, new, |app| {
        let thread = new_thread(app, 1);
        let hint = new_hint(app, 120.0);
        let views = vec![
            DiffHostedView::block(on_new(1), thread),
            DiffHostedView::over(on_new(3), DiffAlign::Start, hint.clone()),
        ];
        (hint, views)
    });
    cx.update(|_, app| {
        pane.set_insets(
            vec![gitcomet_extension_api::DiffInset::new(
                DiffLineSide::New,
                1,
                ["A note".into(), "Its second line".into()],
            )],
            app,
        )
    });
    draw(cx);
    assert!(reserved(cx, &renderer) > 2, "rows placed above line 3");
    let three = painted(cx, &renderer, DiffTextRegion::Inline, "three");
    assert_eq!(
        cx.debug_bounds(view_frame(id, &hint)).unwrap().top(),
        three.bottom()
    );
}

/// A wrapped line's view goes below its last row.
#[gpui::test]
fn over_views_go_below_a_wrapped_lines_last_row(cx: &mut gpui::TestAppContext) {
    let words = (0..80)
        .map(|word| format!("w{word:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (hint, _pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        "one\nthree\n".into(),
        format!("one\n{words}\nthree\n"),
        2,
        DiffAlign::Start,
        120.0,
    );
    cx.update(|_, app| renderer.update(app, |pane, cx| pane.set_diff_word_wrap(true, cx)));
    draw(cx);
    let (last, _) = text_end(cx, &renderer, DiffTextRegion::Inline, "w");
    let first = painted(cx, &renderer, DiffTextRegion::Inline, "w00");
    assert!(last.top() > first.top(), "the line wraps");
    assert_eq!(
        cx.debug_bounds(view_frame(id, &hint)).unwrap().top(),
        last.bottom()
    );
}

/// A view lined up with the text start starts where the text does, past a
/// people column, without line numbers, and in a split's old side.
#[gpui::test]
fn over_views_start_where_the_text_does(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let (hint, pane, renderer, id, inline) =
        mount_over(cx, DiffLayout::Inline, old, new, 1, DiffAlign::Start, 120.0);
    let frame = view_frame(id, &hint);
    assert_near(
        inline.debug_bounds(frame).unwrap().left(),
        text_left(inline, &renderer, DiffTextRegion::Inline),
        "with line numbers",
    );
    inline.update(|_, app| {
        pane.set_annotations(
            Arc::new(
                gitcomet_extension_api::DiffAnnotations::new().with(
                    DiffLineSide::New,
                    1,
                    gitcomet_extension_api::DiffAnnotation::new(gpui::blue())
                        .with_people(["Ada Lovelace", "Grace Hopper"]),
                ),
            ),
            app,
        )
    });
    draw(inline);
    let with_people = inline.debug_bounds(frame).unwrap().left();
    assert_near(
        with_people,
        text_left(inline, &renderer, DiffTextRegion::Inline),
        "past the people column",
    );
    inline.update(|_, app| {
        let mut policy = gitcomet_extension_api::DiffPanePolicy::default();
        policy.line_numbers = false;
        pane.set_policy(policy, app);
    });
    draw(inline);
    let bare = inline.debug_bounds(frame).unwrap().left();
    assert!(bare < with_people, "the gutters went");
    assert_near(
        bare,
        text_left(inline, &renderer, DiffTextRegion::Inline),
        "without line numbers",
    );

    let on_old = DiffScrollAnchor {
        side: DiffLineSide::Old,
        line: 2,
    };
    let (old, new) = three_lines();
    let (hint, _pane, renderer, id, cx) = mount_views(cx, DiffLayout::Split, old, new, |app| {
        let hint = new_hint(app, 120.0);
        let view = DiffHostedView::over(on_old, DiffAlign::Start, hint.clone());
        (hint, vec![view])
    });
    let frame = cx.debug_bounds(view_frame(id, &hint)).unwrap();
    assert_near(
        frame.left(),
        text_left(cx, &renderer, DiffTextRegion::SplitLeft),
        "in the old side's column",
    );
    assert!(frame.right() < text_left(cx, &renderer, DiffTextRegion::SplitRight));
}

/// A view lined up with the text start starts past the annotation column
/// of a pane that shows blame.
#[gpui::test]
fn over_views_start_past_the_annotation_column(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, app_cx) = open_repository(cx);
    let (pane, hint) = app_cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        let mut policy = gitcomet_extension_api::DiffPanePolicy::default();
        policy.blame = true;
        let pane = host
            .create_diff_pane(
                &repository,
                DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
                DiffPaneOptions {
                    policy,
                    layout: DiffLayout::Inline,
                    content_width: Some(900.0),
                    ..Default::default()
                },
                app,
            )
            .unwrap();
        let hint = new_hint(app, 120.0);
        pane.set_views(
            vec![DiffHostedView::over(
                on_new(2),
                DiffAlign::Start,
                hint.clone(),
            )],
            app,
        );
        (pane, hint)
    });
    settle(app_cx, &view, &store, "hosted file", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
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
    draw(cx);
    assert!(cx.update(|_, app| renderer.read(app).annotation_active()));
    let annotation = cx.update(|_, app| renderer.read(app).annotate_column_width_px(100));
    let text = text_left(cx, &renderer, DiffTextRegion::Inline);
    let frame = cx.debug_bounds(view_frame(id, &hint)).unwrap();
    assert!(frame.left() > annotation, "past the annotation column");
    assert_near(frame.left(), text, "where the text starts");
}

/// A line the pane reveals shows its view right below it in the frame it
/// scrolls to.
#[gpui::test]
fn a_revealed_line_shows_its_over_view_below_it(cx: &mut gpui::TestAppContext) {
    let (hint, pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        71,
        DiffAlign::Start,
        120.0,
    );
    assert!(
        cx.debug_bounds(view_frame(id, &hint)).is_none(),
        "far below"
    );
    cx.update(|_, app| pane.reveal(DiffLineSide::New, 71, app));
    draw(cx);
    let line = painted(cx, &renderer, DiffTextRegion::Inline, "line 70");
    assert_eq!(
        cx.debug_bounds(view_frame(id, &hint)).unwrap().top(),
        line.bottom()
    );
}

/// The wheel over a view scrolls the rows under it, and a click beside it
/// reaches them.
#[gpui::test]
fn the_rows_under_an_over_view_still_scroll_and_take_clicks_beside_it(
    cx: &mut gpui::TestAppContext,
) {
    let (hint, pane, renderer, id, cx) = mount_over(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        2,
        DiffAlign::Start,
        120.0,
    );
    let frame = cx.debug_bounds(view_frame(id, &hint)).unwrap();
    let offset = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            renderer
                .read(app)
                .diff_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y
        })
    };
    let before = offset(cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: frame.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-100.0))),
        ..Default::default()
    });
    draw(cx);
    assert!(offset(cx) < before, "the rows scrolled");

    scroll_to(cx, &renderer, px(0.0));
    let frame = cx.debug_bounds(view_frame(id, &hint)).unwrap();
    // Line 4's row, under the view's height but to its right.
    let row = list_row(cx, &renderer, 4);
    let bounds = cx
        .debug_bounds(selector(format!("hosted_diff_{id}_row_{row}")))
        .unwrap();
    assert!(bounds.top() > frame.top() && bounds.bottom() < frame.bottom());
    let at = gpui::point(frame.right() + px(40.0), bounds.center().y);
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    cx.simulate_mouse_down(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, app| pane.selection(app)),
        Some(DiffLineRange {
            side: DiffLineSide::New,
            start: 4,
            end: 4
        })
    );
}

/// A view lined up with the text start is no wider than the room from
/// there to the column's end, which a view lined up with the end fills.
#[gpui::test]
fn a_start_view_is_no_wider_than_the_room_after_the_text_start(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let ((start, end), _pane, renderer, id, cx) =
        mount_views(cx, DiffLayout::Inline, old, new, |app| {
            let start = new_hint(app, 5000.0);
            let end = new_hint(app, 5000.0);
            let views = vec![
                DiffHostedView::over(on_new(1), DiffAlign::Start, start.clone()),
                DiffHostedView::over(on_new(3), DiffAlign::End, end.clone()),
            ];
            ((start, end), views)
        });
    let layer = cx
        .debug_bounds(over_layer(id, DiffTextRegion::Inline))
        .unwrap();
    let start = cx.debug_bounds(view_frame(id, &start)).unwrap();
    let end = cx.debug_bounds(view_frame(id, &end)).unwrap();
    assert_near(
        start.left(),
        text_left(cx, &renderer, DiffTextRegion::Inline),
        "at the text start",
    );
    assert!(end.size.width < px(5000.0), "capped at the column");
    assert_near(
        start.size.width + (start.left() - layer.left()),
        end.size.width,
        "the column less the text start",
    );
}

/// At a row height the list rounds to device pixels, the pane's scroll
/// anchor still names the line at the top of the rows, far down a long
/// diff too.
#[gpui::test]
fn the_scroll_anchor_names_the_top_line_at_a_fractional_row_height(cx: &mut gpui::TestAppContext) {
    let ((), pane, renderer, id, cx) = mount_views(
        cx,
        DiffLayout::Inline,
        numbered("line", 400, None),
        numbered("line", 400, Some(1)),
        |_| ((), Vec::new()),
    );
    cx.update(|window, app| {
        ui_scale::set_default(app, 115);
        ui_scale::apply_to_window(window, 115);
        window.refresh();
    });
    draw(cx);
    let height = cx.update(|_, app| renderer.read(app).theme.editor_row_height(115));
    assert!(f32::from(height).fract() != 0.0, "{height:?}");
    let list_top = cx
        .debug_bounds(selector(format!("hosted_diff_{id}_row_0")))
        .unwrap()
        .top();
    scroll_to(cx, &renderer, -height * 300.0 - px(2.0));
    // The line whose row holds the rows' top: "line N" is new line N + 1.
    let line = cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .values()
            .find(|hit| hit.bounds.top() <= list_top && hit.bounds.bottom() > list_top)
            .map(|hit| hit.painted_text.to_string())
            .unwrap()
    });
    let shown: u32 = line.strip_prefix("line ").unwrap().parse().unwrap();
    assert_eq!(
        cx.update(|_, app| pane.scroll_anchor(app)),
        Some(DiffScrollAnchor {
            side: DiffLineSide::New,
            line: shown + 1
        })
    );
}

/// The line at the top of the rows, as the pane's scroll anchor names it.
fn top_line(cx: &mut gpui::VisualTestContext, pane: &DiffPane) -> Option<DiffScrollAnchor> {
    cx.update(|_, app| pane.scroll_anchor(app))
}

/// Removing tall block views above the top line keeps that line at the top.
#[gpui::test]
fn removing_block_views_above_the_top_line_keeps_it(cx: &mut gpui::TestAppContext) {
    let ((), pane, renderer, _, cx) = mount_views(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        |app| {
            let views = [2, 4]
                .map(|line| DiffHostedView::block(on_new(line), new_thread(app, 20)))
                .into();
            ((), views)
        },
    );
    assert!(reserved(cx, &renderer) > 30, "tall views above");
    let row = list_row(cx, &renderer, 40);
    let height = row_height(cx, &renderer);
    scroll_to(cx, &renderer, -height * row as f32);
    let top = top_line(cx, &pane);
    assert_eq!(top, Some(on_new(40)));
    cx.update(|_, app| pane.set_views(Vec::new(), app));
    draw(cx);
    assert_eq!(reserved(cx, &renderer), 0);
    assert_eq!(top_line(cx, &pane), top, "the same line at the top");
}

/// At a row height the list rounds to device pixels, a block view growing
/// above the top line, far down a long diff, keeps that line at the top.
#[gpui::test]
fn a_growing_block_view_keeps_the_top_line_at_a_fractional_row_height(
    cx: &mut gpui::TestAppContext,
) {
    let (thread, pane, renderer, _, cx) = mount_views(
        cx,
        DiffLayout::Inline,
        numbered("line", 400, None),
        numbered("line", 400, Some(1)),
        |app| {
            let thread = new_thread(app, 1);
            (
                thread.clone(),
                vec![DiffHostedView::block(on_new(2), thread)],
            )
        },
    );
    cx.update(|window, app| {
        ui_scale::set_default(app, 115);
        ui_scale::apply_to_window(window, 115);
        window.refresh();
    });
    draw(cx);
    let asked = cx.update(|_, app| renderer.read(app).theme.editor_row_height(115));
    assert!(f32::from(asked).fract() != 0.0, "{asked:?}");
    let row = list_row(cx, &renderer, 300);
    let height = row_height_of(cx, &renderer);
    scroll_to(cx, &renderer, -height * row as f32);
    let top = top_line(cx, &pane);
    assert_eq!(top, Some(on_new(300)));
    cx.update(|_, app| {
        thread.update(app, |thread, cx| {
            thread.replies += 6;
            cx.notify();
        })
    });
    draw(cx);
    assert_eq!(top_line(cx, &pane), top, "the same line at the top");
}

/// The row height the pane places its rows at: the list's own.
fn row_height_of(
    cx: &mut gpui::VisualTestContext,
    renderer: &gpui::Entity<DiffRenderer>,
) -> Pixels {
    cx.update(|_, app| renderer.read(app).hosted_row_height())
}

/// A row of a block view at the top stays at the top when an earlier view
/// is removed and when it comes back: the view's identity holds its place.
#[gpui::test]
fn the_top_row_of_a_block_view_holds_when_an_earlier_view_goes_and_returns(
    cx: &mut gpui::TestAppContext,
) {
    let ((note, answer), pane, renderer, _, cx) = mount_views(
        cx,
        DiffLayout::Inline,
        numbered("line", 100, None),
        numbered("line", 100, Some(1)),
        |app| {
            let note = new_thread(app, 1);
            let answer = new_thread(app, 40);
            let views = vec![
                DiffHostedView::block(on_new(2), note.clone()),
                DiffHostedView::block(on_new(4), answer.clone()),
            ];
            ((note, answer), views)
        },
    );
    let answer_id = answer.entity_id();
    let at_answer = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            renderer.update(app, |renderer, _| {
                renderer.hosted_view_row_index(answer_id, 8).unwrap()
            })
        })
    };
    let height = row_height_of(cx, &renderer);
    let index = at_answer(cx);
    scroll_to(cx, &renderer, -height * (index as f32 + 0.25));
    for views in [
        vec![DiffHostedView::block(on_new(4), answer.clone())],
        vec![
            DiffHostedView::block(on_new(4), answer.clone()),
            DiffHostedView::block(on_new(2), note.clone()),
        ],
    ] {
        cx.update(|_, app| pane.set_views(views, app));
        draw(cx);
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
        assert_eq!(offset, -height * (at_answer(cx) as f32 + 0.25));
    }
}

/// In a split of a pane that shows blame, a view lined up with the text
/// start is no wider than the room from there to its column's end, in the
/// old side's column past the annotation column and in the new side's.
#[gpui::test]
fn start_views_in_a_split_fit_their_column(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, app_cx) = open_repository(cx);
    let on_old = |line| DiffScrollAnchor {
        side: DiffLineSide::Old,
        line,
    };
    let (pane, hints) = app_cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        let mut policy = gitcomet_extension_api::DiffPanePolicy::default();
        policy.blame = true;
        let pane = host
            .create_diff_pane(
                &repository,
                DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
                DiffPaneOptions {
                    policy,
                    layout: DiffLayout::Split,
                    content_width: Some(1200.0),
                    ..Default::default()
                },
                app,
            )
            .unwrap();
        let hints = [0; 4].map(|_| new_hint(app, 5000.0));
        pane.set_views(
            vec![
                DiffHostedView::over(on_old(2), DiffAlign::Start, hints[0].clone()),
                DiffHostedView::over(on_old(6), DiffAlign::End, hints[1].clone()),
                DiffHostedView::over(on_new(2), DiffAlign::Start, hints[2].clone()),
                DiffHostedView::over(on_new(6), DiffAlign::End, hints[3].clone()),
            ],
            app,
        );
        (pane, hints)
    });
    settle(app_cx, &view, &store, "hosted file", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
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
    draw(cx);
    assert!(cx.update(|_, app| renderer.read(app).annotation_active()));
    for (region, [start, end]) in [
        (DiffTextRegion::SplitLeft, [&hints[0], &hints[1]]),
        (DiffTextRegion::SplitRight, [&hints[2], &hints[3]]),
    ] {
        let layer = cx.debug_bounds(over_layer(id, region)).unwrap();
        let start = cx.debug_bounds(view_frame(id, start)).unwrap();
        let end = cx.debug_bounds(view_frame(id, end)).unwrap();
        assert_near(
            start.left(),
            text_left(cx, &renderer, region),
            &format!("{region:?}: at the text start"),
        );
        assert!(end.size.width < px(5000.0), "{region:?}: capped");
        assert_near(
            start.size.width + (start.left() - layer.left()),
            end.size.width,
            &format!("{region:?}: the column less the text start"),
        );
    }
}

/// When the row height changes, a view over the text far down a pane that
/// is cached as the app's are sits right below its line in the one frame
/// that redraws the pane: the pane counts in the height as gpui snaps it
/// until the list has laid its rows out at it, then in the list's pitch.
#[gpui::test]
fn a_row_height_change_places_the_rows_in_the_frame_that_redraws(cx: &mut gpui::TestAppContext) {
    let (hint, pane, renderer, id, cx) = mount_views_in(
        cx,
        DiffLayout::Inline,
        numbered("line", 300, None),
        numbered("line", 300, Some(1)),
        |app| {
            let hint = new_hint(app, 120.0);
            let view = DiffHostedView::over(on_new(201), DiffAlign::Start, hint.clone());
            (hint, vec![view])
        },
        true,
    );
    let row = list_row(cx, &renderer, 201);
    let height = row_height(cx, &renderer);
    scroll_to(cx, &renderer, -height * (row as f32 - 10.0));
    let asked = cx.update(|_, app| renderer.read(app).theme.editor_row_height(110));
    assert!(f32::from(asked).fract() != 0.0, "{asked:?}: gpui snaps it");
    let frame = view_frame(id, &hint);
    // The one frame the scale change draws.
    cx.update(|window, app| {
        ui_scale::set_default(app, 110);
        ui_scale::apply_to_window(window, 110);
        window.refresh();
        let _ = window.draw(app);
    });
    let line = painted(cx, &renderer, DiffTextRegion::Inline, "line 200");
    assert_near(
        cx.debug_bounds(frame).unwrap().top(),
        line.bottom(),
        "in the frame that redraws",
    );
    // What the pane counts in between frames: the line at the rows' top.
    let top = cx
        .debug_bounds(over_layer(id, DiffTextRegion::Inline))
        .unwrap()
        .top();
    let shown = cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .values()
            .find(|hit| hit.bounds.top() <= top && hit.bounds.bottom() > top)
            .map(|hit| hit.painted_text.to_string())
            .unwrap()
    });
    let shown: u32 = shown.strip_prefix("line ").unwrap().parse().unwrap();
    assert_eq!(top_line(cx, &pane), Some(on_new(shown + 1)), "the top line");
    // The frame it asks for, which counts in the list's own pitch.
    cx.update(|window, app| {
        assert!(
            window.simulate_next_frame(app) > 0,
            "a next frame is asked for"
        );
        let _ = window.draw(app);
    });
    let line = painted(cx, &renderer, DiffTextRegion::Inline, "line 200");
    assert_near(
        cx.debug_bounds(frame).unwrap().top(),
        line.bottom(),
        "in the frame after",
    );
}

/// Dragging a split's divider moves it with the pointer, though a short
/// diff whose columns scroll together shows no scrollbar.
#[gpui::test]
fn the_split_divider_follows_the_pointer(cx: &mut gpui::TestAppContext) {
    let (old, new) = three_lines();
    let ((), pane, renderer, _, cx) =
        mount_views(cx, DiffLayout::Split, old, new, |_| ((), Vec::new()));
    // Wide enough for the divider to move.
    cx.update(|_, app| pane.set_content_width(Some(1600.0), app));
    draw(cx);
    let left_width = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| renderer.read(app).diff_columns().widths[0])
    };
    let before = left_width(cx);
    let start = cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.diff_text_start(
                DiffTextRegion::SplitLeft,
                crate::view::panes::diff::LineNumberCells::One,
                cx,
            )
        })
    });
    // The divider sits right of the left column, whose text starts `start`
    // in.
    let left = text_left(cx, &renderer, DiffTextRegion::SplitLeft) - start;
    let row = painted(cx, &renderer, DiffTextRegion::SplitLeft, "one");
    let at = gpui::point(
        left + before + px(PANE_RESIZE_HANDLE_PX / 2.0),
        row.center().y,
    );
    let button = gpui::MouseButton::Left;
    let step = |ix: usize| at + gpui::point(px(30.0) * ix as f32, px(0.0));
    cx.simulate_mouse_down(at, button, gpui::Modifiers::default());
    for ix in 1..=10 {
        cx.simulate_mouse_move(step(ix), Some(button), gpui::Modifiers::default());
    }
    cx.simulate_mouse_up(step(10), button, gpui::Modifiers::default());
    draw(cx);
    assert_near(
        left_width(cx),
        before + px(300.0),
        "the column grew by the drag",
    );
}

/// In a split whose columns scroll apart, each with its own scrollbar, the
/// wrap width is the drawn rows' text width, and the layer over each
/// column's text stops at its rows, short of its scrollbar.
#[gpui::test]
fn a_split_scrolling_apart_wraps_and_floats_within_each_columns_rows(
    cx: &mut gpui::TestAppContext,
) {
    let on_old = DiffScrollAnchor {
        side: DiffLineSide::Old,
        line: 1,
    };
    let (old, new) = three_lines();
    let ((), _pane, renderer, id, cx) = mount_views(cx, DiffLayout::Split, old, new, |app| {
        let views = vec![
            DiffHostedView::over(on_old, DiffAlign::End, new_hint(app, 80.0)),
            DiffHostedView::over(on_new(1), DiffAlign::End, new_hint(app, 80.0)),
        ];
        ((), views)
    });
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.diff_scroll_sync = crate::view::DiffScrollSync::Horizontal;
            cx.notify();
        })
    });
    draw(cx);
    let pad = rows::diff_canvas_row_horizontal_padding(100);
    for region in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight] {
        let layer = cx.debug_bounds(over_layer(id, region)).unwrap();
        let text = painted(cx, &renderer, region, "one");
        assert_near(layer.right(), text.right() + pad, &format!("{region:?}"));
    }
    let left = painted(cx, &renderer, DiffTextRegion::SplitLeft, "one");
    let right = painted(cx, &renderer, DiffTextRegion::SplitRight, "one");
    let (columns, char_width) = cx.update(|window, app| {
        renderer.update(app, |renderer, cx| {
            let family = renderer.diff_wrap_measure_font_family(cx);
            let char_width = rows::diff_canvas_text_wrap_char_width(
                window,
                family,
                renderer.theme.metrics.editor_font_size_px,
            );
            (renderer.diff_wrap_columns(window, cx).1, char_width)
        })
    });
    let narrower = left.size.width.min(right.size.width);
    assert_eq!(
        columns,
        (f32::from(narrower) / f32::from(char_width)).floor() as usize,
        "the wrap width is the narrower column's drawn text width"
    );
}
