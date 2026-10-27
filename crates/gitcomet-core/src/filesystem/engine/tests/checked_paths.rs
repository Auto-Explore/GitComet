use super::*;
use std::os::unix::fs::symlink;

struct AliasedDirectory {
    directory: tempfile::TempDir,
    admitted: PathBuf,
    bare: PathBuf,
    alias: PathBuf,
}

impl AliasedDirectory {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = canonical_path(directory.path()).unwrap();
        let admitted = root.join("admitted");
        let bare = root.join("bare");
        let alias = root.join("alias");
        fs::create_dir(&admitted).unwrap();
        let output = crate::test_support::git_fixture::git_command()
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "")
            .args([
                "init",
                "--bare",
                "--quiet",
                "--initial-branch=main",
                "--template=",
            ])
            .arg(&bare)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        symlink(&admitted, &alias).unwrap();
        Self {
            directory,
            admitted,
            bare,
            alias,
        }
    }

    fn redirect(&self) {
        fs::remove_file(&self.alias).unwrap();
        symlink(&self.bare, &self.alias).unwrap();
    }

    fn file_in_both(&self) {
        fs::write(self.admitted.join("file"), b"admitted contents").unwrap();
        fs::write(self.bare.join("file"), b"bare contents").unwrap();
    }
}

#[test]
fn transfer_destination_stays_admitted_when_its_alias_changes_inside_the_check() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let fixture = AliasedDirectory::new();
        let source = fixture.directory.path().join("file");
        fs::write(&source, b"transferred contents").unwrap();
        let mut service = Filesystem::default();
        let result = service.execute_checked(
            Request::new(Operation::Transfer {
                sources: vec![source.clone()],
                destination: fixture.alias.clone(),
                intent,
            }),
            |paths| {
                assert!(paths.contains(&fixture.admitted));
                fixture.redirect();
                Ok(())
            },
            |_| {},
        );
        success(&result);
        assert!(!fixture.bare.join("file").exists());
        assert_eq!(
            fs::read(fixture.admitted.join("file")).unwrap(),
            b"transferred contents"
        );
        assert_eq!(source.exists(), intent == TransferIntent::Copy);
    }
}

#[test]
fn transfer_sources_stay_admitted_when_their_parent_alias_changes_inside_the_check() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let fixture = AliasedDirectory::new();
        fixture.file_in_both();
        let destination = fixture.directory.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let mut service = Filesystem::default();
        let result = service.execute_checked(
            Request::new(Operation::Transfer {
                sources: vec![fixture.alias.join("file")],
                destination: destination.clone(),
                intent,
            }),
            |_| {
                fixture.redirect();
                Ok(())
            },
            |_| {},
        );
        success(&result);
        assert_eq!(
            fs::read(fixture.bare.join("file")).unwrap(),
            b"bare contents"
        );
        assert_eq!(
            fs::read(destination.join("file")).unwrap(),
            b"admitted contents"
        );
        assert_eq!(
            fixture.admitted.join("file").exists(),
            intent == TransferIntent::Copy
        );
    }
}

#[test]
fn created_entries_stay_admitted_when_their_parent_alias_changes_inside_the_check() {
    for directory in [false, true] {
        let fixture = AliasedDirectory::new();
        let path = fixture.alias.join("created");
        let operation = if directory {
            Operation::CreateDirectory { path }
        } else {
            Operation::CreateFile { path }
        };
        let mut service = Filesystem::default();
        let result = service.execute_checked(
            Request::new(operation),
            |paths| {
                assert_eq!(paths, &[fixture.admitted.join("created")]);
                fixture.redirect();
                Ok(())
            },
            |_| {},
        );
        success(&result);
        assert!(!fixture.bare.join("created").exists());
        assert_eq!(fixture.admitted.join("created").is_dir(), directory);
        assert_eq!(fixture.admitted.join("created").is_file(), !directory);
    }
}

