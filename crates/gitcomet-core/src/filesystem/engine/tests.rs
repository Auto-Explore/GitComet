use super::*;
use std::ffi::OsStr;

fn run(fs: &mut Filesystem, operation: Operation) -> OperationResult {
    fs.execute(Request::new(operation), |_| {})
}

fn success(result: &OperationResult) {
    assert!(result.succeeded(), "{:#?}", result.items);
}

#[test]
fn public_filesystem_paths_match_repository_identities_through_save_move_and_undo() {
    let directory = tempfile::tempdir().unwrap();
    // canonicalize supplies verbatim-prefixed input on Windows, while the
    // backend publishes repository workdirs without that prefix.
    let canonical = fs::canonicalize(directory.path()).unwrap();
    let worktree = crate::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let source = worktree.join("note.txt");
    fs::write(&source, b"contents").unwrap();
    fs::create_dir(worktree.join("folder")).unwrap();
    let identity = DocumentIdentity::resolve(&canonical.join("note.txt")).unwrap();
    assert_eq!(identity.0, source);
    assert_eq!(
        identity.0.strip_prefix(&worktree).unwrap(),
        Path::new("note.txt")
    );
    assert_eq!(
        absolute_identity(&canonical.join("new.txt")).unwrap(),
        worktree.join("new.txt")
    );
    let mut service = Filesystem::default();
    let saved = run(
        &mut service,
        Operation::Save {
            path: canonical.join("note.txt"),
            worktree: None,
            contents: Arc::from(&b"saved contents"[..]),
            expected: Some(DiskVersion::read(&source).unwrap()),
            overwrite: false,
        },
    );
    success(&saved);
    assert_eq!(saved.changes[0].new.as_ref(), Some(&source));
    let moved = run(
        &mut service,
        Operation::Transfer {
            sources: vec![canonical.join("note.txt")],
            destination: canonical.join("folder"),
            intent: TransferIntent::Move,
        },
    );
    success(&moved);
    let destination = worktree.join("folder/note.txt");
    assert_eq!(moved.changes[0].old.as_ref(), Some(&source));
    assert_eq!(moved.changes[0].new.as_ref(), Some(&destination));
    assert_eq!(identity.retarget(&moved.changes).0, destination);
    assert!(moved.moved_versions.contains_key(&destination));
    let undone = run(&mut service, Operation::Undo);
    success(&undone);
    assert_eq!(undone.changes[0].new.as_ref(), Some(&source));
    assert_eq!(fs::read(&source).unwrap(), b"saved contents");
    success(&run(&mut service, Operation::Redo));
    assert_eq!(fs::read(destination).unwrap(), b"saved contents");
    let created = worktree.join("created.txt");
    success(&run(
        &mut service,
        Operation::CreateFile {
            path: canonical.join("created.txt"),
        },
    ));
    let entry = service.undo.back().unwrap();
    for step in &entry.steps {
        assert_eq!(absolute_identity(&step.from).unwrap(), step.from);
        assert_eq!(absolute_identity(&step.to).unwrap(), step.to);
    }
    success(&run(&mut service, Operation::Undo));
    assert!(!created.exists());
    success(&run(&mut service, Operation::Redo));
    assert!(created.is_file());
}

#[cfg(unix)]
#[test]
fn symlinked_trash_ancestors_preserve_receipts_through_undo_and_redo() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical_path(directory.path()).unwrap();
    let data = root.join("data");
    let alias = root.join("alias");
    fs::create_dir(&data).unwrap();
    std::os::unix::fs::symlink(&data, &alias).unwrap();
    let source = root.join("notes.txt");
    fs::write(&source, b"restore these contents").unwrap();
    for trash_root in [alias.join("Trash"), alias.join("nested/share/Trash")] {
        fs::create_dir_all(trash_root.parent().unwrap()).unwrap();
        let receipt = super::super::trash::prepare_in(&source, &trash_root).unwrap();
        assert_eq!(receipt.item, absolute_identity(&receipt.item).unwrap());
        assert_eq!(receipt.info, absolute_identity(&receipt.info).unwrap());
        assert!(receipt.item.starts_with(&data));
        let info = fs::read(&receipt.info).unwrap();
        let request = Request::new(Operation::Trash {
            sources: vec![source.clone()],
        });
        let mut journal = JournalEntry::default();
        assert!(matches!(
            trash_with_receipt(&source, &receipt, &request, &mut journal).unwrap(),
            ItemOutcome::Completed
        ));
        assert!(!source.exists());
        let mut service = Filesystem::default();
        service.undo.push_back(journal);
        for _ in 0..2 {
            success(&run(&mut service, Operation::Undo));
            assert_eq!(fs::read(&source).unwrap(), b"restore these contents");
            assert!(!receipt.item.exists() && !receipt.info.exists());
            success(&run(&mut service, Operation::Redo));
            assert!(!source.exists());
            assert_eq!(fs::read(&receipt.item).unwrap(), b"restore these contents");
            assert_eq!(fs::read(&receipt.info).unwrap(), info);
        }
        success(&run(&mut service, Operation::Undo));
    }
}

#[test]
fn cancelling_during_a_collision_hash_stops_reading_without_a_conflict_or_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical_path(directory.path()).unwrap();
    let destination = root.join("destination");
    fs::create_dir(&destination).unwrap();
    for is_directory in [false, true] {
        let source = root.join(if is_directory { "folder" } else { "file" });
        let target = destination.join(source.file_name().unwrap());
        let (source_file, target_file) = if is_directory {
            fs::create_dir(&source).unwrap();
            fs::create_dir(&target).unwrap();
            (source.join("child"), target.join("child"))
        } else {
            (source.clone(), target.clone())
        };
        let payload = vec![b'x'; 256 * 1024];
        fs::write(&source_file, b"source contents").unwrap();
        fs::write(&target_file, &payload).unwrap();
        for intent in [TransferIntent::Copy, TransferIntent::Move] {
            let request = Request::new(Operation::Transfer {
                sources: vec![source.clone()],
                destination: destination.clone(),
                intent,
            });
            CONTENT_BYTES_HASHED.set(0);
            CANCEL_DURING_HASH.set(Some((target_file.clone(), request.cancellation.clone())));
            let mut service = Filesystem::default();
            let result = service.execute(request, |_| {});
            assert!(
                CANCEL_DURING_HASH.take().is_none(),
                "the request must reach the collision read"
            );
            assert!(
                matches!(result.items[0].outcome, ItemOutcome::Cancelled),
                "{:?}",
                result.items
            );
            assert!(
                CONTENT_BYTES_HASHED.get() < payload.len() as u64,
                "cancellation must interrupt hashing"
            );
            assert!(result.changes.is_empty() && !result.undo_available);
            assert_eq!(fs::read(&source_file).unwrap(), b"source contents");
            assert_eq!(fs::read(&target_file).unwrap(), payload);
        }
    }
}

