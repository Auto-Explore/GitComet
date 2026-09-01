use super::*;

/// A worktree reveal scrolls to the worktree's own row, which sits one line
/// *above* the commit that located it. The selected-list-index cache it
/// writes is keyed on that commit, though, so it has to remember the
/// commit's row: caching the row we scrolled to hands the commit its
/// neighbour's index, and the first arrow step off that commit computes
/// `neighbour + 1` and lands back on the commit itself.
#[gpui::test]
fn a_worktree_reveal_caches_the_commits_row_not_the_worktree_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let worktree_path = PathBuf::from("/tmp/history-worktree-reveal/linked");
    let page = Arc::new(log_page(vec![commit("tip", &[], "tip")], None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-worktree-reveal"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    repo.worktree_dirty = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::WorktreeDirtySummary {
            path: worktree_path.clone(),
            head: Some(CommitId("tip".into())),
            branch: Some("side".into()),
            detached: false,
            added: 1,
            modified: 0,
            deleted: 0,
            staged: Vec::new(),
            unstaged: Vec::new(),
        },
    ]));
    repo.worktree_dirty_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "history cache for the worktree reveal", |cx| {
        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view
                .read(app)
                .history_cache
                .as_ref()
                .is_some_and(|cache| cache.base.row_vms.len() == 1)
        })
    });

    cx.update(|_window, app| {
        let history_view = view.read(app).main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            let plan = history.ensure_history_list_plan();
            let worktree_row_ix =
                worktree_row_list_ix(&plan, history.active_repo(), &worktree_path)
                    .expect("the dirty worktree should have a row");
            let commit_row_ix = plan.list_ix_for_visible(0);
            assert_eq!(
                commit_row_ix,
                worktree_row_ix + 1,
                "fixture must put the worktree row directly above its commit"
            );

            history.pending_history_reveal = Some(PendingHistoryReveal {
                worktree_path: Some(worktree_path.clone()),
                repo_id,
                commit_id: CommitId("tip".into()),
                fallback_scope: None,
            });
            history.drive_pending_history_reveal(cx);

            let cache = history
                .history_selected_list_index_cache
                .as_ref()
                .expect("the reveal should leave a list-index cache");
            assert_eq!(
                cache.selected_commit.as_ref().map(|id| id.as_ref()),
                Some("tip")
            );
            assert_eq!(
                cache.list_ix, commit_row_ix,
                "the cache is keyed on the commit, so it holds the commit's row"
            );
        });
    });
}

/// `list_ix_for_worktree` returns `None` once the worktree goes clean or its
/// HEAD leaves the loaded page, and a selected row with no index is not the
/// same as nothing being selected. Falling through to the no-selection arms
/// wrapped the selection to the far end of the log instead of moving it by
/// one, and the user lost their place.
#[gpui::test]
fn arrowing_off_a_worktree_row_with_no_index_does_not_jump_to_the_end(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let worktree_path = PathBuf::from("/tmp/history-worktree-nav/linked");
    let page = Arc::new(log_page(
        vec![commit("tip", &["base"], "tip"), commit("base", &[], "base")],
        None,
    ));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-worktree-nav"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    // Selected, but with no row: the scan that would list it has not landed,
    // which is exactly the state the reducer refuses to read as "clean".
    repo.history_state.worktree_selection = Some(worktree_path.clone());
    repo.worktree_dirty = Loadable::Ready(Arc::new(Vec::new()));
    repo.worktree_dirty_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "history cache for the worktree nav", |cx| {
        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view
                .read(app)
                .history_cache
                .as_ref()
                .is_some_and(|cache| cache.base.row_vms.len() == 2)
        })
    });

    cx.update(|_window, app| {
        let history_view = view.read(app).main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            let plan = history.ensure_history_list_plan();
            assert!(
                worktree_row_list_ix(&plan, history.active_repo(), &worktree_path).is_none(),
                "fixture must leave the selected worktree without a row"
            );

            assert!(
                !history.history_select_adjacent_commit(-1, cx),
                "there is nothing to step from, so the key is not handled"
            );
            assert!(
                history
                    .active_repo()
                    .is_none_or(|repo| repo.history_state.selected_commit.is_none()),
                "and nothing at the far end of the log may be selected in its place"
            );
        });
    });
}

/// The commit set never changes here -- only the stash list does -- so the
/// log fingerprint is identical across both halves of this test. That is the
/// point: the plan's anchors are `visible_ix_by_commit` lookups, and that map
/// is renumbered when stash helper commits are filtered out of the page. A
/// plan cache keyed on the fingerprint alone hands back the pre-filter
/// indices, which puts every worktree row above the wrong commit and leaves a
/// blank gap wherever the stale index ran past the end of `graph_rows`.
#[gpui::test]
fn a_stash_list_arriving_replans_the_worktree_rows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let worktree_path = PathBuf::from("/tmp/history-stash-replan/linked");
    // `helper` is the stash's second parent, so it disappears from the page
    // once the stash list names `wip` as a stash tip. `base` -- the commit the
    // worktree is anchored on -- moves up a row when it does.
    let page = Arc::new(log_page(
        vec![
            commit("wip", &["base", "helper"], "stash push"),
            commit("helper", &["base"], "index on main"),
            commit("base", &[], "base"),
        ],
        None,
    ));

    let state_with_stashes = |stashes: Vec<StashEntry>, stashes_rev: u64| {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/history-stash-replan"),
            },
        );
        repo.history_state.history_scope = LogScope::AllBranches;
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.head_branch_rev = 1;
        repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "wip")]));
        repo.branches_rev = 1;
        repo.log = Loadable::Ready(Arc::clone(&page));
        repo.log_rev = 1;
        repo.history_state.log = Loadable::Ready(Arc::clone(&page));
        repo.history_state.log_rev = 1;
        repo.stashes = Loadable::Ready(Arc::new(stashes));
        repo.stashes_rev = stashes_rev;
        repo.worktree_dirty = Loadable::Ready(Arc::new(vec![
            gitcomet_core::domain::WorktreeDirtySummary {
                path: worktree_path.clone(),
                head: Some(CommitId("base".into())),
                branch: Some("side".into()),
                detached: false,
                added: 1,
                modified: 0,
                deleted: 0,
                staged: Vec::new(),
                unstaged: Vec::new(),
            },
        ]));
        repo.worktree_dirty_rev = 1;
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..Default::default()
        })
    };

    /// The visible row `base` renders on, and the list row its worktree
    /// sits on, read back after the cache has settled at `visible_len` rows.
    fn anchored_rows(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<GitCometView>,
        visible_len: usize,
    ) -> (usize, usize, usize) {
        wait_until(cx, "history cache to match the stash list", |cx| {
            cx.update(|_window, app| {
                let history_view = view.read(app).main_pane.read(app).history_view.clone();
                history_view
                    .read(app)
                    .history_cache
                    .as_ref()
                    .is_some_and(|cache| cache.base.row_vms.len() == visible_len)
            })
        });

        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view.update(app, |history, _cx| {
                let plan = history.ensure_history_list_plan();
                let base_visible_ix = history
                    .history_cache
                    .as_ref()
                    .expect("cache")
                    .base
                    .visible_ix_by_commit
                    .get(&CommitId("base".into()))
                    .copied()
                    .expect("the anchored commit is on screen");
                (
                    base_visible_ix,
                    plan.list_ix_for_worktree(0)
                        .expect("the dirty worktree keeps its row"),
                    plan.list_ix_for_visible(base_visible_ix),
                )
            })
        })
    }

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(cx, &view, state_with_stashes(Vec::new(), 1));
    let (before_visible_ix, before_worktree_ix, before_commit_ix) = anchored_rows(cx, &view, 3);
    assert_eq!(
        before_visible_ix, 2,
        "with no stashes every commit is on screen"
    );
    assert_eq!(
        before_worktree_ix + 1,
        before_commit_ix,
        "the worktree row sits directly above the commit it is anchored on"
    );

    ensure_history_cache_for_tests(
        cx,
        &view,
        state_with_stashes(
            vec![StashEntry {
                index: 0,
                id: CommitId("wip".into()),
                message: "WIP on main: base".into(),
                created_at: None,
            }],
            2,
        ),
    );
    let (after_visible_ix, after_worktree_ix, after_commit_ix) = anchored_rows(cx, &view, 2);
    assert_eq!(
        after_visible_ix, 1,
        "the stash helper commit must have been filtered out of the page"
    );
    assert_eq!(
        after_worktree_ix + 1,
        after_commit_ix,
        "the replanned worktree row must follow its commit up the renumbered page"
    );
}

