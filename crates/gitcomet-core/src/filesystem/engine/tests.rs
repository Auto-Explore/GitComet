use super::*;
use std::ffi::OsStr;

fn run(fs: &mut Filesystem, operation: Operation) -> OperationResult {
    fs.execute(Request::new(operation), |_| {})
}

fn success(result: &OperationResult) {
    assert!(result.succeeded(), "{:#?}", result.items);
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
    assert!(validate_name(OsStr::new("file name\nline")).is_ok());
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
    let source = dir.path().join("a/folder");
    let destination = dir.path().join("b/folder");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("new"), b"new").unwrap();
    fs::write(destination.join("existing"), b"old").unwrap();
    let mut service = Filesystem::default();
    let mut request = Request::new(Operation::Transfer {
        sources: vec![source.clone()],
        destination: dir.path().join("b"),
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
        let source = directory.path().join("source/items");
        let parent = directory.path().join("destination");
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
    let source = directory.path().join("notes.txt");
    fs::write(&source, b"x").unwrap();
    let destination = directory.path().join("into");
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
