use super::*;

struct NavigationFixture {
    root: Entity<GitCometView>,
    history: Entity<HistoryView>,
    store: AppStore,
    commits: Vec<Commit>,
}

fn mount_navigation(
    cx: &mut gpui::TestAppContext,
    indexed: bool,
    count: usize,
) -> (NavigationFixture, &mut gpui::VisualTestContext) {
    let (index, commits) = super::refresh::indexed_fixture(count);
    // Most indexed destinations have no loaded text. Navigation must resolve
    // them against the projection, not stop at the end of this initial page.
    let loaded = if indexed { count.min(4) } else { count };
    let (root, cx, mut state, store) =
        super::refresh::mount(cx, Arc::new(log_page(commits[..loaded].to_vec(), None)));
    state.repos[0].history_state.selected_commit = Some(commits[0].id.clone());
    if indexed {
        install_index(&mut state, index.clone());
    }
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &root, Arc::new(state));
    let history = cx.update(|_, app| root.read(app).main_pane.read(app).history_view.clone());
    if indexed {
        wait_until(cx, "indexed navigation layout and publication", |cx| {
            cx.debug_bounds("indexed_history_viewport").is_some()
                && store.snapshot().repos[0]
                    .history_state
                    .indexed
                    .displayed_index
                    .as_ref()
                    .is_some_and(|shown| Arc::ptr_eq(shown, &index))
        });
    }
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::bind_text_input_keys_for_test(app);
        window.activate();
        let focus = history.read(app).history_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    (
        NavigationFixture {
            root,
            history,
            store,
            commits,
        },
        cx,
    )
}

impl NavigationFixture {
    // Measure rendered geometry independently of the navigation helper.
    fn metrics(&self, cx: &mut gpui::VisualTestContext) -> (f64, f64, f64) {
        cx.update(|_, app| {
            self.history.update(app, |history, _| {
                if history.indexed.presentation.is_some() {
                    let scroll = history.scroll_interaction.borrow();
                    let logical = scroll.logical.as_ref().unwrap();
                    return (logical.position(), logical.viewport, logical.height);
                }
                let commits = history
                    .history_cache
                    .as_ref()
                    .unwrap()
                    .base
                    .visible_indices
                    .len();
                let count = history.ensure_history_list_plan().list_len(commits);
                let scroll = history.history_scroll.0.borrow();
                let size = scroll.last_item_size.unwrap();
                (
                    -f64::from(f32::from(scroll.base_handle.offset().y)),
                    f64::from(f32::from(size.item.height)),
                    f64::from(f32::from(size.contents.height)) / count as f64,
                )
            })
        })
    }

    fn page_rows(&self, cx: &mut gpui::VisualTestContext) -> usize {
        let (_, viewport, height) = self.metrics(cx);
        (viewport / height).floor().max(1.0) as usize
    }

    fn resize_to_rows(&self, cx: &mut gpui::VisualTestContext, rows: f64) {
        let (_, viewport, height) = self.metrics(cx);
        let size = cx.update(|window, _| window.window_bounds().get_bounds().size);
        cx.simulate_resize(gpui::size(
            size.width,
            size.height + px((rows * height - viewport) as f32),
        ));
        sync_view_with_store(cx, &self.root);
        assert_eq!(self.page_rows(cx), rows.floor() as usize);
    }

    fn assert_commit(&self, cx: &mut gpui::VisualTestContext, index: usize) {
        let expected = &self.commits[index].id;
        wait_for_store(cx, &self.root, "page destination selected", || {
            self.store.snapshot().repos[0]
                .history_state
                .selected_commit
                .as_ref()
                == Some(expected)
        });
        cx.update(|_, app| {
            let history = self.history.read(app);
            assert_eq!(
                history
                    .active_repo()
                    .unwrap()
                    .history_state
                    .selected_commit
                    .as_ref(),
                Some(expected)
            );
        });
    }

    fn assert_visible(&self, cx: &mut gpui::VisualTestContext, row: usize) {
        let (position, viewport, height) = self.metrics(cx);
        let top = row as f64 * height;
        assert!(top >= position - 0.01, "destination above the viewport");
        assert!(
            top + height <= position + viewport + 0.01,
            "destination below the viewport"
        );
    }
}

