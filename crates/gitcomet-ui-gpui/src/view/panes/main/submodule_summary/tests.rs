use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::{RepoSpec, RepoStatus, Submodule, SubmoduleDiffRange};
use std::path::PathBuf;

fn summary(count: usize) -> SubmoduleDiffSummary {
    SubmoduleDiffSummary {
        path: PathBuf::from("vendor/large"),
        mode: SubmoduleDiffSummaryMode::Worktree,
        status: Some(SubmoduleStatus::HeadMismatch),
        checkout_available: true,
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: Some(CommitId("b".into())),
        ranges: vec![SubmoduleDiffRange {
            kind: SubmoduleDiffRangeKind::StagedPointer,
            from: Some(CommitId("a".into())),
            to: Some(CommitId("b".into())),
            unavailable_reason: None,
            changes: (0..count)
                .map(|ix| SubmoduleInnerChange {
                    path: PathBuf::from(format!("src/deep/file_{ix:06}.rs")),
                    kind: FileStatusKind::Modified,
                    additions: Some(2),
                    deletions: Some(1),
                })
                .collect(),
        }],
        live_staged: Vec::new(),
        live_unstaged: Vec::new(),
    }
}

fn publish(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
    summary: SubmoduleDiffSummary,
) {
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                RepoSpec {
                    workdir: PathBuf::from("/tmp/gitcomet-summary-render"),
                },
            );
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
            repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
            repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
            repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
                path: summary.path.clone(),
                recorded_head: CommitId("a".into()),
                checked_out_head: summary.checked_out_head.clone(),
                status: SubmoduleStatus::HeadMismatch,
            }]));
            repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
                path: summary.path.clone(),
                area: DiffArea::Staged,
            });
            repo.diff_state.submodule_summary_rev = view
                .state
                .repos
                .first()
                .map_or(1, |repo| repo.diff_state.submodule_summary_rev + 1);
            repo.diff_state.diff_state_rev = repo.diff_state.submodule_summary_rev;
            repo.diff_state.submodule_summary = Loadable::Ready(Arc::new(summary));
            let state = Arc::new(AppState {
                active_repo: Some(repo_id),
                repos: vec![repo],
                ..AppState::default()
            });
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, state, cx);
        })
    });
    test_support::redraw(cx);
}

