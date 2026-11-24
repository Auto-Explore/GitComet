use super::*;
use crate::test_support::{git_success, init_test_repo, open_repo};
use gitcomet_core::domain::{DiffArea, DiffTarget};

fn nested_pointer_moves(dir: &Path) -> (CommitId, CommitId) {
    init_test_repo(dir);
    let point = |digit: char| {
        let id = digit
            .to_string()
            .repeat(gix::open(dir).unwrap().object_hash().len_in_hex());
        git_success(
            dir,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{id},vendor/x"),
            ],
        );
    };
    let head = || {
        CommitId(
            gix::open(dir)
                .unwrap()
                .head_id()
                .unwrap()
                .to_string()
                .into(),
        )
    };
    point('1');
    git_success(dir, &["commit", "-q", "-m", "first"]);
    let first = head();
    point('2');
    git_success(dir, &["commit", "-q", "-m", "second"]);
    let second = head();
    point('3');
    (first, second)
}

fn pointer_counts(changes: &[SubmoduleInnerChange]) -> (Option<u32>, Option<u32>) {
    let pointer = changes
        .iter()
        .find(|change| change.path == Path::new("vendor/x"))
        .expect("the nested pointer is listed");
    (pointer.additions, pointer.deletions)
}

/// numstat counts a gitlink's "Subproject commit" line, not content.
#[test]
fn a_nested_pointer_has_no_line_counts_between_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let (first, second) = nested_pointer_moves(tmp.path());
    let changes = submodule_range_changes_from_commits(
        tmp.path(),
        &first,
        &second,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(pointer_counts(&changes), (None, None));
}

#[test]
fn a_nested_pointer_has_no_line_counts_in_the_live_lists() {
    let tmp = tempfile::tempdir().unwrap();
    nested_pointer_moves(tmp.path());
    let repo = open_repo(tmp.path()).repo();
    let (staged, _) =
        submodule_live_inner_changes(tmp.path(), Some(&repo), &CancellationToken::new()).unwrap();
    assert_eq!(pointer_counts(&staged), (None, None));
}

/// Stale stats would make `git diff --numstat` write refreshed ones back to
/// the submodule's index, which its watcher reports as an index change.
#[test]
fn live_lists_do_not_write_the_submodule_index() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    std::fs::write(dir.join("touched.txt"), "unchanged\n").unwrap();
    std::fs::write(dir.join("edited.txt"), "one\ntwo\n").unwrap();
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-q", "-m", "seed"]);
    std::fs::write(dir.join("edited.txt"), "one\n2\n").unwrap();
    let stale =
        filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 120, 0);
    for name in ["touched.txt", "edited.txt"] {
        crate::test_support::set_file_mtime(dir.join(name), stale).unwrap();
    }
    let index = dir.join(".git/index");
    let before = std::fs::read(&index).unwrap();

    let repo = open_repo(dir).repo();
    let (_, unstaged) =
        submodule_live_inner_changes(dir, Some(&repo), &CancellationToken::new()).unwrap();

    let listed: Vec<_> = unstaged
        .iter()
        .map(|change| (change.path.as_path(), change.additions, change.deletions))
        .collect();
    assert_eq!(listed, vec![(Path::new("edited.txt"), Some(1), Some(1))]);
    assert_eq!(
        std::fs::read(&index).unwrap(),
        before,
        "the live lists refreshed index stat metadata"
    );
}

/// numstat still counts a gitlink replaced by a file, submodules ignored
/// or not; that change is a submodule's, so it has no counts either.
#[test]
fn a_nested_pointer_replaced_by_a_file_has_no_line_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (_, second) = nested_pointer_moves(dir);
    git_success(dir, &["rm", "--cached", "-q", "vendor/x"]);
    std::fs::create_dir_all(dir.join("vendor")).unwrap();
    std::fs::write(dir.join("vendor/x"), "a\nb\nc\n").unwrap();
    git_success(dir, &["add", "vendor/x"]);
    git_success(dir, &["commit", "-q", "-m", "third"]);
    let third = CommitId(
        gix::open(dir)
            .unwrap()
            .head_id()
            .unwrap()
            .to_string()
            .into(),
    );

    let changes =
        submodule_range_changes_from_commits(dir, &second, &third, &CancellationToken::new())
            .unwrap();

    assert_eq!(pointer_counts(&changes), (None, None));
}

#[test]
fn worktree_summary_for_staged_removed_gitlink_uses_head_pointer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let submodule_path = Path::new("vendor/submodule");
    init_test_repo(tmp.path());
    git_success(
        tmp.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!(
                "160000,{},vendor/submodule",
                "1".repeat(gix::open(tmp.path()).unwrap().object_hash().len_in_hex())
            ),
        ],
    );
    git_success(tmp.path(), &["commit", "-m", "add submodule gitlink"]);
    git_success(
        tmp.path(),
        &["update-index", "--force-remove", "vendor/submodule"],
    );

    let repo = open_repo(tmp.path());
    let summary = repo
        .submodule_diff_summary_cancellable_impl(
            &DiffTarget::working_tree(submodule_path.into(), DiffArea::Staged),
            &gitcomet_core::services::CancellationToken::new(),
        )
        .expect("staged submodule removal summary");
    let staged_range = summary
        .ranges
        .iter()
        .find(|range| range.kind == SubmoduleDiffRangeKind::StagedPointer)
        .expect("staged pointer range");

    assert_eq!(summary.path, submodule_path);
    assert_eq!(summary.status, None);
    assert_eq!(
        staged_range.from,
        Some(CommitId(
            "1".repeat(repo.repo().object_hash().len_in_hex()).into()
        ))
    );
    assert_eq!(staged_range.to, None);
}
