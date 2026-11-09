use crate::git_cli::{git_cmd, git_output, git_stdout, git_stdout_raw, run_git};
use gitcomet_core::domain::{HistoryMode, Upstream, UpstreamDivergence};
use gitcomet_core::services::{CancellationToken, CheckoutRemoteBranchMode, GitBackend};
use gitcomet_core::test_support::git_fixture::{
    LinearCommit, append_config, import_linear_history,
};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;

fn initialize_repo_with_commit(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    run_git(repo, &["init", "-b", "main"]);
    append_config(
        repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
    let mut cmd = git_cmd(repo);
    import_linear_history(
        &mut cmd,
        "main",
        [LinearCommit {
            author: "You <you@example.com>",
            timestamp: 1_600_000_000,
            message: "base",
            path: "file.txt",
            contents: "base\n",
        }],
    );
    run_git(repo, &["reset", "--hard", "main"]);
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
    let branches = opened
        .list_remote_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    assert_eq!(
        branches
            .iter()
            .map(|branch| (branch.remote.as_str(), branch.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("team", "alice/main"), ("team/alice", "main")]
    );
}

// `fetch --prune` leaves `origin/HEAD` dangling when it deletes the branch the
// symref names. Git's for-each-ref skips such refs; so must every listing.
#[test]
fn dangling_symrefs_are_left_out_of_every_listing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    initialize_repo_with_commit(&repo);
    run_git(&repo, &["config", "remote.origin.url", "."]);
    run_git(
        &repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    run_git(&repo, &["update-ref", "refs/remotes/origin/master", "HEAD"]);
    for (name, target) in [
        ("refs/remotes/origin/HEAD", "refs/remotes/origin/main"),
        ("refs/heads/dangling", "refs/heads/missing"),
        ("refs/tags/dangling", "refs/tags/missing"),
    ] {
        run_git(&repo, &["symbolic-ref", name, target]);
    }

    let opened = GixBackend.open(&repo).unwrap();
    let cancellation = CancellationToken::new();
    let remote_branches = opened
        .list_remote_branches(&cancellation)
        .expect("list remote branches");
    assert_eq!(
        remote_branches
            .iter()
            .map(|branch| (branch.remote.as_str(), branch.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("origin", "master")]
    );
    // No `[branch]` config: the gix listing runs, not `git for-each-ref`.
    let branches = opened
        .list_branches(&cancellation)
        .expect("list local branches");
    assert_eq!(
        branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        vec!["main"]
    );
    assert!(
        opened
            .list_tags(&cancellation)
            .expect("list tags")
            .is_empty()
    );
    assert_eq!(
        opened
            .log_all_branches_page(10, None, &cancellation)
            .expect("all-branches history")
            .commits
            .len(),
        1
    );
    assert_eq!(
        opened
            .build_history_index(
                &HistoryMode::AllBranches.into(),
                None,
                &Default::default(),
                &cancellation,
                &mut |_| {}
            )
            .expect("all-branches history index")
            .expect("indexed history")
            .len(),
        1
    );

    // Only a missing ref is skipped: a ref naming a missing object still fails.
    let missing_object = "1".repeat(40);
    fs::write(
        repo.join(".git/refs/tags/broken"),
        format!("{missing_object}\n"),
    )
    .unwrap();
    assert!(
        GixBackend
            .open(&repo)
            .unwrap()
            .list_tags(&cancellation)
            .is_err()
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
            git_stdout(&repo, &["config", "--get", "branch.feature.remote"]),
            target.remote
        );
        assert_eq!(
            git_stdout(&repo, &["config", "--get", "branch.feature.merge"]),
            format!("refs/heads/{}", target.branch)
        );
        assert_eq!(
            git_stdout(
                &repo,
                &[
                    "for-each-ref",
                    "--format=%(upstream:short)|%(upstream:track)|%(upstream:remotename)|%(upstream:remoteref)",
                    "refs/heads/feature",
                ],
            ),
            format!(
                "team/alice/main||{}|refs/heads/{}",
                target.remote, target.branch
            )
        );
        let feature = opened
            .list_branches(&gitcomet_core::services::CancellationToken::new())
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
            .list_remote_branches(&gitcomet_core::services::CancellationToken::new())
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
        git_stdout(&repo, &["branch", "--show-current"]),
        "review-local"
    );
    assert_eq!(
        git_stdout(
            &repo,
            &[
                "for-each-ref",
                "--format=%(upstream:remotename)|%(upstream:remoteref)",
                "refs/heads/review-local",
            ],
        ),
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

#[test]
fn list_branches_reports_upstream_and_divergence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    let peer_repo = root.join("peer");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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
    let branches = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
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
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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
    let branches = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");

    assert_eq!(feature.upstream, None);
    assert_eq!(feature.divergence, None);
}

#[test]
fn list_branches_reflects_new_upstream_without_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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

    let before = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_before.upstream, None);

    opened
        .push_set_upstream_with_output("origin", "feature")
        .unwrap();

    let after = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
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

/// `git branch --set-upstream-to` honours `branch.autoSetupRebase`, so
/// setting an upstream from GitComet must too.
#[test]
fn set_upstream_honours_auto_setup_rebase() {
    let dir = tempfile::tempdir().unwrap();
    let remote_repo = dir.path().join("remote.git");
    let work_repo = dir.path().join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();
    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
    let origin_url = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &origin_url]);
    run_git(&work_repo, &["commit", "--allow-empty", "-m", "base"]);
    run_git(&work_repo, &["push", "origin", "main"]);
    let opened = GixBackend.open(&work_repo).unwrap();

    // Like Git, a setting that asks for no rebase leaves the old value.
    for (setting, rebase) in [
        ("always", "true"),
        ("remote", "true"),
        ("local", "false"),
        ("never", "false"),
    ] {
        run_git(&work_repo, &["config", "branch.autoSetupRebase", setting]);
        run_git(&work_repo, &["config", "branch.main.rebase", "false"]);
        opened
            .set_upstream_branch_with_output("main", &upstream("origin", "main"))
            .expect("set upstream");

        let actual = git_stdout_raw(&work_repo, &["config", "branch.main.rebase"]);
        assert_eq!(actual.trim(), rebase, "{setting}");
    }
}

