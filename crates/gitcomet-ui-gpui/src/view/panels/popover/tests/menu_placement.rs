//! Context menus show whole whenever the window can hold them: they flip, then
//! slide, and only scroll when taller than the usable window.

use super::*;
use gitcomet_core::domain::{Branch, RepoSpec};

const MARGIN: f32 = 16.0;
const POINTER_GAP: f32 = 8.0;

fn commit_id() -> CommitId {
    CommitId("0123456789abcdef0123456789abcdef01234567".into())
}

/// A commit carrying `branches` local branches, so its menu is long.
fn repo_with_branches(branches: usize) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/menu-placement".into(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("branch-0".into());
    repo.branches = Loadable::Ready(Arc::new(
        (0..branches)
            .map(|ix| Branch {
                name: format!("branch-{ix}"),
                target: commit_id(),
                upstream: None,
                divergence: None,
            })
            .collect(),
    ));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![]));
    repo.tags = Loadable::Ready(Arc::new(vec![]));
    repo.remote_tags = Loadable::Ready(Arc::new(vec![]));
    repo.remotes = Loadable::Ready(Arc::new(vec![]));
    repo
}

fn window_with_commit_menu_repo(
    cx: &mut gpui::TestAppContext,
    branches: usize,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo = repo_with_branches(branches);
    let state = Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..AppState::test_default()
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.state = state.clone();
            this.ui_model
                .update(cx, |model, cx| model.set_state(state.clone(), cx));
            this.popover_host
                .update(cx, |host, _cx| host.state = state.clone());
        });
    });
    (view, cx)
}

fn open_commit_menu_at(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    at: Point<Pixels>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CommitMenu {
                        repo_id: RepoId(1),
                        commit_id: commit_id(),
                    },
                    at,
                    window,
                    cx,
                );
            });
        });
    });
    crate::view::test_support::redraw(cx);
}

fn close_menu(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.close_popover_and_restore_focus(window, cx)
            });
        });
    });
    crate::view::test_support::redraw(cx);
}

fn menu_bounds(cx: &mut gpui::VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds("app_popover")
        .expect("the context menu should be on screen")
}

fn menu_max_scroll(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> Pixels {
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .context_menu_scroll
            .max_offset()
            .y
    })
}