/// The lane colour is read out of `graph_rows`, which `force_branch_head_lane`
/// reshapes whenever the branch list changes -- again without touching the log
/// fingerprint. A fingerprint-keyed memo keeps saturating whichever lane held
/// that colour index before the branch appeared.
#[gpui::test]
fn a_new_branch_recolours_the_selected_lane(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    // `behind` sits on the main lane. Pointing a branch at it makes
    // `force_branch_head_lane` fork a whisker lane for the head, and that fork
    // takes a palette slot -- so `other`, whose lane is born on the row *below*
    // it, draws a different colour than it did before the branch existed.
    let page = Arc::new(log_page(
        vec![
            commit("tip", &["behind"], "tip"),
            commit("behind", &["base"], "behind"),
            commit("other", &["base"], "other"),
            commit("base", &[], "base"),
        ],
        None,
    ));

    let state_with_branches = |branches: Vec<Branch>, branches_rev: u64| {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/history-lane-recolour"),
            },
        );
        repo.history_state.history_scope = LogScope::AllBranches;
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.head_branch_rev = 1;
        repo.branches = Loadable::Ready(Arc::new(branches));
        repo.branches_rev = branches_rev;
        repo.log = Loadable::Ready(Arc::clone(&page));
        repo.log_rev = 1;
        repo.history_state.log = Loadable::Ready(Arc::clone(&page));
        repo.history_state.log_rev = 1;
        repo.history_state.selected_commit = Some(CommitId("other".into()));
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..Default::default()
        })
    };

    fn selected_lane_colour(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<GitCometView>,
    ) -> (
        Option<crate::view::rows::history_graph_paint::SelectedLane>,
        Option<crate::view::rows::history_graph_paint::SelectedLane>,
    ) {
        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view.update(app, |history, _cx| {
                let memoised = history.history_selected_lane(false);
                // The same answer computed from scratch. The memo is the only
                // thing that can make these two disagree.
                history.history_selected_lane_color_cache = None;
                let fresh = history.history_selected_lane(false);
                (memoised, fresh)
            })
        })
    }

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(
        cx,
        &view,
        state_with_branches(vec![branch("main", "tip")], 1),
    );
    wait_until(cx, "history cache for the unbranched graph", |cx| {
        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view
                .read(app)
                .history_cache
                .as_ref()
                .is_some_and(|cache| cache.base.request.branches_rev == 1)
        })
    });
    let (before, before_fresh) = selected_lane_colour(cx, &view);
    assert_eq!(before, before_fresh, "the memo must start out agreeing");
    let before = before.expect("the selected commit is on a lane");

    ensure_history_cache_for_tests(
        cx,
        &view,
        state_with_branches(vec![branch("main", "tip"), branch("behind", "behind")], 2),
    );
    wait_until(cx, "history cache for the branched graph", |cx| {
        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view
                .read(app)
                .history_cache
                .as_ref()
                .is_some_and(|cache| cache.base.request.branches_rev == 2)
        })
    });
    let (after, after_fresh) = selected_lane_colour(cx, &view);
    let after_fresh = after_fresh.expect("the selected commit is still on a lane");
    assert_ne!(
        before.color_ix, after_fresh.color_ix,
        "fixture must actually recolour the selected lane, or this test proves \
             nothing about the memo"
    );
    assert_eq!(
        after,
        Some(after_fresh),
        "the memo must be reissued when the graph it read is rebuilt"
    );
}

#[gpui::test]
fn switching_same_graph_workspaces_highlights_the_new_heads_lane(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let page = Arc::new(log_page(
        vec![
            commit("main-tip", &["base"], "main tip"),
            commit("feature-tip", &["base"], "feature tip"),
            commit("base", &[], "base"),
        ],
        None,
    ));
    let make_repo = |repo_id: RepoId, path: &str, head_branch: &str| {
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from(path),
            },
        );
        repo.history_state.history_scope = LogScope::AllBranches;
        repo.head_branch = Loadable::Ready(head_branch.to_string());
        repo.head_branch_rev = 1;
        repo.branches = Loadable::Ready(Arc::new(vec![
            branch("main", "main-tip"),
            branch("feature", "feature-tip"),
        ]));
        repo.branches_rev = 1;
        repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
        repo.remote_branches_rev = 1;
        repo.log = Loadable::Ready(Arc::clone(&page));
        repo.log_rev = 1;
        repo.history_state.log = Loadable::Ready(Arc::clone(&page));
        repo.history_state.log_rev = 1;
        repo
    };
    let main_repo = make_repo(RepoId(1), "/tmp/history-main-workspace", "main");
    let feature_repo = make_repo(RepoId(2), "/tmp/history-feature-workspace", "feature");
    let state_for = |active_repo| {
        Arc::new(AppState {
            repos: vec![main_repo.clone(), feature_repo.clone()],
            active_repo: Some(active_repo),
            ..Default::default()
        })
    };

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state_for(RepoId(1)));

    cx.update(|_window, app| {
        let history_view = view.read(app).main_pane.read(app).history_view.clone();
        history_view.update(app, |history, _cx| {
            history
                .history_selected_lane(false)
                .expect("clean main workspace should highlight HEAD")
        })
    });

    ensure_history_cache_for_tests(cx, &view, state_for(RepoId(2)));
    let (feature_lane, expected_feature_lane, cached_repo_id, memo_anchor) =
        cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view.update(app, |history, _cx| {
                let feature_lane = history.history_selected_lane(false);
                let cache = history
                    .history_cache
                    .as_ref()
                    .expect("feature workspace history cache");
                let anchor_row = *cache
                    .base
                    .visible_ix_by_commit
                    .get(&CommitId("feature-tip".into()))
                    .expect("feature HEAD should be visible");
                let row = &cache.base.graph_rows[anchor_row];
                let expected = crate::view::rows::history_graph_paint::selected_lane_at(
                    &cache.base.graph_rows,
                    anchor_row,
                    row.node_color_ix,
                );
                let memo_anchor = history
                    .history_selected_lane_color_cache
                    .as_ref()
                    .map(|memo| memo.anchor.clone());
                (
                    feature_lane,
                    expected,
                    cache.base.request.repo_id,
                    memo_anchor,
                )
            })
        });

    assert_eq!(cached_repo_id, RepoId(2));
    assert_eq!(
        memo_anchor,
        Some(HistoryLaneAnchor::Commit(CommitId("feature-tip".into()))),
        "the lane memo must be anchored to the newly active workspace's HEAD"
    );
    assert_eq!(
        feature_lane, expected_feature_lane,
        "the sibling workspace must not reuse the previous tab's lane"
    );
}