#[test]
fn collecting_moved_editor_versions_stops_mid_file_when_cancelled() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("moved.txt");
    fs::write(&path, vec![b'x'; 256 * 1024]).unwrap();
    let cancellation = Cancellation::default();
    CONTENT_BYTES_HASHED.set(0);
    CANCEL_DURING_HASH.set(Some((path.clone(), cancellation.clone())));
    let versions = moved_versions(
        &[PathChange {
            old: Some(directory.path().join("old.txt")),
            new: Some(path),
        }],
        &cancellation,
    );
    assert!(CANCEL_DURING_HASH.take().is_none());
    assert!(versions.is_empty());
    assert!(CONTENT_BYTES_HASHED.get() < 256 * 1024);
}

#[test]
fn retargeted_file_paths_can_be_read_without_a_trailing_directory_separator() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("before.txt");
    let destination = directory.path().join("after.txt");
    fs::write(&destination, b"moved contents").unwrap();
    let change = PathChange {
        old: Some(source.clone()),
        new: Some(destination.clone()),
    };
    let retargeted = change.retarget(&source).unwrap();
    assert_eq!(retargeted.as_os_str(), destination.as_os_str());
    assert_eq!(fs::read(retargeted).unwrap(), b"moved contents");
}

#[test]
fn repository_saves_require_a_path_inside_the_worktree() {
    let directory = tempfile::tempdir().unwrap();
    let worktree = directory.path().join("repo");
    fs::create_dir(&worktree).unwrap();
    let outside = directory.path().join("outside.txt");
    fs::write(&outside, b"external contents").unwrap();
    let mut service = Filesystem::default();
    for path in [outside.clone(), worktree.join("../outside.txt")] {
        let result = run(
            &mut service,
            Operation::Save {
                path,
                worktree: Some(worktree.clone()),
                contents: Arc::from(&b"replacement"[..]),
                expected: DiskVersion::read(&outside).ok(),
                overwrite: true,
            },
        );
        assert!(!result.succeeded());
        assert!(result.saved_version.is_none());
        assert_eq!(fs::read(&outside).unwrap(), b"external contents");
    }
    let path = worktree.join("inside.txt");
    success(&run(
        &mut service,
        Operation::Save {
            path: path.clone(),
            worktree: Some(worktree),
            contents: Arc::from(&b"allowed"[..]),
            expected: None,
            overwrite: false,
        },
    ));
    assert_eq!(fs::read(path).unwrap(), b"allowed");
}

#[cfg(unix)]
#[test]
fn repository_saves_recheck_symlinks_when_the_queued_request_executes() {
    let directory = tempfile::tempdir().unwrap();
    let worktree = directory.path().join("repo");
    let parent = worktree.join("folder");
    let outside = directory.path().join("outside");
    fs::create_dir_all(&parent).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(parent.join("file.txt"), b"original").unwrap();
    fs::write(outside.join("file.txt"), b"external contents").unwrap();
    let path = parent.join("file.txt");
    let mut request = Request::new(Operation::Save {
        path: path.clone(),
        worktree: Some(worktree.clone()),
        contents: Arc::from(&b"replacement"[..]),
        expected: DiskVersion::read(&path).ok(),
        overwrite: true,
    });
    fs::rename(&parent, worktree.join("original-folder")).unwrap();
    std::os::unix::fs::symlink(&outside, &parent).unwrap();
    let mut service = Filesystem::default();
    // Even an explicit Replace must enforce the boundary at execution time.
    assert!(!service.execute(request.clone(), |_| {}).succeeded());
    // A historical-tree editor can have loaded this external file already,
    // so a matching disk baseline alone cannot protect the worktree boundary.
    if let Operation::Save {
        expected,
        overwrite,
        ..
    } = &mut request.operation
    {
        *expected = Some(DiskVersion::read(&path).unwrap());
        *overwrite = false;
    }
    assert!(!service.execute(request.clone(), |_| {}).succeeded());
    assert_eq!(fs::read(&path).unwrap(), b"external contents");
    assert_eq!(
        fs::read(worktree.join("original-folder/file.txt")).unwrap(),
        b"original"
    );

    // Standalone documents intentionally accept arbitrary absolute paths.
    if let Operation::Save { worktree, .. } = &mut request.operation {
        *worktree = None;
    }
    success(&service.execute(request, |_| {}));
    assert_eq!(fs::read(outside.join("file.txt")).unwrap(), b"replacement");
}

#[cfg(windows)]
#[test]
fn exclusive_rename_preserves_identity_and_existing_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let target = directory.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"existing").unwrap();
    let identity = entry_identity(&source).unwrap();
    assert_eq!(
        rename_exclusive(&source, &target).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(fs::read(&target).unwrap(), b"existing");
    let renamed = directory.path().join("renamed");
    rename_exclusive(&source, &renamed).unwrap();
    assert!(!source.exists());
    assert_eq!(entry_identity(&renamed).unwrap(), identity);
    rename_exclusive(&renamed, &source).unwrap();
    assert_eq!(entry_identity(&source).unwrap(), identity);
}

#[cfg(windows)]
#[test]
fn cross_volume_windows_moves_use_staged_copy_and_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    fs::write(&source, b"source").unwrap();
    let identity = entry_identity(&source).unwrap();
    // Windows CI may expose a second data volume. An explicit writable
    // directory also lets developer/CI setups exercise mounted volumes.
    let candidates = std::env::var_os("GITCOMET_TEST_SECOND_VOLUME")
        .map(PathBuf::from)
        .into_iter()
        .chain(('A'..='Z').map(|drive| PathBuf::from(format!("{drive}:\\"))));
    for root in candidates {
        let Ok(other) = tempfile::tempdir_in(root) else {
            continue;
        };
        if entry_identity(other.path()).unwrap().0 == identity.0 {
            continue;
        }
        let target = other.path().join("source");
        assert_eq!(
            rename_exclusive(&source, &target).unwrap_err().kind(),
            io::ErrorKind::CrossesDevices
        );
        assert_eq!(entry_identity(&source).unwrap(), identity);
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert!(!target.exists());
        let mut service = Filesystem::default();
        success(&run(
            &mut service,
            Operation::Transfer {
                sources: vec![source.clone()],
                destination: other.path().to_path_buf(),
                intent: TransferIntent::Move,
            },
        ));
        assert!(!source.exists());
        assert_eq!(fs::read(&target).unwrap(), b"source");
        success(&run(&mut service, Operation::Undo));
        assert_eq!(entry_identity(&source).unwrap(), identity);
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert!(!target.exists());
        success(&run(&mut service, Operation::Redo));
        assert!(!source.exists());
        assert_eq!(fs::read(&target).unwrap(), b"source");
        return;
    }
    eprintln!("No writable second Windows volume; cross-volume assertion skipped");
}