fn paging_tracks_viewport_and_repeated_keys(cx: &mut gpui::TestAppContext, indexed: bool) {
    let (fixture, cx) = mount_navigation(cx, indexed, 300);
    fixture.resize_to_rows(cx, 10.5);
    cx.simulate_keystrokes("pagedown");
    fixture.assert_commit(cx, 10);
    fixture.assert_visible(cx, 10);
    cx.simulate_keystrokes("pageup");
    fixture.assert_commit(cx, 0);
    fixture.assert_visible(cx, 0);
    cx.simulate_keystrokes("down");
    fixture.assert_commit(cx, 1);
    cx.simulate_keystrokes("up");
    fixture.assert_commit(cx, 0);

    // No store snapshot is synchronized between these inputs. Each key must
    // step from the previous intent, and only its final target is selected.
    cx.simulate_keystrokes("pagedown pagedown pagedown");
    cx.update(|_, app| {
        let history = fixture.history.read(app);
        assert_eq!(history.pending_history_selections.len(), 3);
    });
    fixture.assert_commit(cx, 30);
    fixture.assert_visible(cx, 30);

    fixture.resize_to_rows(cx, 20.5);
    cx.simulate_keystrokes("pagedown");
    fixture.assert_commit(cx, 50);
    fixture.assert_visible(cx, 50);

    cx.update(|window, app| {
        crate::ui_scale::set_default(app, 150);
        crate::ui_scale::apply_to_window(window, 150);
        fixture.history.update(app, |history, cx| {
            history.ui_scale_percent = 150;
            cx.notify();
        });
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    let rows = fixture.page_rows(cx);
    assert!(
        rows > 0 && rows < 20,
        "larger rows reduce the page distance"
    );
    cx.simulate_keystrokes("pagedown");
    fixture.assert_commit(cx, 50 + rows);
    fixture.assert_visible(cx, 50 + rows);
}

#[gpui::test]
fn cached_history_paging_tracks_viewport_and_repeated_keys(cx: &mut gpui::TestAppContext) {
    paging_tracks_viewport_and_repeated_keys(cx, false);
}

#[gpui::test]
fn indexed_history_paging_tracks_viewport_and_repeated_keys(cx: &mut gpui::TestAppContext) {
    paging_tracks_viewport_and_repeated_keys(cx, true);
}

fn paging_clamps_and_respects_focus(cx: &mut gpui::TestAppContext, indexed: bool) {
    let (fixture, cx) = mount_navigation(cx, indexed, 3);
    assert!(fixture.page_rows(cx) > 3);
    cx.simulate_keystrokes("pagedown pagedown");
    fixture.assert_commit(cx, 2);
    fixture.assert_visible(cx, 2);
    cx.simulate_keystrokes("pageup pageup");
    fixture.assert_commit(cx, 0);
    fixture.assert_visible(cx, 0);

    // Modified keys belong to text selection, repository tabs and other
    // contexts, even when the history list has focus.
    cx.simulate_keystrokes("shift-pagedown alt-pagedown secondary-pagedown");
    cx.update(|_, app| {
        assert!(
            fixture
                .history
                .read(app)
                .pending_history_selections
                .is_empty()
        );
    });
    cx.simulate_keystrokes("secondary-f");
    sync_view_with_store(cx, &fixture.root);
    cx.update(|window, app| {
        let history = fixture.history.read(app);
        let input = &history.find.as_ref().unwrap().input;
        assert!(input.read(app).focus_handle().is_focused(window));
    });
    cx.simulate_keystrokes("pagedown pageup");
    cx.update(|_, app| {
        assert!(
            fixture
                .history
                .read(app)
                .pending_history_selections
                .is_empty()
        );
    });
    fixture.assert_commit(cx, 0);

    cx.simulate_keystrokes("escape pagedown");
    fixture.assert_commit(cx, 2);
}

#[gpui::test]
fn cached_history_paging_clamps_and_respects_focus(cx: &mut gpui::TestAppContext) {
    paging_clamps_and_respects_focus(cx, false);
}

#[gpui::test]
fn indexed_history_paging_clamps_and_respects_focus(cx: &mut gpui::TestAppContext) {
    paging_clamps_and_respects_focus(cx, true);
}

fn paging_includes_worktree_rows(cx: &mut gpui::TestAppContext, indexed: bool) {
    use gitcomet_core::domain::{FileStatus, FileStatusKind, WorktreeDirtySummary};

    let (fixture, cx) = mount_navigation(cx, indexed, 100);
    let path = PathBuf::from("/tmp/history-page-linked-worktree");
    let mut state = (*fixture.store.snapshot()).clone();
    let repo = &mut state.repos[0];
    repo.history_state.selected_commit = None;
    repo.worktree_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: "README.md".into(),
        kind: FileStatusKind::Modified,
        conflict: None,
    }]));
    repo.worktree_status_rev += 1;
    repo.worktree_dirty = Loadable::Ready(Arc::new(vec![WorktreeDirtySummary {
        path: path.clone(),
        head: Some(fixture.commits[9].id.clone()),
        branch: Some("linked".into()),
        detached: false,
        added: 1,
        modified: 0,
        deleted: 0,
        staged: Vec::new(),
        unstaged: Vec::new(),
        line_stats: Default::default(),
    }]));
    repo.worktree_dirty_rev += 1;
    fixture
        .store
        .replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &fixture.root, Arc::new(state));
    fixture.resize_to_rows(cx, 10.5);

    // The working tree is row 0. The linked worktree sits at row 10,
    // immediately above commit 9, so exactly one page must select it.
    cx.simulate_keystrokes("pagedown");
    wait_for_store(
        cx,
        &fixture.root,
        "linked worktree selected by page",
        || {
            fixture.store.snapshot().repos[0]
                .history_state
                .worktree_selection
                .as_ref()
                == Some(&path)
        },
    );
    fixture.assert_visible(cx, 10);
    cx.simulate_keystrokes("pagedown");
    fixture.assert_commit(cx, 18);
    fixture.assert_visible(cx, 20);
    cx.simulate_keystrokes("pageup pageup");
    wait_for_store(cx, &fixture.root, "working tree selected by page", || {
        let snapshot = fixture.store.snapshot();
        let history = &snapshot.repos[0].history_state;
        history.selected_commit.is_none() && history.worktree_selection.is_none()
    });
    fixture.assert_visible(cx, 0);
}