fn wait_for_inline_selection(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    expected: Option<usize>,
) {
    let store = cx.update(|_window, app| Arc::clone(&view.read(app).store));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let state = store.snapshot();
        let selected = state.repos[0]
            .diff_state
            .inline_submodule_diff
            .as_ref()
            .map(|inline| inline.selected_ix);
        if selected == expected {
            cx.update(|_window, app| {
                view.update(app, |view, cx| {
                    test_support::push_test_state(view, state, cx)
                });
            });
            test_support::redraw(cx);
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "inline selection {selected:?}, expected {expected:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[gpui::test]
fn large_submodule_summaries_render_a_bounded_window_and_reuse_their_rows(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    for (count, last_selector) in [
        (1_000, "submodule_change_1001"),
        (10_000, "submodule_change_10001"),
        (50_000, "submodule_change_50001"),
    ] {
        publish(&view, cx, RepoId(71), summary(count));
        assert!(cx.debug_bounds("submodule_change_2").is_some());
        assert!(cx.debug_bounds(last_selector).is_none());
        let built = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let cache = pane.submodule_summary_cache.as_ref().unwrap();
            assert!(
                cache.rendered_rows > 0 && cache.rendered_rows < 160,
                "{count} files built {} rows",
                cache.rendered_rows
            );
            assert_eq!(cache.presentation.entries.len(), count);
            Arc::clone(&cache.presentation)
        });
        cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .clone()
                .update(app, |_pane, cx| cx.notify())
        });
        test_support::redraw(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let cache = pane.submodule_summary_cache.as_ref().unwrap();
            assert!(
                Arc::ptr_eq(&built, &cache.presentation),
                "hover/unchanged redraw must reuse all prepared rows"
            );
            assert!(cache.rendered_rows < 160);
        });

        cx.update(|_window, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                pane.submodule_summary_cache
                    .as_ref()
                    .unwrap()
                    .scroll
                    .scroll_to(gpui::ListOffset {
                        item_ix: count + 1,
                        offset_in_item: px(0.0),
                    });
                cx.notify();
            })
        });
        test_support::redraw(cx);
        assert!(
            cx.debug_bounds(last_selector).is_some(),
            "last file remains reachable"
        );
        assert!(cx.debug_bounds("submodule_change_2").is_none());
        cx.simulate_resize(gpui::size(px(1100.0), px(650.0)));
        test_support::redraw(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert!(pane.submodule_summary_cache.as_ref().unwrap().rendered_rows < 160);
        });
        if count == 50_000 {
            let summary_top = cx.update(|_window, app| {
                view.read(app)
                    .main_pane
                    .read(app)
                    .submodule_summary_cache
                    .as_ref()
                    .unwrap()
                    .scroll
                    .logical_scroll_top()
            });
            // Returning to the summary can change the toolbar's height. Keep
            // the reading position, rather than requiring bottom alignment.
            let anchor_selector: &'static str =
                format!("submodule_change_{}", summary_top.item_ix + 1).leak();
            let bounds = cx.debug_bounds(last_selector).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            wait_for_inline_selection(&view, cx, Some(count - 1));
            cx.update(|window, app| {
                view.read(app).main_pane.clone().update(app, |pane, cx| {
                    assert!(pane.try_select_adjacent_diff_file(RepoId(71), -1, window, cx));
                });
            });
            wait_for_inline_selection(&view, cx, Some(count - 2));
            cx.update(|window, app| {
                view.read(app).main_pane.clone().update(app, |pane, cx| {
                    assert!(pane.try_select_adjacent_diff_file(RepoId(71), 1, window, cx));
                });
            });
            wait_for_inline_selection(&view, cx, Some(count - 1));
            cx.update(|_window, app| {
                view.read(app)
                    .store
                    .dispatch(Msg::CloseInlineSubmoduleDiff {
                        repo_id: RepoId(71),
                    });
            });
            wait_for_inline_selection(&view, cx, None);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while cx.debug_bounds(anchor_selector).is_none() && std::time::Instant::now() < deadline
            {
                cx.run_until_parked();
                test_support::redraw(cx);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let restored = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let cache = pane.submodule_summary_cache.as_ref().unwrap();
                assert!(Arc::ptr_eq(&built, &cache.presentation));
                let restored = cache.scroll.logical_scroll_top();
                assert_eq!(restored.item_ix, summary_top.item_ix);
                assert_eq!(restored.offset_in_item, summary_top.offset_in_item);
                (restored, cache.rendered_rows)
            });
            assert!(
                cx.debug_bounds(anchor_selector).is_some(),
                "back restores summary scroll: {restored:?}"
            );
        }
        // A same-target refresh with fewer files must clamp the old scroll.
        publish(&view, cx, RepoId(71), summary(3));
        assert!(cx.debug_bounds("submodule_change_2").is_some());
        // Another repository must reset scroll/data even for the same path.
        publish(&view, cx, RepoId(72), summary(2));
        assert!(cx.debug_bounds("submodule_change_2").is_some());
    }
}

