#[cfg(unix)]
use gitcomet_core::process::{
    GitExecutablePreference, current_git_executable_preference, install_git_executable_path,
    install_git_executable_preference,
};
use gitcomet_core::remote_url::{RemoteProtocol, RemoteUrlPolicy};
use gitcomet_core::services::{GitBackend, PullMode, RemoteUrlKind};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // Keep tests deterministic by isolating from host git config.
    test_git_env::apply(&mut cmd);
    // Local bare remotes require file protocol to be permitted.
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn run_git_capture(repo: &Path, args: &[&str]) -> String {
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn run_git_status(repo: &Path, args: &[&str]) -> std::process::ExitStatus {
    git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run")
}

fn git_remote_url(path: &Path) -> String {
    if cfg!(windows) {
        // Ensure Windows drive-letter paths are never treated as scp-style host:path.
        let normalized = path.to_string_lossy().replace('\\', "/");
        format!("file:///{normalized}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn remote_management_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(unix)]
struct GitExecutablePreferenceGuard {
    original: GitExecutablePreference,
}

#[cfg(unix)]
impl GitExecutablePreferenceGuard {
    fn install(path: &Path) -> Self {
        let original = current_git_executable_preference();
        let runtime = install_git_executable_path(Some(path.to_path_buf()));
        assert!(
            runtime.is_available(),
            "localized Git test wrapper must be executable: {:?}",
            runtime.availability
        );
        Self { original }
    }
}

#[cfg(unix)]
impl Drop for GitExecutablePreferenceGuard {
    fn drop(&mut self) {
        let _ = install_git_executable_preference(self.original.clone());
    }
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_local_push_available_for_remote_management_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(_) => return true,
        };
        let remote_repo = dir.path().join("probe-remote.git");
        let work_repo = dir.path().join("probe-work");
        if fs::create_dir_all(&remote_repo).is_err() || fs::create_dir_all(&work_repo).is_err() {
            return true;
        }

        let init_remote = match git_command()
            .arg("-C")
            .arg(&remote_repo)
            .args(["init", "--bare"])
            .status()
        {
            Ok(status) => status.success(),
            Err(_) => true,
        };
        if !init_remote {
            return true;
        }

        let init_work = match git_command()
            .arg("-C")
            .arg(&work_repo)
            .args(["init"])
            .status()
        {
            Ok(status) => status.success(),
            Err(_) => true,
        };
        if !init_work {
            return true;
        }

        for args in [
            ["config", "user.email", "you@example.com"].as_slice(),
            ["config", "user.name", "You"].as_slice(),
            ["config", "commit.gpgsign", "false"].as_slice(),
            ["config", "core.autocrlf", "false"].as_slice(),
            ["config", "core.eol", "lf"].as_slice(),
        ] {
            let status = match git_command().arg("-C").arg(&work_repo).args(args).status() {
                Ok(status) => status,
                Err(_) => return true,
            };
            if !status.success() {
                return true;
            }
        }

        if fs::write(work_repo.join("probe.txt"), "probe\n").is_err() {
            return true;
        }

        for args in [
            ["add", "probe.txt"].as_slice(),
            ["-c", "commit.gpgsign=false", "commit", "-m", "probe"].as_slice(),
        ] {
            let status = match git_command().arg("-C").arg(&work_repo).args(args).status() {
                Ok(status) => status,
                Err(_) => return true,
            };
            if !status.success() {
                return true;
            }
        }

        let remote_url = git_remote_url(&remote_repo);
        let add_remote = match git_command()
            .arg("-C")
            .arg(&work_repo)
            .args(["remote", "add", "origin", remote_url.as_str()])
            .status()
        {
            Ok(status) => status.success(),
            Err(_) => true,
        };
        if !add_remote {
            return true;
        }

        let push_output = match git_command()
            .arg("-C")
            .arg(&work_repo)
            .args(["push", "-u", "origin", "HEAD"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };

        if push_output.status.success() {
            return true;
        }

        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&push_output.stdout),
            String::from_utf8_lossy(&push_output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_local_push_for_remote_management_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_local_push_available_for_remote_management_tests() {
            eprintln!(
                "skipping remote-management integration test: Git-for-Windows local push shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

fn configure_repo_with_user(repo: &Path) {
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);
}

fn init_repo_with_user(repo: &Path) {
    run_git(repo, &["init"]);
    configure_repo_with_user(repo);
}

#[test]
fn remote_add_set_url_and_remove_round_trip() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let repo = root.join("repo");
    let fetch_remote = root.join("fetch.git");
    let push_remote = root.join("push.git");

    fs::create_dir_all(&repo).expect("create repo dir");
    fs::create_dir_all(&fetch_remote).expect("create fetch remote dir");
    fs::create_dir_all(&push_remote).expect("create push remote dir");

    run_git(&fetch_remote, &["init", "--bare"]);
    run_git(&push_remote, &["init", "--bare"]);

    init_repo_with_user(&repo);

    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed file");
    run_git(&repo, &["add", "seed.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let fetch_remote_str = git_remote_url(&fetch_remote);
    let push_remote_str = git_remote_url(&push_remote);

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open repository");

    let add_output = opened
        .add_remote_with_output("origin", &fetch_remote_str)
        .expect("add remote");
    assert_eq!(add_output.exit_code, Some(0));

    let remotes = opened.list_remotes().expect("list remotes after add");
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "origin");
    assert_eq!(remotes[0].url.as_deref(), Some(fetch_remote_str.as_str()));

    let fetch_set_output = opened
        .set_remote_url_with_output("origin", &push_remote_str, RemoteUrlKind::Fetch)
        .expect("set fetch url");
    assert_eq!(fetch_set_output.exit_code, Some(0));

    let remotes_after_fetch = opened
        .list_remotes()
        .expect("list remotes after fetch url update");
    assert_eq!(remotes_after_fetch.len(), 1);
    assert_eq!(
        remotes_after_fetch[0].url.as_deref(),
        Some(push_remote_str.as_str())
    );

    let push_set_output = opened
        .set_remote_url_with_output("origin", &fetch_remote_str, RemoteUrlKind::Push)
        .expect("set push url");
    assert_eq!(push_set_output.exit_code, Some(0));

    let push_url = run_git_capture(&repo, &["config", "--get", "remote.origin.pushurl"])
        .trim()
        .to_string();
    assert_eq!(push_url, fetch_remote_str);

    let remove_output = opened
        .remove_remote_with_output("origin")
        .expect("remove remote");
    assert_eq!(remove_output.exit_code, Some(0));

    let remotes_after_remove = opened.list_remotes().expect("list remotes after remove");
    assert!(remotes_after_remove.is_empty());
}

#[test]
fn remote_management_honors_the_selected_protocol_policy() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    init_repo_with_user(&repo);

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open repository");
    let http_url = "http://git.internal/example.git";
    let blocked = opened
        .add_remote_with_output_and_policy("origin", http_url, RemoteUrlPolicy::default())
        .expect_err("HTTP must be blocked by default");
    assert!(
        blocked.to_string().contains("Allowed remote protocols"),
        "unexpected policy error: {blocked}"
    );

    let http_allowed = RemoteUrlPolicy::default().with_allowed(RemoteProtocol::Http, true);
    opened
        .add_remote_with_output_and_policy("origin", http_url, http_allowed)
        .expect("explicitly allowed HTTP remote");

    let ftp_url = "ftp://git.internal/example.git";
    assert!(
        opened
            .set_remote_url_with_output_and_policy(
                "origin",
                ftp_url,
                RemoteUrlKind::Fetch,
                RemoteUrlPolicy::default(),
            )
            .is_err(),
        "FTP must be blocked by default"
    );
    let ftp_allowed = RemoteUrlPolicy::default().with_allowed(RemoteProtocol::Ftp, true);
    opened
        .set_remote_url_with_output_and_policy("origin", ftp_url, RemoteUrlKind::Fetch, ftp_allowed)
        .expect("explicitly allowed FTP remote");
    assert_eq!(
        run_git_capture(&repo, &["config", "--get", "remote.origin.url"]).trim(),
        ftp_url
    );
}

#[test]
fn push_with_output_sets_upstream_when_missing() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "hi\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");

    let output = opened.push_with_output().expect("push with output");
    assert_eq!(output.exit_code, Some(0));

    let upstream = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    )
    .trim()
    .to_string();
    assert_eq!(upstream, "origin/feature");

    let remote_head = run_git_capture(&work_repo, &["ls-remote", "--heads", "origin", "feature"]);
    assert!(
        !remote_head.trim().is_empty(),
        "expected pushed feature branch on origin"
    );
}

#[test]
fn push_with_output_uses_tracked_upstream_when_branch_names_differ() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);
    run_git(&work_repo, &["config", "push.default", "simple"]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "main2"]);
    run_git(
        &work_repo,
        &["branch", "--set-upstream-to", "origin/main", "main2"],
    );

    fs::write(work_repo.join("file.txt"), "base\nnext\n").expect("write updated file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "next"],
    );

    let local_head = run_git_capture(&work_repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened.push_with_output().expect("push tracked upstream");
    assert_eq!(output.exit_code, Some(0));

    let remote_head = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string();
    assert_eq!(remote_head, local_head);
}

