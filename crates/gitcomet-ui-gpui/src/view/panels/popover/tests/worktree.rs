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

fn pending_branch_removal(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
) -> Option<String> {
    cx.update(|_window, app| {
        view.read(app)
            .pending_worktree_branch_removals
            .get(&(repo_id, PathBuf::from(FEATURE)))
            .cloned()
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
fn remove_worktree_keeps_its_branch_unless_asked(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(&view, cx, remove_confirm(repo_id, Some("feat/x")));

    assert!(
        cx.debug_bounds("worktree_remove_delete_branch").is_some(),
        "a worktree on a branch offers to delete it"
    );
    click(cx, "worktree_remove_go");

    assert_eq!(popover_kind(&view, cx), None);
    assert_eq!(
        pending_branch_removal(&view, cx, repo_id),
        None,
        "the branch survives a plain remove"
    );
}

#[gpui::test]
fn remove_worktree_deletes_the_branch_once_ticked(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(&view, cx, remove_confirm(repo_id, Some("feat/x")));

    click(cx, "worktree_remove_delete_branch");
    click(cx, "worktree_remove_go");

    assert_eq!(popover_kind(&view, cx), None);
    assert_eq!(
        pending_branch_removal(&view, cx, repo_id).as_deref(),
        Some("feat/x")
    );
}

#[gpui::test]
fn remove_worktree_checkbox_resets_when_the_dialog_reopens(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_view(cx, repo_with_feature_worktree(repo_id), repo_id);
    open_popover(&view, cx, remove_confirm(repo_id, Some("feat/x")));
    click(cx, "worktree_remove_delete_branch");
    click(cx, "worktree_remove_cancel");
    assert_eq!(popover_kind(&view, cx), None);

    open_popover(&view, cx, remove_confirm(repo_id, Some("feat/x")));
    click(cx, "worktree_remove_go");

    assert_eq!(
        pending_branch_removal(&view, cx, repo_id),
        None,
        "a tick from an earlier dialog must not carry over"
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