#[gpui::test]
fn date_time_changes_reuse_history_cache_and_rows_still_render(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(vec![commit("tip", &[], "tip")], None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-date-time-reuse"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "initial history cache for date-time reuse", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.row_vms.len() == 1
                    && cache.base.row_vms[0].summary.as_ref() == "tip"
                    && cache.decorations.row_vms.len() == 1
            })
        })
    });

    let (before_graph_rows, before_base_request, before_decoration_request, before_when_text) = cx
        .update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let rows_len = history_view.update(app, |history, cx| {
                HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
            });
            assert_eq!(rows_len, 1, "initial history row should render");

            let history = history_view.read(app);
            let cache = history
                .history_cache
                .as_ref()
                .expect("history cache should be available");
            (
                Arc::clone(&cache.base.graph_rows),
                cache.base.request.clone(),
                cache.decorations.request.clone(),
                cache.base.row_vms[0]
                    .when
                    .resolve(HistoryDisplayKey::new(
                        DateTimeFormat::YmdHm,
                        Timezone::Utc,
                        true,
                        false,
                    ))
                    .as_ref()
                    .to_owned(),
            )
        });

    assert_eq!(
        before_when_text,
        format_datetime(
            SystemTime::UNIX_EPOCH,
            DateTimeFormat::YmdHm,
            Timezone::Utc,
            true,
        )
    );

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            history.set_date_time_format(DateTimeFormat::MdyHm, cx);
            history.ensure_history_cache(cx);
            let rows = HistoryView::render_history_table_rows(history, 0..1, window, cx);
            assert_eq!(
                rows.len(),
                1,
                "history row should still render after date change"
            );
        });
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let (after_graph_rows, after_base_request, after_decoration_request, after_when_text) = cx
        .update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            assert!(
                history.history_cache_inflight.is_none(),
                "display-only changes should not enqueue a cache rebuild"
            );
            let cache = history
                .history_cache
                .as_ref()
                .expect("history cache should still be available");
            (
                Arc::clone(&cache.base.graph_rows),
                cache.base.request.clone(),
                cache.decorations.request.clone(),
                cache.base.row_vms[0]
                    .when
                    .resolve(HistoryDisplayKey::new(
                        DateTimeFormat::MdyHm,
                        Timezone::Utc,
                        true,
                        false,
                    ))
                    .as_ref()
                    .to_owned(),
            )
        });

    assert!(
        Arc::ptr_eq(&before_graph_rows, &after_graph_rows),
        "date/time changes should keep the heavy graph cache"
    );
    assert_eq!(after_base_request, before_base_request);
    assert_eq!(after_decoration_request, before_decoration_request);
    assert_eq!(
        after_when_text,
        format_datetime(
            SystemTime::UNIX_EPOCH,
            DateTimeFormat::MdyHm,
            Timezone::Utc,
            true,
        )
    );
    assert_ne!(after_when_text, before_when_text);
}

