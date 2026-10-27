use crate::test_git_env;
use gitcomet_core::domain::Upstream;
use gitcomet_core::error::{ErrorKind, GitFailureId, WorktreeRemovalBlocker};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{GitBackend, WorktreeCheckout};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git_command() -> Command {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut cmd);
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

fn same_path(lhs: &Path, rhs: &Path) -> bool {
    canonicalize_or_original(lhs.to_path_buf()) == canonicalize_or_original(rhs.to_path_buf())
}

#[test]
fn worktree_add_list_remove_round_trip() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("create repo directory");

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    run_git(&repo, &["config", "core.eol", "lf"]);

    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed file");
    run_git(&repo, &["add", "seed.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open repository");

    let before = opened
        .list_worktrees(&gitcomet_core::services::CancellationToken::new())
        .expect("list initial worktrees");
    let primary = before
        .iter()
        .find(|worktree| same_path(&worktree.path, &repo))
        .expect("primary worktree should be listed");
    assert!(primary.head.is_some());
    assert!(!primary.detached);
    if let Some(branch) = primary.branch.as_deref() {
        assert!(
            !branch.starts_with("refs/heads/"),
            "branch name should be normalized: {branch}"
        );
    }

    #[cfg(unix)]
    let linked_path = root.join("linked\ntree");
    #[cfg(not(unix))]
    let linked_path = root.join("linked tree");
    // `HEAD` is not a branch, so git checks the new worktree out detached.
    let add_output = opened
        .add_worktree_with_output(
            &linked_path,
            &WorktreeCheckout::Existing {
                reference: "HEAD".to_string(),
            },
        )
        .expect("add linked worktree");
    assert_eq!(add_output.exit_code, Some(0));

    let listed = opened
        .list_worktrees(&gitcomet_core::services::CancellationToken::new())
        .expect("list worktrees after add");
    let linked = listed
        .iter()
        .find(|worktree| same_path(&worktree.path, &linked_path))
        .expect("linked worktree should be listed");
    assert!(linked.detached);
    assert!(linked.branch.is_none());
    assert!(linked.head.is_some());

    let remove_output = opened
        .remove_worktree_with_output(&linked_path)
        .expect("remove linked worktree");
    assert_eq!(remove_output.exit_code, Some(0));

    let after = opened
        .list_worktrees(&gitcomet_core::services::CancellationToken::new())
        .expect("list worktrees after remove");
    assert!(
        after
            .iter()
            .all(|worktree| !same_path(&worktree.path, &linked_path)),
        "linked worktree should be removed"
    );
}

fn assert_remove_refused(
    opened: &dyn gitcomet_core::services::GitRepository,
    linked: &Path,
    expected: GitFailureId,
) {
    let error = opened
        .remove_worktree_with_output(linked)
        .expect_err("plain remove should be refused");
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected a git failure: {error:?}");
    };
    assert_eq!(failure.id(), expected, "{error}");
    assert!(linked.exists(), "a refused remove must keep the worktree");
}

#[test]
fn worktree_remove_classifies_the_refusals_that_force_overrides() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();
    let sub = root.join("sub");
    fs::create_dir_all(&sub).expect("create submodule directory");
    run_git(&sub, &["init"]);
    run_git(
        &sub,
        &[
            "-c",
            "user.email=you@example.com",
            "-c",
            "user.name=You",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "sub",
        ],
    );
    let repo = cloned_repo(root);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(
        &repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../sub",
            "sub",
        ],
    );
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );
    let opened = GixBackend.open(&repo).expect("open repository");
    let add_detached = |path: &Path| {
        opened
            .add_worktree_with_output(
                path,
                &WorktreeCheckout::Existing {
                    reference: "HEAD".to_string(),
                },
            )
            .expect("add linked worktree");
    };

    let dirty = root.join("dirty");
    add_detached(&dirty);
    fs::write(dirty.join("seed.txt"), "edited\n").expect("edit tracked file");
    assert_remove_refused(
        opened.as_ref(),
        &dirty,
        GitFailureId::WorktreeRemovalBlocked(WorktreeRemovalBlocker::Changes),
    );

    // Git refuses even a clean worktree once its submodule is checked out.
    let with_submodule = root.join("with-submodule");
    add_detached(&with_submodule);
    run_git(
        &with_submodule,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
        ],
    );
    assert_remove_refused(
        opened.as_ref(),
        &with_submodule,
        GitFailureId::WorktreeRemovalBlocked(WorktreeRemovalBlocker::Submodules),
    );

    opened
        .force_remove_worktree_with_output(&with_submodule)
        .expect("force removes a worktree with submodules");
    assert!(!with_submodule.exists());
}

#[cfg(unix)]
#[test]
fn worktree_remove_classifies_refusals_under_a_localized_git() {
    // Git translates both refusals that `--force` overrides.
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "worktree_remove_classifies_refusals_under_a_localized_git",
        r#"#!/bin/sh
for argument in "$@"; do
    if [ "$argument" = remove ]; then
        if [ "${LC_ALL-}" = C ]; then
            printf '%s\n' 'fatal: working trees containing submodules cannot be moved or removed' >&2
        else
            printf '%s\n' 'Schwerwiegend: Arbeitsverzeichnisse, die Submodule enthalten, können nicht verschoben oder entfernt werden.' >&2
        fi
        exit 128
    fi
done
exec git "$@"
"#,
        || {
            let dir = tempfile::tempdir().expect("create tempdir");
            let repo = cloned_repo(dir.path());
            let opened = GixBackend.open(&repo).expect("open repository");
            let linked = dir.path().join("linked");
            fs::create_dir_all(&linked).expect("create worktree directory");
            assert_remove_refused(
                opened.as_ref(),
                &linked,
                GitFailureId::WorktreeRemovalBlocked(WorktreeRemovalBlocker::Submodules),
            );
        },
    );
}

