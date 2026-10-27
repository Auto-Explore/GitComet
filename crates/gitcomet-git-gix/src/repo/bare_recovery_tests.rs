use super::*;
use crate::repo::bare::tests::bare_fixture;
use crate::test_support::{git_stdout, git_success, open_repo};
use gitcomet_core::services::{GitBackend, GitRepository};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn interrupt(root: &Path, point: Failpoint) -> Journal {
    let repo = open_repo(root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    FAILPOINT.with(|active| active.set(Some(TestFault { point, crash: true })));
    let result = catch_unwind(AssertUnwindSafe(|| repo.disable_bare_mode(&preview)));
    FAILPOINT.with(|active| active.set(None));
    assert!(result.is_err(), "crash failpoint must unwind");
    let journal: Journal =
        serde_json::from_slice(&fs::read(journal_path(root).unwrap()).unwrap()).unwrap();
    let expected = match point {
        Failpoint::OriginalMoved => Phase::OriginalMoved,
        Failpoint::Installed | Failpoint::Repaired => Phase::Installed,
        Failpoint::Committed => Phase::Committed,
    };
    assert_eq!(journal.phase, expected);
    journal
}

#[test]
fn recovery_reopens_after_every_install_phase() {
    for point in [
        Failpoint::OriginalMoved,
        Failpoint::Installed,
        Failpoint::Repaired,
        Failpoint::Committed,
    ] {
        let (_directory, root) = bare_fixture();
        let before = tree_digest(&root).unwrap();
        let journal = interrupt(&root, point);
        assert!(journal.backup.path().exists());
        let repo = crate::GixBackend.open(&root).unwrap();
        if point == Failpoint::Committed {
            assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::WorkingTree);
            assert_eq!(
                fs::read_to_string(root.join("tracked.txt")).unwrap(),
                "original\n"
            );
        } else {
            assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::Bare);
            assert_eq!(tree_digest(&root).unwrap(), before);
        }
        assert!(!journal_path(&root).unwrap().exists());
        assert!(!journal.backup.path().exists());
        assert!(!journal.staging.path().exists());
    }
}

