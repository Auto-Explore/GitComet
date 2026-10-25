use super::*;
use crate::view::rows::CommitFileSort;
use gitcomet_core::domain::{FileStatus, FileStatusKind};
use gitcomet_state::model::{Loadable, RepoId, RepoState};

fn status_repo(repo_id: RepoId, entries: usize) -> RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/status-footer"));
    let files: Vec<_> = (0..entries)
        .map(|ix| FileStatus {
            path: format!("src/feature_{:04}/file_{ix:06}.rs", ix / 20).into(),
            kind: if ix % 3 == 0 {
                FileStatusKind::Untracked
            } else {
                FileStatusKind::Modified
            },
            conflict: None,
        })
        .collect();
    repo.diff_state.diff_target = files
        .last()
        .map(|file| DiffTarget::working_tree(file.path.clone(), DiffArea::Unstaged));
    repo.diff_state.diff_state_rev = 1;
    repo.worktree_status = Loadable::Ready(Arc::new(files));
    repo.worktree_status_rev = 1;
    repo
}

fn fixture(
    cx: &mut gpui::TestAppContext,
    repo: RepoState,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let repo_id = repo.id;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    (view, cx)
}

fn configure(
    cx: &mut gpui::VisualTestContext,
    root: &gpui::Entity<GitCometView>,
    view: ChangeTrackingView,
    layout: FileListLayout,
    sort: CommitFileSort,
) {
    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            root.change_tracking_view = view;
            root.details_pane.update(cx, |pane, _| {
                pane.change_tracking_view = view;
                pane.file_list_layout = layout;
                pane.file_list_layout_override.clear();
                for section in [
                    StatusSection::CombinedUnstaged,
                    StatusSection::Untracked,
                    StatusSection::Unstaged,
                    StatusSection::Staged,
                ] {
                    pane.status_file_sort.insert(section, sort);
                }
            });
        });
    });
}

fn assert_matches_previous_path(
    cx: &mut gpui::VisualTestContext,
    root: &gpui::Entity<GitCometView>,
) -> Option<(usize, usize)> {
    cx.update(|_window, app| {
        let renderer = root.read(app).diff_renderer.clone();
        renderer.update(app, |renderer, cx| {
            let repo_id = renderer.active_repo_id().expect("active repo");
            let expected = renderer.uncached_diff_file_place_for_test(repo_id, cx);
            let actual = renderer.diff_file_place(repo_id, cx);
            assert_eq!(actual, expected);
            actual.map(|place| (place.index, place.count))
        })
    })
}

/// Exercise WindowNavigation and the real DetailsPaneView caches, including
/// projection/plan acquisition. A cache around only the final path scan would
/// still materialize a whole tree order on every call and fail this test.
#[gpui::test]
fn status_footer_warm_caller_does_not_walk_or_rebuild_large_lists(cx: &mut gpui::TestAppContext) {
    let (root, cx) = fixture(cx, status_repo(RepoId(714), 12_000));
    for view in [
        ChangeTrackingView::Combined,
        ChangeTrackingView::SplitUntracked,
    ] {
        for layout in [
            FileListLayout::Flat,
            FileListLayout::Tree,
            FileListLayout::Groups,
        ] {
            configure(cx, &root, view, layout, CommitFileSort::PathAscending);
            let expected =
                assert_matches_previous_path(cx, &root).expect("selected file is listed");
            cx.update(|_window, app| {
                let renderer = root.read(app).diff_renderer.clone();
                renderer.update(app, |renderer, cx| {
                    let repo_id = renderer.active_repo_id();
                    // Use the actual bar caller, not just the cache helper.
                    assert_eq!(
                        renderer
                            .diff_bar_nav(repo_id, cx)
                            .and_then(|nav| nav.position),
                        Some(expected)
                    );
                    DetailsPaneView::take_status_footer_work();
                    for _ in 0..20 {
                        assert_eq!(
                            renderer
                                .diff_bar_nav(repo_id, cx)
                                .and_then(|nav| nav.position),
                            Some(expected)
                        );
                    }
                    let work = DetailsPaneView::take_status_footer_work();
                    assert_eq!(work, Default::default(), "{view:?} {layout:?}: {work:?}");
                });
            });
        }
    }
}