#[gpui::test]
fn history_refs_hover_lists_refs_and_opens_item_menus(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("tip".into());
    let base_commit_id = CommitId("base".into());
    let page = Arc::new(log_page(
        vec![
            commit("tip", &[base_commit_id.as_ref()], "tip"),
            commit(base_commit_id.as_ref(), &[], "base"),
        ],
        None,
    ));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", "tip"),
        branch("feature", "tip"),
    ]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(vec![remote_branch("origin", "main", "tip")]));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::Tag {
            name: "release".to_string(),
            target: commit_id.clone(),
        },
        gitcomet_core::domain::Tag {
            name: "old-release".to_string(),
            target: base_commit_id.clone(),
        },
    ]));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "history row with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });
    wait_until(cx, "history second row with displayed refs", |cx| {
        cx.debug_bounds("history_row_1").is_some()
    });

    let redraw = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };

    let refs_column_point = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        let selector = match row_ix {
            0 => "history_row_0",
            1 => "history_row_1",
            _ => panic!("unsupported row index {row_ix}"),
        };
        let row = cx
            .debug_bounds(selector)
            .expect("history row should be rendered");
        point(row.left() + px(24.0), row.center().y)
    };

    let away_from_refs_column_point = |cx: &mut gpui::VisualTestContext| {
        let row = cx
            .debug_bounds("history_row_0")
            .expect("history row should be rendered");
        point(row.right() - px(8.0), row.center().y)
    };

    let move_to_refs_column = |cx: &mut gpui::VisualTestContext| {
        let point = refs_column_point(cx, 0);
        cx.simulate_mouse_move(point, None, gpui::Modifiers::default());
        cx.run_until_parked();
        redraw(cx);
    };

    let open_refs_hover = |cx: &mut gpui::VisualTestContext| {
        move_to_refs_column(cx);
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        redraw(cx);
    };

    // The first chip represents both `main` and `origin/main`. A direct
    // right-click must disambiguate those exact refs rather than silently
    // choosing whichever one happened to be inserted first.
    let combined_chip_point = refs_column_point(cx, 0);
    cx.simulate_mouse_move(combined_chip_point, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        combined_chip_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        combined_chip_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    redraw(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .active_context_menu_invoker
                .as_ref()
                .map(|invoker| invoker.as_ref()),
            Some("history_branch_chip_menu_1_tip_main"),
            "a chip menu must pin the chip instead of the whole commit row"
        );
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::BranchRefsMenu {
                repo_id,
                display_name: "main".to_string(),
                targets: vec![
                    BranchMenuTarget {
                        section: BranchSection::Local,
                        name: "main".to_string(),
                    },
                    BranchMenuTarget {
                        section: BranchSection::Remote,
                        name: "origin/main".to_string(),
                    },
                ],
            })
        );
    });
    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| host.close_popover(cx));
    });
    cx.run_until_parked();
    redraw(cx);

    move_to_refs_column(cx);
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
    cx.update(|_window, app| {
        assert!(!crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });

    let away = away_from_refs_column_point(cx);
    cx.simulate_mouse_move(away, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    redraw(cx);
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
    cx.update(|_window, app| {
        assert!(!crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });

    open_refs_hover(cx);
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            None
        );
    });

    let feature_center = cx
        .debug_bounds("history_refs_hover_item_local_branch_feature")
        .expect("expected feature ref item in debug bounds")
        .center();
    cx.simulate_mouse_move(feature_center, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    redraw(cx);
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            None
        );
    });
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("feature".into()),
        "hover-menu rows should expose their complete ref name"
    );

    let click_hover_item =
        |cx: &mut gpui::VisualTestContext, selector: &'static str, button: gpui::MouseButton| {
            let center = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
                .center();
            cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
            cx.simulate_mouse_down(center, button, gpui::Modifiers::default());
            cx.simulate_mouse_up(center, button, gpui::Modifiers::default());
            cx.run_until_parked();
            redraw(cx);
        };

    click_hover_item(
        cx,
        "history_refs_hover_item_local_branch_feature",
        gpui::MouseButton::Left,
    );
    let feature_pinned_ix = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app)
    });
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: "feature".to_string(),
            })
        );
    });
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            feature_pinned_ix
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            Some("feature".into())
        );
    });

    click_hover_item(
        cx,
        "history_refs_hover_item_tag_release",
        gpui::MouseButton::Left,
    );
    let release_left_pinned_ix = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app)
    });
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::TagRefMenu {
                repo_id,
                commit_id: commit_id.clone(),
                name: "release".to_string()
            })
        );
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            release_left_pinned_ix
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            Some("release".into())
        );
    });

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| host.close_popover(cx));
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            None
        );
    });

    open_refs_hover(cx);
    click_hover_item(
        cx,
        "history_refs_hover_item_local_branch_feature",
        gpui::MouseButton::Right,
    );
    let feature_context_pinned_ix = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app)
    });
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: "feature".to_string(),
            })
        );
    });
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            feature_context_pinned_ix
        );
    });

    click_hover_item(
        cx,
        "history_refs_hover_item_tag_release",
        gpui::MouseButton::Right,
    );
    let release_context_pinned_ix = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app)
    });
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::TagRefMenu {
                repo_id,
                commit_id: commit_id.clone(),
                name: "release".to_string()
            })
        );
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            release_context_pinned_ix
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            Some("release".into())
        );
    });

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| host.close_popover(cx));
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            None
        );
    });

    open_refs_hover(cx);
    click_hover_item(
        cx,
        "history_refs_hover_item_tag_release",
        gpui::MouseButton::Left,
    );
    let release_pinned_ix = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app)
    });
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::TagRefMenu {
                repo_id,
                commit_id: commit_id.clone(),
                name: "release".to_string()
            })
        );
    });
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            release_pinned_ix
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            Some("release".into())
        );
    });

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| host.close_popover(cx));
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            None
        );
    });

    open_refs_hover(cx);
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    let source_bounds = cx
        .update(|_window, app| {
            crate::view::test_support::history_refs_hover_source_bounds(view.read(app), app)
        })
        .expect("history refs hover should expose source bounds");
    click_hover_item(
        cx,
        "history_refs_hover_item_local_branch_feature",
        gpui::MouseButton::Right,
    );
    let frozen_feature_pinned_ix = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app)
    });
    let frozen_source_bounds = cx
        .update(|_window, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::BranchMenu {
                    repo_id,
                    section: BranchSection::Local,
                    name: "feature".to_string(),
                })
            );
            assert_eq!(
                crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
                frozen_feature_pinned_ix
            );
            assert_eq!(
                crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
                Some("feature".into())
            );
            crate::view::test_support::history_refs_hover_source_bounds(view.read(app), app)
        })
        .expect("history refs hover should remain open while menu is open");

    let other_commit_ref_point = refs_column_point(cx, 1);
    cx.simulate_mouse_move(other_commit_ref_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(250));
    cx.run_until_parked();
    redraw(cx);
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: "feature".to_string(),
            })
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_source_bounds(view.read(app), app),
            Some(frozen_source_bounds)
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            frozen_feature_pinned_ix
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            Some("feature".into())
        );
    });

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| host.close_popover(cx));
    });
    cx.run_until_parked();
    redraw(cx);
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_ix(view.read(app), app),
            None
        );
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            None
        );
    });

    let row = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let away_x = if source_bounds.right() + px(8.0) < row.right() {
        source_bounds.right() + px(8.0)
    } else {
        source_bounds.left() - px(8.0)
    };
    let away = point(away_x, source_bounds.center().y);
    assert!(!source_bounds.contains(&away));
    cx.simulate_mouse_move(away, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let hover_open = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
    });
    assert!(!hover_open, "history refs hover host should be closed");
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
    cx.update(|_window, app| {
        assert!(!crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });
}

/// Commit rows open their hover and context menu from window-level mouse
/// listeners, which run for every event no matter what is painted over the
/// history. They must therefore defer to the hit test: a click that landed
/// on the collapsed sidebar's popover — or on the scrim that dismisses it —
/// belongs to that popover, not to the row it happens to cover.
#[gpui::test]
fn history_row_selection_follows_the_press_not_the_release(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let repo_path = PathBuf::from(format!(
        "/tmp/history-press-selects-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let commits = (0..12)
        .map(|ix| {
            let id = format!("c{ix:02}");
            commit(&id, &[], &format!("commit {ix:02}"))
        })
        .collect::<Vec<_>>();
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
    // Everything the panes read is already loaded, so rendering never has
    // to ask the store (and its worker threads) for data.
    repo.open = Loadable::Ready(());
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("feature", "c00")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    // The rows dispatch into the store, so it has to hold the same repo the
    // view renders; the reducer thread mutates exactly this state.
    store_for_assert.replace_snapshot_for_test(Arc::clone(&state));
    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_3").is_some()
    });

    let selected = |store: &AppStore| {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.clone())
    };
    let row = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
            .center()
    };

    // Positive control: an ordinary click selects, and the dispatch really
    // does reach the store, so the assertions below are not vacuous.
    let row_3 = row(cx, "history_row_3");
    cx.simulate_mouse_move(row_3, None, gpui::Modifiers::default());
    cx.simulate_click(row_3, gpui::Modifiers::default());
    wait_until(cx, "row 3 selected by a click", |_cx| {
        selected(&store_for_assert) == Some(CommitId("c03".into()))
    });

    // Press on one row, release on another: the press decides.
    let row_1 = row(cx, "history_row_1");
    let row_5 = row(cx, "history_row_5");
    cx.simulate_mouse_move(row_1, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(row_1, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(row_5, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(row_5, gpui::MouseButton::Left, gpui::Modifiers::default());

    wait_until(cx, "row 1 selected by the press", |_cx| {
        selected(&store_for_assert) == Some(CommitId("c01".into()))
    });
    // A release-driven selection would have been queued before this point,
    // so a short settle is enough to prove none was.
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
        assert_eq!(
            selected(&store_for_assert),
            Some(CommitId("c01".into())),
            "releasing over another row must not move the selection"
        );
    }
}

#[gpui::test]
fn history_rows_ignore_clicks_that_landed_on_the_collapsed_sidebar_popover(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commits = (0..12)
        .map(|ix| {
            let id = format!("c{ix:02}");
            commit(&id, &[], &format!("commit {ix:02}"))
        })
        .collect::<Vec<_>>();
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-collapsed-popover-clicks"),
        },
    );
    // Everything the sidebar reads is already loaded, so opening a section
    // popover never has to ask the store (and its worker threads) for data.
    repo.open = Loadable::Ready(());
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("feature", "c00")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_3").is_some()
    });

    // Draw only: every step here is synchronous, and pumping the executor
    // (or advancing the clock) would let store background work race the
    // deliberately deterministic test scheduler.
    let settle = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };
    let right_click = |cx: &mut gpui::VisualTestContext, at: Point<Pixels>| {
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        settle(cx);
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(
                crate::view::panes::sidebar::CollapsedSidebarSection::Local,
                cx,
            );
        });
    });
    settle(cx);
    settle(cx);

    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected the collapsed sidebar popover");
    let row = cx
        .debug_bounds("history_row_3")
        .expect("history row should be rendered");

    // Right of the popover, over the dismiss scrim, on a commit row: the
    // click dismisses the popover and stops there. That it dismisses at all
    // is what proves the event reached this point, so a silent commit menu
    // cannot be mistaken for nothing having been clicked.
    let on_scrim = point(panel.right() + px(120.0), row.center().y);
    assert!(
        row.contains(&on_scrim),
        "expected the test point to sit on a commit row (row={row:?}, point={on_scrim:?})"
    );
    right_click(cx, on_scrim);

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            None,
            "dismissing the popover must not open the commit menu underneath it"
        );
        assert_eq!(
            view.read(app).sidebar_collapsed_popover,
            None,
            "the click must still dismiss the popover"
        );
    });
}

