use super::*;

fn pb(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(path)
}

fn repo_state(id: RepoId, path: &str) -> RepoState {
    RepoState::new_opening(id, gitcomet_core::domain::RepoSpec { workdir: pb(path) })
}

fn file_status(
    path: &str,
    kind: gitcomet_core::domain::FileStatusKind,
) -> gitcomet_core::domain::FileStatus {
    gitcomet_core::domain::FileStatus {
        path: pb(path),
        kind,
        conflict: None,
    }
}

#[test]
fn step_ix_clamps_and_reports_no_move() {
    assert_eq!(step_ix(2, 5, 1), Some(3));
    assert_eq!(step_ix(2, 5, -1), Some(1));
    assert_eq!(step_ix(0, 5, -1), None);
    assert_eq!(step_ix(4, 5, 1), None);
    assert_eq!(step_ix(1, 10, 4), Some(5));
    assert_eq!(step_ix(7, 10, 4), Some(9));
    assert_eq!(step_ix(7, 10, -40), Some(0));
    assert_eq!(step_ix(3, 5, isize::MIN), Some(0));
    assert_eq!(step_ix(3, 5, isize::MAX), Some(4));
    assert_eq!(step_ix(0, 0, 1), None);
    assert_eq!(step_ix(2, 5, 0), None);
}

/// The bar's one-pass place must agree with what the prev/next actions
/// then do, for every entry, section order and change-tracking view: an
/// arrow is enabled exactly when it has a file to step to.
#[test]
fn status_navigation_position_matches_adjacent_targets() {
    use gitcomet_core::domain::FileStatusKind::{Added, Modified, Untracked};
    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    let unstaged = vec![
        file_status("a.txt", Modified),
        file_status("b.txt", Untracked),
        file_status("c.txt", Modified),
        file_status("d.txt", Untracked),
        file_status("e.txt", Modified),
    ];
    let staged = vec![
        file_status("s1.txt", Added),
        file_status("s2.txt", Modified),
    ];
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(staged.clone()),
            unstaged: std::sync::Arc::new(unstaged.clone()),
        }
        .into(),
    );
    let orders: [Option<Vec<usize>>; 4] = [
        None,
        Some(vec![4, 2, 0]),
        Some(vec![3, 1]),
        Some(vec![1, 9, 0]),
    ];
    for view in [
        ChangeTrackingView::Combined,
        ChangeTrackingView::SplitUntracked,
    ] {
        for (paths, area) in [(&unstaged, DiffArea::Unstaged), (&staged, DiffArea::Staged)] {
            for entry in paths {
                let target = DiffTarget::working_tree(entry.path.clone(), area);
                for order in &orders {
                    let order = order.as_deref();
                    let expected = (
                        adjacent_diff_file_target_for_repo(&repo, &target, view, -1, None, order)
                            .is_some(),
                        adjacent_diff_file_target_for_repo(&repo, &target, view, 1, None, order)
                            .is_some(),
                    );
                    assert_eq!(
                        diff_file_position_for_repo(&repo, &target, view, None, order)
                            .map(|place| (place.index > 0, place.index + 1 < place.count))
                            .unwrap_or((false, false)),
                        expected,
                        "{view:?} {target:?} {order:?}"
                    );
                }
            }
        }
    }
}

/// Once the open file's last change is staged or discarded it leaves its
/// list; F1/F4 then open the list's first file (End its last) instead of
/// doing nothing.
#[test]
fn a_step_from_a_file_that_left_its_list_opens_the_first_file() {
    use gitcomet_core::domain::FileStatusKind::{Added, Modified, Untracked};
    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![
                file_status("s1.txt", Added),
                file_status("s2.txt", Modified),
            ]),
            unstaged: std::sync::Arc::new(vec![
                file_status("a.txt", Untracked),
                file_status("b.txt", Modified),
                file_status("c.txt", Modified),
            ]),
        }
        .into(),
    );
    let opened = |target: Option<AdjacentDiffFileTarget>| match target {
        Some(AdjacentDiffFileTarget::WorkingTree { path, .. }) => Some(path),
        _ => None,
    };
    for (view, area, order, first, last) in [
        (
            ChangeTrackingView::Combined,
            DiffArea::Unstaged,
            None,
            "a.txt",
            "c.txt",
        ),
        (
            ChangeTrackingView::Combined,
            DiffArea::Unstaged,
            Some(&[2, 0, 1][..]),
            "c.txt",
            "b.txt",
        ),
        // The split view tries the tracked list first.
        (
            ChangeTrackingView::SplitUntracked,
            DiffArea::Unstaged,
            None,
            "b.txt",
            "c.txt",
        ),
        (
            ChangeTrackingView::Combined,
            DiffArea::Staged,
            None,
            "s1.txt",
            "s2.txt",
        ),
    ] {
        let gone = DiffTarget::working_tree("gone.txt".into(), area);
        let case = format!("{view:?} {area:?} {order:?}");
        for delta in [-1, 1, isize::MIN] {
            assert_eq!(
                opened(adjacent_diff_file_target_for_repo(
                    &repo, &gone, view, delta, None, order
                )),
                Some(first.into()),
                "{case} {delta}"
            );
        }
        assert_eq!(
            opened(adjacent_diff_file_target_for_repo(
                &repo,
                &gone,
                view,
                isize::MAX,
                None,
                order
            )),
            Some(last.into()),
            "{case} End"
        );
    }
}

