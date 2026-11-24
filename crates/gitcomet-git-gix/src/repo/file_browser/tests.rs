use crate::test_support::{git_stdout, git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::domain::{CommitId, FileEntry, FileEntryKind};
use std::fs;
use std::path::Path;

fn paths_of(entries: &[FileEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect()
}

fn commit_file(workdir: &Path, path: &str, contents: &str, message: &str) {
    write_file(workdir, path, contents);
    git_success(workdir, &["add", path]);
    git_success(workdir, &["commit", "-m", message]);
}

#[test]
fn tracked_sample_is_bounded_and_spans_the_index_in_a_clean_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    init_test_repo(workdir);
    for path in ["a.c", "b.c", "c.c", "d.rs", "e.rs", "f.rs"] {
        write_file(workdir, path, "");
    }
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "tracked languages"]);
    let repo = open_repo(workdir);
    let limit = std::num::NonZeroUsize::new(2).unwrap();
    assert_eq!(
        repo.sample_tracked_file_paths_impl(limit).unwrap(),
        [Path::new("a.c"), Path::new("d.rs")]
    );
}

#[test]
fn list_worktree_files_returns_flat_files_correctly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "README.md", "# Project", "first");
    commit_file(workdir, "main.rs", "fn main() {}", "second");

    let repo = open_repo(workdir);
    let entries = repo.list_worktree_files_impl().expect("list tree files");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "README.md");
    assert_eq!(entries[0].kind, FileEntryKind::File);
    assert_eq!(entries[0].depth, 0);
    assert_eq!(entries[1].name, "main.rs");
    assert_eq!(entries[1].kind, FileEntryKind::File);
    assert_eq!(entries[1].depth, 0);
}

#[test]
fn list_worktree_files_hides_service_owned_directories() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    commit_file(workdir, "src/main.rs", "fn main() {}", "first");
    write_file(workdir, "src/.gitcomet-operation-abc/item", "parked");
    write_file(workdir, ".gitcomet-operation-def/recovery.log", "move");
    write_file(workdir, "src/.gitcomet-save-xyz", "staged");
    write_file(workdir, "src/new.rs", "untracked");

    let entries = open_repo(workdir)
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(paths_of(&entries), ["src", "src/main.rs", "src/new.rs"]);
}

#[test]
fn list_worktree_files_sorts_directories_before_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "z_file.txt", "z", "first");
    commit_file(workdir, "a_dir/nested.txt", "nested", "second");

    let repo = open_repo(workdir);
    let entries = repo.list_worktree_files_impl().expect("list tree files");

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].name, "a_dir");
    assert_eq!(entries[0].kind, FileEntryKind::Directory);
    assert_eq!(entries[0].depth, 0);
    assert_eq!(entries[1].name, "nested.txt");
    assert_eq!(entries[1].kind, FileEntryKind::File);
    assert_eq!(entries[1].depth, 1);
    assert_eq!(entries[2].name, "z_file.txt");
    assert_eq!(entries[2].kind, FileEntryKind::File);
    assert_eq!(entries[2].depth, 0);
}

#[test]
fn list_worktree_files_nested_directories_have_correct_depth() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "src/main.rs", "fn main() {}", "add src/main.rs");
    commit_file(
        workdir,
        "src/utils/helpers.rs",
        "pub fn help() {}",
        "add helpers",
    );

    let repo = open_repo(workdir);
    let entries = repo.list_worktree_files_impl().expect("list tree files");

    assert_eq!(entries.len(), 4);

    assert_eq!(entries[0].name, "src");
    assert_eq!(entries[0].kind, FileEntryKind::Directory);
    assert_eq!(entries[0].depth, 0);

    assert_eq!(entries[1].name, "utils");
    assert_eq!(entries[1].kind, FileEntryKind::Directory);
    assert_eq!(entries[1].depth, 1);

    assert_eq!(entries[2].name, "helpers.rs");
    assert_eq!(entries[2].kind, FileEntryKind::File);
    assert_eq!(entries[2].depth, 2);

    assert_eq!(entries[3].name, "main.rs");
    assert_eq!(entries[3].kind, FileEntryKind::File);
    assert_eq!(entries[3].depth, 1);
}

/// Expansion state and reveal-by-path are keyed by path across both sources,
/// so a divergence here collapses the tree when browsing a commit and back.
#[test]
fn worktree_and_commit_listings_agree_on_a_clean_tree() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "alpha.txt", "alpha", "first commit");
    commit_file(workdir, "beta.txt", "beta", "second commit");
    commit_file(workdir, "src/nested/deep.txt", "deep", "third commit");

    let repo = open_repo(workdir);

    let head_entries = repo.list_worktree_files_impl().expect("head tree");

    let commit_id = CommitId(git_stdout(workdir, &["rev-parse", "HEAD"]).into());

    let commit_entries = repo
        .list_tree_files_at_commit_impl(&commit_id)
        .expect("commit tree");

    assert_eq!(head_entries.len(), commit_entries.len());
    for (a, b) in head_entries.iter().zip(commit_entries.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.path, b.path);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.depth, b.depth);
    }
}

