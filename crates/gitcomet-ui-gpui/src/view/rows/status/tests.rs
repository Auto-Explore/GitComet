use super::*;

fn pb(s: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(s)
}

/// Status entries arrive byte-sorted from the backend, where `Zoo.rs` beats
/// `apple.rs`. The UI sorts case-insensitively instead, the same way the
/// committed-file list always has.
#[test]
fn status_sort_is_case_insensitive_in_both_directions() {
    let entries = vec![
        file_status("Zoo.rs", FileStatusKind::Modified),
        file_status("apple.rs", FileStatusKind::Modified),
        file_status("Banana.rs", FileStatusKind::Modified),
    ];
    let all: Vec<usize> = (0..entries.len()).collect();

    let ascending = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::PathAscending,
        None,
    );
    assert_eq!(ascending.as_ref(), &[1, 2, 0], "apple, Banana, Zoo");

    let descending = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::PathDescending,
        None,
    );
    assert_eq!(descending.as_ref(), &[0, 2, 1], "Zoo, Banana, apple");
}

/// The status sections carry their own copy of the comparator, so the
/// file-type grouping has to be proved here too, not just on the committed
/// list.
#[test]
fn status_file_type_sort_groups_by_extension() {
    let entries = vec![
        file_status("src/view.ts", FileStatusKind::Modified),
        file_status("Makefile", FileStatusKind::Untracked),
        file_status("src/main.rs", FileStatusKind::Modified),
        file_status("Cargo.toml", FileStatusKind::Modified),
        file_status("src/app.ts", FileStatusKind::Modified),
    ];
    let all: Vec<usize> = (0..entries.len()).collect();

    let ascending = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::FileTypeAscending,
        None,
    );
    assert_eq!(
        ascending.as_ref(),
        &[1, 2, 3, 4, 0],
        "Makefile (no extension), rs, toml, then ts A-Z",
    );

    let descending = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::FileTypeDescending,
        None,
    );
    assert_eq!(
        descending.as_ref(),
        &[4, 0, 3, 2, 1],
        "groups reverse, paths inside a group stay A-Z",
    );
}

/// Status entries have no edit counts, so the edit-size modes have nothing
/// to order by and must not shuffle the list into an arbitrary order.
#[test]
fn edit_size_sorts_fall_back_to_path_order_for_status_entries() {
    let entries = vec![
        file_status("b.rs", FileStatusKind::Modified),
        file_status("a.rs", FileStatusKind::Modified),
    ];
    let all: Vec<usize> = (0..entries.len()).collect();
    for sort in [
        crate::view::rows::CommitFileSort::EditSizeAscending,
        crate::view::rows::CommitFileSort::EditSizeDescending,
    ] {
        let ordered = crate::view::rows::status_section_sorted_indexes(&entries, &all, sort, None);
        assert_eq!(ordered.as_ref(), &[1, 0], "{sort:?} falls back to path A-Z");
    }
}

/// Mirrors the committed-file list: unknown sizes last, path breaks ties.
#[test]
fn edit_size_sort_orders_by_size_with_unknowns_last() {
    use gitcomet_core::domain::LineStats;

    let entries = vec![
        file_status("small.rs", FileStatusKind::Modified),
        file_status("big.rs", FileStatusKind::Modified),
        file_status("unknown.rs", FileStatusKind::Modified),
    ];
    let all: Vec<usize> = (0..entries.len()).collect();
    let mut stats = rustc_hash::FxHashMap::default();
    stats.insert(pb("small.rs"), LineStats::from((Some(1), Some(1))));
    stats.insert(pb("big.rs"), LineStats::from((Some(90), Some(10))));
    // `unknown.rs` is absent on purpose — a binary file, say.

    let largest = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::EditSizeDescending,
        Some(&stats),
    );
    assert_eq!(largest.as_ref(), &[1, 0, 2], "big, small, then unknown");

    let smallest = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::EditSizeAscending,
        Some(&stats),
    );
    assert_eq!(
        smallest.as_ref(),
        &[0, 1, 2],
        "unknowns stay last in both directions"
    );
}

/// A section's lane carries the edits; files sharing one sit together.
#[test]
fn edits_sort_groups_a_sections_repeated_edits() {
    use crate::view::rows::tests::edit;
    use gitcomet_core::domain::LineStats;

    let entries = vec![
        file_status("c.rs", FileStatusKind::Modified),
        file_status("b.rs", FileStatusKind::Modified),
        file_status("a.rs", FileStatusKind::Modified),
        file_status("d.rs", FileStatusKind::Modified),
    ];
    let all: Vec<usize> = (0..entries.len()).collect();
    let stats: rustc_hash::FxHashMap<_, _> = [("a.rs", "x"), ("b.rs", "y"), ("d.rs", "y")]
        .into_iter()
        .map(|(path, text)| {
            let mut stats = LineStats::from((Some(1), Some(0)));
            stats.edit = edit(text);
            (pb(path), stats)
        })
        .collect();

    let ordered = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::Edits,
        Some(&stats),
    );
    assert_eq!(
        ordered.as_ref(),
        &[1, 3, 2, 0],
        "b+d, then a, then c (unknown)"
    );
}

