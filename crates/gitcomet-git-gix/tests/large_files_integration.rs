//! Git LFS and git-annex detection against real repositories. LFS fixtures
//! use the real `git-lfs` filter (installed on every CI lane); annex fixtures
//! are built by hand so they need no git-annex binary.

#[path = "support/test_git_env.rs"]
mod test_git_env;

use gitcomet_core::domain::{DiffArea, DiffTarget, FileStatus, FileStatusKind, RepoStatus};
use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileWorktree};
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ANNEX_KEY: &str =
    "SHA256E-s5--2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824.bin";

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

fn git_lfs_available() -> bool {
    Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(repo, &["config", key, value]);
    }
}

/// A repository whose `*.bin` files go through the real LFS filter.
fn init_lfs_repo(repo: &Path) {
    init_repo(repo);
    configure_lfs_filters(repo);
    fs::write(
        repo.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n*.psd filter=lfs diff=lfs merge=lfs -text lockable\n",
    )
    .unwrap();
    fs::write(repo.join("a.bin"), b"large file contents\n").unwrap();
    fs::write(repo.join("notes.txt"), b"plain\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "init"]);
}

fn status(
    repo: &Path,
) -> (
    RepoStatus,
    std::sync::Arc<dyn gitcomet_core::services::GitRepository>,
) {
    let opened = GixBackend.open(repo).unwrap();
    let status = opened.status().unwrap();
    (status, opened)
}

fn row(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: PathBuf::from(path),
        kind,
        conflict: None,
    }
}

#[test]
fn lfs_repository_reports_filter_patterns_and_store() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);

    let (_, opened) = status(&repo);
    let support = opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(support.lfs.filter_configured && support.lfs.filter_required);
    assert!(
        support.lfs.has_local_store,
        "committing through clean fills the store"
    );
    let patterns = support
        .lfs
        .tracked_patterns
        .iter()
        .map(|p| p.pattern.as_str())
        .collect::<Vec<_>>();
    assert_eq!(patterns, ["*.bin", "*.psd"]);
    assert!(support.lfs.has_lockable_patterns);
    assert!(support.is_active() && !support.annex.in_use());
}

#[test]
fn lfs_lock_capability_detects_separate_attribute_rules() {
    for (attributes, overrides, lockable) in [
        ("*.bin filter=lfs\n*.bin lockable\n", "", true),
        ("*.bin lockable\n*.bin filter=lfs\n", "", true),
        ("*.bin filter=lfs\na.bin lockable\n", "", true),
        ("*.bin filter=lfs\n", "a.bin lockable\n", true),
        ("*.bin filter=lfs\n*.bin -lockable\n", "", false),
        ("*.bin filter=lfs\n*.bin !lockable\n", "", false),
        ("*.bin filter=lfs lockable -lockable\n", "", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        fs::write(repo.join(".gitattributes"), attributes).unwrap();
        fs::write(repo.join(".git/info/attributes"), overrides).unwrap();
        fs::write(
            repo.join("a.bin"),
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
                "1".repeat(64)
            ),
        )
        .unwrap();
        git(repo, &["add", "."]);

        let (status, opened) = status(repo);
        let support = opened
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap();
        let files = opened
            .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
            .unwrap();
        assert_eq!(files.staged[Path::new("a.bin")].lockable, lockable);
        assert_eq!(
            support.lfs.has_lockable_patterns, lockable,
            "{attributes:?}, {overrides:?}"
        );
        assert_eq!(support.lfs.tracked_patterns.len(), 1);
        assert_eq!(support.lfs.tracked_patterns[0].pattern, "*.bin");
    }
}

#[test]
fn plain_repository_is_not_active() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let (_, opened) = status(&repo);
    let support = opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(!support.is_active(), "{support:?}");
}

#[test]
fn plain_pointer_text_diff_regression() {
    use gitcomet_core::domain::{CommitId, DiffPreviewTextSide};
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_repo(repo);
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
        "1".repeat(64)
    );
    fs::write(repo.join("example.png"), &pointer).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "pointer example"]);
    let opened = GixBackend.open(repo).unwrap();
    let target = DiffTarget::Commit {
        commit_id: CommitId(git(repo, &["rev-parse", "HEAD"]).trim().into()),
        path: Some("example.png".into()),
    };
    let text = opened.diff_file_text(&target).unwrap().unwrap();
    assert!(
        text.new_large.is_none(),
        "ordinary text must remain diffable"
    );
    assert_eq!(
        source_text(text.new_source.as_ref()).as_deref(),
        Some(pointer.as_str())
    );
    let preview = opened
        .diff_preview_text_file(&target, DiffPreviewTextSide::New)
        .unwrap()
        .unwrap();
    assert!(preview.large_file.is_none());
    let image = opened.diff_file_image(&target).unwrap().unwrap();
    assert!(image.new_large.is_none());
    assert_eq!(image.new.as_deref(), Some(pointer.as_bytes()));
}

#[test]
fn lfs_rows_report_pointer_worktree_and_presence() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    fs::write(repo.join("a.bin"), b"new large contents\n").unwrap();
    fs::write(repo.join("b.psd"), b"layered image\n").unwrap();
    git(&repo, &["add", "b.psd"]);

    let (status, opened) = status(&repo);
    let files = opened
        .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();

    let staged = &files.staged[Path::new("b.psd")];
    let LargeFilePointer::Lfs(pointer) = &staged.pointer else {
        panic!("expected an LFS pointer, got {staged:?}");
    };
    assert_eq!(pointer.size, 14);
    assert_eq!(staged.in_local_store, Some(true));
    assert_eq!(staged.worktree, None, "staged rows describe the index");
    assert!(staged.lockable);

    let unstaged = &files.unstaged[Path::new("a.bin")];
    assert_eq!(unstaged.worktree, Some(LargeFileWorktree::Content));
    assert_eq!(
        unstaged.pointer.size(),
        Some(20),
        "the committed pointer's size"
    );
    assert!(!unstaged.lockable);
    assert!(!files.unstaged.contains_key(Path::new("notes.txt")));
}

/// A skip-smudge checkout leaves pointer text in the worktree and nothing in
/// the store: the row must say the content is missing here.
#[test]
fn lfs_pointer_only_worktree_reports_missing_content() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    let pointer = git(&repo, &["cat-file", "blob", "HEAD:a.bin"]);
    fs::remove_dir_all(repo.join(".git/lfs/objects")).unwrap();
    // Pointer text in the worktree; touch so status compares content.
    fs::write(repo.join("a.bin"), &pointer).unwrap();

    let opened = GixBackend.open(&repo).unwrap();
    let status = RepoStatus {
        staged: Default::default(),
        unstaged: vec![row("a.bin", FileStatusKind::Modified)].into(),
    };
    let files = opened
        .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    let state = &files.unstaged[Path::new("a.bin")];
    assert_eq!(state.worktree, Some(LargeFileWorktree::Pointer));
    assert_eq!(state.in_local_store, Some(false));
    assert!(state.content_missing());
}