fn git_stdout(repo: &Path, args: &[&str]) -> Option<String> {
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `origin` holds `main` (its HEAD) and `feature`; `repo` is a clone of it.
fn cloned_repo(root: &Path) -> std::path::PathBuf {
    let origin = root.join("origin");
    fs::create_dir_all(&origin).expect("create origin directory");
    run_git(&origin, &["init", "-b", "main"]);
    run_git(&origin, &["config", "user.email", "you@example.com"]);
    run_git(&origin, &["config", "user.name", "You"]);
    fs::write(origin.join("seed.txt"), "seed\n").expect("write seed file");
    run_git(&origin, &["add", "seed.txt"]);
    run_git(
        &origin,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );
    run_git(&origin, &["tag", "v1"]);
    run_git(&origin, &["branch", "feature"]);
    run_git(root, &["clone", "origin", "repo"]);
    root.join("repo")
}

fn track(remote: &str, branch: &str) -> WorktreeCheckout {
    WorktreeCheckout::TrackRemote {
        name: branch.to_string(),
        upstream: Upstream {
            remote: remote.to_string(),
            branch: branch.to_string(),
        },
    }
}

#[test]
fn worktree_add_tracks_the_remote_branch_even_without_auto_setup_merge() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    run_git(&repo, &["config", "branch.autoSetupMerge", "false"]);
    let opened = GixBackend.open(&repo).expect("open repository");
    let linked = dir.path().join("feature");

    opened
        .add_worktree_with_output(&linked, &track("origin", "feature"))
        .expect("add tracking worktree");

    assert_eq!(
        git_stdout(&linked, &["rev-parse", "--abbrev-ref", "HEAD"]).as_deref(),
        Some("feature")
    );
    assert_eq!(
        git_stdout(&linked, &["rev-parse", "--abbrev-ref", "@{u}"]).as_deref(),
        Some("origin/feature")
    );
}

#[test]
fn worktree_add_new_branch_from_a_remote_branch_sets_no_upstream() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    let opened = GixBackend.open(&repo).expect("open repository");
    let linked = dir.path().join("topic");

    opened
        .add_worktree_with_output(
            &linked,
            &WorktreeCheckout::NewBranch {
                name: "topic".to_string(),
                start: Some("origin/main".to_string()),
            },
        )
        .expect("add new-branch worktree");

    assert_eq!(
        git_stdout(&linked, &["rev-parse", "--abbrev-ref", "HEAD"]).as_deref(),
        Some("topic")
    );
    assert_eq!(
        git_stdout(&linked, &["rev-parse", "--abbrev-ref", "@{u}"]),
        None,
        "git's default autoSetupMerge would make origin/main the upstream"
    );
}

#[test]
fn worktree_add_new_branch_starts_from_a_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    let opened = GixBackend.open(&repo).expect("open repository");
    let linked = dir.path().join("from-tag");

    opened
        .add_worktree_with_output(
            &linked,
            &WorktreeCheckout::NewBranch {
                name: "from-tag".to_string(),
                start: Some("v1".to_string()),
            },
        )
        .expect("add worktree from a tag");

    assert_eq!(
        git_stdout(&linked, &["rev-parse", "HEAD"]),
        git_stdout(&repo, &["rev-parse", "v1^{commit}"])
    );
}

#[test]
fn worktree_add_refuses_an_option_like_branch_name() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    let opened = GixBackend.open(&repo).expect("open repository");
    let linked = dir.path().join("refused");

    let err = opened
        .add_worktree_with_output(
            &linked,
            &WorktreeCheckout::NewBranch {
                name: "--detach".to_string(),
                start: None,
            },
        )
        .expect_err("an option-looking branch name must be refused");

    assert!(err.to_string().contains("worktree branch"), "{err}");
    assert!(!linked.exists(), "git must not have run");
}

#[test]
fn worktree_add_reports_a_pruned_remote_branch() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    let opened = GixBackend.open(&repo).expect("open repository");
    let linked = dir.path().join("gone");

    let err = opened
        .add_worktree_with_output(&linked, &track("origin", "gone"))
        .expect_err("a missing remote branch must fail");

    assert!(
        err.to_string().contains("origin/gone no longer exists"),
        "{err}"
    );
    assert!(!linked.exists(), "git must not have run");
}

#[test]
fn list_remotes_reads_the_remote_default_branch() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    run_git(&repo, &["remote", "add", "bare", "../origin"]);
    let opened = GixBackend.open(&repo).expect("open repository");

    let remotes = opened
        .list_remotes(&gitcomet_core::services::CancellationToken::new())
        .expect("list remotes");
    let default_of = |name: &str| {
        remotes
            .iter()
            .find(|remote| remote.name == name)
            .map(|remote| remote.default_branch.clone())
    };

    assert_eq!(default_of("origin"), Some(Some("main".to_string())));
    assert_eq!(
        default_of("bare"),
        Some(None),
        "a remote never fetched has no HEAD"
    );
}

#[test]
fn list_remotes_survives_a_remote_whose_head_cannot_be_read() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = cloned_repo(dir.path());
    // Git refuses this name, but a hand-edited config can still hold it, and
    // `refs/remotes/my remote/HEAD` is no valid ref name to look up.
    run_git(&repo, &["config", "remote.my remote.url", "../origin"]);
    let opened = GixBackend.open(&repo).expect("open repository");

    let remotes = opened
        .list_remotes(&gitcomet_core::services::CancellationToken::new())
        .expect("one remote's HEAD must not fail the whole list");

    assert_eq!(
        remotes
            .iter()
            .find(|remote| remote.name == "origin")
            .and_then(|remote| remote.default_branch.as_deref()),
        Some("main")
    );
}
