use super::*;
use crate::view::panes::main::{DiffWrapVisibleCacheKey, DiffWrapVisualRow};
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::domain::{CommitDetails, CommitFileChange};
use gpui::{ScrollDelta, ScrollWheelEvent};
use std::time::{Duration, Instant};

fn copied_path_ends_with(text: &str, suffix: &std::path::Path) -> bool {
    let normalize = |value: &str| value.replace('\\', "/");
    normalize(text).ends_with(&normalize(&suffix.to_string_lossy()))
}

fn declared_shortcuts(model: &ContextMenuModel) -> Vec<String> {
    model
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { shortcut, .. } => shortcut.as_ref().map(|s| s.to_string()),
            _ => None,
        })
        .collect()
}

fn assert_declared_shortcuts(model: &ContextMenuModel, expected: &[&str]) {
    let expected = expected.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(declared_shortcuts(model), expected);
}

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

fn context_menu_model_for(
    view: &gpui::Entity<super::super::GitCometView>,
    app: &mut gpui::App,
    kind: PopoverKind,
) -> ContextMenuModel {
    view.update(app, |this, cx| {
        this.popover_host.update(cx, |host, cx| {
            host.context_menu_model(&kind, cx)
                .unwrap_or_else(|| panic!("expected context menu model for {kind:?}"))
        })
    })
}

fn apply_state(
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

fn wait_until(
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

fn wait_for_diff_search_debounce(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
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
                Some(DiffTarget::Commit {
                    path: Some(path), ..
                }) => path == expected,
                _ => false,
            }
        })
    });
}

fn app_state_with_active_repo(repo: RepoState) -> Arc<AppState> {
    let repo_id = repo.id;
    Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    })
}

fn set_change_tracking_view_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    next: ChangeTrackingView,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| this.set_change_tracking_view(next, cx));
        let _ = window.draw(app);
    });
    cx.run_until_parked();
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

fn active_commit_diff_target_path(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<std::path::PathBuf> {
    cx.update(|_window, app| {
        let root = view.read(app);
        let repo_id = root.state.active_repo?;
        let repo = root.state.repos.iter().find(|repo| repo.id == repo_id)?;
        match repo.diff_state.diff_target.clone()? {
            DiffTarget::Commit {
                path: Some(path), ..
            } => Some(path),
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

fn focus_diff_search_input(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                let focus = pane.diff_search_input.read(cx).focus_handle();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
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

fn set_diff_selection_area(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    anchor: Option<usize>,
    range: Option<(usize, usize)>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_selection_anchor = anchor;
                pane.diff_selection_range = range;
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
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn diff_view_mode(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> DiffViewMode {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_view)
}

fn reveal_whitespace_chars(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).main_pane.read(app).reveal_whitespace_chars)
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
    kind: PopoverKind,
) {
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

fn set_ui_scale_percent_for_test(
    cx: &mut gpui::VisualTestContext,
    _view: &gpui::Entity<super::super::GitCometView>,
    percent: u32,
) {
    cx.update(|_window, app| {
        crate::app::set_app_ui_scale_percent(app, percent);
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

fn shortcut_fixture_repo(
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

fn three_hunk_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    let mut diff = two_hunk_diff(target);
    diff.lines.extend([
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Context,
            text: " unchanged again".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Hunk,
            text: "@@ -20 +20 @@".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Remove,
            text: "-old three".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Add,
            text: "+new three".into(),
        },
    ]);
    diff
}

fn searchable_scroll_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    let mut lines = vec![
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
            text: "@@ -1,160 +1,160 @@".into(),
        },
    ];

    for ix in 0..160 {
        let text = match ix {
            1 => " context needle first".to_string(),
            120 => " context needle second".to_string(),
            _ => format!(" context filler line {ix}"),
        };
        lines.push(gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Context,
            text: text.into(),
        });
    }

    gitcomet_core::domain::Diff { target, lines }
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
            staged: vec![],
            unstaged: paths
                .iter()
                .cloned()
                .map(|path| gitcomet_core::domain::FileStatus {
                    path,
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                })
                .collect(),
        }
        .into(),
    );
    let target = DiffTarget::WorkingTree {
        path: selected_path.to_path_buf(),
        area: DiffArea::Unstaged,
    };
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

#[gpui::test]
fn history_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(700);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_settings_history_shortcuts",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let history_filter_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::HistoryBranchFilter { repo_id })
    });
    assert_declared_shortcuts(&history_filter_model, &["F", "P", "N", "M", "A"]);
    assert_shortcut_action!(
        history_filter_model,
        "F",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::FullReachable
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "P",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::FirstParent
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "N",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::NoMerges
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "M",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::MergesOnly
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "A",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::LogScope::AllBranches
        } if *rid == repo_id
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.history_view.update(cx, |history, cx| {
                    history.history_show_author = true;
                    history.history_show_date = true;
                    history.history_show_sha = true;
                    cx.notify();
                });
            });
        });
    });

    let change_tracking_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::ChangeTrackingSettings)
    });
    assert_declared_shortcuts(&change_tracking_model, &["C", "S"]);
    assert_shortcut_action!(
        change_tracking_model,
        "C",
        ContextMenuAction::SetChangeTrackingView {
            view: ChangeTrackingView::Combined
        }
    );
    assert_shortcut_action!(
        change_tracking_model,
        "S",
        ContextMenuAction::SetChangeTrackingView {
            view: ChangeTrackingView::SplitUntracked
        }
    );
}

#[gpui::test]
fn repo_operation_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(701);
    let commit_id = CommitId("feedfacefeedface".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_repo_shortcuts",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let pull_model =
        cx.update(|_window, app| context_menu_model_for(&view, app, PopoverKind::PullPicker));
    assert_declared_shortcuts(&pull_model, &["F", "O", "R", "A"]);
    assert_shortcut_action!(
        pull_model,
        "Enter",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::Default
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "F",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::FastForwardIfPossible
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "O",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::FastForwardOnly
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "R",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::Rebase
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "A",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let push_model =
        cx.update(|_window, app| context_menu_model_for(&view, app, PopoverKind::PushPicker));
    assert_declared_shortcuts(&push_model, &["F"]);
    assert_shortcut_action!(
        push_model,
        "Enter",
        ContextMenuAction::Push { repo_id: rid } if *rid == repo_id
    );
    assert_shortcut_action!(
        push_model,
        "F",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::ForcePushConfirm { repo_id: rid }
        } if *rid == repo_id
    );

    let branch_section_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchSectionMenu {
                repo_id,
                section: BranchSection::Remote,
            },
        )
    });
    assert_declared_shortcuts(&branch_section_model, &["F"]);
    assert_shortcut_action!(
        branch_section_model,
        "Enter",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::BranchPicker
        }
    );
    assert_shortcut_action!(
        branch_section_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let local_branch_name = "feature".to_string();
    let local_branch_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: local_branch_name.clone(),
            },
        )
    });
    assert_declared_shortcuts(&local_branch_model, &["P", "M", "S"]);
    assert_shortcut_action!(
        local_branch_model,
        "Enter",
        ContextMenuAction::CheckoutBranch { repo_id: rid, name } if *rid == repo_id && name == "feature"
    );
    assert_shortcut_action!(
        local_branch_model,
        "P",
        ContextMenuAction::PullBranch {
            repo_id: rid,
            remote,
            branch
        } if *rid == repo_id && remote == "." && branch == "feature"
    );
    assert_shortcut_action!(
        local_branch_model,
        "M",
        ContextMenuAction::MergeRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "feature"
    );
    assert_shortcut_action!(
        local_branch_model,
        "S",
        ContextMenuAction::SquashRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "feature"
    );

    let remote_branch_name = "origin/feature".to_string();
    let remote_branch_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Remote,
                name: remote_branch_name.clone(),
            },
        )
    });
    assert_declared_shortcuts(&remote_branch_model, &["P", "M", "S", "F"]);
    assert_shortcut_action!(
        remote_branch_model,
        "Enter",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::CheckoutRemoteBranchPrompt {
                repo_id: rid,
                remote,
                branch
            }
        } if *rid == repo_id && remote == "origin" && branch == "feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "P",
        ContextMenuAction::PullBranch {
            repo_id: rid,
            remote,
            branch
        } if *rid == repo_id && remote == "origin" && branch == "feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "M",
        ContextMenuAction::MergeRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "origin/feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "S",
        ContextMenuAction::SquashRef {
            repo_id: rid,
            reference
        } if *rid == repo_id && reference == "origin/feature"
    );
    assert_shortcut_action!(
        remote_branch_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let remote_menu_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::remote(
                repo_id,
                RemotePopoverKind::Menu {
                    name: "origin".into(),
                },
            ),
        )
    });
    assert_declared_shortcuts(&remote_menu_model, &["F"]);
    assert_shortcut_action!(
        remote_menu_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let stash_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StashMenu {
                repo_id,
                index: 3,
                message: "WIP".into(),
            },
        )
    });
    assert_declared_shortcuts(&stash_model, &["A", "P"]);
    assert_shortcut_action!(
        stash_model,
        "A",
        ContextMenuAction::ApplyStash {
            repo_id: rid,
            index
        } if *rid == repo_id && *index == 3
    );
    assert_shortcut_action!(
        stash_model,
        "P",
        ContextMenuAction::PopStash {
            repo_id: rid,
            index
        } if *rid == repo_id && *index == 3
    );
}