#[test]
fn delete_remote_branch_with_output_deletes_remote_and_tracking_ref() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);
    let base = run_git_capture(&work_repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    // Ensure remote-tracking refs are present before deletion.
    run_git(&work_repo, &["fetch", "--all"]);
    // Direct deletion is authoritative even if this repository subsequently
    // narrows its fetch refspec so feature would not be pruned automatically.
    run_git(
        &work_repo,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    let narrow_refspec = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    run_git(
        &work_repo,
        &["config", "--add", "remote.origin.fetch", &narrow_refspec],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .delete_remote_branch_with_output("origin", "feature")
        .expect("delete remote branch");
    assert_eq!(output.exit_code, Some(0));

    let remote_head = run_git_capture(&work_repo, &["ls-remote", "--heads", "origin", "feature"]);
    assert!(
        remote_head.trim().is_empty(),
        "expected feature branch to be deleted from origin"
    );

    let tracking_ref_status = run_git_status(
        &work_repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/feature",
        ],
    );
    assert!(
        !tracking_ref_status.success(),
        "expected local tracking ref to be removed"
    );
    assert_eq!(
        run_git_capture(
            &work_repo,
            &[
                "for-each-ref",
                "--format=%(upstream:short)",
                "refs/heads/feature",
            ],
        )
        .trim(),
        "",
        "deleting a remote branch must unlink its surviving local branch"
    );
}

#[test]
fn delete_remote_branches_with_output_deletes_every_branch_in_one_push() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    // The initial branch name depends on the host git config, so read it back
    // rather than assuming `main`.
    let base = run_git_capture(&work_repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();
    for branch in ["feat/a", "feat/b", "keep"] {
        run_git(&work_repo, &["checkout", "-b", branch, &base]);
        run_git(&work_repo, &["push", "-u", "origin", branch]);
    }
    run_git(&work_repo, &["checkout", &base]);
    run_git(&work_repo, &["fetch", "--all"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .delete_remote_branches_with_output("origin", &["feat/a".to_string(), "feat/b".to_string()])
        .expect("delete remote branches");
    assert_eq!(output.exit_code, Some(0));

    for branch in ["feat/a", "feat/b"] {
        let remote_head = run_git_capture(&work_repo, &["ls-remote", "--heads", "origin", branch]);
        assert!(
            remote_head.trim().is_empty(),
            "expected {branch} to be deleted from origin"
        );
        let tracking = run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{branch}"),
            ],
        );
        assert!(
            !tracking.success(),
            "expected the local tracking ref for {branch} to be removed"
        );
        assert_eq!(
            run_git_capture(
                &work_repo,
                &[
                    "for-each-ref",
                    "--format=%(upstream:short)",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .trim(),
            "",
            "expected {branch} to be unlinked"
        );
    }

    // A branch outside the batch is untouched, both on the remote and locally.
    assert!(
        !run_git_capture(&work_repo, &["ls-remote", "--heads", "origin", "keep"])
            .trim()
            .is_empty(),
        "expected `keep` to survive the batch delete"
    );
    assert!(
        run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/keep"
            ],
        )
        .success(),
        "expected the tracking ref for `keep` to survive"
    );
    assert_eq!(
        run_git_capture(
            &work_repo,
            &[
                "for-each-ref",
                "--format=%(upstream:short)",
                "refs/heads/keep",
            ],
        )
        .trim(),
        "origin/keep",
        "a branch outside the deletion batch must keep its upstream"
    );
}

/// A failed batch must not prune the tracking refs of branches that are still on
/// the remote.
///
/// For a missing refspec git rejects the whole push and deletes nothing, so
/// blanket-pruning on failure — the obvious shortcut — would erase the sidebar
/// rows for branches that very much still exist.
#[test]
fn delete_remote_branches_keeps_tracking_refs_when_the_batch_deletes_nothing() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    let base = run_git_capture(&work_repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();
    run_git(&work_repo, &["checkout", "-b", "feat/a", &base]);
    run_git(&work_repo, &["push", "-u", "origin", "feat/a"]);
    run_git(&work_repo, &["checkout", &base]);
    run_git(&work_repo, &["fetch", "--all"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    // `feat/gone` never existed on the remote, so the push exits non-zero.
    let result = opened.delete_remote_branches_with_output(
        "origin",
        &["feat/a".to_string(), "feat/gone".to_string()],
    );
    assert!(
        result.is_err(),
        "a missing refspec must surface as an error"
    );

    // git refuses the whole push when a refspec names a ref the remote does not
    // have, so `feat/a` survives...
    assert!(
        !run_git_capture(&work_repo, &["ls-remote", "--heads", "origin", "feat/a"])
            .trim()
            .is_empty(),
        "expected feat/a to survive a rejected batch"
    );
    // ...and its tracking ref must survive with it.
    assert!(
        run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/feat/a",
            ],
        )
        .success(),
        "a failed batch must not prune a branch that is still on the remote"
    );
}

#[test]
fn prune_merged_branches_with_output_reports_noop_when_nothing_to_prune() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "seed\n").expect("write seed file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .prune_merged_branches_with_output()
        .expect("prune merged branches");

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.contains("No merged local branches to prune."),
        "unexpected prune stdout: {}",
        output.stdout
    );
}