#[cfg(unix)]
#[test]
fn symlinked_recovery_directory_supports_immediate_undo_and_redo() {
    let directory = tempfile::tempdir().unwrap();
    let storage = directory.path().join("storage");
    let alias = directory.path().join("alias");
    fs::create_dir(&storage).unwrap();
    std::os::unix::fs::symlink(&storage, &alias).unwrap();
    let mut journal = JournalEntry::new(
        &Arc::new(StoragePolicy::with_candidates(vec![alias.clone()])),
        None,
    );
    let staged = journal.reserve(directory.path()).unwrap();
    assert!(staged.starts_with(fs::canonicalize(&storage).unwrap()));
    assert_eq!(staged, absolute_identity(&staged).unwrap());
    let file = fs::canonicalize(directory.path())
        .unwrap()
        .join("created.txt");
    fs::write(&staged, b"created").unwrap();
    journal.move_entry(staged, file.clone(), None).unwrap();
    let mut service = Filesystem::default();
    service.undo.push_back(journal);
    success(&run(&mut service, Operation::Undo));
    assert!(!file.exists());
    success(&run(&mut service, Operation::Redo));
    assert_eq!(fs::read(file).unwrap(), b"created");
}

#[test]
fn recovery_content_validation_ignores_only_service_owned_ancestors() {
    let directory = tempfile::tempdir().unwrap();
    let recovery = directory.path().join(".git/recovery/item");
    fs::create_dir_all(&recovery).unwrap();
    fs::write(recovery.join("ordinary.txt"), b"data").unwrap();
    let cancellation = Cancellation::default();
    assert!(protect(&recovery, true, &cancellation).is_err());
    protect_contents(&recovery, true, &cancellation).unwrap();
    for metadata in [".git", ".GIT"] {
        fs::write(recovery.join(metadata), b"gitdir: elsewhere").unwrap();
        assert!(protect_contents(&recovery, true, &cancellation).is_err());
        fs::remove_file(recovery.join(metadata)).unwrap();
    }
    fs::create_dir_all(recovery.join("nested/objects")).unwrap();
    fs::create_dir_all(recovery.join("nested/refs")).unwrap();
    fs::write(recovery.join("nested/HEAD"), b"ref: refs/heads/main").unwrap();
    assert!(protect_contents(&recovery, true, &cancellation).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn deletion_and_outbound_cleanup_use_repository_recovery_on_another_filesystem() {
    use std::os::unix::fs::MetadataExt;
    let Ok(directory) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    if fs::metadata(directory.path()).unwrap().dev()
        == fs::metadata(std::env::temp_dir()).unwrap().dev()
    {
        return;
    }
    let git = directory.path().join(".git");
    fs::create_dir(&git).unwrap();
    let mut journal = JournalEntry::default();
    assert!(journal.reserve(directory.path()).unwrap().starts_with(&git));
    let mut service = Filesystem::default();
    for outbound in [false, true] {
        let source = directory.path().join("folder");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file"), b"retained until verified").unwrap();
        let operation = if outbound {
            Operation::CompleteOutbound {
                receipt: service
                    .prepare_outbound(
                        OperationId::allocate(),
                        vec![source.clone()],
                        &Cancellation::default(),
                    )
                    .unwrap(),
                intent: Some(TransferIntent::Move),
                source_removed: false,
            }
        } else {
            Operation::DeletePermanently {
                sources: vec![source.clone()],
                confirmed: true,
            }
        };
        let result = run(&mut service, operation);
        success(&result);
        assert!(!source.exists());
        assert!(!result.undo_available);
        assert!(git.is_dir());
    }
}

#[test]
fn editor_baselines_reject_oversized_files_and_directories_without_hashing_contents() {
    let directory = tempfile::tempdir().unwrap();
    let oversized = directory.path().join("large.bin");
    File::create(&oversized)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    let folder = directory.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("child"), b"must not be hashed").unwrap();
    super::super::io::CONTENT_BYTES_HASHED.with(|bytes| bytes.set(0));
    for path in [&oversized, &folder] {
        assert!(DiskVersion::read_file(path, 32 * 1024 * 1024).is_err());
    }
    assert_eq!(
        super::super::io::CONTENT_BYTES_HASHED.with(|bytes| bytes.get()),
        0
    );
    let normal = directory.path().join("normal.txt");
    fs::write(&normal, b"small").unwrap();
    assert_eq!(
        DiskVersion::read_file(&normal, 5).unwrap(),
        DiskVersion::read(&normal).unwrap()
    );
}

#[test]
fn case_only_rename_and_numbered_duplicates_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("Readme.txt");
    fs::write(&original, b"unchanged\0bytes").unwrap();
    let mut service = Filesystem::default();
    success(&run(
        &mut service,
        Operation::Rename {
            source: original.clone(),
            name: "README.txt".into(),
        },
    ));
    let renamed = directory.path().join("README.txt");
    assert_eq!(fs::read(&renamed).unwrap(), b"unchanged\0bytes");
    success(&run(&mut service, Operation::Undo));
    assert_eq!(fs::read(&original).unwrap(), b"unchanged\0bytes");
    success(&run(&mut service, Operation::Redo));
    for _ in 0..2 {
        success(&run(
            &mut service,
            Operation::Duplicate {
                sources: vec![renamed.clone()],
            },
        ));
    }
    assert!(directory.path().join("README copy.txt").is_file());
    assert!(directory.path().join("README copy 2.txt").is_file());
}

#[test]
fn native_source_owned_moves_copy_without_removing_the_source_or_claiming_undo() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let destination = directory.path().join("destination");
    fs::write(&source, b"saved contents").unwrap();
    fs::create_dir(&destination).unwrap();
    let mut request = Request::new(Operation::Transfer {
        sources: vec![source.clone()],
        destination: destination.clone(),
        intent: TransferIntent::Copy,
    });
    request.native_source_move = true;
    let result = Filesystem::default().execute(request, |_| {});
    success(&result);
    assert!(!result.undo_available);
    assert!(source.is_file());
    assert_eq!(
        fs::read(destination.join("source")).unwrap(),
        b"saved contents"
    );
}

#[test]
fn recursive_copy_preserves_hidden_contents_and_undo_redo() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("folder");
    fs::create_dir_all(source.join(".hidden")).unwrap();
    fs::write(source.join(".hidden/ignored"), b"\0\xff\r\n").unwrap();
    fs::write(source.join("empty"), b"").unwrap();
    let mut fs = Filesystem::default();
    success(&run(
        &mut fs,
        Operation::Duplicate {
            sources: vec![source.clone(), source.join("empty")],
        },
    ));
    let copy = temp.path().join("folder copy");
    assert_eq!(
        std::fs::read(copy.join(".hidden/ignored")).unwrap(),
        b"\0\xff\r\n"
    );
    success(&run(&mut fs, Operation::Undo));
    assert!(!copy.exists());
    success(&run(&mut fs, Operation::Redo));
    assert!(copy.join("empty").exists());
}

