use super::*;
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

fn diff_view_mode(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> DiffViewMode {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_view)
}

fn show_whitespace(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).main_pane.read(app).show_whitespace)
}

fn diff_search_active(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_active)
}

fn popover_kind(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<PopoverKind> {
    cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app))
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
    assert_declared_shortcuts(&unstaged_status_model, &["H", "S", "D", "C"]);
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
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "S",
        ContextMenuAction::StageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "C",
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
    assert_declared_shortcuts(&staged_status_model, &["H", "U", "D", "C"]);
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
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "U",
        ContextMenuAction::UnstageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        "C",
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
    assert_declared_shortcuts(&conflicted_status_model, &["H", "O", "T", "M", "D", "C"]);
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
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "O",
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
        "T",
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
        "M",
        ContextMenuAction::SelectConflictDiff {
            repo_id: rid,
            path
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "D",
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "C",
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
    assert_declared_shortcuts(&diff_hunk_unstaged_model, &["S", "D"]);
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        "S",
        ContextMenuAction::StageHunk {
            repo_id: rid,
            src_ix
        } if *rid == repo_id && *src_ix == 3
    );
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        "D",
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
        },
        CommitFileChange {
            path: std::path::PathBuf::from("src/commit_details/second.rs"),
            kind: FileStatusKind::Modified,
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
        two_hunk_diff(DiffTarget::WorkingTree {
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

    set_diff_selection_anchor(cx, &view, Some(second_change));
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 from commit-message input to fall back to the previous diff change when search is inactive"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after F2 change navigation"
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
fn commit_message_text_input_ctrl_f_does_not_activate_diff_search(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7055);
    let commit_id = CommitId("1111222233334444".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_ctrl_f",
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

    cx.simulate_keystrokes("ctrl-f");
    draw_and_drain_test_window(cx);

    assert!(
        !diff_search_active(cx, &view),
        "expected Ctrl-F from commit-message input to avoid activating diff search"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Ctrl-F"
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

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.show_whitespace = false;
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
                pane.show_whitespace = false;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-w");
    draw_and_drain_test_window(cx);
    assert!(
        !show_whitespace(cx, &view),
        "expected Alt-W from commit-message input to avoid toggling whitespace visibility"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Alt-I/Alt-S/Alt-W"
    );
}

#[gpui::test]
fn commit_message_text_input_alt_h_does_not_open_diff_hunks(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7057);
    let commit_id = CommitId("1111222233336666".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_alt_h",
        std::process::id()
    ));
    let target = DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id.clone(),
        message: "subject".into(),
        committed_at: "2026-04-14 12:00:00 +0300".into(),
        parent_ids: vec![],
        files: vec![CommitFileChange {
            path: std::path::PathBuf::from("src/lib.rs"),
            kind: FileStatusKind::Modified,
        }],
    }));
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(target).into());

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);

    assert_eq!(
        popover_kind(cx, &view),
        None,
        "expected no popover before Alt-H from commit-message input"
    );

    cx.simulate_keystrokes("alt-h");
    draw_and_drain_test_window(cx);

    assert_eq!(
        popover_kind(cx, &view),
        None,
        "expected Alt-H from commit-message input to avoid opening the diff hunks popover"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Alt-H"
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
