use super::super::worktree_picker;
use super::*;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state};
use crate::view::test_support::{push_test_state, redraw};
use gitcomet_core::domain::{CommitId, Worktree};
use gitcomet_state::model::{Loadable, RepoId, RepoState};
use std::path::PathBuf;

const MAIN: &str = "/tmp/ws/main";
const FEATURE: &str = "/tmp/ws/feature";

fn worktree(path: &str, branch: Option<&str>) -> Worktree {
    Worktree {
        bare: false,
        path: PathBuf::from(path),
        head: Some(CommitId(Arc::from(
            "399f41d0000000000000000000000000000000aa",
        ))),
        branch: branch.map(str::to_string),
        detached: branch.is_none(),
    }
}

fn repo_with_feature_worktree(repo_id: RepoId) -> RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new(MAIN));
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        worktree(MAIN, Some("main")),
        worktree(FEATURE, Some("feat/x")),
    ]));
    repo
}

fn open_view(
    cx: &mut gpui::TestAppContext,
    repo: RepoState,
    repo_id: RepoId,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
        let _ = window.draw(app);
    });
    (view, cx)
}

fn open_popover(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    kind: PopoverKind,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(kind, gpui::point(px(120.0), px(72.0)), window, cx);
            });
        });
    });
    redraw(cx);
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    redraw(cx);
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} must be drawn"))
        .center();
    cx.simulate_mouse_down(center, MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, gpui::Modifiers::default());
    redraw(cx);
}

fn popover_kind(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Option<PopoverKind> {
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

fn remove_confirm(repo_id: RepoId, branch: Option<&str>) -> PopoverKind {
    PopoverKind::worktree(
        repo_id,
        WorktreePopoverKind::RemoveConfirm {
            path: PathBuf::from(FEATURE),
            branch: branch.map(str::to_string),
        },
    )
}

#[gpui::test]
fn remove_worktree_offers_to_delete_its_branch(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (_view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(&_view, cx, remove_confirm(repo_id, Some("feat/x")));

    assert!(
        cx.debug_bounds("worktree_remove_delete_branch").is_some(),
        "a worktree on a branch offers to delete it"
    );
}

#[gpui::test]
fn remove_worktree_without_a_branch_offers_no_checkbox(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(&view, cx, remove_confirm(repo_id, None));

    assert!(cx.debug_bounds("worktree_remove_go").is_some());
    assert!(cx.debug_bounds("worktree_remove_delete_branch").is_none());
}

#[gpui::test]
fn remove_picker_names_the_worktree_branch(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(repo_id, WorktreePopoverKind::RemovePicker),
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                worktree_picker::activate(
                    host,
                    repo_id,
                    true,
                    PathBuf::from(FEATURE),
                    None,
                    window,
                    cx,
                );
            });
        });
    });
    redraw(cx);

    assert_eq!(
        popover_kind(&view, cx),
        Some(remove_confirm(repo_id, Some("feat/x")))
    );
}

fn host_read<T>(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    read: impl FnOnce(&PopoverHost, &gpui::App) -> T,
) -> T {
    cx.update(|_window, app| read(view.read(app).popover_host.read(app), app))
}

/// `main` tracks `origin/main`, the remote's default; `feat/free` is checked out nowhere.
fn repo_with_default_branch(repo_id: RepoId) -> RepoState {
    let mut repo = repo_with_feature_worktree(repo_id);
    let target = CommitId(Arc::from("1111111111111111111111111111111111111111"));
    repo.branches = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::Branch {
            name: "main".to_string(),
            target: target.clone(),
            upstream: Some(gitcomet_core::domain::Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            divergence: None,
        },
        gitcomet_core::domain::Branch {
            name: "feat/free".to_string(),
            target,
            upstream: None,
            divergence: None,
        },
    ]));
    repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
        name: "origin".to_string(),
        url: None,
        default_branch: Some("main".to_string()),
    }]));
    repo
}

