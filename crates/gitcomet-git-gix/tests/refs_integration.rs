use gitcomet_core::domain::{Upstream, UpstreamDivergence};
use gitcomet_core::services::{CheckoutRemoteBranchMode, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;
#[cfg(windows)]
use std::sync::OnceLock;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn run_git_capture(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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

fn initialize_repo_with_commit(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("file.txt"), "base\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
}

fn upstream(remote: &str, branch: &str) -> Upstream {
    Upstream {
        remote: remote.to_string(),
        branch: branch.to_string(),
    }
}

#[test]
fn overlapping_remote_names_keep_exact_branch_identities() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    initialize_repo_with_commit(&repo);

    // `git remote add` rejects this overlapping pair, but the configuration is
    // valid syntax and can arise from hand-edited or generated config.
    run_git(&repo, &["config", "remote.team.url", "."]);
    run_git(
        &repo,
        &[
            "config",
            "remote.team.fetch",
            "+refs/heads/*:refs/remotes/team/*",
        ],
    );
    run_git(&repo, &["config", "remote.team/alice.url", "."]);
    run_git(
        &repo,
        &[
            "config",
            "remote.team/alice.fetch",
            "+refs/heads/*:refs/remotes/team/alice/*",
        ],
    );
    run_git(
        &repo,
        &["update-ref", "refs/remotes/team/alice/main", "HEAD"],
    );

    let opened = GixBackend.open(&repo).unwrap();
    let branches = opened.list_remote_branches().unwrap();
    assert_eq!(
        branches
            .iter()
            .map(|branch| (branch.remote.as_str(), branch.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("team", "alice/main"), ("team/alice", "main")]
    );
}

#[test]
fn structured_upstream_target_preserves_an_ambiguous_name_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    initialize_repo_with_commit(&repo);
    run_git(&repo, &["branch", "feature"]);
    run_git(&repo, &["config", "remote.team.url", "."]);
    run_git(
        &repo,
        &[
            "config",
            "remote.team.fetch",
            "+refs/heads/*:refs/remotes/team/*",
        ],
    );
    run_git(&repo, &["config", "remote.team/alice.url", "."]);
    run_git(
        &repo,
        &[
            "config",
            "remote.team/alice.fetch",
            "+refs/heads/*:refs/remotes/team/alice/*",
        ],
    );
    run_git(
        &repo,
        &["update-ref", "refs/remotes/team/alice/main", "HEAD"],
    );

    let opened = GixBackend.open(&repo).unwrap();
    for target in [
        upstream("team", "alice/main"),
        upstream("team/alice", "main"),
    ] {
        opened
            .set_upstream_branch_with_output("feature", &target)
            .expect("set exact upstream");

        assert_eq!(
            run_git_capture(&repo, &["config", "--get", "branch.feature.remote"]).trim(),
            target.remote
        );
        assert_eq!(
            run_git_capture(&repo, &["config", "--get", "branch.feature.merge"]).trim(),
            format!("refs/heads/{}", target.branch)
        );
        assert_eq!(
            run_git_capture(
                &repo,
                &[
                    "for-each-ref",
                    "--format=%(upstream:short)|%(upstream:track)|%(upstream:remotename)|%(upstream:remoteref)",
                    "refs/heads/feature",
                ],
            )
            .trim(),
            format!(
                "team/alice/main||{}|refs/heads/{}",
                target.remote, target.branch
            )
        );
        let feature = opened
            .list_branches()
            .unwrap()
            .into_iter()
            .find(|branch| branch.name == "feature")
            .expect("feature branch");
        assert_eq!(feature.upstream, Some(target));
    }
}

#[test]
fn checkout_remote_branch_uses_the_fetch_refspec_tracking_destination() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    initialize_repo_with_commit(&repo);

    run_git(&repo, &["config", "remote.origin.url", "."]);
    run_git(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/main:refs/remotes/origin/review",
        ],
    );
    run_git(&repo, &["update-ref", "refs/remotes/origin/review", "HEAD"]);

    let opened = GixBackend.open(&repo).unwrap();
    assert!(
        opened
            .list_remote_branches()
            .unwrap()
            .iter()
            .any(|branch| branch.remote == "origin" && branch.name == "main")
    );

    opened
        .checkout_remote_branch(
            "origin",
            "main",
            "review-local",
            CheckoutRemoteBranchMode::Create,
        )
        .expect("checkout should resolve the mapped local tracking ref");

    assert_eq!(
        run_git_capture(&repo, &["branch", "--show-current"]).trim(),
        "review-local"
    );
    assert_eq!(
        run_git_capture(
            &repo,
            &[
                "for-each-ref",
                "--format=%(upstream:remotename)|%(upstream:remoteref)",
                "refs/heads/review-local",
            ],
        )
        .trim(),
        "origin|refs/heads/main"
    );
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

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_refs_integration_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_refs_integration_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_refs_integration_tests() {
            eprintln!(
                "skipping refs integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

#[test]
fn list_branches_reports_upstream_and_divergence() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    let peer_repo = root.join("peer");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature-1\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature-1"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    fs::write(
        work_repo.join("feature.txt"),
        "feature-1\nfeature-local-ahead\n",
    )
    .unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature-local-ahead",
        ],
    );

    run_git(
        root,
        &[
            "clone",
            origin_url.as_str(),
            peer_repo.to_str().expect("peer path"),
        ],
    );
    run_git(&peer_repo, &["config", "user.email", "you@example.com"]);
    run_git(&peer_repo, &["config", "user.name", "You"]);
    run_git(&peer_repo, &["config", "commit.gpgsign", "false"]);
    run_git(&peer_repo, &["checkout", "feature"]);

    fs::write(peer_repo.join("peer.txt"), "remote-ahead\n").unwrap();
    run_git(&peer_repo, &["add", "peer.txt"]);
    run_git(
        &peer_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature-remote-ahead",
        ],
    );
    run_git(&peer_repo, &["push", "origin", "feature"]);

    run_git(&work_repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");

    assert_eq!(
        feature.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
    assert_eq!(
        feature.divergence,
        Some(UpstreamDivergence {
            ahead: 1,
            behind: 1,
        })
    );
}

#[test]
fn list_branches_gone_upstream_is_exposed_as_untracked() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("base.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "base.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    run_git(&work_repo, &["push", "origin", "--delete", "feature"]);
    run_git(&work_repo, &["fetch", "--prune", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");

    assert_eq!(feature.upstream, None);
    assert_eq!(feature.divergence, None);
}

#[test]
fn list_branches_reflects_new_upstream_without_reopen() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let before = opened.list_branches().unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_before.upstream, None);

    opened.push_set_upstream("origin", "feature").unwrap();

    let after = opened.list_branches().unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(
        feature_after.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn list_branches_reflects_tracking_upstream_set_without_push() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "origin", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let before = opened.list_branches().unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_before.upstream, None);

    let output = opened
        .set_upstream_branch_with_output("feature", &upstream("origin", "feature"))
        .expect("set upstream");
    assert_eq!(output.exit_code, Some(0));

    let upstream_after = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    );
    assert_eq!(upstream_after.trim(), "origin/feature");

    let after = opened.list_branches().unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(
        feature_after.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn set_upstream_can_configure_a_remote_branch_before_its_first_push() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["checkout", "-b", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let target = upstream("origin", "review/feature");
    opened
        .set_upstream_branch_with_output("feature", &target)
        .expect("configure not-yet-pushed upstream");
    opened
        .fetch_all_with_output_prune(true)
        .expect("pending upstream survives a pruning fetch before its first push");

    assert!(
        run_git_capture(
            &work_repo,
            &[
                "ls-remote",
                "--heads",
                "origin",
                "refs/heads/review/feature"
            ],
        )
        .trim()
        .is_empty(),
        "setting upstream must not push or create the remote ref"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.feature.remote"]).trim(),
        "origin"
    );
    assert_eq!(
        run_git_capture(&work_repo, &["config", "--get", "branch.feature.merge"]).trim(),
        "refs/heads/review/feature"
    );
    assert_eq!(
        run_git_capture(
            &work_repo,
            &["config", "--get", "branch.feature.gitcometPendingUpstream"],
        )
        .trim(),
        "true"
    );

    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature.upstream.as_ref(), Some(&target));
    assert_eq!(feature.divergence, None);

    opened
        .push_with_output()
        .expect("later push uses the configured target");
    assert!(
        !run_git_capture(
            &work_repo,
            &[
                "ls-remote",
                "--heads",
                "origin",
                "refs/heads/review/feature"
            ],
        )
        .trim()
        .is_empty(),
        "the next normal push must create the configured remote branch"
    );
    assert!(
        run_git_capture(
            &work_repo,
            &["ls-remote", "--heads", "origin", "refs/heads/feature"],
        )
        .trim()
        .is_empty(),
        "the push must not silently fall back to the local branch name"
    );
    let pending_marker = Command::new("git")
        .arg("-C")
        .arg(&work_repo)
        .args(["config", "--get", "branch.feature.gitcometPendingUpstream"])
        .status()
        .expect("read pending-upstream marker");
    assert!(
        !pending_marker.success(),
        "a successful push must clear the pending-upstream marker"
    );
}

#[test]
fn fetching_a_pending_upstream_that_now_exists_clears_the_pending_marker() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );
    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["checkout", "-b", "feature"]);

    let opened = GixBackend.open(&work_repo).unwrap();
    let target = upstream("origin", "review/feature");
    opened
        .set_upstream_branch_with_output("feature", &target)
        .expect("configure future upstream");

    // Simulate another client creating the configured target. Remove the
    // push-updated local tracking ref so this fetch is what observes it.
    run_git(
        &work_repo,
        &["push", "origin", "HEAD:refs/heads/review/feature"],
    );
    run_git(
        &work_repo,
        &["update-ref", "-d", "refs/remotes/origin/review/feature"],
    );
    opened
        .fetch_all_with_output_prune(true)
        .expect("fetch newly live upstream");

    let marker = Command::new("git")
        .arg("-C")
        .arg(&work_repo)
        .args(["config", "--get", "branch.feature.gitcometPendingUpstream"])
        .status()
        .expect("read pending-upstream marker");
    assert!(
        !marker.success(),
        "observing the mapped tracking ref must clear the pending marker"
    );

    run_git(
        &work_repo,
        &["push", "origin", "--delete", "review/feature"],
    );
    opened
        .fetch_all_with_output_prune(true)
        .expect("prune removed upstream");
    let feature = opened
        .list_branches()
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch");
    assert_eq!(
        feature.upstream, None,
        "once live, a later pruned upstream must be unlinked normally"
    );
}

