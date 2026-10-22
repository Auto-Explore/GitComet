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

