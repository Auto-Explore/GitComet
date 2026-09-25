//! git-annex operations against real repositories. Skipped when git-annex is
//! not installed; a `directory` special remote stands in for a server.

#[path = "support/test_git_env.rs"]
mod test_git_env;

use gitcomet_core::domain::{
    CommitId, DiffArea, DiffTarget, FileStatus, FileStatusKind, RepoStatus,
};
use gitcomet_core::large_files::{AnnexAdjustMode, LargeFileCommand, LargeFileContent};
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn git_annex_available() -> bool {
    Command::new("git")
        .args(["annex", "version", "--raw"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// An initialized annex with `big.bin` (locked) and a `backup` directory remote.
fn init_annex_repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        git(&repo, &["config", key, value]);
    }
    git(&repo, &["annex", "init", "-q", "laptop"]);
    fs::write(repo.join("big.bin"), vec![7u8; 4096]).unwrap();
    git(&repo, &["annex", "add", "-q", "big.bin"]);
    git(&repo, &["commit", "-qm", "add big.bin"]);
    let store = dir.join("store");
    fs::create_dir_all(&store).unwrap();
    git(
        &repo,
        &[
            "annex",
            "initremote",
            "-q",
            "backup",
            "type=directory",
            &format!("directory={}", store.display()),
            "encryption=none",
        ],
    );
    repo
}

fn open(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).unwrap()
}

fn run(repo: &Path, command: LargeFileCommand) -> Result<String, String> {
    open(repo)
        .run_large_file_command(&command)
        .map(|output| output.stdout)
        .map_err(|error| match error.kind() {
            gitcomet_core::error::ErrorKind::Git(failure) => {
                failure.detail().unwrap_or_default().to_string()
            }
            other => format!("{other:?}"),
        })
}

fn paths(name: &str) -> Vec<PathBuf> {
    vec![PathBuf::from(name)]
}

fn staged_row(name: &str) -> RepoStatus {
    RepoStatus {
        staged: vec![FileStatus {
            path: PathBuf::from(name),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]
        .into(),
        unstaged: Vec::new().into(),
    }
}

/// CI lanes that install git-annex set `GITCOMET_REQUIRE_GIT_ANNEX=1`, so a
/// broken install fails instead of skipping every test.
macro_rules! require_annex {
    () => {
        if !git_annex_available() {
            assert!(
                std::env::var_os("GITCOMET_REQUIRE_GIT_ANNEX").is_none(),
                "GITCOMET_REQUIRE_GIT_ANNEX is set but git-annex is not installed"
            );
            eprintln!("skipping: git-annex is not installed");
            return;
        }
    };
}

#[test]
fn support_lists_repositories_special_remotes_and_numcopies() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    run(&repo, LargeFileCommand::AnnexNumcopies { copies: 2 }).unwrap();

    let support = open(&repo)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(support.annex.initialized() && support.annex.has_annex_branch);
    assert_eq!(support.annex.numcopies, Some(2));
    let names: Vec<_> = support
        .annex
        .repositories
        .iter()
        .filter(|repo| !repo.is_builtin())
        .map(|repo| {
            (
                repo.display_name().to_string(),
                repo.here,
                repo.special_type.clone(),
            )
        })
        .collect();
    assert!(names.contains(&("laptop".into(), true, None)), "{names:?}");
    assert!(
        names.contains(&("backup".into(), false, Some("directory".into()))),
        "{names:?}"
    );
}

/// Content moves between this clone and a special remote, and git-annex's
/// numcopies check refuses a drop that would lose the last copy.
#[test]
fn copy_drop_get_and_refused_drop() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());

    let refused = run(
        &repo,
        LargeFileCommand::AnnexDrop {
            paths: paths("big.bin"),
            from: None,
            force: false,
        },
    )
    .expect_err("the only copy cannot be dropped");
    assert!(refused.contains("big.bin: unsafe"), "{refused}");
    assert!(
        refused.contains("Hint: git-annex keeps content"),
        "{refused}"
    );
    assert!(repo.join("big.bin").is_file(), "content kept");

    run(
        &repo,
        LargeFileCommand::AnnexCopy {
            paths: paths("big.bin"),
            to: "backup".into(),
        },
    )
    .unwrap();
    let whereis = open(&repo)
        .annex_whereis_cancellable(Path::new("big.bin"), &CancellationToken::new())
        .unwrap();
    let places: Vec<_> = whereis
        .copies
        .iter()
        .map(|c| c.description.as_str())
        .collect();
    assert_eq!(places.len(), 2, "{places:?}");

    run(
        &repo,
        LargeFileCommand::AnnexDrop {
            paths: paths("big.bin"),
            from: None,
            force: false,
        },
    )
    .unwrap();
    assert!(!repo.join("big.bin").is_file(), "locked link now dangles");
    let files = open(&repo)
        .uncommitted_large_files_for_status_cancellable(
            &staged_row("big.bin"),
            &CancellationToken::new(),
        )
        .unwrap();
    // Staged rows describe the index; a committed file is not staged, so
    // check through the unstaged path instead.
    assert!(
        files
            .staged
            .get(Path::new("big.bin"))
            .is_none_or(|s| s.in_local_store == Some(false))
    );

    run(
        &repo,
        LargeFileCommand::AnnexGet {
            paths: paths("big.bin"),
            from: Some("backup".into()),
        },
    )
    .unwrap();
    assert_eq!(fs::read(repo.join("big.bin")).unwrap(), vec![7u8; 4096]);
}