#[test]
fn prune_merged_branches_unlinks_but_keeps_an_unmerged_local_branch() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);
    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);
    let base = run_git_capture(&work_repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);
    run_git(&work_repo, &["checkout", &base]);
    run_git(&remote_repo, &["update-ref", "-d", "refs/heads/feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .prune_merged_branches_with_output()
        .expect("prune merged branches");

    assert!(
        output.stdout.contains("No merged local branches to prune."),
        "the unmerged local branch must not be deleted: {}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("Unlinked deleted upstream branches:\n- feature"),
        "the stale upstream must be reported: {}",
        output.stdout
    );
    assert!(
        run_git_status(
            &work_repo,
            &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
        )
        .success(),
        "the unmerged local branch must survive"
    );
    assert!(
        !run_git_status(&work_repo, &["config", "--get", "branch.feature.remote"],).success(),
        "the surviving branch must no longer track the deleted remote branch"
    );
    assert!(
        opened
            .list_remote_branches()
            .expect("list remote branches")
            .iter()
            .all(|branch| !(branch.remote == "origin" && branch.name == "feature")),
        "the deleted remote branch must disappear from the backend list"
    );
}

#[test]
fn fetch_all_variants_prune_deleted_remote_tracking_branches() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    let feature_commit = run_git_capture(&work_repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    run_git(&work_repo, &["tag", "obsolete-remote-tag"]);
    run_git(&work_repo, &["push", "origin", "obsolete-remote-tag"]);
    run_git(&work_repo, &["pack-refs", "--all", "--prune"]);

    let tracking_ref = "refs/remotes/origin/feature";
    let tracking_ref_present = || {
        run_git_status(
            &work_repo,
            &["show-ref", "--verify", "--quiet", tracking_ref],
        )
    };
    let feature_upstream = || {
        run_git_capture(
            &work_repo,
            &[
                "for-each-ref",
                "--format=%(upstream:short)",
                "refs/heads/feature",
            ],
        )
        .trim()
        .to_string()
    };

    assert!(
        tracking_ref_present().success(),
        "expected local tracking ref to exist before remote deletion"
    );
    assert_eq!(feature_upstream(), "origin/feature");

    run_git(&remote_repo, &["update-ref", "-d", "refs/heads/feature"]);
    run_git(
        &remote_repo,
        &["update-ref", "-d", "refs/tags/obsolete-remote-tag"],
    );
    assert!(
        tracking_ref_present().success(),
        "expected local tracking ref to remain stale until fetch --prune"
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    assert!(
        opened
            .list_remote_branches()
            .expect("prime packed remote branch snapshot")
            .iter()
            .any(|branch| branch.remote == "origin" && branch.name == "feature"),
        "expected the stale tracking branch to be visible before pruning"
    );
    let output = opened
        .fetch_all_with_output_prune(false)
        .expect("fetch all without pruning");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.command, "git fetch --all --no-prune --no-prune-tags");
    assert!(
        tracking_ref_present().success(),
        "expected fetch with pruning disabled to preserve stale tracking refs"
    );
    assert_eq!(
        feature_upstream(),
        "origin/feature",
        "a non-pruning fetch must preserve upstream configuration"
    );

    let output = opened
        .fetch_all_with_output_prune(true)
        .expect("fetch all with pruning");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.command, "git fetch --all --prune --no-prune-tags");
    assert!(
        output
            .stdout
            .contains("Unlinked deleted upstream branches:\n- feature"),
        "expected fetch output to report the unlinked local branch: {}",
        output.stdout
    );
    assert!(
        !tracking_ref_present().success(),
        "expected fetch with pruning enabled to prune stale remote-tracking refs"
    );
    assert!(
        opened
            .list_remote_branches()
            .expect("refresh remote branch list after pruning")
            .iter()
            .all(|branch| !(branch.remote == "origin" && branch.name == "feature")),
        "expected the backend refresh to omit the pruned tracking branch"
    );
    assert!(
        run_git_status(
            &work_repo,
            &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
        )
        .success(),
        "pruning must preserve the local branch"
    );
    assert_eq!(
        feature_upstream(),
        "",
        "a successful pruning fetch must unlink the deleted upstream"
    );
    assert!(
        run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/tags/obsolete-remote-tag"
            ],
        )
        .success(),
        "expected branch pruning to preserve a locally present tag deleted from the remote"
    );

    run_git(
        &work_repo,
        &["update-ref", tracking_ref, feature_commit.as_str()],
    );
    assert!(
        tracking_ref_present().success(),
        "expected stale tracking ref recreation to succeed"
    );

    opened.fetch_all().expect("fetch all");
    assert!(
        !tracking_ref_present().success(),
        "expected fetch_all to prune stale remote-tracking refs"
    );
}