#[gpui::test]
fn status_footer_invalidates_for_source_sort_layout_and_line_stats(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(715);
    let mut repo = status_repo(repo_id, 4);
    let selected = "root.rs";
    repo.worktree_status = Loadable::Ready(Arc::new(vec![
        FileStatus {
            path: selected.into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        },
        FileStatus {
            path: "src/a.rs".into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        },
        FileStatus {
            path: "src/a.rs".into(),
            kind: FileStatusKind::Untracked,
            conflict: None,
        },
        FileStatus {
            path: "z.rs".into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        },
    ]));
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        selected.into(),
        DiffArea::Unstaged,
    ));
    let (root, cx) = fixture(cx, repo);

    configure(
        cx,
        &root,
        ChangeTrackingView::Combined,
        FileListLayout::Flat,
        CommitFileSort::PathAscending,
    );
    let ascending = assert_matches_previous_path(cx, &root).unwrap();
    configure(
        cx,
        &root,
        ChangeTrackingView::Combined,
        FileListLayout::Flat,
        CommitFileSort::PathDescending,
    );
    let descending = assert_matches_previous_path(cx, &root).unwrap();
    assert_ne!(ascending.0, descending.0);
    configure(
        cx,
        &root,
        ChangeTrackingView::Combined,
        FileListLayout::Tree,
        CommitFileSort::PathAscending,
    );
    let tree = assert_matches_previous_path(cx, &root).unwrap();
    assert_ne!(
        ascending.0, tree.0,
        "tree hoists the src directory before root.rs"
    );

    // A new immutable source can have the same length. The selected path is
    // now untracked, so split tracking must resolve a different section.
    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            let mut state = (*root.state).clone();
            let repo = &mut state.repos[0];
            let Loadable::Ready(files) = &mut repo.worktree_status else {
                panic!("ready source")
            };
            Arc::make_mut(files)[0].kind = FileStatusKind::Untracked;
            repo.worktree_status_rev += 1;
            push_test_state(root, Arc::new(state), cx);
        });
    });
    draw_and_drain_test_window(cx);
    configure(
        cx,
        &root,
        ChangeTrackingView::SplitUntracked,
        FileListLayout::Flat,
        CommitFileSort::PathAscending,
    );
    assert_eq!(assert_matches_previous_path(cx, &root).unwrap().1, 2);

    configure(
        cx,
        &root,
        ChangeTrackingView::Combined,
        FileListLayout::Flat,
        CommitFileSort::EditSizeDescending,
    );
    let before_counts = assert_matches_previous_path(cx, &root).unwrap();
    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            let mut state = (*root.state).clone();
            load_unstaged_line_stats(
                &mut state.repos[0],
                &[("root.rs", 1, 0), ("src/a.rs", 99, 0), ("z.rs", 50, 0)],
            );
            push_test_state(root, Arc::new(state), cx);
        });
    });
    draw_and_drain_test_window(cx);
    let after_counts = assert_matches_previous_path(cx, &root).unwrap();
    assert_ne!(
        before_counts.0, after_counts.0,
        "line-stat-only arrival reorders the list"
    );
}