#[gpui::test]
fn file_and_diff_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(702);
    let commit_id = CommitId("cafebabecafebabe".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_shortcuts",
        std::process::id()
    ));
    let commit_file_path = std::path::PathBuf::from("src/main.rs");
    let unstaged_path = std::path::PathBuf::from("unstaged.rs");
    let staged_path = std::path::PathBuf::from("staged_added.rs");
    let conflicted_path = std::path::PathBuf::from("conflicted.rs");
    let hunk_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![gitcomet_core::domain::FileStatus {
                path: staged_path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Added,
                conflict: None,
            }],
            unstaged: vec![
                gitcomet_core::domain::FileStatus {
                    path: unstaged_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: hunk_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: conflicted_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                    conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                },
            ],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: hunk_path.clone(),
        area: DiffArea::Unstaged,
    });
    repo.diff_state.diff = Loadable::Ready(
        simple_hunk_diff(DiffTarget::WorkingTree {
            path: hunk_path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let commit_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::CommitMenu {
                repo_id,
                commit_id: commit_id.clone(),
            },
        )
    });
    assert_declared_shortcuts(&commit_model, &["T", "D", "P", "R"]);
    assert_shortcut_action!(
        commit_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::Commit {
                commit_id: cid,
                path: None
            }
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "T",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateTagPrompt { repo_id: rid, target }
        } if *rid == repo_id && target == commit_id.as_ref()
    );
    assert_shortcut_action!(
        commit_model,
        "D",
        ContextMenuAction::CheckoutCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "P",
        ContextMenuAction::CherryPickCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "R",
        ContextMenuAction::RevertCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );

    let commit_file_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::CommitFileMenu {
                repo_id,
                commit_id: commit_id.clone(),
                path: commit_file_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&commit_file_model, &["H", "C"]);
    assert_shortcut_action!(
        commit_file_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::Commit {
                commit_id: cid,
                path: Some(path)
            }
        } if *rid == repo_id && cid == &commit_id && path == &commit_file_path
    );
    assert_shortcut_action!(
        commit_file_model,
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &commit_file_path
    );
    assert_shortcut_action!(
        commit_file_model,
        "C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &commit_file_path)
    );

    let unstaged_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: unstaged_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(
        &unstaged_status_model,
        &["Ctrl+H", "Ctrl+S", "Ctrl+D", "Ctrl+Shift+C"],
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::WorkingTree { path, area }
        } if *rid == repo_id && path == &unstaged_path && *area == DiffArea::Unstaged
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "Ctrl+H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "Ctrl+S",
        ContextMenuAction::StageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "Ctrl+D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "Ctrl+Shift+C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &unstaged_path)
    );

    let staged_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Staged,
                path: staged_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(
        &staged_status_model,
        &["Ctrl+H", "Ctrl+U", "Ctrl+D", "Ctrl+Shift+C"],
    );
    assert_shortcut_action!(
        staged_status_model,
        "Enter",
        ContextMenuAction::SelectDiff {
            repo_id: rid,
            target: DiffTarget::WorkingTree { path, area }
        } if *rid == repo_id && path == &staged_path && *area == DiffArea::Staged
    );
    assert_shortcut_action!(
        staged_status_model,
        "Ctrl+H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "Ctrl+U",
        ContextMenuAction::UnstageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "Ctrl+D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "Ctrl+Shift+C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &staged_path)
    );

    let conflicted_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: conflicted_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(
        &conflicted_status_model,
        &[
            "Ctrl+H",
            "Ctrl+O",
            "Ctrl+T",
            "Ctrl+M",
            "Ctrl+D",
            "Ctrl+Shift+C",
        ],
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Enter",
        ContextMenuAction::SelectConflictDiff {
            repo_id: rid,
            path
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Ctrl+H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Ctrl+O",
        ContextMenuAction::CheckoutConflictSideSelectionOrPath {
            repo_id: rid,
            area,
            path,
            side
        } if *rid == repo_id
            && *area == DiffArea::Unstaged
            && path == &conflicted_path
            && *side == gitcomet_core::services::ConflictSide::Ours
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Ctrl+T",
        ContextMenuAction::CheckoutConflictSideSelectionOrPath {
            repo_id: rid,
            area,
            path,
            side
        } if *rid == repo_id
            && *area == DiffArea::Unstaged
            && path == &conflicted_path
            && *side == gitcomet_core::services::ConflictSide::Theirs
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Ctrl+M",
        ContextMenuAction::SelectConflictDiff {
            repo_id: rid,
            path
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Ctrl+D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Ctrl+Shift+C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &conflicted_path)
    );

    let diff_editor_unstaged_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::DiffEditorMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: Some(unstaged_path.clone()),
                hunk_patch: Some("hunk patch".into()),
                hunks_count: 2,
                lines_patch: Some("line patch".into()),
                discard_lines_patch: Some("discard patch".into()),
                lines_count: 3,
                copy_text: Some("copied selection".into()),
                copy_target: None,
            },
        )
    });
    assert_declared_shortcuts(&diff_editor_unstaged_model, &["S", "D", "C"]);
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "S",
        ContextMenuAction::ApplyIndexPatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "line patch" && !*reverse
    );
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "D",
        ContextMenuAction::ApplyWorktreePatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "discard patch" && *reverse
    );
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "C",
        ContextMenuAction::CopyText { text } if text == "copied selection"
    );

    let diff_editor_staged_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::DiffEditorMenu {
                repo_id,
                area: DiffArea::Staged,
                path: Some(staged_path.clone()),
                hunk_patch: Some("staged hunk".into()),
                hunks_count: 1,
                lines_patch: Some("staged line".into()),
                discard_lines_patch: None,
                lines_count: 1,
                copy_text: Some("staged copy".into()),
                copy_target: None,
            },
        )
    });
    assert_declared_shortcuts(&diff_editor_staged_model, &["U", "C"]);
    assert_shortcut_action!(
        diff_editor_staged_model,
        "U",
        ContextMenuAction::ApplyIndexPatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "staged line" && *reverse
    );
    assert_shortcut_action!(
        diff_editor_staged_model,
        "C",
        ContextMenuAction::CopyText { text } if text == "staged copy"
    );

    let diff_hunk_unstaged_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::DiffHunkMenu { repo_id, src_ix: 3 })
    });
    assert_declared_shortcuts(&diff_hunk_unstaged_model, &["Ctrl+S", "Ctrl+D"]);
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        "Ctrl+S",
        ContextMenuAction::StageHunk {
            repo_id: rid,
            src_ix
        } if *rid == repo_id && *src_ix == 3
    );
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        "Ctrl+D",
        ContextMenuAction::ApplyWorktreePatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && !patch.is_empty() && *reverse
    );

    let conflict_output_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line: 12,
                selected_text: Some("chosen text".into()),
                has_source_a: true,
                has_source_b: true,
                has_source_c: true,
                is_three_way: true,
            },
        )
    });
    assert_declared_shortcuts(&conflict_output_model, &["Ctrl+C", "Ctrl+X", "Ctrl+V"]);
    assert_shortcut_action!(
        conflict_output_model,
        "Ctrl+C",
        ContextMenuAction::CopyText { text } if text == "chosen text"
    );
    assert_shortcut_action!(
        conflict_output_model,
        "Ctrl+X",
        ContextMenuAction::ConflictResolverOutputCut { text } if text == "chosen text"
    );
    assert_shortcut_action!(
        conflict_output_model,
        "Ctrl+V",
        ContextMenuAction::ConflictResolverOutputPaste
    );
}

