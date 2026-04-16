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