#[cfg(unix)]
#[test]
fn annex_locked_and_unlocked_rows_are_recognised() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let object_dir = repo.join(format!(
        ".git/annex/objects/{ANNEX_KEY_MIXED_DIR}/{ANNEX_KEY}"
    ));
    fs::create_dir_all(&object_dir).unwrap();
    fs::write(object_dir.join(ANNEX_KEY), b"hello").unwrap();
    let link_target = format!(".git/annex/objects/{ANNEX_KEY_MIXED_DIR}/{ANNEX_KEY}/{ANNEX_KEY}");
    symlink(&link_target, repo.join("present.bin")).unwrap();
    let missing_target = link_target.replace(ANNEX_KEY, &ANNEX_KEY.replace("2cf", "0cf"));
    symlink(&missing_target, repo.join("absent.bin")).unwrap();
    fs::write(
        repo.join("unlocked.bin"),
        format!("/annex/objects/{ANNEX_KEY}\n"),
    )
    .unwrap();
    fs::write(repo.join("README"), "annex fixture\n").unwrap();
    git(&repo, &["add", "README"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["branch", "git-annex"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "config",
            "annex.uuid",
            "0d8f0d6e-3b77-4a0e-9b1a-7b0e1f2d3c4b",
        ],
    );

    let opened = GixBackend.open(&repo).unwrap();
    let support = opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(support.annex.has_annex_dir && support.annex.has_annex_branch);
    assert!(support.annex.initialized() && support.is_active());

    let status = opened.status().unwrap();
    let files = opened
        .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    let state = |name: &str| files.staged[Path::new(name)].clone();
    let present = state("present.bin");
    let LargeFilePointer::Annex(key) = &present.pointer else {
        panic!("expected an annex key, got {present:?}");
    };
    assert_eq!((&*key.raw, key.size), (ANNEX_KEY, Some(5)));
    assert_eq!(present.in_local_store, Some(true));
    assert_eq!(state("absent.bin").in_local_store, Some(false));
    assert_eq!(
        state("unlocked.bin").in_local_store,
        Some(true),
        "found where git-annex keeps it, without running git-annex"
    );
}

fn source_text(source: Option<&gitcomet_core::domain::FileDiffTextSource>) -> Option<String> {
    source.map(|source| fs::read_to_string(&source.path).expect("readable diff source"))
}

fn unstaged(path: &str) -> DiffTarget {
    DiffTarget::WorkingTree {
        path: PathBuf::from(path),
        area: DiffArea::Unstaged,
    }
}

/// With content in the store and the worktree, the diff shows the real text
/// on both sides instead of two pointers.
#[test]
fn lfs_text_diff_reads_real_content_when_present() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    fs::write(repo.join("a.bin"), b"new large contents\n").unwrap();

    let opened = GixBackend.open(&repo).unwrap();
    let diff = opened.diff_file_text(&unstaged("a.bin")).unwrap().unwrap();
    let (old, new) = (
        diff.old_large.clone().unwrap(),
        diff.new_large.clone().unwrap(),
    );
    assert_eq!(
        (old.content, new.content),
        (LargeFileContent::Available, LargeFileContent::Available)
    );
    assert_eq!(
        source_text(diff.old_source.as_ref()).as_deref(),
        Some("large file contents\n")
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("new large contents\n")
    );
    assert!(gitcomet_core::large_files::large_file_sides_show_content(
        Some(&old),
        Some(&new)
    ));

    let plain = opened
        .diff_file_text(&unstaged("notes.txt"))
        .unwrap()
        .unwrap();
    assert!(plain.old_large.is_none() && plain.new_large.is_none());
}

/// A pointer-only checkout with an empty store: both sides are pointers and
/// the card, not a pointer diff, must explain the file.
#[test]
fn lfs_text_diff_reports_missing_content() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    let pointer = git(&repo, &["cat-file", "blob", "HEAD:a.bin"]);
    fs::remove_dir_all(repo.join(".git/lfs/objects")).unwrap();
    fs::write(repo.join("a.bin"), pointer.replace("size 20", "size 21")).unwrap();

    let opened = GixBackend.open(&repo).unwrap();
    let diff = opened.diff_file_text(&unstaged("a.bin")).unwrap().unwrap();
    let old = diff.old_large.clone().expect("old side is a pointer");
    let new = diff.new_large.clone().expect("new side is a pointer");
    assert_eq!(old.content, LargeFileContent::MissingLocally);
    assert_eq!(new.content, LargeFileContent::MissingLocally);
    assert_eq!(
        (old.pointer.size(), new.pointer.size()),
        (Some(20), Some(21))
    );
    assert!(!gitcomet_core::large_files::large_file_sides_show_content(
        Some(&old),
        Some(&new)
    ));
}

#[test]
fn lfs_image_diff_decodes_both_sides_from_the_store() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    fs::write(
        repo.join(".gitattributes"),
        "*.png filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    let before = b"\x89PNG\r\n\x1a\nbefore".to_vec();
    let after = b"\x89PNG\r\n\x1a\nafter".to_vec();
    fs::write(repo.join("pic.png"), &before).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "image"]);
    fs::write(repo.join("pic.png"), &after).unwrap();
    git(&repo, &["add", "pic.png"]);

    let opened = GixBackend.open(&repo).unwrap();
    let image = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: PathBuf::from("pic.png"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        image.old.as_deref(),
        Some(before.as_slice()),
        "HEAD side from the store"
    );
    assert_eq!(
        image.new.as_deref(),
        Some(after.as_slice()),
        "index side from the store"
    );
}

#[test]
fn commit_file_rows_carry_large_file_state() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    let head = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();

    let opened = GixBackend.open(&repo).unwrap();
    let details = opened
        .commit_details(&gitcomet_core::domain::CommitId(head.into()))
        .unwrap();
    let file = |name: &str| {
        details
            .files
            .iter()
            .find(|file| file.path == Path::new(name))
            .unwrap_or_else(|| panic!("{name} in {:?}", details.files))
    };
    let lfs = file("a.bin")
        .large_file
        .clone()
        .expect("a.bin is an LFS pointer");
    assert_eq!(lfs.pointer.size(), Some(20));
    assert_eq!(lfs.in_local_store, Some(true));
    assert!(file("notes.txt").large_file.is_none());
    assert!(file(".gitattributes").large_file.is_none());
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn run_lfs(repo: &Path, command: gitcomet_core::large_files::LargeFileCommand) -> String {
    let opened = GixBackend.open(repo).unwrap();
    let output = opened
        .run_large_file_command(&command)
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    format!("{}{}", output.stdout, output.stderr)
}

fn configure_lfs_filters(repo: &Path) {
    for (key, value) in [
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.required", "true"),
    ] {
        git(repo, &["config", key, value]);
    }
}

/// End to end over a `file://` remote (git-lfs's standalone adapter): push
/// every object, clone without content, then download one path by name.
#[test]
fn lfs_push_all_then_pull_downloads_only_the_named_path() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let remote = dir.path().join("remote.git");
    init_lfs_repo(&repo);
    fs::write(repo.join("b.bin"), b"second large file\n").unwrap();
    git(&repo, &["add", "b.bin"]);
    git(&repo, &["commit", "-qm", "second"]);
    git(
        dir.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git(&repo, &["remote", "add", "origin", &file_url(&remote)]);
    git(&repo, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsPushAll {
            remote: "origin".into(),
        },
    );

    let clone = dir.path().join("clone");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--branch",
            "main",
            &file_url(&remote),
            clone.to_str().unwrap(),
        ],
    );
    configure_lfs_filters(&clone);
    let pointer = |name: &str| fs::read_to_string(clone.join(name)).unwrap();
    assert!(
        pointer("a.bin").starts_with("version https://git-lfs"),
        "cloned without smudge"
    );

    run_lfs(
        &clone,
        gitcomet_core::large_files::LargeFileCommand::LfsPull {
            paths: vec![PathBuf::from("a.bin")],
        },
    );
    assert_eq!(pointer("a.bin"), "large file contents\n");
    assert!(
        pointer("b.bin").starts_with("version https://git-lfs"),
        "only the named path is downloaded"
    );
    let status = git(&clone, &["status", "--porcelain"]);
    assert!(
        status.trim().is_empty(),
        "checkout leaves a clean tree: {status}"
    );
}

