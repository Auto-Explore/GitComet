use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::process::{GitVersion, refresh_git_runtime};
use gitcomet_core::services::{GitBackend, MAINTENANCE_CHECK_COMMAND};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    // Fixtures must not run git's own detached maintenance: it would repack
    // behind the test's back.
    let output = cmd
        .args(["-c", "maintenance.auto=false", "-C"])
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `git maintenance is-needed` arrived in 2.53; older git skips these tests.
/// Probed once: a refresh racing another test's probe returns the unprobed
/// runtime, which would skip the test.
fn supports_is_needed() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    let supported = *SUPPORTED.get_or_init(|| {
        refresh_git_runtime()
            .version()
            .is_some_and(|version| version >= GitVersion::MAINTENANCE_IS_NEEDED)
    });
    if !supported {
        eprintln!(
            "skipping: git older than {}",
            GitVersion::MAINTENANCE_IS_NEEDED
        );
    }
    supported
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo dir");
    run_git(repo, &["init", "--quiet"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        run_git(repo, &["config", key, value]);
    }
}

/// A repository with four packs where git's limit is two.
fn repo_with_too_many_packs(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    init_repo(&repo);
    run_git(&repo, &["config", "gc.autoPackLimit", "2"]);
    for index in 0..4 {
        fs::write(repo.join(format!("file-{index}.txt")), format!("{index}\n"))
            .expect("write file");
        run_git(&repo, &["add", "."]);
        run_git(&repo, &["commit", "--quiet", "-m", &format!("c{index}")]);
        run_git(&repo, &["repack", "-q"]);
    }
    repo
}

fn pack_count(repo: &Path) -> usize {
    fs::read_dir(repo.join(".git/objects/pack"))
        .expect("read pack dir")
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
        })
        .count()
}

#[test]
fn maintenance_is_recommended_until_it_runs() {
    if !supports_is_needed() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = repo_with_too_many_packs(dir.path());
    let opened = GixBackend.open(&repo).expect("open repo");

    assert!(opened.maintenance_needed().expect("check"));
    let output = opened.run_maintenance_with_output().expect("maintenance");

    assert_eq!(output.exit_code, Some(0));
    assert!(
        pack_count(&repo) <= 2,
        "packs consolidated: {}",
        pack_count(&repo)
    );
    assert!(!opened.maintenance_needed().expect("check after run"));
}

#[test]
fn scheduled_maintenance_is_never_recommended() {
    if !supports_is_needed() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = repo_with_too_many_packs(dir.path());
    // What `git maintenance register` writes: a scheduler runs it instead.
    run_git(&repo, &["config", "maintenance.auto", "false"]);

    let opened = GixBackend.open(&repo).expect("open repo");
    assert!(!opened.maintenance_needed().expect("check"));
}

/// A committed repository using git's geometric strategy, the default for
/// manual maintenance since git 2.54.
fn geometric_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    init_repo(&repo);
    run_git(&repo, &["config", "maintenance.strategy", "geometric"]);
    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed");
    run_git(&repo, &["add", "seed.txt"]);
    run_git(&repo, &["commit", "--quiet", "-m", "seed"]);
    repo
}

#[test]
fn trivial_upkeep_is_not_recommended() {
    if !supports_is_needed() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = geometric_repo(dir.path());
    // A deleted worktree is enough for git's own auto check to ask for
    // maintenance, but pruning it is not worth asking the user about.
    run_git(&repo, &["config", "gc.worktreePruneExpire", "now"]);
    let linked = dir.path().join("linked");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    fs::remove_dir_all(&linked).expect("delete worktree");

    let opened = GixBackend.open(&repo).expect("open repo");
    assert!(!opened.maintenance_needed().expect("check"));
}

#[test]
fn run_without_pending_work_reports_the_check() {
    if !supports_is_needed() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = geometric_repo(dir.path());
    let opened = GixBackend.open(&repo).expect("open repo");

    let output = opened.run_maintenance_with_output().expect("maintenance");

    assert_eq!(output.command, MAINTENANCE_CHECK_COMMAND);
    assert_eq!(pack_count(&repo), 0, "nothing was repacked");
}

#[test]
fn held_maintenance_lock_is_reported() {
    if !supports_is_needed() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = repo_with_too_many_packs(dir.path());
    let packs = pack_count(&repo);
    let lock = repo.join(".git/objects/maintenance.lock");
    fs::write(&lock, b"").expect("lock");
    let opened = GixBackend.open(&repo).expect("open repo");

    let error = opened
        .run_maintenance_with_output()
        .expect_err("a held lock makes git skip maintenance silently");

    assert!(
        error.to_string().contains("maintenance.lock"),
        "the error names the lock: {error}"
    );
    assert_eq!(pack_count(&repo), packs, "nothing was repacked");
    assert!(lock.exists(), "someone else's lock is left alone");
}

#[test]
fn linked_worktree_shares_the_main_repository_common_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed");
    run_git(&repo, &["add", "seed.txt"]);
    run_git(&repo, &["commit", "--quiet", "-m", "seed"]);
    let linked = dir.path().join("linked");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );

    let main = GixBackend.open(&repo).expect("open main");
    let worktree = GixBackend.open(&linked).expect("open linked worktree");
    let expected = canonicalize_or_original(repo.join(".git"));
    assert_eq!(main.common_dir(), Some(expected.clone()));
    assert_eq!(worktree.common_dir(), Some(expected));
}
