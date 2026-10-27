use super::*;
use gitcomet_core::large_files::AnnexTrust;

#[test]
fn bare_annex_all_versions_commands_transfer_content_without_checkout() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let source = init_annex_repo(dir.path());
    let bare = dir.path().join("bare");
    let key = git(&source, &["annex", "lookupkey", "big.bin"])
        .trim()
        .to_string();
    git(&source, &["clone", "--bare", ".", bare.to_str().unwrap()]);
    git(&bare, &["config", "user.name", "Test"]);
    git(&bare, &["config", "user.email", "test@example.com"]);
    let repo = open(&bare);
    repo.run_large_file_command(&LargeFileCommand::AnnexInit)
        .unwrap();
    repo.run_large_file_command(&LargeFileCommand::AnnexEnableRemote {
        name: "backup".into(),
        params: vec![format!("directory={}", dir.path().join("store").display())],
    })
    .unwrap();
    let here = || {
        repo.annex_whereis(&key, &CancellationToken::new())
            .unwrap()
            .copies
            .iter()
            .any(|copy| copy.here)
    };
    assert!(!here());
    for (command, present) in [
        (
            LargeFileCommand::AnnexGet {
                paths: Vec::new(),
                from: Some("origin".into()),
            },
            true,
        ),
        (
            LargeFileCommand::AnnexCopy {
                paths: Vec::new(),
                to: "backup".into(),
            },
            true,
        ),
        (
            LargeFileCommand::AnnexDrop {
                paths: Vec::new(),
                from: None,
                force: false,
            },
            false,
        ),
        (
            LargeFileCommand::AnnexGet {
                paths: Vec::new(),
                from: Some("backup".into()),
            },
            true,
        ),
        (
            LargeFileCommand::AnnexMove {
                paths: Vec::new(),
                to: "backup".into(),
            },
            false,
        ),
    ] {
        let output = repo.run_large_file_command(&command).unwrap();
        assert!(output.command.ends_with("(all versions)"), "{output:?}");
        assert_eq!(here(), present, "{command:?}");
    }
    repo.run_large_file_command(&LargeFileCommand::AnnexGetKeys {
        keys: vec![key.clone()],
    })
    .unwrap();
    assert!(here());
    assert!(!bare.join("big.bin").exists());
    assert!(!bare.join("index").exists());
}

#[test]
fn bare_annex_sync_and_repository_management_remain_available() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--bare", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    let repo = open(dir.path());
    repo.run_large_file_command(&LargeFileCommand::AnnexInit)
        .unwrap();
    for content in [false, true] {
        for command in [
            LargeFileCommand::AnnexPull { content },
            LargeFileCommand::AnnexPush { content },
            LargeFileCommand::AnnexSync { content },
        ] {
            repo.run_large_file_command(&command).unwrap();
        }
    }
    for command in [
        LargeFileCommand::AnnexNumcopies { copies: 2 },
        LargeFileCommand::AnnexDescribe {
            repository: "here".into(),
            description: "Bare archive".into(),
        },
        LargeFileCommand::AnnexTrust {
            repository: "here".into(),
            trust: AnnexTrust::Semitrusted,
        },
        LargeFileCommand::AnnexFsck,
        LargeFileCommand::AnnexRestage,
    ] {
        repo.run_large_file_command(&command).unwrap();
    }
    let support = repo.large_file_support(&CancellationToken::new()).unwrap();
    assert!(support.annex.initialized());
    assert_eq!(support.annex.numcopies, Some(2));
    assert!(
        repo.annex_unused(&CancellationToken::new())
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(!dir.path().join("index").exists());
}