#[test]
fn list_branches_reflects_tracking_upstream_set_without_push() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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

    let before = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_before.upstream, None);

    let output = opened
        .set_upstream_branch_with_output("feature", &upstream("origin", "feature"))
        .expect("set upstream");
    assert_eq!(output.exit_code, Some(0));

    let upstream_after = git_stdout_raw(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    );
    assert_eq!(upstream_after.trim(), "origin/feature");

    let after = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
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
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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
        .fetch_all(true)
        .expect("pending upstream survives a pruning fetch before its first push");

    assert!(
        git_stdout(
            &work_repo,
            &[
                "ls-remote",
                "--heads",
                "origin",
                "refs/heads/review/feature"
            ],
        )
        .is_empty(),
        "setting upstream must not push or create the remote ref"
    );
    assert_eq!(
        git_stdout(&work_repo, &["config", "--get", "branch.feature.remote"]),
        "origin"
    );
    assert_eq!(
        git_stdout(&work_repo, &["config", "--get", "branch.feature.merge"]),
        "refs/heads/review/feature"
    );
    assert_eq!(
        git_stdout(
            &work_repo,
            &["config", "--get", "branch.feature.gitcometPendingUpstream"],
        ),
        "true"
    );

    let branches = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
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
        !git_stdout(
            &work_repo,
            &[
                "ls-remote",
                "--heads",
                "origin",
                "refs/heads/review/feature"
            ],
        )
        .is_empty(),
        "the next normal push must create the configured remote branch"
    );
    assert!(
        git_stdout(
            &work_repo,
            &["ls-remote", "--heads", "origin", "refs/heads/feature"],
        )
        .is_empty(),
        "the push must not silently fall back to the local branch name"
    );
    let pending_marker = git_output(
        &work_repo,
        &["config", "--get", "branch.feature.gitcometPendingUpstream"],
    );
    assert!(
        !pending_marker.status.success(),
        "a successful push must clear the pending-upstream marker"
    );
}