#[test]
fn unlocked_presence_comes_from_git_annex_find() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    run(
        &repo,
        LargeFileCommand::AnnexUnlock {
            paths: paths("big.bin"),
        },
    )
    .unwrap();
    git(&repo, &["commit", "-qam", "unlock"]);
    // Touch the index entry so it is a staged row of an unlocked pointer.
    fs::write(repo.join("other.bin"), vec![9u8; 2048]).unwrap();
    git(&repo, &["annex", "add", "-q", "other.bin"]);
    run(
        &repo,
        LargeFileCommand::AnnexUnlock {
            paths: paths("other.bin"),
        },
    )
    .unwrap();
    git(&repo, &["add", "other.bin"]);

    let files = open(&repo)
        .uncommitted_large_files_for_status_cancellable(
            &staged_row("other.bin"),
            &CancellationToken::new(),
        )
        .unwrap();
    let state = &files.staged[Path::new("other.bin")];
    assert_eq!(state.in_local_store, Some(true), "{state:?}");
}

/// A historical unlocked version resolves to real content through
/// `git annex contentlocation`, so the diff compares files, not keys.
#[test]
fn unlocked_commit_diff_reads_content_from_the_annex() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    fs::write(repo.join("notes.txt"), "first version\n").unwrap();
    git(
        &repo,
        &[
            "-c",
            "annex.largefiles=anything",
            "annex",
            "add",
            "-q",
            "notes.txt",
        ],
    );
    run(
        &repo,
        LargeFileCommand::AnnexUnlock {
            paths: paths("notes.txt"),
        },
    )
    .unwrap();
    git(&repo, &["commit", "-qm", "notes"]);
    fs::write(repo.join("notes.txt"), "second version\n").unwrap();
    git(
        &repo,
        &["-c", "annex.largefiles=anything", "add", "notes.txt"],
    );
    git(&repo, &["commit", "-qm", "edit notes"]);
    let head = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();

    let diff = open(&repo)
        .diff_file_text(&DiffTarget::Commit {
            commit_id: CommitId(head.into()),
            path: Some(PathBuf::from("notes.txt")),
        })
        .unwrap()
        .unwrap();
    let old = diff.old_large.clone().expect("old side is annexed");
    let new = diff.new_large.clone().expect("new side is annexed");
    assert_eq!(
        (old.content, new.content),
        (LargeFileContent::Available, LargeFileContent::Available)
    );
    let text = |source: Option<&gitcomet_core::domain::FileDiffTextSource>| {
        fs::read_to_string(&source.unwrap().path).unwrap()
    };
    assert_eq!(text(diff.old_source.as_ref()), "first version\n");
    assert_eq!(text(diff.new_source.as_ref()), "second version\n");
}

#[test]
fn adjust_and_leave_adjusted_branch() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let base = git(&repo, &["branch", "--show-current"]).trim().to_string();
    run(
        &repo,
        LargeFileCommand::AnnexAdjust {
            mode: AnnexAdjustMode::Unlock,
        },
    )
    .unwrap();
    let head = git(&repo, &["branch", "--show-current"]);
    assert_eq!(
        gitcomet_core::annex::adjusted_branch(head.trim()),
        Some((base.as_str(), "unlocked"))
    );

    run(
        &repo,
        LargeFileCommand::AnnexLeaveAdjusted { base: base.clone() },
    )
    .unwrap();
    assert_eq!(git(&repo, &["branch", "--show-current"]).trim(), base);
}

