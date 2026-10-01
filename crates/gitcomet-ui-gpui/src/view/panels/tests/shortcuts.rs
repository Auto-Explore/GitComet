use super::*;
use crate::view::panes::main::{DiffWrapVisibleCacheKey, DiffWrapVisualRow};
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::domain::{CommitDetails, CommitFileChange};
use gpui::{ScrollDelta, ScrollWheelEvent};
use std::time::{Duration, Instant};

fn shortcut_entry<'a>(
    model: &'a ContextMenuModel,
    shortcut: &str,
) -> (&'a ContextMenuAction, usize) {
    if shortcut == "Enter" {
        let ix = runtime_entry_ix_for_shortcut(model, shortcut)
            .unwrap_or_else(|| panic!("expected shortcut `{shortcut}` to resolve at runtime"));
        return match model.items.get(ix) {
            Some(ContextMenuItem::Entry { action, .. }) => (action.as_ref(), ix),
            _ => panic!("expected runtime shortcut `{shortcut}` to target an entry"),
        };
    }

    model
        .items
        .iter()
        .enumerate()
        .find_map(|(ix, item)| match item {
            ContextMenuItem::Entry {
                shortcut: Some(entry_shortcut),
                action,
                ..
            } if entry_shortcut.as_ref() == shortcut => Some((action.as_ref(), ix)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected shortcut `{shortcut}` to exist"))
}

fn runtime_entry_ix_for_shortcut(model: &ContextMenuModel, shortcut: &str) -> Option<usize> {
    match shortcut {
        "Enter" => super::super::popover::context_menu::context_menu_activate_entry_ix(model, None),
        _ if shortcut.chars().count() == 1 => {
            let key = shortcut.to_ascii_lowercase();
            super::super::popover::context_menu::context_menu_shortcut_entry_ix(model, &key)
        }
        _ => None,
    }
}

macro_rules! assert_shortcut_action {
    ($model:expr, $shortcut:expr, $pat:pat $(if $guard:expr)? ) => {{
        let (action, expected_ix) = shortcut_entry(&$model, $shortcut);
        if let Some(runtime_ix) = runtime_entry_ix_for_shortcut(&$model, $shortcut) {
            assert_eq!(
                runtime_ix, expected_ix,
                "expected runtime resolution for `{}` to target entry {}",
                $shortcut, expected_ix
            );
        }
        assert!(
            matches!(action, $pat $(if $guard)?),
            "unexpected action for shortcut `{}`",
            $shortcut,
        );
    }};
}

pub(super) fn apply_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    state: Arc<AppState>,
) {
    let store_state = Arc::clone(&state);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.store
                .replace_snapshot_for_test(Arc::clone(&store_state));
            push_test_state(this, Arc::clone(&state), cx);
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn sync_store_snapshot(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });
    draw_and_drain_test_window(cx);
}

pub(super) fn wait_until(
    cx: &mut gpui::VisualTestContext,
    description: &str,
    ready: impl Fn(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        draw_and_drain_test_window(cx);
        if ready(cx) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until_store_diff_target_path(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    expected: &std::path::Path,
) {
    wait_until(cx, "store diff target to update", |cx| {
        cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            let Some(repo_id) = snapshot.active_repo else {
                return false;
            };
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            match repo.diff_state.diff_target.as_ref() {
                Some(DiffTarget::WorkingTree { path, .. }) => path == expected,
                Some(
                    DiffTarget::Commit { path, .. }
                    | DiffTarget::CommitRange {
                        path: Some(path), ..
                    },
                ) => path == expected,
                _ => false,
            }
        })
    });
}

pub(super) fn app_state_with_active_repo(repo: RepoState) -> Arc<AppState> {
    let repo_id = repo.id;
    Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    })
}

fn diff_panel_is_focused(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .is_focused(window)
    })
}

fn popover_is_open(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).popover_host.read(app).is_open())
}

