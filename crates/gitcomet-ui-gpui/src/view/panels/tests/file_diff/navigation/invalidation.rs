//! Diff keys redraw the diff, not the other cached panes.

use super::*;

type GitCometView = super::super::super::super::GitCometView;

/// Render counts of the status bar, the sidebar and the details pane, then
/// of the main pane.
fn pane_renders(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> ((usize, usize, usize), usize) {
    cx.update(|_window, app| {
        let root = view.read(app);
        (
            (
                root.bottom_status_bar.read(app).render_count,
                root.sidebar_pane.read(app).render_count,
                root.details_pane.read(app).render_count,
            ),
            root.main_pane.read(app).render_count,
        )
    })
}

/// Draws, then reads the diff pane.
fn read_diff<T: Copy + Default>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    read: impl Fn(&DiffRenderer) -> T,
) -> T {
    let value = std::cell::Cell::new(T::default());
    wait_for_main_pane_condition(
        cx,
        view,
        "the diff pane to be readable",
        |pane| {
            value.set(read(pane));
            true
        },
        |_| (),
    );
    value.get()
}

/// A full diff with three change blocks in a full window, cached views on.
fn cached_nav_shell(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    repo_id: u64,
    fixture_name: &str,
    focus_diff: bool,
) -> crate::view::StableCachedViewsTestGuard {
    cx.simulate_resize(gpui::size(px(1400.0), px(900.0)));
    activate_full_diff_nav_fixture(
        cx,
        view,
        gitcomet_state::model::RepoId(repo_id),
        fixture_name,
        DiffViewMode::Inline,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    if focus_diff {
        focus_diff_panel(cx, view);
    } else {
        cx.update(|window, app| window.blur(app));
    }
    let guard = crate::view::enable_stable_cached_views_for_test();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    guard
}

/// Presses `keys` in turn. Every press after the first must change what
/// `read` returns and redraw the main pane; none of them may re-render the
/// status bar, sidebar or details.
fn assert_keys_redraw_only_the_diff<T: Copy + Default + PartialEq + std::fmt::Debug>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    keys: &[&str],
    read: impl Fn(&DiffRenderer) -> T,
) {
    // The first key after pointer input switches gpui to keyboard modality,
    // which refreshes the window once.
    cx.simulate_keystrokes(keys[0]);
    let mut last = (read_diff(cx, view, &read), pane_renders(cx, view));
    let others_before = last.1.0;
    let mut others = Vec::new();
    for (ix, key) in keys.iter().enumerate().skip(1) {
        cx.simulate_keystrokes(key);
        let now = (read_diff(cx, view, &read), pane_renders(cx, view));
        assert_ne!(now.0, last.0, "press {ix} ({key}) must change the diff");
        assert!(
            now.1.1 > last.1.1,
            "press {ix} ({key}) must redraw the main pane"
        );
        others.push(now.1.0);
        last = now;
    }
    assert_eq!(
        last.1.0, others_before,
        "diff keys must not re-render the status bar, sidebar or details; after each press: {others:?}"
    );
}

/// F2/F3 are bound actions that the root view hands to the diff.
#[gpui::test]
fn diff_navigation_keys_redraw_only_the_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });
    let _cache_guard = cached_nav_shell(cx, &view, 70641, "nav_keys_invalidation", true);
    // The first F3 outlines the first block; the ten after it step to and fro.
    let presses = [
        "f3", "f3", "f2", "f3", "f2", "f3", "f2", "f3", "f2", "f3", "f2",
    ];
    assert_keys_redraw_only_the_diff(cx, &view, &presses, |pane| pane.diff_selection_anchor);
}

/// With focus outside the diff, the keystroke fallback (`app/bindings.rs`)
/// takes the diff's keys that no bound action handled.
#[gpui::test]
fn fallback_diff_keys_redraw_only_the_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        app.clear_key_bindings();
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });
    let _cache_guard = cached_nav_shell(cx, &view, 70642, "fallback_keys_invalidation", false);

    // Alt+W, reveal whitespace: a diff key that leaves focus where it is.
    assert_keys_redraw_only_the_diff(cx, &view, &["alt-w"; 11], |pane| {
        pane.reveal_whitespace_chars
    });
}
