#[cfg(unix)]
use super::append_gitignore_patterns_in_workdir;
use super::resolve_worktree_save_target;
use gitcomet_core::error::ErrorKind;
use std::path::Path;

#[test]
fn markdown_checkbox_saves_accept_the_read_baseline_and_reject_intervening_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    let mut filesystem = gitcomet_core::filesystem::Filesystem::default();
    let unchecked = b"- [ ] ship it\n";
    let checked = b"- [x] ship it\n";
    std::fs::write(&path, unchecked).unwrap();
    super::guarded_worktree_save(&mut filesystem, &path, checked, Some(unchecked)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), checked);
    super::guarded_worktree_save(&mut filesystem, &path, unchecked, Some(checked)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), unchecked);
    std::fs::write(&path, b"external edits\n").unwrap();
    assert!(
        super::guarded_worktree_save(&mut filesystem, &path, checked, Some(unchecked)).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"external edits\n");
}

#[test]
fn merge_save_preserves_external_edits_and_does_not_recreate_a_moved_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflict.txt");
    let mut filesystem = gitcomet_core::filesystem::Filesystem::default();
    std::fs::write(&path, b"external edit").unwrap();
    assert!(
        super::guarded_worktree_save(&mut filesystem, &path, b"resolved", Some(b"markers"))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    std::fs::rename(&path, dir.path().join("renamed.txt")).unwrap();
    assert!(
        super::guarded_worktree_save(&mut filesystem, &path, b"resolved", Some(b"external edit"))
            .is_err()
    );
    assert!(!path.exists());
    super::guarded_worktree_save(
        &mut filesystem,
        &dir.path().join("renamed.txt"),
        b"resolved",
        Some(b"external edit"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("renamed.txt")).unwrap(),
        b"resolved"
    );
}

#[test]
fn git_metadata_paths_are_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    for path in [".git/hooks/pre-commit", "a/../.git/config", "./.GIT/config"] {
        assert!(
            resolve_worktree_save_target(dir.path(), Path::new(path)).is_err(),
            "{path:?} must be rejected"
        );
    }
}

#[cfg(unix)]
#[test]
fn gitignore_append_through_symlink_out_of_workdir_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("victim");
    std::fs::write(&victim, "original\n").expect("write victim");
    std::os::unix::fs::symlink(&victim, dir.path().join(".gitignore")).expect("symlink");

    let err = append_gitignore_patterns_in_workdir(
        &std::sync::Mutex::default(),
        dir.path(),
        &["secret".to_string()],
    )
    .expect_err("symlinked .gitignore");
    let ErrorKind::Backend(message) = err.kind() else {
        panic!("expected a backend refusal, got {err:?}");
    };
    assert!(message.contains("symlink"), "{message}");
    assert_eq!(
        std::fs::read_to_string(&victim).expect("victim"),
        "original\n"
    );
}

#[test]
fn nested_relative_path_resolves_under_workdir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (relative, full) =
        resolve_worktree_save_target(dir.path(), Path::new("./src/../src/lib.rs"))
            .expect("nested path");
    assert_eq!(relative, Path::new("src/lib.rs"));
    assert_eq!(full, dir.path().join("src/lib.rs"));
}

#[test]
fn parent_dir_escape_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = resolve_worktree_save_target(dir.path(), Path::new("../outside.txt"))
        .expect_err("lexical escape");
    assert!(matches!(err.kind(), ErrorKind::Backend(_)));
}

#[cfg(unix)]
#[test]
fn save_through_symlink_out_of_workdir_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside dir");
    let victim = outside.path().join("authorized_keys");
    std::fs::write(&victim, "original").expect("write victim");
    std::os::unix::fs::symlink(&victim, dir.path().join("notes.md")).expect("symlink");

    let err = resolve_worktree_save_target(dir.path(), Path::new("notes.md"))
        .expect_err("symlinked file");
    let ErrorKind::Backend(message) = err.kind() else {
        panic!("expected a backend refusal, got {err:?}");
    };
    assert!(message.contains("symlink"), "{message}");
    assert_eq!(
        std::fs::read_to_string(&victim).expect("victim"),
        "original"
    );
}
