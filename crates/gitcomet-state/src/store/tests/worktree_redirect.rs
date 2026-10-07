//! End-to-end: a branch held by another worktree is opened there instead of
//! failing here, through the real gix backend and a real linked worktree.
use super::*;

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Main worktree on `main` (two commits), linked worktree on `feature` (first commit).
pub(in crate::store) fn repo_with_linked_worktree() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "user.name", "Test User"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    fs::write(repo.join("a.txt"), "one\n").expect("write a.txt");
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["commit", "-q", "-m", "init"]);
    run_git(&repo, &["branch", "feature"]);
    fs::write(repo.join("b.txt"), "two\n").expect("write b.txt");
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["commit", "-q", "-m", "second"]);
    let worktree = dir.path().join("feature-worktree");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().expect("utf-8 path"),
            "feature",
        ],
    );
    (dir, repo, canonicalize_or_original(worktree))
}

pub(super) fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn open_repo_and_wait(store: &AppStore, path: &Path) -> RepoId {
    let expected = canonicalize_or_original(path.to_path_buf());
    store.dispatch(Msg::OpenRepo(path.to_path_buf()));
    wait_until("repository to open", || {
        let snapshot = store.snapshot();
        snapshot
            .active_repo
            .and_then(|id| snapshot.repos.iter().find(|repo| repo.id == id))
            .is_some_and(|repo| {
                repo.spec.workdir == expected && matches!(repo.open, Loadable::Ready(()))
            })
    });
    store.snapshot().active_repo.expect("active repo")
}

fn wait_for_active_tab(store: &AppStore, workdir: &Path) {
    wait_until("worktree tab to open and activate", || {
        let snapshot = store.snapshot();
        snapshot
            .active_repo
            .and_then(|id| snapshot.repos.iter().find(|repo| repo.id == id))
            .is_some_and(|repo| repo.spec.workdir == workdir)
    });
}

fn assert_no_error_diagnostics(store: &AppStore, repo_id: RepoId) {
    let snapshot = store.snapshot();
    let repo = snapshot
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("origin repo state");
    let errors: Vec<_> = repo
        .feedback
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == DiagnosticKind::Error)
        .collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
}

#[test]
fn checkout_of_branch_held_by_linked_worktree_opens_that_worktree_tab() {
    let (_dir, repo, worktree) = repo_with_linked_worktree();
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let repo_id = open_repo_and_wait(&store, &repo);

    store.dispatch(Msg::CheckoutBranch {
        repo_id,
        name: "feature".to_string(),
    });

    wait_for_active_tab(&store, &worktree);
    wait_until("origin action to finish", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| repo.local_actions_in_flight == 0)
    });
    assert_no_error_diagnostics(&store, repo_id);
    assert_eq!(
        git_output(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main",
        "nothing is checked out in the origin worktree"
    );
}