#[test]
fn collision_requires_version_bound_decision_and_replacement_is_reversible() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("note.txt");
    let target = temp.path().join("dest");
    fs::create_dir(&target).unwrap();
    fs::write(&source, b"new").unwrap();
    fs::write(target.join("note.txt"), b"old").unwrap();
    let mut fs = Filesystem::default();
    let mut request = Request::new(Operation::Transfer {
        sources: vec![source],
        destination: target.clone(),
        intent: TransferIntent::Copy,
    });
    let result = fs.execute(request.clone(), |_| {});
    let ItemOutcome::Conflict(conflict) = &result.items[0].outcome else {
        panic!("{result:?}");
    };
    assert_eq!(std::fs::read(target.join("note.txt")).unwrap(), b"old");
    request.resolutions.insert(
        conflict.destination.clone(),
        ConflictResolution {
            expected: conflict.version.clone(),
            choice: ConflictChoice::Replace,
        },
    );
    success(&fs.execute(request, |_| {}));
    assert_eq!(std::fs::read(target.join("note.txt")).unwrap(), b"new");
    success(&run(&mut fs, Operation::Undo));
    assert_eq!(std::fs::read(target.join("note.txt")).unwrap(), b"old");
    success(&run(&mut fs, Operation::Redo));
    assert_eq!(std::fs::read(target.join("note.txt")).unwrap(), b"new");
}

#[test]
fn undo_refuses_external_changes_and_remains_retryable() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("file");
    fs::write(&source, b"original").unwrap();
    let mut fs = Filesystem::default();
    success(&run(
        &mut fs,
        Operation::Rename {
            source: source.clone(),
            name: "renamed".into(),
        },
    ));
    let renamed = source.with_file_name("renamed");
    std::fs::write(&renamed, b"other window").unwrap();
    let result = run(&mut fs, Operation::Undo);
    assert!(!result.succeeded());
    assert!(result.undo_available);
    assert_eq!(std::fs::read(&renamed).unwrap(), b"other window");
    assert!(!source.exists());
}

#[test]
fn protected_nested_repositories_and_descendant_destinations_are_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("parent");
    fs::create_dir_all(source.join("nested/.git")).unwrap();
    let mut fs = Filesystem::default();
    let result = run(
        &mut fs,
        Operation::DeletePermanently {
            sources: vec![source.clone()],
            confirmed: true,
        },
    );
    assert!(!result.succeeded());
    assert!(source.join("nested/.git").exists());
    let folder = temp.path().join("folder");
    std::fs::create_dir_all(folder.join("child")).unwrap();
    assert!(
        !run(
            &mut fs,
            Operation::Transfer {
                sources: vec![folder.clone()],
                destination: folder.join("child"),
                intent: TransferIntent::Move
            }
        )
        .succeeded()
    );
}

#[test]
fn cancellation_keeps_completed_batch_portion_in_journal() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::write(&first, b"a").unwrap();
    fs::write(&second, b"b").unwrap();
    let mut fs = Filesystem::default();
    let request = Request::new(Operation::Duplicate {
        sources: vec![first.clone(), second],
    });
    let token = request.cancellation.clone();
    let result = fs.execute(request, |p| {
        if p.completed_items == 1 {
            token.cancel();
        }
    });
    assert!(matches!(result.items[0].outcome, ItemOutcome::Completed));
    assert!(matches!(result.items[1].outcome, ItemOutcome::Cancelled));
    assert!(result.undo_available);
    success(&run(&mut fs, Operation::Undo));
    assert!(!first.with_file_name("first copy").exists());
}

#[cfg(unix)]
#[test]
fn links_are_copied_as_links_and_executable_bits_survive() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("script"), b"#!/bin/sh").unwrap();
    fs::set_permissions(source.join("script"), fs::Permissions::from_mode(0o751)).unwrap();
    symlink("../missing", source.join("broken")).unwrap();
    symlink(".", source.join("cycle")).unwrap();
    let mut fs = Filesystem::default();
    success(&run(
        &mut fs,
        Operation::Duplicate {
            sources: vec![source],
        },
    ));
    let copy = temp.path().join("source copy");
    assert_eq!(
        std::fs::read_link(copy.join("broken")).unwrap(),
        Path::new("../missing")
    );
    assert_eq!(
        std::fs::read_link(copy.join("cycle")).unwrap(),
        Path::new(".")
    );
    assert_eq!(
        std::fs::metadata(copy.join("script"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
}

#[test]
fn save_checks_loaded_version_and_preserves_newer_disk_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("document");
    fs::write(&path, b"loaded").unwrap();
    let version = DiskVersion::read(&path).unwrap();
    fs::write(&path, b"external").unwrap();
    let mut fs = Filesystem::default();
    assert!(fs.save(&path, b"buffer", Some(&version), false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"external");
    fs.save(&path, b"buffer", Some(&version), true).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"buffer");
}

#[test]
fn explicit_replacement_recreates_deleted_parents_for_repository_and_standalone_saves() {
    for repository in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = canonical_path(directory.path()).unwrap();
        let parent = root.join("deleted/nested");
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join("file.txt");
        fs::write(&path, b"loaded").unwrap();
        let expected = DiskVersion::read(&path).unwrap();
        fs::remove_dir_all(root.join("deleted")).unwrap();
        let mut service = Filesystem::default();
        let save = |overwrite| Operation::Save {
            path: path.clone(),
            worktree: repository.then(|| root.clone()),
            contents: Arc::from(&b"retained edits"[..]),
            expected: Some(expected.clone()),
            overwrite,
        };
        assert!(!run(&mut service, save(false)).succeeded());
        assert!(
            !root.join("deleted").exists(),
            "autosave must not recreate deleted directories"
        );
        let result = run(&mut service, save(true));
        success(&result);
        assert_eq!(fs::read(&path).unwrap(), b"retained edits");
        assert_eq!(
            result.saved_version,
            Some(DiskVersion::read(&path).unwrap())
        );
        assert_eq!(
            result.changes,
            vec![PathChange {
                old: None,
                new: Some(path.clone())
            }]
        );
        assert_eq!(service.changes_since(0), result.changes);
        // The direct save API uses the same recreation and validation path.
        fs::remove_dir_all(root.join("deleted")).unwrap();
        service
            .save(&path, b"direct save", Some(&expected), true)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"direct save");
    }
}

#[test]
fn recreating_save_parents_preserves_protected_paths_and_non_directories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join("blocked"), b"existing file").unwrap();
    let mut service = Filesystem::default();
    for path in [
        root.join(".git/missing/file"),
        root.join("blocked/missing/file"),
    ] {
        assert!(service.save(&path, b"edits", None, true).is_err());
    }
    assert!(!root.join(".git/missing").exists());
    assert_eq!(fs::read(root.join("blocked")).unwrap(), b"existing file");
    let outside = root.join("outside");
    let worktree = root.join("repo");
    fs::create_dir(&worktree).unwrap();
    for path in [outside.join("file"), worktree.join("../outside/file")] {
        assert!(
            !run(
                &mut service,
                Operation::Save {
                    path,
                    worktree: Some(worktree.clone()),
                    contents: Arc::from(&b"edits"[..]),
                    expected: None,
                    overwrite: true,
                }
            )
            .succeeded()
        );
        assert!(!outside.exists());
    }
}