#[gpui::test]
fn status_footer_resolved_missing_and_unavailable_provider_are_distinct(
    cx: &mut gpui::TestAppContext,
) {
    let (root, cx) = fixture(cx, status_repo(RepoId(716), 5));
    configure(
        cx,
        &root,
        ChangeTrackingView::Combined,
        FileListLayout::Flat,
        CommitFileSort::PathDescending,
    );
    assert_eq!(assert_matches_previous_path(cx, &root), Some((0, 5)));

    cx.update(|_window, app| {
        let details = root.read(app).details_pane.clone();
        let saved = details.read(app).state.clone();
        let mut mismatched = (*saved).clone();
        let Loadable::Ready(files) = &mut mismatched.repos[0].worktree_status else {
            panic!("ready source")
        };
        Arc::make_mut(files)[0].path = "different.rs".into();
        mismatched.repos[0].worktree_status_rev += 1;
        details.update(app, |pane, _| pane.state = Arc::new(mismatched));
        let renderer = root.read(app).diff_renderer.clone();
        renderer.update(app, |renderer, cx| {
            let repo = renderer.active_repo().unwrap();
            let Some(DiffTarget::WorkingTree { path, area, .. }) =
                renderer.bound_diff_state(repo).diff_target.as_ref()
            else {
                panic!("working-tree target")
            };
            assert!(
                renderer
                    .commands
                    .navigation
                    .as_ref()
                    .unwrap()
                    .status_position(repo, path, *area, ChangeTrackingView::Combined, cx)
                    .is_none(),
                "separately updated panes must not reuse different source snapshots"
            );
            assert_eq!(
                renderer.diff_file_place(repo.id, cx),
                renderer.uncached_diff_file_place_for_test(repo.id, cx)
            );
        });
        details.update(app, |pane, _| {
            pane.state = Arc::new(AppState::test_default())
        });
        let renderer = root.read(app).diff_renderer.clone();
        renderer.update(app, |renderer, cx| {
            let repo_id = renderer.active_repo_id().unwrap();
            let fallback = renderer
                .diff_file_place(repo_id, cx)
                .expect("source-order fallback");
            assert_eq!((fallback.index, fallback.count), (4, 5));
            assert_eq!(
                Some(fallback),
                renderer.uncached_diff_file_place_for_test(repo_id, cx)
            );
        });
        details.update(app, |pane, _| pane.state = saved);
    });

    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            let mut state = (*root.state).clone();
            state.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
                "missing.rs".into(),
                DiffArea::Unstaged,
            ));
            state.repos[0].diff_state.diff_state_rev += 1;
            push_test_state(root, Arc::new(state), cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert_eq!(assert_matches_previous_path(cx, &root), None);
    cx.update(|_window, app| {
        let renderer = root.read(app).diff_renderer.clone();
        renderer.update(app, |renderer, cx| {
            let repo_id = renderer.active_repo_id();
            DetailsPaneView::take_status_footer_work();
            assert!(renderer.diff_bar_nav(repo_id, cx).is_none());
            assert_eq!(
                DetailsPaneView::take_status_footer_work(),
                Default::default(),
                "resolved missing must not enter the uncached fallback"
            );
        });
    });
}