#[test]
fn sync_never_commits_and_describe_trust_apply() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    fs::write(repo.join("draft.txt"), "uncommitted\n").unwrap();
    let before = git(&repo, &["rev-parse", "HEAD"]);
    run(&repo, LargeFileCommand::AnnexSync { content: false }).unwrap();
    assert_eq!(
        git(&repo, &["rev-parse", "HEAD"]),
        before,
        "sync must not commit"
    );

    run(
        &repo,
        LargeFileCommand::AnnexDescribe {
            repository: "here".into(),
            description: "work laptop".into(),
        },
    )
    .unwrap();
    run(
        &repo,
        LargeFileCommand::AnnexTrust {
            repository: "backup".into(),
            trust: gitcomet_core::large_files::AnnexTrust::Untrusted,
        },
    )
    .unwrap();
    let support = open(&repo)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    let find = |name: &str| {
        support
            .annex
            .repositories
            .iter()
            .find(|repo| repo.display_name() == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} in {:?}", support.annex.repositories))
    };
    assert!(find("work laptop").here);
    assert_eq!(
        find("backup").trust,
        gitcomet_core::large_files::AnnexTrust::Untrusted
    );
}

#[test]
fn invalid_arguments_are_rejected_before_running() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    for command in [
        LargeFileCommand::AnnexCopy {
            paths: paths("big.bin"),
            to: "--force".into(),
        },
        LargeFileCommand::AnnexInitRemote {
            name: "bad name".into(),
            special_type: "directory".into(),
            params: vec![],
        },
        LargeFileCommand::AnnexInitRemote {
            name: "store".into(),
            special_type: "directory".into(),
            params: vec!["noequals".into()],
        },
        LargeFileCommand::AnnexNumcopies { copies: 0 },
    ] {
        assert!(run(&repo, command.clone()).is_err(), "{command:?}");
    }
    let _ = DiffArea::Staged;
}

/// `--json-progress` lines reach the operation that started the transfer,
/// which is what keeps a long copy visible (and alive) in the activity panel.
#[test]
fn transfers_report_progress_to_the_operation() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let (sender, receiver) = std::sync::mpsc::channel();
    let context =
        gitcomet_core::git_operation::GitOperationContext::new("annex copy", move |_, event| {
            let _ = sender.send(event);
        });
    {
        let _scope = gitcomet_core::git_operation::attach(&context);
        run(
            &repo,
            LargeFileCommand::AnnexCopy {
                paths: paths("big.bin"),
                to: "backup".into(),
            },
        )
        .unwrap();
    }
    let progress: Vec<_> = receiver
        .try_iter()
        .filter_map(|event| match event {
            gitcomet_core::git_operation::GitOperationEvent::TransferProgress(progress) => {
                Some(progress)
            }
            _ => None,
        })
        .collect();
    let last = progress.last().expect("at least one progress line");
    assert_eq!(last.direction, "annex copy");
    assert_eq!((last.bytes_done, last.bytes_total), (4096, 4096));
    assert_eq!(last.name, "big.bin");
}

/// A historical version's content comes back by key, which the current path
/// no longer names.
#[test]
fn get_by_key_restores_a_dropped_historical_version() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let key = git(&repo, &["annex", "find", "--format=${key}", "big.bin"]);
    run(
        &repo,
        LargeFileCommand::AnnexCopy {
            paths: paths("big.bin"),
            to: "backup".into(),
        },
    )
    .unwrap();
    run(
        &repo,
        LargeFileCommand::AnnexDrop {
            paths: paths("big.bin"),
            from: None,
            force: false,
        },
    )
    .unwrap();
    assert!(!repo.join("big.bin").is_file());
    run(
        &repo,
        LargeFileCommand::AnnexGetKeys {
            keys: vec![key.trim().to_string()],
        },
    )
    .unwrap();
    assert!(
        repo.join("big.bin").is_file(),
        "the locked link resolves again"
    );
    assert!(
        run(
            &repo,
            LargeFileCommand::AnnexGetKeys {
                keys: vec!["not a key".into()]
            }
        )
        .is_err()
    );
}

/// git-annex's own bookkeeping must not look like repository changes to the
/// file watcher, or reading it (line stats, `git annex find`) refreshes forever.
#[test]
fn watch_info_treats_the_annex_directory_as_private_cache() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    let info = GixBackend
        .repository_watch_info(&repo)
        .unwrap()
        .expect("watch info");
    assert!(
        info.cache_dirs
            .iter()
            .any(|dir| dir.ends_with(".git/annex")),
        "{:?}",
        info.cache_dirs
    );
}