#[test]
fn recovery_waiting_for_a_live_conversion_respects_open_cancellation() {
    let (_directory, root) = bare_fixture();
    let before = tree_digest(&root).unwrap();
    let journal = interrupt(&root, Failpoint::OriginalMoved);
    let exclusive = ConversionLock::recover(&root, &CancellationToken::new()).unwrap();
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let worker_root = root.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = crate::GixBackend
            .open_cancellable(&worker_root, &worker_cancellation)
            .map(|_| ());
        tx.send(result).unwrap();
    });
    assert!(matches!(
        rx.recv_timeout(std::time::Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    cancellation.cancel();
    let result = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
    assert!(matches!(result.unwrap_err().kind(), ErrorKind::Cancelled));
    assert!(!root.exists());
    assert_eq!(tree_digest(&journal.backup.path()).unwrap(), before);
    assert!(journal.staging.path().exists());
    drop(exclusive);
    crate::GixBackend.open(&root).unwrap();
    assert_eq!(tree_digest(&root).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn mutable_lfs_and_annex_files_are_copied_while_git_objects_are_linked() {
    use std::os::unix::fs::MetadataExt;
    let (directory, root) = bare_fixture();
    for relative in ["lfs/objects/content", "annex/objects/content"] {
        fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
        fs::write(root.join(relative), "original\n").unwrap();
    }
    let destination = directory.path().join("copied");
    fs::create_dir(&destination).unwrap();
    copy_metadata(
        &root,
        &destination,
        Path::new(""),
        &CancellationToken::new(),
    )
    .unwrap();
    for relative in ["lfs/objects/content", "annex/objects/content", "config"] {
        assert_ne!(
            fs::metadata(root.join(relative)).unwrap().ino(),
            fs::metadata(destination.join(relative)).unwrap().ino()
        );
        let before = fs::read(root.join(relative)).unwrap();
        fs::write(destination.join(relative), "changed copy\n").unwrap();
        assert_eq!(fs::read(root.join(relative)).unwrap(), before);
    }
    let oid = git_stdout(&root, &["rev-parse", "HEAD"]);
    let object = PathBuf::from("objects").join(&oid[..2]).join(&oid[2..]);
    assert_eq!(
        fs::metadata(root.join(&object)).unwrap().ino(),
        fs::metadata(destination.join(object)).unwrap().ino()
    );
}

#[test]
fn ordinary_install_failures_roll_back_without_changing_source() {
    for point in [
        Failpoint::OriginalMoved,
        Failpoint::Installed,
        Failpoint::Repaired,
    ] {
        let (_directory, root) = bare_fixture();
        let before = tree_digest(&root).unwrap();
        let repo = open_repo(&root);
        let preview = repo.preview_disable_bare_mode().unwrap();
        FAILPOINT.with(|active| {
            active.set(Some(TestFault {
                point,
                crash: false,
            }))
        });
        let result = repo.disable_bare_mode(&preview);
        FAILPOINT.with(|active| active.set(None));
        assert!(
            matches!(result.unwrap_err().kind(), ErrorKind::Backend(message) if message.starts_with("conversion failpoint"))
        );
        assert_eq!(tree_digest(&root).unwrap(), before);
        assert!(!journal_path(&root).unwrap().exists());
        assert_eq!(
            crate::GixBackend
                .open(&root)
                .unwrap()
                .repository_mode()
                .unwrap(),
            RepositoryMode::Bare
        );
    }
}

#[test]
fn linked_checkout_open_recovers_when_original_root_is_absent() {
    let (_directory, root) = bare_fixture();
    let linked = root.parent().unwrap().join("linked");
    git_success(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let before_pointer = fs::read(linked.join(".git")).unwrap();
    let before_root = tree_digest(&root).unwrap();
    interrupt(&root, Failpoint::OriginalMoved);
    assert!(!root.exists());
    let repo = crate::GixBackend.open(&linked).unwrap();
    assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::WorkingTree);
    assert_eq!(fs::read(linked.join(".git")).unwrap(), before_pointer);
    assert_eq!(tree_digest(&root).unwrap(), before_root);
    assert_eq!(git_stdout(&linked, &["branch", "--show-current"]), "linked");
}

#[test]
fn recovery_restores_repaired_linked_pointer() {
    let (_directory, root) = bare_fixture();
    let linked = root.parent().unwrap().join("linked");
    git_success(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let before = fs::read(linked.join(".git")).unwrap();
    interrupt(&root, Failpoint::Repaired);
    assert_ne!(fs::read(linked.join(".git")).unwrap(), before);
    crate::GixBackend.open(&linked).unwrap();
    assert_eq!(fs::read(linked.join(".git")).unwrap(), before);
    assert_eq!(native_mode(&root).unwrap(), RepositoryMode::Bare);
}

#[test]
fn interrupted_rollback_restores_all_linked_pointers_after_original_root_is_back() {
    for restored_pointers in [0, 1] {
        let (directory, root) = bare_fixture();
        for name in ["first", "second"] {
            let linked = directory.path().join(name);
            git_success(
                &root,
                &["worktree", "add", "-b", name, linked.to_str().unwrap()],
            );
            fs::write(linked.join("tracked.txt"), format!("{name} dirty\n")).unwrap();
        }
        let before = tree_digest(&root).unwrap();
        let journal = interrupt(&root, Failpoint::Repaired);
        rename_directory(&root, &journal.staging.path()).unwrap();
        rename_directory(&journal.backup.path(), &root).unwrap();
        for linked in journal.linked.iter().take(restored_pointers) {
            fs::write(linked.root.path().join(".git"), &linked.original_pointer).unwrap();
        }
        assert!(!journal.backup.path().exists());
        assert!(journal_path(&root).unwrap().exists());
        let selected = journal.linked.last().unwrap().root.path();
        let repo = crate::GixBackend.open(&selected).unwrap();
        assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::WorkingTree);
        assert_eq!(tree_digest(&root).unwrap(), before);
        for linked in &journal.linked {
            assert_eq!(
                fs::read(linked.root.path().join(".git")).unwrap(),
                linked.original_pointer
            );
            assert_eq!(
                git_stdout(&linked.root.path(), &["status", "--porcelain=v1"]),
                "M tracked.txt"
            );
            let name = linked
                .root
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                fs::read_to_string(linked.root.path().join("tracked.txt")).unwrap(),
                format!("{name} dirty\n")
            );
        }
        assert!(!journal.staging.path().exists());
        assert!(!journal_path(&root).unwrap().exists());
        assert!(!trusted_journal_path(&root).unwrap().exists());
    }
}

#[test]
fn recovery_preserves_modified_installed_checkout_and_original_backup() {
    let (_directory, root) = bare_fixture();
    let journal = interrupt(&root, Failpoint::Installed);
    fs::write(root.join("tracked.txt"), "new user edit\n").unwrap();
    let error = match crate::GixBackend.open(&root) {
        Ok(_) => panic!("edited recovery must refuse"),
        Err(error) => error,
    };
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRecovery { .. }
    ));
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "new user edit\n"
    );
    assert_eq!(
        tree_digest(&journal.backup.path()).unwrap(),
        journal.source_digest
    );
    assert!(journal_path(&root).unwrap().exists());
    remove_journals(&root).unwrap();
}

