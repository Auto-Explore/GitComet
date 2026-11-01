use super::*;
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::ErrorKind;
use gitcomet_core::large_files::LargeFileCommand;
use gitcomet_core::remote_url::RemoteUrlPolicy;

#[test]
fn bare_lfs_committed_fetches_pushes_and_maintenance_preserve_absent_checkout() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (remote, middle) = seed_lfs_history(dir.path());
    let bare = dir.path().join("bare");
    git(
        dir.path(),
        &[
            "clone",
            "--bare",
            "--branch",
            "main",
            &file_url(&remote),
            bare.to_str().unwrap(),
        ],
    );
    let repo = GixBackend.open(&bare).unwrap();
    repo.run_large_file_command(&LargeFileCommand::LfsInstall)
        .unwrap();
    let middle = CommitId(middle.into());
    let target = DiffTarget::commit(middle.clone(), "a.bin".into());
    repo.run_large_file_command(&LargeFileCommand::LfsFetchForDiff {
        target: target.clone(),
    })
    .unwrap();
    let diff = repo
        .diff_file_text(&target, None, &CancellationToken::new())
        .unwrap()
        .unwrap();
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("middle version\n")
    );
    let head = repo.head_commit_id().unwrap().unwrap();
    repo.run_large_file_command(&LargeFileCommand::LfsFetchForDiff {
        target: DiffTarget::commit_range(middle, Some(head), Some("a.bin".into())),
    })
    .unwrap();
    for command in [
        LargeFileCommand::LfsFetchAll,
        LargeFileCommand::LfsFsck,
        LargeFileCommand::LfsPrune,
    ] {
        repo.run_large_file_command(&command).unwrap();
    }
    let backup = dir.path().join("backup.git");
    git(dir.path(), &["init", "--bare", backup.to_str().unwrap()]);
    repo.add_remote_with_output("backup", &file_url(&backup), RemoteUrlPolicy::default())
        .unwrap();
    repo.run_large_file_command(&LargeFileCommand::LfsPushAll {
        remote: "backup".into(),
    })
    .unwrap();
    for rev in ["HEAD", "HEAD^", "HEAD~2"] {
        let text = git(&bare, &["show", &format!("{rev}:a.bin")]);
        let pointer = gitcomet_core::lfs::parse_pointer(text.as_bytes()).unwrap();
        let object = gitcomet_core::lfs::object_relative_path(&pointer.oid);
        assert!(bare.join("lfs").join(&object).is_file());
        assert!(backup.join("lfs").join(object).is_file());
    }
    assert!(!bare.join("index").exists());
    assert!(!bare.join("a.bin").exists());
}

#[test]
fn bare_lfs_unlock_requires_explicit_force_and_preserves_remote_lock_on_refusal() {
    if !git_lfs_available() {
        return;
    }
    let server = LockServer::start(&[("theirs.psd", "alice")]);
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--bare", "-b", "main"]);
    git(dir.path(), &["config", "lfs.url", &server.url]);
    let repo = GixBackend.open(dir.path()).unwrap();
    let held = server.held();
    let error = repo
        .run_large_file_command(&LargeFileCommand::LfsUnlock {
            paths: vec!["theirs.psd".into()],
            force: false,
        })
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::BareUnsupported));
    assert_eq!(server.held(), held);
    repo.run_large_file_command(&LargeFileCommand::LfsUnlock {
        paths: vec!["theirs.psd".into()],
        force: true,
    })
    .unwrap();
    assert!(server.held().is_empty());
    assert!(!dir.path().join("index").exists());
    assert!(!dir.path().join("theirs.psd").exists());
}