#[test]
fn pruning_fetch_preserves_upstreams_outside_a_narrow_fetch_refspec() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);
    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);
    let base = run_git_capture(&work_repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    // This clone only fetches the base branch. A missing origin/feature ref is
    // therefore not evidence that the remote deleted feature.
    run_git(
        &work_repo,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    let narrow_refspec = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    run_git(
        &work_repo,
        &["config", "--add", "remote.origin.fetch", &narrow_refspec],
    );
    run_git(
        &work_repo,
        &["update-ref", "-d", "refs/remotes/origin/feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .fetch_all_with_output_prune(true)
        .expect("pruning fetch with narrow refspec");

    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.feature.remote"]).trim(),
        "origin",
        "fetch must preserve an upstream outside its authoritative refspec"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.feature.merge"]).trim(),
        "refs/heads/feature"
    );
    let feature = opened
        .list_branches()
        .expect("list branches")
        .into_iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch");
    assert_eq!(
        feature.upstream, None,
        "an excluded, missing tracking ref is not a live upstream in the UI"
    );
}

#[test]
fn push_force_without_output_updates_remote_head_after_rewrite() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    fs::write(work_repo.join("file.txt"), "base\nnext\n").expect("write updated file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "next"],
    );
    run_git(&work_repo, &["push"]);

    let remote_head_before = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string();

    run_git(&work_repo, &["reset", "--hard", "HEAD~1"]);
    fs::write(work_repo.join("file.txt"), "base\nrewritten\n").expect("write rewritten file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "rewritten"],
    );

    let local_head = run_git_capture(&work_repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened.push_force().expect("force push");

    let remote_head_after = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string();
    assert_ne!(remote_head_before, remote_head_after);
    assert_eq!(remote_head_after, local_head);
}

#[test]
fn push_force_with_output_uses_tracked_upstream_when_branch_names_differ() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);
    run_git(&work_repo, &["config", "push.default", "simple"]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "main2"]);
    run_git(
        &work_repo,
        &["branch", "--set-upstream-to", "origin/main", "main2"],
    );

    fs::write(work_repo.join("file.txt"), "base\nnext\n").expect("write updated file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "next"],
    );
    run_git(&work_repo, &["push", "origin", "HEAD:refs/heads/main"]);
    run_git(&work_repo, &["fetch", "origin"]);

    run_git(&work_repo, &["reset", "--hard", "HEAD~1"]);
    fs::write(work_repo.join("file.txt"), "base\nrewritten\n").expect("write rewritten file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "rewritten"],
    );

    let local_head = run_git_capture(&work_repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .push_force_with_output()
        .expect("force push tracked upstream");
    assert_eq!(output.exit_code, Some(0));

    let remote_head = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/main"])
        .trim()
        .to_string();
    assert_eq!(remote_head, local_head);
}

#[test]
fn pull_non_output_supports_all_modes_when_upstream_exists() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let origin = root.join("origin.git");
    let repo_a = root.join("repo-a");
    let repo_b = root.join("repo-b");
    fs::create_dir_all(&origin).expect("create origin dir");
    fs::create_dir_all(&repo_a).expect("create repo-a dir");

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&repo_a, &["init", "-b", "main"]);
    configure_repo_with_user(&repo_a);
    fs::write(repo_a.join("a.txt"), "one\n").expect("write initial file");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let origin_url = git_remote_url(&origin);
    run_git(&repo_a, &["remote", "add", "origin", origin_url.as_str()]);
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

    run_git(
        root,
        &[
            "clone",
            origin_url.as_str(),
            repo_b.to_string_lossy().as_ref(),
        ],
    );
    configure_repo_with_user(&repo_b);

    fs::write(repo_a.join("a.txt"), "one\ntwo\n").expect("write updated file");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo_a, &["push"]);

    let backend = GixBackend;
    let opened_b = backend.open(&repo_b).expect("open repo-b");
    let stale_target = run_git_capture(&repo_b, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    run_git(
        &repo_b,
        &["update-ref", "refs/remotes/origin/deleted", &stale_target],
    );
    opened_b
        .pull_with_output_prune(PullMode::Default, true)
        .expect("pull and prune stale tracking refs");
    assert!(
        !run_git_status(
            &repo_b,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/deleted",
            ],
        )
        .success(),
        "expected pull with pruning enabled to remove stale remote-tracking refs"
    );
    opened_b
        .pull(PullMode::FastForwardIfPossible)
        .expect("pull ff-if-possible");
    opened_b.pull(PullMode::Merge).expect("pull merge");
    opened_b
        .pull(PullMode::FastForwardOnly)
        .expect("pull ff-only");
    opened_b.pull(PullMode::Rebase).expect("pull rebase");
    opened_b.pull(PullMode::Default).expect("pull default");
}

#[test]
fn failed_pruning_pull_unlinks_an_upstream_removed_by_its_fetch_phase() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("origin.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let remote_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", remote_url.as_str()],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    run_git(&remote_repo, &["update-ref", "-d", "refs/heads/main"]);

    opened
        .pull_with_output_prune(PullMode::Default, true)
        .expect_err("pull must fail after its upstream is deleted");

    assert!(
        !run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/main",
            ],
        )
        .success(),
        "the fetch phase must prune the deleted tracking ref"
    );
    assert!(
        !run_git_status(&work_repo, &["config", "--get", "branch.main.remote"],).success(),
        "a later pull failure must not leave stale upstream configuration"
    );
    assert!(
        run_git_status(
            &work_repo,
            &["show-ref", "--verify", "--quiet", "refs/heads/main"],
        )
        .success(),
        "unlinking the upstream must preserve the local branch"
    );
}