/// The menu's outer height when nothing constrains it.
fn natural_menu_height(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> Pixels {
    cx.simulate_resize(size(px(1200.0), px(2400.0)));
    crate::view::test_support::redraw(cx);
    open_commit_menu_at(view, cx, point(px(100.0), px(40.0)));
    let height = menu_bounds(cx).size.height;
    assert_eq!(
        menu_max_scroll(view, cx),
        px(0.0),
        "a 2400px window must show the whole menu"
    );
    close_menu(view, cx);
    height
}

fn resize(cx: &mut gpui::VisualTestContext, height: Pixels) {
    cx.simulate_resize(size(px(1200.0), height));
    crate::view::test_support::redraw(cx);
}

fn assert_inside_window(menu: Bounds<Pixels>, window_h: Pixels) {
    assert!(
        menu.top() >= px(MARGIN - 0.5) && menu.bottom() <= window_h - px(MARGIN - 0.5),
        "the menu must sit inside the window margins, got {menu:?} in a window {window_h:?} tall"
    );
}

/// Opened low with just over 240px below the pointer, a long menu used to
/// stay below and be cut to that space even though the room above held it.
#[gpui::test]
fn a_long_menu_flips_above_the_pointer_when_only_above_fits(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = window_with_commit_menu_repo(cx, 12);
    let natural = natural_menu_height(&view, cx);
    let below = px(250.0);
    assert!(natural > below + px(40.0), "the fixture menu is too short");

    let pointer_y = px(MARGIN + POINTER_GAP + 20.0) + natural;
    let window_h = pointer_y + px(POINTER_GAP + MARGIN) + below;
    resize(cx, window_h);
    open_commit_menu_at(&view, cx, point(px(100.0), pointer_y));

    let menu = menu_bounds(cx);
    assert_eq!(menu.size.height, natural, "the menu must not be cut short");
    assert!(
        menu.bottom() <= pointer_y - px(POINTER_GAP - 0.5),
        "the menu must open above the pointer, got {menu:?} for a pointer at {pointer_y:?}"
    );
    assert_inside_window(menu, window_h);
    assert_eq!(menu_max_scroll(&view, cx), px(0.0), "nothing may scroll");
}

/// Too tall for either side of the pointer but not for the window, the menu
/// slides over the pointer rather than hiding rows behind a scroll.
#[gpui::test]
fn a_menu_that_fits_neither_side_slides_into_the_window_whole(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = window_with_commit_menu_repo(cx, 12);
    let natural = natural_menu_height(&view, cx);

    let window_h = natural + px(2.0 * MARGIN + 60.0);
    resize(cx, window_h);
    let pointer_y = window_h / 2.0;
    open_commit_menu_at(&view, cx, point(px(100.0), pointer_y));

    let menu = menu_bounds(cx);
    assert_eq!(menu.size.height, natural, "the menu must not be cut short");
    assert_inside_window(menu, window_h);
    assert_eq!(menu_max_scroll(&view, cx), px(0.0), "nothing may scroll");
}

/// Only a menu taller than the whole usable window scrolls, and it then uses
/// all of that height.
#[gpui::test]
fn a_menu_taller_than_the_window_scrolls_inside_the_margins(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = window_with_commit_menu_repo(cx, 12);
    let natural = natural_menu_height(&view, cx);

    let window_h = (natural / 2.0).max(px(200.0));
    resize(cx, window_h);
    open_commit_menu_at(&view, cx, point(px(100.0), window_h / 2.0));

    let menu = menu_bounds(cx);
    assert_inside_window(menu, window_h);
    assert!(
        (menu.size.height - (window_h - px(2.0 * MARGIN))).abs() <= px(1.0),
        "the capped menu should use the whole usable height, got {menu:?} in {window_h:?}"
    );
    assert!(menu_max_scroll(&view, cx) > px(0.0), "the rest must scroll");
}

const GROUP_HEADER: &str = "Local branch branch-1";

/// The debug selector of the open menu's `GROUP_HEADER` row.
fn header_selector(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> &'static str {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let kind = host.popover.clone().expect("a menu is open");
                let model = host.context_menu_model(&kind, cx).expect("a menu model");
                let ix = model
                    .items
                    .iter()
                    .position(|item| {
                        matches!(item, ContextMenuItem::Entry { label, .. } if label == GROUP_HEADER)
                    })
                    .expect("the group header row");
                model
                    .entry_debug_selectors
                    .get(&ix)
                    .map(|selector| selector.to_string())
                    .unwrap_or_else(|| {
                        super::context_menu::context_menu_entry_debug_selector(GROUP_HEADER)
                    })
                    .leak()
            })
        })
    })
}

/// Hovers the ref group header and expands it with Right.
fn expand_group(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    let selector = header_selector(view, cx);
    let header = cx
        .debug_bounds(selector)
        .expect("the ref group header should render");
    cx.simulate_mouse_move(header.center(), None, gpui::Modifiers::none());
    simulate_key_press(cx, "right");
    crate::view::test_support::redraw(cx);
    let expanded = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .expanded_history_ref
            .is_some()
    });
    assert!(expanded, "Right should expand the hovered group");
}

/// Once the pointer is on the menu, expanding a ref group must not move the
/// menu or the header under the pointer, even though the grown menu would now
/// only fit on the other side of the pointer.
#[gpui::test]
fn expanding_a_ref_group_does_not_move_the_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = window_with_commit_menu_repo(cx, 4);
    let collapsed = natural_menu_height(&view, cx);
    open_commit_menu_at(&view, cx, point(px(100.0), px(40.0)));
    expand_group(&view, cx);
    let expanded = menu_bounds(cx).size.height;
    close_menu(&view, cx);
    assert!(expanded > collapsed + px(40.0), "the group adds rows");

    // Room below fits the collapsed menu but not the expanded one; room
    // above fits both.
    let below = collapsed + (expanded - collapsed) / 2.0;
    let pointer_y = px(MARGIN + POINTER_GAP + 10.0) + expanded;
    let window_h = pointer_y + px(POINTER_GAP + MARGIN) + below;
    resize(cx, window_h);
    open_commit_menu_at(&view, cx, point(px(100.0), pointer_y));
    let before = menu_bounds(cx);
    assert!(before.top() > pointer_y, "the collapsed menu opens below");
    let header = header_selector(&view, cx);
    let header_before = cx.debug_bounds(header).expect("header");

    expand_group(&view, cx);

    let after = menu_bounds(cx);
    assert_eq!(after.origin, before.origin, "the menu must stay put");
    assert!(after.size.height > before.size.height, "it grows downward");
    assert_eq!(
        cx.debug_bounds(header).map(|bounds| bounds.origin),
        Some(header_before.origin),
        "the header under the pointer must not move"
    );
    assert_inside_window(after, window_h);
    assert!(
        menu_max_scroll(&view, cx) > px(0.0),
        "what does not fit below scrolls"
    );
}