#[gpui::test]
fn add_worktree_creates_a_branch_from_the_default_branch_by_default(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_default_branch(repo_id), repo_id);
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
    );

    let (create, source) = host_read(&view, cx, |host, _| {
        (
            host.worktree_add_create_branch,
            host.worktree_ref_source_target.clone(),
        )
    });
    assert!(create, "Create new branch starts ticked");
    assert_eq!(
        host_read(&view, cx, |host, app| host
            .worktree_path_input_text_for_tests(app)),
        MAIN,
        "the folder starts as the current worktree's path"
    );
    assert_eq!(source, "main", "the remote's default branch is preselected");
    assert!(cx.debug_bounds("worktree_add_new_branch").is_some());
    assert!(cx.debug_bounds("worktree_add_refused").is_none());

    click(cx, "worktree_add_create_branch");

    assert!(!host_read(&view, cx, |host, _| host.worktree_add_create_branch));
    assert!(
        cx.debug_bounds("worktree_add_new_branch").is_none(),
        "checking out a branch needs no new name"
    );
    assert!(cx.debug_bounds("worktree_add_refused").is_none());
    assert!(
        cx.debug_bounds("worktree_add_checked_out").is_some(),
        "main is checked out in this worktree already"
    );
    assert!(cx.debug_bounds("worktree_add_go").is_none());

    click(cx, "worktree_add_open");

    assert_eq!(
        popover_kind(&view, cx),
        None,
        "opening the worktree the dialog started in only closes it"
    );
}

/// Opens the local branch's menu and picks "Open in new worktree".
fn open_branch_in_new_worktree(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
    branch: &str,
) {
    let menu = PopoverKind::BranchMenu {
        repo_id,
        target: BranchMenuTarget::Local {
            name: branch.to_string(),
        },
    };
    open_popover(view, cx, menu.clone());

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let action = host
                    .context_menu_model(&menu, cx)
                    .expect("branch menu model")
                    .items
                    .into_iter()
                    .find_map(|item| match item {
                        ContextMenuItem::Entry { label, action, .. }
                            if label.as_ref() == "Open in new worktree" =>
                        {
                            Some(*action)
                        }
                        _ => None,
                    })
                    .expect("the branch menu offers a new worktree");
                host.context_menu_activate_action(action, window, cx);
            });
        });
    });
    redraw(cx);
}

#[gpui::test]
fn branch_menu_opens_the_add_dialog_ready_to_check_the_branch_out(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_default_branch(repo_id), repo_id);
    open_branch_in_new_worktree(&view, cx, repo_id, "feat/free");

    assert_eq!(
        popover_kind(&view, cx),
        Some(PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::AddPrompt
        )),
        "the dialog stays open after the menu closes"
    );
    let (path, source, create, open_after) = host_read(&view, cx, |host, app| {
        (
            host.worktree_path_input_text_for_tests(app),
            host.worktree_ref_source_target.clone(),
            host.worktree_add_create_branch,
            host.worktree_add_open_after,
        )
    });
    assert_eq!(
        PathBuf::from(path),
        PathBuf::from(MAIN),
        "the folder starts as the current worktree's path"
    );
    assert_eq!(source, "feat/free");
    assert!(!create, "an existing local branch is checked out as-is");
    assert!(open_after, "the new worktree opens once git creates it");
    assert!(cx.debug_bounds("worktree_add_refused").is_none());
}

#[gpui::test]
fn a_branch_checked_out_elsewhere_gets_a_new_branch_or_opens_its_worktree(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_default_branch(repo_id), repo_id);
    open_branch_in_new_worktree(&view, cx, repo_id, "feat/x");

    assert_eq!(
        popover_kind(&view, cx),
        Some(PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::AddPrompt
        ))
    );
    let (source, create) = host_read(&view, cx, |host, _| {
        (
            host.worktree_ref_source_target.clone(),
            host.worktree_add_create_branch,
        )
    });
    assert_eq!(source, "feat/x");
    assert!(create, "git refuses to check feat/x out a second time");
    assert!(cx.debug_bounds("worktree_add_checked_out").is_none());
    assert!(cx.debug_bounds("worktree_add_go").is_some());

    click(cx, "worktree_add_create_branch");

    assert!(
        cx.debug_bounds("worktree_add_checked_out").is_some(),
        "unticked, the dialog says where feat/x is checked out"
    );
    assert!(cx.debug_bounds("worktree_add_open").is_some());
    assert!(cx.debug_bounds("worktree_add_go").is_none());
    assert!(cx.debug_bounds("worktree_add_refused").is_none());
}