/// The bar's "N of M" counts the section the file is in, in the order the
/// section is drawn, and names it.
#[test]
fn a_working_tree_file_is_placed_in_its_section() {
    use gitcomet_core::domain::FileStatusKind::{Added, Modified, Untracked};
    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![
                file_status("s1.txt", Added),
                file_status("s2.txt", Modified),
            ]),
            unstaged: std::sync::Arc::new(vec![
                file_status("a.txt", Modified),
                file_status("b.txt", Untracked),
                file_status("c.txt", Modified),
                file_status("d.txt", Untracked),
                file_status("e.txt", Modified),
            ]),
        }
        .into(),
    );
    let place = |path: &str, area, view, order: Option<&[usize]>| {
        diff_file_position_for_repo(
            &repo,
            &DiffTarget::working_tree(pb(path), area),
            view,
            None,
            order,
        )
        .map(|place| (place.index, place.count, place.context.to_string()))
    };
    use ChangeTrackingView::{Combined, SplitUntracked};
    assert_eq!(
        place("s2.txt", DiffArea::Staged, Combined, None),
        Some((1, 2, "Staged changes".into()))
    );
    assert_eq!(
        place("c.txt", DiffArea::Unstaged, Combined, None),
        Some((2, 5, "Changes".into()))
    );
    assert_eq!(
        place("c.txt", DiffArea::Unstaged, SplitUntracked, None),
        Some((1, 3, "Unstaged changes".into()))
    );
    assert_eq!(
        place("d.txt", DiffArea::Unstaged, SplitUntracked, None),
        Some((1, 2, "Untracked files".into()))
    );
    // A sorted section: drawn e, c, a.
    assert_eq!(
        place("c.txt", DiffArea::Unstaged, Combined, Some(&[4, 2, 0])),
        Some((1, 3, "Changes".into()))
    );
    assert_eq!(place("zz.txt", DiffArea::Unstaged, Combined, None), None);
}

#[test]
fn a_file_is_placed_in_a_list_as_drawn() {
    let files: Vec<gitcomet_core::domain::CommitFileChange> = ["a.rs", "b.rs", "c.rs"]
        .into_iter()
        .map(|path| {
            gitcomet_core::domain::CommitFileChange::new(
                pb(path),
                gitcomet_core::domain::FileStatusKind::Modified,
            )
        })
        .collect();
    assert_eq!(
        file_position_in_list(&files, None, &pb("b.rs")),
        Some((1, 3))
    );
    assert_eq!(
        file_position_in_list(&files, Some(&[2, 1, 0]), &pb("c.rs")),
        Some((0, 3))
    );
    // A filter that hides the file: no place.
    assert_eq!(
        file_position_in_list(&files, Some(&[0, 2]), &pb("b.rs")),
        None
    );
    assert_eq!(inline_diff_file_position(1, 3, None), Some((1, 3)));
    assert_eq!(inline_diff_file_position(1, 3, Some(&[2, 1])), Some((1, 2)));
    assert_eq!(inline_diff_file_position(5, 3, None), None);
}