#[gpui::test]
fn split_untracked_file_navigation_stays_within_untracked_section(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(703);
    let commit_id = CommitId("cafebabecafebabe".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_untracked_nav",
        std::process::id()
    ));
    let untracked_a = std::path::PathBuf::from("new-a.txt");
    let tracked = std::path::PathBuf::from("src/lib.rs");
    let untracked_b = std::path::PathBuf::from("new-b.txt");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![
                gitcomet_core::domain::FileStatus {
                    path: untracked_a.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: untracked_b.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
            ],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: untracked_a.clone(),
        area: DiffArea::Unstaged,
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    set_change_tracking_view_for_test(cx, &view, ChangeTrackingView::SplitUntracked);

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, 1, window, cx)
        })
    });
    assert!(
        moved,
        "expected adjacent navigation to move to the next untracked row"
    );
}

#[gpui::test]
fn split_tracked_file_navigation_does_not_cross_into_untracked_section(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(704);
    let commit_id = CommitId("deadc0dedeadc0de".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_tracked_nav",
        std::process::id()
    ));
    let untracked = std::path::PathBuf::from("new-a.txt");
    let tracked_a = std::path::PathBuf::from("src/lib.rs");
    let tracked_b = std::path::PathBuf::from("src/main.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![
                gitcomet_core::domain::FileStatus {
                    path: untracked.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked_a.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked_b.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
            ],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: tracked_a.clone(),
        area: DiffArea::Unstaged,
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    set_change_tracking_view_for_test(cx, &view, ChangeTrackingView::SplitUntracked);

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, -1, window, cx)
        })
    });
    assert!(
        !moved,
        "tracked-section navigation should not jump into the split untracked section"
    );
}

#[gpui::test]
fn commit_details_file_navigation_scrolls_selected_row_into_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7051);
    let commit_id = CommitId("fedcba0987654321".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_details_file_nav_scroll",
        std::process::id()
    ));
    let files = (0..64)
        .map(|ix| CommitFileChange {
            path: std::path::PathBuf::from(format!("src/commit_nav/file_{ix:02}.rs")),
            kind: FileStatusKind::Modified,
            is_submodule: false,
        })
        .collect::<Vec<_>>();
    let start_ix = 40usize;
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id.clone(),
        message: "subject".into(),
        committed_at: "2026-04-14 12:00:00 +0300".into(),
        parent_ids: vec![],
        files: files.clone(),
    }));
    repo.diff_state.diff_target = Some(DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(files[start_ix].path.clone()),
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1024.0), px(420.0)));
    draw_and_drain_test_window(cx);

    let initial_offset_y = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        uniform_list_offset(&pane.commit_files_scroll).y
    });
    assert_eq!(
        initial_offset_y,
        px(0.0),
        "expected the commit-details file list to start at the top"
    );

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, 1, window, cx)
        })
    });
    assert!(
        moved,
        "expected commit-details adjacent navigation to succeed"
    );
    draw_and_drain_test_window(cx);

    let offset_y = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        uniform_list_offset(&pane.commit_files_scroll).y
    });
    assert!(
        offset_y < px(0.0),
        "expected commit-details file navigation to scroll the selected row into view (offset_y={offset_y:?})",
    );
}

#[gpui::test]
fn commit_details_text_input_f4_navigates_files_without_stealing_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7052);
    let commit_id = CommitId("1122334455667788".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_details_input_nav",
        std::process::id()
    ));
    let files = vec![
        CommitFileChange {
            path: std::path::PathBuf::from("src/commit_details/first.rs"),
            kind: FileStatusKind::Modified,
            is_submodule: false,
        },
        CommitFileChange {
            path: std::path::PathBuf::from("src/commit_details/second.rs"),
            kind: FileStatusKind::Modified,
            is_submodule: false,
        },
    ];

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id.clone(),
        message: "subject".into(),
        committed_at: "2026-04-14 12:00:00 +0300".into(),
        parent_ids: vec![],
        files: files.clone(),
    }));
    repo.diff_state.diff_target = Some(DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(files[0].path.clone()),
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_details_sha_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, files[1].path.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_commit_diff_target_path(cx, &view),
        Some(files[1].path.clone()),
        "expected F4 from commit-details text input to select the next commit file"
    );
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-details SHA input to keep focus after F4 navigation"
        );
    });
}

#[gpui::test]
fn commit_message_text_input_f3_prefers_diff_search_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7053);
    let commit_id = CommitId("8899aabbccddeeff".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_search_nav",
        std::process::id()
    ));
    let hunk_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![gitcomet_core::domain::FileStatus {
                path: hunk_path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: hunk_path.clone(),
        area: DiffArea::Unstaged,
    });
    repo.diff_state.diff = Loadable::Ready(
        simple_hunk_diff(DiffTarget::WorkingTree {
            path: hunk_path,
            area: DiffArea::Unstaged,
        })
        .into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_matches = vec![3, 5];
                pane.diff_search_match_ix = Some(0);
                cx.notify();
            });
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let root = view.read(app);
        assert_eq!(
            root.main_pane.read(app).diff_search_match_ix,
            Some(1),
            "expected F3 from commit-message input to advance the active diff search match"
        );
        let focus = root
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-message input to keep focus after F3 search navigation"
        );
    });
}

#[gpui::test]
fn commit_message_text_input_f2_prefers_previous_diff_search_match(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70531);
    let commit_id = CommitId("8899aabbccddef00".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_search_prev",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_matches = vec![3, 5];
                pane.diff_search_match_ix = Some(1);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let root = view.read(app);
        assert_eq!(
            root.main_pane.read(app).diff_search_match_ix,
            Some(0),
            "expected F2 from commit-message input to move to the previous diff search match"
        );
        let focus = root
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-message input to keep focus after F2 search navigation"
        );
    });
}

#[gpui::test]
fn commit_message_text_input_secondary_enter_commits_staged_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(705315);
    let commit_id = CommitId("8899aabbccddef10".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_submit_shortcut",
        std::process::id()
    ));
    let staged_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![gitcomet_core::domain::FileStatus {
                path: staged_path,
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }],
            unstaged: vec![],
        }
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("hello shortcut".to_string(), cx);
                });
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("secondary-enter");
    draw_and_drain_test_window(cx);

    wait_until(cx, "commit to be in flight", |cx| {
        cx.update(|_window, app| -> bool {
            let root = view.read(app);
            let snapshot = root.store.snapshot();
            snapshot.repos.iter().any(|repo| repo.commit_in_flight == 1)
        })
    });

    cx.update(|window, app| {
        let root = view.read(app);
        let snapshot = root.store.snapshot();
        let repo = snapshot
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .expect("expected repo in store snapshot");
        assert_eq!(
            repo.commit_in_flight, 1,
            "expected secondary-enter from the commit message input to dispatch a commit"
        );
        let focus = root
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-message input to keep focus after secondary-enter commit"
        );
        assert_eq!(
            root.details_pane
                .read(app)
                .commit_message_input
                .read(app)
                .text(),
            "",
            "expected secondary-enter commit to clear the commit message input"
        );
    });
}

#[gpui::test]
fn commit_message_text_input_change_navigation_shortcuts_move_diff_without_stealing_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70532);
    let commit_id = CommitId("8899aabbccddef11".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_change_nav",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        three_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    wait_for_main_pane_condition(
        cx,
        &view,
        "diff rows for text-input change navigation",
        |pane| pane.diff_visible_len() > 0,
        |pane| {
            format!(
                "diff_visible_len={} diff_target={:?}",
                pane.diff_visible_len(),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone())
            )
        },
    );

    set_diff_selection_anchor(cx, &view, None);
    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let first_change = diff_selection_anchor(cx, &view)
        .expect("expected F7 from commit-message input to navigate to the first diff change");

    set_diff_selection_anchor(cx, &view, Some(first_change));
    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let second_change = diff_selection_anchor(cx, &view)
        .expect("expected F7 from commit-message input to reach the second diff change");
    assert!(
        second_change > first_change,
        "expected a later diff change target after the second F7 navigation"
    );

    set_diff_selection_area(
        cx,
        &view,
        Some(first_change),
        Some((first_change, second_change)),
    );
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let third_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 from a selected diff area to reach the third diff change");
    assert!(
        third_change > second_change,
        "expected F3 to continue after the selected diff area"
    );
    assert_eq!(
        diff_selection_range(cx, &view),
        Some((third_change, third_change)),
        "expected F3 to replace the selected diff area with the target change"
    );

    set_diff_selection_area(
        cx,
        &view,
        Some(third_change),
        Some((second_change, third_change)),
    );
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 to continue before the selected diff area"
    );
    assert_eq!(
        diff_selection_range(cx, &view),
        Some((first_change, first_change)),
        "expected F2 to replace the selected diff area with the target change"
    );

    set_diff_text_selection_on_row(cx, &view, second_change);
    assert!(
        diff_text_has_selection(cx, &view),
        "expected test setup to create a diff text selection"
    );
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 from commit-message input to fall back to the previous diff change when search is inactive"
    );
    assert!(
        !diff_text_has_selection(cx, &view),
        "expected F2 to clear the active diff text selection"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after F2 change navigation"
    );

    set_diff_text_selection_on_row(cx, &view, second_change);
    assert!(
        diff_text_has_selection(cx, &view),
        "expected test setup to create a diff text selection"
    );
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(third_change),
        "expected F3 from commit-message input to continue after the selected diff text"
    );
    assert!(
        !diff_text_has_selection(cx, &view),
        "expected F3 to clear the active diff text selection"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after F3 change navigation"
    );

    set_diff_selection_anchor(cx, &view, Some(second_change));
    cx.simulate_keystrokes("shift-f7");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected Shift-F7 from commit-message input to navigate to the previous diff change"
    );

    set_diff_selection_anchor(cx, &view, Some(second_change));
    cx.simulate_keystrokes("alt-up");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected Alt-Up from commit-message input to navigate to the previous diff change"
    );

    set_diff_selection_anchor(cx, &view, None);
    cx.simulate_keystrokes("alt-down");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected Alt-Down from commit-message input to navigate to the next diff change"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after change-navigation shortcuts"
    );
}