#[gpui::test]
fn history_refs_hover_closes_when_history_scrolls_without_mouse_move(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commits = (0..80)
        .map(|ix| {
            let id = format!("c{ix:02}");
            commit(&id, &[], &format!("commit {ix:02}"))
        })
        .collect::<Vec<_>>();
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover-scroll"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("feature", "c00")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "history row with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });

    let row = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let hover_point = point(row.left() + px(24.0), row.center().y);
    cx.simulate_mouse_move(hover_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });

    let scroll_y = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_scroll.0.borrow().base_handle.offset().y
        })
    };
    let before_scroll_y = scroll_y(cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: hover_point,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-240.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    wait_until(cx, "history list to scroll", |cx| {
        scroll_y(cx) != before_scroll_y
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let hover_open = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
    });
    assert!(
        !hover_open,
        "history refs hover should close when history scrolls without a mouse move"
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
}

#[gpui::test]
fn history_refs_hover_does_not_open_while_overlay_is_open(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(vec![commit("c00", &[], "commit 00")], None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover-overlay"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("feature", "c00")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "history row with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });

    let row = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let refs_column_point = point(row.left() + px(24.0), row.center().y);

    // Open a context menu (an overlay) via right-click, away from the refs column.
    let menu_point = point(row.right() - px(8.0), row.center().y);
    cx.simulate_mouse_down(
        menu_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        menu_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "right-click should have opened a context menu overlay"
        );
    });

    // Hovering the refs column while the overlay is open must not open the hover:
    // the history canvas handles mouse-move at the window level, so it still fires
    // under the overlay, but the trigger is now guarded.
    cx.simulate_mouse_move(refs_column_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let hover_open = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
    });
    assert!(
        !hover_open,
        "history refs hover must not open while an overlay is open on top of it"
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
}

#[gpui::test]
fn history_refs_hover_closes_when_click_selects_another_commit_without_mouse_move(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commits = vec![
        commit("c1", &["c0"], "commit 1"),
        commit("c0", &[], "commit 0"),
    ];
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover-click-close"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("feature", "c1")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "history rows with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some() && cx.debug_bounds("history_row_1").is_some()
    });

    let hover_row = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let hover_point = point(hover_row.left() + px(24.0), hover_row.center().y);
    cx.simulate_mouse_move(hover_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });

    let other_row = cx
        .debug_bounds("history_row_1")
        .expect("second history row should be rendered");
    let click_point = point(other_row.right() - px(8.0), other_row.center().y);
    cx.simulate_mouse_down(
        click_point,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        click_point,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let hover_open = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
    });
    assert!(
        !hover_open,
        "history refs hover should close when another commit is clicked without a mouse move"
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
}

#[gpui::test]
fn history_refs_hover_item_click_keeps_existing_history_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let selected_commit = CommitId("c0".into());
    let hovered_commit = CommitId("c1".into());
    let commits = vec![
        commit(
            hovered_commit.as_ref(),
            &[selected_commit.as_ref()],
            "commit 1",
        ),
        commit(selected_commit.as_ref(), &[], "commit 0"),
    ];
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover-selection-priority"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.history_state.selected_commit = Some(selected_commit.clone());
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", hovered_commit.as_ref()),
        branch("feature", hovered_commit.as_ref()),
    ]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "history rows with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some() && cx.debug_bounds("history_row_1").is_some()
    });

    let hover_row = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let hover_point = point(hover_row.left() + px(24.0), hover_row.center().y);
    cx.simulate_mouse_move(hover_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert_eq!(
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history
                .active_repo()
                .and_then(|repo| repo.history_state.selected_commit.clone())
        }),
        Some(selected_commit.clone())
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());

    let item_center = cx
        .debug_bounds("history_refs_hover_item_local_branch_feature")
        .expect("expected feature ref item in debug bounds")
        .center();
    cx.simulate_mouse_move(item_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        item_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        item_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: "feature".to_string(),
            })
        );
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::history_refs_hover_pinned_item_text(view.read(app), app),
            Some("feature".into())
        );
    });
    assert_eq!(
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history
                .active_repo()
                .and_then(|repo| repo.history_state.selected_commit.clone())
        }),
        Some(selected_commit)
    );
}

#[gpui::test]
fn history_refs_hover_and_item_menu_close_when_history_page_changes_without_mouse_move(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let base_commit_id = CommitId("base".into());
    let initial_page = Arc::new(log_page(
        vec![
            commit("tip", &[base_commit_id.as_ref()], "tip"),
            commit(base_commit_id.as_ref(), &[], "base"),
        ],
        None,
    ));
    let mut initial_repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover-page-change"),
        },
    );
    initial_repo.history_state.history_scope = LogScope::AllBranches;
    initial_repo.head_branch = Loadable::Ready("main".to_string());
    initial_repo.head_branch_rev = 1;
    initial_repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", "tip"),
        branch("feature", "tip"),
    ]));
    initial_repo.branches_rev = 1;
    initial_repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    initial_repo.remote_branches_rev = 1;
    initial_repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    initial_repo.tags_rev = 1;
    initial_repo.log = Loadable::Ready(Arc::clone(&initial_page));
    initial_repo.log_rev = 1;
    initial_repo.history_state.log = Loadable::Ready(Arc::clone(&initial_page));
    initial_repo.history_state.log_rev = 1;

    let initial_state = Arc::new(AppState {
        repos: vec![initial_repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let switched_page = Arc::new(log_page(vec![commit("main-tip", &[], "main tip")], None));
    let mut switched_repo = initial_repo;
    switched_repo.history_state.history_scope = LogScope::CurrentBranch;
    switched_repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "main-tip")]));
    switched_repo.branches_rev = 2;
    switched_repo.log = Loadable::Ready(Arc::clone(&switched_page));
    switched_repo.log_rev = 2;
    switched_repo.history_state.log = Loadable::Ready(Arc::clone(&switched_page));
    switched_repo.history_state.log_rev = 2;

    let switched_state = Arc::new(AppState {
        repos: vec![switched_repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let apply_state = |cx: &mut gpui::VisualTestContext, state: Arc<AppState>| {
        cx.update(|window, app| {
            let ui_model = view.read(app).ui_model.clone();
            ui_model.update(app, |model, cx| {
                model.set_state(Arc::clone(&state), cx);
            });
            window.refresh();
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            history_view.update(app, |history, cx| history.ensure_history_cache(cx));
            window.refresh();
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    };

    apply_state(cx, initial_state);

    wait_until(cx, "history rows with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some() && cx.debug_bounds("history_row_1").is_some()
    });

    let refs_column_point = |cx: &mut gpui::VisualTestContext| {
        let row = cx
            .debug_bounds("history_row_0")
            .expect("history row should be rendered");
        point(row.left() + px(24.0), row.center().y)
    };
    let hover_point = refs_column_point(cx);
    cx.simulate_mouse_move(hover_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let feature_center = cx
        .debug_bounds("history_refs_hover_item_local_branch_feature")
        .expect("expected feature ref item in debug bounds")
        .center();
    cx.simulate_mouse_move(feature_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        feature_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        feature_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: "feature".to_string(),
            })
        );
    });

    apply_state(cx, switched_state);

    wait_until(cx, "switched history row", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.row_vms.len() == 1
                    && cache.base.row_vms[0].summary.as_ref() == "main tip"
            })
        })
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let hover_open = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
    });
    assert!(
        !hover_open,
        "history refs hover should close when the history page changes without a mouse move"
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            None,
            "history refs item menu should close when the history page changes"
        );
    });
}

