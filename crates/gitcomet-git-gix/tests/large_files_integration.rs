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
    for (key, value) in [
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.required", "true"),
    ] {
        git(repo, &["config", key, value]);
    }
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
        .map(|p| (p.pattern.as_str(), p.lockable))
        .collect::<Vec<_>>();
    assert_eq!(patterns, [("*.bin", false), ("*.psd", true)]);
    assert!(support.is_active() && !support.annex.in_use());
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
    let object_dir = repo.join(format!(".git/annex/objects/Xk/Wq/{ANNEX_KEY}"));
    fs::create_dir_all(&object_dir).unwrap();
    fs::write(object_dir.join(ANNEX_KEY), b"hello").unwrap();
    let link_target = format!(".git/annex/objects/Xk/Wq/{ANNEX_KEY}/{ANNEX_KEY}");
    symlink(&link_target, repo.join("present.bin")).unwrap();
    let missing_target = link_target.replace("Xk/Wq", "Zz/Zz");
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
        None,
        "needs git-annex to know"
    );
}

#[test]
fn adjusted_branch_head_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["checkout", "-qb", "adjusted/main(unlocked)"]);
    let opened = GixBackend.open(&repo).unwrap();
    let support = opened
        .large_file_support_cancellable(&CancellationToken::new())
        .unwrap();
    assert_eq!(
        support.annex.adjusted,
        Some(("main".to_string(), "unlocked".to_string()))
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
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_lfs_repo(&repo);
    for name in ["a[1].bin", "a1.bin"] {
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
    for name in ["a[1].bin", "a1.bin"] {
        fs::write(
            repo.join(name),
            git(&repo, &["show", &format!("HEAD:{name}")]),
        )
        .unwrap();
    }
    run_lfs(
        &repo,
        gitcomet_core::large_files::LargeFileCommand::LfsPull {
            paths: vec!["a[1].bin".into()],
        },
    );
    assert_eq!(
        fs::read_to_string(repo.join("a[1].bin")).unwrap(),
        "a[1].bin content\n"
    );
    assert!(
        fs::read_to_string(repo.join("a1.bin"))
            .unwrap()
            .starts_with("version https://git-lfs"),
        "only the named file may be checked out"
    );
}