#[cfg(unix)]
#[test]
fn recreating_save_parents_never_follows_a_replaced_or_dangling_symlink() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let outside = root.join("outside");
    let worktree = root.join("repo");
    fs::create_dir(&worktree).unwrap();
    let link = worktree.join("deleted");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let mut service = Filesystem::default();
    for dangling in [true, false] {
        if !dangling {
            fs::create_dir(&outside).unwrap();
        }
        for repository in [true, false] {
            let result = run(
                &mut service,
                Operation::Save {
                    path: link.join("nested/file.txt"),
                    worktree: repository.then(|| worktree.clone()),
                    contents: Arc::from(&b"edits"[..]),
                    expected: None,
                    overwrite: true,
                },
            );
            assert!(!result.succeeded());
            assert!(!outside.join("nested").exists());
            assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        }
    }
    // An ancestor alias must not hide Git metadata during parent creation.
    fs::create_dir_all(root.join(".git/existing")).unwrap();
    std::os::unix::fs::symlink(root.join(".git"), root.join("alias")).unwrap();
    assert!(
        service
            .save(
                &root.join("alias/existing/nested/file"),
                b"edits",
                None,
                true
            )
            .is_err()
    );
    assert!(!root.join(".git/existing/nested").exists());
}

fn replace_for_shutdown(service: &mut Filesystem, root: &Path) -> PathBuf {
    let root = canonical_path(root).unwrap();
    let source = root.join("file.txt");
    let folder = root.join("destination");
    fs::create_dir(&folder).unwrap();
    let destination = folder.join("file.txt");
    fs::write(&source, b"new").unwrap();
    fs::write(&destination, b"old").unwrap();
    let mut request = Request::new(Operation::Transfer {
        sources: vec![source],
        destination: folder,
        intent: TransferIntent::Copy,
    });
    request.resolutions.insert(
        destination,
        ConflictResolution {
            expected: DiskVersion::read(&root.join("destination/file.txt")).unwrap(),
            choice: ConflictChoice::Replace,
        },
    );
    success(&service.execute(request, |_| {}));
    let parked = service.undo.back().unwrap().steps[0].to.clone();
    assert_eq!(fs::read(&parked).unwrap(), b"old");
    parked
}

fn journal_areas(service: &Filesystem) -> Vec<PathBuf> {
    service
        .undo
        .iter()
        .chain(&service.redo)
        .flat_map(|entry| entry.areas.iter().map(|area| area.path().to_path_buf()))
        .collect()
}

#[test]
fn shutdown_cleans_global_journals_before_process_exit() {
    const CHILD: &str = "GITCOMET_TEST_FILESYSTEM_SHUTDOWN_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // The singleton never drops at process exit; exercise the actual
        // shutdown entry point in an isolated process, not a local destructor.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "filesystem::engine::tests::shutdown_cleans_global_journals_before_process_exit",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let areas = {
        let mut service = global().lock().unwrap();
        replace_for_shutdown(&mut service, root);
        success(&run(
            &mut service,
            Operation::CreateFile {
                path: root.join("created"),
            },
        ));
        success(&run(&mut service, Operation::Undo));
        assert!(!service.undo.is_empty() && !service.redo.is_empty());
        journal_areas(&service)
    };
    assert!(areas.iter().all(|area| area.exists()));
    cleanup_on_shutdown();
    cleanup_on_shutdown();
    assert!(areas.iter().all(|area| !area.exists()));
    assert_eq!(fs::read(root.join("destination/file.txt")).unwrap(), b"new");
    assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"new");
    let mut service = global().lock().unwrap();
    let result = run(
        &mut service,
        Operation::CreateFile {
            path: root.join("late"),
        },
    );
    assert!(matches!(result.items[0].outcome, ItemOutcome::Cancelled));
    assert!(!root.join("late").exists());
    assert!(
        service
            .save(&root.join("late/file"), b"late save", None, true)
            .is_err()
    );
    assert!(!root.join("late").exists());
    assert!(service.undo.is_empty() && service.redo.is_empty());
}

#[test]
fn shutdown_preserves_recovery_data_after_failed_or_partial_undo() {
    for partial in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut service = Filesystem::default();
        let parked = replace_for_shutdown(&mut service, directory.path());
        let destination = directory.path().join("destination/file.txt");
        fs::write(
            if partial { &parked } else { &destination },
            b"external changes",
        )
        .unwrap();
        assert!(!run(&mut service, Operation::Undo).succeeded());
        let areas = journal_areas(&service);
        service.shutdown();
        assert!(areas.iter().all(|area| area.exists()));
        assert_eq!(
            fs::read(&parked).unwrap(),
            if partial {
                &b"external changes"[..]
            } else {
                &b"old"[..]
            }
        );
        if !partial {
            assert_eq!(fs::read(&destination).unwrap(), b"external changes");
        }
        // These directories intentionally outlive the service for manual recovery.
        for area in areas {
            fs::remove_dir_all(area).unwrap();
        }
    }
}

#[test]
fn successful_undo_retry_releases_recovery_storage_on_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = Filesystem::default();
    replace_for_shutdown(&mut service, directory.path());
    let destination = directory.path().join("destination/file.txt");
    fs::write(&destination, b"external").unwrap();
    assert!(!run(&mut service, Operation::Undo).succeeded());
    fs::write(&destination, b"new").unwrap();
    success(&run(&mut service, Operation::Undo));
    let areas = journal_areas(&service);
    service.shutdown();
    assert!(areas.iter().all(|area| !area.exists()));
    assert_eq!(fs::read(destination).unwrap(), b"old");
}

#[test]
fn cancelling_undo_before_any_steps_does_not_retain_ordinary_journal_storage() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = Filesystem::default();
    replace_for_shutdown(&mut service, directory.path());
    let areas = journal_areas(&service);
    let undo = Request::new(Operation::Undo);
    undo.cancellation.cancel();
    assert!(!service.execute(undo, |_| {}).succeeded());
    service.shutdown();
    assert!(areas.iter().all(|area| !area.exists()));
    assert_eq!(
        fs::read(directory.path().join("destination/file.txt")).unwrap(),
        b"new"
    );
}

#[test]
fn copy_names_increment_before_extension_and_preserve_native_names() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("report.final.txt");
    fs::write(&path, b"a").unwrap();
    fs::write(temp.path().join("report.final copy.txt"), b"b").unwrap();
    assert_eq!(
        copy_name(&path).unwrap(),
        temp.path().join("report.final copy 2.txt")
    );
    assert!(validate_name(OsStr::new("../escape")).is_err());
    assert!(validate_name(OsStr::new(".")).is_err());
    // Windows reserves control characters in names; other systems accept them.
    assert_eq!(
        validate_name(OsStr::new("file name\nline")).is_ok(),
        cfg!(not(windows))
    );
}