#[test]
fn overwrite_of_branch_held_by_linked_worktree_resets_it_there_and_opens_the_tab() {
    let (_dir, repo, worktree) = repo_with_linked_worktree();
    let main_commit = git_output(&repo, &["rev-parse", "main"]);
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let repo_id = open_repo_and_wait(&store, &repo);

    store.dispatch(Msg::CreateBranchAndCheckout {
        repo_id,
        name: "feature".to_string(),
        target: "main".to_string(),
        force: true,
    });

    wait_for_active_tab(&store, &worktree);
    wait_until("origin action to finish", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| repo.local_actions_in_flight == 0)
    });
    assert_no_error_diagnostics(&store, repo_id);
    assert_eq!(
        git_output(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
    assert_eq!(
        git_output(&worktree, &["rev-parse", "HEAD"]),
        main_commit,
        "the branch was reset inside the worktree that holds it"
    );
    assert_eq!(
        git_output(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
}

#[test]
fn rename_onto_branch_held_by_linked_worktree_overwrites_there_and_opens_the_tab() {
    let (_dir, repo, worktree) = repo_with_linked_worktree();
    run_git(&repo, &["branch", "renamed-source", "main"]);
    let main_commit = git_output(&repo, &["rev-parse", "main"]);
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let repo_id = open_repo_and_wait(&store, &repo);

    store.dispatch(Msg::RenameBranch {
        repo_id,
        old_name: "renamed-source".to_string(),
        new_name: "feature".to_string(),
        force: true,
    });

    wait_for_active_tab(&store, &worktree);
    wait_until("origin action to finish", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| repo.local_actions_in_flight == 0)
    });
    assert_no_error_diagnostics(&store, repo_id);
    assert_eq!(git_output(&worktree, &["rev-parse", "HEAD"]), main_commit);
    assert_eq!(
        git_output(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
    assert!(
        Command::new("git")
            .args([
                "-C",
                repo.to_str().expect("utf-8"),
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/renamed-source"
            ])
            .status()
            .expect("show-ref")
            .code()
            != Some(0),
        "the old name is gone"
    );
    assert_eq!(
        git_output(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main",
        "the origin worktree keeps its branch"
    );
}

fn git_config_is_unset(repo: &Path, key: &str) -> bool {
    !Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", key])
        .status()
        .expect("git runs")
        .success()
}

#[test]
fn overwrite_from_remote_branch_held_by_linked_worktree_clears_its_upstream() {
    let (dir, repo, worktree) = repo_with_linked_worktree();
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&origin).expect("origin dir");
    run_git(&origin, &["init", "-q", "--bare"]);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("utf-8 path"),
        ],
    );
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/feature", "main"],
    );
    run_git(&repo, &["config", "branch.feature.remote", "origin"]);
    run_git(
        &repo,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    );
    let main_commit = git_output(&repo, &["rev-parse", "main"]);
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let repo_id = open_repo_and_wait(&store, &repo);

    store.dispatch(Msg::CreateBranchAndCheckout {
        repo_id,
        name: "feature".to_string(),
        target: "origin/feature".to_string(),
        force: true,
    });

    wait_for_active_tab(&store, &worktree);
    wait_until("origin action to finish", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| repo.local_actions_in_flight == 0)
    });
    assert_no_error_diagnostics(&store, repo_id);
    assert_eq!(git_output(&worktree, &["rev-parse", "HEAD"]), main_commit);
    assert!(
        git_config_is_unset(&worktree, "branch.feature.remote")
            && git_config_is_unset(&worktree, "branch.feature.merge"),
        "an overwritten branch tracks nothing, even when reset in another worktree"
    );
}

/// A list and a session on a linked worktree read that checkout through the
/// real backend: its edit is listed and diffed while History stays as it was.
#[test]
fn a_linked_worktrees_changes_and_diff_load_from_its_own_checkout() {
    use crate::diff_session::{ChangeSource, DiffSessionMsg, DiffViewId};
    let (_dir, repo, worktree) = repo_with_linked_worktree();
    fs::write(worktree.join("a.txt"), "one\nlinked edit\n").expect("edit the worktree");
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let repo_id = open_repo_and_wait(&store, &repo);
    let state = |store: &AppStore| {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .cloned()
            .expect("repository")
    };
    let before = state(&store);
    let lifetime = before.lifetime();
    let history = (
        before.diff_state.diff_target_rev,
        before.navigation.main_history.entries.len(),
    );

    let (list, pane) = (DiffViewId::next(), DiffViewId::next());
    let source = ChangeSource::linked_worktree(
        worktree.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
        true,
    );
    store.dispatch(Msg::DiffSession(DiffSessionMsg::OpenChanges {
        repo_id,
        lifetime,
        view: list,
        source: source.clone(),
    }));
    wait_until("the linked worktree's changes", || {
        state(&store)
            .change_lists
            .get(&list)
            .is_some_and(|list| !list.is_loading() && !matches!(list.files, Loadable::NotLoaded))
    });
    let files = match &state(&store).change_lists[&list].files {
        Loadable::Ready(files) => files.as_ref().clone(),
        other => panic!("changes did not load: {other:?}"),
    };
    let paths: Vec<_> = files.iter().map(|file| file.path.clone()).collect();
    assert_eq!(
        paths,
        vec![PathBuf::from("a.txt")],
        "the main checkout is clean"
    );

    store.dispatch(Msg::DiffSession(DiffSessionMsg::Open {
        repo_id,
        lifetime,
        view: pane,
        target: source.target_for(&files[0], None),
    }));
    wait_until("the linked file's diff", || {
        state(&store)
            .diff_sessions
            .get(&pane)
            .is_some_and(|session| !matches!(session.diff, Loadable::Loading | Loadable::NotLoaded))
    });
    let after = state(&store);
    let Loadable::Ready(diff) = &after.diff_sessions[&pane].diff else {
        panic!(
            "the diff did not load: {:?}",
            after.diff_sessions[&pane].diff
        );
    };
    assert!(
        diff.lines
            .iter()
            .any(|line| line.text.as_ref().contains("linked edit")),
        "the diff reads the linked checkout"
    );
    assert_eq!(after.diff_state.diff_target, None);
    assert_eq!(
        (
            after.diff_state.diff_target_rev,
            after.navigation.main_history.entries.len(),
        ),
        history
    );
}