#[test]
fn fetching_a_pending_upstream_that_now_exists_clears_the_pending_marker() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);
    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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
    opened.fetch_all(true).expect("fetch newly live upstream");

    let marker = git_output(
        &work_repo,
        &["config", "--get", "branch.feature.gitcometPendingUpstream"],
    );
    assert!(
        !marker.status.success(),
        "observing the mapped tracking ref must clear the pending marker"
    );

    run_git(
        &work_repo,
        &["push", "origin", "--delete", "review/feature"],
    );
    opened.fetch_all(true).expect("prune removed upstream");
    let feature = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
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
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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

    assert_eq!(
        feature_upstream(
            &opened
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
        ),
        None
    );

    let output = opened
        .set_upstream_branch_with_output("feature", &upstream("origin", "feature"))
        .expect("set upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(
            &opened
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
        ),
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );

    let output = opened
        .unset_upstream_branch_with_output("feature")
        .expect("unset upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(
            &opened
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
        ),
        None
    );

    let output = opened
        .set_upstream_branch_with_output("feature", &upstream("origin", "feature"))
        .expect("restore upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(
            &opened
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
        ),
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn list_branches_preserves_nested_upstream_branch_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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
    let branches = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
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
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    append_config(
        &work_repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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

    let before = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
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

    let upstream_after = git_stdout_raw(
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

    let after = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_after.upstream, None);
}

#[test]
fn list_ref_metadata_reports_author_date_and_subject_for_local_and_remote_refs() {
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
    let metadata = opened
        .list_ref_metadata(&gitcomet_core::services::CancellationToken::new())
        .expect("list ref metadata");

    let lookup = |name: &str| {
        metadata
            .iter()
            .find(|(ref_name, _)| ref_name.as_str() == name)
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

#[test]
fn list_branches_uses_first_merge_entry() {
    for separate_sections in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        initialize_repo_with_commit(repo);
        run_git(repo, &["remote", "add", "origin", "."]);
        run_git(repo, &["update-ref", "refs/remotes/origin/old", "HEAD"]);
        run_git(repo, &["commit", "--allow-empty", "-m", "ahead"]);
        run_git(repo, &["update-ref", "refs/remotes/origin/new", "HEAD"]);
        run_git(repo, &["config", "branch.main.remote", "origin"]);
        run_git(
            repo,
            &["config", "--add", "branch.main.merge", "refs/heads/old"],
        );
        if separate_sections {
            use std::io::Write;
            let mut config = fs::OpenOptions::new()
                .append(true)
                .open(repo.join(".git/config"))
                .unwrap();
            writeln!(config, "\n[branch \"main\"]\n merge = refs/heads/new").unwrap();
        } else {
            run_git(
                repo,
                &["config", "--add", "branch.main.merge", "refs/heads/new"],
            );
        }
        assert_eq!(
            git_stdout(
                repo,
                &[
                    "for-each-ref",
                    "--format=%(upstream:short) %(upstream:track)",
                    "refs/heads/main"
                ]
            ),
            "origin/old [ahead 1]"
        );
        let opened = GixBackend.open(repo).unwrap();
        let branches = opened
            .list_branches(&gitcomet_core::services::CancellationToken::new())
            .unwrap();
        let main = branches
            .iter()
            .find(|branch| branch.name == "main")
            .unwrap();
        assert_eq!(
            main.upstream,
            Some(upstream("origin", "old")),
            "separate sections: {separate_sections}"
        );
        assert_eq!(
            main.divergence,
            Some(UpstreamDivergence {
                ahead: 1,
                behind: 0
            })
        );
    }
}

#[test]
fn changing_upstream_replaces_repeated_merge_entries() {
    for separate_sections in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        initialize_repo_with_commit(repo);
        run_git(repo, &["remote", "add", "origin", "."]);
        for name in ["old", "other", "new"] {
            run_git(
                repo,
                &["update-ref", &format!("refs/remotes/origin/{name}"), "HEAD"],
            );
        }
        run_git(repo, &["config", "branch.main.remote", "origin"]);
        run_git(
            repo,
            &["config", "--add", "branch.main.merge", "refs/heads/old"],
        );
        if separate_sections {
            use std::io::Write;
            let mut config = fs::OpenOptions::new()
                .append(true)
                .open(repo.join(".git/config"))
                .unwrap();
            writeln!(
                config,
                "\n[branch \"main\"]\n merge = refs/heads/other\n description = keep-me"
            )
            .unwrap();
        } else {
            run_git(
                repo,
                &["config", "--add", "branch.main.merge", "refs/heads/other"],
            );
            run_git(repo, &["config", "branch.main.description", "keep-me"]);
        }
        let opened = GixBackend.open(repo).unwrap();
        opened
            .set_upstream_branch_with_output("main", &upstream("origin", "new"))
            .unwrap();
        assert_eq!(
            git_stdout(repo, &["rev-parse", "--abbrev-ref", "main@{upstream}"]),
            "origin/new",
            "separate sections: {separate_sections}"
        );
        assert_eq!(
            git_stdout(repo, &["config", "--get-all", "branch.main.merge"]),
            "refs/heads/new"
        );
        assert_eq!(
            git_stdout(repo, &["config", "branch.main.description"]),
            "keep-me"
        );
        let branches = opened
            .list_branches(&gitcomet_core::services::CancellationToken::new())
            .unwrap();
        assert_eq!(
            branches.iter().find(|b| b.name == "main").unwrap().upstream,
            Some(upstream("origin", "new"))
        );
    }
}