/// Status refreshes run constantly, so they must never start git-annex: its
/// clean filter hashes whole files and writes the keys database. Stale
/// unlocked files (content present, index stat data old) are exactly what
/// an interrupted `get` leaves behind.
#[cfg(unix)]
#[test]
fn status_and_line_stats_never_start_the_annex_filter() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    run(
        &repo,
        LargeFileCommand::AnnexUnlock {
            paths: paths("big.bin"),
        },
    )
    .unwrap();
    git(&repo, &["commit", "-qam", "unlock"]);
    // Same size, new mtime: Git would have to hash the file to know.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(repo.join("big.bin"))
        .unwrap()
        .set_modified(later)
        .unwrap();
    let log = dir.path().join("filter-launches.log");
    let wrapper = format!(
        "sh -c 'echo launch >> {}; exec git-annex filter-process'",
        log.display()
    );
    git(&repo, &["config", "filter.annex.process", &wrapper]);

    let opened = open(&repo);
    let status = opened.status().unwrap();
    assert!(!log.exists(), "status started git-annex");
    assert_eq!(
        status.unstaged.len(),
        1,
        "like `git status`, the stale file reads as modified until restaged"
    );
    opened
        .uncommitted_line_stats_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    assert!(!log.exists(), "line stats started git-annex");
}

/// What an interrupted `get` leaves: content in place, Git's index stat data
/// stale, and the path queued in `restage.log`. A locked index makes git-annex
/// defer its restage exactly as a killed command does.
#[cfg(unix)]
fn make_stale_unlocked_file(repo: &Path) {
    run(
        repo,
        LargeFileCommand::AnnexUnlock {
            paths: paths("big.bin"),
        },
    )
    .unwrap();
    git(repo, &["commit", "-qam", "unlock"]);
    git(repo, &["annex", "copy", "-q", "--to", "backup", "big.bin"]);
    git(repo, &["annex", "drop", "-q", "big.bin"]);
    fs::write(repo.join(".git/index.lock"), "").unwrap();
    git(repo, &["annex", "get", "-q", "big.bin"]);
    fs::remove_file(repo.join(".git/index.lock")).unwrap();
    assert_eq!(fs::metadata(repo.join("big.bin")).unwrap().len(), 4096);
}

#[cfg(unix)]
fn modified_rows(repo: &Path) -> usize {
    open(repo).status().unwrap().unstaged.len()
}

#[cfg(unix)]
#[test]
fn refresh_restages_files_left_stale_by_an_interrupted_command() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    make_stale_unlocked_file(&repo);
    let support = open(&repo)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(support.annex.restage_pending);
    assert_eq!(modified_rows(&repo), 1, "stale file reads as modified");

    run(&repo, LargeFileCommand::AnnexRestage).unwrap();
    assert_eq!(modified_rows(&repo), 0, "restage refreshes Git's index");
    assert!(
        !open(&repo)
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap()
            .annex
            .restage_pending
    );
}

/// git-annex restages at the end of a command; a failed or cancelled one
/// skips it, so GitComet restages afterwards regardless of the outcome.
#[cfg(unix)]
#[test]
fn failed_annex_commands_still_restage() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    make_stale_unlocked_file(&repo);
    let failed = run(
        &repo,
        LargeFileCommand::AnnexCopy {
            paths: paths("big.bin"),
            to: "no-such-remote".into(),
        },
    );
    let failed = failed.expect_err("a missing remote fails the command");
    assert!(failed.contains("no-such-remote"), "{failed}");
    assert_eq!(modified_rows(&repo), 0, "restage still ran");
}

/// A clone that has not enabled a special remote has no config for it; the
/// type still comes from the git-annex branch's remote.log.
#[test]
fn special_remote_types_come_from_the_annex_branch() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let clone = dir.path().join("clone");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            repo.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    for (key, value) in [("user.name", "Test"), ("user.email", "test@example.com")] {
        git(&clone, &["config", key, value]);
    }
    git(&clone, &["annex", "init", "-q", "desktop"]);

    let support = open(&clone)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    let backup = support
        .annex
        .repositories
        .iter()
        .find(|repo| repo.special_name.as_deref() == Some("backup"))
        .expect("the special remote is listed");
    assert_eq!(backup.remote_name, None, "not enabled in this clone");
    assert_eq!(backup.special_type.as_deref(), Some("directory"));
    // A directory remote's path is local, so enabling asks for it again.
    let enable = |params: Vec<String>| {
        run(
            &clone,
            LargeFileCommand::AnnexEnableRemote {
                name: "backup".into(),
                params,
            },
        )
    };
    assert!(enable(Vec::new()).unwrap_err().contains("directory="));
    enable(vec![format!(
        "directory={}",
        dir.path().join("store").display()
    )])
    .unwrap();
    let enabled = open(&clone)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(enabled.annex.repositories.iter().any(|repo| {
        repo.remote_name.as_deref() == Some("backup")
            && repo.special_type.as_deref() == Some("directory")
    }));
    let origin = support
        .annex
        .repositories
        .iter()
        .find(|repo| repo.description.contains("laptop"))
        .expect("the origin clone is listed");
    assert_eq!(origin.special_type, None, "a git repository has no type");
}