#[gpui::test]
fn submodule_summary_rows_keep_section_specific_navigation_and_menus(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut data = summary(1);
    data.live_staged = data.ranges[0].changes.clone();
    data.live_unstaged = data.ranges[0].changes.clone();
    publish(&view, cx, RepoId(73), data);
    let (row_ix, target) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let rows = &pane.submodule_summary_cache.as_ref().unwrap().presentation;
        let (ix, entry) = rows
            .rows
            .iter()
            .enumerate()
            .find_map(|(ix, row)| match row {
                SummaryRow::Change {
                    section: ChangeSection::Unstaged,
                    inline_index: Some(entry),
                    ..
                } => Some((ix, *entry)),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            entry, 2,
            "same path in range, staged and unstaged must have distinct indexes"
        );
        (ix, rows.entries[entry].target.clone())
    });
    assert_eq!(row_ix, 7);
    let bounds = cx.debug_bounds("submodule_change_7").unwrap();
    cx.simulate_event(MouseDownEvent {
        position: bounds.center(),
        button: MouseButton::Right,
        ..Default::default()
    });
    test_support::redraw(cx);
    let menu = cx.update(|_window, app| test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(menu, Some(PopoverKind::SubmoduleInnerDiffMenu { target: menu_target, .. }) if menu_target == target)
    );
}

fn summary_without_ranges(count: usize) -> SubmoduleDiffSummary {
    let mut summary = summary(count);
    summary.ranges = Vec::new();
    summary.live_unstaged = (0..count)
        .map(|ix| SubmoduleInnerChange {
            path: PathBuf::from(format!("src/deep/file_{ix:06}.rs")),
            kind: FileStatusKind::Modified,
            additions: Some(2),
            deletions: Some(1),
        })
        .collect();
    summary
}

/// A range header is several times taller than the 28px change row that can
/// replace it at the same index.
#[test]
fn restoring_scroll_drops_the_offset_when_the_row_at_that_index_changed_shape() {
    let with_ranges = SummaryRows::new(Arc::new(summary(4)));
    let without_ranges = SummaryRows::new(Arc::new(summary_without_ranges(4)));
    assert!(matches!(with_ranges.rows[1], SummaryRow::RangeHeader(_)));
    assert!(!matches!(
        without_ranges.rows[1],
        SummaryRow::RangeHeader(_)
    ));

    let top = gpui::ListOffset {
        item_ix: 1,
        offset_in_item: px(90.0),
    };
    let swapped = restored_scroll_top(top, &with_ranges, &without_ranges);
    assert_eq!(swapped.item_ix, 1);
    assert_eq!(swapped.offset_in_item, px(0.0));

    let kept = restored_scroll_top(top, &with_ranges, &with_ranges);
    assert_eq!(kept.item_ix, 1);
    assert_eq!(kept.offset_in_item, px(90.0));
}

#[test]
fn restoring_scroll_clamps_past_the_end_of_a_shorter_rebuild() {
    let long = SummaryRows::new(Arc::new(summary(400)));
    let short = SummaryRows::new(Arc::new(summary(2)));
    let restored = restored_scroll_top(
        gpui::ListOffset {
            item_ix: 300,
            offset_in_item: px(12.0),
        },
        &long,
        &short,
    );
    assert_eq!(restored.item_ix, short.rows.len() - 1);
    assert_eq!(restored.offset_in_item, px(0.0));
}

/// Both are frame-invariant: rebuilding them costs a `PathBuf` per visible row
/// and a `Vec` per click.
#[gpui::test]
fn summary_redraws_reuse_the_cached_repo_path_and_entry_list(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    publish(&view, cx, RepoId(72), summary(2_000));
    let (path, entries) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let cache = pane.submodule_summary_cache.as_ref().unwrap();
        assert_eq!(
            cache.submodule_repo_path.as_path(),
            std::path::Path::new("/tmp/gitcomet-summary-render/vendor/large")
        );
        (
            Arc::clone(&cache.submodule_repo_path),
            Arc::clone(&cache.presentation.entries),
        )
    });

    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .clone()
            .update(app, |_pane, cx| cx.notify())
    });
    test_support::redraw(cx);
    cx.simulate_resize(gpui::size(px(1100.0), px(650.0)));
    test_support::redraw(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let cache = pane.submodule_summary_cache.as_ref().unwrap();
        assert!(
            Arc::ptr_eq(&path, &cache.submodule_repo_path),
            "redraws must not rebuild the submodule workdir path"
        );
        assert!(
            Arc::ptr_eq(&entries, &cache.presentation.entries),
            "redraws must not rebuild the inline entry list"
        );
    });
}