#[gpui::test]
fn cached_history_paging_includes_worktree_rows(cx: &mut gpui::TestAppContext) {
    paging_includes_worktree_rows(cx, false);
}

#[gpui::test]
fn indexed_history_paging_includes_worktree_rows(cx: &mut gpui::TestAppContext) {
    paging_includes_worktree_rows(cx, true);
}

#[gpui::test]
fn cached_history_paging_uses_retained_rows_while_loading(cx: &mut gpui::TestAppContext) {
    let (fixture, cx) = mount_navigation(cx, false, 100);
    fixture.resize_to_rows(cx, 10.5);
    let mut state = (*fixture.store.snapshot()).clone();
    let repo = &mut state.repos[0];
    let Loadable::Ready(page) = &repo.log else {
        panic!("loaded fixture")
    };
    repo.history_state.retained_log_while_loading = Some(page.clone());
    repo.log = Loadable::Loading;
    repo.log_rev += 1;
    repo.history_state.log = Loadable::Loading;
    repo.history_state.log_rev += 1;
    fixture
        .store
        .replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &fixture.root, Arc::new(state));
    cx.simulate_keystrokes("pagedown");
    fixture.assert_commit(cx, 10);
    fixture.assert_visible(cx, 10);
}

#[gpui::test]
fn history_paging_without_rows_or_layout_is_safe(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            assert_eq!(history.history_page_rows(), 1);
            assert!(!history.history_select_page(1, cx));
            assert!(!history.history_select_page(-1, cx));
            assert!(history.pending_history_selections.is_empty());
        });
    });
}

/// With nothing selected, a page key enters the list at its first row in
/// both modes. Cached history jumped to its last loaded row on PageUp, and
/// indexed history skipped the whole first page on PageDown.
fn paging_without_selection_enters_at_the_first_row(cx: &mut gpui::TestAppContext, indexed: bool) {
    let (fixture, cx) = mount_navigation(cx, indexed, 100);
    fixture.resize_to_rows(cx, 10.5);
    for key in ["pageup", "pagedown"] {
        let mut state = (*fixture.store.snapshot()).clone();
        state.repos[0].history_state.selected_commit = None;
        fixture
            .store
            .replace_snapshot_for_test(Arc::new(state.clone()));
        set_history_view_state_for_tests(cx, &fixture.root, Arc::new(state));
        sync_view_with_store(cx, &fixture.root);
        cx.simulate_keystrokes(key);
        fixture.assert_commit(cx, 0);
        fixture.assert_visible(cx, 0);
    }
}

#[gpui::test]
fn cached_history_paging_without_selection_enters_at_the_first_row(cx: &mut gpui::TestAppContext) {
    paging_without_selection_enters_at_the_first_row(cx, false);
}

#[gpui::test]
fn indexed_history_paging_without_selection_enters_at_the_first_row(cx: &mut gpui::TestAppContext) {
    paging_without_selection_enters_at_the_first_row(cx, true);
}

/// Holding a page key at either end must not re-select the same row on every
/// repeat; each re-selection dispatches to the store and re-scrolls.
fn paging_at_a_boundary_keeps_the_selection(cx: &mut gpui::TestAppContext, indexed: bool) {
    let (fixture, cx) = mount_navigation(cx, indexed, 3);
    for (key, last) in [("pageup", 0), ("pagedown", 2)] {
        cx.simulate_keystrokes(key);
        fixture.assert_commit(cx, last);
        cx.simulate_keystrokes(&format!("{key} {key}"));
        cx.update(|_, app| {
            assert!(
                fixture
                    .history
                    .read(app)
                    .pending_history_selections
                    .is_empty(),
                "{key} at the boundary re-selected the current row"
            );
        });
        fixture.assert_commit(cx, last);
    }
}

#[gpui::test]
fn cached_history_paging_at_a_boundary_keeps_the_selection(cx: &mut gpui::TestAppContext) {
    paging_at_a_boundary_keeps_the_selection(cx, false);
}

#[gpui::test]
fn indexed_history_paging_at_a_boundary_keeps_the_selection(cx: &mut gpui::TestAppContext) {
    paging_at_a_boundary_keeps_the_selection(cx, true);
}