#[gpui::test]
fn checking_out_a_remote_ref_says_the_worktree_starts_detached(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let mut repo = repo_with_default_branch(repo_id);
    // No local branch carries origin/main, so the dialog starts from the remote ref.
    repo.branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId(Arc::from("1111111111111111111111111111111111111111")),
    }]));
    let (view, cx) = open_view(cx, repo, repo_id);
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
    );
    assert_eq!(
        host_read(&view, cx, |host, _| host.worktree_ref_source_target.clone()),
        "origin/main"
    );
    assert!(
        cx.debug_bounds("worktree_add_detached").is_none(),
        "a new branch from origin/main is not detached"
    );

    click(cx, "worktree_add_create_branch");

    assert!(
        cx.debug_bounds("worktree_add_detached").is_some(),
        "git checks a remote ref out detached"
    );
}

/// Clicks `selector` and returns the folder the dialog it opens starts in.
fn browse_start(cx: &mut gpui::VisualTestContext, selector: &'static str) -> Option<PathBuf> {
    click(cx, selector);
    assert!(cx.did_prompt_for_paths(), "{selector} opens a dialog");
    let mut directory = None;
    cx.simulate_path_prompt_response(|options| {
        directory = options.directory.clone();
        None
    });
    redraw(cx);
    directory
}

#[gpui::test]
fn browse_opens_at_the_typed_worktree_folder_when_it_exists(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let dir = tempfile::tempdir().expect("temp dir");
    let workdir = dir.path().to_path_buf();
    let (view, cx) = open_view(cx, opening_repo_state(repo_id, &workdir), repo_id);
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
    );

    assert_eq!(
        browse_start(cx, "worktree_browse"),
        Some(workdir.clone()),
        "the prefilled folder exists"
    );

    let missing = workdir.join("missing");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.worktree_path_input.update(cx, |input, cx| {
                    input.set_text(missing.display().to_string(), cx);
                });
            });
        });
    });
    assert_eq!(
        browse_start(cx, "worktree_browse"),
        None,
        "a missing folder keeps the platform default"
    );
}

#[gpui::test]
fn force_removal_dialog_wraps_its_explanation_at_the_dialog_width(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(
        &view,
        cx,
        PopoverKind::ForceRemoveWorktreeConfirm(gitcomet_state::model::ForceRemoveWorktreePrompt {
            generation: 1,
            repo_id,
            path: PathBuf::from(FEATURE),
            delete_branch: Some("feat/x".to_string()),
            blocker: gitcomet_core::error::WorktreeRemovalBlocker::Submodules,
        }),
    );

    let cancel = cx
        .debug_bounds("force_remove_worktree_cancel")
        .expect("cancel is drawn");
    let go = cx
        .debug_bounds("force_remove_worktree_go")
        .expect("Delete anyway is drawn");
    let width = cx.update(|_window, app| {
        DIALOG_460_WIDTH.preferred_px(crate::ui_scale::UiScale::from_percent(
            crate::ui_scale::current(app).percent,
        ))
    });
    assert!(
        go.right() - cancel.left() <= width,
        "the buttons stay within the dialog's {width:?}: {cancel:?} {go:?}"
    );
}

// Driven through the real store: the remove dialog dispatches, the tracked
// repository refuses or removes, and the view shows what the store decided.

fn open_store_view<'a>(
    cx: &'a mut gpui::TestAppContext,
    label: &str,
) -> (
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    gitcomet_state::store::AppStore,
    Arc<super::branch::TrackingRepo>,
    RepoId,
    PathBuf,
) {
    let (store, events, repo, workdir) = super::branch::create_tracking_store(label);
    let repo_id = store
        .snapshot()
        .active_repo
        .expect("tracked repo is active");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (view, cx, store, repo, repo_id, workdir)
}

