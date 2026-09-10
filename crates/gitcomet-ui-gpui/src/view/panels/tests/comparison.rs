use super::*;
use gitcomet_core::domain::{Commit, CommitFileChange, CommitId, CommitParentIds, FileStatusKind};
use gitcomet_state::model::{CommitMultiSelection, Loadable, RangeSelection, RepoId};

fn sha(ix: usize) -> String {
    format!("{ix:040}")
}

fn log_commit(ix: usize) -> Commit {
    Commit {
        id: CommitId(sha(ix).into()),
        parent_ids: CommitParentIds::new(),
        summary: format!("commit {ix}").into(),
        author: "Alice".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

/// What state the comparison's changed-file list is in. `Loaded(n)` is the
/// common case; the other two exist because their render paths differ in ways
/// that are easy to regress into each other.
enum Files {
    Loading,
    Failed(&'static str),
    Loaded(usize),
}

impl Files {
    fn into_loadable(self) -> Loadable<Arc<Vec<CommitFileChange>>> {
        match self {
            Files::Loading => Loadable::Loading,
            Files::Failed(message) => Loadable::Error(message.to_string()),
            Files::Loaded(count) => Loadable::Ready(Arc::new(
                (0..count)
                    .map(|ix| CommitFileChange {
                        path: std::path::PathBuf::from(format!("src/file_{ix}.rs")),
                        kind: FileStatusKind::Modified,
                        is_submodule: false,
                        additions: Some(1),
                        deletions: Some(0),
                    })
                    .collect(),
            )),
        }
    }
}

/// Put the details pane into an active comparison and draw it. `commit_count`
/// commits go into the log (newest first, so index 0 is the tip); `selected` is
/// how many of them are also multi-selected, which is what decides whether the
/// comparison presents itself as a merged multi-selection or as two named
/// points. `files` is the state of the changed-file section below the cards.
fn draw_comparison(
    cx: &mut gpui::TestAppContext,
    repo_id: RepoId,
    commit_count: usize,
    selected: usize,
    files: Files,
) -> &mut gpui::VisualTestContext {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-range-compare"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: (0..commit_count).map(log_commit).collect(),
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.multi_selection = CommitMultiSelection {
                commits: (0..selected).map(|ix| CommitId(sha(ix).into())).collect(),
                ..Default::default()
            };
            // Oldest is the base, newest the tip — the same ordering the
            // reducer produces for either flow.
            repo.history_state.range_selection = Some(RangeSelection {
                from: CommitId(sha(commit_count.saturating_sub(1)).into()),
                to: Some(CommitId(sha(0).into())),
                from_label: "base".into(),
                to_label: "tip".into(),
            });
            repo.history_state.range_files = files.into_loadable();

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx
}

/// A range comparison started via "mark + compare" (or a branch/tag/worktree
/// compare) leaves `multi_selection` empty — the endpoints live only in
/// `range_selection`. The compared-commit preview cards must still render,
/// resolved from those endpoints by looking each SHA up in the log.
///
/// Regression guard: the cards were built with a virtualized `uniform_list`
/// inside a fixed-height, non-scrolling container, which painted nothing and
/// left only the "Viewing diff between N commits" subheader visible.
#[gpui::test]
fn range_comparison_renders_endpoint_commit_cards(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(77), 2, 0, Files::Loaded(1));

    assert!(
        cx.debug_bounds("commit_multi_row_0").is_some(),
        "expected the tip commit's preview card to render for a range comparison"
    );
    assert!(
        cx.debug_bounds("commit_multi_row_1").is_some(),
        "expected the base commit's preview card to render for a range comparison"
    );
}

/// Every plain history click leaves one commit in `multi_selection`, so a
/// comparison started from a context menu afterwards finds a stale selection
/// sitting there. The cards must come from the comparison's own endpoints, not
/// from that leftover — showing it would name a commit that is not part of the
/// comparison at all.
#[gpui::test]
fn a_leftover_single_selection_does_not_supply_the_cards(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(78), 3, 1, Files::Loaded(1));

    assert!(
        cx.debug_bounds("commit_multi_row_0").is_some(),
        "expected the comparison's tip card"
    );
    assert!(
        cx.debug_bounds("commit_multi_row_1").is_some(),
        "expected both endpoints as cards, not the single leftover selection"
    );
}

/// A multi-selection comparison renders one card per selected commit, so a
/// large selection would otherwise grow an unbounded column and push the
/// changed-file list — the part the user came for — off the pane. The card
/// area is capped at half the comparison body, so the two lists split it
/// evenly and the cards scroll beyond that.
#[gpui::test]
fn a_large_multi_selection_splits_the_body_evenly_with_the_file_list(
    cx: &mut gpui::TestAppContext,
) {
    // Far more commits than can fit in half the body at ~44px per card. The
    // `debug_bounds` selectors below are literals, so they track these.
    const REPO_ID: RepoId = RepoId(79);
    const SELECTED: usize = 60;
    let cx = draw_comparison(cx, REPO_ID, SELECTED, SELECTED, Files::Loaded(3));

    let body = cx
        .debug_bounds("range_comparison_body")
        .expect("the comparison body should render");
    let cards = cx
        .debug_bounds("range_comparison_cards")
        .expect("the card area should render");
    let files = cx
        .debug_bounds("range_comparison_files")
        .expect("the changed-file section should render");
    let row = cx
        .debug_bounds("commit_multi_row_0")
        .expect("the first card should render");

    // Guard the comparisons below against a collapsed (zero-height) layout,
    // which would satisfy them vacuously.
    assert!(
        cards.size.height >= row.size.height,
        "the card area should be at least one card tall"
    );

    // Derive the uncapped height from a real card rather than the layout
    // constant, so this stays honest if the card height changes.
    let natural_height = row.size.height * SELECTED as f32;
    assert!(
        natural_height > cards.size.height,
        "the selection should overflow the card area (natural {natural_height:?}, actual {:?})",
        cards.size.height
    );

    // The cap is half the *body*, not half the window — so it holds whatever
    // height the splitter leaves the pane at.
    assert!(
        cards.size.height <= body.size.height * 0.5 + px(1.0),
        "the card area should be capped at half the body (got {:?} of {:?})",
        cards.size.height,
        body.size.height
    );
    // And the file list keeps essentially the other half. Not exactly half: the
    // subheader line and the body's two gaps come out of the body before either
    // list, so the shortfall is that chrome rather than anything the cards took.
    // A cap that stopped working would put this far below 40%.
    assert!(
        files.size.height >= body.size.height * 0.4,
        "the file list should keep essentially the other half (files {:?} of body {:?})",
        files.size.height,
        body.size.height
    );

    // Capped means virtualized: the far end of the selection is scrolled out.
    assert!(
        cx.debug_bounds("commit_multi_row_59").is_none(),
        "the last card should be out of view rather than stretching the pane"
    );

    // The point of the cap: the changed-file list still has room below.
    assert!(
        cx.debug_bounds("range_file_79_0").is_some(),
        "the changed-file list must stay visible under a large selection"
    );
}

/// A two-endpoint comparison is nowhere near the cap, so its cards size to
/// their content and leave the rest of the body to the file list.
#[gpui::test]
fn a_small_comparison_sizes_its_cards_to_content(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(83), 2, 0, Files::Loaded(3));

    let body = cx
        .debug_bounds("range_comparison_body")
        .expect("the comparison body should render");
    let cards = cx
        .debug_bounds("range_comparison_cards")
        .expect("the card area should render");
    let row = cx
        .debug_bounds("commit_multi_row_0")
        .expect("the first card should render");

    assert!(
        cards.size.height <= row.size.height * 2.0 + px(1.0),
        "two endpoints should occupy two rows, not the whole cap"
    );
    assert!(
        cards.size.height < body.size.height * 0.5,
        "a small comparison should stay well under the cap"
    );
}