#[test]
fn recovery_preserves_modified_backup_and_staging_copies() {
    for mutate_backup in [true, false] {
        let (_directory, root) = bare_fixture();
        let journal = interrupt(&root, Failpoint::OriginalMoved);
        let edited = if mutate_backup {
            journal.backup.path().join("config")
        } else {
            journal.staging.path().join("tracked.txt")
        };
        fs::write(&edited, "external edit\n").unwrap();
        let error = match crate::GixBackend.open(&root) {
            Ok(_) => panic!("edited recovery must refuse"),
            Err(error) => error,
        };
        assert!(matches!(
            error.kind(),
            ErrorKind::BareConversionRecovery { .. }
        ));
        assert!(!root.exists());
        assert!(journal.backup.path().exists());
        assert!(journal.staging.path().exists());
        assert_eq!(fs::read_to_string(edited).unwrap(), "external edit\n");
        remove_journals(&root).unwrap();
    }
}

#[test]
fn recovery_preserves_changed_linked_pointer() {
    let (_directory, root) = bare_fixture();
    let linked = root.parent().unwrap().join("linked");
    git_success(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let journal = interrupt(&root, Failpoint::Repaired);
    fs::write(linked.join(".git"), "gitdir: /different/repository\n").unwrap();
    let error = match crate::GixBackend.open(&root) {
        Ok(_) => panic!("changed linked pointer must refuse"),
        Err(error) => error,
    };
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRecovery { .. }
    ));
    assert_eq!(
        fs::read_to_string(linked.join(".git")).unwrap(),
        "gitdir: /different/repository\n"
    );
    assert!(journal.backup.path().exists());
    assert!(root.join(".git").is_dir());
    remove_journals(&root).unwrap();
}

#[test]
fn recovery_handles_rename_completed_before_journal_phase_update() {
    let (_directory, root) = bare_fixture();
    let before = tree_digest(&root).unwrap();
    let mut journal = interrupt(&root, Failpoint::Installed);
    journal.phase = Phase::Prepared;
    write_journal(&journal).unwrap();
    crate::GixBackend.open(&root).unwrap();
    assert_eq!(tree_digest(&root).unwrap(), before);
}

#[test]
fn recovery_rejects_unknown_fields_and_foreign_paths() {
    for foreign_path in [false, true] {
        let (_directory, root) = bare_fixture();
        let journal = interrupt(&root, Failpoint::Installed);
        let mut value = serde_json::to_value(&journal).unwrap();
        if foreign_path {
            value["backup"] =
                serde_json::to_value(StoredPath::from_path(Path::new("/unrelated"))).unwrap();
        } else {
            value["unexpected"] = serde_json::Value::Bool(true);
        }
        fs::write(
            journal_path(&root).unwrap(),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let error = match crate::GixBackend.open(&root) {
            Ok(_) => panic!("invalid journal must refuse"),
            Err(error) => error,
        };
        assert!(matches!(
            error.kind(),
            ErrorKind::BareConversionRecovery { .. }
        ));
        assert!(journal.backup.path().exists());
        assert!(root.join(".git").is_dir());
        remove_journals(&root).unwrap();
    }
}

#[test]
fn directory_install_never_overwrites_an_existing_destination() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let destination = directory.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("source.txt"), "source").unwrap();
    fs::write(destination.join("user.txt"), "user").unwrap();
    assert!(rename_directory(&source, &destination).is_err());
    assert_eq!(
        fs::read_to_string(source.join("source.txt")).unwrap(),
        "source"
    );
    assert_eq!(
        fs::read_to_string(destination.join("user.txt")).unwrap(),
        "user"
    );
}