#[test]
fn unused_lists_old_versions_and_drop_unused_honours_numcopies() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let unused = || {
        open(&repo)
            .annex_unused_cancellable(&CancellationToken::new())
            .unwrap()
    };
    assert!(unused().entries.is_empty());

    git(&repo, &["annex", "unlock", "-q", "big.bin"]);
    fs::write(repo.join("big.bin"), vec![8u8; 100]).unwrap();
    git(&repo, &["annex", "add", "-q", "big.bin"]);
    git(&repo, &["commit", "-qm", "new big.bin"]);
    let listed = unused();
    assert_eq!(listed.entries.len(), 1, "{listed:?}");
    assert_eq!(
        listed.entries[0].kind,
        gitcomet_core::large_files::AnnexUnusedKind::Unused
    );
    assert_eq!(listed.known_bytes(), 4096);

    let refused = run(&repo, LargeFileCommand::AnnexDropUnused { force: false }).unwrap_err();
    assert!(refused.contains("Could not verify"), "{refused}");
    assert_eq!(
        unused().entries.len(),
        1,
        "a refused drop keeps the content"
    );

    git(
        &repo,
        &["annex", "copy", "-q", "--unused", "--to", "backup"],
    );
    run(&repo, LargeFileCommand::AnnexDropUnused { force: false }).unwrap();
    assert!(unused().entries.is_empty());
}

#[test]
fn assistant_counts_as_running_only_while_its_process_lives() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let running = || {
        open(&repo)
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap()
            .annex
            .assistant_running
    };
    assert!(!running());
    let pid_file = repo.join(".git/annex/daemon.pid");
    fs::write(&pid_file, format!("{}\n", std::process::id())).unwrap();
    assert!(running());
    if cfg!(unix) {
        // Left behind by a killed assistant.
        let mut child = Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        fs::write(&pid_file, format!("{dead}\n")).unwrap();
        assert!(!running());
    }
}

/// git-annex merges fetched `git-annex` branches whenever it reads the branch,
/// rewriting refs, its index and `.git/config`. The support summary loads in
/// the background, so it must read without merging; a command the user starts
/// is where that merge belongs.
#[test]
fn support_load_never_merges_fetched_annex_branches() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let origin = dir.path().join("origin.git");
    git(
        dir.path(),
        &["init", "-q", "--bare", origin.to_str().unwrap()],
    );
    git(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&repo, &["annex", "sync", "-q", "--no-content", "origin"]);

    let other = dir.path().join("other");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    for (key, value) in [("user.name", "Other"), ("user.email", "other@example.com")] {
        git(&other, &["config", key, value]);
    }
    git(&other, &["annex", "init", "-q", "other"]);
    git(&other, &["annex", "sync", "-q", "--no-content", "origin"]);
    git(&repo, &["fetch", "-q", "origin"]);

    let head_before = git(&repo, &["rev-parse", "refs/heads/git-annex"]);
    assert_ne!(
        head_before,
        git(&repo, &["rev-parse", "refs/remotes/origin/git-annex"]),
        "fixture: origin's git-annex branch has news"
    );
    let config_before = fs::read(repo.join(".git/config")).unwrap();
    open(&repo)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert_eq!(
        git(&repo, &["rev-parse", "refs/heads/git-annex"]),
        head_before,
        "a background read merged the fetched git-annex branch"
    );
    assert_eq!(fs::read(repo.join(".git/config")).unwrap(), config_before);
}

/// The support summary is a background read, so it must not take credentials
/// staged for a command the user is retrying (git-annex reaches remotes
/// through Git's credentials).
#[test]
fn support_load_does_not_consume_credentials_staged_for_a_command() {
    use gitcomet_core::auth::{
        GitAuthKind, ScopedStagedGitAuth, StagedGitAuth, take_staged_git_auth,
    };
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let opened = open(&repo);
    let _auth = ScopedStagedGitAuth::stage(StagedGitAuth {
        kind: GitAuthKind::UsernamePassword,
        username: Some("alice".into()),
        secret: "test-token".into(),
    });
    opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    let pending = take_staged_git_auth()
        .expect("the support probe must leave the retried command's credentials staged");
    assert_eq!(pending.username.as_deref(), Some("alice"));
}

