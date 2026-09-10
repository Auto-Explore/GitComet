use super::*;
use crate::view::rows::CommitFileSort;

fn tree(paths: &[&str], sort: CommitFileSort) -> FileTree {
    let owned: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    FileTree::build(owned.iter().map(|p| FileTreeItem::new(p.as_path())), sort)
}

fn plan(paths: &[&str], collapsed: &CollapsedDirs) -> FileListPlan {
    tree(paths, CommitFileSort::PathAscending).flatten(collapsed)
}

fn dir_labels(plan: &FileListPlan) -> Vec<String> {
    (0..plan.row_len())
        .filter_map(|ix| match plan.row_at(RowIx(ix)) {
            Some(FileListRow::Directory { label, .. }) => Some(label.to_string()),
            _ => None,
        })
        .collect()
}

fn rows(plan: &FileListPlan) -> Vec<String> {
    (0..plan.row_len())
        .filter_map(|ix| plan.row_at(RowIx(ix)))
        .map(|row| match row {
            FileListRow::Directory {
                label,
                depth,
                collapsed,
                ..
            } => format!(
                "{}{}{}/",
                "  ".repeat(depth),
                if collapsed { ">" } else { "v" },
                label
            ),
            FileListRow::File { ordinal, depth } => {
                format!("{}#{}", "  ".repeat(depth), ordinal.0)
            }
        })
        .collect()
}

fn collapse_label(plan: &FileListPlan, collapsed: &mut CollapsedDirs, label: &str) {
    let row = (0..plan.row_len())
        .filter_map(|ix| plan.row_at(RowIx(ix)))
        .find(|row| matches!(row, FileListRow::Directory { label: l, .. } if l == label))
        .unwrap_or_else(|| panic!("no directory row labelled {label}"));
    let FileListRow::Directory { key, chain, .. } = row else {
        unreachable!()
    };
    collapsed.collapse(key, &chain);
}

#[test]
fn flat_plan_is_the_identity() {
    let plan = FileListPlan::flat(3);
    assert_eq!(plan.row_len(), 3);
    assert_eq!(plan.file_count(), 3);
    for ix in 0..3 {
        assert_eq!(plan.ordinal_at(RowIx(ix)), Some(FileOrdinal(ix)));
        assert_eq!(plan.row_ix_for_ordinal(FileOrdinal(ix)), Some(RowIx(ix)));
    }
    assert_eq!(plan.row_at(RowIx(3)), None);
    assert!(plan.reveal(FileOrdinal(0)).is_empty());
}

#[test]
fn single_child_chains_fold_into_one_row() {
    let plan = plan(&["a/b/c/d.rs", "a/b/c/e.rs"], &CollapsedDirs::default());
    assert_eq!(dir_labels(&plan), vec!["a/b/c"]);
    assert_eq!(rows(&plan), vec!["va/b/c/", "  #0", "  #1"]);
}

#[test]
fn a_new_sibling_splits_a_folded_chain() {
    let plan = plan(
        &["a/b/c/d.rs", "a/b/c/e.rs", "a/f.rs"],
        &CollapsedDirs::default(),
    );
    assert_eq!(dir_labels(&plan), vec!["a", "b/c"]);
    assert_eq!(
        rows(&plan),
        vec!["va/", "  vb/c/", "    #0", "    #1", "  #2"]
    );
}

#[test]
fn collapse_survives_a_chain_split() {
    let mut collapsed = CollapsedDirs::default();
    let before = plan(&["a/b/c/d.rs", "a/b/c/e.rs"], &collapsed);
    collapse_label(&before, &mut collapsed, "a/b/c");

    let after = plan(&["a/b/c/d.rs", "a/b/c/e.rs", "a/f.rs"], &collapsed);
    assert_eq!(rows(&after), vec!["va/", "  >b/c/", "  #2"]);
}