#[test]
fn rapid_linked_worktree_switches_and_reopening_finish_with_the_correct_checkout() {
    let (_dir, main, linked) = repo_with_linked_worktree();
    fs::write(main.join("b.txt"), "main staged edit\n").unwrap();
    run_git(&main, &["add", "b.txt"]);
    fs::write(linked.join("a.txt"), "linked unstaged edit\n").unwrap();
    fs::write(linked.join("linked-only.txt"), "linked untracked\n").unwrap();
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    let main_id = open_repo_and_wait(&store, &main);
    let linked_id = open_repo_and_wait(&store, &linked);
    let settled = |id| {
        let snapshot = store.snapshot();
        snapshot.active_repo == Some(id)
            && snapshot
                .repos
                .iter()
                .find(|repo| repo.id == id)
                .is_some_and(|repo| {
                    !repo
                        .loads_in_flight
                        .is_in_flight(!crate::model::RepoLoadsInFlight::WORKTREE_DIRTY)
                        && matches!(repo.status, Loadable::Ready(_))
                        && matches!(repo.head_branch, Loadable::Ready(_))
                })
    };
    wait_until("initial linked foreground loads", || settled(linked_id));
    for id in [main_id, linked_id, main_id, linked_id] {
        store.dispatch(Msg::SetActiveRepo { repo_id: id });
        wait_until("settled linked checkout switch", || settled(id));
        let snapshot = store.snapshot();
        let repo = snapshot.repos.iter().find(|repo| repo.id == id).unwrap();
        let status = repo.status.ready().unwrap();
        if id == main_id {
            assert_eq!(repo.head_branch.ready().unwrap(), "main");
            assert_eq!(status.staged.len(), 1);
            assert!(status.unstaged.is_empty());
        } else {
            assert_eq!(repo.head_branch.ready().unwrap(), "feature");
            assert!(status.staged.is_empty());
            assert_eq!(status.unstaged.len(), 2);
        }
    }
    for _ in 0..20 {
        store.dispatch(Msg::SetActiveRepo { repo_id: main_id });
        store.dispatch(Msg::SetActiveRepo { repo_id: linked_id });
    }
    store.dispatch(Msg::CloseRepo { repo_id: main_id });
    wait_until("old checkout to close", || {
        store.snapshot().repos.iter().all(|repo| repo.id != main_id)
    });
    let reopened = open_repo_and_wait(&store, &main);
    assert_ne!(reopened, main_id);
    wait_until("reopened foreground and background loads", || {
        settled(reopened)
            && store
                .snapshot()
                .repos
                .iter()
                .find(|repo| repo.id == reopened)
                .is_some_and(|repo| !repo.loads_in_flight.any_in_flight())
    });
    let snapshot = store.snapshot();
    assert!(snapshot.repos.iter().all(|repo| repo.id != main_id));
    let repo = snapshot
        .repos
        .iter()
        .find(|repo| repo.id == reopened)
        .unwrap();
    assert_eq!(repo.head_branch.ready().unwrap(), "main");
    assert_eq!(repo.status.ready().unwrap().staged.len(), 1);
    assert!(repo.status.ready().unwrap().unstaged.is_empty());
    assert!(
        repo.worktree_dirty
            .ready()
            .unwrap()
            .iter()
            .any(|summary| summary.path == linked && summary.modified == 1 && summary.added == 1)
    );
    assert_no_error_diagnostics(&store, reopened);
}
