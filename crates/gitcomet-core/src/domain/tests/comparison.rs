use super::super::*;
use std::path::Path;

fn commit(id: &str) -> CommitId {
    CommitId(id.into())
}

#[test]
fn targets_equal_whether_or_not_they_carry_the_derived_rename_source() {
    let bare = DiffTarget::commit(commit("c"), PathBuf::from("new.rs"));
    let renamed = bare.clone().with_old_path(Some(PathBuf::from("old.rs")));
    assert_eq!(renamed.old_file_path(), Some(Path::new("old.rs")));
    assert_eq!(bare, renamed);
    assert_ne!(
        bare,
        DiffTarget::commit(commit("c"), PathBuf::from("other.rs"))
    );
    assert_ne!(
        DiffTarget::commit_range(commit("a"), None, None),
        DiffTarget::commit_range(commit("a"), Some(commit("b")), None)
    );
}

#[test]
fn a_rename_source_equal_to_the_path_is_no_rename() {
    let target = DiffTarget::commit_range(commit("a"), None, Some(PathBuf::from("same.rs")))
        .with_old_path(Some(PathBuf::from("same.rs")));
    assert_eq!(target.old_file_path(), None);
    let change = CommitFileChange::new(PathBuf::from("same.rs"), FileStatusKind::Modified)
        .with_old_path(Some(PathBuf::from("same.rs")));
    assert_eq!(change.old_path, None);
    // Working-tree targets have no rename source.
    let worktree = DiffTarget::working_tree(PathBuf::from("a.rs"), DiffArea::Unstaged)
        .with_old_path(Some(PathBuf::from("b.rs")));
    assert_eq!(worktree.old_file_path(), None);
}

#[test]
fn a_change_list_entry_becomes_its_files_target() {
    let change = CommitFileChange::new(PathBuf::from("new.rs"), FileStatusKind::Renamed)
        .with_old_path(Some(PathBuf::from("old.rs")));
    let target = DiffTarget::commit_range(commit("a"), Some(commit("b")), None).for_change(&change);
    assert_eq!(target.file_path(), Some(Path::new("new.rs")));
    assert_eq!(target.old_file_path(), Some(Path::new("old.rs")));
    let DiffTarget::CommitRange {
        from_commit_id,
        to_commit_id,
        ..
    } = &target
    else {
        panic!("a range target stays a range");
    };
    assert_eq!(
        (from_commit_id, to_commit_id),
        (&commit("a"), &Some(commit("b")))
    );
}

#[test]
fn modes_and_ids_parse_from_git_and_identify_mode_only_changes() {
    assert_eq!(FileMode::from_octal(b"100644"), Some(FileMode::Regular));
    assert_eq!(FileMode::from_octal(b"100755"), Some(FileMode::Executable));
    assert_eq!(FileMode::from_octal(b"120000"), Some(FileMode::Symlink));
    assert_eq!(FileMode::from_octal(b"160000"), Some(FileMode::Gitlink));
    assert_eq!(FileMode::from_octal(b"000000"), None);
    assert_eq!(FileMode::from_octal(b"040000"), None);
    assert_eq!(ObjectHash::from_hex(&"0".repeat(40)), None);
    let blob = ObjectHash::from_hex("abc123");
    assert!(blob.is_some());

    let chmod = CommitFileChange::new(PathBuf::from("run.sh"), FileStatusKind::Modified)
        .with_ids(blob.clone(), blob.clone())
        .with_modes(Some(FileMode::Regular), Some(FileMode::Executable));
    assert!(chmod.is_mode_change_only());
    let edit = chmod.clone().with_ids(blob, ObjectHash::from_hex("def456"));
    assert!(!edit.is_mode_change_only());
}