#[test]
fn collapse_survives_a_chain_merge() {
    let mut collapsed = CollapsedDirs::default();
    let before = plan(&["a/b/c/d.rs", "a/b/c/e.rs", "a/f.rs"], &collapsed);
    collapse_label(&before, &mut collapsed, "a");

    // `a/f.rs` is gone, so `a` folds back into `a/b/c` and the key the user
    // collapsed no longer names a row of its own.
    let after = plan(&["a/b/c/d.rs", "a/b/c/e.rs"], &collapsed);
    assert_eq!(rows(&after), vec![">a/b/c/"]);
}

#[test]
fn collapsing_replaces_stale_chain_segments() {
    let mut collapsed = CollapsedDirs::default();
    let split = plan(&["a/b/c/d.rs", "a/f.rs"], &collapsed);
    collapse_label(&split, &mut collapsed, "a");
    assert_eq!(collapsed.set().len(), 1);

    let folded = plan(&["a/b/c/d.rs"], &collapsed);
    collapse_label(&folded, &mut collapsed, "a/b/c");
    assert_eq!(
        collapsed.set().len(),
        1,
        "the chain's other segments are dropped, not accumulated"
    );
}

#[test]
fn ordered_is_collapse_independent() {
    let open = plan(&["a/b/c/d.rs", "a/f.rs", "g.rs"], &CollapsedDirs::default());
    let open_order: Vec<usize> = open.ordered().iter().collect();

    let mut collapsed = CollapsedDirs::default();
    collapse_label(&open, &mut collapsed, "a");
    let shut = plan(&["a/b/c/d.rs", "a/f.rs", "g.rs"], &collapsed);

    assert_eq!(shut.ordered().iter().collect::<Vec<_>>(), open_order);
    assert_eq!(shut.file_count(), 3);
}

#[test]
fn reveal_expands_every_collapsed_ancestor_at_once() {
    let mut collapsed = CollapsedDirs::default();
    let paths = ["a/b/c/d.rs", "a/f.rs"];
    let open = plan(&paths, &collapsed);
    collapse_label(&open, &mut collapsed, "b/c");
    collapse_label(&plan(&paths, &collapsed), &mut collapsed, "a");

    let shut = plan(&paths, &collapsed);
    assert_eq!(shut.row_ix_for_ordinal(FileOrdinal(0)), None);

    let chains = shut.reveal(FileOrdinal(0));
    assert_eq!(chains.len(), 2, "both the outer and the nested collapse");
    for chain in chains {
        collapsed.expand(&chain);
    }

    let reopened = plan(&paths, &collapsed);
    assert!(reopened.row_ix_for_ordinal(FileOrdinal(0)).is_some());
}

#[test]
fn path_sorts_group_directories_and_mirror_each_other() {
    let paths = ["a/one.rs", "z.rs"];
    let ascending = tree(&paths, CommitFileSort::PathAscending).flatten(&CollapsedDirs::default());
    assert_eq!(rows(&ascending), vec!["va/", "  #0", "#1"]);

    let descending =
        tree(&paths, CommitFileSort::PathDescending).flatten(&CollapsedDirs::default());
    assert_eq!(rows(&descending), vec!["#1", "va/", "  #0"]);
}

#[test]
fn edit_size_sorts_interleave_directories_with_files() {
    // Projection order is the caller's; the tree only refines it.
    let paths = ["big.rs", "src/small.rs"];
    let interleaved =
        tree(&paths, CommitFileSort::EditSizeDescending).flatten(&CollapsedDirs::default());
    assert_eq!(rows(&interleaved), vec!["#0", "vsrc/", "  #1"]);
}

#[test]
fn tree_order_is_a_refinement_of_flat_order() {
    for sort in CommitFileSort::ALL {
        let paths = ["a/b/one.rs", "a/two.rs", "c/three.rs", "four.rs"];
        let plan = tree(&paths, sort).flatten(&CollapsedDirs::default());
        let mut seen: Vec<usize> = plan.ordered().iter().collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            vec![0, 1, 2, 3],
            "{sort:?} keeps every file exactly once"
        );
        assert_eq!(plan.ordered().len(), 4);
    }
}