#[test]
fn rename_stays_admitted_when_the_source_parent_alias_changes_inside_the_check() {
    let fixture = AliasedDirectory::new();
    fixture.file_in_both();
    let mut service = Filesystem::default();
    let result = service.execute_checked(
        Request::new(Operation::Rename {
            source: fixture.alias.join("file"),
            name: "renamed".into(),
        }),
        |_| {
            fixture.redirect();
            Ok(())
        },
        |_| {},
    );
    success(&result);
    assert_eq!(
        fs::read(fixture.bare.join("file")).unwrap(),
        b"bare contents"
    );
    assert!(!fixture.bare.join("renamed").exists());
    assert!(!fixture.admitted.join("file").exists());
    assert_eq!(
        fs::read(fixture.admitted.join("renamed")).unwrap(),
        b"admitted contents"
    );
}

#[test]
fn duplicate_stays_admitted_when_the_source_parent_alias_changes_inside_the_check() {
    let fixture = AliasedDirectory::new();
    fixture.file_in_both();
    let mut service = Filesystem::default();
    let result = service.execute_checked(
        Request::new(Operation::Duplicate {
            sources: vec![fixture.alias.join("file")],
        }),
        |_| {
            fixture.redirect();
            Ok(())
        },
        |_| {},
    );
    success(&result);
    assert!(!fixture.bare.join("file copy").exists());
    assert_eq!(
        fs::read(fixture.bare.join("file")).unwrap(),
        b"bare contents"
    );
    assert_eq!(
        fs::read(fixture.admitted.join("file copy")).unwrap(),
        b"admitted contents"
    );
}

#[test]
fn permanent_delete_stays_admitted_when_the_source_parent_alias_changes_inside_the_check() {
    let fixture = AliasedDirectory::new();
    fixture.file_in_both();
    let mut service = Filesystem::default();
    let result = service.execute_checked(
        Request::new(Operation::DeletePermanently {
            sources: vec![fixture.alias.join("file")],
            confirmed: true,
        }),
        |_| {
            fixture.redirect();
            Ok(())
        },
        |_| {},
    );
    success(&result);
    assert_eq!(
        fs::read(fixture.bare.join("file")).unwrap(),
        b"bare contents"
    );
    assert!(!fixture.admitted.join("file").exists());
}

#[test]
fn trash_job_keeps_the_admitted_source_when_its_parent_alias_changes_inside_the_check() {
    let fixture = AliasedDirectory::new();
    fixture.file_in_both();
    let request = Request::new(Operation::Trash {
        sources: vec![fixture.alias.join("file")],
    });
    let cancellation = request.cancellation.clone();
    let mut service = Filesystem::default();
    let mut observed_source = false;
    let result = service.execute_checked(
        request,
        |_| {
            fixture.redirect();
            Ok(())
        },
        |progress| {
            if progress.completed_items == 0 {
                assert_eq!(progress.current_path, fixture.admitted.join("file"));
                observed_source = true;
                cancellation.cancel();
            }
        },
    );
    assert!(observed_source);
    assert!(matches!(result.items[0].outcome, ItemOutcome::Cancelled));
    assert_eq!(
        fs::read(fixture.bare.join("file")).unwrap(),
        b"bare contents"
    );
    assert_eq!(
        fs::read(fixture.admitted.join("file")).unwrap(),
        b"admitted contents"
    );
}

#[test]
fn saves_keep_the_admitted_target_and_worktree_when_aliases_change_inside_the_check() {
    for repository in [false, true] {
        let fixture = AliasedDirectory::new();
        fixture.file_in_both();
        let mut service = Filesystem::default();
        let result = service.execute_checked(
            Request::new(Operation::Save {
                path: fixture.alias.join("file"),
                worktree: repository.then(|| fixture.alias.clone()),
                contents: Arc::from(b"replacement".as_slice()),
                expected: None,
                overwrite: true,
            }),
            |_| {
                fixture.redirect();
                Ok(())
            },
            |_| {},
        );
        success(&result);
        assert_eq!(
            fs::read(fixture.bare.join("file")).unwrap(),
            b"bare contents"
        );
        assert_eq!(
            fs::read(fixture.admitted.join("file")).unwrap(),
            b"replacement"
        );
    }
}