#[gpui::test]
fn history_refs_hover_closes_when_history_scrolls_programmatically(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let selected_commit = CommitId("c50".into());
    let commits = (0..80)
        .map(|ix| {
            let id = format!("c{ix:02}");
            commit(&id, &[], &format!("commit {ix:02}"))
        })
        .collect::<Vec<_>>();
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refs-hover-programmatic-scroll"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.history_state.selected_commit = Some(selected_commit.clone());
    repo.history_state.commit_details =
        Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
            id: selected_commit.clone(),
            message: "commit 50".into(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "2026-05-26 12:00:00 +0300".into(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files: vec![],
        }));
    repo.branches = Loadable::Ready(Arc::new(vec![branch("feature", "c00")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| {
            model.set_state(Arc::clone(&state), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "history row with displayed refs", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });

    let row = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let hover_point = point(row.left() + px(24.0), row.center().y);
    cx.simulate_mouse_move(hover_point, None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });

    let scroll_y = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_scroll.0.borrow().base_handle.offset().y
        })
    };
    let before_scroll_y = scroll_y(cx);

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            history.request_reveal_commit(repo_id, selected_commit.clone(), None, cx);
        });
        window.refresh();
        let _ = window.draw(app);
    });
    wait_until(cx, "history list to scroll programmatically", |cx| {
        scroll_y(cx) != before_scroll_y
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let hover_open = cx.update(|_window, app| {
        crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
    });
    assert!(
        !hover_open,
        "history refs hover should close when history scrolls programmatically"
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
}

#[gpui::test]
fn current_branch_remote_branch_changes_reuse_base_cache_and_refresh_decorations(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(vec![commit("tip", &[], "tip")], None));
    let repo_path = PathBuf::from("/tmp/history-current-branch-remote-reuse");

    let mut initial_repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
    initial_repo.history_state.history_scope = LogScope::CurrentBranch;
    initial_repo.head_branch = Loadable::Ready("main".to_string());
    initial_repo.head_branch_rev = 1;
    initial_repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    initial_repo.branches_rev = 1;
    initial_repo.remote_branches =
        Loadable::Ready(Arc::new(vec![remote_branch("origin", "main", "tip")]));
    initial_repo.remote_branches_rev = 1;
    initial_repo.log = Loadable::Ready(Arc::clone(&page));
    initial_repo.log_rev = 1;
    initial_repo.history_state.log = Loadable::Ready(Arc::clone(&page));
    initial_repo.history_state.log_rev = 1;

    let initial_state = Arc::new(AppState {
        repos: vec![initial_repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let mut updated_repo = initial_repo;
    updated_repo.remote_branches = Loadable::Ready(Arc::new(vec![
        remote_branch("origin", "main", "tip"),
        remote_branch("upstream", "main", "tip"),
    ]));
    updated_repo.remote_branches_rev = 2;

    let updated_state = Arc::new(AppState {
        repos: vec![updated_repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(cx, &view, initial_state);

    wait_until(cx, "initial current-branch history cache", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.request.remote_branches_rev == 0
                    && cache.decorations.row_vms.len() == 1
                    && cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .contains("origin/main")
            })
        })
    });

    let (before_graph_rows, before_base_request, before_branches_text) =
        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let rows_len = history_view.update(app, |history, cx| {
                HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
            });
            assert_eq!(rows_len, 1, "initial current-branch row should render");

            let history = history_view.read(app);
            let cache = history
                .history_cache
                .as_ref()
                .expect("history cache should be available");
            (
                Arc::clone(&cache.base.graph_rows),
                cache.base.request.clone(),
                cache.decorations.row_vms[0]
                    .branches_text
                    .as_ref()
                    .to_owned(),
            )
        });

    assert!(before_branches_text.contains("origin/main"));
    assert!(!before_branches_text.contains("upstream/main"));

    ensure_history_cache_for_tests(cx, &view, updated_state);

    wait_until(cx, "updated current-branch decorations", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.request.remote_branches_rev == 0
                    && cache.decorations.request.remote_branches_rev == 2
                    && cache.decorations.row_vms.len() == 1
                    && cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .contains("upstream/main")
            })
        })
    });

    let (after_graph_rows, after_base_request, after_branches_text) = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        let rows_len = history_view.update(app, |history, cx| {
            HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
        });
        assert_eq!(
            rows_len, 1,
            "updated current-branch row should still render"
        );

        let history = history_view.read(app);
        let cache = history
            .history_cache
            .as_ref()
            .expect("history cache should be available");
        (
            Arc::clone(&cache.base.graph_rows),
            cache.base.request.clone(),
            cache.decorations.row_vms[0]
                .branches_text
                .as_ref()
                .to_owned(),
        )
    });

    assert!(
        Arc::ptr_eq(&before_graph_rows, &after_graph_rows),
        "remote branch changes in current-branch mode should reuse the heavy base cache"
    );
    assert_eq!(after_base_request, before_base_request);
    assert!(after_branches_text.contains("origin/main"));
    assert!(after_branches_text.contains("upstream/main"));
}