#[test]
fn directory_rows_carry_subtree_edit_totals() {
    let owned = [PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")];
    let items = [
        FileTreeItem {
            path: owned[0].as_path(),
            kind: Some(gitcomet_core::domain::FileStatusKind::Modified),
            additions: Some(3),
            deletions: Some(1),
        },
        FileTreeItem {
            path: owned[1].as_path(),
            kind: Some(gitcomet_core::domain::FileStatusKind::Added),
            additions: Some(4),
            deletions: Some(0),
        },
    ];
    let plan = FileTree::build(items.into_iter(), CommitFileSort::PathAscending)
        .flatten(&CollapsedDirs::default());
    let Some(FileListRow::Directory {
        additions,
        deletions,
        subtree,
        ..
    }) = plan.row_at(RowIx(0))
    else {
        panic!("expected a directory row");
    };
    assert_eq!(additions, Some(7));
    assert_eq!(deletions, Some(1));
    assert_eq!(subtree, 0..2);
}

fn kinded(paths: &[(&str, gitcomet_core::domain::FileStatusKind)]) -> Vec<PathBuf> {
    paths.iter().map(|(path, _)| PathBuf::from(path)).collect()
}

#[test]
fn directory_rows_carry_subtree_kind_counts() {
    use gitcomet_core::domain::FileStatusKind::*;

    let entries = [
        ("src/a.rs", Modified),
        ("src/b.rs", Modified),
        ("src/nested/c.rs", Added),
        ("src/nested/d.rs", Deleted),
        ("src/e.rs", Renamed),
    ];
    let owned = kinded(&entries);
    let items = owned
        .iter()
        .zip(entries.iter())
        .map(|(path, (_, kind))| FileTreeItem {
            path: path.as_path(),
            kind: Some(*kind),
            additions: None,
            deletions: None,
        });
    let plan =
        FileTree::build(items, CommitFileSort::PathAscending).flatten(&CollapsedDirs::default());

    let Some(FileListRow::Directory { counts, .. }) = plan.row_at(RowIx(0)) else {
        panic!("expected the src/ folder row");
    };
    assert_eq!(counts.all, 5);
    assert_eq!(counts.modified, 2);
    assert_eq!(counts.added, 1);
    assert_eq!(counts.removed, 1);
    assert_eq!(counts.renamed, 1, "renames stay their own badge");
}

/// The badge counts the subtree, not what is on screen.
#[test]
fn kind_counts_are_collapse_independent() {
    use gitcomet_core::domain::FileStatusKind::Modified;

    let paths = ["a/b/one.rs", "a/b/two.rs", "a/three.rs"];
    let owned: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let build = || {
        FileTree::build(
            owned.iter().map(|path| FileTreeItem {
                path: path.as_path(),
                kind: Some(Modified),
                additions: None,
                deletions: None,
            }),
            CommitFileSort::PathAscending,
        )
    };

    let mut collapsed = CollapsedDirs::default();
    let open = build().flatten(&collapsed);
    let open_counts = match open.row_at(RowIx(0)) {
        Some(FileListRow::Directory { counts, .. }) => counts,
        other => panic!("expected a folder row, got {other:?}"),
    };

    collapse_label(&open, &mut collapsed, "b");
    let shut = build().flatten(&collapsed);
    let shut_counts = match shut.row_at(RowIx(0)) {
        Some(FileListRow::Directory { counts, .. }) => counts,
        other => panic!("expected a folder row, got {other:?}"),
    };
    assert_eq!(shut_counts, open_counts);
    assert_eq!(shut_counts.all, 3);
}

/// Locks the bottom-up roll-up: totals reach every leaf, not just children.
#[test]
fn nested_directory_totals_roll_all_the_way_up() {
    use gitcomet_core::domain::FileStatusKind::Modified;

    let owned = [
        PathBuf::from("a/b/c/deep.rs"),
        PathBuf::from("a/b/mid.rs"),
        PathBuf::from("a/top.rs"),
    ];
    let plan = FileTree::build(
        owned.iter().enumerate().map(|(ix, path)| FileTreeItem {
            path: path.as_path(),
            kind: Some(Modified),
            additions: Some(ix as u32 + 1),
            deletions: Some(1),
        }),
        CommitFileSort::PathAscending,
    )
    .flatten(&CollapsedDirs::default());

    let Some(FileListRow::Directory {
        counts,
        additions,
        deletions,
        ..
    }) = plan.row_at(RowIx(0))
    else {
        panic!("expected the a/ folder row");
    };
    assert_eq!(counts.all, 3, "every leaf below, not just direct children");
    assert_eq!(additions, Some(1 + 2 + 3));
    assert_eq!(deletions, Some(3));
}

#[test]
fn root_level_files_keep_depth_zero() {
    let plan = plan(&["only.rs"], &CollapsedDirs::default());
    assert_eq!(
        plan.row_at(RowIx(0)),
        Some(FileListRow::File {
            ordinal: FileOrdinal(0),
            depth: 0
        })
    );
}

/// UI scale grows the row but not the pane, so 100% and 200% differ.
#[test]
fn directory_row_detail_drops_the_stat_before_the_label() {
    use crate::view::rows::{
        CommitFileKindCounts, DirectoryRowDetail, directory_row_detail_for_width,
    };
    use gpui::px;

    let counts = CommitFileKindCounts {
        all: 12,
        modified: 7,
        added: 3,
        removed: 2,
        renamed: 0,
    };

    assert_eq!(
        directory_row_detail_for_width(gpui::Pixels::MAX, 0, counts, true, 100),
        DirectoryRowDetail::BadgesAndStat,
        "an unmeasured list always shows everything"
    );
    assert_eq!(
        directory_row_detail_for_width(px(600.0), 0, counts, true, 100),
        DirectoryRowDetail::BadgesAndStat
    );
    // ~232 design px needed: 400 device px is roomy at 100%, short at 200%.
    assert_eq!(
        directory_row_detail_for_width(px(400.0), 0, counts, true, 100),
        DirectoryRowDetail::BadgesAndStat
    );
    assert_eq!(
        directory_row_detail_for_width(px(400.0), 0, counts, true, 200),
        DirectoryRowDetail::BadgesOnly,
        "the same pane is too narrow once everything is twice the size"
    );
    assert_eq!(
        directory_row_detail_for_width(px(180.0), 0, counts, true, 100),
        DirectoryRowDetail::BadgesOnly
    );
    assert_eq!(
        directory_row_detail_for_width(px(600.0), 0, counts, false, 100),
        DirectoryRowDetail::BadgesOnly,
        "nothing to show means nothing to budget for"
    );

    // Depth costs indent, so a deep row gives way before a shallow one.
    let deep = directory_row_detail_for_width(px(260.0), 8, counts, true, 100);
    let shallow = directory_row_detail_for_width(px(260.0), 0, counts, true, 100);
    assert_eq!(deep, DirectoryRowDetail::BadgesOnly);
    assert_eq!(shallow, DirectoryRowDetail::BadgesAndStat);
}

/// A non-UTF-8 component still gets its own node. Dropping it would attach the
/// file to its grandparent, making `a/<bad>/x.rs` and `a/x.rs` look identical.
#[cfg(unix)]
#[test]
fn a_non_utf8_path_component_still_forms_its_own_directory() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let mut weird = PathBuf::from("a");
    weird.push(OsStr::from_bytes(b"b\xffd"));
    weird.push("x.rs");
    let plain = PathBuf::from("a/x.rs");
    let owned = [weird, plain];

    let plan = FileTree::build(
        owned.iter().map(|path| FileTreeItem {
            path: path.as_path(),
            kind: Some(gitcomet_core::domain::FileStatusKind::Modified),
            additions: None,
            deletions: None,
        }),
        CommitFileSort::PathAscending,
    )
    .flatten(&CollapsedDirs::default());

    let dirs: Vec<String> = (0..plan.row_len())
        .filter_map(|ix| match plan.row_at(RowIx(ix)) {
            Some(FileListRow::Directory { label, .. }) => Some(label.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        dirs.len(),
        2,
        "expected `a` and the lossy child to be separate folders, got {dirs:?}"
    );
    assert_eq!(plan.ordered().len(), 2, "both files are still present");
}

/// Every worktree in one scan shares a `worktree_dirty_rev`, so without the
/// scope two of them would be served each other's trie.
#[test]
fn projection_keys_separate_lists_that_share_a_rev() {
    use crate::view::rows::{
        CommitFileFilter, CommitFileSort, file_list_projection_key, file_list_projection_key_scoped,
    };

    let base = |scope: Option<&Path>| {
        file_list_projection_key_scoped(
            7,
            42,
            CommitFileSort::PathAscending,
            CommitFileFilter::All,
            scope,
        )
    };
    let a = PathBuf::from("/tmp/wt-a");
    let b = PathBuf::from("/tmp/wt-b");

    assert_ne!(
        base(Some(a.as_path())),
        base(Some(b.as_path())),
        "two worktrees at the same rev must not share a cache slot"
    );
    assert_eq!(
        base(Some(a.as_path())),
        base(Some(a.as_path())),
        "the same worktree keeps its slot"
    );
    assert_eq!(
        base(None),
        file_list_projection_key(7, 42, CommitFileSort::PathAscending, CommitFileFilter::All),
        "an unscoped list keeps the plain key"
    );
}

/// Half-known counts are the shape `commit_file_line_stats` never produces —
/// it reports `(None, None)` when either side is unreadable. Folding one into
/// `Some((n, 0))` would put a made-up zero into the folder badge's subtotal, so
/// the builder keeps unknown unknown.
#[test]
fn a_one_sided_unknown_count_is_not_folded_into_a_directory_total() {
    let known = PathBuf::from("dir/known.rs");
    let half = PathBuf::from("dir/half.rs");
    let tree = FileTree::build(
        [
            FileTreeItem {
                path: known.as_path(),
                kind: Some(gitcomet_core::domain::FileStatusKind::Modified),
                additions: Some(3),
                deletions: Some(1),
            },
            FileTreeItem {
                path: half.as_path(),
                kind: Some(gitcomet_core::domain::FileStatusKind::Modified),
                additions: Some(5),
                deletions: None,
            },
        ]
        .into_iter(),
        CommitFileSort::PathAscending,
    );
    let plan = tree.flatten(&CollapsedDirs::default());
    let totals = (0..plan.row_len())
        .filter_map(|ix| match plan.row_at(RowIx(ix)) {
            Some(FileListRow::Directory {
                additions,
                deletions,
                ..
            }) => Some((additions, deletions)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        totals,
        vec![(Some(3), Some(1))],
        "the half-known file must not contribute a deletions count nobody measured"
    );
}

/// `display_position` runs once per *visible* row every frame, so a scan over
/// every file in the list makes one section's render O(visible x files).
#[test]
#[ignore]
fn perf_display_position_per_visible_row() {
    const FILES: usize = 5_000;
    const VISIBLE: usize = 40;
    const FRAMES: usize = 200;

    let paths: Vec<PathBuf> = (0..FILES)
        .map(|ix| PathBuf::from(format!("crates/pkg_{}/src/file_{ix}.rs", ix % 50)))
        .collect();
    let tree = FileTree::build(
        paths.iter().map(|p| FileTreeItem::new(p.as_path())),
        CommitFileSort::PathAscending,
    );
    let plan = tree.flatten(&CollapsedDirs::default());

    // The worst case is the visible window sitting at the end of the list.
    let window: Vec<FileOrdinal> = (FILES - VISIBLE..FILES).map(FileOrdinal).collect();
    let start = std::time::Instant::now();
    let mut sink = 0usize;
    for _ in 0..FRAMES {
        for ordinal in &window {
            sink += plan.display_position(*ordinal).unwrap_or(0);
        }
    }
    let elapsed = start.elapsed();
    eprintln!(
        "display_position: {FRAMES} frames x {VISIBLE} rows over {FILES} files in {elapsed:?} ({:?}/frame, sink={sink})",
        elapsed / FRAMES as u32
    );
}