#[gpui::test]
fn create_branch_popover_text_input_f4_navigates_diff_without_closing_popover(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7054);
    let commit_id = CommitId("0102030405060708".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_create_branch_f4",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![
                gitcomet_core::domain::FileStatus {
                    path: first.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: second.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
            ],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: first.clone(),
        area: DiffArea::Unstaged,
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranch,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to hold focus before navigation"
        );
    });

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected create-branch popover to remain open after F4 diff navigation"
    );
    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 from create-branch input to select the next diff target"
    );
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to keep focus after F4 navigation"
        );
    });
}

#[gpui::test]
fn create_branch_popover_text_input_f1_navigates_previous_diff_without_closing_popover(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70541);
    let commit_id = CommitId("0102030405060718".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_create_branch_f1",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &second,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranch,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to hold focus before previous-file navigation"
        );
    });

    cx.simulate_keystrokes("f1");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, first.as_path());
    sync_store_snapshot(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected create-branch popover to remain open after F1 diff navigation"
    );
    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected F1 from create-branch input to select the previous diff target"
    );
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to keep focus after F1 navigation"
        );
    });
}

#[gpui::test]
fn diff_search_secondary_f_selects_existing_query(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70540);
    let commit_id = CommitId("1122334455667740".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_secondary_f_selects",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let query = "needle";

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = query.into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query, cx));
                let focus = pane.diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let pane = view.read(app).main_pane.read(app);
        let input = pane.diff_search_input.read(app);
        assert!(
            input.focus_handle().is_focused(window),
            "expected secondary-f to focus the diff search input"
        );
        assert_eq!(
            input.selected_range(),
            0..query.len(),
            "expected secondary-f to select the whole existing diff search query"
        );
    });
}

#[gpui::test]
fn diff_search_input_accepts_spaces_without_staging_file(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70546);
    let commit_id = CommitId("1122334455667746".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_space",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second],
        &first,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.simulate_input("needle one");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "needle one");
        assert_eq!(pane.diff_search_input.read(app).text(), "needle one");
    });

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected Space from the diff search input to avoid staging or advancing the diff target"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected the diff search input to keep focus after Space"
    );
}

#[gpui::test]
fn diff_search_close_clears_query_and_input(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70547);
    let commit_id = CommitId("1122334455667747".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_close_clears",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.simulate_input("needle one");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "needle one");
        assert_eq!(pane.diff_search_input.read(app).text(), "needle one");
    });

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(!pane.diff_search_active);
        assert_eq!(pane.diff_search_query.as_ref(), "");
        assert_eq!(pane.diff_search_input.read(app).text(), "");
        assert!(pane.diff_search_matches.is_empty());
        assert_eq!(pane.diff_search_match_ix, None);
    });
}

#[gpui::test]
fn whitespace_only_diff_search_query_recomputes_on_whitespace_mode_change(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = " ".into();
                pane.diff_search_matches = vec![7];
                pane.diff_search_match_ix = Some(0);
                pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.is_empty(),
            "expected whitespace-only queries to refresh instead of leaving stale matches behind"
        );
        assert_eq!(
            pane.diff_search_match_ix, None,
            "expected recomputing an empty result set to clear the active diff search match"
        );
    });
}

#[gpui::test]
fn diff_search_overlay_does_not_reflow_action_bar_or_content(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70545);
    let commit_id = CommitId("1122334455667745".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_overlay_layout",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                let focus = pane.diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("diff_search_overlay").is_none(),
        "expected diff search overlay to be absent before search opens"
    );
    let close_before = cx
        .debug_bounds("diff_close")
        .expect("expected diff close button before search opens");
    let content_before = cx
        .debug_bounds("diff_body_container")
        .expect("expected diff body before search opens");

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    let close_after = cx
        .debug_bounds("diff_close")
        .expect("expected diff close button after search opens");
    let content_after = cx
        .debug_bounds("diff_body_container")
        .expect("expected diff body after search opens");
    assert!(
        cx.debug_bounds("diff_search_overlay").is_some(),
        "expected diff search overlay after secondary-f"
    );
    let overlay_empty_query = cx
        .debug_bounds("diff_search_overlay")
        .expect("expected diff search overlay bounds after secondary-f");
    let input_slot_empty_query = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot bounds after secondary-f");
    let match_label_empty_query = cx
        .debug_bounds("diff_search_match_label")
        .expect("expected diff search match label bounds after secondary-f");
    assert_eq!(
        close_after, close_before,
        "expected diff close button bounds to remain stable when search opens"
    );
    assert_eq!(
        content_after.top(),
        content_before.top(),
        "expected diff content top to remain stable when search opens"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("new", cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    wait_for_diff_search_debounce(cx);

    let overlay_with_matches = cx
        .debug_bounds("diff_search_overlay")
        .expect("expected diff search overlay bounds after entering a query");
    let input_slot_with_matches = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot bounds after entering a query");
    let match_label_with_matches = cx
        .debug_bounds("diff_search_match_label")
        .expect("expected diff search match label bounds after entering a query");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            2,
            "expected the query to switch the search label to a match count"
        );
    });
    assert_eq!(
        overlay_with_matches.size.width, overlay_empty_query.size.width,
        "expected diff search overlay width to stay stable when the status text changes"
    );
    assert_eq!(
        input_slot_with_matches.origin.x, input_slot_empty_query.origin.x,
        "expected diff search input slot x position to stay stable when the status text changes"
    );
    assert_eq!(
        input_slot_with_matches.size.width, input_slot_empty_query.size.width,
        "expected diff search input slot width to stay stable when the status text changes"
    );
    assert_eq!(
        match_label_with_matches.size.width, match_label_empty_query.size.width,
        "expected diff search status label width to stay stable when the status text changes"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_search_overlay").is_none(),
        "expected Escape to remove diff search overlay"
    );

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    let search_close_bounds = cx
        .debug_bounds("diff_search_close")
        .expect("expected diff search close button after reopening search");
    cx.simulate_click(search_close_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_search_overlay").is_none(),
        "expected search close button to remove diff search overlay"
    );
}

#[gpui::test]
fn reveal_whitespace_toggle_invalidates_wrapped_diff_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_word_wrap = true;
                pane.diff_wrap_visible_cache_key = Some(DiffWrapVisibleCacheKey {
                    source_len: 1,
                    diff_view: DiffViewMode::Inline,
                    is_file_view: false,
                    collapsed_projection_active: false,
                    projection_rev: 0,
                    diff_cache_rev: 0,
                    file_diff_cache_seq: 0,
                    inline_columns: 8,
                    split_columns: 8,
                    reveal_whitespace_chars: false,
                });
                pane.diff_wrap_visible_rows = vec![DiffWrapVisualRow {
                    source_visible_ix: 0,
                    wrap_ix: 0,
                    primary_range: rows::DiffWrapByteRange { start: 0, end: 4 },
                    secondary_range: rows::DiffWrapByteRange::default(),
                }];
                pane.set_diff_reveal_whitespace_chars(true, cx);
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_wrap_visible_cache_key, None,
            "expected reveal-whitespace changes to invalidate wrapped-row cache keys"
        );
        assert!(
            pane.diff_wrap_visible_rows.is_empty(),
            "expected reveal-whitespace changes to drop cached wrapped rows"
        );
    });
}