#[gpui::test]
fn current_branch_local_branch_changes_reuse_base_cache_and_refresh_decorations(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(vec![commit("tip", &[], "tip")], None));
    let repo_path = PathBuf::from("/tmp/history-current-branch-local-reuse");

    let mut initial_repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
    initial_repo.history_state.history_scope = LogScope::CurrentBranch;
    initial_repo.head_branch = Loadable::Ready("main".to_string());
    initial_repo.head_branch_rev = 1;
    initial_repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    initial_repo.branches_rev = 1;
    initial_repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    initial_repo.remote_branches_rev = 1;
    initial_repo.log = Loadable::Ready(Arc::clone(&page));
    initial_repo.log_rev = 1;
    initial_repo.history_state.log = Loadable::Ready(Arc::clone(&page));
    initial_repo.history_state.log_rev = 1;

    let initial_state = Arc::new(AppState {
        repos: vec![initial_repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let mut updated_repo = initial_repo;
    updated_repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", "tip"),
        branch("feature", "tip"),
    ]));
    updated_repo.branches_rev = 2;

    let updated_state = Arc::new(AppState {
        repos: vec![updated_repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(cx, &view, initial_state);

    wait_until(cx, "initial current-branch local history cache", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.request.branches_rev == 0
                    && cache.decorations.row_vms.len() == 1
                    && cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .contains("main")
            })
        })
    });

    let (before_graph_rows, before_base_request, before_branches_text) =
        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let rows_len = history_view.update(app, |history, cx| {
                HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
            });
            assert_eq!(rows_len, 1, "initial current-branch row should render");

            let history = history_view.read(app);
            let cache = history
                .history_cache
                .as_ref()
                .expect("history cache should be available");
            (
                Arc::clone(&cache.base.graph_rows),
                cache.base.request.clone(),
                cache.decorations.row_vms[0]
                    .branches_text
                    .as_ref()
                    .to_owned(),
            )
        });

    assert!(before_branches_text.contains("main"));
    assert!(!before_branches_text.contains("feature"));

    ensure_history_cache_for_tests(cx, &view, updated_state);

    wait_until(cx, "updated current-branch local decorations", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.request.branches_rev == 0
                    && cache.decorations.request.branches_rev == 2
                    && cache.decorations.row_vms.len() == 1
                    && cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .contains("feature")
            })
        })
    });

    let (after_graph_rows, after_base_request, after_branches_text) = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        let rows_len = history_view.update(app, |history, cx| {
            HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
        });
        assert_eq!(
            rows_len, 1,
            "updated current-branch row should still render"
        );

        let history = history_view.read(app);
        let cache = history
            .history_cache
            .as_ref()
            .expect("history cache should be available");
        (
            Arc::clone(&cache.base.graph_rows),
            cache.base.request.clone(),
            cache.decorations.row_vms[0]
                .branches_text
                .as_ref()
                .to_owned(),
        )
    });

    assert!(
        Arc::ptr_eq(&before_graph_rows, &after_graph_rows),
        "local branch changes in current-branch mode should reuse the heavy base cache"
    );
    assert_eq!(after_base_request, before_base_request);
    assert!(after_branches_text.contains("main"));
    assert!(after_branches_text.contains("feature"));
}

#[gpui::test]
fn current_branch_head_target_changes_rebuild_base_cache_and_move_head_marker(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(
        vec![commit("tip", &["base"], "tip"), commit("base", &[], "base")],
        None,
    ));
    let repo_path = PathBuf::from("/tmp/history-current-branch-head-target");

    let mut initial_repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
    initial_repo.history_state.history_scope = LogScope::CurrentBranch;
    initial_repo.head_branch = Loadable::Ready("main".to_string());
    initial_repo.head_branch_rev = 1;
    initial_repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    initial_repo.branches_rev = 1;
    initial_repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    initial_repo.remote_branches_rev = 1;
    initial_repo.log = Loadable::Ready(Arc::clone(&page));
    initial_repo.log_rev = 1;
    initial_repo.history_state.log = Loadable::Ready(Arc::clone(&page));
    initial_repo.history_state.log_rev = 1;

    let initial_state = Arc::new(AppState {
        repos: vec![initial_repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let mut updated_repo = initial_repo;
    updated_repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "base")]));
    updated_repo.branches_rev = 2;

    let updated_state = Arc::new(AppState {
        repos: vec![updated_repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(cx, &view, initial_state);

    wait_until(cx, "initial current-branch head target cache", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.request.branches_rev == 0
                    && cache
                        .base
                        .request
                        .head_branch_target
                        .as_ref()
                        .map(AsRef::as_ref)
                        == Some("tip")
                    && cache.base.row_vms.len() == 2
                    && cache.base.row_vms[0].is_head
                    && !cache.base.row_vms[1].is_head
                    && cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .contains("main")
            })
        })
    });

    let (before_graph_rows, before_base_request, before_head_rows, before_branches_text) = cx
        .update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let rows_len = history_view.update(app, |history, cx| {
                HistoryView::render_history_table_rows(history, 0..2, window, cx).len()
            });
            assert_eq!(rows_len, 2, "initial rows should render");

            let history = history_view.read(app);
            let cache = history
                .history_cache
                .as_ref()
                .expect("history cache should be available");
            (
                Arc::clone(&cache.base.graph_rows),
                cache.base.request.clone(),
                cache
                    .base
                    .row_vms
                    .iter()
                    .map(|row| row.is_head)
                    .collect::<Vec<_>>(),
                cache
                    .decorations
                    .row_vms
                    .iter()
                    .map(|row| row.branches_text.as_ref().to_owned())
                    .collect::<Vec<_>>(),
            )
        });

    assert_eq!(before_head_rows, vec![true, false]);
    assert!(before_branches_text[0].contains("main"));
    assert!(before_branches_text[1].is_empty());

    ensure_history_cache_for_tests(cx, &view, updated_state);

    wait_until(cx, "updated current-branch head target cache", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::CurrentBranch
                    && cache.base.request.branches_rev == 0
                    && cache
                        .base
                        .request
                        .head_branch_target
                        .as_ref()
                        .map(AsRef::as_ref)
                        == Some("base")
                    && cache.base.row_vms.len() == 2
                    && !cache.base.row_vms[0].is_head
                    && cache.base.row_vms[1].is_head
                    && cache.decorations.row_vms[1]
                        .branches_text
                        .as_ref()
                        .contains("main")
            })
        })
    });

    let (after_graph_rows, after_base_request, after_head_rows, after_branches_text) =
        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let rows_len = history_view.update(app, |history, cx| {
                HistoryView::render_history_table_rows(history, 0..2, window, cx).len()
            });
            assert_eq!(rows_len, 2, "updated rows should still render");

            let history = history_view.read(app);
            let cache = history
                .history_cache
                .as_ref()
                .expect("history cache should be available");
            (
                Arc::clone(&cache.base.graph_rows),
                cache.base.request.clone(),
                cache
                    .base
                    .row_vms
                    .iter()
                    .map(|row| row.is_head)
                    .collect::<Vec<_>>(),
                cache
                    .decorations
                    .row_vms
                    .iter()
                    .map(|row| row.branches_text.as_ref().to_owned())
                    .collect::<Vec<_>>(),
            )
        });

    assert!(
        !Arc::ptr_eq(&before_graph_rows, &after_graph_rows),
        "head target changes should rebuild the heavy base cache in current-branch mode"
    );
    assert_eq!(before_base_request.branches_rev, 0);
    assert_eq!(after_base_request.branches_rev, 0);
    assert_ne!(after_base_request, before_base_request);
    assert_eq!(
        before_base_request
            .head_branch_target
            .as_ref()
            .map(AsRef::as_ref),
        Some("tip")
    );
    assert_eq!(
        after_base_request
            .head_branch_target
            .as_ref()
            .map(AsRef::as_ref),
        Some("base")
    );
    assert_eq!(after_head_rows, vec![false, true]);
    assert!(after_branches_text[0].is_empty());
    assert!(after_branches_text[1].contains("main"));
}