#[test]
fn list_tree_files_at_commit_respects_historical_tree() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "only_in_first.txt", "first", "first commit");

    let first_commit_id = CommitId(git_stdout(workdir, &["rev-parse", "HEAD"]).into());

    commit_file(workdir, "added_later.txt", "later", "second commit");

    let repo = open_repo(workdir);

    let first_entries = repo
        .list_tree_files_at_commit_impl(&first_commit_id)
        .expect("first commit tree");

    assert_eq!(first_entries.len(), 1);
    assert_eq!(first_entries[0].name, "only_in_first.txt");
}

/// The reported bug: the listing came from `HEAD`'s tree, so a new folder of
/// untracked files was invisible.
#[test]
fn list_worktree_files_includes_untracked_files_in_a_new_folder() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "committed.txt", "committed", "first");
    write_file(workdir, "newfolder/a.txt", "a");
    write_file(workdir, "newfolder/b.txt", "b");
    write_file(workdir, "newfolder/c.txt", "c");

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(
        paths_of(&entries),
        vec![
            "newfolder",
            "newfolder/a.txt",
            "newfolder/b.txt",
            "newfolder/c.txt",
            "committed.txt",
        ]
    );
    assert_eq!(entries[0].kind, FileEntryKind::Directory);
    assert_eq!(entries[0].depth, 0);
    assert_eq!(entries[1].kind, FileEntryKind::File);
    assert_eq!(entries[1].depth, 1);
}

#[test]
fn list_worktree_files_includes_staged_but_uncommitted_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "committed.txt", "committed", "first");
    write_file(workdir, "staged/new.txt", "staged");
    git_success(workdir, &["add", "staged/new.txt"]);

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(
        paths_of(&entries),
        vec!["staged", "staged/new.txt", "committed.txt"]
    );
}

#[test]
fn list_worktree_files_omits_files_deleted_from_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "kept.txt", "kept", "first");
    commit_file(workdir, "removed.txt", "removed", "second");
    fs::remove_file(workdir.join("removed.txt")).expect("remove file");

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(paths_of(&entries), vec!["kept.txt"]);
}

#[test]
fn list_worktree_files_omits_ignored_entries() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, ".gitignore", "target/\n*.log\n", "add ignores");
    write_file(workdir, "target/debug/huge.bin", "binary");
    write_file(workdir, "noisy.log", "log");
    write_file(workdir, "kept.txt", "kept");

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(paths_of(&entries), vec![".gitignore", "kept.txt"]);
}

#[test]
fn list_worktree_files_includes_empty_directories() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    commit_file(workdir, "file.txt", "file", "first");
    fs::create_dir_all(workdir.join("empty_dir")).expect("create dir");

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(paths_of(&entries), vec!["empty_dir", "file.txt"]);
    assert_eq!(entries[0].kind, FileEntryKind::Directory);
}

/// An unborn HEAD used to fail `head_commit()`, so every freshly-initialized
/// repository showed "Error loading files."
#[test]
fn list_worktree_files_works_without_any_commit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "src/main.rs", "fn main() {}");

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(paths_of(&entries), vec!["src", "src/main.rs"]);
}

#[test]
fn list_worktree_files_sorts_directories_first_then_alphabetically() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "z_file.txt", "z");
    write_file(workdir, "a_file.txt", "a");
    write_file(workdir, "z_dir/inner.txt", "inner");
    write_file(workdir, "a_dir/inner.txt", "inner");

    let repo = open_repo(workdir);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    assert_eq!(
        paths_of(&entries),
        vec![
            "a_dir",
            "a_dir/inner.txt",
            "z_dir",
            "z_dir/inner.txt",
            "a_file.txt",
            "z_file.txt",
        ]
    );
}

/// The tree walk drops gitlinks outright (mode `160000` is neither tree, blob,
/// nor link), so submodules used to be missing entirely.
#[test]
fn list_worktree_files_lists_a_submodule_as_a_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let outer = tmp.path().join("outer");
    let inner = tmp.path().join("inner");
    fs::create_dir_all(&outer).expect("create outer");
    fs::create_dir_all(&inner).expect("create inner");

    init_test_repo(&inner);
    commit_file(&inner, "inner.txt", "inner", "inner commit");

    init_test_repo(&outer);
    commit_file(&outer, "outer.txt", "outer", "outer commit");
    git_success(
        &outer,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            inner.to_str().expect("utf8 path"),
            "sub",
        ],
    );

    let repo = open_repo(&outer);
    let entries = repo
        .list_worktree_files_impl()
        .expect("list worktree files");

    let sub = entries
        .iter()
        .find(|e| e.name == "sub")
        .expect("submodule listed");
    assert_eq!(sub.kind, FileEntryKind::Directory);
    // `Path::starts_with` compares whole components, so the `sub` row itself
    // matches `sub/` and has to be excluded by depth.
    assert!(
        !entries
            .iter()
            .any(|e| e.depth > 0 && e.path.starts_with("sub")),
        "submodule contents leaked into the parent listing: {:?}",
        paths_of(&entries)
    );
}