/// With nothing to order by the list must stay in a stable path order.
#[test]
fn edit_size_sort_falls_back_to_path_order_without_stats() {
    let entries = vec![
        file_status("b.rs", FileStatusKind::Modified),
        file_status("a.rs", FileStatusKind::Modified),
    ];
    let all: Vec<usize> = (0..entries.len()).collect();
    let ordered = crate::view::rows::status_section_sorted_indexes(
        &entries,
        &all,
        crate::view::rows::CommitFileSort::EditSizeDescending,
        None,
    );
    assert_eq!(ordered.as_ref(), &[1, 0]);
}

fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: pb(path),
        kind,
        conflict: None,
    }
}

fn repo_with_status_entries(entries: Vec<FileStatus>) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: pb("/tmp/status-section-entries-test"),
        },
    );
    repo.worktree_status = Loadable::Ready(Arc::new(entries));
    repo.worktree_status_rev = 1;
    repo
}

fn commit_id(id: &str) -> gitcomet_core::domain::CommitId {
    gitcomet_core::domain::CommitId(Arc::from(id))
}

#[test]
fn status_section_entries_index_filtered_sections_directly() {
    let repo = repo_with_status_entries(vec![
        file_status("tracked-a.txt", FileStatusKind::Modified),
        file_status("new-a.txt", FileStatusKind::Untracked),
        file_status("tracked-b.txt", FileStatusKind::Deleted),
        file_status("new-b.txt", FileStatusKind::Untracked),
    ]);

    let untracked = StatusSectionEntries::from_repo(&repo, StatusSection::Untracked).unwrap();
    assert_eq!(untracked.len(), 2);
    assert_eq!(untracked.get(0).unwrap().path, pb("new-a.txt"));
    assert_eq!(untracked.get(1).unwrap().path, pb("new-b.txt"));
    assert!(untracked.get(2).is_none());

    let unstaged = StatusSectionEntries::from_repo(&repo, StatusSection::Unstaged).unwrap();
    assert_eq!(unstaged.len(), 2);
    assert_eq!(unstaged.get(0).unwrap().path, pb("tracked-a.txt"));
    assert_eq!(unstaged.get(1).unwrap().path, pb("tracked-b.txt"));
    assert!(unstaged.get(2).is_none());
}

#[test]
fn submodule_status_lookup_maps_ready_submodules_by_path() {
    let mut repo = repo_with_status_entries(Vec::new());
    repo.submodules = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::Submodule {
            path: pb("vendor/ready"),
            recorded_head: commit_id("1111111"),
            checked_out_head: Some(commit_id("2222222")),
            status: SubmoduleStatus::HeadMismatch,
        },
        gitcomet_core::domain::Submodule {
            path: pb("vendor/missing"),
            recorded_head: commit_id("3333333"),
            checked_out_head: None,
            status: SubmoduleStatus::NotInitialized,
        },
    ]));

    let lookup = submodule_status_lookup(&repo);
    assert_eq!(
        lookup.get(std::path::Path::new("vendor/ready")).copied(),
        Some(SubmoduleStatus::HeadMismatch)
    );
    assert_eq!(
        lookup.get(std::path::Path::new("vendor/missing")).copied(),
        Some(SubmoduleStatus::NotInitialized)
    );
    assert_eq!(lookup.get(std::path::Path::new("vendor/other")), None);
}

#[test]
fn status_select_all_preserves_the_anchor_and_supports_range_replacement() {
    for section in [
        StatusSection::CombinedUnstaged,
        StatusSection::Untracked,
        StatusSection::Unstaged,
        StatusSection::Staged,
    ] {
        let mut selection = StatusMultiSelection::default();
        apply_status_multi_selection_click(
            &mut selection,
            section,
            pb("b"),
            Some(1),
            gpui::Modifiers::default(),
            Some(1),
            true,
            None,
        );
        let order = vec![pb("d"), pb("c"), pb("b"), pb("a")];
        selection.select_all(section, order.clone(), 2);
        assert_eq!(
            selection.selected_paths_for_area(section.diff_area()),
            &order
        );
        apply_status_multi_selection_click(
            &mut selection,
            section,
            pb("d"),
            Some(0),
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            Some(2),
            true,
            Some(&order),
        );
        assert_eq!(
            selection.selected_paths_for_area(section.diff_area()),
            &order[..3]
        );
        for (ix, path) in order[..3].iter().enumerate() {
            apply_status_multi_selection_click(
                &mut selection,
                section,
                path.clone(),
                Some(ix),
                gpui::Modifiers {
                    control: true,
                    ..Default::default()
                },
                Some(2),
                true,
                None,
            );
        }
        assert!(selection.is_empty());
        assert_eq!(selection.explicit_section, Some(section));
    }
}

