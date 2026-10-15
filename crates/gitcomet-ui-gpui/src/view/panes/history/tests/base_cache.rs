use super::*;

fn author_run_base(commits: Vec<Commit>) -> (HistoryBaseCache, LogPage) {
    let page = log_page(commits, None);
    let base = build_history_base_cache(
        HistoryBaseCacheRequest {
            repo_id: RepoId(1),
            history_scope: LogScope::AllBranches,
            log_source: 0,
            history_author_filter: None,
            head_branch_rev: 0,
            detached_head_commit: None,
            head_branch_target: None,
            branches_rev: 0,
            remote_branches_rev: 0,
            stashes_rev: 0,
        },
        &page,
        AppTheme::gitcomet_dark(),
        None,
        &[],
        &[],
        &[],
    );
    (base, page)
}

fn interiors(base: &HistoryBaseCache) -> Vec<bool> {
    base.row_vms
        .iter()
        .map(|row| row.author_run_interior)
        .collect()
}

#[test]
fn author_runs_keep_both_ends_and_compare_full_names() {
    let mut commits = (0..7)
        .map(|ix| commit(&format!("c{ix}"), &[&format!("c{}", ix + 1)], "same lane"))
        .collect::<Vec<_>>();
    for (ix, commit) in commits.iter_mut().enumerate() {
        commit.author = if ix < 3 { "Ada Lovelace" } else { "Alan Light" }.into();
    }
    let (base, _) = author_run_base(commits);
    // Both authors have AL initials; identity comes from the full author name.
    assert_eq!(
        interiors(&base),
        [false, true, false, false, true, true, false]
    );
}

#[test]
fn author_runs_follow_connections_across_interleaved_lanes() {
    let (base, _) = author_run_base(vec![
        commit("a0", &["a1"], "lane A"),
        commit("b0", &["b1"], "lane B"),
        commit("a1", &["a2"], "lane A"),
        commit("b1", &["b2"], "lane B"),
        commit("a2", &[], "lane A"),
        commit("b2", &[], "lane B"),
    ]);
    assert_eq!(interiors(&base), [false, false, true, true, false, false]);
}

/// Indexed windows only carry this much context, so a longer link must not
/// count in cached history either, or the two modes would disagree.
#[test]
fn author_runs_ignore_links_beyond_the_window_context() {
    use crate::view::caches::{
        HISTORY_AUTHOR_RUN_MAX_LINK_ROWS, HistoryAuthorRunRow, history_author_run_interiors,
    };
    let graph = history_graph::GraphRow {
        lanes_now: Default::default(),
        lanes_next: Default::default(),
        joins_in: Default::default(),
        edges_out: Default::default(),
        node_col: 0,
        node_color_ix: 0,
        is_merge: false,
        from_node_cols: Default::default(),
    };
    for gap in [
        HISTORY_AUTHOR_RUN_MAX_LINK_ROWS,
        HISTORY_AUTHOR_RUN_MAX_LINK_ROWS + 1,
    ] {
        let commits = [
            commit("a", &["b"], "child"),
            commit("b", &["c"], "middle"),
            commit("c", &[], "parent"),
        ];
        let rows = commits
            .iter()
            .map(|commit| HistoryAuthorRunRow {
                commit,
                graph: &graph,
                is_stash: false,
            })
            .collect::<Vec<_>>();
        // Other lanes' rows sit between the middle commit and its parent.
        let row_of = |id: &CommitId| match id.as_ref() {
            "a" => Some(0),
            "b" => Some(1),
            "c" => Some(1 + gap),
            _ => None,
        };
        let mut padded = rows;
        padded.splice(
            2..2,
            (1..gap).map(|_| HistoryAuthorRunRow {
                commit: &commits[2],
                graph: &graph,
                is_stash: true,
            }),
        );
        let interiors = history_author_run_interiors(&padded, row_of);
        assert_eq!(interiors[1], gap <= HISTORY_AUTHOR_RUN_MAX_LINK_ROWS);
    }
}

#[test]
fn author_runs_break_at_special_and_unloaded_nodes() {
    let (mut base, mut page) = author_run_base(
        (0..7)
            .map(|ix| commit(&format!("c{ix}"), &[&format!("c{}", ix + 1)], "same author"))
            .collect(),
    );
    assert_eq!(
        interiors(&base),
        [false, true, true, true, true, true, false]
    );
    base.row_vms[3].is_stash = true;
    crate::view::caches::mark_history_author_runs(&mut base, &page);
    assert_eq!(
        interiors(&base),
        [false, true, false, false, false, true, false]
    );
    base.row_vms[3].is_stash = false;
    Arc::make_mut(&mut base.graph_rows)[3].is_merge = true;
    crate::view::caches::mark_history_author_runs(&mut base, &page);
    assert_eq!(
        interiors(&base),
        [false, true, false, false, false, true, false]
    );
    Arc::make_mut(&mut base.graph_rows)[3].is_merge = false;
    // An unloaded indexed row is a placeholder without an author.
    page.commits[3].author = "".into();
    crate::view::caches::mark_history_author_runs(&mut base, &page);
    assert_eq!(
        interiors(&base),
        [false, true, false, false, false, true, false]
    );
}