#[gpui::test]
fn history_scope_switch_keeps_rows_visible_and_refreshes_automatically(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let initial_scope = LogScope::FullReachable;
    let switched_scope = LogScope::AllBranches;
    let repo_path = PathBuf::from("/tmp/history-scope-switch-test");
    let initial_page = Arc::new(log_page(vec![commit("main-tip", &[], "main tip")], None));
    let switched_page = Arc::new(log_page(
        vec![
            commit("all-tip", &[], "all branches tip"),
            commit("main-tip", &[], "main tip"),
        ],
        None,
    ));

    let mut initial_repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
    initial_repo.history_state.history_scope = initial_scope;
    initial_repo.log = Loadable::Ready(Arc::clone(&initial_page));
    initial_repo.log_rev = 1;
    initial_repo.history_state.log = Loadable::Ready(Arc::clone(&initial_page));
    initial_repo.history_state.log_rev = 1;

    let initial_state = Arc::new(AppState {
        repos: vec![initial_repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let mut loading_repo = initial_repo.clone();
    loading_repo.history_state.history_scope = switched_scope;
    loading_repo.log = Loadable::Loading;
    loading_repo.log_rev = 2;
    loading_repo.history_state.log = Loadable::Loading;
    loading_repo.history_state.log_rev = 2;
    loading_repo.history_state.retained_log_while_loading = Some(Arc::clone(&initial_page));

    let loading_state = Arc::new(AppState {
        repos: vec![loading_repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    let mut loaded_repo = loading_repo;
    loaded_repo.log = Loadable::Ready(Arc::clone(&switched_page));
    loaded_repo.log_rev = 3;
    loaded_repo.history_state.log = Loadable::Ready(Arc::clone(&switched_page));
    loaded_repo.history_state.log_rev = 3;
    loaded_repo.history_state.retained_log_while_loading = None;

    let loaded_state = Arc::new(AppState {
        repos: vec![loaded_repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    ensure_history_cache_for_tests(cx, &view, Arc::clone(&initial_state));

    wait_until(cx, "initial history rows", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == initial_scope
                    && cache.base.visible_indices.len() == 1
                    && cache.base.row_vms.len() == 1
                    && cache.base.row_vms[0].summary.as_ref() == "main tip"
            })
        })
    });

    ensure_history_cache_for_tests(cx, &view, Arc::clone(&loading_state));

    wait_until(cx, "retained history rows during loading", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.active_repo().is_some_and(|repo| {
                repo.history_state.history_scope == switched_scope
                    && matches!(repo.log, Loadable::Loading)
                    && repo
                        .history_state
                        .retained_log_while_loading
                        .as_ref()
                        .is_some_and(|page| Arc::ptr_eq(page, &initial_page))
            }) && history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.visible_indices.len() == 1
                    && cache.base.row_vms.len() == 1
                    && cache.base.row_vms[0].summary.as_ref() == "main tip"
            })
        })
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            let rows = HistoryView::render_history_table_rows(history, 0..1, window, cx);
            assert_eq!(rows.len(), 1, "retained history row should still render");
        });
    });

    ensure_history_cache_for_tests(cx, &view, Arc::clone(&loaded_state));

    wait_until(cx, "history rows refresh after scope load", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == switched_scope
                    && cache.base.visible_indices.len() == 2
                    && cache.base.row_vms.len() == 2
                    && cache.base.row_vms[0].summary.as_ref() == "all branches tip"
                    && cache.base.row_vms[1].summary.as_ref() == "main tip"
            })
        })
    });
}

#[gpui::test]
fn filtered_modes_do_not_infer_detached_head_target_from_first_visible_row(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for (scope, commits, expected_summary) in [
        (
            LogScope::NoMerges,
            vec![commit("visible", &["hidden"], "visible non-merge")],
            "visible non-merge",
        ),
        (
            LogScope::MergesOnly,
            vec![commit("visible-merge", &["p0", "p1"], "visible merge")],
            "visible merge",
        ),
    ] {
        let page = Arc::new(log_page(commits, None));
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/history-detached-head-filtered"),
            },
        );
        repo.history_state.history_scope = scope;
        repo.head_branch = Loadable::Ready("HEAD".to_string());
        repo.head_branch_rev = 1;
        repo.log = Loadable::Ready(Arc::clone(&page));
        repo.log_rev = 1;
        repo.history_state.log = Loadable::Ready(page);
        repo.history_state.log_rev = 1;

        let state = Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..Default::default()
        });

        ensure_history_cache_for_tests(cx, &view, state);

        let description = format!("filtered {scope:?} history cache");
        wait_until(cx, &description, |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request.history_scope == scope
                        && cache.base.row_vms.len() == 1
                        && !cache.base.row_vms[0].is_head
                        && cache.base.row_vms[0].summary.as_ref() == expected_summary
                })
            })
        });
    }
}

#[gpui::test]
fn retained_history_rows_support_keyboard_navigation_while_loading(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let first = CommitId("tip".into());
    let second = CommitId("base".into());
    let repo_path = PathBuf::from(format!(
        "/tmp/history-retained-nav-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    store_for_assert.dispatch(Msg::OpenRepo(repo_path.clone()));
    wait_until(cx, "opened repo placeholder", |_cx| {
        let snapshot = store_for_assert.snapshot();
        snapshot.active_repo == Some(repo_id)
            && snapshot.repos.iter().any(|repo| repo.id == repo_id)
    });

    let page = Arc::new(log_page(
        vec![commit("tip", &["base"], "tip"), commit("base", &[], "base")],
        None,
    ));
    let mut repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.history_state.selected_commit = Some(first.clone());
    repo.history_state.retained_log_while_loading = Some(Arc::clone(&page));
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.log = Loadable::Loading;
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Loading;
    repo.history_state.log_rev = 1;

    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    });

    ensure_history_cache_for_tests(cx, &view, state);

    wait_until(cx, "retained rows available during loading", |cx| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            let history = history_view.read(app);
            history.active_repo().is_some_and(|repo| {
                repo.history_state.history_scope == LogScope::AllBranches
                    && matches!(repo.log, Loadable::Loading)
                    && repo.history_state.retained_log_while_loading.is_some()
                    && repo.history_state.selected_commit.as_ref() == Some(&first)
            }) && history.history_cache.as_ref().is_some_and(|cache| {
                cache.base.request.history_scope == LogScope::AllBranches
                    && cache.base.row_vms.len() == 2
                    && cache.base.row_vms[0].summary.as_ref() == "tip"
                    && cache.base.row_vms[1].summary.as_ref() == "base"
            })
        })
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            assert!(history.history_select_adjacent_commit(1, cx));
        });
        window.refresh();
        let _ = window.draw(app);
    });

    wait_until(cx, "selected second retained commit", |_cx| {
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        repo.history_state.selected_commit.as_ref() == Some(&second)
    });
}