#[gpui::test]
fn diff_search_input_slot_grows_for_multiline_query(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70548);
    let commit_id = CommitId("1122334455667748".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_multiline_grows",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));
    focus_diff_search_input(cx, &view);
    draw_and_drain_test_window(cx);

    let single_line = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot for a single-line query");
    assert!(
        single_line.size.height <= px(30.0),
        "expected compact one-line diff search slot height; got {single_line:?}"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("alpha\nbeta", cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let multiline = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot for a multiline query");
    assert_eq!(
        multiline.size.width, single_line.size.width,
        "expected multiline diff search to preserve the input slot width"
    );
    assert!(
        multiline.size.height > single_line.size.height + px(8.0),
        "expected multiline diff search slot to grow; single={single_line:?} multiline={multiline:?}"
    );
}

#[gpui::test]
fn diff_search_input_slot_caps_tall_multiline_query(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70549);
    let commit_id = CommitId("1122334455667749".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_multiline_caps",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));
    focus_diff_search_input(cx, &view);

    let tall_query = (0..40)
        .map(|ix| format!("needle_{ix}"))
        .collect::<Vec<_>>()
        .join("\n");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(tall_query.clone(), cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let tall_slot = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot for a tall multiline query");
    let max_height = px(super::super::COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX);
    assert!(
        tall_slot.size.height <= max_height + px(1.0),
        "expected tall diff search slot to cap at {max_height:?}; got {tall_slot:?}"
    );

    let max_scroll_y = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_scroll
            .max_offset()
            .y
    });
    assert!(
        max_scroll_y > px(0.0),
        "expected tall diff search query to be vertically scrollable"
    );
}

#[gpui::test]
fn diff_action_menu_contains_whitespace_setting(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70546);
    let commit_id = CommitId("1122334455667746".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_action_menu",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("diff_whitespace_mode_header").is_none(),
        "expected whitespace setting to be removed from the diff action bar"
    );
    let menu_bounds = cx
        .debug_bounds("diff_action_menu")
        .expect("expected diff action menu button in the diff action bar");
    let close_bounds = cx
        .debug_bounds("diff_close")
        .expect("expected diff close button in the diff action bar");
    assert!(
        menu_bounds.right() <= close_bounds.left(),
        "expected diff action menu button to be before the close button"
    );

    focus_diff_panel(cx, &view);
    cx.simulate_click(menu_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let popover_kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert_eq!(
        popover_kind,
        Some(PopoverKind::DiffActionMenu),
        "expected clicking the cog to open the diff action menu"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected opening the diff action menu to move focus away from the diff panel"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(
        !popover_is_open(cx, &view),
        "expected Escape to close the diff action menu"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected closing the diff action menu to restore diff-panel focus"
    );

    cx.simulate_click(menu_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        Some(PopoverKind::DiffActionMenu),
        "expected reopening the cog menu to show diff actions"
    );

    let whitespace_bounds = cx
        .debug_bounds("context_menu_show_whitespace_changes")
        .expect("expected whitespace setting to be rendered in the diff action menu");
    assert!(
        cx.debug_bounds("context_menu_reveal_whitespace_characters")
            .is_some(),
        "expected reveal whitespace characters setting to be rendered in the diff action menu"
    );
    assert!(
        cx.debug_bounds("context_menu_word_wrap").is_some(),
        "expected word wrap setting to be rendered in the diff action menu"
    );
    cx.simulate_click(whitespace_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let whitespace_mode =
        cx.update(|_window, app| crate::view::test_support::diff_whitespace_mode(view.read(app)));
    assert_eq!(
        whitespace_mode,
        DiffWhitespaceMode::Ignore,
        "expected selecting the whitespace entry to toggle the global diff whitespace mode"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected the diff action menu to remain open after selecting whitespace mode"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting whitespace mode to restore diff-panel focus"
    );
    assert!(
        cx.debug_bounds("context_menu_show_whitespace_changes")
            .is_some(),
        "expected the whitespace setting to remain visible after toggling"
    );

    let reveal_bounds = cx
        .debug_bounds("context_menu_reveal_whitespace_characters")
        .expect("expected reveal whitespace setting to remain visible");
    cx.simulate_click(reveal_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.update(
            |_window, app| crate::view::test_support::diff_reveal_whitespace_chars(view.read(app))
        ),
        "expected selecting reveal whitespace to toggle the global reveal preference"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected the diff action menu to remain open after selecting reveal whitespace"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting reveal whitespace to restore diff-panel focus"
    );

    let word_wrap_bounds = cx
        .debug_bounds("context_menu_word_wrap")
        .expect("expected word wrap setting to remain visible");
    cx.simulate_click(word_wrap_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.update(|_window, app| crate::view::test_support::diff_word_wrap(view.read(app))),
        "expected selecting word wrap to toggle the global word wrap preference"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected the diff action menu to remain open after selecting word wrap"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting word wrap to restore diff-panel focus"
    );
}

#[gpui::test]
fn diff_view_toolbar_toggle_restores_diff_panel_focus(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70547);
    let commit_id = CommitId("1122334455667747".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_view_toggle_focus",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    focus_commit_message_input(cx, &view);
    let split_bounds = cx
        .debug_bounds("diff_split")
        .expect("expected split diff toolbar button");
    cx.simulate_click(split_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Split,
        "expected clicking Split to switch diff view"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected clicking Split to restore diff-panel focus"
    );
}

#[gpui::test]
fn diff_search_query_edit_selects_first_match_and_updates_count(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70541);
    let commit_id = CommitId("1122334455667741".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_query_edit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("new", cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert!(
            pane.diff_search_matches.is_empty(),
            "expected match recompute to wait for the search debounce"
        );
    });
    wait_for_diff_search_debounce(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert_eq!(
            pane.diff_search_matches.len(),
            2,
            "expected the edited query to find both matching diff rows"
        );
        assert_eq!(
            pane.diff_search_match_ix,
            Some(0),
            "expected query edits to select the first match"
        );
        assert_eq!(
            pane.diff_selection_anchor,
            pane.diff_search_matches.first().copied(),
            "expected query edits to scroll/anchor to the first match"
        );
    });
}

#[gpui::test]
fn diff_search_navigation_keys_flush_pending_query_recompute(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70543);
    let commit_id = CommitId("1122334455667743".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_nav_flush",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                let focus = pane.diff_search_input.read(cx).focus_handle();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let set_pending_query = |cx: &mut gpui::VisualTestContext, query: &str| {
        let query = query.to_string();
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_search_input
                        .update(cx, |input, cx| input.set_text(query.clone(), cx));
                    cx.notify();
                });
            });
            let _ = window.draw(app);
        });
        draw_and_drain_test_window(cx);
    };

    set_pending_query(cx, "new");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert!(
            pane.diff_search_matches.is_empty(),
            "expected F3 to navigate before the debounce has recomputed matches"
        );
    });
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert_eq!(pane.diff_search_matches.len(), 2);
        assert_eq!(
            pane.diff_search_match_ix,
            Some(1),
            "expected F3 to flush the pending search before advancing"
        );
    });

    set_pending_query(cx, "old");
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "old");
        assert_eq!(pane.diff_search_matches.len(), 2);
        assert_eq!(
            pane.diff_search_match_ix,
            Some(1),
            "expected F2 to flush the pending search before moving backward"
        );
    });

    set_pending_query(cx, "unchanged");
    cx.simulate_keystrokes("enter");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "unchanged");
        assert_eq!(pane.diff_search_matches.len(), 1);
        assert_eq!(
            pane.diff_search_match_ix,
            Some(0),
            "expected Enter to flush the pending search before navigating"
        );
    });
}

#[gpui::test]
fn diff_search_preserve_current_scrolls_when_matches_first_appear(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70542);
    let commit_id = CommitId("1122334455667742".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_first_matches",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                pane.diff_search_query = "new".into();
                pane.diff_search_matches.clear();
                pane.diff_search_match_ix = None;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_scroll.0.borrow_mut().deferred_scroll_to_item = None;
                pane.diff_search_recompute_matches();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let first_match = pane
            .diff_search_matches
            .first()
            .copied()
            .expect("expected query to find diff search matches");
        assert_eq!(
            pane.diff_search_match_ix,
            Some(0),
            "expected first newly discovered match to become active"
        );
        assert_eq!(
            pane.diff_selection_anchor,
            Some(first_match),
            "expected first newly discovered match to be scrolled into view"
        );
        assert_eq!(
            pane.diff_selection_range,
            Some((first_match, first_match)),
            "expected scroll-to-match to update the diff selection range"
        );
    });
}