/// Every progress event becomes a store message and a repaint. git-annex
/// prints one per file at least, hundreds a second for small files, so the
/// reader forwards at most one per interval (and always the last).
#[test]
fn transfer_progress_is_coalesced_for_many_small_files() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let names: Vec<PathBuf> = (0..200)
        .map(|i| {
            let name = PathBuf::from(format!("small/f{i}.bin"));
            fs::create_dir_all(repo.join("small")).unwrap();
            fs::write(repo.join(&name), format!("small file {i}\n").repeat(100)).unwrap();
            name
        })
        .collect();
    git(&repo, &["annex", "add", "-q", "small"]);
    git(&repo, &["commit", "-qm", "small files"]);
    run(
        &repo,
        LargeFileCommand::AnnexCopy {
            paths: vec![PathBuf::from("small")],
            to: "backup".into(),
        },
    )
    .unwrap();
    git(&repo, &["annex", "drop", "-q", "small"]);

    let (sender, receiver) = std::sync::mpsc::channel();
    let context =
        gitcomet_core::git_operation::GitOperationContext::new("annex get", move |_, event| {
            let _ = sender.send(event);
        });
    let started = std::time::Instant::now();
    {
        let _scope = gitcomet_core::git_operation::attach(&context);
        run(
            &repo,
            LargeFileCommand::AnnexGet {
                paths: vec![PathBuf::from("small")],
                from: None,
            },
        )
        .unwrap();
    }
    let elapsed = started.elapsed();
    let progress: Vec<_> = receiver
        .try_iter()
        .filter_map(|event| match event {
            gitcomet_core::git_operation::GitOperationEvent::TransferProgress(progress) => {
                Some(progress)
            }
            _ => None,
        })
        .collect();
    // One per 100 ms, plus the final one.
    let budget = (elapsed.as_millis() / 100) as usize + 2;
    assert!(
        progress.len() <= budget,
        "{} progress events for {} files in {elapsed:?} (budget {budget})",
        progress.len(),
        names.len()
    );
    let last = progress.last().expect("the final progress still arrives");
    assert_eq!(last.bytes_done, last.bytes_total);
}

#[cfg(unix)]
/// Runs `name` again in a child process with `env` set, returning true in the
/// child. PATH and the silence deadline are process-wide, so tests that change
/// them must not share a process with the others.
fn in_child(name: &str, env: &[(&str, std::ffi::OsString)]) -> bool {
    const CHILD_ENV: &str = "GITCOMET_ANNEX_TEST_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        return true;
    }
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args(["--exact", name, "--nocapture"])
        .env(CHILD_ENV, "1");
    for (key, value) in env {
        command.env(key, value);
    }
    let status = command.status().expect("run child test");
    assert!(status.success(), "{name} failed in its child process");
    false
}

#[cfg(unix)]
/// PATH with `dir` in front, so its programs shadow installed ones.
fn path_with(dir: &Path) -> std::ffi::OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(std::env::split_paths(&path));
    std::env::join_paths(dirs).unwrap()
}

#[cfg(unix)]
fn write_script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
/// A `git-annex` that logs each call as `cwd<TAB>args` and, like git-annex
/// before 10.20230626, has no `pull` or `push`. Returns its directory.
fn old_git_annex_shim(root: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let real = std::env::split_paths(&path)
        .map(|dir| dir.join("git-annex"))
        .find(|candidate| candidate.is_file())?;
    let dir = root.join("shim");
    fs::create_dir_all(&dir).unwrap();
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\t%s\\n' \"$PWD\" \"$*\" >> '{log}'\n\
         for arg in \"$@\"; do case \"$arg\" in -*) ;; *) sub=\"$arg\"; break ;; esac; done\n\
         case \"$sub\" in pull|push)\n\
           printf \"Invalid argument \\`%s'\\n\\nUsage: git-annex COMMAND\\n\" \"$sub\" >&2; exit 1 ;;\n\
         esac\n\
         exec '{real}' \"$@\"\n",
        log = dir.join("calls.log").display(),
        real = real.display(),
    );
    write_script(&dir.join("git-annex"), &script);
    Some(dir)
}

#[cfg(unix)]
/// Shim calls made with `repo` as the working directory.
fn shim_calls(shim: &Path, repo: &Path) -> Vec<String> {
    let repo = fs::canonicalize(repo).unwrap();
    fs::read_to_string(shim.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(cwd, _)| fs::canonicalize(cwd).is_ok_and(|cwd| cwd == repo))
        .map(|(_, args)| args.to_string())
        .collect()
}