#[test]
fn lfs_track_writes_attributes_and_converts_existing_files() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    fs::write(repo.join("scene.blend"), b"binary scene\n").unwrap();
    git(&repo, &["add", "scene.blend"]);
    git(&repo, &["commit", "-qm", "scene in git"]);

    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsTrack {
            patterns: vec!["*.blend".into()],
            filename: false,
            lockable: true,
            renormalize: vec![PathBuf::from("scene.blend")],
        },
    );
    let attributes = fs::read_to_string(repo.join(".gitattributes")).unwrap();
    assert!(
        attributes.contains("*.blend filter=lfs diff=lfs merge=lfs -text lockable"),
        "{attributes}"
    );
    let staged = git(&repo, &["cat-file", "blob", ":scene.blend"]);
    assert!(
        staged.starts_with("version https://git-lfs"),
        "renormalized: {staged}"
    );
    let staged_names = git(&repo, &["diff", "--cached", "--name-only"]);
    assert!(staged_names.contains(".gitattributes"), "{staged_names}");
}

#[test]
fn lfs_install_local_configures_filters_and_hooks() {
    if !git_lfs_available() {
        eprintln!("skipping: git-lfs is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsInstall,
    );
    assert_eq!(
        git(&repo, &["config", "--local", "filter.lfs.process"]).trim(),
        "git-lfs filter-process"
    );
    assert!(repo.join(".git/hooks/pre-push").is_file());
    let opened = GixBackend.open(&repo).unwrap();
    assert!(
        opened
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap()
            .lfs
            .filter_configured
    );
}

#[test]
fn lfs_first_tracking_stages_new_attributes_with_the_pointer() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_repo(repo);
    configure_lfs_filters(repo);
    fs::write(
        repo.join("scene.blend"),
        b"scene previously stored in Git\n",
    )
    .unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "ordinary file"]);
    run_lfs(
        repo,
        gitcomet_core::large_files::LargeFileCommand::LfsTrack {
            patterns: vec!["*.blend".into()],
            filename: false,
            lockable: false,
            renormalize: vec!["scene.blend".into()],
        },
    );
    let names = git(repo, &["diff", "--cached", "--name-only"]);
    assert!(
        names.lines().any(|name| name == ".gitattributes"),
        "attributes must accompany the pointer: {names}"
    );
    assert!(git(repo, &["show", ":scene.blend"]).starts_with("version https://git-lfs"));
}

#[test]
fn lfs_tracking_a_literal_filename_does_not_track_a_glob_match() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_lfs_repo(repo);
    let selected = "project [1].blend";
    let other = "project 1.blend";
    for name in [selected, other] {
        fs::write(repo.join(name), "ordinary file\n").unwrap();
    }
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "unmanaged files"]);
    fs::write(repo.join(other), "unstaged changes must stay unstaged\n").unwrap();
    run_lfs(
        repo,
        gitcomet_core::large_files::LargeFileCommand::LfsTrack {
            patterns: vec![format!("/{selected}")],
            filename: true,
            lockable: false,
            renormalize: vec![selected.into()],
        },
    );
    let attributes = git(repo, &["check-attr", "filter", "--", selected, other]);
    assert!(
        attributes.contains("project [1].blend: filter: lfs"),
        "{attributes}"
    );
    assert!(
        attributes.contains("project 1.blend: filter: unspecified"),
        "{attributes}"
    );
    assert!(git(repo, &["show", &format!(":{selected}")]).starts_with("version https://git-lfs"));
    assert_eq!(
        git(repo, &["show", &format!(":{other}")]),
        "ordinary file\n"
    );
}

/// Three revisions ensure a historical download cannot pass by fetching HEAD.
fn clone_lfs_history(root: &Path) -> (PathBuf, String) {
    let repo = root.join("source");
    init_lfs_repo(&repo);
    fs::write(repo.join("a.bin"), "middle version\n").unwrap();
    git(&repo, &["add", "a.bin"]);
    git(&repo, &["commit", "-qm", "middle"]);
    let middle = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    fs::write(repo.join("a.bin"), "current version\n").unwrap();
    git(&repo, &["add", "a.bin"]);
    git(&repo, &["commit", "-qm", "current"]);
    let remote = root.join("remote.git");
    git(root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&repo, &["remote", "add", "origin", &file_url(&remote)]);
    git(&repo, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsPushAll {
            remote: "origin".into(),
        },
    );
    let clone = root.join("clone");
    git(
        root,
        &[
            "clone",
            "-q",
            "--branch",
            "main",
            &file_url(&remote),
            clone.to_str().unwrap(),
        ],
    );
    configure_lfs_filters(&clone);
    (clone, middle)
}

#[test]
fn lfs_explicit_download_overrides_fetch_exclusions() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, _) = clone_lfs_history(dir.path());
    git(&repo, &["config", "lfs.fetchexclude", "*.bin"]);
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsPull {
            paths: vec!["a.bin".into()],
        },
    );
    assert_eq!(
        fs::read_to_string(repo.join("a.bin")).unwrap(),
        "current version\n"
    );
    assert_eq!(
        git(&repo, &["config", "lfs.fetchexclude"]).trim(),
        "*.bin",
        "the override must not change user configuration"
    );
}

#[test]
fn lfs_positional_fetch_preserves_default_remote_selection() {
    use gitcomet_core::large_files::LargeFileCommand;
    if !git_lfs_available() {
        return;
    }
    for selection in ["single", "lfsdefault", "branch"] {
        let dir = tempfile::tempdir().unwrap();
        let (repo, middle) = clone_lfs_history(dir.path());
        git(&repo, &["remote", "rename", "origin", "upstream"]);
        git(&repo, &["config", "--unset", "branch.main.remote"]);
        if selection != "single" {
            git(
                &repo,
                &[
                    "remote",
                    "add",
                    "decoy",
                    &file_url(&dir.path().join("absent.git")),
                ],
            );
            git(
                &repo,
                &[
                    "config",
                    "remote.lfsdefault",
                    if selection == "branch" {
                        "decoy"
                    } else {
                        "upstream"
                    },
                ],
            );
        }
        if selection == "branch" {
            git(&repo, &["config", "branch.main.remote", "upstream"]);
        }
        let opened = GixBackend.open(&repo).unwrap();
        let target = DiffTarget::Commit {
            commit_id: gitcomet_core::domain::CommitId(middle.into()),
            path: Some("a.bin".into()),
        };
        opened
            .run_large_file_command(&LargeFileCommand::LfsFetchForDiff {
                target: target.clone(),
            })
            .unwrap();
        let diff = opened.diff_file_text(&target).unwrap().unwrap();
        assert_eq!(
            source_text(diff.new_source.as_ref()).as_deref(),
            Some("middle version\n"),
            "{selection}"
        );
    }
}