fn active_worktree_diff_target_path(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<std::path::PathBuf> {
    cx.update(|_window, app| {
        let root = view.read(app);
        let repo_id = root.state.active_repo?;
        let repo = root.state.repos.iter().find(|repo| repo.id == repo_id)?;
        match repo.diff_state.diff_target.clone()? {
            DiffTarget::WorkingTree { path, .. } => Some(path),
            _ => None,
        }
    })
}

fn focus_commit_message_input(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });
}

fn commit_message_input_is_focused(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle()
            .is_focused(window)
    })
}

fn diff_search_input_is_focused(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_input
            .read(app)
            .focus_handle()
            .is_focused(window)
    })
}

fn diff_selection_anchor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<usize> {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_selection_anchor)
}

fn diff_selection_range(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<(usize, usize)> {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_selection_range)
}

fn diff_text_has_selection(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_text_has_selection())
}

fn set_diff_selection_anchor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    anchor: Option<usize>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_selection_anchor = anchor;
                pane.diff_selection_range = anchor.map(|ix| (ix, ix));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn set_diff_text_selection_on_row(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    visible_ix: usize,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: visible_ix,
                    region: DiffTextRegion::Inline,
                    offset: 0,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: visible_ix,
                    region: DiffTextRegion::Inline,
                    offset: 1,
                });
                pane.diff_selection_anchor = Some(visible_ix);
                pane.diff_selection_range = None;
                // Match production: a real selection owns the window's, so a
                // seeded one must too or the next press collapses it.
                pane.diff_text_selection_owner.adopt(window, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn diff_search_active(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_active)
}

fn conflict_navigation_anchor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<usize> {
    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .conflict_resolver
            .nav_anchor
            .map(|anchor| anchor.order_hint)
    })
}

fn active_conflict_ix(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> usize {
    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .conflict_resolver
            .active_conflict
            .expect("test resolver should have an actionable displayed conflict")
    })
}

fn open_change_tracking_settings_popover(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::ChangeTrackingSettings,
                    gpui::point(px(72.0), px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
}

fn bind_app_keys_for_test(cx: &mut gpui::VisualTestContext) {
    cx.update(|_window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
    });
}

fn bind_app_keys_and_global_diff_fallback_for_test(cx: &mut gpui::VisualTestContext) {
    cx.update(|_window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });
}

fn install_global_diff_shortcut_fallback_for_test(cx: &mut gpui::VisualTestContext) {
    cx.update(|_window, app| {
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });
}

fn focus_detached_window_focus(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let focus = app.focus_handle();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
}

fn open_popover_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    kind: impl Into<PopoverRequest>,
) {
    let kind: PopoverRequest = kind.into();
    cx.update(|window, app| {
        let kind = kind.clone();
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(kind.clone(), gpui::point(px(72.0), px(72.0)), window, cx);
            });
        });
        let _ = window.draw(app);
    });
}

fn debug_width(cx: &mut gpui::VisualTestContext, selector: &'static str) -> f32 {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected `{selector}` bounds"));
    bounds.size.width.into()
}

fn assert_context_menu_entry_fills_popover_width(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) {
    let popover_width = debug_width(cx, "app_popover");
    let entry_width = debug_width(cx, selector);
    assert!(
        entry_width >= popover_width * 0.80,
        "expected `{selector}` to fill most of the popover width (entry={entry_width}, popover={popover_width})"
    );
}

pub(super) fn shortcut_fixture_repo(
    repo_id: RepoId,
    workdir: &std::path::Path,
    commit_id: &CommitId,
) -> RepoState {
    let mut repo = RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: workdir.to_path_buf(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
    repo.log = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits: vec![gitcomet_core::domain::Commit {
                id: commit_id.clone(),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "Initial commit".into(),
                author: "Alice".into(),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        }
        .into(),
    );
    repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
        name: "origin".into(),
        url: Some("https://example.com/origin.git".into()),
    }]));
    repo.tags = Loadable::Ready(Arc::new(vec![]));
    repo.remote_tags = Loadable::Ready(Arc::new(vec![]));
    repo.stashes = Loadable::Ready(Arc::new(vec![]));
    repo
}