#[gpui::test]
fn diff_search_passive_visible_refresh_preserves_scroll_and_match(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70544);
    let commit_id = CommitId("1122334455667744".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_passive_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        searchable_scroll_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                pane.diff_search_query = "needle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("needle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "diff search fixture matches",
        |pane| pane.diff_search_matches.len() >= 2,
        |pane| {
            format!(
                "matches={:?} offset={:?} deferred_scroll={:?}",
                pane.diff_search_matches,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().deferred_scroll_to_item,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let last_match_ix = pane.diff_search_matches.len() - 1;
                pane.diff_search_match_ix = Some(last_match_ix);
                set_uniform_list_offset(&pane.diff_scroll, gpui::point(px(0.0), px(-120.0)));
                pane.diff_scroll.0.borrow_mut().deferred_scroll_to_item = None;
                cx.notify();
            });
        });
    });

    let (before_offset, expected_match_ix) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.diff_scroll.0.borrow().base_handle.offset(),
            pane.diff_search_match_ix,
        )
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_visible_cache_len = usize::MAX;
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_match_ix, expected_match_ix,
            "expected passive visible-index refresh to preserve the active search match"
        );
        assert_eq!(
            pane.diff_scroll.0.borrow().base_handle.offset(),
            before_offset,
            "expected passive visible-index refresh not to move the diff scroll position"
        );
        assert!(
            pane.diff_scroll
                .0
                .borrow()
                .deferred_scroll_to_item
                .is_none(),
            "expected passive visible-index refresh not to schedule a diff scroll"
        );
    });
}

#[gpui::test]
fn diff_search_text_input_file_navigation_preserves_focus_and_last_file_boundary(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70542);
    let commit_id = CommitId("1122334455667700".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_file_nav",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &second,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to hold focus before adjacent-file navigation"
    );

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second.clone()),
        "expected F4 from diff-search input at the last file to leave the diff target unchanged"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to keep focus after a no-op F4 navigation"
    );

    cx.simulate_keystrokes("f1");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, first.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected F1 from diff-search input to select the previous diff target"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to keep focus after F1 navigation"
    );
}

#[gpui::test]
fn conflict_diff_search_input_change_navigation_preserves_focus(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70543);
    let commit_id = CommitId("1122334455667711".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_input_nav",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/conflicted.rs");

    let repo = simple_conflict_repo(repo_id, &workdir, &commit_id, path.as_path());
    apply_state(cx, &view, app_state_with_active_repo(repo));
    wait_for_main_pane_condition(
        cx,
        &view,
        "conflict resolver state for text-input navigation",
        |pane| {
            pane.conflict_resolver.path.as_deref() == Some(path.as_path())
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .map(|marker| marker.conflict_ix)
                    .max()
                    .is_some_and(|ix| ix >= 1)
        },
        |pane| {
            format!(
                "path={:?} markers={} active_conflict={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolver.active_conflict,
            )
        },
    );
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
            });
        });
        let _ = window.draw(app);
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "two-way conflict navigation entries for text-input navigation",
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::TwoWayDiff
                && pane.conflict_nav_entries().len() >= 2
        },
        |pane| {
            format!(
                "view_mode={:?} nav_entries={:?}",
                pane.conflict_resolver.view_mode,
                pane.conflict_nav_entries(),
            )
        },
    );
    focus_diff_search_input(cx, &view);

    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to hold focus before conflict navigation"
    );
    assert_eq!(
        active_conflict_ix(cx, &view),
        0,
        "expected the first conflict to be active before navigation"
    );

    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let first_anchor = conflict_navigation_anchor(cx, &view)
        .expect("expected F7 from diff search input to set a navigation anchor");

    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let second_anchor = conflict_navigation_anchor(cx, &view)
        .expect("expected the second F7 to keep a conflict navigation anchor");
    assert!(
        second_anchor > first_anchor,
        "expected repeated F7 from diff search input to move to a later conflict"
    );
    assert_eq!(
        active_conflict_ix(cx, &view),
        1,
        "expected repeated F7 from diff search input to advance to the second conflict"
    );

    cx.simulate_keystrokes("shift-f7");
    draw_and_drain_test_window(cx);

    assert_eq!(
        active_conflict_ix(cx, &view),
        0,
        "expected Shift-F7 from diff search input to return to the previous conflict"
    );
    assert!(
        conflict_navigation_anchor(cx, &view).is_some_and(|anchor| anchor < second_anchor),
        "expected Shift-F7 from diff search input to move the navigation anchor backward"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to keep focus after conflict navigation shortcuts"
    );
}

#[gpui::test]
fn commit_message_text_input_secondary_f_activates_diff_search(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7055);
    let commit_id = CommitId("1111222233334444".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_secondary_f",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    let query = "needle";
    cx.update(|window, app| {
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = query.into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query.to_string(), cx));
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    assert!(
        diff_search_active(cx, &view),
        "expected secondary-f from commit-message input to activate diff search when a diff is visible"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected secondary-f from commit-message input to focus diff search when a diff is visible"
    );
    assert!(
        !commit_message_input_is_focused(cx, &view),
        "expected secondary-f from commit-message input to move focus to diff search"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_input.read(app).selected_range(),
            0..query.len(),
            "expected secondary-f to select the full existing diff search query"
        );
    });
}

#[gpui::test]
fn commit_message_text_input_secondary_f_without_visible_diff_is_noop(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70551);
    let commit_id = CommitId("1111222233334445".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_secondary_f_no_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff_target = None;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    cx.update(|window, app| {
        crate::app::bind_app_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    assert!(
        !diff_search_active(cx, &view),
        "expected secondary-f to avoid activating diff search when no diff is visible"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected secondary-f with no visible diff to leave focus unchanged"
    );
}

#[gpui::test]
fn commit_message_text_input_view_and_whitespace_shortcuts_do_not_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7056);
    let commit_id = CommitId("1111222233335555".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_view_toggle",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    install_global_diff_shortcut_fallback_for_test(cx);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.reveal_whitespace_chars = false;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-i");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Split,
        "expected Alt-I from commit-message input to avoid switching the diff view"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-s");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Inline,
        "expected Alt-S from commit-message input to avoid switching the diff view"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.reveal_whitespace_chars = false;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-w");
    draw_and_drain_test_window(cx);
    assert!(
        !reveal_whitespace_chars(cx, &view),
        "expected Alt-W from commit-message input to avoid toggling whitespace visibility"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Alt-I/Alt-S/Alt-W"
    );
}

#[gpui::test]
fn commit_message_text_input_space_does_not_stage_or_advance_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7058);
    let commit_id = CommitId("1111222233337777".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_space",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second],
        &first,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    install_global_diff_shortcut_fallback_for_test(cx);

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    std::thread::sleep(Duration::from_millis(20));
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected Space from commit-message input to avoid staging or advancing the diff selection"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Space"
    );
}

#[gpui::test]
fn diff_editor_staging_context_menu_restores_diff_panel_focus_for_f4(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70560);
    let commit_id = CommitId("abcdef0011223344".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_editor_stage_focus",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    open_popover_for_test(
        cx,
        &view,
        PopoverKind::DiffEditorMenu {
            repo_id,
            area: DiffArea::Unstaged,
            path: Some(first.clone()),
            hunk_patch: Some("diff --git a/src/first.rs b/src/first.rs\n".into()),
            hunks_count: 1,
            lines_patch: Some("diff --git a/src/first.rs b/src/first.rs\n".into()),
            discard_lines_patch: None,
            lines_count: 1,
            copy_text: None,
            copy_target: None,
        },
    );

    assert!(
        popover_is_open(cx, &view),
        "expected the diff editor context menu to open"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected the diff editor context menu to take focus"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::ApplyIndexPatch {
                        repo_id,
                        patch: "diff --git a/src/first.rs b/src/first.rs\n".into(),
                        reverse: false,
                    },
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    assert!(
        !popover_is_open(cx, &view),
        "expected staging from the diff editor context menu to close the menu"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected staging from the diff editor context menu to restore diff-panel focus"
    );

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 to navigate immediately after staging from the diff editor context menu"
    );
}

#[gpui::test]
fn non_text_context_menu_focus_f4_uses_app_level_diff_navigation(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70561);
    let commit_id = CommitId("abcdef0011223355".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_menu_f4",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_for_test(cx);
    open_change_tracking_settings_popover(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking context menu to remain open before F4"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected context-menu focus to exercise the app-level shortcut fallback"
    );

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 from non-text context-menu focus to select the next diff target"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected app-level F4 navigation not to dismiss an unrelated context menu"
    );
}