/// A failed file listing must not render as "No files." — that is exactly what
/// a genuinely identical pair of commits looks like, so the user would read a
/// broken comparison as a successful, empty one.
#[gpui::test]
fn a_failed_file_listing_shows_the_error_not_an_empty_comparison(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(80), 2, 0, Files::Failed("object not found"));

    assert!(
        cx.debug_bounds("range_files_error").is_some(),
        "a failed listing should render its error"
    );
    assert!(
        cx.debug_bounds("range_files_empty").is_none(),
        "a failed listing must not be presented as an empty comparison"
    );
    // And it must not claim a count it never got.
    assert!(cx.debug_bounds("range_files_label_pending").is_some());
    assert!(cx.debug_bounds("range_files_label_count").is_none());
}

/// While the listing loads there is no count to state, so the section header
/// must not assert one — "0 changed" is both definite and usually wrong.
#[gpui::test]
fn a_loading_file_listing_does_not_claim_a_count(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(81), 2, 0, Files::Loading);

    assert!(
        cx.debug_bounds("range_files_loading").is_some(),
        "a loading listing should say so"
    );
    assert!(
        cx.debug_bounds("range_files_label_pending").is_some(),
        "the header must not state a count before there is one"
    );
    assert!(cx.debug_bounds("range_files_label_count").is_none());
    assert!(cx.debug_bounds("range_files_empty").is_none());
}