/// Waits until the repository ran `action` for the `times`th time and the
/// store finished the worktree command, then shows the result in the view.
fn settle(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    store: &gitcomet_state::store::AppStore,
    repo: &super::branch::TrackingRepo,
    action: &str,
    times: usize,
) {
    super::branch::wait_until(action, || {
        repo.actions().iter().filter(|done| *done == action).count() >= times
            && store
                .snapshot()
                .repos
                .iter()
                .all(|repo| repo.worktrees_in_flight == 0)
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    redraw(cx);
}

fn linked_worktree(workdir: &Path, name: &str) -> PathBuf {
    let path = workdir.join(name);
    std::fs::create_dir_all(&path).expect("create linked worktree directory");
    gitcomet_core::path_utils::canonicalize_or_original(path)
}

fn request_removal(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
    path: &Path,
    delete_branch: bool,
) {
    open_popover(
        view,
        cx,
        PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::RemoveConfirm {
                path: path.to_path_buf(),
                branch: Some("feat/x".to_string()),
            },
        ),
    );
    if delete_branch {
        click(cx, "worktree_remove_delete_branch");
    }
    click(cx, "worktree_remove_go");
}

fn force_prompt_path(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Option<PathBuf> {
    match popover_kind(view, cx) {
        Some(PopoverKind::ForceRemoveWorktreeConfirm(prompt)) => Some(prompt.path),
        _ => None,
    }
}

fn error_toast_count(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> usize {
    cx.update(|_window, app| view.read(app).toast_host.read(app).error_notices().len())
}

fn force_removal_after_submodule_refusal(cx: &mut gpui::TestAppContext, label: &str, name: &str) {
    let (view, cx, store, repo, repo_id, workdir) = open_store_view(cx, label);
    repo.block_worktree_removal(gitcomet_core::error::WorktreeRemovalBlocker::Submodules);
    let linked = linked_worktree(&workdir, name);

    request_removal(&view, cx, repo_id, &linked, true);
    settle(
        &view,
        cx,
        &store,
        &repo,
        &format!("remove:{}", linked.display()),
        1,
    );

    assert_eq!(
        force_prompt_path(&view, cx),
        Some(linked.clone()),
        "the refusal asks to force-remove the worktree the user chose"
    );
    assert_eq!(
        error_toast_count(&view, cx),
        0,
        "the prompt replaces the error"
    );

    click(cx, "force_remove_worktree_go");
    settle(
        &view,
        cx,
        &store,
        &repo,
        &format!("force-remove:{}", linked.display()),
        1,
    );
    super::branch::wait_until("the ticked branch to be deleted", || {
        repo.actions()
            .iter()
            .any(|done| done == "delete-branch:feat/x")
    });
}

#[gpui::test]
fn submodule_refusal_offers_to_force_remove_the_worktree(cx: &mut gpui::TestAppContext) {
    force_removal_after_submodule_refusal(cx, "worktree-force-remove", "feature");
}

/// The path travels as data: one ending in a space is still the one removed.
#[cfg(not(windows))]
#[gpui::test]
fn force_removal_keeps_a_path_that_ends_in_whitespace(cx: &mut gpui::TestAppContext) {
    force_removal_after_submodule_refusal(cx, "worktree-force-remove-space", "feature ");
}

#[gpui::test]
fn cancelled_force_removal_removes_nothing_and_asks_again(cx: &mut gpui::TestAppContext) {
    let (view, cx, store, repo, repo_id, workdir) =
        open_store_view(cx, "worktree-force-remove-cancel");
    repo.block_worktree_removal(gitcomet_core::error::WorktreeRemovalBlocker::Changes);
    let linked = linked_worktree(&workdir, "feature");
    let removal = format!("remove:{}", linked.display());

    request_removal(&view, cx, repo_id, &linked, false);
    settle(&view, cx, &store, &repo, &removal, 1);
    click(cx, "force_remove_worktree_cancel");
    assert_eq!(popover_kind(&view, cx), None);

    request_removal(&view, cx, repo_id, &linked, false);
    settle(&view, cx, &store, &repo, &removal, 2);
    assert_eq!(
        force_prompt_path(&view, cx),
        Some(linked.clone()),
        "a second refusal asks again"
    );
    assert!(
        !repo
            .actions()
            .iter()
            .any(|done| done.starts_with("force-remove:")),
        "cancelling forces nothing"
    );
}

#[gpui::test]
fn removal_keeps_the_branch_unless_asked(cx: &mut gpui::TestAppContext) {
    let (view, cx, store, repo, repo_id, workdir) = open_store_view(cx, "worktree-remove-keep");
    let linked = linked_worktree(&workdir, "feature");

    request_removal(&view, cx, repo_id, &linked, false);
    settle(
        &view,
        cx,
        &store,
        &repo,
        &format!("remove:{}", linked.display()),
        1,
    );

    assert!(
        !repo
            .actions()
            .iter()
            .any(|done| done.starts_with("delete-branch:")),
        "the branch survives a plain remove"
    );
}

#[gpui::test]
fn removal_deletes_the_ticked_branch_once_the_worktree_is_gone(cx: &mut gpui::TestAppContext) {
    let (view, cx, store, repo, repo_id, workdir) = open_store_view(cx, "worktree-remove-branch");
    let linked = linked_worktree(&workdir, "feature");

    request_removal(&view, cx, repo_id, &linked, true);
    settle(
        &view,
        cx,
        &store,
        &repo,
        &format!("remove:{}", linked.display()),
        1,
    );
    super::branch::wait_until("the ticked branch to be deleted", || {
        repo.actions()
            .iter()
            .any(|done| done == "delete-branch:feat/x")
    });
}

#[gpui::test]
fn removal_checkbox_resets_when_the_dialog_reopens(cx: &mut gpui::TestAppContext) {
    let (view, cx, store, repo, repo_id, workdir) = open_store_view(cx, "worktree-remove-reset");
    let linked = linked_worktree(&workdir, "feature");
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::RemoveConfirm {
                path: linked.clone(),
                branch: Some("feat/x".to_string()),
            },
        ),
    );
    click(cx, "worktree_remove_delete_branch");
    click(cx, "worktree_remove_cancel");

    request_removal(&view, cx, repo_id, &linked, false);
    settle(
        &view,
        cx,
        &store,
        &repo,
        &format!("remove:{}", linked.display()),
        1,
    );

    assert!(
        !repo
            .actions()
            .iter()
            .any(|done| done.starts_with("delete-branch:")),
        "a tick from an earlier dialog must not carry over"
    );
}

/// A refusal can land while another tab is active; the dialog then waits for
/// its repository instead of being dropped.
#[gpui::test]
fn force_removal_prompt_waits_for_its_repository_tab(cx: &mut gpui::TestAppContext) {
    let (origin, other) = (RepoId(1), RepoId(2));
    let (view, cx) = open_view(cx, repo_with_feature_worktree(origin), origin);
    let prompt = gitcomet_state::model::ForceRemoveWorktreePrompt {
        generation: 1,
        repo_id: origin,
        path: PathBuf::from(FEATURE),
        delete_branch: None,
        blocker: gitcomet_core::error::WorktreeRemovalBlocker::Submodules,
    };
    let state_with_active = |active: RepoId| {
        Arc::new(gitcomet_state::model::AppState {
            repos: vec![
                repo_with_feature_worktree(origin),
                opening_repo_state(other, Path::new("/tmp/ws/other")),
            ],
            active_repo: Some(active),
            force_remove_worktree_prompt: Some(prompt.clone()),
            ..gitcomet_state::model::AppState::test_default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state_with_active(other), cx)
        });
    });
    redraw(cx);
    assert_eq!(popover_kind(&view, cx), None, "not over another repository");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state_with_active(origin), cx)
        });
    });
    redraw(cx);
    assert_eq!(
        popover_kind(&view, cx),
        Some(PopoverKind::ForceRemoveWorktreeConfirm(prompt))
    );
}