#[gpui::test]
fn status_footer_uses_ready_split_source_then_legacy_fallback(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(717);
    let mut repo = status_repo(repo_id, 3);
    let Loadable::Ready(split) = &repo.worktree_status else {
        panic!("ready source")
    };
    let legacy = Arc::new(vec![split[2].clone()]);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: Arc::new(vec![]),
            unstaged: legacy,
        }
        .into(),
    );
    let (root, cx) = fixture(cx, repo);
    configure(
        cx,
        &root,
        ChangeTrackingView::Combined,
        FileListLayout::Flat,
        CommitFileSort::PathAscending,
    );
    assert_eq!(assert_matches_previous_path(cx, &root), Some((2, 3)));
    for lane in [
        Loadable::Loading,
        Loadable::NotLoaded,
        Loadable::Error("transient status error".into()),
    ] {
        cx.update(|_window, app| {
            root.update(app, |root, cx| {
                let mut state = (*root.state).clone();
                state.repos[0].worktree_status = lane;
                state.repos[0].worktree_status_rev += 1;
                push_test_state(root, Arc::new(state), cx);
            });
        });
        draw_and_drain_test_window(cx);
        assert_eq!(assert_matches_previous_path(cx, &root), Some((0, 1)));
    }
    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            let mut state = (*root.state).clone();
            state.repos[0].status = Loadable::NotLoaded;
            state.repos[0].status_rev += 1;
            push_test_state(root, Arc::new(state), cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert_eq!(assert_matches_previous_path(cx, &root), None);

    // Changing areas resolves the staged source independently of the missing
    // worktree lane and must not keep the cached empty result.
    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            let mut state = (*root.state).clone();
            let repo = &mut state.repos[0];
            let Some(DiffTarget::WorkingTree { path, .. }) = repo.diff_state.diff_target.as_ref()
            else {
                panic!("working-tree target")
            };
            let path = path.clone();
            repo.staged_status = Loadable::Ready(Arc::new(vec![FileStatus {
                path: path.clone(),
                kind: FileStatusKind::Added,
                conflict: None,
            }]));
            repo.staged_status_rev += 1;
            repo.diff_state.diff_target = Some(DiffTarget::working_tree(path, DiffArea::Staged));
            repo.diff_state.diff_state_rev += 1;
            push_test_state(root, Arc::new(state), cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert_eq!(assert_matches_previous_path(cx, &root), Some((0, 1)));
}

/// Measures the real caller and DetailsPaneView cache acquisition, with the old
/// lookup as its baseline in the same process. Run alone with --ignored
/// --nocapture --test-threads=1 in release for timing and allocation evidence.
#[gpui::test]
#[ignore = "timing probe"]
fn timing_status_footer_lookup_with_large_status(cx: &mut gpui::TestAppContext) {
    use std::hint::black_box;
    use std::time::Instant;
    let entries = std::env::var("GITCOMET_PROBE_STATUS_ENTRIES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);
    let (root, cx) = fixture(cx, status_repo(RepoId(718), entries));
    for view in [
        ChangeTrackingView::Combined,
        ChangeTrackingView::SplitUntracked,
    ] {
        for layout in [FileListLayout::Flat, FileListLayout::Tree] {
            configure(cx, &root, view, layout, CommitFileSort::PathAscending);
            assert_matches_previous_path(cx, &root).expect("selected file is shown");
            cx.update(|_window, app| {
                let renderer = root.read(app).diff_renderer.clone();
                renderer.update(app, |renderer, cx| {
                    let repo_id = renderer.active_repo_id().unwrap();
                    let mut cached_ns = Vec::new();
                    let mut previous_ns = Vec::new();
                    for sample in 0..20 {
                        let started = Instant::now();
                        if sample % 2 == 0 {
                            black_box(renderer.uncached_diff_file_place_for_test(repo_id, cx));
                            previous_ns.push(started.elapsed().as_nanos());
                            let started = Instant::now();
                            black_box(renderer.diff_file_place(repo_id, cx));
                            cached_ns.push(started.elapsed().as_nanos());
                        } else {
                            black_box(renderer.diff_file_place(repo_id, cx));
                            cached_ns.push(started.elapsed().as_nanos());
                            let started = Instant::now();
                            black_box(renderer.uncached_diff_file_place_for_test(repo_id, cx));
                            previous_ns.push(started.elapsed().as_nanos());
                        }
                    }
                    let (_, previous_allocations) = crate::perf_alloc::measure_allocations(|| {
                        black_box(renderer.uncached_diff_file_place_for_test(repo_id, cx));
                    });
                    DetailsPaneView::take_status_footer_work();
                    let (_, cached_allocations) = crate::perf_alloc::measure_allocations(|| {
                        black_box(renderer.diff_file_place(repo_id, cx));
                    });
                    let work = DetailsPaneView::take_status_footer_work();
                    assert_eq!(work, Default::default());
                    println!("status_footer entries={entries} view={view:?} layout={layout:?} previous_ns={previous_ns:?} cached_ns={cached_ns:?} previous_allocations={previous_allocations:?} cached_allocations={cached_allocations:?} warm_work={work:?}");
                });
            });
        }
    }
}