#[test]
fn split_untracked_navigation_scopes_to_untracked_section() {
    let status = gitcomet_core::domain::RepoStatus {
        staged: std::sync::Arc::new(Vec::new()),
        unstaged: std::sync::Arc::new(vec![
            file_status(
                "new-a.txt",
                gitcomet_core::domain::FileStatusKind::Untracked,
            ),
            file_status(
                "src/lib.rs",
                gitcomet_core::domain::FileStatusKind::Modified,
            ),
            file_status(
                "new-b.txt",
                gitcomet_core::domain::FileStatusKind::Untracked,
            ),
        ]),
    };
    let target = DiffTarget::working_tree(pb("new-a.txt"), DiffArea::Unstaged);

    let navigation =
        status_navigation_context(&status, &target, ChangeTrackingView::SplitUntracked)
            .expect("split untracked navigation");

    assert_eq!(navigation.section, StatusSection::Untracked);
    assert_eq!(navigation.current_ix, 0);
    assert_eq!(
        navigation
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        vec![pb("new-a.txt"), pb("new-b.txt")]
    );
    assert_eq!(navigation.next_or_prev_path(), Some(pb("new-b.txt")));
}

#[test]
fn split_tracked_navigation_scopes_to_tracked_section() {
    let status = gitcomet_core::domain::RepoStatus {
        staged: std::sync::Arc::new(Vec::new()),
        unstaged: std::sync::Arc::new(vec![
            file_status(
                "new-a.txt",
                gitcomet_core::domain::FileStatusKind::Untracked,
            ),
            file_status(
                "src/lib.rs",
                gitcomet_core::domain::FileStatusKind::Modified,
            ),
            file_status(
                "src/main.rs",
                gitcomet_core::domain::FileStatusKind::Modified,
            ),
        ]),
    };
    let target = DiffTarget::working_tree(pb("src/lib.rs"), DiffArea::Unstaged);

    let navigation =
        status_navigation_context(&status, &target, ChangeTrackingView::SplitUntracked)
            .expect("split tracked navigation");

    assert_eq!(navigation.section, StatusSection::Unstaged);
    assert_eq!(navigation.current_ix, 0);
    assert_eq!(navigation.prev_ix(), None);
    assert_eq!(navigation.next_ix(), Some(1));
    assert_eq!(
        navigation
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        vec![pb("src/lib.rs"), pb("src/main.rs")]
    );
}

#[test]
fn combined_navigation_keeps_untracked_and_tracked_together() {
    let status = gitcomet_core::domain::RepoStatus {
        staged: std::sync::Arc::new(Vec::new()),
        unstaged: std::sync::Arc::new(vec![
            file_status(
                "new-a.txt",
                gitcomet_core::domain::FileStatusKind::Untracked,
            ),
            file_status(
                "src/lib.rs",
                gitcomet_core::domain::FileStatusKind::Modified,
            ),
            file_status(
                "new-b.txt",
                gitcomet_core::domain::FileStatusKind::Untracked,
            ),
        ]),
    };
    let target = DiffTarget::working_tree(pb("src/lib.rs"), DiffArea::Unstaged);

    let navigation = status_navigation_context(&status, &target, ChangeTrackingView::Combined)
        .expect("combined navigation");

    assert_eq!(navigation.section, StatusSection::CombinedUnstaged);
    assert_eq!(navigation.current_ix, 1);
    assert_eq!(navigation.prev_ix(), Some(0));
    assert_eq!(navigation.next_ix(), Some(2));
}