#[test]
fn lfs_diff_download_fetches_both_historical_sides_without_checkout() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, middle) = clone_lfs_history(dir.path());
    git(&repo, &["config", "lfs.fetchexclude", "*.bin"]);
    let before = fs::read(repo.join("a.bin")).unwrap();
    let target = DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId(middle.into()),
        path: Some("a.bin".into()),
    };
    let opened = GixBackend.open(&repo).unwrap();
    let diff = opened.diff_file_text(&target).unwrap().unwrap();
    assert_eq!(
        diff.old_large.unwrap().content,
        LargeFileContent::MissingLocally
    );
    assert_eq!(
        diff.new_large.unwrap().content,
        LargeFileContent::MissingLocally
    );
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsFetchForDiff {
            target: target.clone(),
        },
    );
    let diff = opened.diff_file_text(&target).unwrap().unwrap();
    assert_eq!(
        source_text(diff.old_source.as_ref()).as_deref(),
        Some("large file contents\n")
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("middle version\n")
    );
    assert_eq!(
        fs::read(repo.join("a.bin")).unwrap(),
        before,
        "history downloads must not check out the current file"
    );
}

#[test]
fn lfs_added_and_deleted_previews_show_payloads() {
    if !git_lfs_available() {
        return;
    }
    use gitcomet_core::domain::DiffPreviewTextSide;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_lfs_repo(repo);
    fs::write(repo.join("new.bin"), "new content\n").unwrap();
    git(repo, &["add", "new.bin"]);
    git(repo, &["rm", "a.bin"]);
    let opened = GixBackend.open(repo).unwrap();
    for (path, side, expected) in [
        ("new.bin", DiffPreviewTextSide::New, "new content\n"),
        ("a.bin", DiffPreviewTextSide::Old, "large file contents\n"),
    ] {
        let target = DiffTarget::WorkingTree {
            path: path.into(),
            area: DiffArea::Staged,
        };
        let preview = opened
            .diff_preview_text_file(&target, side)
            .unwrap()
            .unwrap();
        assert_eq!(
            preview.large_file.as_ref().unwrap().content,
            LargeFileContent::Available
        );
        assert_eq!(
            fs::read_to_string(preview.path).unwrap(),
            expected,
            "{path}"
        );
    }
}

#[test]
fn lfs_missing_images_do_not_send_pointer_text_to_the_decoder() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_lfs_repo(repo);
    fs::write(
        repo.join(".gitattributes"),
        "*.png filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    fs::write(repo.join("pic.png"), b"\x89PNG\r\n\x1a\nimage").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "image"]);
    fs::remove_dir_all(repo.join(".git/lfs/objects")).unwrap();
    let target = DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId(git(repo, &["rev-parse", "HEAD"]).trim().into()),
        path: Some("pic.png".into()),
    };
    let opened = GixBackend.open(repo).unwrap();
    let image = opened.diff_file_image(&target).unwrap().unwrap();
    assert_eq!(
        image.new_large.as_ref().unwrap().content,
        LargeFileContent::MissingLocally
    );
    assert!(
        image.new.is_none(),
        "missing image bytes must be described by the card instead of decoded as an image"
    );
}

#[test]
fn lfs_diff_download_handles_root_commits_and_revision_ranges() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, _) = clone_lfs_history(dir.path());
    let root = gitcomet_core::domain::CommitId(git(&repo, &["rev-parse", "HEAD~2"]).trim().into());
    let head = gitcomet_core::domain::CommitId(git(&repo, &["rev-parse", "HEAD"]).trim().into());
    let before = fs::read(repo.join("a.bin")).unwrap();
    let opened = GixBackend.open(&repo).unwrap();
    let root_target = DiffTarget::Commit {
        commit_id: root.clone(),
        path: Some("a.bin".into()),
    };
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsFetchForDiff {
            target: root_target.clone(),
        },
    );
    let preview = opened
        .diff_preview_text_file(
            &root_target,
            gitcomet_core::domain::DiffPreviewTextSide::New,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::read_to_string(preview.path).unwrap(),
        "large file contents\n"
    );
    let range = DiffTarget::CommitRange {
        from_commit_id: root,
        to_commit_id: Some(head),
        path: Some("a.bin".into()),
    };
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsFetchForDiff {
            target: range.clone(),
        },
    );
    let diff = opened.diff_file_text(&range).unwrap().unwrap();
    assert_eq!(
        source_text(diff.old_source.as_ref()).as_deref(),
        Some("large file contents\n")
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("current version\n")
    );
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), before);
}

#[test]
fn lfs_images_use_the_image_size_limit_instead_of_the_text_limit() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_lfs_repo(repo);
    fs::write(
        repo.join(".gitattributes"),
        "*.png filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    let mut bytes = vec![0; 17 * 1024 * 1024];
    bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    fs::write(repo.join("large.png"), &bytes).unwrap();
    git(repo, &["add", "."]);
    let opened = GixBackend.open(repo).unwrap();
    let target = DiffTarget::WorkingTree {
        path: "large.png".into(),
        area: DiffArea::Staged,
    };
    let text = opened.diff_file_text(&target).unwrap().unwrap();
    assert!(matches!(
        text.new_large.unwrap().content,
        LargeFileContent::TooLarge { .. }
    ));
    let image = opened.diff_file_image(&target).unwrap().unwrap();
    assert_eq!(
        image.new_large.unwrap().content,
        LargeFileContent::Available
    );
    assert_eq!(image.new.unwrap(), bytes);
}

#[test]
fn lfs_named_checkout_treats_glob_characters_literally() {
    if !git_lfs_available() {
        return;
    }
    for (selected, other) in [
        ("a[1].bin", "a1.bin"),
        ("clip.bin ", "clip.bin"),
        ("clip.bin\t", "clip.bin"),
        #[cfg(unix)]
        ("clip.bin\\", "clip.bin"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_lfs_repo(&repo);
        fs::write(repo.join(".gitattributes"), "*.bin* filter=lfs\n").unwrap();
        for name in [selected, other] {
            fs::write(repo.join(name), format!("{name} content\n")).unwrap();
        }
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "named files"]);
        let remote = dir.path().join("remote.git");
        git(
            dir.path(),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        git(&repo, &["remote", "add", "origin", &file_url(&remote)]);
        git(&repo, &["push", "-q", "-u", "origin", "HEAD:main"]);
        run_lfs(
            &repo,
            gitcomet_core::large_files::LargeFileCommand::LfsPushAll {
                remote: "origin".into(),
            },
        );
        for name in [selected, other] {
            fs::write(
                repo.join(name),
                git(&repo, &["show", &format!("HEAD:{name}")]),
            )
            .unwrap();
        }
        run_lfs(
            &repo,
            gitcomet_core::large_files::LargeFileCommand::LfsPull {
                paths: vec![selected.into()],
            },
        );
        assert_eq!(
            fs::read_to_string(repo.join(selected)).unwrap(),
            format!("{selected} content\n")
        );
        assert!(
            fs::read_to_string(repo.join(other))
                .unwrap()
                .starts_with("version https://git-lfs"),
            "only the named file may be checked out"
        );
    }
}