#[gpui::test]
fn non_text_context_menu_focus_f2_f3_use_diff_search_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70562);
    let commit_id = CommitId("abcdef0011223366".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_menu_f2_f3",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_for_test(cx);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_matches = vec![3, 5];
                pane.diff_search_match_ix = Some(0);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    open_change_tracking_settings_popover(cx, &view);

    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected context-menu focus to exercise the app-level search shortcut fallback"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_match_ix),
        Some(1),
        "expected F3 from non-text context-menu focus to advance the diff search match"
    );

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_match_ix),
        Some(0),
        "expected F2 from non-text context-menu focus to move to the previous diff search match"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected app-level F2/F3 navigation not to dismiss an unrelated context menu"
    );
}

#[gpui::test]
fn detached_window_focus_uses_global_diff_shortcut_fallback(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70563);
    let commit_id = CommitId("abcdef0011223377".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_detached_focus_global_shortcuts",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: first.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    repo.diff_state.diff_rev = repo.diff_state.diff_rev.wrapping_add(1);
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_detached_window_focus(cx);
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected detached focus to avoid the rendered diff-panel key path"
    );

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    assert!(
        diff_search_active(cx, &view),
        "expected secondary-f from detached focus to activate diff search"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected secondary-f from detached focus to focus diff search"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.reveal_whitespace_chars = false;
                pane.diff_search_active = false;
                pane.diff_search_matches.clear();
                pane.diff_search_match_ix = None;
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("alt-i");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Inline,
        "expected Alt-I from detached focus to switch to inline diff view"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("alt-s");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Split,
        "expected Alt-S from detached focus to switch to split diff view"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("alt-w");
    draw_and_drain_test_window(cx);
    assert!(
        reveal_whitespace_chars(cx, &view),
        "expected Alt-W from detached focus to toggle whitespace visibility"
    );

    set_diff_selection_anchor(cx, &view, None);
    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let first_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 from detached focus to navigate to the first diff change");

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let second_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 from detached focus to navigate to the second diff change");
    assert!(
        second_change > first_change,
        "expected repeated F3 from detached focus to move forward through diff changes"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 from detached focus to move back to the previous diff change"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 from detached focus to select the next diff target"
    );
}

#[gpui::test]
fn detached_window_focus_space_stages_and_advances_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70564);
    let commit_id = CommitId("abcdef0011223388".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_detached_focus_space",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_detached_window_focus(cx);

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected Space from detached focus to stage the active file and advance the diff target"
    );
}

#[gpui::test]
fn detached_window_focus_conflict_quick_pick_uses_global_diff_shortcut_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70565);
    let commit_id = CommitId("abcdef0011223399".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_detached_focus_conflict_pick",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/conflicted.rs");
    let repo = simple_conflict_repo(repo_id, &workdir, &commit_id, path.as_path());

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    wait_for_main_pane_condition(
        cx,
        &view,
        "conflict resolver state for detached-focus quick pick",
        |pane| {
            pane.conflict_resolver.path.as_deref() == Some(path.as_path())
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .map(|marker| marker.conflict_ix)
                    .max()
                    .is_some_and(|ix| ix >= 1)
        },
        |pane| {
            format!(
                "path={:?} markers={} active_conflict={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolver.active_conflict,
            )
        },
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("b");
    draw_and_drain_test_window(cx);

    assert_eq!(
        active_conflict_ix(cx, &view),
        1,
        "expected conflict quick-pick key from detached focus to pick the first conflict and advance"
    );
}

#[gpui::test]
fn switching_diff_content_mode_restores_diff_panel_focus_for_change_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70566);
    let commit_id = CommitId("abcdef00112233aa".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_content_focus_switch",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
        .into(),
    );
    repo.diff_state.diff_rev = repo.diff_state.diff_rev.wrapping_add(1);
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    focus_diff_panel(cx, &view);
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected the diff panel to be focused before opening diff mode settings"
    );

    open_popover_for_test(cx, &view, PopoverKind::DiffContentModeSettings);
    assert!(
        popover_is_open(cx, &view),
        "expected the diff mode settings popover to open"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected the diff mode settings popover to move focus away from the diff panel"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::SetDiffContentMode {
                        mode: DiffContentMode::Collapsed,
                    },
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed diff content mode with navigable changes",
        |pane| {
            pane.diff_content_mode == DiffContentMode::Collapsed
                && pane.diff_nav_entries().len() >= 2
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_visible_len(),
                pane.diff_nav_entries(),
            )
        },
    );

    assert!(
        !popover_is_open(cx, &view),
        "expected selecting a diff mode to close the popover"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting a diff mode to restore diff-panel focus"
    );
    assert_eq!(
        cx.update(|_window, app| crate::view::test_support::diff_content_mode(view.read(app))),
        DiffContentMode::Collapsed,
        "expected selecting the collapsed entry to update the global diff content mode"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let next_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 after closing diff mode settings to navigate to a change");

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    let previous_change = diff_selection_anchor(cx, &view)
        .expect("expected F2 after closing diff mode settings to navigate to a change");
    assert!(
        previous_change < next_change,
        "expected F2 after closing diff mode settings to refresh and move to the previous change"
    );
}

#[gpui::test]
fn switching_change_tracking_view_restores_diff_panel_focus_for_adjacent_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(705);
    let commit_id = CommitId("1234567812345678".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_change_tracking_focus_switch",
        std::process::id()
    ));
    let untracked_a = std::path::PathBuf::from("new-a.txt");
    let tracked = std::path::PathBuf::from("src/lib.rs");
    let untracked_b = std::path::PathBuf::from("new-b.txt");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![
                gitcomet_core::domain::FileStatus {
                    path: untracked_a.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked,
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: untracked_b.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
            ],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: untracked_a.clone(),
        area: DiffArea::Unstaged,
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected the diff panel to be focused before opening change-tracking settings"
    );

    open_change_tracking_settings_popover(cx, &view);
    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings popover to open"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected opening the change-tracking settings popover to move focus away from the diff panel"
    );

    cx.simulate_keystrokes("s");
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.update(|_window, app| {
            crate::view::test_support::change_tracking_view(view.read(app))
        }),
        ChangeTrackingView::SplitUntracked,
        "expected selecting the split view menu entry to update the change-tracking layout"
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the change-tracking settings popover to close after selecting split view"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected closing the change-tracking settings popover to restore diff-panel focus"
    );
    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(untracked_a),
        "expected the active diff target to stay selected after switching to split view"
    );

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, 1, window, cx)
        })
    });
    assert!(
        moved,
        "expected adjacent navigation to keep working immediately after switching to split view"
    );
}

#[gpui::test]
fn dismissing_change_tracking_settings_with_escape_restores_diff_panel_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(706);
    let commit_id = CommitId("8765432187654321".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_change_tracking_focus_escape",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![gitcomet_core::domain::FileStatus {
                path: path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }],
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path,
        area: DiffArea::Unstaged,
    });

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    open_change_tracking_settings_popover(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings popover to be open before dismissing it"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected the change-tracking settings popover to hold focus while it is open"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);

    assert!(
        !popover_is_open(cx, &view),
        "expected Escape to close the change-tracking settings popover"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected dismissing change-tracking settings to restore diff-panel focus"
    );
}

#[gpui::test]
fn ui_scale_picker_selection_updates_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(707);
    let commit_id = CommitId("1122334455667788".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ui_scale_picker",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::UiScalePicker,
                    point(px(72.0), px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected opening the UI scale picker to show a popover"
    );
    assert!(
        cx.debug_bounds("context_menu_125").is_some(),
        "expected the UI scale picker to expose a 125% menu item"
    );

    let zoom_125_bounds = cx
        .debug_bounds("context_menu_125")
        .expect("expected the 125% zoom entry to be rendered");
    cx.simulate_click(zoom_125_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let zoom_percent = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoom_percent, 125,
        "expected selecting 125% from the zoom picker to update the UI scale"
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the UI scale picker to close after selecting a zoom level"
    );
}