#[cfg(unix)]
/// The test's fixed scratch root, shared by the parent and its child.
fn child_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("gitcomet-{name}-{}", std::process::id()))
}

/// Ubuntu 22.04 and Debian 11 ship git-annex 8.x, which has `sync` but not
/// the one-way `pull` and `push` split out in 10.20230626.
#[cfg(unix)]
#[test]
fn annex_pull_and_push_fall_back_to_one_way_sync_on_older_git_annex() {
    require_annex!();
    const NAME: &str = "annex_pull_and_push_fall_back_to_one_way_sync_on_older_git_annex";
    let root = match std::env::var_os("GITCOMET_ANNEX_TEST_ROOT") {
        Some(root) => PathBuf::from(root),
        None => {
            let root = child_root("old-annex");
            fs::create_dir_all(&root).unwrap();
            let shim = old_git_annex_shim(&root).expect("git-annex on PATH");
            let passed = in_child(
                NAME,
                &[
                    ("PATH", path_with(&shim)),
                    ("GITCOMET_ANNEX_TEST_ROOT", root.clone().into()),
                ],
            );
            let _ = fs::remove_dir_all(&root);
            if !passed {
                return;
            }
            unreachable!("the parent never runs the body")
        }
    };
    let repo = init_annex_repo(&root);
    let opened = open(&repo);
    for (command, direction) in [
        (LargeFileCommand::AnnexPull { content: false }, "--no-push"),
        (LargeFileCommand::AnnexPush { content: true }, "--no-pull"),
    ] {
        opened
            .run_large_file_command(&command)
            .unwrap_or_else(|e| panic!("{command:?}: {e}"));
        let calls = shim_calls(&root.join("shim"), &repo);
        assert!(
            calls
                .iter()
                .any(|call| call.contains("sync") && call.contains(direction)),
            "{command:?} must fall back to a one-way sync: {calls:?}"
        );
    }
}

/// Status and its per-row large-file state are background reads that run on
/// every refresh; they must never start git-annex, even for unlocked rows.
#[cfg(unix)]
#[test]
fn status_rows_never_run_git_annex() {
    require_annex!();
    const NAME: &str = "status_rows_never_run_git_annex";
    let root = match std::env::var_os("GITCOMET_ANNEX_TEST_ROOT") {
        Some(root) => PathBuf::from(root),
        None => {
            let root = child_root("status-no-annex");
            fs::create_dir_all(&root).unwrap();
            let shim = old_git_annex_shim(&root).expect("git-annex on PATH");
            let passed = in_child(
                NAME,
                &[
                    ("PATH", path_with(&shim)),
                    ("GITCOMET_ANNEX_TEST_ROOT", root.clone().into()),
                ],
            );
            let _ = fs::remove_dir_all(&root);
            if !passed {
                return;
            }
            unreachable!("the parent never runs the body")
        }
    };
    let repo = init_annex_repo(&root);
    git(&repo, &["annex", "unlock", "big.bin"]);
    fs::write(repo.join("other.bin"), vec![9u8; 2048]).unwrap();
    git(&repo, &["annex", "add", "-q", "other.bin"]);
    git(&repo, &["annex", "unlock", "other.bin"]);
    let shim = root.join("shim");
    let before = shim_calls(&shim, &repo).len();

    let opened = open(&repo);
    let status = opened.status().unwrap();
    let files = opened
        .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        files.staged[Path::new("other.bin")].in_local_store,
        Some(true)
    );
    opened
        .uncommitted_line_stats_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    let calls = shim_calls(&shim, &repo);
    assert!(
        calls.len() == before,
        "status ran git-annex: {:?}",
        &calls[before..]
    );
}

#[cfg(unix)]
/// A minimal external special remote that stores keys in `directory=` and
/// takes `@SECS@` seconds per transfer, printing nothing meanwhile.
const SLOW_REMOTE: &str = r#"#!/bin/sh
dir=""
echo VERSION 1
while IFS= read -r line; do
  set -- $line
  case "$1" in
    INITREMOTE) echo INITREMOTE-SUCCESS ;;
    PREPARE)
      echo "GETCONFIG directory"
      IFS= read -r reply
      dir="${reply#VALUE }"
      echo PREPARE-SUCCESS ;;
    TRANSFER)
      op="$2"; key="$3"; file="${line#TRANSFER $op $key }"
      sleep @SECS@
      if [ "$op" = STORE ]; then cp "$file" "$dir/$key"; else cp "$dir/$key" "$file"; fi
      echo "TRANSFER-SUCCESS $op $key" ;;
    CHECKPRESENT)
      if [ -e "$dir/$2" ]; then echo "CHECKPRESENT-SUCCESS $2"; else echo "CHECKPRESENT-FAILURE $2"; fi ;;
    REMOVE) rm -f "$dir/$2"; echo "REMOVE-SUCCESS $2" ;;
    *) echo UNSUPPORTED-REQUEST ;;
  esac