/// Where `git annex contentlocation` puts `ANNEX_KEY` in a non-bare repo
/// (`hashdirmixed`, two levels) and in bare or crippled ones (`hashdirlower`).
const ANNEX_KEY_MIXED_DIR: &str = "V7/pK";
const ANNEX_KEY_LOWER_DIR: &str = "c79/e4f";
const OTHER_ANNEX_KEY: &str =
    "SHA256E-s3--ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.bin";

/// Unlocked annex content lives at a path git-annex derives from the key, so
/// presence is read from the store: status never runs git-annex for it, and
/// the answer is the same where git-annex is not installed.
#[test]
fn unlocked_annex_presence_is_read_from_the_object_store() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let object = |hash_dir: &str, key: &str| {
        let dir = repo.join(format!(".git/annex/objects/{hash_dir}/{key}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(key), b"hello").unwrap();
    };
    object(ANNEX_KEY_MIXED_DIR, ANNEX_KEY);
    let pointer = |key: &str| format!("/annex/objects/{key}\n");
    fs::write(repo.join("present.bin"), pointer(ANNEX_KEY)).unwrap();
    fs::write(repo.join("absent.bin"), pointer(OTHER_ANNEX_KEY)).unwrap();
    git(&repo, &["add", "."]);

    let (status, opened) = status(&repo);
    let files = opened
        .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    let presence = |name: &str| files.staged[Path::new(name)].in_local_store;
    assert_eq!(presence("present.bin"), Some(true));
    assert_eq!(presence("absent.bin"), Some(false));

    // Bare and crippled-filesystem repos use the lower-case layout.
    fs::remove_dir_all(repo.join(".git/annex/objects")).unwrap();
    object(ANNEX_KEY_LOWER_DIR, ANNEX_KEY);
    let files = opened
        .uncommitted_large_files_for_status_cancellable(&status, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        files.staged[Path::new("present.bin")].in_local_store,
        Some(true)
    );
}

/// `git reset --soft` (or a merge) leaves an index pointer HEAD does not
/// have. Downloading content for a staged diff must fetch that side too.
#[test]
fn lfs_diff_download_fetches_the_index_side_of_a_staged_diff() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, _) = clone_lfs_history(dir.path());
    git(&repo, &["config", "lfs.fetchexclude", "*.bin"]);
    git(&repo, &["reset", "-q", "--soft", "HEAD~1"]);
    // A staged diff must not try to fetch an unrelated worktree object that
    // the remote does not have.
    let worktree = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
        "0".repeat(64)
    );
    fs::write(repo.join("a.bin"), &worktree).unwrap();
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("a.bin"),
        area: DiffArea::Staged,
    };
    let opened = GixBackend.open(&repo).unwrap();
    let diff = opened.diff_file_text(&target).unwrap().unwrap();
    assert_eq!(
        diff.new_large.unwrap().content,
        LargeFileContent::MissingLocally
    );
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsFetchForDiff {
            target: target.clone(),
        },
    );
    let diff = opened.diff_file_text(&target).unwrap().unwrap();
    assert_eq!(
        source_text(diff.old_source.as_ref()).as_deref(),
        Some("middle version\n"),
        "HEAD side"
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("current version\n"),
        "index side"
    );
    assert_eq!(fs::read_to_string(repo.join("a.bin")).unwrap(), worktree);
}

#[test]
fn lfs_diff_download_fetches_a_worktree_pointer_absent_from_head_and_index() {
    use gitcomet_core::domain::CommitId;
    use gitcomet_core::large_files::LargeFileCommand;
    if !git_lfs_available() {
        return;
    }
    for (path, range) in [
        ("a.bin", false),
        ("nested/a [1].bin", false),
        ("clip ", false),
        ("clip\t", false),
        #[cfg(unix)]
        ("clip\\", false),
        ("a.bin", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (repo, middle) = clone_lfs_history(dir.path());
        let pointer = git(&repo, &["show", "HEAD~2:a.bin"]);
        git(&repo, &["config", "lfs.fetchexclude", "*.bin"]);
        git(&repo, &["reset", "-q", "--soft", "HEAD~1"]);
        if path != "a.bin" {
            fs::create_dir(repo.join("nested")).unwrap();
            git(&repo, &["mv", "a.bin", path]);
        }
        fs::write(repo.join(path), &pointer).unwrap();
        let index_before = git(&repo, &["ls-files", "--stage"]);
        let target = if range {
            DiffTarget::CommitRange {
                from_commit_id: CommitId(middle.into()),
                to_commit_id: None,
                path: Some(path.into()),
            }
        } else {
            DiffTarget::WorkingTree {
                path: if path == "a.bin" {
                    path.into()
                } else {
                    repo.join(path)
                },
                area: DiffArea::Unstaged,
            }
        };
        let opened = GixBackend.open(&repo).unwrap();
        let diff = opened.diff_file_text(&target).unwrap().unwrap();
        assert_eq!(
            diff.old_large.unwrap().content,
            LargeFileContent::MissingLocally
        );
        assert_eq!(
            diff.new_large.unwrap().content,
            LargeFileContent::MissingLocally
        );

        opened
            .run_large_file_command(&LargeFileCommand::LfsFetchForDiff {
                target: target.clone(),
            })
            .unwrap();

        let diff = opened.diff_file_text(&target).unwrap().unwrap();
        assert_eq!(diff.old_large.unwrap().content, LargeFileContent::Available);
        assert_eq!(diff.new_large.unwrap().content, LargeFileContent::Available);
        assert_eq!(
            source_text(diff.old_source.as_ref()).as_deref(),
            Some(if range {
                "middle version\n"
            } else {
                "current version\n"
            })
        );
        assert_eq!(
            source_text(diff.new_source.as_ref()).as_deref(),
            Some("large file contents\n")
        );
        assert_eq!(git(&repo, &["ls-files", "--stage"]), index_before);
        assert_eq!(fs::read_to_string(repo.join(path)).unwrap(), pointer);
    }
}

#[cfg(unix)]
#[test]
fn lfs_diff_download_does_not_follow_worktree_symlinks() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, _) = clone_lfs_history(dir.path());
    // The diff displays the link text. Its target's missing LFS object is
    // unrelated and cannot be fetched from the remote.
    fs::write(
        repo.join("other.txt"),
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
            "0".repeat(64)
        ),
    )
    .unwrap();
    fs::remove_file(repo.join("a.bin")).unwrap();
    std::os::unix::fs::symlink("other.txt", repo.join("a.bin")).unwrap();
    let target = unstaged("a.bin");
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsFetchForDiff {
            target: target.clone(),
        },
    );
    let diff = GixBackend
        .open(&repo)
        .unwrap()
        .diff_file_text(&target)
        .unwrap()
        .unwrap();
    assert_eq!(
        source_text(diff.old_source.as_ref()).as_deref(),
        Some("current version\n")
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("other.txt")
    );
    assert!(diff.new_large.is_none());
    assert_eq!(
        fs::read_link(repo.join("a.bin")).unwrap(),
        Path::new("other.txt")
    );
}