#[test]
fn failed_pruning_pull_preserves_an_upstream_whose_tracking_ref_was_already_missing() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");

    assert!(
        ref_exists(&remote_repo, "refs/heads/main"),
        "the upstream branch must still exist at the real remote endpoint"
    );
    run_git(
        &work_repo,
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );
    let unavailable_url = git_remote_url(&dir.path().join("unavailable.git"));
    run_git(
        &work_repo,
        &["remote", "set-url", "origin", unavailable_url.as_str()],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let error = opened
        .pull_with_output_prune(PullMode::Default, true)
        .expect_err("an unavailable remote must fail the pull");

    assert!(
        !error.to_string().contains("no longer exists"),
        "the original transport failure must not be reclassified as a deletion: {error}"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.main.remote"]).trim(),
        "origin",
        "a failed fetch cannot prove that an already-untracked upstream was deleted"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.main.merge"]).trim(),
        "refs/heads/main"
    );
    assert!(
        ref_exists(&work_repo, "refs/heads/main"),
        "the local branch must survive the network failure"
    );
    assert!(
        ref_exists(&remote_repo, "refs/heads/main"),
        "the live remote branch must be untouched"
    );
}

#[test]
fn push_without_origin_uses_first_remote_name_for_upstream() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("backup.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare"]);
    init_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "backup", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened.push().expect("push branch");

    let upstream = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    )
    .trim()
    .to_string();
    assert_eq!(upstream, "backup/feature");
}

#[test]
fn pull_without_remotes_on_local_branch_returns_error() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();
    init_repo_with_user(repo);

    fs::write(repo.join("a.txt"), "one\n").expect("write file");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repository");
    assert!(opened.pull(PullMode::Default).is_err());
}

#[test]
fn pull_on_detached_head_returns_error() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();
    init_repo_with_user(repo);

    fs::write(repo.join("a.txt"), "one\n").expect("write file");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["checkout", "--detach", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repository");
    assert!(opened.pull(PullMode::Default).is_err());
}

#[test]
fn pull_local_branch_with_pruning_does_not_treat_dot_as_a_named_remote() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();
    init_repo_with_user(repo);
    run_git(repo, &["branch", "-m", "main"]);

    fs::write(repo.join("base.txt"), "base\n").expect("write base file");
    run_git(repo, &["add", "base.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(repo, &["checkout", "-b", "dev"]);
    fs::write(repo.join("dev.txt"), "dev\n").expect("write dev file");
    run_git(repo, &["add", "dev.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "dev"]);
    run_git(repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repository");
    let output = opened
        .pull_branch_with_output_prune(".", "dev", true)
        .expect("pull local branch while automatic pruning is enabled");

    assert_eq!(output.exit_code, Some(0));
    assert!(repo.join("dev.txt").exists(), "expected dev to be merged");
}

#[test]
fn pull_branch_with_output_merges_named_remote_branch() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    let remote_str = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_str]);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);
    run_git(&work_repo, &["checkout", "main"]);

    let stale_target = run_git_capture(&work_repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    run_git(
        &work_repo,
        &["update-ref", "refs/remotes/origin/deleted", &stale_target],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .pull_branch_with_output_prune("origin", "feature", true)
        .expect("pull branch with output");
    assert_eq!(output.exit_code, Some(0));
    assert!(
        !run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/deleted",
            ],
        )
        .success(),
        "expected pull-into-current with pruning enabled to remove stale tracking refs"
    );

    let merged = run_git_capture(&work_repo, &["show-ref", "--verify", "refs/heads/main"]);
    assert!(
        !merged.trim().is_empty(),
        "expected main branch to remain valid"
    );

    run_git(&remote_repo, &["update-ref", "-d", "refs/heads/feature"]);
    let error = opened
        .pull_branch_with_output_prune("origin", "feature", true)
        .expect_err("deleted remote branch should not be merged");
    assert!(
        error
            .to_string()
            .contains("origin/feature no longer exists"),
        "expected a friendly stale-branch error, got: {error}"
    );
    assert!(
        !run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/feature",
            ],
        )
        .success(),
        "expected the deleted requested branch to be pruned"
    );
}