#[test]
fn status_select_all_switches_sections_and_uses_the_first_file_without_an_anchor() {
    let mut selection = StatusMultiSelection::default();
    selection.select_all(StatusSection::Untracked, vec![pb("new")], 1);
    selection.select_all(StatusSection::Staged, vec![pb("z"), pb("a")], 2);
    assert!(selection.untracked.is_empty());
    assert!(selection.untracked_anchor.is_none());
    assert_eq!(selection.staged_anchor, Some(pb("z")));
    assert_eq!(selection.staged_anchor_index, Some(0));
    assert_eq!(selection.explicit_section, Some(StatusSection::Staged));
}

#[test]
fn status_selection_ctrl_click_toggles() {
    let mut sel = StatusMultiSelection::default();
    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("a"),
        None,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        None,
        false,
        None,
    );
    assert_eq!(sel.unstaged, vec![pb("a")]);

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("b"),
        None,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        None,
        false,
        None,
    );
    assert_eq!(sel.unstaged, vec![pb("a"), pb("b")]);

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("a"),
        None,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        None,
        false,
        None,
    );
    assert_eq!(sel.unstaged, vec![pb("b")]);
}

#[test]
fn status_selection_shift_click_selects_range() {
    let mut sel = StatusMultiSelection::default();
    let entries = vec![pb("a"), pb("b"), pb("c"), pb("d")];

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("b"),
        None,
        gpui::Modifiers::default(),
        Some(1),
        false,
        Some(&entries),
    );
    assert_eq!(sel.unstaged, vec![pb("b")]);

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("d"),
        None,
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        Some(1),
        false,
        Some(&entries),
    );
    assert_eq!(sel.unstaged, vec![pb("b"), pb("c"), pb("d")]);
}

#[test]
fn split_untracked_selection_clears_tracked_selection() {
    let mut sel = StatusMultiSelection {
        unstaged: vec![pb("tracked.txt")],
        unstaged_anchor: Some(pb("tracked.txt")),
        ..Default::default()
    };

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::Untracked,
        pb("new.txt"),
        None,
        gpui::Modifiers::default(),
        None,
        false,
        None,
    );

    assert!(sel.unstaged.is_empty());
    assert_eq!(sel.untracked, vec![pb("new.txt")]);
}

#[test]
fn status_selection_shift_click_uses_index_hints_without_scanning() {
    reset_status_selection_scan_steps();

    let mut sel = StatusMultiSelection::default();
    let entries = vec![pb("a"), pb("b"), pb("c"), pb("d")];

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("b"),
        Some(1),
        gpui::Modifiers::default(),
        Some(1),
        true,
        Some(&entries),
    );
    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("d"),
        Some(3),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        Some(1),
        true,
        Some(&entries),
    );

    assert_eq!(sel.unstaged, vec![pb("b"), pb("c"), pb("d")]);
    assert_eq!(sel.unstaged_anchor, Some(pb("b")));
    assert_eq!(sel.unstaged_anchor_index, Some(1));
    assert_eq!(sel.unstaged_anchor_order_rev, Some(1));
    assert_eq!(status_selection_scan_steps(), 0);
}

#[test]
fn status_selection_shift_click_falls_back_when_index_hint_is_stale() {
    reset_status_selection_scan_steps();

    let mut sel = StatusMultiSelection::default();
    let entries = vec![pb("a"), pb("b"), pb("c"), pb("d")];

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("b"),
        Some(1),
        gpui::Modifiers::default(),
        Some(1),
        true,
        Some(&entries),
    );
    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::CombinedUnstaged,
        pb("d"),
        Some(0),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        Some(2),
        false,
        Some(&entries),
    );

    assert_eq!(sel.unstaged, vec![pb("b"), pb("c"), pb("d")]);
    assert_eq!(sel.unstaged_anchor, Some(pb("b")));
    assert_eq!(sel.unstaged_anchor_index, Some(1));
    assert!(
        status_selection_scan_steps() > 0,
        "stale index hints should fall back to a path scan"
    );
}

#[test]
fn staged_selection_clears_other_section_anchor_indexes() {
    let mut sel = StatusMultiSelection {
        untracked: vec![pb("new.txt")],
        untracked_anchor: Some(pb("new.txt")),
        unstaged: vec![pb("tracked.txt")],
        unstaged_anchor: Some(pb("tracked.txt")),
        unstaged_anchor_index: Some(4),
        ..Default::default()
    };

    apply_status_multi_selection_click(
        &mut sel,
        StatusSection::Staged,
        pb("staged.txt"),
        Some(2),
        gpui::Modifiers::default(),
        Some(3),
        false,
        None,
    );

    assert!(sel.untracked.is_empty());
    assert!(sel.unstaged.is_empty());
    assert!(sel.untracked_anchor.is_none());
    assert!(sel.unstaged_anchor.is_none());
    assert!(sel.unstaged_anchor_index.is_none());
    assert!(sel.unstaged_anchor_order_rev.is_none());
    assert_eq!(sel.staged, vec![pb("staged.txt")]);
    assert_eq!(sel.staged_anchor, Some(pb("staged.txt")));
    assert_eq!(sel.staged_anchor_index, Some(2));
    assert_eq!(sel.staged_anchor_order_rev, Some(3));
}