done
"#;

/// git-annex prints nothing while one file moves (no terminal, and `sync`,
/// `pull` and `push` have no JSON progress), so a long transfer must not be
/// ended by the silence deadline; cancelling it is the user's call.
#[cfg(unix)]
#[test]
fn a_silent_content_transfer_outlives_the_silence_deadline() {
    require_annex!();
    const NAME: &str = "a_silent_content_transfer_outlives_the_silence_deadline";
    let root = match std::env::var_os("GITCOMET_ANNEX_TEST_ROOT") {
        Some(root) => PathBuf::from(root),
        None => {
            let root = child_root("silent-transfer");
            let bin = root.join("bin");
            fs::create_dir_all(&bin).unwrap();
            write_script(
                &bin.join("git-annex-remote-slowtest"),
                &SLOW_REMOTE.replace("@SECS@", "4"),
            );
            let passed = in_child(
                NAME,
                &[
                    ("PATH", path_with(&bin)),
                    ("GITCOMET_GIT_COMMAND_TIMEOUT_SECS", "2".into()),
                    ("GITCOMET_ANNEX_TEST_ROOT", root.clone().into()),
                ],
            );
            let _ = fs::remove_dir_all(&root);
            if !passed {
                return;
            }
            unreachable!("the parent never runs the body")
        }
    };
    let repo = init_annex_repo(&root);
    git(
        &repo,
        &[
            "annex",
            "initremote",
            "-q",
            "slow",
            "type=external",
            "externaltype=slowtest",
            "encryption=none",
            &format!("directory={}", root.join("slow-store").display()),
        ],
    );
    fs::create_dir_all(root.join("slow-store")).unwrap();
    open(&repo)
        .run_large_file_command(&LargeFileCommand::AnnexCopy {
            paths: paths("big.bin"),
            to: "slow".into(),
        })
        .map(|_| ())
        .unwrap_or_else(|e| panic!("copy with JSON progress was cut off: {e}"));
    git(&repo, &["annex", "drop", "-q", "--from", "slow", "big.bin"]);
    open(&repo)
        .run_large_file_command(&LargeFileCommand::AnnexPush { content: true })
        .unwrap_or_else(|e| panic!("push --content was cut off: {e}"));
    let whereis = git(&repo, &["annex", "whereis", "big.bin"]);
    assert!(whereis.contains("slow"), "{whereis}");
}

/// The summary reads git-annex's logs instead of running `git annex info`, so
/// it must list what `info` lists: same repositories, descriptions and trust.
#[test]
fn repositories_read_from_logs_match_git_annex_info() {
    require_annex!();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_annex_repo(dir.path());
    let origin = dir.path().join("origin.git");
    git(
        dir.path(),
        &["init", "-q", "--bare", origin.to_str().unwrap()],
    );
    git(&origin, &["annex", "init", "-q", "shared store"]);
    git(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&repo, &["annex", "sync", "-q", "--no-content", "origin"]);
    git(&repo, &["annex", "untrust", "-q", "backup"]);
    git(&repo, &["annex", "trust", "-q", "--force", "origin"]);
    git(&repo, &["annex", "describe", "-q", "here", "my laptop"]);
    git(&repo, &["annex", "dead", "-q", "web"]);
    run(&repo, LargeFileCommand::AnnexNumcopies { copies: 3 }).unwrap();

    let support = open(&repo)
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    let ours: Vec<_> = support
        .annex
        .repositories
        .iter()
        .map(|repo| {
            (
                repo.uuid.clone(),
                repo.description.clone(),
                repo.trust.label().to_string(),
                repo.here,
            )
        })
        .collect();
    let info: serde_json::Value =
        serde_json::from_str(&git(&repo, &["annex", "info", "--fast", "--json"])).unwrap();
    let mut theirs = Vec::new();
    for trust in ["trusted", "semitrusted", "untrusted"] {
        for entry in info[format!("{trust} repositories")].as_array().unwrap() {
            theirs.push((
                entry["uuid"].as_str().unwrap().to_string(),
                entry["description"].as_str().unwrap().to_string(),
                trust.to_string(),
                entry["here"].as_bool().unwrap(),
            ));
        }
    }
    assert_eq!(ours, theirs);
    assert_eq!(support.annex.numcopies, Some(3));
}