#[test]
fn pruning_pull_preserves_tags_from_explicit_fetch_refspecs() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let remote_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", remote_url.as_str()],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    // `--no-prune-tags` only disables Git's implicit tag refspec. This
    // explicit destination used to let `git pull --prune` delete the tag.
    run_git(
        &work_repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/tags/*:refs/tags/*",
        ],
    );
    run_git(&work_repo, &["update-ref", "refs/tags/local-only", "HEAD"]);
    run_git(
        &work_repo,
        &["update-ref", "refs/remotes/origin/deleted", "HEAD"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .pull_with_output_prune(PullMode::Default, true)
        .expect("pull with branch pruning");

    assert!(
        run_git_status(
            &work_repo,
            &["show-ref", "--verify", "--quiet", "refs/tags/local-only"],
        )
        .success(),
        "branch pruning must never delete a tag selected by a configured refspec"
    );
    assert!(
        !run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/deleted",
            ],
        )
        .success(),
        "the same pull must still prune stale remote-tracking branches"
    );
}

#[test]
fn pruning_pull_branch_fetches_a_requested_branch_excluded_by_remote_refspecs() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    let publisher_repo = root.join("publisher");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("base.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "base.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let remote_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", remote_url.as_str()],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);
    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "old\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "old feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);
    run_git(&work_repo, &["checkout", "main"]);

    run_git(
        root,
        &[
            "clone",
            remote_url.as_str(),
            publisher_repo.to_string_lossy().as_ref(),
        ],
    );
    configure_repo_with_user(&publisher_repo);
    run_git(&publisher_repo, &["checkout", "feature"]);
    fs::write(publisher_repo.join("remote-new.txt"), "new\n").expect("write remote update");
    run_git(&publisher_repo, &["add", "remote-new.txt"]);
    run_git(
        &publisher_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "new feature"],
    );
    run_git(&publisher_repo, &["push", "origin", "feature"]);
    let remote_feature = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/feature"])
        .trim()
        .to_string();

    run_git(
        &work_repo,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    run_git(
        &work_repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .pull_branch_with_output_prune("origin", "feature", true)
        .expect("pull excluded branch into current");

    assert_eq!(
        run_git_capture(&work_repo, &["rev-parse", "HEAD"]).trim(),
        remote_feature,
        "pull-into-current must merge the freshly fetched remote tip, not a stale tracking ref"
    );
    assert!(
        work_repo.join("remote-new.txt").exists(),
        "the newest remote commit must be present in the worktree"
    );
}

#[cfg(unix)]
#[test]
fn pruning_pull_branch_recognizes_a_localized_missing_remote_ref() {
    const CHILD_ENV: &str = "GITCOMET_LOCALIZED_MISSING_REF_TEST_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let status = Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg("pruning_pull_branch_recognizes_a_localized_missing_remote_ref")
            .arg("--nocapture")
            .env(CHILD_ENV, "1")
            // The wrapper below emits a localized diagnostic unless the caller
            // explicitly pins the stable C locale. Running in a child keeps
            // this process-wide setting isolated from concurrent tests.
            .env("LC_ALL", "POSIX")
            .status()
            .expect("run localized child test");
        assert!(status.success(), "localized child test failed");
        return;
    }

    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();
    let (remote_repo, work_repo) = init_work_repo_with_remote(root, "origin");

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);
    run_git(&work_repo, &["checkout", "main"]);
    run_git(
        &work_repo,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    run_git(
        &work_repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    );
    run_git(&remote_repo, &["update-ref", "-d", "refs/heads/feature"]);
    assert!(
        ref_exists(&work_repo, "refs/remotes/origin/feature"),
        "the narrow refspec must leave a stale tracking ref for the explicit probe"
    );

    let wrapper = root.join("localized-git");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
saw_fetch=false
saw_feature=false
for argument in "$@"; do
    if [ "$argument" = "fetch" ]; then
        saw_fetch=true
    fi
    if [ "$argument" = "refs/heads/feature" ]; then
        saw_feature=true
    fi
done
if [ "$saw_fetch" = true ] && [ "$saw_feature" = true ] && [ "${LC_ALL-}" != C ]; then
    printf '%s\n' 'ödesdigert: kunde inte hitta fjärr-referensen refs/heads/feature' >&2
    exit 128
fi
exec git "$@"
"#,
    )
    .expect("write localized Git wrapper");
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = fs::metadata(&wrapper)
        .expect("localized Git wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper, permissions).expect("make localized Git wrapper executable");
    let _git_executable = GitExecutablePreferenceGuard::install(&wrapper);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let error = opened
        .pull_branch_with_output_prune("origin", "feature", true)
        .expect_err("the deleted remote branch must not be merged");

    assert!(
        error
            .to_string()
            .contains("origin/feature no longer exists"),
        "localized missing-ref output must be classified as a deletion: {error}"
    );
    assert!(
        !ref_exists(&work_repo, "refs/remotes/origin/feature"),
        "the confirmed-deleted tracking ref must be removed"
    );
    assert!(
        !run_git_status(&work_repo, &["config", "--get", "branch.feature.remote"]).success(),
        "the confirmed-deleted upstream must be unlinked"
    );
}

#[test]
fn pruning_pull_branch_merges_the_fetched_oid_when_a_short_ref_is_ambiguous() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("base.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "base.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let remote_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", remote_url.as_str()],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("remote-only.txt"), "remote\n").expect("write remote-only file");
    run_git(&work_repo, &["add", "remote-only.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "remote feature",
        ],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);
    let remote_feature = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/feature"])
        .trim()
        .to_string();

    run_git(&work_repo, &["checkout", "main"]);
    run_git(&work_repo, &["checkout", "-b", "origin/feature"]);
    fs::write(work_repo.join("local-only.txt"), "local\n").expect("write colliding local file");
    run_git(&work_repo, &["add", "local-only.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "ambiguous local branch",
        ],
    );
    run_git(&work_repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .pull_branch_with_output_prune("origin", "feature", true)
        .expect("pull unambiguous fetched commit");

    assert_eq!(
        run_git_capture(&work_repo, &["rev-parse", "HEAD"]).trim(),
        remote_feature,
        "a local origin/feature branch must not shadow refs/remotes/origin/feature"
    );
    assert!(work_repo.join("remote-only.txt").exists());
    assert!(!work_repo.join("local-only.txt").exists());
}

#[test]
fn failed_fetch_all_cleans_up_refs_pruned_before_a_later_remote_fails() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let good_remote = root.join("good.git");
    let missing_remote = root.join("missing.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&good_remote).expect("create good remote dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&good_remote, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("base.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "base.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let good_url = git_remote_url(&good_remote);
    run_git(&work_repo, &["remote", "add", "a-good", good_url.as_str()]);
    run_git(&work_repo, &["push", "-u", "a-good", "main"]);
    run_git(&work_repo, &["checkout", "-b", "feature"]);
    run_git(&work_repo, &["push", "-u", "a-good", "feature"]);
    run_git(&work_repo, &["checkout", "main"]);
    let missing_url = git_remote_url(&missing_remote);
    run_git(
        &work_repo,
        &["remote", "add", "z-broken", missing_url.as_str()],
    );
    run_git(&good_remote, &["update-ref", "-d", "refs/heads/feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .fetch_all_with_output_prune(true)
        .expect_err("the later inaccessible remote must fail fetch --all");

    assert!(
        !run_git_status(
            &work_repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/a-good/feature",
            ],
        )
        .success(),
        "the earlier remote should already have pruned its deleted tracking ref"
    );
    assert!(
        !run_git_status(&work_repo, &["config", "--get", "branch.feature.remote"],).success(),
        "a partial fetch failure must still unlink an upstream whose tracking ref disappeared"
    );
}

#[test]
fn remote_deletion_preserves_fetch_tracking_when_pushurl_is_different() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let fetch_remote = root.join("fetch.git");
    let push_remote = root.join("push.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&fetch_remote).expect("create fetch remote dir");
    fs::create_dir_all(&push_remote).expect("create push remote dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&fetch_remote, &["init", "--bare", "-b", "main"]);
    run_git(&push_remote, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("base.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "base.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let fetch_url = git_remote_url(&fetch_remote);
    let push_url = git_remote_url(&push_remote);
    run_git(&work_repo, &["remote", "add", "origin", fetch_url.as_str()]);
    run_git(&work_repo, &["push", "-u", "origin", "main"]);
    run_git(
        &work_repo,
        &["push", push_url.as_str(), "main:refs/heads/main"],
    );

    for branch in ["single", "batch/a", "batch/b"] {
        run_git(&work_repo, &["checkout", "-b", branch, "main"]);
        run_git(&work_repo, &["push", "-u", "origin", branch]);
        let refspec = format!("{branch}:refs/heads/{branch}");
        run_git(&work_repo, &["push", push_url.as_str(), &refspec]);
    }
    run_git(&work_repo, &["checkout", "main"]);
    run_git(
        &work_repo,
        &["config", "remote.origin.pushurl", push_url.as_str()],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .delete_remote_branch_with_output("origin", "single")
        .expect("delete single branch from push endpoint");
    opened
        .delete_remote_branches_with_output(
            "origin",
            &["batch/a".to_string(), "batch/b".to_string()],
        )
        .expect("delete batch from push endpoint");

    for branch in ["single", "batch/a", "batch/b"] {
        let remote_ref = format!("refs/heads/{branch}");
        assert!(
            run_git_status(
                &fetch_remote,
                &["show-ref", "--verify", "--quiet", &remote_ref],
            )
            .success(),
            "{branch} must remain on the fetch endpoint"
        );
        assert!(
            !run_git_status(
                &push_remote,
                &["show-ref", "--verify", "--quiet", &remote_ref],
            )
            .success(),
            "{branch} should be deleted from the push endpoint"
        );
        let tracking_ref = format!("refs/remotes/origin/{branch}");
        assert!(
            run_git_status(
                &work_repo,
                &["show-ref", "--verify", "--quiet", &tracking_ref],
            )
            .success(),
            "{branch}'s fetch tracking ref must remain"
        );
        let branch_remote_key = format!("branch.{branch}.remote");
        assert_eq!(
            run_git_capture(&work_repo, &["config", "--get", &branch_remote_key]).trim(),
            "origin",
            "{branch}'s upstream must remain linked to the live fetch endpoint"
        );
    }
}

/// Set up a work repo with one commit pushed to a bare remote named `remote_name`.
fn init_work_repo_with_remote(
    root: &Path,
    remote_name: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).expect("create remote repo dir");
    fs::create_dir_all(&work_repo).expect("create work repo dir");
    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(&work_repo);

    fs::write(work_repo.join("file.txt"), "base\n").expect("write base file");
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let remote_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", remote_name, remote_url.as_str()],
    );
    run_git(&work_repo, &["push", "-u", remote_name, "main"]);
    (remote_repo, work_repo)
}

fn ref_exists(repo: &Path, ref_name: &str) -> bool {
    run_git_status(repo, &["show-ref", "--verify", "--quiet", ref_name]).success()
}

fn assert_fetch_all_preserves_upstream_for_skipped_remote(
    root: &Path,
    remote_name: &str,
    skip_key: &str,
) {
    let (_remote_repo, work_repo) = init_work_repo_with_remote(root, "origin");
    let unavailable_url = git_remote_url(&root.join("unavailable.git"));
    run_git(
        &work_repo,
        &["remote", "add", remote_name, unavailable_url.as_str()],
    );
    let skip_config = format!("remote.{remote_name}.{skip_key}");
    run_git(&work_repo, &["config", skip_config.as_str(), "true"]);
    run_git(&work_repo, &["branch", "offline", "main"]);
    run_git(
        &work_repo,
        &["config", "branch.offline.remote", remote_name],
    );
    run_git(
        &work_repo,
        &["config", "branch.offline.merge", "refs/heads/offline"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .fetch_all_with_output_prune(true)
        .expect("fetch --all must skip the unavailable remote");

    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.offline.remote"]).trim(),
        remote_name,
        "a fetch that skips {remote_name:?} via {skip_key} cannot prove its upstream was deleted"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.offline.merge"]).trim(),
        "refs/heads/offline"
    );
}

#[test]
fn pruning_fetch_all_preserves_tags_from_explicit_fetch_refspecs() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");

    // `--no-prune-tags` only disables Git's implicit tag refspec. This explicit
    // destination used to let `git fetch --all --prune` delete the tag.
    run_git(
        &work_repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/tags/*:refs/tags/*",
        ],
    );
    run_git(&work_repo, &["update-ref", "refs/tags/local-only", "HEAD"]);
    run_git(
        &work_repo,
        &["update-ref", "refs/remotes/origin/deleted", "HEAD"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .fetch_all_with_output_prune(true)
        .expect("fetch all with pruning");

    assert!(
        ref_exists(&work_repo, "refs/tags/local-only"),
        "branch pruning must never delete a tag selected by a configured refspec"
    );
    assert!(
        !ref_exists(&work_repo, "refs/remotes/origin/deleted"),
        "the same fetch must still prune stale remote-tracking branches"
    );
}

#[test]
fn pruning_fetch_all_keeps_upstreams_of_remotes_it_does_not_fetch() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");

    // A second remote that `git fetch --all` never contacts, so it says nothing
    // about whether its branches still exist.
    let skipped_repo = dir.path().join("skipped.git");
    fs::create_dir_all(&skipped_repo).expect("create skipped remote dir");
    run_git(&skipped_repo, &["init", "--bare", "-b", "main"]);
    let skipped_url = git_remote_url(&skipped_repo);
    run_git(
        &work_repo,
        &["remote", "add", "skipped", skipped_url.as_str()],
    );
    run_git(
        &work_repo,
        &["config", "remote.skipped.skipFetchAll", "true"],
    );
    run_git(&work_repo, &["branch", "offline", "main"]);
    run_git(&work_repo, &["config", "branch.offline.remote", "skipped"]);
    run_git(
        &work_repo,
        &["config", "branch.offline.merge", "refs/heads/offline"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .fetch_all_with_output_prune(true)
        .expect("fetch all with pruning");

    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.offline.remote"]).trim(),
        "skipped",
        "a fetch that skips a remote must not unlink upstreams pointing at it"
    );
}

#[test]
fn pruning_fetch_all_keeps_upstreams_skipped_by_skip_default_update() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    assert_fetch_all_preserves_upstream_for_skipped_remote(
        dir.path(),
        "skip-default",
        "skipDefaultUpdate",
    );
}

#[test]
fn pruning_fetch_all_keeps_upstreams_of_unsafe_remote_names_skipped_by_fetch_all() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    assert_fetch_all_preserves_upstream_for_skipped_remote(dir.path(), "a=b", "skipFetchAll");
}

#[test]
fn pruning_pull_refuses_to_retarget_a_deleted_upstream_at_another_branch() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");

    // `wip` tracks `origin/pr-123`; `origin/wip` exists but is a different
    // branch the user never asked to merge.
    run_git(&work_repo, &["checkout", "-q", "-b", "wip"]);
    run_git(&work_repo, &["push", "-q", "origin", "wip:pr-123"]);
    fs::write(work_repo.join("other.txt"), "other\n").expect("write other file");
    run_git(&work_repo, &["add", "other.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "other"],
    );
    run_git(&work_repo, &["push", "-q", "origin", "wip"]);
    run_git(&work_repo, &["reset", "-q", "--hard", "HEAD~1"]);
    run_git(&work_repo, &["config", "branch.wip.remote", "origin"]);
    run_git(
        &work_repo,
        &["config", "branch.wip.merge", "refs/heads/pr-123"],
    );
    run_git(&work_repo, &["fetch", "-q", "origin"]);
    run_git(&remote_repo, &["update-ref", "-d", "refs/heads/pr-123"]);

    let head_before = run_git_capture(&work_repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let err = opened
        .pull_with_output_prune(PullMode::Default, true)
        .expect_err("pull must not fall back to another remote branch");

    assert!(
        err.to_string().contains("origin/pr-123 no longer exists"),
        "expected the deleted upstream to be named, got: {err}"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["rev-parse", "HEAD"]).trim(),
        head_before,
        "a deleted upstream must never merge a different remote branch"
    );
    assert!(
        !run_git_status(&work_repo, &["config", "--get", "branch.wip.merge"]).success(),
        "the deleted upstream must be unlinked, not retargeted"
    );
}

#[test]
fn pruning_pull_prunes_inside_the_pull_when_the_remote_only_maps_tracking_refs() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");
    run_git(
        &work_repo,
        &["update-ref", "refs/remotes/origin/deleted", "HEAD"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .pull_with_output_prune(PullMode::Default, true)
        .expect("pull with branch pruning");

    assert_eq!(
        output.command, "git pull --prune",
        "a remote whose refspecs only map refs/remotes/ needs one fetch, not two"
    );
    assert!(
        !ref_exists(&work_repo, "refs/remotes/origin/deleted"),
        "the pull must still prune stale remote-tracking branches"
    );
}

#[test]
fn pruning_pull_branch_merges_the_pruned_tracking_ref_without_a_second_fetch() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");

    run_git(&work_repo, &["checkout", "-q", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").expect("write feature file");
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-q", "origin", "feature"]);
    let remote_feature = run_git_capture(&remote_repo, &["rev-parse", "refs/heads/feature"])
        .trim()
        .to_string();
    run_git(&work_repo, &["checkout", "-q", "main"]);
    run_git(&work_repo, &["branch", "-q", "-D", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened
        .pull_branch_with_output_prune("origin", "feature", true)
        .expect("pull the named remote branch");

    assert_eq!(
        output.command, "git fetch origin --prune && git merge refs/remotes/origin/feature",
        "a refspec that maps every branch makes the prune fetch authoritative"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["rev-parse", "HEAD"]).trim(),
        remote_feature
    );
}

#[test]
fn pruning_pull_keeps_tags_when_the_remote_name_cannot_be_a_config_key() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "a=b");

    // `git -c remote.a=b.pruneTags=false` parses as `remote.a` = `b.pruneTags=false`,
    // so tag pruning can only be suppressed by keeping it out of the pull.
    run_git(&work_repo, &["config", "remote.a=b.pruneTags", "true"]);
    run_git(&work_repo, &["update-ref", "refs/tags/local-only", "HEAD"]);
    run_git(
        &work_repo,
        &["update-ref", "refs/remotes/a=b/deleted", "HEAD"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    opened
        .pull_with_output_prune(PullMode::Default, true)
        .expect("pull with branch pruning");

    assert!(
        ref_exists(&work_repo, "refs/tags/local-only"),
        "branch pruning must never delete a local tag"
    );
    assert!(
        !ref_exists(&work_repo, "refs/remotes/a=b/deleted"),
        "the same pull must still prune stale remote-tracking branches"
    );
}

#[test]
fn delete_remote_branch_succeeds_when_the_upstream_cleanup_cannot_write_config() {
    let _guard = remote_management_test_lock();
    if !require_git_local_push_for_remote_management_tests() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_remote_repo, work_repo) = init_work_repo_with_remote(dir.path(), "origin");
    run_git(&work_repo, &["checkout", "-q", "-b", "feature"]);
    run_git(&work_repo, &["push", "-q", "-u", "origin", "feature"]);
    run_git(&work_repo, &["checkout", "-q", "main"]);

    // A held config lock - another Git process, say - makes
    // `git branch --unset-upstream` fail after the branch has already been
    // deleted on the remote.
    let config_lock = work_repo.join(".git").join("config.lock");
    fs::write(&config_lock, "").expect("hold the config lock");

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open work repo");
    let output = opened.delete_remote_branch_with_output("origin", "feature");

    fs::remove_file(&config_lock).expect("release the config lock");

    let output = output.expect("a completed remote deletion must not be reported as a failure");
    assert!(
        output.stderr.contains("Could not unlink"),
        "the failed cleanup belongs in the output, got: {:?}",
        output.stderr
    );
    assert!(
        !ref_exists(&work_repo, "refs/remotes/origin/feature"),
        "the remote branch deletion itself must still take effect"
    );
}