/// The worktree side of an unstaged image diff can be a locked annex file: a
/// symlink whose target is the content. It resolves like the index side does
/// instead of reading as a deleted file.
#[cfg(unix)]
#[test]
fn annex_locked_worktree_image_resolves_through_its_symlink() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let before = b"\x89PNG\r\n\x1a\nbefore".to_vec();
    let after = b"\x89PNG\r\n\x1a\nafter!".to_vec();
    let key = |bytes: &[u8], tag: char| {
        format!(
            "SHA256E-s{}--{}.png",
            bytes.len(),
            tag.to_string().repeat(64)
        )
    };
    let (old_key, new_key) = (key(&before, 'a'), key(&after, 'b'));
    let link = |key: &str, bytes: &[u8]| {
        let relative =
            Path::new(".git/annex/objects").join(&gitcomet_core::annex::object_paths(key, 2)[0]);
        let object = repo.join(&relative);
        fs::create_dir_all(object.parent().unwrap()).unwrap();
        fs::write(object, bytes).unwrap();
        let path = repo.join("pic.png");
        let _ = fs::remove_file(&path);
        symlink(relative, &path).unwrap();
    };
    link(&old_key, &before);
    git(&repo, &["add", "pic.png"]);
    git(&repo, &["commit", "-qm", "image"]);
    link(&new_key, &after);

    let opened = GixBackend.open(&repo).unwrap();
    let image = opened
        .diff_file_image(&unstaged("pic.png"))
        .unwrap()
        .unwrap();
    assert_eq!(image.old.as_deref(), Some(before.as_slice()), "index side");
    assert_eq!(
        image.new.as_deref(),
        Some(after.as_slice()),
        "worktree side"
    );
    assert!(image.new_large.is_some());
}

/// The lock list loads on its own when a repository has lockable patterns.
/// Like other background probes it must not take the credentials staged for
/// the command the user is retrying.
#[test]
fn lfs_lock_listing_does_not_consume_credentials_staged_for_a_command() {
    use gitcomet_core::auth::{
        GitAuthKind, ScopedStagedGitAuth, StagedGitAuth, take_staged_git_auth,
    };
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    let remote = dir.path().join("remote.git");
    git(
        dir.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git(&repo, &["remote", "add", "origin", &file_url(&remote)]);
    let opened = GixBackend.open(&repo).unwrap();
    let _auth = ScopedStagedGitAuth::stage(StagedGitAuth {
        kind: GitAuthKind::UsernamePassword,
        username: Some("alice".into()),
        secret: "test-token".into(),
    });
    let _ = opened.lfs_locks_cancellable(&CancellationToken::new());
    let pending = take_staged_git_auth()
        .expect("the lock listing must leave the retried command's credentials staged");
    assert_eq!(pending.username.as_deref(), Some("alice"));
}

/// Commit rows read every small blob to test for a pointer, which was most
/// of the cost of opening a large commit. Repositories that use neither tool
/// skip that; one that tracks LFS files in `.gitattributes` does not, even
/// without git-lfs installed or anything downloaded.
#[test]
fn commit_rows_look_for_pointers_only_where_lfs_or_annex_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let pointer = "version https://git-lfs.github.com/spec/v1\n\
                   oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                   size 12345\n";
    fs::write(repo.join("a.bin"), pointer).unwrap();
    git(&repo, &["add", "a.bin"]);
    git(&repo, &["commit", "-qm", "pointer-shaped text"]);
    let row = |repo: &Path| {
        let head = git(repo, &["rev-parse", "HEAD"]).trim().to_string();
        let details = GixBackend
            .open(repo)
            .unwrap()
            .commit_details(&gitcomet_core::domain::CommitId(head.into()))
            .unwrap();
        details.files[0].large_file.clone()
    };
    assert!(row(&repo).is_none(), "plain repository: no blob is read");

    fs::write(
        repo.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    assert!(
        row(&repo).is_some_and(|state| state.pointer.is_lfs()),
        "tracked in .gitattributes"
    );
}

/// Timing probe: commit details for one large commit in a plain repository,
/// where no row can be an LFS or annex pointer. Run with
/// `cargo test --test large_files_integration -- --ignored --nocapture timing_`.
#[test]
#[ignore]
fn timing_commit_details_of_a_large_plain_commit() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let files = 20_000;
    for i in 0..files {
        let sub = repo.join(format!("src/m{}", i % 100));
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join(format!("f{i}.rs")),
            format!("// module {i}\npub fn f{i}() -> usize {{ {i} }}\n").repeat(1 + i % 40),
        )
        .unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "large"]);
    git(&repo, &["gc", "-q"]);
    let head = gitcomet_core::domain::CommitId(git(&repo, &["rev-parse", "HEAD"]).trim().into());
    let opened = GixBackend.open(&repo).unwrap();
    let mut samples = Vec::new();
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let details = opened.commit_details(&head).unwrap();
        samples.push(started.elapsed());
        assert_eq!(details.files.len(), files);
    }
    samples.sort();
    println!(
        "timing commit_details files={files} min={:?} median={:?}",
        samples[0], samples[2]
    );
}

#[test]
fn support_refresh_reads_external_config_and_attribute_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_repo(repo);
    let opened = GixBackend.open(repo).unwrap();
    assert!(
        !opened
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap()
            .lfs
            .in_use()
    );
    git(repo, &["config", "filter.lfs.clean", "git-lfs clean -- %f"]);
    git(repo, &["config", "lfs.storage", "custom-lfs"]);
    fs::write(repo.join(".gitattributes"), "*.bin filter=lfs -text\n").unwrap();
    let support = opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(support.lfs.filter_configured);
    assert!(support.lfs.storage_dir.ends_with("custom-lfs"));
    assert_eq!(support.lfs.tracked_patterns.len(), 1);
    assert_eq!(support.lfs.tracked_patterns[0].pattern, "*.bin");
    fs::write(
        repo.join(".gitattributes"),
        "*.psd filter=lfs -text lockable\n",
    )
    .unwrap();
    let support = opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert_eq!(support.lfs.tracked_patterns[0].pattern, "*.psd");
    assert!(support.lfs.has_lockable_patterns);
}

#[test]
fn support_refresh_updates_worktree_filter_configuration() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_repo(repo);
    // Override any global LFS installation with a pass-through filter until
    // the repository is open, so user configuration cannot mask stale reads.
    for (key, value) in [
        ("filter.lfs.process", ""),
        ("filter.lfs.clean", "cat"),
        ("filter.lfs.smudge", "cat"),
        ("filter.lfs.required", "false"),
    ] {
        git(repo, &["config", key, value]);
    }
    fs::write(repo.join(".gitattributes"), "*.bin filter=lfs\n").unwrap();
    fs::write(
        repo.join("a.bin"),
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12\n",
            "1".repeat(64)
        ),
    )
    .unwrap();
    git(repo, &["add", "."]);
    git(
        repo,
        &["commit", "-qm", "pointer before enabling LFS filters"],
    );
    let opened = GixBackend.open(repo).unwrap();
    opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    configure_lfs_filters(repo);
    fs::write(repo.join("a.bin"), "new content\n").unwrap();
    opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    let diff = opened.diff_file_text(&unstaged("a.bin")).unwrap().unwrap();
    assert!(
        diff.new_large.is_some(),
        "the new filter config must clean the worktree content to its LFS pointer"
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("new content\n")
    );

    git(repo, &["add", "a.bin"]);
    git(repo, &["commit", "-qm", "content tracked with LFS"]);
    // Invalidate the cached stat without changing the content, forcing status
    // to compare through the newly installed clean filter.
    fs::write(repo.join("a.bin"), "new content\n").unwrap();
    opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert!(git(repo, &["--no-optional-locks", "status", "--porcelain"]).is_empty());
    let fresh_status = GixBackend.open(repo).unwrap().status().unwrap();
    assert!(fresh_status.staged.is_empty() && fresh_status.unstaged.is_empty());
    assert_eq!(
        opened.status().unwrap(),
        fresh_status,
        "status on the existing handle must also use the refreshed LFS filters"
    );
}