// Timing assertions are opt-in so shared CI load cannot make the suite flaky.
#[test]
#[ignore = "manual branch-listing scaling regression check"]
fn tracked_branch_listing_scales_with_branch_count() {
    use std::io::Write;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut durations = Vec::new();
    for count in [200, 1000] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        initialize_repo_with_commit(repo);
        run_git(repo, &["remote", "add", "origin", "."]);
        run_git(repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let oid = git_stdout_raw(repo, &["rev-parse", "HEAD"]);
        let mut refs = String::new();
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(repo.join(".git/config"))
            .unwrap();
        for index in 0..count {
            refs.push_str(&format!(
                "create refs/heads/branch-{index} {}\n",
                oid.trim()
            ));
            writeln!(config, "\n[branch \"branch-{index}\"]\n remote = origin\n merge = refs/heads/main\n gitcometPendingUpstream = false").unwrap();
        }
        drop(config);
        let mut child = git_cmd(repo)
            .args(["update-ref", "--stdin"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(refs.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        let opened = GixBackend.open(repo).unwrap();
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let start = Instant::now();
            let branches = opened
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            best = best.min(start.elapsed());
            assert_eq!(branches.len(), count + 1);
            assert_eq!(
                branches
                    .iter()
                    .filter(|b| b.upstream == Some(upstream("origin", "main")))
                    .count(),
                count
            );
        }
        eprintln!("{count} tracked branches: {best:?}");
        durations.push(best);
    }
    assert!(
        durations[1] < durations[0] * 12,
        "5x branches should not take 12x as long: {durations:?}"
    );
}

/// A refspec fetch brings exactly the named refs, including ones outside the
/// remote's configured refspecs (the way pull-request heads are fetched).
#[test]
fn fetch_refspecs_fetches_exactly_the_named_refs() {
    let dir = tempfile::tempdir().unwrap();
    let upstream_repo = dir.path().join("upstream");
    initialize_repo_with_commit(&upstream_repo);
    run_git(&upstream_repo, &["update-ref", "refs/pull/7/head", "HEAD"]);
    run_git(&upstream_repo, &["branch", "unrelated"]);

    let repo = dir.path().join("repo");
    initialize_repo_with_commit(&repo);
    run_git(
        &repo,
        &["remote", "add", "origin", &git_remote_url(&upstream_repo)],
    );

    let opened = GixBackend.open(&repo).unwrap();
    let output = opened
        .fetch_refspecs_with_output(
            "origin",
            &["+refs/pull/7/head:refs/remotes/origin/pr/7".to_string()],
        )
        .expect("refspec fetch");
    assert!(output.command.contains("refs/pull/7/head"), "{output:?}");

    let refs = git_stdout_raw(&repo, &["for-each-ref", "--format=%(refname)"]);
    assert!(refs.contains("refs/remotes/origin/pr/7"), "{refs}");
    assert!(
        !refs.contains("refs/remotes/origin/unrelated"),
        "only the named refspec is fetched: {refs}"
    );

    let error = opened
        .fetch_refspecs_with_output("origin", &["--upload-pack=evil".to_string()])
        .expect_err("an option-shaped refspec is refused");
    assert!(error.to_string().contains("refspec"), "{error}");
    assert!(opened.fetch_refspecs_with_output("origin", &[]).is_err());
}

/// Backend cost of the listings a git-state refresh (commit, checkout, fetch,
/// focus) reloads, on real repositories: `GITCOMET_PROBE_REPOS=/a:/b`.
/// Each call is repeated with nothing changed, as a watcher refresh is.
#[test]
#[ignore = "timing probe"]
fn timing_refresh_listings_real_repos() {
    use std::time::{Duration, Instant};
    let Ok(repos) = std::env::var("GITCOMET_PROBE_REPOS") else {
        eprintln!("GITCOMET_PROBE_REPOS not set");
        return;
    };
    fn best<T>(mut f: impl FnMut() -> T) -> (Duration, T) {
        let mut best = Duration::MAX;
        let mut out = f();
        for _ in 0..5 {
            let start = Instant::now();
            out = f();
            best = best.min(start.elapsed());
        }
        (best, out)
    }
    for path in repos.split(':') {
        let opened = GixBackend.open(Path::new(path)).unwrap();
        let name = Path::new(path).file_name().unwrap().to_string_lossy();
        let (t, v) = best(|| {
            opened
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .len()
        });
        println!(
            "timing list_branches {name} n={v} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, v) = best(|| {
            opened
                .list_remote_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .len()
        });
        println!(
            "timing list_remote_branches {name} n={v} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, v) = best(|| {
            opened
                .list_tags(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .len()
        });
        println!(
            "timing list_tags {name} n={v} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, v) = best(|| {
            opened
                .list_remotes(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .len()
        });
        println!(
            "timing list_remotes {name} n={v} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, _) = best(|| {
            opened
                .upstream_divergence(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
        });
        println!(
            "timing upstream_divergence {name} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, _) = best(|| {
            opened
                .current_branch(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
        });
        println!(
            "timing current_branch {name} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, v) = best(|| {
            opened
                .stash_list(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .len()
        });
        println!(
            "timing stash_list {name} n={v} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
        let (t, v) = best(|| {
            let s = opened
                .status(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            s.staged.len() + s.unstaged.len()
        });
        println!("timing status {name} n={v} {:.3}ms", t.as_secs_f64() * 1e3);
        // What a refresh runs after its status load: the counts for that status.
        let status = opened
            .status(&gitcomet_core::services::CancellationToken::new())
            .unwrap();
        let cancellation = gitcomet_core::services::CancellationToken::new();
        let (t, v) = best(|| {
            let stats = opened
                .uncommitted_line_stats_for_status(&status, &cancellation)
                .unwrap();
            stats.staged.len() + stats.unstaged.len()
        });
        println!(
            "timing line_stats {name} n={v} {:.3}ms",
            t.as_secs_f64() * 1e3
        );
    }
}