#[cfg(unix)]
#[test]
fn trash_info_escapes_native_path_and_reserves_distinct_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("a\n#% b");
    fs::write(&path, b"test").unwrap();
    let root = temp.path().join("Trash");
    let first = crate::filesystem::trash::prepare_in(&path, &root).unwrap();
    let second = crate::filesystem::trash::prepare_in(&path, &root).unwrap();
    assert_ne!(first.item, second.item);
    let text = fs::read_to_string(first.info).unwrap();
    assert!(text.starts_with("[Trash Info]\nPath="));
    assert!(text.contains("a%0A%23%25%20b\nDeletionDate="));
    assert!(path.exists());
}

#[test]
fn merge_undo_preserves_existing_children_and_reports_only_moved_paths() {
    let dir = tempfile::tempdir().unwrap();
    // Resolutions are keyed by the canonical destination the engine reports.
    let root = canonical_path(dir.path()).unwrap();
    let source = root.join("a/folder");
    let destination = root.join("b/folder");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("new"), b"new").unwrap();
    fs::write(destination.join("existing"), b"old").unwrap();
    let mut service = Filesystem::default();
    let mut request = Request::new(Operation::Transfer {
        sources: vec![source.clone()],
        destination: root.join("b"),
        intent: TransferIntent::Move,
    });
    request.resolutions.insert(
        destination.clone(),
        ConflictResolution {
            expected: DiskVersion::read(&destination).unwrap(),
            choice: ConflictChoice::Merge,
        },
    );
    success(&service.execute(request, |_| {}));
    assert!(!source.exists());
    let undone = run(&mut service, Operation::Undo);
    success(&undone);
    assert!(source.join("new").exists());
    assert_eq!(fs::read(destination.join("existing")).unwrap(), b"old");
    assert!(
        undone
            .changes
            .iter()
            .all(|change| change.retarget(&destination.join("existing")).is_none())
    );
    success(&run(&mut service, Operation::Redo));
    assert!(destination.join("new").exists());
}

#[test]
fn moving_to_current_folder_is_a_noop_and_does_not_consume_undo() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x");
    fs::write(&file, b"kept").unwrap();
    let mut service = Filesystem::default();
    let result = run(
        &mut service,
        Operation::Transfer {
            sources: vec![file.clone()],
            destination: dir.path().to_path_buf(),
            intent: TransferIntent::Move,
        },
    );
    assert!(matches!(result.items[0].outcome, ItemOutcome::Skipped));
    assert!(!result.undo_available);
    assert_eq!(fs::read(file).unwrap(), b"kept");
}

#[test]
fn outdated_conflict_resolution_cannot_replace_a_newer_destination() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    fs::write(&source, b"a").unwrap();
    fs::write(&destination, b"b").unwrap();
    let expected = DiskVersion::read(&destination).unwrap();
    fs::write(&destination, b"newer").unwrap();
    let mut request = Request::new(Operation::Rename {
        source: source.clone(),
        name: "destination".into(),
    });
    request.resolutions.insert(
        destination.clone(),
        ConflictResolution {
            expected,
            choice: ConflictChoice::Replace,
        },
    );
    let result = Filesystem::default().execute(request, |_| {});
    assert!(matches!(result.items[0].outcome, ItemOutcome::Conflict(_)));
    assert_eq!(fs::read(source).unwrap(), b"a");
    assert_eq!(fs::read(destination).unwrap(), b"newer");
}

#[test]
fn successful_external_move_cleanup_is_once_only_and_never_journaled() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source");
    fs::write(&file, b"data").unwrap();
    let mut service = Filesystem::default();
    let receipt = service
        .prepare_outbound(
            OperationId::allocate(),
            vec![file.clone()],
            &Cancellation::default(),
        )
        .unwrap();
    let result = run(
        &mut service,
        Operation::CompleteOutbound {
            receipt: receipt.clone(),
            intent: Some(TransferIntent::Move),
            source_removed: false,
        },
    );
    success(&result);
    assert!(!file.exists());
    assert!(!result.undo_available);
    fs::write(&file, b"replacement").unwrap();
    let result = run(
        &mut service,
        Operation::CompleteOutbound {
            receipt,
            intent: Some(TransferIntent::Move),
            source_removed: false,
        },
    );
    assert!(!result.succeeded());
    assert_eq!(fs::read(file).unwrap(), b"replacement");
}

#[test]
fn cancelled_or_receiver_owned_transfers_never_remove_sources() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source");
    fs::write(&file, b"data").unwrap();
    let mut service = Filesystem::default();
    let receipt = service
        .prepare_outbound(
            OperationId::allocate(),
            vec![file.clone()],
            &Cancellation::default(),
        )
        .unwrap();
    success(&run(
        &mut service,
        Operation::CompleteOutbound {
            receipt: receipt.clone(),
            intent: None,
            source_removed: false,
        },
    ));
    success(&run(
        &mut service,
        Operation::CompleteOutbound {
            receipt,
            intent: Some(TransferIntent::Move),
            source_removed: true,
        },
    ));
    assert_eq!(fs::read(file).unwrap(), b"data");
}

#[cfg(target_os = "linux")]
#[test]
fn cross_filesystem_move_and_its_undo_redo_preserve_bytes() {
    use std::os::unix::fs::MetadataExt;
    let source_dir = tempfile::tempdir().unwrap();
    let Ok(destination_dir) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    if fs::metadata(source_dir.path()).unwrap().dev()
        == fs::metadata(destination_dir.path()).unwrap().dev()
    {
        return;
    }
    let source = source_dir.path().join("data");
    fs::write(&source, b"across devices").unwrap();
    let mut service = Filesystem::default();
    success(&run(
        &mut service,
        Operation::Transfer {
            sources: vec![source.clone()],
            destination: destination_dir.path().to_path_buf(),
            intent: TransferIntent::Move,
        },
    ));
    assert!(!source.exists());
    assert_eq!(
        fs::read(destination_dir.path().join("data")).unwrap(),
        b"across devices"
    );
    success(&run(&mut service, Operation::Undo));
    assert_eq!(fs::read(&source).unwrap(), b"across devices");
    success(&run(&mut service, Operation::Redo));
    assert!(!source.exists());
}

#[test]
fn merge_resumes_after_a_child_collision_and_undo_groups_all_successes() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let directory = tempfile::tempdir().unwrap();
        let root = canonical_path(directory.path()).unwrap();
        let source = root.join("source/items");
        let parent = root.join("destination");
        let destination = parent.join("items");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        for name in ["a", "b", "c"] {
            fs::write(source.join(name), name).unwrap();
        }
        fs::write(destination.join("b"), "existing").unwrap();
        let mut service = Filesystem::default();
        let mut request = Request::new(Operation::Transfer {
            sources: vec![source.clone()],
            destination: parent,
            intent,
        });
        request.resolutions.insert(
            destination.clone(),
            ConflictResolution {
                expected: DiskVersion::read(&destination).unwrap(),
                choice: ConflictChoice::Merge,
            },
        );
        let result = service.execute(request, |_| {});
        let ItemOutcome::Conflict(conflict) = &result.items[0].outcome else {
            panic!("{result:?}");
        };
        assert_eq!(conflict.source, source.join("b"));
        assert!(destination.join("a").exists());
        let mut continuation = *conflict.continuation.clone().unwrap();
        continuation.id = OperationId::allocate();
        continuation.resolutions.insert(
            conflict.destination.clone(),
            ConflictResolution {
                expected: conflict.version.clone(),
                choice: ConflictChoice::Replace,
            },
        );
        success(&service.execute(continuation, |_| {}));
        for name in ["a", "b", "c"] {
            assert_eq!(fs::read_to_string(destination.join(name)).unwrap(), name);
        }
        assert_eq!(
            service.undo.len(),
            1,
            "one logical paste, including its continuation"
        );
        success(&run(&mut service, Operation::Undo));
        assert_eq!(
            fs::read_to_string(destination.join("b")).unwrap(),
            "existing"
        );
        assert!(!destination.join("a").exists());
        assert!(!destination.join("c").exists());
        for name in ["a", "b", "c"] {
            assert_eq!(fs::read_to_string(source.join(name)).unwrap(), name);
        }
    }
}