#[test]
fn annex_remote_tracking_detection_handles_loose_and_packed_refs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_repo(repo);
    git(repo, &["commit", "--allow-empty", "-qm", "initial"]);
    let opened = GixBackend.open(repo).unwrap();
    assert!(
        !opened
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap()
            .annex
            .in_use()
    );
    git(
        repo,
        &["update-ref", "refs/remotes/origin/git-annex", "HEAD"],
    );
    for packed in [false, true] {
        if packed {
            git(repo, &["pack-refs", "--all", "--prune"]);
        }
        let support = opened
            .large_file_support_cancellable(&CancellationToken::new())
            .unwrap();
        assert!(support.annex.has_annex_branch, "packed={packed}");
        assert!(support.annex.in_use());
        assert!(!support.annex.initialized());
    }
}

#[test]
fn annex_remote_tracking_detection_matches_the_complete_branch_name() {
    for (remotes, reference, annex) in [
        (
            vec!["origin"],
            "refs/remotes/origin/feature/git-annex",
            false,
        ),
        (
            vec!["team", "team/alice"],
            "refs/remotes/team/alice/git-annex",
            true,
        ),
        (
            vec!["team", "team/alice"],
            "refs/remotes/team/alice/feature/git-annex",
            false,
        ),
        (vec!["team"], "refs/remotes/team/alice/git-annex", false),
        (vec![], "refs/remotes/origin/feature/git-annex", false),
        (vec![], "refs/heads/feature/git-annex", false),
        (vec![], "refs/heads/git-annex", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        git(repo, &["commit", "--allow-empty", "-qm", "initial"]);
        for remote in remotes {
            // Configure overlapping namespaces directly: `remote add` rejects
            // them even though existing config can contain both names.
            git(
                repo,
                &[
                    "config",
                    &format!("remote.{remote}.url"),
                    "https://example.invalid/repo.git",
                ],
            );
            git(
                repo,
                &[
                    "config",
                    &format!("remote.{remote}.fetch"),
                    &format!("+refs/heads/*:refs/remotes/{remote}/*"),
                ],
            );
        }
        let opened = GixBackend.open(repo).unwrap();
        git(repo, &["update-ref", reference, "HEAD"]);
        for packed in [false, true] {
            if packed {
                git(repo, &["pack-refs", "--all", "--prune"]);
            }
            let support = opened
                .large_file_support_cancellable(&CancellationToken::new())
                .unwrap();
            assert_eq!(
                support.annex.has_annex_branch, annex,
                "{reference}, packed={packed}"
            );
            assert_eq!(support.is_active(), annex, "{reference}, packed={packed}");
        }
    }
}

#[test]
fn lfs_status_download_fetches_staged_content_and_preserves_staged_blobs() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, _) = clone_lfs_history(dir.path());
    git(&repo, &["config", "lfs.fetchexclude", "*.bin"]);
    git(&repo, &["reset", "-q", "--soft", "HEAD~1"]);
    let index_before = git(&repo, &["ls-files", "--stage"]);
    let pointer = fs::read(repo.join("a.bin")).unwrap();
    assert!(gitcomet_core::lfs::parse_pointer(&pointer).is_some());
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsPull {
            paths: vec![repo.join("a.bin")],
        },
    );
    assert_eq!(
        fs::read_to_string(repo.join("a.bin")).unwrap(),
        "current version\n"
    );
    assert_eq!(git(&repo, &["ls-files", "--stage"]), index_before);
}

#[test]
fn lfs_worktree_range_download_fetches_the_displayed_index_pointer() {
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (repo, middle) = clone_lfs_history(dir.path());
    git(&repo, &["reset", "-q", "--soft", "HEAD~1"]);
    let before = git(&repo, &["ls-files", "--stage"]);
    let pointer = fs::read(repo.join("a.bin")).unwrap();
    let target = DiffTarget::CommitRange {
        from_commit_id: gitcomet_core::domain::CommitId(middle.into()),
        to_commit_id: None,
        path: Some("a.bin".into()),
    };
    let opened = GixBackend.open(&repo).unwrap();
    assert_eq!(
        opened
            .diff_file_text(&target)
            .unwrap()
            .unwrap()
            .new_large
            .unwrap()
            .content,
        LargeFileContent::MissingLocally
    );
    opened
        .run_large_file_command(
            &gitcomet_core::large_files::LargeFileCommand::LfsFetchForDiff {
                target: target.clone(),
            },
        )
        .unwrap();
    let diff = opened.diff_file_text(&target).unwrap().unwrap();
    assert_eq!(
        source_text(diff.old_source.as_ref()).as_deref(),
        Some("middle version\n")
    );
    assert_eq!(
        source_text(diff.new_source.as_ref()).as_deref(),
        Some("current version\n")
    );
    assert_eq!(git(&repo, &["ls-files", "--stage"]), before);
    assert_eq!(fs::read(repo.join("a.bin")).unwrap(), pointer);
}