#[test]
fn list_branches_reflects_repeated_tracking_toggles_on_same_repo_instance() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "origin", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let feature_upstream = |branches: &[gitcomet_core::domain::Branch]| {
        branches
            .iter()
            .find(|branch| branch.name == "feature")
            .expect("feature branch present")
            .upstream
            .clone()
    };

    assert_eq!(feature_upstream(&opened.list_branches().unwrap()), None);

    let output = opened
        .set_upstream_branch_with_output("feature", &upstream("origin", "feature"))
        .expect("set upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(&opened.list_branches().unwrap()),
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );

    let output = opened
        .unset_upstream_branch_with_output("feature")
        .expect("unset upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(feature_upstream(&opened.list_branches().unwrap()), None);

    let output = opened
        .set_upstream_branch_with_output("feature", &upstream("origin", "feature"))
        .expect("restore upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(&opened.list_branches().unwrap()),
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn list_branches_preserves_nested_upstream_branch_names() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    let nested_branch = "feature/nested/name";
    run_git(&work_repo, &["checkout", "-b", nested_branch]);
    fs::write(work_repo.join("nested.txt"), "nested\n").unwrap();
    run_git(&work_repo, &["add", "nested.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "nested feature",
        ],
    );
    run_git(&work_repo, &["push", "-u", "origin", nested_branch]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == nested_branch)
        .expect("nested feature branch present");

    assert_eq!(
        feature.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: nested_branch.to_string(),
        })
    );
    assert_eq!(
        feature.divergence,
        Some(UpstreamDivergence {
            ahead: 0,
            behind: 0,
        })
    );
}