#[test]
fn journal_keeps_one_hundred_logical_operations() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = Filesystem::default();
    for i in 0..101 {
        success(&run(
            &mut service,
            Operation::CreateFile {
                path: directory.path().join(i.to_string()),
            },
        ));
    }
    assert_eq!(service.undo.len(), 100);
    for _ in 0..100 {
        success(&run(&mut service, Operation::Undo));
    }
    assert!(directory.path().join("0").exists());
    assert!(service.undo.is_empty());
    assert_eq!(service.redo.len(), 100);
}

#[test]
fn undo_preserves_a_replaced_parent_and_a_new_repository_boundary() {
    for new_repository in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let source_parent = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::create_dir(&source_parent).unwrap();
        fs::create_dir(&destination).unwrap();
        let source = source_parent.join("file");
        fs::write(&source, "original").unwrap();
        let mut service = Filesystem::default();
        success(&run(
            &mut service,
            Operation::Transfer {
                sources: vec![source.clone()],
                destination: destination.clone(),
                intent: TransferIntent::Move,
            },
        ));
        if new_repository {
            fs::create_dir(source_parent.join(".git")).unwrap();
        } else {
            // Keep the old parent alive so its inode cannot be reused.
            fs::rename(&source_parent, directory.path().join("old-parent")).unwrap();
            fs::create_dir(&source_parent).unwrap();
        }
        let undo = run(&mut service, Operation::Undo);
        assert!(!undo.succeeded());
        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(destination.join("file")).unwrap(),
            "original"
        );
        assert!(undo.undo_available);
    }
}

#[test]
#[ignore = "exercises the current user's native Trash; run explicitly for platform acceptance"]
fn native_trash_restores_the_exact_entry_and_preserves_a_conflicting_file() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("native trash # %.txt");
    fs::write(&source, "original bytes").unwrap();
    let mut service = Filesystem::default();
    success(&run(
        &mut service,
        Operation::Trash {
            sources: vec![source.clone()],
        },
    ));
    assert!(!source.exists());
    fs::write(&source, "new file at the same path").unwrap();
    assert!(!run(&mut service, Operation::Undo).succeeded());
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "new file at the same path"
    );
    let newer = directory.path().join("newer.txt");
    fs::rename(&source, &newer).unwrap();
    success(&run(&mut service, Operation::Undo));
    assert_eq!(fs::read_to_string(&source).unwrap(), "original bytes");
    success(&run(&mut service, Operation::Redo));
    assert!(!source.exists());
    success(&run(&mut service, Operation::Undo));
    assert_eq!(fs::read_to_string(&source).unwrap(), "original bytes");
    assert_eq!(
        fs::read_to_string(&newer).unwrap(),
        "new file at the same path"
    );
}

#[test]
fn the_recovery_log_records_both_native_paths_of_every_move() {
    // Nothing covered `record_intent`, which is the only record of where a
    // parked `.gitcomet-operation-*/item` came from.
    let directory = tempfile::tempdir().unwrap();
    let root = canonical_path(directory.path()).unwrap();
    let source = root.join("notes.txt");
    fs::write(&source, b"x").unwrap();
    let destination = root.join("into");
    fs::create_dir(&destination).unwrap();
    let mut service = Filesystem::default();
    success(&run(
        &mut service,
        Operation::Transfer {
            sources: vec![source.clone()],
            destination: destination.clone(),
            intent: TransferIntent::Move,
        },
    ));
    let entry = service.undo.back().expect("journal entry");
    let area = entry.areas.first().expect("recovery area");
    let log = fs::read_to_string(area.path().join("recovery.log")).unwrap();
    assert_eq!(
        log,
        format!(
            "move\t{}\t{}\n",
            encoded_path(&source),
            encoded_path(&destination.join("notes.txt"))
        ),
        "one tab-separated line per move, newline terminated"
    );
}

/// Pins how many times each transfer shape walks the bytes it moves. These
/// counts are the whole reason a large drag-and-drop used to stall, so a
/// regression here is a user-visible one.
#[test]
fn transfers_hash_their_trees_a_bounded_number_of_times() {
    const SIZE: u64 = 512 * 1024;

    let passes = |operation: Operation, service: &mut Filesystem| -> u64 {
        CONTENT_BYTES_HASHED.with(|counted| counted.set(0));
        let result = service.execute(Request::new(operation), |_| {});
        success(&result);
        CONTENT_BYTES_HASHED
            .with(|counted| counted.get())
            .div_ceil(SIZE)
    };

    let directory = tempfile::tempdir().unwrap();
    let payload = vec![b'x'; SIZE as usize];

    let move_source = directory.path().join("moved.bin");
    fs::write(&move_source, &payload).unwrap();
    let into = directory.path().join("into");
    fs::create_dir(&into).unwrap();
    let mut service = Filesystem::default();
    let move_passes = passes(
        Operation::Transfer {
            sources: vec![move_source],
            destination: into.clone(),
            intent: TransferIntent::Move,
        },
        &mut service,
    );

    let copy_source = directory.path().join("copied.bin");
    fs::write(&copy_source, &payload).unwrap();
    let copy_passes = passes(
        Operation::Transfer {
            sources: vec![copy_source],
            destination: into,
            intent: TransferIntent::Copy,
        },
        &mut service,
    );

    // A same-device move is a rename: one pass to record what moved, one for
    // the version open editors re-adopt their baseline from.
    assert_eq!(move_passes, 2, "same-device move");
    // Copy reads the source, re-reads it to prove it held still, and reads the
    // copy back. `copy_tree`'s own read is not hashed and so not counted.
    assert_eq!(copy_passes, 3, "copy");
}

#[test]
fn version_checks_stop_when_the_operation_is_cancelled() {
    // `matches` walks the whole tree, so on a large directory it is one of the
    // longest things an operation does; it used to run to completion regardless.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.txt");
    fs::write(&path, b"contents").unwrap();
    let version = DiskVersion::read(&path).unwrap();
    let cancellation = Cancellation::default();

    assert!(version.matches(&path, &cancellation).is_ok());
    cancellation.cancel();
    assert_eq!(
        version.matches(&path, &cancellation).unwrap_err().kind(),
        io::ErrorKind::Interrupted,
    );
}