/// The commit-id index the base cache carries agrees with the visible order it
/// was built from.
///
/// Its readers -- the worktree row anchors and the selected lane's colour --
/// look commits up during layout, and both used to scan the page instead. A
/// map that disagrees with `visible_indices` would anchor rows on the wrong
/// commits, so this pins the two together.
#[test]
fn the_base_cache_indexes_every_visible_commit_by_id() {
    let commits = vec![
        commit("c0", &["c1"], "newest"),
        commit("c1", &["c2"], "middle"),
        commit("c2", &[], "oldest"),
    ];
    let page = log_page(commits, None);
    let base = build_history_base_cache(
        HistoryBaseCacheRequest {
            repo_id: RepoId(1),
            history_scope: LogScope::AllBranches,
            log_source: 0,
            history_author_filter: None,
            head_branch_rev: 0,
            detached_head_commit: None,
            head_branch_target: None,
            branches_rev: 0,
            remote_branches_rev: 0,
            stashes_rev: 0,
        },
        &page,
        AppTheme::gitcomet_dark(),
        None,
        &[],
        &[],
        &[],
    );

    for (visible_ix, commit_ix) in base.visible_indices.iter().enumerate() {
        let id = &page.commits[commit_ix].id;
        assert_eq!(
            base.visible_ix_by_commit.get(id).copied(),
            Some(visible_ix),
            "{id:?} should resolve to the row it renders at"
        );
    }
    assert_eq!(base.visible_ix_by_commit.len(), base.visible_indices.len());
    assert_eq!(
        base.visible_ix_by_commit.get(&CommitId("absent".into())),
        None
    );
}

/// Indexed windows are block aligned, so the commit on a window's first or
/// last row has its child or parent outside the window. Run marks must still
/// see that neighbour, or the node shows initials until a scroll reloads a
/// window that contains it and then shrinks to a dot.
#[gpui::test]
fn indexed_author_runs_do_not_change_at_window_edges(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::history_index::HISTORY_BLOCK_SIZE;

    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = super::refresh::indexed_fixture(3 * HISTORY_BLOCK_SIZE);
    let (view, cx, mut state, store) =
        super::refresh::mount(cx, Arc::new(log_page(commits.clone(), None)));
    install_index(&mut state, index.clone());
    let indexed = &mut state.repos[0].history_state.indexed;
    indexed.range_index = Some(index.clone());
    for start in (0..commits.len()).step_by(HISTORY_BLOCK_SIZE) {
        indexed.ranges.insert(
            start,
            Arc::new(gitcomet_core::history_index::HistoryRange {
                snapshot: index.snapshot.clone(),
                start,
                commits: commits[start..start + HISTORY_BLOCK_SIZE].to_vec(),
            }),
        );
    }
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state));
    wait_until(cx, "indexed viewport", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let interior_at = |cx: &mut gpui::VisualTestContext, top: usize, row: usize| {
        cx.update(|window, app| {
            history.update(app, |history, cx| {
                let mut scroll = history.scroll_interaction.borrow_mut();
                let logical = scroll.logical.as_mut().unwrap();
                logical.set_position(top as f64 * logical.height);
                cx.notify();
            });
            window.refresh();
            let _ = window.draw(app);
        });
        wait_until(cx, "indexed window around the viewport", |cx| {
            cx.update(|_, app| {
                history
                    .read(app)
                    .indexed
                    .window
                    .as_ref()
                    .is_some_and(|window| {
                        window.start <= top
                            && top < window.start + window.cache.base.row_vms.len()
                            && window.loaded.iter().all(|loaded| *loaded)
                    })
            })
        });
        cx.update(|_, app| {
            let window = history.read(app).indexed.window.clone().unwrap();
            let ix = window.cache.base.visible_ix_by_commit[&commits[row].id];
            (
                window.start,
                window.cache.base.row_vms[ix].author_run_interior,
            )
        })
    };
    let first = HISTORY_BLOCK_SIZE;
    // One row above the block boundary loads the previous block too.
    assert_eq!(interior_at(cx, first - 1, first), (0, true));
    // The boundary row is now the window's first row; its child is outside.
    let (start, interior) = interior_at(cx, first, first);
    assert_eq!(start, first);
    assert!(interior, "the first row of a window lost its run neighbour");
    // The viewport's last row sits on the window's last row.
    let rows = cx.update(|_, app| {
        let history = history.read(app);
        let scroll = history.scroll_interaction.borrow();
        scroll.logical.as_ref().unwrap().visible_range().len()
    });
    let top = 2 * HISTORY_BLOCK_SIZE - rows;
    let last = 2 * HISTORY_BLOCK_SIZE - 1;
    let (start, interior) = interior_at(cx, top, last);
    assert_eq!(start, first);
    assert!(interior, "the last row of a window lost its run neighbour");
}