#[test]
fn list_branches_reflects_removed_upstream_without_reopen() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let before = opened.list_branches().unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(
        feature_before.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );

    let output = opened
        .unset_upstream_branch_with_output("feature")
        .expect("unset upstream");
    assert_eq!(output.exit_code, Some(0));

    let upstream_after = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    );
    assert!(
        upstream_after.trim().is_empty(),
        "expected feature to have no upstream after unlink: {upstream_after:?}"
    );

    let after = opened.list_branches().unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_after.upstream, None);
}

#[test]
fn list_ref_metadata_reports_author_date_and_subject_for_local_and_remote_refs() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "Ada Lovelace"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base commit"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add the feature",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let metadata = opened.list_ref_metadata().expect("list ref metadata");

    let lookup = |name: &str| {
        metadata
            .iter()
            .find(|(ref_name, _)| ref_name == name)
            .map(|(_, meta)| meta)
    };

    let main = lookup("main").expect("main present");
    assert_eq!(main.author, "Ada Lovelace");
    assert_eq!(main.summary, "base commit");
    assert!(main.committed_at > 0, "expected a real timestamp");

    let feature = lookup("feature").expect("feature present");
    assert_eq!(feature.summary, "add the feature");

    // Remote-tracking refs are covered by the same call.
    let remote_main = lookup("origin/main").expect("origin/main present");
    assert_eq!(remote_main.summary, "base commit");
}