/// Every `.gitcomet-operation-*` entry below `root`.
fn operation_areas_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if crate::path_utils::is_service_owned_name(entry.file_name().as_encoded_bytes()) {
                found.push(path);
            } else if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            }
        }
    }
    found
}

#[test]
fn journal_storage_prefers_the_first_writable_same_volume_candidate() {
    let fixture = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let state_path = canonical_path(state.path()).unwrap();
    let mut service = Filesystem::with_storage_candidates(vec![
        PathBuf::from("/nonexistent/gitcomet-journal"),
        state_path.clone(),
    ]);
    replace_for_shutdown(&mut service, fixture.path());
    let root = canonical_path(fixture.path()).unwrap();
    let mut keep_both = Request::new(Operation::Transfer {
        sources: vec![root.join("file.txt")],
        destination: root.join("destination"),
        intent: TransferIntent::Copy,
    });
    keep_both.resolutions.insert(
        root.join("destination/file.txt"),
        ConflictResolution {
            expected: DiskVersion::read(&root.join("destination/file.txt")).unwrap(),
            choice: ConflictChoice::KeepBoth,
        },
    );
    success(&service.execute(keep_both, |_| {}));
    assert_eq!(
        fs::read(root.join("destination/file copy.txt")).unwrap(),
        b"new"
    );

    let areas = journal_areas(&service);
    assert!(!areas.is_empty());
    assert!(
        areas.iter().all(|area| area.starts_with(&state_path)),
        "{areas:?}"
    );
    assert_eq!(operation_areas_under(&root), Vec::<PathBuf>::new());
    success(&run(&mut service, Operation::Undo));
    success(&run(&mut service, Operation::Undo));
    assert_eq!(fs::read(root.join("destination/file.txt")).unwrap(), b"old");
    assert!(!root.join("destination/file copy.txt").exists());
}

#[test]
fn journal_storage_resolves_a_git_file_to_its_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = canonical_path(directory.path()).unwrap();
    let worktree = root.join("worktree");
    let gitdir = root.join("gitdir-target");
    fs::create_dir_all(&worktree).unwrap();
    fs::create_dir_all(&gitdir).unwrap();
    fs::write(worktree.join(".git"), "gitdir: ../gitdir-target\n").unwrap();
    let mut service = Filesystem::with_storage_candidates(vec![]);
    replace_for_shutdown(&mut service, &worktree);
    let areas = journal_areas(&service);
    assert!(!areas.is_empty());
    assert!(
        areas.iter().all(|area| area.starts_with(&gitdir)),
        "{areas:?}"
    );
    assert_eq!(operation_areas_under(&worktree), Vec::<PathBuf>::new());
}

#[cfg(target_os = "linux")]
#[test]
fn journal_storage_skips_cross_volume_candidates() {
    use std::os::unix::fs::MetadataExt;
    let Ok(other_volume) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    let fixture = tempfile::tempdir().unwrap();
    if fs::metadata(other_volume.path()).unwrap().dev()
        == fs::metadata(fixture.path()).unwrap().dev()
    {
        return;
    }
    let root = canonical_path(fixture.path()).unwrap();
    // Keep the repository fallback local even if the temp directory has a
    // repository ancestor, such as /tmp/.git.
    let gitdir = root.join(".git");
    fs::create_dir(&gitdir).unwrap();
    let mut service = Filesystem::with_storage_candidates(vec![other_volume.path().to_path_buf()]);
    replace_for_shutdown(&mut service, &root);
    let areas = journal_areas(&service);
    assert!(!areas.is_empty());
    assert!(
        areas.iter().all(|area| area.starts_with(&gitdir)),
        "{areas:?}"
    );
    assert_eq!(fs::read_dir(other_volume.path()).unwrap().count(), 0);
}

#[test]
fn shutdown_removes_the_empty_instance_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut service = Filesystem::default();
    let instance = service
        .configure_journal_storage(&state.path().join("journal"))
        .unwrap();
    assert!(instance.join("lock").is_file());
    // The temp dir comes first in production; point areas at the instance.
    service.storage = Arc::new(StoragePolicy::with_candidates(vec![instance.clone()]));
    replace_for_shutdown(&mut service, fixture.path());
    let areas = journal_areas(&service);
    assert!(areas.iter().all(|area| area.starts_with(&instance)));
    service.storage = Arc::new(StoragePolicy::instance(instance.clone()));
    service.shutdown();
    assert!(!instance.exists());
    assert!(state.path().join("journal").is_dir());
}

#[test]
fn shutdown_keeps_an_instance_directory_that_retains_recovery_data() {
    let fixture = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut service = Filesystem::default();
    let instance = service
        .configure_journal_storage(&state.path().join("journal"))
        .unwrap();
    service.storage = Arc::new(StoragePolicy::with_candidates(vec![instance.clone()]));
    let parked = replace_for_shutdown(&mut service, fixture.path());
    fs::write(fixture.path().join("destination/file.txt"), b"external").unwrap();
    assert!(!run(&mut service, Operation::Undo).succeeded());
    service.storage = Arc::new(StoragePolicy::instance(instance.clone()));
    service.shutdown();
    assert_eq!(fs::read(&parked).unwrap(), b"old");
    assert!(instance.is_dir());
}

#[test]
fn sweep_removes_dead_instances_but_keeps_retained_items_and_live_locks() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path();
    let instance = |name: &str, item: bool| {
        let area = root.join(name).join(".gitcomet-operation-x");
        fs::create_dir_all(&area).unwrap();
        fs::write(area.join("recovery.log"), b"move\ta\tb\n").unwrap();
        if item {
            fs::write(area.join("item"), b"parked").unwrap();
        }
        fs::write(root.join(name).join("lock"), b"").unwrap();
    };
    instance("1-10", false);
    instance("2-20", true);
    instance("3-30", false);
    let live = File::open(root.join("3-30/lock")).unwrap();
    live.try_lock().unwrap();
    // Without a lock file: still starting when fresh, dead once stale.
    fs::create_dir(root.join("4-40")).unwrap();
    fs::create_dir(root.join("5-50")).unwrap();
    let stale = filetime::FileTime::from_unix_time(1_000_000, 0);
    filetime::set_file_mtime(root.join("5-50"), stale).unwrap();
    fs::create_dir(root.join("notes")).unwrap();

    sweep_leaked_journal_storage(root);

    assert!(!root.join("1-10").exists(), "dead and empty");
    assert!(
        root.join("2-20/.gitcomet-operation-x/item").exists(),
        "retained"
    );
    assert!(root.join("3-30").exists(), "live");
    assert!(root.join("4-40").exists(), "fresh, unlocked");
    assert!(!root.join("5-50").exists(), "stale, unlocked");
    assert!(root.join("notes").exists(), "not an instance");
    drop(live);
}