/// Shrinking the window under a latched menu moves it back inside.
#[gpui::test]
fn a_latched_menu_follows_a_shrinking_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = window_with_commit_menu_repo(cx, 2);
    resize(cx, px(1400.0));
    open_commit_menu_at(&view, cx, point(px(100.0), px(700.0)));
    let menu = menu_bounds(cx);
    cx.simulate_mouse_move(menu.center(), None, gpui::Modifiers::none());

    let window_h = px(700.0);
    resize(cx, window_h);
    assert_inside_window(menu_bounds(cx), window_h);
}

/// At 200% the margins double with everything else.
#[gpui::test]
fn menu_margins_follow_the_ui_scale(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = window_with_commit_menu_repo(cx, 12);
    crate::view::panels::tests::set_ui_scale_percent_for_test(cx, &view, 200);
    let window_h = px(600.0);
    resize(cx, window_h);
    open_commit_menu_at(&view, cx, point(px(100.0), px(300.0)));

    let menu = menu_bounds(cx);
    assert!(
        menu.top() >= px(2.0 * MARGIN - 0.5) && menu.bottom() <= window_h - px(2.0 * MARGIN - 0.5),
        "200% margins are {}px, got {menu:?}",
        2.0 * MARGIN
    );
}

fn scroll_offset(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> Pixels {
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .context_menu_scroll
            .offset()
            .y
    })
}

fn arrows(cx: &mut gpui::VisualTestContext) -> (Option<Bounds<Pixels>>, Option<Bounds<Pixels>>) {
    (
        cx.debug_bounds("context_menu_scroll_up"),
        cx.debug_bounds("context_menu_scroll_down"),
    )
}

/// One animation frame: time passes, queued frame callbacks run, it draws.
fn next_frame(cx: &mut gpui::VisualTestContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update(|window, app| {
        window.simulate_next_frame(app);
        let _ = window.draw(app);
    });
}

/// The selected row's painted bounds.
fn selected_row(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Option<Bounds<Pixels>> {
    let selector = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let ix = host.context_menu_selected_ix?;
                let kind = host.popover.clone()?;
                let model = host.context_menu_model(&kind, cx)?;
                let ContextMenuItem::Entry { label, .. } = &model.items[ix] else {
                    return None;
                };
                let selector: &'static str = model
                    .entry_debug_selectors
                    .get(&ix)
                    .map(|selector| selector.to_string())
                    .unwrap_or_else(|| {
                        super::context_menu::context_menu_entry_debug_selector(label.as_ref())
                    })
                    .leak();
                Some(selector)
            })
        })
    })?;
    cx.debug_bounds(selector)
}

/// Opens the long menu in a window it overflows.
fn open_overflowing_menu(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (view, cx) = window_with_commit_menu_repo(cx, 12);
    resize(cx, px(420.0));
    open_commit_menu_at(&view, cx, point(px(100.0), px(200.0)));
    assert!(
        menu_max_scroll(&view, cx) > px(100.0),
        "the menu must overflow"
    );
    (view, cx)
}

