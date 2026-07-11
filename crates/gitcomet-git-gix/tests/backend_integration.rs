use gitcomet_core::error::ErrorKind;
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("run git command");
    assert!(status.success(), "git {:?} failed", args);
}

/// `git init` a non-bare repo at `path` with one commit, using an inline
/// identity so the test does not depend on ambient git config.
fn init_repo_with_commit(path: &Path) {
    fs::create_dir_all(path).expect("create repo directory");
    run_git(path, &["init"]);
    fs::write(path.join("file.txt"), "contents").expect("write file");
    run_git(path, &["add", "."]);
    run_git(
        path,
        &[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit",
            "-m",
            "init",
        ],
    );
}

#[test]
fn gix_backend_open_succeeds_for_git_repository() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo directory");

    run_git(&repo, &["init"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open repository");
    assert_eq!(
        opened.spec().workdir,
        canonicalize_or_original(repo.clone())
    );
}

#[test]
fn gix_backend_open_maps_not_a_repository_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let non_repo = dir.path().join("plain-dir");
    fs::create_dir_all(&non_repo).expect("create plain directory");

    let backend = GixBackend;
    let err = match backend.open(&non_repo) {
        Ok(_) => panic!("opening a non-git directory should fail"),
        Err(err) => err,
    };
    assert!(matches!(err.kind(), ErrorKind::NotARepository));
}

#[test]
fn gix_backend_open_maps_io_error_for_missing_path() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let missing = dir.path().join("does-not-exist");

    let backend = GixBackend;
    let err = match backend.open(&missing) {
        Ok(_) => panic!("opening a missing path should fail"),
        Err(err) => err,
    };
    assert!(matches!(
        err.kind(),
        ErrorKind::Io(std::io::ErrorKind::NotFound)
    ));
}

// gix misreads a worktree whose directory ends in `.git` as a bare git dir, so
// a plain `gix::open` fails on it. These tests pin that `GixBackend::open` (via
// the crate's single `open_worktree_repo` chokepoint) opens such repos.

#[test]
fn gix_backend_open_succeeds_for_dot_git_suffixed_worktree() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("myrepo.git");
    init_repo_with_commit(&repo);

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open .git-suffixed repository");
    assert_eq!(opened.spec().workdir, canonicalize_or_original(repo));
}

#[test]
fn gix_backend_open_succeeds_for_dot_git_suffixed_linked_worktree() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let main = dir.path().join("main");
    init_repo_with_commit(&main);

    // A linked worktree stores its `.git` as a `gitdir:` file, not a directory.
    let linked = dir.path().join("linked.git");
    run_git(&main, &["worktree", "add", linked.to_str().unwrap()]);
    assert!(
        linked.join(".git").is_file(),
        "linked worktree has a .git file"
    );

    let backend = GixBackend;
    let opened = backend
        .open(&linked)
        .expect("open .git-suffixed linked worktree");
    assert_eq!(opened.spec().workdir, canonicalize_or_original(linked));
}

// Regression guard for the submodule open path: enumerating a submodule whose
// worktree directory ends in `.git` must still resolve its checked-out head,
// which requires the nested repository to open successfully.
#[test]
fn list_submodules_opens_dot_git_suffixed_submodule() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let sub_src = dir.path().join("sub-src");
    init_repo_with_commit(&sub_src);

    let superproject = dir.path().join("super");
    init_repo_with_commit(&superproject);
    run_git(
        &superproject,
        &[
            "-c",
            "protocol.file.allow=always",
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "submodule",
            "add",
            sub_src.to_str().unwrap(),
            "nested.git",
        ],
    );
    run_git(
        &superproject,
        &[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&superproject).expect("open superproject");
    let submodules = opened.list_submodules().expect("list submodules");

    let nested = submodules
        .iter()
        .find(|s| s.path == Path::new("nested.git"))
        .expect("submodule at nested.git");
    assert!(
        nested.checked_out_head.is_some(),
        "nested .git-suffixed submodule must open to report its checked-out head",
    );
}