#[gpui::test]
fn bottom_status_bar_zoom_button_keeps_icon_at_default_scale_and_opens_picker(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(709);
    let commit_id = CommitId("9988776655443322".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_bottom_status_zoom_button",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("bottom_status_bar_zoom_icon").is_some(),
        "expected the bottom status bar zoom icon to be visible at the default scale"
    );

    let default_button_width = debug_width(cx, "bottom_status_bar_zoom");
    assert!(
        default_button_width < 40.0,
        "expected the default zoom button to stay icon-only (width={default_button_width})"
    );

    let zoom_button_bounds = cx
        .debug_bounds("bottom_status_bar_zoom")
        .expect("expected bottom status bar zoom button bounds");
    cx.simulate_click(zoom_button_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected clicking the bottom status bar zoom button to open the UI scale picker"
    );
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_125");

    let zoom_125_bounds = cx
        .debug_bounds("context_menu_125")
        .expect("expected the 125% zoom entry to be rendered");
    cx.simulate_click(zoom_125_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let zoom_percent = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoom_percent, 125,
        "expected selecting 125% from the zoom button picker to update the UI scale"
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the UI scale picker to close after selecting a zoom level from the bottom bar"
    );
    assert!(
        cx.debug_bounds("bottom_status_bar_zoom_icon").is_some(),
        "expected the bottom status bar zoom icon to remain visible after changing zoom"
    );

    let zoomed_button_width = debug_width(cx, "bottom_status_bar_zoom");
    assert!(
        zoomed_button_width > default_button_width + 10.0,
        "expected the non-default zoom button to grow to include its percent label (default={default_button_width}, zoomed={zoomed_button_width})"
    );
}

#[gpui::test]
fn shared_context_menu_rows_fill_the_popover_width(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(710);
    let commit_id = CommitId("1234432112344321".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_shared_context_menu_width",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    open_change_tracking_settings_popover(cx, &view);
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings popover to be open"
    );
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_combine_with_unstaged");
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_show_separate_untracked_block");
}

#[gpui::test]
fn context_menus_grow_wider_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(711);
    let commit_id = CommitId("2233445566778899".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_menu_zoom_width",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    open_change_tracking_settings_popover(cx, &view);
    draw_and_drain_test_window(cx);

    let default_width = debug_width(cx, "app_popover");
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_combine_with_unstaged");

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings context menu to remain open after zooming"
    );

    let zoomed_width = debug_width(cx, "app_popover");
    assert!(
        zoomed_width > default_width * 1.6,
        "expected the context menu to grow substantially with zoom (default={default_width}, zoomed={zoomed_width})"
    );
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_combine_with_unstaged");
}

#[gpui::test]
fn prompt_popovers_grow_wider_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(712);
    let commit_id = CommitId("3344556677889900".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_prompt_popover_zoom_width",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    open_popover_for_test(cx, &view, PopoverKind::CreateBranch);
    draw_and_drain_test_window(cx);

    let default_width = debug_width(cx, "app_popover");

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected the create-branch popover to remain open after zooming"
    );

    let zoomed_width = debug_width(cx, "app_popover");
    assert!(
        zoomed_width > default_width * 1.6,
        "expected the prompt popover to grow substantially with zoom (default={default_width}, zoomed={zoomed_width})"
    );
}

#[gpui::test]
fn history_horizontal_wheel_does_not_scroll_vertically(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(709);
    let commit_id = CommitId("8877665544332211".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_history_horizontal_wheel",
        std::process::id()
    ));
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    let commits = (0..160)
        .map(|ix| gitcomet_core::domain::Commit {
            id: CommitId(format!("{ix:040x}").into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: format!("Commit {ix:03}").into(),
            author: "Alice".into(),
            time: std::time::SystemTime::UNIX_EPOCH
                + Duration::from_secs(ix.try_into().unwrap_or(0)),
        })
        .collect();
    repo.log = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits,
            next_cursor: None,
        }
        .into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    let (history_bounds, max_offset_y) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app).history_view.read(app);
        let handle = pane.history_scroll.0.borrow().base_handle.clone();
        (handle.bounds(), handle.max_offset().y)
    });
    let position = history_bounds.center();
    assert!(
        max_offset_y > px(0.0),
        "expected history list to be vertically scrollable"
    );

    let offset_before = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
        ..Default::default()
    });
    draw_and_drain_test_window(cx);
    let offset_after_horizontal = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    assert_eq!(
        offset_after_horizontal.y, offset_before.y,
        "expected horizontal-only wheel scroll not to move history vertically"
    );

    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    draw_and_drain_test_window(cx);
    let offset_after_vertical = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    assert!(
        offset_after_vertical.y < offset_before.y - px(0.5),
        "expected vertical wheel scroll to continue moving history vertically"
    );
}

#[gpui::test]
fn ui_scale_ctrl_scroll_wheel_changes_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(708);
    let commit_id = CommitId("8877665544332211".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ui_scale_ctrl_scroll",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    let position = point(px(320.0), px(240.0));
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.0), px(120.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_and_drain_test_window(cx);

    let zoomed_in = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoomed_in, 110,
        "expected Ctrl/Cmd + wheel up to step the UI zoom to the next preset"
    );

    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_and_drain_test_window(cx);

    let zoomed_back_out = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoomed_back_out, 100,
        "expected Ctrl/Cmd + wheel down to step the UI zoom back to the previous preset"
    );
}

#[gpui::test]
fn ctrl_s_stages_current_file_and_advances_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70600);
    let commit_id = CommitId("abcdef00112233bb".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_s_stage",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-s");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected Ctrl+S to stage the active file and advance the diff target"
    );
}

#[gpui::test]
fn ctrl_s_stages_last_file_and_clears_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70601);
    let commit_id = CommitId("abcdef00112233cc".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_s_last_file",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-s");
    draw_and_drain_test_window(cx);
    wait_until(cx, "store diff target to clear after staging last file", |cx| {
        cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            let Some(repo_id) = snapshot.active_repo else {
                return false;
            };
            let Some(repo) = snapshot.repos.iter().find(|r| r.id == repo_id) else {
                return false;
            };
            repo.diff_state.diff_target.is_none()
        })
    });
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        None,
        "expected Ctrl+S on the last unstaged file to stage it and clear the diff target"
    );
}

#[gpui::test]
fn ctrl_shift_c_copies_current_file_path(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = crate::test_support::lock_clipboard_test();

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70602);
    let commit_id = CommitId("abcdef00112233dd".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_shift_c",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-shift-c");
    draw_and_drain_test_window(cx);

    let clipboard_text = cx.read_from_clipboard().and_then(|item| item.text());
    assert!(
        clipboard_text
            .as_ref()
            .is_some_and(|text| text.contains("src/lib.rs")),
        "expected Ctrl+Shift+C to copy the current file path to clipboard, got: {clipboard_text:?}"
    );
}

#[gpui::test]
fn ctrl_d_opens_discard_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70603);
    let commit_id = CommitId("abcdef00112233ee".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_d_discard",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-d");
    draw_and_drain_test_window(cx);

    let is_discard_confirm = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        matches!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::DiscardChangesConfirm { .. })
        )
    });
    assert!(
        is_discard_confirm,
        "expected Ctrl+D to open the DiscardChangesConfirm popover"
    );
}

#[gpui::test]
fn ctrl_h_opens_file_history_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70604);
    let commit_id = CommitId("abcdef00112233ff".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_h_history",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-h");
    draw_and_drain_test_window(cx);

    let is_file_history = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        matches!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::FileHistory { .. })
        )
    });
    assert!(
        is_file_history,
        "expected Ctrl+H to open the FileHistory popover"
    );
}

#[gpui::test]
fn ctrl_shortcuts_do_not_crash_without_diff_target(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = crate::test_support::lock_clipboard_test();

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70605);
    let commit_id = CommitId("abcdef00112233gg".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_no_diff_target",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![gitcomet_core::domain::FileStatus {
                path: path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }],
        }
        .into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-s ctrl-d ctrl-h ctrl-shift-c");
    draw_and_drain_test_window(cx);

    let clipboard_text = cx.read_from_clipboard().and_then(|item| item.text());

    assert!(
        clipboard_text.is_none(),
        "expected Ctrl+Shift+C to not copy anything without a diff target, got: {clipboard_text:?}"
    );
}

#[gpui::test]
fn ctrl_u_unstages_current_file_and_advances_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70606);
    let commit_id = CommitId("abcdef00112233hh".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_u_unstage",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            unstaged: vec![],
            staged: vec![
                gitcomet_core::domain::FileStatus {
                    path: first.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Added,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: second.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Added,
                    conflict: None,
                },
            ],
        }
        .into(),
    );
    let target = DiffTarget::WorkingTree {
        path: first.clone(),
        area: DiffArea::Staged,
    };
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(target).into());
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-u");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected Ctrl+U to unstage the active file and advance the diff target"
    );
}