fn simple_hunk_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    gitcomet_core::domain::Diff {
        target,
        lines: vec![
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "diff --git a/src/lib.rs b/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "--- a/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "+++ b/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-old".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Add,
                text: "+new".into(),
            },
        ],
    }
}

fn two_hunk_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    gitcomet_core::domain::Diff {
        target,
        lines: vec![
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "diff --git a/src/lib.rs b/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "--- a/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Header,
                text: "+++ b/src/lib.rs".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Hunk,
                text: "@@ -1 +1 @@".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-old one".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Add,
                text: "+new one".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Context,
                text: " unchanged".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Hunk,
                text: "@@ -10 +10 @@".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Remove,
                text: "-old two".into(),
            },
            gitcomet_core::domain::DiffLine {
                kind: gitcomet_core::domain::DiffLineKind::Add,
                text: "+new two".into(),
            },
        ],
    }
}

fn simple_worktree_repo(
    repo_id: RepoId,
    workdir: &std::path::Path,
    commit_id: &CommitId,
    paths: &[std::path::PathBuf],
    selected_path: &std::path::Path,
) -> RepoState {
    let mut repo = shortcut_fixture_repo(repo_id, workdir, commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(
                paths
                    .iter()
                    .cloned()
                    .map(|path| gitcomet_core::domain::FileStatus {
                        path,
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    })
                    .collect(),
            ),
        }
        .into(),
    );
    let target = DiffTarget::working_tree(selected_path.to_path_buf(), DiffArea::Unstaged);
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(target).into());
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
    repo
}

fn simple_conflict_repo(
    repo_id: RepoId,
    workdir: &std::path::Path,
    commit_id: &CommitId,
    path: &std::path::Path,
) -> RepoState {
    let path = path.to_path_buf();
    let base = "base one\nbase two\n";
    let ours = "ours one\nours two\n";
    let theirs = "theirs one\ntheirs two\n";
    let current = concat!(
        "context before\n",
        "<<<<<<< ours\n",
        "ours one\n",
        "=======\n",
        "theirs one\n",
        ">>>>>>> theirs\n",
        "middle context\n",
        "<<<<<<< ours\n",
        "ours two\n",
        "=======\n",
        "theirs two\n",
        ">>>>>>> theirs\n",
    );

    let mut repo = shortcut_fixture_repo(repo_id, workdir, commit_id);
    set_test_conflict_status(&mut repo, path.clone(), DiffArea::Unstaged);
    set_test_conflict_file(&mut repo, path.clone(), base, ours, theirs, current);
    repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
        path,
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text(base.into()),
        ConflictPayload::Text(ours.into()),
        ConflictPayload::Text(theirs.into()),
        current,
    ));
    repo.conflict_state.conflict_rev = 1;
    repo
}

fn author_filter_fixture_repo(repo_id: RepoId) -> RepoState {
    author_filter_repo_with_authors(repo_id, &["Alice", "Bob"])
}

fn author_filter_repo_with_authors(repo_id: RepoId, authors: &[&str]) -> RepoState {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_author_filter",
        std::process::id()
    ));
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &CommitId("deadbeefdeadbeef".into()));
    let commits = authors
        .iter()
        .enumerate()
        .map(|(index, author)| gitcomet_core::domain::Commit {
            id: CommitId(format!("deadbeefdeadbee{index}").into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: format!("commit {index}").into(),
            author: (*author).into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        })
        .collect();
    let log_page: Loadable<std::sync::Arc<gitcomet_core::domain::LogPage>> = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits,
            next_cursor: None,
        }
        .into(),
    );
    repo.log = log_page.clone();
    repo.history_state.log = log_page;
    repo
}

mod hook_activity;
mod status_selection;
mod window_and_file_actions;

mod context_menus;
mod file_navigation;
mod focus_contexts;
mod search;
mod staging_conflicts;
