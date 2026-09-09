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
            additions: Some(3),
            deletions: Some(1),
        },
        FileTreeItem {
            path: owned[1].as_path(),
            additions: Some(4),
            deletions: None,
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