/// Arrow keys past the fold scroll just far enough to show the new row in
/// full: the old scroll anchor jumped by the menu's own window offset and could
/// leave the selection out of sight.
#[gpui::test]
fn keyboard_selection_is_revealed_with_the_least_scroll(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = open_overflowing_menu(cx);
    let viewport = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .context_menu_scroll
            .bounds()
    });

    let mut scrolled_down = false;
    let mut previous_offset = scroll_offset(&view, cx);
    for _ in 0..60 {
        simulate_key_press(cx, "down");
        crate::view::test_support::redraw(cx);
        let Some(row) = selected_row(&view, cx) else {
            continue;
        };
        let (up, down) = arrows(cx);
        let visible_top = up.map_or(viewport.top(), |arrow| arrow.bottom());
        let visible_bottom = down.map_or(viewport.bottom(), |arrow| arrow.top());
        assert!(
            row.top() >= visible_top - px(0.5) && row.bottom() <= visible_bottom + px(0.5),
            "the selected row {row:?} must be fully visible in {visible_top:?}..{visible_bottom:?}"
        );
        let offset = scroll_offset(&view, cx);
        if offset < previous_offset {
            scrolled_down = true;
            if offset > -menu_max_scroll(&view, cx) {
                assert!(
                    (row.bottom() - visible_bottom).abs() <= px(1.0),
                    "scrolling down stops with the row at the bottom edge, got {row:?} vs {visible_bottom:?}"
                );
            }
        }
        previous_offset = offset;
    }
    assert!(scrolled_down, "walking the menu must scroll it");
}

/// The arrows show only where rows are hidden, from the very first frame,
/// including for a short menu opened right after a scrolled long one.
#[gpui::test]
fn scroll_arrows_mark_only_the_edges_with_hidden_rows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = open_overflowing_menu(cx);
    let (up, down) = arrows(cx);
    assert!(up.is_none(), "nothing is hidden above at first");
    assert!(down.is_some(), "rows are hidden below");

    simulate_key_press(cx, "end");
    crate::view::test_support::redraw(cx);
    let (up, down) = arrows(cx);
    assert!(up.is_some(), "rows are hidden above at the end");
    assert!(down.is_none(), "nothing is hidden below at the end");
    close_menu(&view, cx);

    // A short menu, in a window that holds it.
    resize(cx, px(1400.0));
    open_commit_menu_at(&view, cx, point(px(100.0), px(100.0)));
    assert_eq!(arrows(cx), (None, None), "a menu that fits has no arrows");
}

/// Resting the pointer on the down arrow scrolls the menu frame by frame to
/// its end; the arrow then goes away. It blocks the rows under it, the wheel
/// still scrolls through it, and pressing it leaves the menu open.
#[gpui::test]
fn hovering_a_scroll_arrow_scrolls_the_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = open_overflowing_menu(cx);
    let down = arrows(cx).1.expect("a down arrow");
    let max = menu_max_scroll(&view, cx);

    let selected = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .context_menu_selected_ix
        })
    };
    let initial = selected(cx);
    cx.simulate_mouse_move(down.center(), None, gpui::Modifiers::none());
    let mut previous = scroll_offset(&view, cx);
    let mut frames = 0;
    while scroll_offset(&view, cx) > -max + px(0.5) {
        next_frame(cx);
        let offset = scroll_offset(&view, cx);
        assert!(offset <= previous, "the menu only scrolls down");
        previous = offset;
        frames += 1;
        assert!(frames < 400, "hovering must reach the end");
        // Once the arrow goes, the pointer rests on the last row and that row
        // lights up, as native menus do; until then nothing is hovered.
        if arrows(cx).1.is_some() {
            assert_eq!(
                selected(cx),
                initial,
                "the arrow keeps rows from being hovered"
            );
        }
    }
    assert!(
        frames > 1,
        "it scrolls over several frames, not in one jump"
    );
    next_frame(cx);
    assert!(
        arrows(cx).1.is_none(),
        "at the end the down arrow goes away"
    );

    // The wheel scrolls back up even over the up arrow.
    let up = arrows(cx).0.expect("an up arrow at the end");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: up.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        ..Default::default()
    });
    crate::view::test_support::redraw(cx);
    assert!(
        scroll_offset(&view, cx) > -max,
        "the wheel scrolls through an arrow"
    );

    let up = arrows(cx).0.expect("still an up arrow");
    cx.simulate_click(up.center(), gpui::Modifiers::none());
    crate::view::test_support::redraw(cx);
    let open = cx.update(|_window, app| view.read(app).popover_host.read(app).popover.is_some());
    assert!(open, "pressing an arrow keeps the menu open");
}