#[test]
fn coordination_lock_is_shared_across_profiles_and_processes() {
    const CHILD_ROOT: &str = "GITCOMET_TEST_COORDINATION_ROOT";
    const CHILD_EXPECTATION: &str = "GITCOMET_TEST_COORDINATION_EXPECT_LOCKED";
    const TEST_NAME: &str =
        "repo::bare::storage::tests::coordination_lock_is_shared_across_profiles_and_processes";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let expected = match std::env::var(CHILD_EXPECTATION).unwrap().as_str() {
            "true" => true,
            "false" => false,
            value => panic!("invalid child expectation {value}"),
        };
        let result = OperationLock::acquire(Path::new(&root));
        if expected {
            assert!(
                matches!(result, Err(error) if matches!(error.kind(), ErrorKind::RepositoryChanged))
            );
        } else {
            assert!(result.is_ok());
        }
        return;
    }
    let (directory, root) = bare_fixture();
    let exclusive = ConversionLock::acquire(&root, &CancellationToken::new()).unwrap();
    let run_child = |expected: bool| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ROOT, &root)
            .env(CHILD_EXPECTATION, expected.to_string())
            .env(
                "GITCOMET_PROFILE_ROOT",
                directory.path().join("different-profile"),
            )
            .env("XDG_STATE_HOME", directory.path().join("different-xdg"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_child(true);
    drop(exclusive);
    run_child(false);
}

#[test]
fn old_handles_detect_conversion_performed_by_another_process() {
    const CHILD_ROOT: &str = "GITCOMET_TEST_CONVERSION_CHILD_ROOT";
    const TEST_NAME: &str =
        "repo::bare::storage::tests::old_handles_detect_conversion_performed_by_another_process";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let repo = crate::GixBackend.open(Path::new(&root)).unwrap();
        let preview = repo.preview_disable_bare_mode().unwrap();
        repo.disable_bare_mode(&preview).unwrap();
        return;
    }
    for config_only in [false, true] {
        let (directory, bare) = bare_fixture();
        let root = if config_only {
            let checkout = directory.path().join("checkout");
            git_success(&checkout, &["config", "core.bare", "true"]);
            checkout
        } else {
            bare
        };
        let before = tree_digest(&root).unwrap();
        let mut retained = Some(crate::GixBackend.open(&root).unwrap());
        let run_child = || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD_ROOT, &root)
                .output()
                .unwrap()
        };
        let mut output = run_child();
        if cfg!(windows) && !config_only {
            assert!(!output.status.success());
            assert_eq!(
                retained.as_ref().unwrap().repository_mode().unwrap(),
                RepositoryMode::Bare
            );
            assert_eq!(tree_digest(&root).unwrap(), before);
            drop(retained.take());
            output = run_child();
        }
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(retained) = retained {
            assert!(matches!(
                retained.repository_mode().unwrap_err().kind(),
                ErrorKind::RepositoryChanged
            ));
        }
        assert_eq!(
            crate::GixBackend
                .open(&root)
                .unwrap()
                .repository_mode()
                .unwrap(),
            RepositoryMode::WorkingTree
        );
    }
}

#[test]
fn recovery_never_trusts_a_sibling_journal_to_delete_another_directory() {
    for trusted_record in [false, true] {
        let (directory, root) = bare_fixture();
        let unrelated = directory.path().join(if trusted_record {
            "unrelated-user-data"
        } else {
            ".bare-backup-AbC123"
        });
        let staging = directory.path().join(".bare-checkout-DeF456");
        fs::create_dir(&unrelated).unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(unrelated.join("important.txt"), "never delete this\n").unwrap();
        let journal = Journal {
            version: JOURNAL_VERSION,
            root: StoredPath::from_path(&root),
            staging: StoredPath::from_path(&staging),
            backup: StoredPath::from_path(&unrelated),
            original_identity: DirectoryIdentity::read(&unrelated).unwrap(),
            installed_identity: DirectoryIdentity::read(&root).unwrap(),
            source_digest: tree_digest(&unrelated).unwrap(),
            installed_digest: tree_digest(&root).unwrap(),
            phase: Phase::Committed,
            linked: Vec::new(),
        };
        if trusted_record {
            write_journal(&journal).unwrap();
        } else {
            fs::write(
                journal_path(&root).unwrap(),
                serde_json::to_vec(&journal).unwrap(),
            )
            .unwrap();
        }
        let error = match crate::GixBackend.open(&root) {
            Ok(_) => panic!("unrelated directory must be preserved"),
            Err(error) => error,
        };
        assert!(matches!(
            error.kind(),
            ErrorKind::BareConversionRecovery { .. }
        ));
        assert_eq!(
            fs::read_to_string(unrelated.join("important.txt")).unwrap(),
            "never delete this\n"
        );
        assert!(staging.exists());
        assert_eq!(native_mode(&root).unwrap(), RepositoryMode::Bare);
        remove_journals(&root).unwrap();
    }
}