#[test]
fn storage_changes_refresh_rows_and_all_diff_resolvers_on_the_same_handle() {
    use gitcomet_core::domain::{CommitId, DiffPreviewTextSide};
    if !git_lfs_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    fs::write(
        repo.join(".gitattributes"),
        "*.bin filter=lfs\n*.png filter=lfs\n",
    )
    .unwrap();
    let image_bytes = b"\x89PNG\r\n\x1a\nimage";
    fs::write(repo.join("pic.png"), image_bytes).unwrap();
    fs::write(repo.join("a.bin"), "new content\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "new content and image"]);
    let head = CommitId(git(&repo, &["rev-parse", "HEAD"]).trim().into());
    let parent = CommitId(git(&repo, &["rev-parse", "HEAD^"]).trim().into());
    let opened = GixBackend.open(&repo).unwrap();
    let text_target = DiffTarget::Commit {
        commit_id: head.clone(),
        path: Some("a.bin".into()),
    };
    let image_target = DiffTarget::Commit {
        commit_id: head.clone(),
        path: Some("pic.png".into()),
    };
    let status = RepoStatus {
        staged: vec![row("a.bin", FileStatusKind::Modified)].into(),
        unstaged: Default::default(),
    };
    let cancellation = CancellationToken::new();
    let mut previous = repo.join(".git/lfs");
    for configured in [PathBuf::from("moved-lfs"), dir.path().join("external-lfs")] {
        git(
            &repo,
            &["config", "lfs.storage", configured.to_str().unwrap()],
        );
        let storage = if configured.is_absolute() {
            configured
        } else {
            repo.join(".git").join(configured)
        };
        let support = opened
            .large_file_support_cancellable(&cancellation)
            .unwrap();
        assert_eq!(support.lfs.storage_dir, storage);
        assert!(!support.lfs.has_local_store);
        let missing = opened
            .uncommitted_large_files_for_status_cancellable(&status, &cancellation)
            .unwrap();
        assert_eq!(
            missing.staged[Path::new("a.bin")].in_local_store,
            Some(false)
        );
        assert_eq!(
            opened
                .diff_file_text(&text_target)
                .unwrap()
                .unwrap()
                .new_large
                .unwrap()
                .content,
            LargeFileContent::MissingLocally
        );
        fs::rename(&previous, &storage).unwrap();
        let present = opened
            .uncommitted_large_files_for_status_cancellable(&status, &cancellation)
            .unwrap();
        assert_eq!(
            present.staged[Path::new("a.bin")].in_local_store,
            Some(true)
        );
        let diff = opened.diff_file_text(&text_target).unwrap().unwrap();
        assert_eq!(
            source_text(diff.new_source.as_ref()).as_deref(),
            Some("new content\n")
        );
        let preview = opened
            .diff_preview_text_file(&text_target, DiffPreviewTextSide::New)
            .unwrap()
            .unwrap();
        assert_eq!(fs::read_to_string(preview.path).unwrap(), "new content\n");
        let image = opened.diff_file_image(&image_target).unwrap().unwrap();
        assert_eq!(image.new.as_deref(), Some(image_bytes.as_slice()));
        for rows in [
            opened.commit_details(&head).unwrap().files,
            opened.diff_range_files(&parent, Some(&head)).unwrap(),
        ] {
            let state = rows
                .iter()
                .find(|row| row.path == Path::new("a.bin"))
                .unwrap()
                .large_file
                .as_ref()
                .unwrap();
            assert_eq!(state.in_local_store, Some(true));
        }
        previous = storage;
    }
}

#[test]
fn committed_pointer_scans_detect_nested_attributes_without_a_local_store() {
    use gitcomet_core::domain::CommitId;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    init_repo(repo);
    git(repo, &["commit", "--allow-empty", "-qm", "empty"]);
    let parent = CommitId(git(repo, &["rev-parse", "HEAD"]).trim().into());
    fs::create_dir(repo.join("nested")).unwrap();
    fs::write(repo.join("nested/.gitattributes"), "*.bin filter=lfs\n").unwrap();
    fs::write(repo.join("nested/a.bin"), "version https://git-lfs.github.com/spec/v1\noid sha256:1111111111111111111111111111111111111111111111111111111111111111\nsize 12\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "nested LFS pointer"]);
    let head = CommitId(git(repo, &["rev-parse", "HEAD"]).trim().into());
    let opened = GixBackend.open(repo).unwrap();
    assert!(!repo.join(".git/lfs").exists());
    for rows in [
        opened.commit_details(&head).unwrap().files,
        opened.diff_range_files(&parent, Some(&head)).unwrap(),
    ] {
        let state = rows
            .iter()
            .find(|row| row.path == Path::new("nested/a.bin"))
            .unwrap()
            .large_file
            .as_ref()
            .expect("nested LFS pattern enables classification");
        assert!(state.pointer.is_lfs());
        assert_eq!(state.in_local_store, Some(false));
    }
}

#[cfg(unix)]
fn with_lfs_shim(name: &str, script: &str, test: impl FnOnce(&Path)) {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "GITCOMET_LFS_TEST_ROOT";
    let Some(root) = std::env::var_os(CHILD) else {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let shim = bin.join("git-lfs");
        fs::write(&shim, script).unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let mut child = Command::new(std::env::current_exe().unwrap());
        test_git_env::apply(&mut child);
        let result = child
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, root.path())
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("GITCOMET_GIT_COMMAND_TIMEOUT_SECS", "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    };
    let repo = PathBuf::from(root).join("repo");
    init_repo(&repo);
    test(&repo);
}

#[cfg(unix)]
fn silent_lfs_commands() -> [gitcomet_core::large_files::LargeFileCommand; 3] {
    use gitcomet_core::large_files::LargeFileCommand;
    [
        LargeFileCommand::LfsFsck,
        LargeFileCommand::LfsPrune,
        LargeFileCommand::LfsTrack {
            patterns: vec!["*.slow".into()],
            filename: false,
            lockable: false,
            renormalize: vec!["file.slow".into()],
        },
    ]
}

#[cfg(unix)]
fn prepare_slow_clean_filter(repo: &Path, clean: &str) {
    fs::write(repo.join("file.slow"), "content\n").unwrap();
    git(repo, &["add", "file.slow"]);
    git(repo, &["config", "filter.slow.clean", clean]);
    fs::write(repo.join(".gitattributes"), "*.slow filter=slow\n").unwrap();
}

#[cfg(unix)]
#[test]
fn lfs_silent_local_commands_outlive_the_silence_deadline() {
    with_lfs_shim(
        "lfs_silent_local_commands_outlive_the_silence_deadline",
        "#!/bin/sh\ncase \"$1\" in fsck|prune) sleep 3 ;; track) ;; *) exit 91 ;; esac\n",
        |repo| {
            prepare_slow_clean_filter(repo, "sleep 3; cat");
            let opened = GixBackend.open(repo).unwrap();
            for command in silent_lfs_commands() {
                opened.run_large_file_command(&command).unwrap();
            }
        },
    );
}

#[cfg(unix)]
#[test]
fn lfs_silent_commands_announce_activity_and_can_be_cancelled() {
    use gitcomet_core::git_operation::{self, GitOperationContext, GitOperationEvent};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    with_lfs_shim(
        "lfs_silent_commands_announce_activity_and_can_be_cancelled",
        "#!/bin/sh\ncase \"$1\" in fsck|prune) printf started > started; exec sleep 30 ;; track) ;; *) exit 91 ;; esac\n",
        |repo| {
            prepare_slow_clean_filter(repo, "printf started > started; exec sleep 30");
            let opened = GixBackend.open(repo).unwrap();
            for command in silent_lfs_commands() {
                let (tx, rx) = mpsc::channel();
                let context = GitOperationContext::new("LFS test", move |_, event| {
                    let _ = tx.send(event);
                });
                let operation = context.clone();
                let opened = opened.clone();
                let worker = std::thread::spawn(move || {
                    let _scope = git_operation::attach(&operation);
                    opened.run_large_file_command(&command)
                });
                let first = rx.recv_timeout(Duration::from_secs(5));
                // Cancel even if the assertion will fail, so a regression
                // cannot leave a silent worker blocking the test suite.
                let deadline = Instant::now() + Duration::from_secs(5);
                while !repo.join("started").exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(git_operation::cancel(context.id()));
                let cancelled_at = Instant::now();
                let result = worker.join().unwrap();
                assert_eq!(first, Ok(GitOperationEvent::CommandStarted));
                assert!(matches!(
                    result.unwrap_err().kind(),
                    gitcomet_core::error::ErrorKind::Cancelled
                ));
                assert!(cancelled_at.elapsed() < Duration::from_secs(5));
                assert!(
                    repo.join("started").exists(),
                    "must cancel a running subprocess"
                );
                assert!(
                    !rx.try_iter()
                        .any(|event| event == GitOperationEvent::CommandStarted),
                    "announce the operation only once"
                );
                fs::remove_file(repo.join("started")).unwrap();
            }
        },
    );
}