#[test]
fn explicit_save_recreates_missing_parents_under_the_admitted_alias_target() {
    let fixture = AliasedDirectory::new();
    fs::create_dir(fixture.admitted.join("existing")).unwrap();
    fs::create_dir(fixture.bare.join("existing")).unwrap();
    let mut service = Filesystem::default();
    let result = service.execute_checked(
        Request::new(Operation::Save {
            path: fixture.alias.join("existing/deleted/file"),
            worktree: None,
            contents: Arc::from(b"replacement".as_slice()),
            expected: None,
            overwrite: true,
        }),
        |paths| {
            assert_eq!(paths, &[fixture.admitted.join("existing/deleted/file")]);
            fixture.redirect();
            Ok(())
        },
        |_| {},
    );
    success(&result);
    assert!(!fixture.bare.join("existing/deleted").exists());
    assert_eq!(
        fs::read(fixture.admitted.join("existing/deleted/file")).unwrap(),
        b"replacement"
    );
}

#[test]
fn outbound_move_removes_the_admitted_receipt_path_when_its_alias_changes_inside_the_check() {
    let fixture = AliasedDirectory::new();
    let original = fixture.admitted.join("file");
    fs::write(&original, b"shared bytes").unwrap();
    fs::hard_link(&original, fixture.bare.join("file")).unwrap();
    let receipt = OutboundReceipt {
        id: OperationId::allocate(),
        paths: vec![fixture.alias.join("file")],
        directories: vec![false],
        versions: vec![DiskVersion::read(&original).unwrap()],
    };
    let mut service = Filesystem::default();
    let result = service.execute_checked(
        Request::new(Operation::CompleteOutbound {
            receipt,
            intent: Some(TransferIntent::Move),
            source_removed: false,
        }),
        |_| {
            fixture.redirect();
            Ok(())
        },
        |_| {},
    );
    success(&result);
    assert_eq!(
        fs::read(fixture.bare.join("file")).unwrap(),
        b"shared bytes"
    );
    assert!(!original.exists());
}

#[test]
fn checked_save_keeps_explicit_recreation_when_the_worktree_root_was_removed() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical_path(directory.path()).unwrap().join("removed");
    let path = root.join("nested/file");
    let mut service = Filesystem::default();
    let result = service.execute_checked(
        Request::new(Operation::Save {
            path: path.clone(),
            worktree: Some(root.clone()),
            contents: Arc::from(b"retained contents".as_slice()),
            expected: None,
            overwrite: true,
        }),
        |paths| {
            assert_eq!(paths, std::slice::from_ref(&path));
            assert!(!root.exists());
            Ok(())
        },
        |_| {},
    );
    success(&result);
    assert_eq!(fs::read(path).unwrap(), b"retained contents");
}

#[test]
fn checked_repository_save_preserves_its_boundary_and_rejects_child_aliases() {
    let fixture = AliasedDirectory::new();
    fixture.file_in_both();
    symlink(&fixture.bare, fixture.admitted.join("child")).unwrap();
    let mut service = Filesystem::default();
    for path in [
        fixture.bare.join("file"),
        fixture.admitted.join("child/file"),
    ] {
        let result = service.execute_checked(
            Request::new(Operation::Save {
                path,
                worktree: Some(fixture.admitted.clone()),
                contents: Arc::from(b"must not be saved".as_slice()),
                expected: None,
                overwrite: true,
            }),
            |_| Ok(()),
            |_| {},
        );
        assert!(!result.succeeded());
        assert_eq!(
            fs::read(fixture.bare.join("file")).unwrap(),
            b"bare contents"
        );
        assert_eq!(
            fs::read(fixture.admitted.join("file")).unwrap(),
            b"admitted contents"
        );
    }
}