/// A genuinely empty comparison still reads as empty, and does state its count.
#[gpui::test]
fn an_empty_comparison_reads_as_empty(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(82), 2, 0, Files::Loaded(0));

    assert!(cx.debug_bounds("range_files_empty").is_some());
    assert!(cx.debug_bounds("range_files_error").is_none());
    assert!(cx.debug_bounds("range_files_label_count").is_some());
}

/// A selected worktree row shows *that* worktree's changed files, not this
/// tab's. Everything below the header belongs to a different checkout, so the
/// view has to take the pane over rather than sit alongside a commit's details.
mod worktree_uncommitted {
    use super::*;
    use gitcomet_core::domain::{FileStatus, WorktreeDirtySummary};

    fn file(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: std::path::PathBuf::from(path),
            kind,
            conflict: None,
        }
    }

    /// Draw the details pane with one dirty linked worktree, optionally selected.
    /// Counts follow the file lists, the way a completed scan reports them.
    fn draw_worktree(
        cx: &mut gpui::TestAppContext,
        repo_id: RepoId,
        staged: Vec<FileStatus>,
        unstaged: Vec<FileStatus>,
        selected: bool,
    ) -> &mut gpui::VisualTestContext {
        let summary = WorktreeDirtySummary {
            path: std::path::PathBuf::from("/tmp/linked-worktree"),
            head: Some(CommitId(sha(0).into())),
            branch: Some("side".into()),
            detached: false,
            added: unstaged.len(),
            modified: staged.len(),
            deleted: 0,
            staged,
            unstaged,
            line_stats: Default::default(),
        };
        draw_worktree_summary(cx, repo_id, summary, selected)
    }

    /// The same, for summaries the counts-and-files relationship does not hold
    /// for -- a scan that reported counts but has not yet carried the files.
    fn draw_worktree_summary(
        cx: &mut gpui::TestAppContext,
        repo_id: RepoId,
        summary: WorktreeDirtySummary,
        selected: bool,
    ) -> &mut gpui::VisualTestContext {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::super::super::GitCometView::new(store, events, None, window, cx)
        });

        let worktree_path = summary.path.clone();
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-worktree"));
                repo.open = Loadable::Ready(());
                repo.head_branch = Loadable::Ready("main".into());
                repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
                repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                    commits: (0..2).map(log_commit).collect(),
                    next_cursor: None,
                }));
                repo.log_rev = 1;
                repo.worktree_dirty = Loadable::Ready(Arc::new(vec![summary.clone()]));
                if selected {
                    repo.history_state.worktree_selection = Some(worktree_path.clone());
                }

                let next_state = app_state_with_repo(repo, repo_id);
                this.store
                    .replace_snapshot_for_test(Arc::clone(&next_state));
                push_test_state(this, next_state, cx);
            });
        });

        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx
    }

    /// `selected_ix` indexes the entry list the reducer re-derives from the
    /// scan, so it must stay a source index no matter how the rows are sorted
    /// or grouped. Passing the display row instead opens a different file, and
    /// the next scan "corrects" it to a third one.
    #[gpui::test]
    fn worktree_file_click_sends_a_source_index_under_a_reversed_sort(
        cx: &mut gpui::TestAppContext,
    ) {
        let repo_id = RepoId(97);
        let summary = WorktreeDirtySummary {
            path: std::path::PathBuf::from("/tmp/linked-worktree"),
            head: Some(CommitId(sha(0).into())),
            branch: Some("side".into()),
            detached: false,
            added: 0,
            modified: 3,
            deleted: 0,
            staged: Vec::new(),
            unstaged: vec![
                file("a.rs", FileStatusKind::Modified),
                file("b.rs", FileStatusKind::Modified),
                file("c.rs", FileStatusKind::Modified),
            ],
            line_stats: Default::default(),
        };

        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::super::super::GitCometView::new(store, events, None, window, cx)
        });
        let worktree_path = summary.path.clone();
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-worktree-sort"));
                repo.open = Loadable::Ready(());
                repo.head_branch = Loadable::Ready("main".into());
                repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
                repo.worktree_dirty = Loadable::Ready(Arc::new(vec![summary.clone()]));
                repo.history_state.worktree_selection = Some(worktree_path.clone());
                let next_state = app_state_with_repo(repo, repo_id);
                this.store
                    .replace_snapshot_for_test(Arc::clone(&next_state));
                push_test_state(this, next_state, cx);
            });
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.details_pane.update(cx, |pane, cx| {
                    pane.set_file_list_sort(
                        crate::view::rows::FileListId::WorktreeFiles,
                        crate::view::rows::CommitFileSort::PathDescending,
                        cx,
                    );
                });
            });
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        // Display order is c, b, a — so row 0 must resolve to source index 2.
        let expected = cx.update(|_window, app| {
            let pane = view.read(app).details_pane.read(app);
            let inputs = pane
                .cached_worktree_file_inputs(repo_id, 0, &summary)
                .entries
                .clone();
            let projection = pane.cached_worktree_file_projection(
                repo_id,
                0,
                &worktree_path,
                &pane.cached_worktree_file_inputs(repo_id, 0, &summary).files,
            );
            let source_ix = projection.source_indices[0];
            (source_ix, inputs[source_ix].path.clone())
        });
        assert_eq!(
            expected.1,
            std::path::PathBuf::from("c.rs"),
            "the reversed sort puts c.rs first"
        );
        assert_eq!(expected.0, 2, "and c.rs is still source index 2");
    }

    /// Three modified files, a reversed sort, and an inline diff open on one of
    /// them. `selected_ix` is a source index, so stepping to the next *drawn*
    /// file is not `selected_ix + 1`.
    fn draw_sorted_worktree_inline_diff(
        cx: &mut gpui::TestAppContext,
        repo_id: RepoId,
        selected_ix: usize,
        layout: crate::view::FileListLayout,
    ) -> (
        gpui::Entity<super::super::super::GitCometView>,
        &mut gpui::VisualTestContext,
    ) {
        let summary = WorktreeDirtySummary {
            path: std::path::PathBuf::from("/tmp/linked-worktree"),
            head: Some(CommitId(sha(0).into())),
            branch: Some("side".into()),
            detached: false,
            added: 0,
            modified: 3,
            deleted: 0,
            staged: Vec::new(),
            unstaged: vec![
                file("a.rs", FileStatusKind::Modified),
                file("b.rs", FileStatusKind::Modified),
                file("c.rs", FileStatusKind::Modified),
            ],
            line_stats: Default::default(),
        };

        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::super::super::GitCometView::new(store, events, None, window, cx)
        });
        let worktree_path = summary.path.clone();
        let entries: Arc<[_]> =
            gitcomet_state::model::worktree_inline_diff_entries(&summary).into();
        let target = entries[selected_ix].target.clone();
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-worktree-nav"));
                repo.open = Loadable::Ready(());
                repo.head_branch = Loadable::Ready("main".into());
                repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
                repo.worktree_dirty = Loadable::Ready(Arc::new(vec![summary.clone()]));
                repo.history_state.worktree_selection = Some(worktree_path.clone());
                repo.diff_state.inline_submodule_diff =
                    Some(gitcomet_state::model::InlineSubmoduleDiffState {
                        origin: gitcomet_state::model::ForeignDiffOrigin::Worktree {
                            branch: Some("side".into()),
                            detached: false,
                        },
                        submodule_repo_path: worktree_path.clone(),
                        parent_submodule_path: worktree_path.clone(),
                        entries: Arc::clone(&entries),
                        selected_ix,
                        target,
                        rev: 1,
                        diff_rev: 1,
                        diff: Loadable::NotLoaded,
                        diff_file_rev: 1,
                        diff_file: Loadable::NotLoaded,
                        diff_file_image: Loadable::NotLoaded,
                    });
                let next_state = app_state_with_repo(repo, repo_id);
                this.store
                    .replace_snapshot_for_test(Arc::clone(&next_state));
                push_test_state(this, next_state, cx);
            });
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.details_pane.update(cx, |pane, cx| {
                    pane.set_file_list_sort(
                        crate::view::rows::FileListId::WorktreeFiles,
                        crate::view::rows::CommitFileSort::PathDescending,
                        cx,
                    );
                    pane.set_file_list_layout(layout, cx);
                });
            });
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        (view, cx)
    }

    fn navigate_worktree_inline_diff(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::super::GitCometView>,
        repo_id: RepoId,
        direction: i8,
    ) -> bool {
        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            main_pane.update(app, |pane, cx| {
                pane.try_select_adjacent_diff_file(repo_id, direction, window, cx)
            })
        })
    }

    /// The path the store settled on, once its worker has reduced the dispatch.
    fn selected_worktree_inline_path(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::super::GitCometView>,
        repo_id: RepoId,
    ) -> Option<std::path::PathBuf> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let selected = cx.update(|_window, app| {
                let snapshot = view.read(app).store.snapshot();
                let repo = snapshot.repos.iter().find(|repo| repo.id == repo_id)?;
                let inline = repo.diff_state.inline_submodule_diff.as_ref()?;
                Some(inline.entries[inline.selected_ix].path.clone())
            });
            match &selected {
                Some(path) if path != std::path::Path::new("c.rs") => return selected,
                _ if std::time::Instant::now() >= deadline => return selected,
                _ => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
    }

    /// Rows draw c, b, a under a reversed sort, so next-file from c.rs is b.rs.
    /// Walking source order instead either dead-ends or opens a file the user
    /// never saw next to the one on screen.
    #[gpui::test]
    fn worktree_file_navigation_follows_the_sorted_order(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(98);
        // c.rs is source index 2 and the first drawn row.
        let (view, cx) =
            draw_sorted_worktree_inline_diff(cx, repo_id, 2, crate::view::FileListLayout::Flat);

        assert!(
            navigate_worktree_inline_diff(cx, &view, repo_id, 1),
            "next-file from the first drawn row should move"
        );
        assert_eq!(
            selected_worktree_inline_path(cx, &view, repo_id),
            Some(std::path::PathBuf::from("b.rs")),
            "next-file must open the row drawn below, not the next source index"
        );
    }

    /// And the last drawn row has no next file, even though its source index does.
    #[gpui::test]
    fn worktree_file_navigation_stops_at_the_last_sorted_row(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(99);
        // a.rs is source index 0 and the last drawn row.
        let (view, cx) =
            draw_sorted_worktree_inline_diff(cx, repo_id, 0, crate::view::FileListLayout::Flat);

        assert!(
            !navigate_worktree_inline_diff(cx, &view, repo_id, 1),
            "next-file from the last drawn row must not jump back up the list"
        );
    }

    /// Tree layout groups the files under folder rows, so drawn order comes from
    /// the plan rather than the projection. Files at the root of a reversed tree
    /// still draw c, b, a.
    #[gpui::test]
    fn worktree_file_navigation_follows_tree_order(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(100);
        let (view, cx) =
            draw_sorted_worktree_inline_diff(cx, repo_id, 2, crate::view::FileListLayout::Tree);

        assert!(
            navigate_worktree_inline_diff(cx, &view, repo_id, 1),
            "next-file from the first drawn row should move in tree layout too"
        );
        assert_eq!(
            selected_worktree_inline_path(cx, &view, repo_id),
            Some(std::path::PathBuf::from("b.rs")),
            "tree order must drive next-file, not the entry order"
        );
    }

    #[gpui::test]
    fn a_selected_worktree_row_takes_over_the_details_pane(cx: &mut gpui::TestAppContext) {
        let cx = draw_worktree(
            cx,
            RepoId(90),
            vec![file("gone.txt", FileStatusKind::Deleted)],
            vec![file("edited.rs", FileStatusKind::Modified)],
            true,
        );

        assert!(
            cx.debug_bounds("worktree_uncommitted_body").is_some(),
            "a selected worktree row should render its own view"
        );
        assert!(
            cx.debug_bounds("worktree_file_90_0").is_some(),
            "the worktree's first changed file should render"
        );
        assert!(
            cx.debug_bounds("worktree_file_90_1").is_some(),
            "both staged and unstaged files belong in the list"
        );
        assert!(
            cx.debug_bounds("worktree_files_loading").is_none(),
            "a worktree whose files have arrived must not read as loading"
        );
        assert!(
            cx.debug_bounds("worktree_uncommitted_open").is_some(),
            "the panel should offer to open the worktree it is showing"
        );
    }

    /// Untracked files are the common case in a worktree list and the one the
    /// shared commit-file table did not expect, so they get their own row here.
    #[gpui::test]
    fn untracked_files_get_rows_like_any_other(cx: &mut gpui::TestAppContext) {
        let cx = draw_worktree(
            cx,
            RepoId(93),
            Vec::new(),
            vec![
                file("new_one.rs", FileStatusKind::Untracked),
                file("new_two.rs", FileStatusKind::Untracked),
            ],
            true,
        );

        assert!(cx.debug_bounds("worktree_file_93_0").is_some());
        assert!(cx.debug_bounds("worktree_file_93_1").is_some());
        assert!(cx.debug_bounds("worktree_files_loading").is_none());
    }

    /// Without a selection the pane stays on this tab's own content, however
    /// dirty the other worktrees are.
    #[gpui::test]
    fn an_unselected_worktree_leaves_the_pane_alone(cx: &mut gpui::TestAppContext) {
        let cx = draw_worktree(
            cx,
            RepoId(91),
            Vec::new(),
            vec![file("edited.rs", FileStatusKind::Modified)],
            false,
        );

        assert!(
            cx.debug_bounds("worktree_uncommitted_body").is_none(),
            "the worktree view must not appear until its row is selected"
        );
    }

    /// Only the selected worktree's file lists are carried in state, so between
    /// selecting a row and its scan landing the summary has counts but no files.
    /// That must read as "loading", not as an empty worktree -- the header right
    /// above it is counting changes the list would be claiming do not exist.
    #[gpui::test]
    fn a_worktree_whose_files_have_not_arrived_reads_as_loading(cx: &mut gpui::TestAppContext) {
        let cx = draw_worktree_summary(
            cx,
            RepoId(94),
            WorktreeDirtySummary {
                path: std::path::PathBuf::from("/tmp/linked-worktree"),
                head: Some(CommitId(sha(0).into())),
                branch: Some("side".into()),
                detached: false,
                added: 2,
                modified: 1,
                deleted: 0,
                staged: Vec::new(),
                unstaged: Vec::new(),
                line_stats: Default::default(),
            },
            true,
        );

        assert!(
            cx.debug_bounds("worktree_uncommitted_body").is_some(),
            "the worktree view still owns the pane while its files load"
        );
        assert!(
            cx.debug_bounds("worktree_files_loading").is_some(),
            "counts without files must read as loading"
        );
        assert!(
            cx.debug_bounds("worktree_file_94_0").is_none(),
            "no rows until the files arrive"
        );
    }

    /// A worktree with no files at all is not a state the scan can produce -- it
    /// only reports worktrees `is_dirty` holds for -- but the pane must still be
    /// the worktree's own rather than falling through to the commit views.
    #[gpui::test]
    fn a_worktree_with_no_files_still_owns_the_pane(cx: &mut gpui::TestAppContext) {
        let cx = draw_worktree(cx, RepoId(92), Vec::new(), Vec::new(), true);

        assert!(cx.debug_bounds("worktree_uncommitted_body").is_some());
    }
}