#[test]
fn commit_details_file_navigation_selects_adjacent_commit_files() {
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let file_a = pb("src/a.rs");
    let file_b = pb("src/b.rs");
    let file_c = pb("src/c.rs");

    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.history_state.commit_details =
        Loadable::Ready(std::sync::Arc::new(gitcomet_core::domain::CommitDetails {
            id: commit_id.clone(),
            message: "subject".into(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "2026-04-14 12:00:00 +0300".into(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files: vec![
                gitcomet_core::domain::CommitFileChange::new(
                    file_a.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                gitcomet_core::domain::CommitFileChange::new(
                    file_b.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                gitcomet_core::domain::CommitFileChange::new(
                    file_c.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
            ],
        }));

    let target = DiffTarget::commit(commit_id.clone(), file_b.clone());

    assert_eq!(
        adjacent_diff_file_target_for_repo(
            &repo,
            &target,
            ChangeTrackingView::Combined,
            -1,
            None,
            None,
        ),
        Some(AdjacentDiffFileTarget::Commit {
            commit_id: commit_id.clone(),
            target_ix: 0,
            path: file_a,
            old_path: None,
        })
    );
    assert_eq!(
        adjacent_diff_file_target_for_repo(
            &repo,
            &target,
            ChangeTrackingView::Combined,
            1,
            None,
            None,
        ),
        Some(AdjacentDiffFileTarget::Commit {
            commit_id,
            target_ix: 2,
            path: file_c,
            old_path: None,
        })
    );
}

#[test]
fn commit_details_file_navigation_uses_the_visible_sorted_projection() {
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let file_a = pb("src/a.rs");
    let file_b = pb("src/b.rs");
    let file_c = pb("src/c.rs");

    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.history_state.commit_details =
        Loadable::Ready(std::sync::Arc::new(gitcomet_core::domain::CommitDetails {
            id: commit_id.clone(),
            message: "subject".into(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "2026-04-14 12:00:00 +0300".into(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files: vec![
                gitcomet_core::domain::CommitFileChange::new(
                    file_a.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                gitcomet_core::domain::CommitFileChange::new(
                    file_b.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                gitcomet_core::domain::CommitFileChange::new(
                    file_c.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
            ],
        }));

    let target = DiffTarget::commit(commit_id.clone(), file_b.clone());
    let visible_source_indices = [2, 1];

    assert_eq!(
        adjacent_diff_file_target_for_repo(
            &repo,
            &target,
            ChangeTrackingView::Combined,
            -1,
            Some(&visible_source_indices),
            None,
        ),
        Some(AdjacentDiffFileTarget::Commit {
            commit_id: commit_id.clone(),
            target_ix: 0,
            path: file_c,
            old_path: None,
        })
    );
    assert_eq!(
        adjacent_diff_file_target_for_repo(
            &repo,
            &target,
            ChangeTrackingView::Combined,
            1,
            Some(&visible_source_indices),
            None,
        ),
        None,
    );

    let hidden_target = DiffTarget::commit(commit_id, file_a);
    assert_eq!(
        adjacent_diff_file_target_for_repo(
            &repo,
            &hidden_target,
            ChangeTrackingView::Combined,
            1,
            Some(&visible_source_indices),
            None,
        ),
        None,
        "navigation is a no-op while the open diff is hidden by the filter"
    );
}

fn file_change(path: &std::path::Path) -> gitcomet_core::domain::CommitFileChange {
    gitcomet_core::domain::CommitFileChange::new(
        path.to_path_buf(),
        gitcomet_core::domain::FileStatusKind::Modified,
    )
}

#[test]
fn comparison_file_navigation_steps_through_the_drawn_range_files() {
    let from = CommitId("1111111111111111".into());
    let to = CommitId("2222222222222222".into());
    let file_a = pb("src/a.rs");
    let file_b = pb("src/b.rs");
    let file_c = pb("src/c.rs");

    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection::new(
        from.clone(),
        Some(to.clone()),
        "from".into(),
        "to".into(),
    ));
    repo.history_state.range_files = Loadable::Ready(std::sync::Arc::new(vec![
        file_change(&file_a),
        file_change(&file_b),
        file_change(&file_c),
    ]));
    let target = DiffTarget::commit_range(from.clone(), Some(to.clone()), Some(file_b.clone()));
    let range_file = |path: &std::path::PathBuf, target_ix| AdjacentDiffFileTarget::Range {
        target: DiffTarget::commit_range(from.clone(), Some(to.clone()), Some(path.clone())),
        target_ix,
    };
    let adjacent = |repo: &RepoState, direction, drawn: Option<&[usize]>| {
        adjacent_diff_file_target_for_repo(
            repo,
            &target,
            ChangeTrackingView::Combined,
            direction,
            drawn,
            None,
        )
    };

    assert_eq!(adjacent(&repo, -1, None), Some(range_file(&file_a, 0)));
    assert_eq!(adjacent(&repo, 1, None), Some(range_file(&file_c, 2)));
    // A sorted list steps in display order, and its ends have no neighbour.
    assert_eq!(
        adjacent(&repo, -1, Some(&[2, 1, 0])),
        Some(range_file(&file_c, 0))
    );
    assert_eq!(adjacent(&repo, -1, Some(&[1, 0, 2])), None);

    // A diff left open from another comparison has no list on screen.
    repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection::new(
        from,
        None,
        "from".into(),
        "Working tree".into(),
    ));
    assert_eq!(adjacent(&repo, 1, None), None);
}
