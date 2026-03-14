use super::main::{
    next_conflict_diff_split_ratio, show_conflict_save_stage_action,
    show_external_mergetool_actions,
};
use super::*;
use crate::test_support::lock_clipboard_test;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_state::store::AppStore;
use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, px};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const _: () = {
    assert!(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX > 0.0);
    assert!(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX <= 400.0);
};

#[test]
fn shows_external_mergetool_actions_only_in_normal_mode() {
    assert!(show_external_mergetool_actions(GitCometViewMode::Normal));
    assert!(!show_external_mergetool_actions(
        GitCometViewMode::FocusedMergetool
    ));
}

#[test]
fn shows_save_stage_action_only_in_normal_mode() {
    assert!(show_conflict_save_stage_action(GitCometViewMode::Normal));
    assert!(!show_conflict_save_stage_action(
        GitCometViewMode::FocusedMergetool
    ));
}

#[test]
fn next_conflict_diff_split_ratio_returns_none_when_main_width_is_not_positive() {
    let state = ConflictDiffSplitResizeState {
        start_x: px(10.0),
        start_ratio: 0.5,
    };
    let ratio = next_conflict_diff_split_ratio(state, px(20.0), [px(-4.0), px(-4.0)]);
    assert!(ratio.is_none());
}

#[test]
fn next_conflict_diff_split_ratio_applies_drag_delta() {
    let state = ConflictDiffSplitResizeState {
        start_x: px(100.0),
        start_ratio: 0.5,
    };
    let ratio = next_conflict_diff_split_ratio(state, px(160.0), [px(300.0), px(300.0)]).unwrap();

    let expected = (0.5 + (60.0 / (300.0 + 300.0 + super::PANE_RESIZE_HANDLE_PX))).clamp(0.1, 0.9);
    assert!((ratio - expected).abs() < 0.0001);
}

#[test]
fn next_conflict_diff_split_ratio_clamps_to_expected_bounds() {
    let state = ConflictDiffSplitResizeState {
        start_x: px(100.0),
        start_ratio: 0.5,
    };
    let min_ratio =
        next_conflict_diff_split_ratio(state, px(-10_000.0), [px(240.0), px(240.0)]).unwrap();
    let max_ratio =
        next_conflict_diff_split_ratio(state, px(10_000.0), [px(240.0), px(240.0)]).unwrap();
    assert_eq!(min_ratio, 0.1);
    assert_eq!(max_ratio, 0.9);
}

#[test]
fn conflict_resolver_strategy_maps_conflict_kinds() {
    use gitcomet_core::conflict_session::ConflictResolverStrategy as S;
    use gitcomet_core::domain::FileConflictKind as K;

    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothModified), false),
        Some(S::FullTextResolver),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothAdded), false),
        Some(S::FullTextResolver),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::AddedByUs), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::AddedByThem), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::DeletedByUs), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::DeletedByThem), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothDeleted), false),
        Some(S::DecisionOnly),
    );
    assert_eq!(MainPaneView::conflict_resolver_strategy(None, false), None);

    // Binary flag overrides any conflict kind to BinarySidePick.
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothModified), true),
        Some(S::BinarySidePick),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::DeletedByUs), true),
        Some(S::BinarySidePick),
    );
}

struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

fn assert_file_preview_ctrl_a_ctrl_c_copies_all(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    status_kind: gitcomet_core::domain::FileStatusKind,
    lines: Arc<Vec<String>>,
) {
    let _clipboard_guard = lock_clipboard_test();
    let expected = lines.join("\n");
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Create the file on disk so is_file_preview_active() can detect it.
    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(workdir.join(&file_rel), lines.join("\n")).expect("write preview fixture file");

    // Push state through the model first; the observer will clear stale
    // worktree_preview on diff-target change.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: status_kind.clone(),
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    // Set preview data in a separate update so it runs after the observer
    // has cleared the stale preview state.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let workdir = workdir.clone();
            let file_rel = file_rel.clone();
            let lines = Arc::clone(&lines);
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_rel));
                pane.worktree_preview = gitcomet_state::model::Loadable::Ready(lines);
                pane.worktree_preview_segments_cache_path = None;
                pane.worktree_preview_segments_cache.clear();
                pane.worktree_preview_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus);
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected.into())
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

fn assert_markdown_file_preview_toggle_visible(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    status_kind: gitcomet_core::domain::FileStatusKind,
    old_text: Option<&str>,
    new_text: Option<&str>,
    create_worktree_file: bool,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create markdown preview workdir");
    if create_worktree_file {
        let contents = new_text.or(old_text).unwrap_or_default();
        std::fs::write(workdir.join(&file_rel), contents).expect("write markdown preview fixture");
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: status_kind,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: file_rel.clone(),
                    old: old_text.map(|text| text.to_string()),
                    new: new_text.map(|text| text.to_string()),
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.as_ref()),
        );
        let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
            false,
            pane.is_file_preview_active(),
            rendered_preview_kind,
        );
        assert!(
            pane.is_file_preview_active(),
            "expected markdown {status_kind:?} target to use single-file preview mode"
        );
        assert_eq!(
            toggle_kind,
            Some(RenderedPreviewKind::Markdown),
            "expected markdown {status_kind:?} target to request the main preview toggle"
        );
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "expected markdown {status_kind:?} target to default to Preview mode"
        );
    });
    assert!(
        cx.debug_bounds("markdown_diff_view_toggle").is_some(),
        "expected markdown Preview/Text toggle for {status_kind:?} file preview"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview fixture");
}

fn focus_diff_panel(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus);
        let _ = window.draw(app);
    });
}

fn wait_for_main_pane_condition<T, Ready, Snapshot>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    description: &str,
    is_ready: Ready,
    snapshot: Snapshot,
) where
    T: std::fmt::Debug,
    Ready: Fn(&MainPaneView) -> bool,
    Snapshot: Fn(&MainPaneView) -> T,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            is_ready(&pane)
        });
        if ready {
            return;
        }
        if std::time::Instant::now() >= deadline {
            let snapshot = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                snapshot(&pane)
            });
            panic!("timed out waiting for {description}: {snapshot:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[gpui::test]
fn file_preview_renders_scrollable_syntax_highlighted_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(1);
    let workdir = std::env::temp_dir().join(format!("gitcomet_ui_test_{}", std::process::id()));
    let file_rel = std::path::PathBuf::from("preview.rs");
    let lines: Arc<Vec<String>> = Arc::new(
        (0..300)
            .map(|_| {
                "fn main() { let x = 1; } // this line is intentionally long to force horizontal overflow in preview rows........................................".to_string()
            })
            .collect(),
    );

    // Create the file on disk so is_file_preview_active() can detect it.
    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(workdir.join(&file_rel), lines.join("\n")).expect("write preview fixture file");

    // Push state through the model first; the observer will clear stale
    // worktree_preview on diff-target change.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    // Set preview data in a separate update so it runs after the observer
    // has cleared the stale preview state.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let workdir = workdir.clone();
            let file_rel = file_rel.clone();
            let lines = Arc::clone(&lines);
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_rel));
                pane.worktree_preview = gitcomet_state::model::Loadable::Ready(lines);
                pane.worktree_preview_segments_cache_path = None;
                pane.worktree_preview_segments_cache.clear();
                pane.worktree_preview_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let max_offset = pane
            .worktree_preview_scroll
            .0
            .borrow()
            .base_handle
            .max_offset();
        assert!(
            max_offset.height > px(0.0),
            "expected file preview to overflow and be scrollable"
        );
        assert!(
            max_offset.width > px(0.0),
            "expected file preview to overflow horizontally"
        );

        let Some(styled) = pane.worktree_preview_segments_cache.get(&0) else {
            panic!("expected first visible preview row to populate segment cache");
        };
        assert!(
            !styled.highlights.is_empty(),
            "expected syntax highlighting highlights for preview row"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn patch_view_applies_syntax_highlighting_to_context_lines(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(2);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_patch", std::process::id()));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let target = gitcomet_core::domain::DiffTarget::Commit {
                commit_id: gitcomet_core::domain::CommitId("deadbeef".to_string()),
                path: None,
            };

            let diff = gitcomet_core::domain::Diff {
                target: target.clone(),
                lines: vec![
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Header,
                        text: "diff --git a/foo.rs b/foo.rs".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Context,
                        text: " fn main() { let x = 1; }".into(),
                    },
                ],
            };

            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(diff.into());

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let styled = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|v| v.as_ref())
            .expect("expected context line to be syntax-highlighted and cached");
        assert!(
            !styled.highlights.is_empty(),
            "expected syntax highlighting highlights for context line"
        );
    });
}

#[gpui::test]
fn smoke_tests_diff_draw_stabilizes_without_notify_churn(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let old_text = include_str!("../../smoke_tests.rs");
    let new_text = format!("{old_text}\n// refresh-loop-regression\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
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
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: path.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path,
                    old: Some(old_text.to_string()),
                    new: Some(new_text),
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    let root_notifies = Arc::new(AtomicUsize::new(0));
    let _root_notify_sub = cx.update(|_window, app| {
        let root_notifies = Arc::clone(&root_notifies);
        view.update(app, |_this, cx| {
            cx.observe_self(move |_this, _cx| {
                root_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    let main_notifies = Arc::new(AtomicUsize::new(0));
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let _main_notify_sub = cx.update(|_window, app| {
        let main_notifies = Arc::clone(&main_notifies);
        main_pane.update(app, |_pane, cx| {
            cx.observe_self(move |_pane, _cx| {
                main_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    for _ in 0..4 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    root_notifies.store(0, Ordering::Relaxed);
    main_notifies.store(0, Ordering::Relaxed);

    for _ in 0..8 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let root_notify_count = root_notifies.load(Ordering::Relaxed);
    let main_notify_count = main_notifies.load(Ordering::Relaxed);
    assert!(
        root_notify_count <= 1,
        "root view kept notifying during steady smoke_tests.rs diff draws: {root_notify_count}",
    );
    assert!(
        main_notify_count <= 1,
        "main pane kept notifying during steady smoke_tests.rs diff draws: {main_notify_count}",
    );
}

#[gpui::test]
fn file_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let old_text = "fn smoke_test_fixture() {\n    let mut x = 1;\n    x += 1;\n}\n".repeat(64);
    let new_text = format!("{old_text}\n// file-diff-cache-rev-stability\n");

    let set_state = |cx: &mut gpui::VisualTestContext, diff_file_rev: u64| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = gitcomet_state::model::RepoState::new_opening(
                    repo_id,
                    gitcomet_core::domain::RepoSpec {
                        workdir: workdir.clone(),
                    },
                );
                repo.status = gitcomet_state::model::Loadable::Ready(
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
                repo.diff_state.diff_target =
                    Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                        path: path.clone(),
                        area: gitcomet_core::domain::DiffArea::Unstaged,
                    });
                repo.diff_state.diff_file_rev = diff_file_rev;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText {
                        path: path.clone(),
                        old: Some(old_text.clone()),
                        new: Some(new_text.clone()),
                    },
                )));

                let next_state = Arc::new(AppState {
                    repos: vec![repo],
                    active_repo: Some(repo_id),
                    ..Default::default()
                });

                this._ui_model.update(cx, |model, cx| {
                    model.set_state(Arc::clone(&next_state), cx);
                });
            });
        });
    };

    set_state(cx, 1);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.file_diff_cache_inflight.is_none() && pane.file_diff_cache_path.is_some()
        });
        if ready {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let snapshot = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_inflight,
                    pane.file_diff_cache_repo_id,
                    pane.file_diff_cache_rev,
                    pane.file_diff_cache_target.clone(),
                    pane.file_diff_cache_path.clone(),
                    pane.file_diff_inline_cache.len(),
                    pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                    pane.active_repo()
                        .and_then(|repo| repo.diff_state.diff_target.clone()),
                    pane.is_file_diff_view_active(),
                )
            });
            panic!("timed out waiting for initial file-diff cache build: {snapshot:?}");
        }
    }

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);

    for rev in 2..=6 {
        set_state(cx, rev);
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_diff_cache_seq, baseline_seq,
                "identical diff payload should not trigger file-diff rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_diff_cache_inflight.is_none(),
                "file-diff cache should remain built with no background rebuild for identical payload refreshes"
            );
        });
    }
}

#[gpui::test]
fn markdown_diff_preview_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(48);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/README.md");
    let old_text =
        "# Preview title\n\n- first item\n- second item\n\n```rust\nlet value = 1;\n```\n"
            .repeat(24);
    let new_text =
        format!("{old_text}\nA trailing paragraph keeps this markdown diff in preview mode.\n");

    let set_state = |cx: &mut gpui::VisualTestContext, diff_file_rev: u64| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = gitcomet_state::model::RepoState::new_opening(
                    repo_id,
                    gitcomet_core::domain::RepoSpec {
                        workdir: workdir.clone(),
                    },
                );
                repo.status = gitcomet_state::model::Loadable::Ready(
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
                repo.diff_state.diff_target =
                    Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                        path: path.clone(),
                        area: gitcomet_core::domain::DiffArea::Unstaged,
                    });
                repo.diff_state.diff_file_rev = diff_file_rev;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText {
                        path: path.clone(),
                        old: Some(old_text.clone()),
                        new: Some(new_text.clone()),
                    },
                )));

                let next_state = Arc::new(AppState {
                    repos: vec![repo],
                    active_repo: Some(repo_id),
                    ..Default::default()
                });

                this._ui_model.update(cx, |model, cx| {
                    model.set_state(Arc::clone(&next_state), cx);
                });
            });
        });
    };

    set_state(cx, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "initial markdown preview cache build",
        |pane| {
            pane.file_markdown_preview_inflight.is_none()
                && matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                )
        },
        |pane| {
            (
                pane.file_markdown_preview_seq,
                pane.file_markdown_preview_inflight,
                pane.file_markdown_preview_cache_repo_id,
                pane.file_markdown_preview_cache_rev,
                pane.file_markdown_preview_cache_target.clone(),
                pane.file_markdown_preview_cache_content_signature,
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
            )
        },
    );

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_markdown_preview_seq);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "markdown diff preview should default to Preview mode"
        );
    });

    for rev in 2..=6 {
        set_state(cx, rev);
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_markdown_preview_seq, baseline_seq,
                "identical markdown diff payload should not trigger preview rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_markdown_preview_inflight.is_none(),
                "markdown preview cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_markdown_preview_cache_rev, rev,
                "identical payload refresh should still advance the markdown cache rev marker"
            );
            assert!(
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
                "markdown preview should remain ready across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn worktree_markdown_diff_defaults_to_preview_mode_and_shows_preview_toggle(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_worktree_markdown_diff_default_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/guide.md");
    let old_text = concat!(
        "# Guide\n",
        "\n",
        "- keep\n",
        "- before\n",
        "\n",
        "```rust\n",
        "let value = 1;\n",
        "```\n",
    );
    let new_text = concat!(
        "# Guide\n",
        "\n",
        "- keep\n",
        "- after\n",
        "\n",
        "```rust\n",
        "let value = 2;\n",
        "```\n",
        "\n",
        "| Col | Value |\n",
        "| --- | --- |\n",
        "| add | 3 |\n",
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create commit markdown diff workdir");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: file_rel.clone(),
                    old: Some(old_text.to_string()),
                    new: Some(new_text.to_string()),
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree markdown diff target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_markdown_preview_cache_repo_id = Some(repo_id);
                pane.file_markdown_preview_cache_rev = 1;
                pane.file_markdown_preview_cache_target = Some(target.clone());
                pane.file_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
                        .expect("worktree markdown diff preview should parse"),
                ));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(!pane.is_file_preview_active());
        assert!(
            pane.is_markdown_preview_active(),
            "expected worktree markdown diff preview to be active; mode={:?} target_kind={:?} diff_target={:?}",
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            crate::view::diff_target_rendered_preview_kind(
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            ),
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone()),
        );
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "expected worktree markdown diff to default to Preview mode"
        );
    });
    assert!(
        cx.debug_bounds("markdown_diff_view_toggle").is_some(),
        "expected markdown Preview/Text toggle for worktree markdown diff"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup worktree markdown diff fixture");
}

#[gpui::test]
fn ctrl_f_from_markdown_file_preview_switches_back_to_text_search(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_search",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");
    let abs_path = workdir.join(&file_rel);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");
    std::fs::write(&abs_path, "# Title\n\npreview body\n").expect("write markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(abs_path.clone());
                pane.worktree_preview = gitcomet_state::model::Loadable::Ready(Arc::new(vec![
                    "# Title".to_string(),
                    "".to_string(),
                    "preview body".to_string(),
                ]));
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });

    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-f");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Source,
            "Ctrl+F should switch markdown preview back to source mode before search"
        );
        assert!(
            pane.diff_search_active,
            "Ctrl+F should activate diff search from markdown preview"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview fixture");
}

#[gpui::test]
fn ctrl_f_from_conflict_markdown_preview_switches_back_to_text_search(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(48);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_markdown_preview_search",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("conflict.md");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                        conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.conflict_state.conflict_file_path = Some(file_rel.clone());
            repo.conflict_state.conflict_file =
                gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
                    path: file_rel.clone(),
                    base_bytes: None,
                    ours_bytes: None,
                    theirs_bytes: None,
                    current_bytes: None,
                    base: Some("# Base\n".to_string()),
                    ours: Some("# Local\n".to_string()),
                    theirs: Some("# Remote\n".to_string()),
                    current: Some(
                        "<<<<<<< ours\n# Local\n=======\n# Remote\n>>>>>>> theirs\n".to_string(),
                    ),
                }));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert_eq!(
                    pane.conflict_resolver.path.as_ref(),
                    Some(&file_rel),
                    "expected conflict resolver state to be ready before toggling preview mode"
                );
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                cx.notify();
            });
        });
    });

    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-f");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.resolver_preview_mode,
            ConflictResolverPreviewMode::Text,
            "Ctrl+F should switch conflict markdown preview back to text mode before search"
        );
        assert!(
            pane.diff_search_active,
            "Ctrl+F should activate diff search from conflict markdown preview"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict markdown preview fixture");
}

#[gpui::test]
fn markdown_file_preview_over_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_over_limit",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("oversized.md");
    let abs_path = workdir.join(&file_rel);
    let oversized_len = crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1;
    let oversized_source = "x".repeat(oversized_len);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create oversize workdir");
    std::fs::write(&abs_path, &oversized_source).expect("write oversize markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(abs_path.clone());
                pane.worktree_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(vec![oversized_source]));
                pane.worktree_preview_source_len = oversized_len;
                pane.worktree_preview_content_rev = 1;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
        assert!(
            pane.worktree_markdown_preview_inflight.is_none(),
            "oversized preview should fail synchronously without background parsing"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown_preview
        else {
            panic!(
                "expected oversize markdown file preview to show fallback error, got {:?}",
                pane.worktree_markdown_preview
            );
        };
        assert!(
            message.contains("1 MiB"),
            "oversize file preview should mention the 1 MiB limit: {message}"
        );
    });
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_none(),
        "oversized markdown file preview should not render the virtualized preview list"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup oversize markdown preview fixture");
}

#[gpui::test]
fn markdown_file_preview_uses_exact_source_length_for_over_limit_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(56);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_exact_source_len",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("exact-source-len.md");
    let abs_path = workdir.join(&file_rel);
    let mut row_limit_source = "x".repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);
    row_limit_source.push('\n');
    let preview_lines = Arc::new(
        row_limit_source
            .lines()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
    );
    assert_eq!(preview_lines.len(), 1);
    assert_eq!(
        preview_lines[0].len(),
        crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES
    );
    assert_eq!(
        row_limit_source.len(),
        crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1
    );
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create exact-source-len workdir");
    std::fs::write(&abs_path, &row_limit_source).expect("write exact-source-len markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(abs_path.clone());
                pane.worktree_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::clone(&preview_lines));
                pane.worktree_preview_content_rev = 1;
                pane.worktree_preview_source_len = row_limit_source.len();
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.ensure_single_markdown_preview_cache(cx);
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
        assert!(
            pane.worktree_markdown_preview_inflight.is_none(),
            "over-limit preview should fail synchronously when exact source length exceeds the markdown cap"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown_preview
        else {
            panic!(
                "expected exact-source-len markdown file preview to show fallback error, got {:?}",
                pane.worktree_markdown_preview
            );
        };
        assert!(
            message.contains("1 MiB"),
            "exact-source-len file preview should mention the 1 MiB limit: {message}"
        );
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_none(),
        "exact-source-len markdown file preview should not render the virtualized preview list"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup exact-source-len markdown preview fixture");
}

#[gpui::test]
fn diff_target_change_clears_worktree_markdown_preview_cache_state(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(55);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_cache_reset",
        std::process::id()
    ));
    let preview_path = std::path::PathBuf::from("docs/preview.md");
    let preview_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: preview_path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let set_state = |cx: &mut gpui::VisualTestContext,
                     diff_target: Option<gitcomet_core::domain::DiffTarget>,
                     diff_state_rev: u64,
                     status_rev: u64| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = gitcomet_state::model::RepoState::new_opening(
                    repo_id,
                    gitcomet_core::domain::RepoSpec {
                        workdir: workdir.clone(),
                    },
                );
                repo.status = gitcomet_state::model::Loadable::Ready(
                    gitcomet_core::domain::RepoStatus::default().into(),
                );
                repo.status_rev = status_rev;
                repo.diff_state.diff_target = diff_target;
                repo.diff_state.diff_state_rev = diff_state_rev;

                let next_state = Arc::new(AppState {
                    repos: vec![repo],
                    active_repo: Some(repo_id),
                    ..Default::default()
                });

                this._ui_model.update(cx, |model, cx| {
                    model.set_state(next_state, cx);
                });
            });
        });
    };

    set_state(cx, Some(preview_target.clone()), 1, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "initial markdown preview target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(preview_target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(workdir.join(&preview_path));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
                pane.worktree_preview_content_rev = 9;
                pane.worktree_preview_source_len = 42;
                pane.worktree_markdown_preview_path = Some(workdir.join(&preview_path));
                pane.worktree_markdown_preview_source_rev = 9;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Loading;
                pane.worktree_markdown_preview_inflight = Some(3);
                cx.notify();
            });
        });
    });

    set_state(cx, None, 2, 2);

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown preview cache reset after diff target change",
        |pane| {
            pane.worktree_preview_path.is_none()
                && pane.worktree_preview_content_rev == 0
                && pane.worktree_preview_source_len == 0
                && pane.worktree_markdown_preview_path.is_none()
                && pane.worktree_markdown_preview_source_rev == 0
                && matches!(
                    pane.worktree_markdown_preview,
                    gitcomet_state::model::Loadable::NotLoaded
                )
                && pane.worktree_markdown_preview_inflight.is_none()
        },
        |pane| {
            format!(
                "worktree_path={:?} worktree_rev={} worktree_source_len={} worktree_markdown_path={:?} worktree_markdown_rev={} worktree_markdown_inflight={:?} worktree_markdown_not_loaded={}",
                pane.worktree_preview_path,
                pane.worktree_preview_content_rev,
                pane.worktree_preview_source_len,
                pane.worktree_markdown_preview_path,
                pane.worktree_markdown_preview_source_rev,
                pane.worktree_markdown_preview_inflight,
                matches!(
                    pane.worktree_markdown_preview,
                    gitcomet_state::model::Loadable::NotLoaded
                ),
            )
        },
    );
}

#[gpui::test]
fn markdown_diff_preview_over_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(52);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_over_limit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/oversized.md");
    let oversized_side =
        "x".repeat(crate::view::markdown_preview::MAX_DIFF_PREVIEW_SOURCE_BYTES / 2 + 1);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
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
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: path.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: path.clone(),
                    old: Some(oversized_side.clone()),
                    new: Some(oversized_side.clone()),
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
        assert!(
            pane.file_markdown_preview_inflight.is_none(),
            "oversized diff preview should fail synchronously without background parsing"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.file_markdown_preview else {
            panic!(
                "expected oversize markdown diff preview to show fallback error, got {:?}",
                pane.file_markdown_preview
            );
        };
        assert!(
            message.contains("2 MiB"),
            "oversize diff preview should mention the 2 MiB limit: {message}"
        );
    });
    assert!(
        cx.debug_bounds("diff_markdown_preview_container").is_none(),
        "oversized markdown diff preview should not render the split preview container"
    );
}

#[gpui::test]
fn markdown_diff_preview_row_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(54);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_row_limit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/row-limit.md");
    let old_text = "---\n".repeat(crate::view::markdown_preview::MAX_PREVIEW_ROWS + 1);
    let new_text = "# still small\n".to_string();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
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
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: path.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: path.clone(),
                    old: Some(old_text.clone()),
                    new: Some(new_text.clone()),
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown diff preview row-limit fallback",
        |pane| {
            pane.file_markdown_preview_inflight.is_none()
                && matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Error(_)
                )
        },
        |pane| {
            (
                pane.file_markdown_preview_seq,
                pane.file_markdown_preview_inflight,
                pane.file_markdown_preview_cache_repo_id,
                pane.file_markdown_preview_cache_rev,
                pane.file_markdown_preview_cache_target.clone(),
                pane.file_markdown_preview_cache_content_signature,
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Loading
                ),
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Error(_)
                ),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.file_markdown_preview else {
            panic!(
                "expected row-limit markdown diff preview to show fallback error, got {:?}",
                pane.file_markdown_preview
            );
        };
        assert!(
            message.contains("row limit"),
            "row-limit diff preview should mention the rendered row limit: {message}"
        );
    });
    assert!(
        cx.debug_bounds("diff_markdown_preview_container").is_none(),
        "row-limit markdown diff preview should not render the split preview container"
    );
}

#[gpui::test]
fn markdown_diff_preview_hides_text_controls_and_ignores_text_hotkeys(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_hotkeys",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/preview.md");
    let old_text = concat!(
        "# Preview\n",
        "one\n",
        "two before\n",
        "three\n",
        "four\n",
        "five\n",
        "six before\n",
        "seven\n",
    );
    let new_text = concat!(
        "# Preview\n",
        "one\n",
        "two after\n",
        "three\n",
        "four\n",
        "five\n",
        "six after\n",
        "seven\n",
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
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
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: path.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: path.clone(),
                    old: Some(old_text.to_string()),
                    new: Some(new_text.to_string()),
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Split;
                pane.show_whitespace = false;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
    });
    assert!(
        cx.debug_bounds("diff_prev_hunk").is_none(),
        "markdown diff preview should hide previous-change control"
    );
    assert!(
        cx.debug_bounds("diff_next_hunk").is_none(),
        "markdown diff preview should hide next-change control"
    );
    assert!(
        cx.debug_bounds("diff_view_toggle").is_none(),
        "markdown diff preview should hide inline/split toggle"
    );

    cx.simulate_keystrokes("alt-i alt-w");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Split);
        assert!(!pane.show_whitespace);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("alt-s");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Inline);
        assert!(!pane.show_whitespace);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered
        );
    });
}

#[gpui::test]
fn conflict_markdown_preview_hides_text_controls_and_ignores_text_hotkeys(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(50);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_preview_hotkeys",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("conflict.md");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create conflict workdir");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                        conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.conflict_state.conflict_file_path = Some(file_rel.clone());
            repo.conflict_state.conflict_file =
                gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
                    path: file_rel.clone(),
                    base_bytes: None,
                    ours_bytes: None,
                    theirs_bytes: None,
                    current_bytes: None,
                    base: Some("# Base one\n\n# Base two\n".to_string()),
                    ours: Some("# Local one\n\n# Local two\n".to_string()),
                    theirs: Some("# Remote one\n\n# Remote two\n".to_string()),
                    current: Some(
                        concat!(
                            "<<<<<<< ours\n",
                            "# Local one\n",
                            "=======\n",
                            "# Remote one\n",
                            ">>>>>>> theirs\n",
                            "\n",
                            "<<<<<<< ours\n",
                            "# Local two\n",
                            "=======\n",
                            "# Remote two\n",
                            ">>>>>>> theirs\n",
                        )
                        .to_string(),
                    ),
                }));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let nav_entries = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.conflict_resolver_set_mode(ConflictDiffMode::Split, cx);
                pane.show_whitespace = false;
                cx.notify();
            });
        });
        view.read(app).main_pane.read(app).conflict_nav_entries()
    });
    assert!(
        nav_entries.len() > 1,
        "expected at least two conflict navigation entries for preview hotkey coverage"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                pane.conflict_resolver.active_conflict = 0;
                pane.conflict_resolver.nav_anchor = None;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_conflict_rendered_preview_active());
    });
    assert!(
        cx.debug_bounds("conflict_show_whitespace_pill").is_none(),
        "conflict markdown preview should hide whitespace control"
    );
    assert!(
        cx.debug_bounds("conflict_mode_toggle").is_none(),
        "conflict markdown preview should hide diff mode toggle"
    );
    assert!(
        cx.debug_bounds("conflict_view_mode_toggle").is_none(),
        "conflict markdown preview should hide view mode toggle"
    );
    assert!(
        cx.debug_bounds("conflict_prev").is_none(),
        "conflict markdown preview should hide previous-conflict navigation"
    );
    assert!(
        cx.debug_bounds("conflict_next").is_none(),
        "conflict markdown preview should hide next-conflict navigation"
    );

    cx.simulate_keystrokes("alt-i alt-w f2 f3 f7");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::TwoWayDiff
        );
        assert_eq!(pane.conflict_resolver.diff_mode, ConflictDiffMode::Split);
        assert!(!pane.show_whitespace);
        assert_eq!(pane.conflict_resolver.active_conflict, 0);
        assert!(
            pane.conflict_resolver.nav_anchor.is_none(),
            "preview hotkeys should not mutate conflict navigation state"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_mode(ConflictDiffMode::Inline, cx);
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                pane.conflict_resolver.active_conflict = 1;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("alt-s");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::TwoWayDiff
        );
        assert_eq!(pane.conflict_resolver.diff_mode, ConflictDiffMode::Inline);
        assert!(!pane.show_whitespace);
        assert_eq!(pane.conflict_resolver.active_conflict, 1);
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict hotkey fixture");
}

#[gpui::test]
fn patch_diff_search_query_keeps_stable_style_cache_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(22);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_patch_search",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let target = gitcomet_core::domain::DiffTarget::Commit {
                commit_id: gitcomet_core::domain::CommitId("feedface".to_string()),
                path: None,
            };

            let diff = gitcomet_core::domain::Diff {
                target: target.clone(),
                lines: vec![
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Header,
                        text: "diff --git a/foo.rs b/foo.rs".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Context,
                        text: " fn main() { let x = 1; }".into(),
                    },
                ],
            };

            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(diff.into());

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut stable_highlights_hash_before = 0u64;
    let mut stable_text_hash_before = 0u64;
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let stable = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|entry| entry.as_ref())
            .expect("expected stable cache entry for context row before search");
        assert!(
            pane.diff_text_query_segments_cache.is_empty(),
            "query overlay cache should start empty"
        );
        stable_highlights_hash_before = stable.highlights_hash;
        stable_text_hash_before = stable.text_hash;
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_search_active = true;
            pane.diff_search_input.update(cx, |input, cx| {
                input.set_text("main", cx);
            });
            cx.notify();
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);

        let stable_after = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|entry| entry.as_ref())
            .expect("expected stable cache entry for context row after search query update");
        assert_eq!(
            stable_after.highlights_hash, stable_highlights_hash_before,
            "search query updates should not rewrite stable style highlights"
        );
        assert_eq!(
            stable_after.text_hash, stable_text_hash_before,
            "search query updates should not rewrite stable styled text"
        );

        assert_eq!(pane.diff_text_query_cache_query.as_ref(), "main");
        let query_overlay = pane
            .diff_text_query_segments_cache
            .get(2)
            .and_then(|entry| entry.as_ref())
            .expect("expected query overlay cache entry for searched context row");
        assert_ne!(
            query_overlay.highlights_hash, stable_after.highlights_hash,
            "query overlay should layer match highlighting on top of stable highlights"
        );
    });
}

#[gpui::test]
fn staged_deleted_file_preview_uses_old_contents(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(3);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_deleted", std::process::id()));
    let file_rel = std::path::PathBuf::from("deleted.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Deleted,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: file_rel.clone(),
                    old: Some("one\ntwo\n".to_string()),
                    new: None,
                },
            )));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.try_populate_worktree_preview_from_diff_file(cx);
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.deleted_file_preview_abs_path(),
            Some(workdir.join(&file_rel))
        );
        let gitcomet_state::model::Loadable::Ready(lines) = &pane.worktree_preview else {
            panic!("expected worktree preview to be ready");
        };
        assert_eq!(lines.as_ref(), &vec!["one".to_string(), "two".to_string()]);
    });
}

#[gpui::test]
fn untracked_markdown_file_preview_defaults_to_preview_mode_and_renders_container(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(59);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_untracked_default_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");
    let abs_path = workdir.join(&file_rel);
    let source = "# Preview title\n\n- first item\n- second item\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create untracked markdown workdir");
    std::fs::write(&abs_path, source).expect("write untracked markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(abs_path.clone());
                pane.worktree_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::clone(&preview_lines));
                pane.worktree_preview_source_len = source.len();
                pane.worktree_preview_content_rev = 1;
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = 1;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("untracked markdown preview should parse"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_file_preview_active());
        assert!(pane.is_markdown_preview_active());
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "expected untracked markdown preview to default to Preview mode"
        );
    });
    assert!(
        cx.debug_bounds("markdown_diff_view_toggle").is_some(),
        "expected markdown Preview/Text toggle for untracked markdown preview"
    );
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_some(),
        "expected rendered markdown preview container for untracked markdown preview"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup untracked markdown preview fixture");
}

#[gpui::test]
fn staged_added_markdown_file_preview_shows_preview_text_toggle(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(57);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_added_toggle",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");

    assert_markdown_file_preview_toggle_visible(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Added,
        None,
        Some("# Added markdown\n\nnew body\n"),
        true,
    );
}

#[gpui::test]
fn staged_deleted_markdown_file_preview_shows_preview_text_toggle(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(58);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_deleted_toggle",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");

    assert_markdown_file_preview_toggle_visible(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Deleted,
        Some("# Deleted markdown\n\nold body\n"),
        None,
        false,
    );
}

#[gpui::test]
fn unstaged_deleted_gitlink_preview_does_not_stay_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_gitlink",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let unified = format!(
        "diff --git a/{0} b/{0}\nindex 1234567..0000000 160000\n--- a/{0}\n+++ /dev/null\n@@ -1 +0,0 @@\n-Subproject commit c35be02cd52b18c7b2894dc570825b43c94130ed\n",
        file_rel.display()
    );
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Deleted,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(None);

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "unstaged gitlink-like deleted target should not remain stuck in File Loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup unstaged gitlink fixture");
}

#[gpui::test]
fn unstaged_modified_gitlink_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(45);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_gitlink_mod",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create gitlink-like directory");

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let unified = format!(
        "diff --git a/{0} b/{0}\nindex 1234567..89abcde 160000\n--- a/{0}\n+++ b/{0}\n@@ -1 +1 @@\n-Subproject commit 1234567890123456789012345678901234567890\n+Subproject commit 89abcdef0123456789abcdef0123456789abcdef\n",
        file_rel.display()
    );
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(None);

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_worktree_target_directory(),
            "gitlink-like target should be treated as directory-backed for unified diff mode"
        );
        assert!(
            !pane.is_file_preview_active(),
            "unstaged modified gitlink target should bypass file preview mode"
        );
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "unstaged modified gitlink target should not show stuck File Loading state"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup unstaged gitlink modified fixture");
}

#[gpui::test]
fn ensure_preview_loading_does_not_reenter_loading_from_error_for_same_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let temp = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_loading_error",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).expect("create temp directory");
    let path_a = temp.join("a.txt");
    let path_b = temp.join("b.txt");
    std::fs::write(&path_a, "a\n").expect("write a.txt");
    std::fs::write(&path_b, "b\n").expect("write b.txt");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(path_a.clone());
                pane.worktree_preview = gitcomet_state::model::Loadable::Error("boom".into());

                // Same path: keep showing the existing error, do not bounce back to Loading.
                pane.ensure_preview_loading(path_a.clone());
                assert!(
                    matches!(
                        pane.worktree_preview,
                        gitcomet_state::model::Loadable::Error(_)
                    ),
                    "same-path retry should not reset Error to Loading"
                );

                // Different path: loading the newly selected file is expected.
                pane.ensure_preview_loading(path_b.clone());
                assert_eq!(pane.worktree_preview_path, Some(path_b.clone()));
                assert!(
                    matches!(
                        pane.worktree_preview,
                        gitcomet_state::model::Loadable::Loading
                    ),
                    "new path selection should enter Loading"
                );
            });
        });
    });

    std::fs::remove_dir_all(&temp).expect("cleanup temp directory");
}

#[gpui::test]
fn switching_diff_target_clears_stale_worktree_preview_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(36);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_switch_preview_target",
        std::process::id()
    ));
    let file_a = std::path::PathBuf::from("a.txt");
    let file_b = std::path::PathBuf::from("b.txt");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let make_state = |target_path: std::path::PathBuf, diff_state_rev: u64| {
        Arc::new(AppState {
            repos: vec![{
                let mut repo = gitcomet_state::model::RepoState::new_opening(
                    repo_id,
                    gitcomet_core::domain::RepoSpec {
                        workdir: workdir.clone(),
                    },
                );
                repo.status = gitcomet_state::model::Loadable::Ready(
                    gitcomet_core::domain::RepoStatus {
                        staged: vec![],
                        unstaged: vec![
                            gitcomet_core::domain::FileStatus {
                                path: file_a.clone(),
                                kind: gitcomet_core::domain::FileStatusKind::Untracked,
                                conflict: None,
                            },
                            gitcomet_core::domain::FileStatus {
                                path: file_b.clone(),
                                kind: gitcomet_core::domain::FileStatusKind::Untracked,
                                conflict: None,
                            },
                        ],
                    }
                    .into(),
                );
                repo.diff_state.diff_target =
                    Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                        path: target_path,
                        area: gitcomet_core::domain::DiffArea::Unstaged,
                    });
                repo.diff_state.diff_state_rev = diff_state_rev;
                repo
            }],
            active_repo: Some(repo_id),
            ..Default::default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let first = make_state(file_a.clone(), 1);
            this._ui_model.update(cx, |model, cx| {
                model.set_state(first, cx);
            });
            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_a));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let second = make_state(file_b.clone(), 2);
            this._ui_model.update(cx, |model, cx| {
                model.set_state(second, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let stale_path = workdir.join(&file_a);
        let is_stale_loading =
            matches!(pane.worktree_preview, gitcomet_state::model::Loadable::Loading)
                && pane.worktree_preview_path.as_ref() == Some(&stale_path);
        assert!(
            !is_stale_loading,
            "switching selected file should not keep stale Loading on previous path; state={:?} path={:?}",
            pane.worktree_preview,
            pane.worktree_preview_path
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup workdir");
}

#[gpui::test]
fn staged_directory_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(34);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_staged_dir",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("subproject");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create staged directory path");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_worktree_target_directory(),
            "expected staged directory target detection for gitlink-like entries"
        );
        assert!(
            !pane.is_file_preview_active(),
            "directory targets should avoid file preview mode to show unified subproject diffs"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup staged directory fixture");
}

#[gpui::test]
fn staged_added_missing_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(43);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_staged_added_missing",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("subproject");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.is_file_preview_active(),
            "staged Added targets that are not real files should bypass file preview to avoid stuck loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup staged-added-missing fixture");
}

#[gpui::test]
fn untracked_directory_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(35);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_dir",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("subproject");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create untracked directory path");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_worktree_target_directory(),
            "expected untracked directory target detection for gitlink-like entries"
        );
        assert!(
            !pane.is_file_preview_active(),
            "untracked directory targets should avoid file preview loading mode"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup untracked directory fixture");
}

#[gpui::test]
fn untracked_directory_target_clears_stale_file_loading_state(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_dir_stale_loading",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create untracked directory path");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(
                    gitcomet_core::domain::DiffTarget::WorkingTree {
                        path: file_rel.clone(),
                        area: gitcomet_core::domain::DiffArea::Unstaged,
                    },
                    "",
                ),
            ));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });

            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_rel));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.untracked_directory_notice().is_some(),
            "expected untracked directory selection to expose a directory-specific notice"
        );
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "untracked directory target should not stay stuck in File Loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup stale-loading untracked directory fixture");
}

#[gpui::test]
fn directory_target_with_loading_status_clears_stale_file_loading_state(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_directory_loading_status",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create directory target path");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitcomet_state::model::Loadable::Loading;
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff = gitcomet_state::model::Loadable::Loading;

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });

            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_rel));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.untracked_directory_notice().is_some(),
            "expected directory target to expose a non-file notice even while status is loading"
        );
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "directory target should not stay stuck in File Loading when status is loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup directory-loading-status fixture");
}

#[gpui::test]
fn added_file_preview_ctrl_a_ctrl_c_copies_all_content(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(31);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_added_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("added.rs");
    let lines: Arc<Vec<String>> = Arc::new(vec!["alpha".into(), "beta".into(), "gamma".into()]);
    assert_file_preview_ctrl_a_ctrl_c_copies_all(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Added,
        lines,
    );
}

#[gpui::test]
fn deleted_file_preview_ctrl_a_ctrl_c_copies_all_content(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(32);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_deleted_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("deleted.rs");
    let lines: Arc<Vec<String>> = Arc::new(vec!["old one".into(), "old two".into()]);
    assert_file_preview_ctrl_a_ctrl_c_copies_all(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Deleted,
        lines,
    );
}

#[gpui::test]
fn commit_details_metadata_fields_are_selectable(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(33);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let parent_sha = "89abcdef0123456789abcdef0123456789abcdef".to_string();
    let commit_date = "2026-03-08 12:34:56 +0200".to_string();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: std::path::PathBuf::from("/tmp/repo-commit-metadata-copy"),
                },
            );
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone()),
                    message: "subject".to_string(),
                    committed_at: commit_date.clone(),
                    parent_ids: vec![gitcomet_core::domain::CommitId(parent_sha.clone())],
                    files: vec![],
                },
            ));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });

            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_details_sha_input.read(app).text(), commit_sha);
        assert_eq!(pane.commit_details_date_input.read(app).text(), commit_date);
        assert_eq!(
            pane.commit_details_parent_input.read(app).text(),
            parent_sha
        );
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(cx));
            pane.commit_details_date_input
                .update(cx, |input, cx| input.select_all_text(cx));
            pane.commit_details_parent_input
                .update(cx, |input, cx| input.select_all_text(cx));
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_details_sha_input.read(app).selected_text(),
            Some(commit_sha)
        );
        assert_eq!(
            pane.commit_details_date_input.read(app).selected_text(),
            Some(commit_date)
        );
        assert_eq!(
            pane.commit_details_parent_input.read(app).selected_text(),
            Some(parent_sha)
        );
    });
}

#[gpui::test]
fn switching_active_repo_restores_commit_message_draft_per_repo(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_a = gitcomet_state::model::RepoId(41);
    let repo_b = gitcomet_state::model::RepoId(42);
    let make_state = |active_repo: gitcomet_state::model::RepoId| {
        Arc::new(AppState {
            repos: vec![
                gitcomet_state::model::RepoState::new_opening(
                    repo_a,
                    gitcomet_core::domain::RepoSpec {
                        workdir: std::path::PathBuf::from("/tmp/repo-a"),
                    },
                ),
                gitcomet_state::model::RepoState::new_opening(
                    repo_b,
                    gitcomet_core::domain::RepoSpec {
                        workdir: std::path::PathBuf::from("/tmp/repo-b"),
                    },
                ),
            ],
            active_repo: Some(active_repo),
            ..Default::default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_a);
            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_b);
            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("repo-b draft".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_a);
            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "draft message");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_b);
            this._ui_model.update(cx, |model, cx| {
                model.set_state(Arc::clone(&next_state), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "repo-b draft");
    });
}

#[gpui::test]
fn merge_start_prefills_default_commit_message(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(43);
    let make_state = |merge_message: Option<&str>| {
        let mut repo = gitcomet_state::model::RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-merge"),
            },
        );
        repo.merge_commit_message = gitcomet_state::model::Loadable::Ready(
            merge_message.map(std::string::ToString::to_string),
        );
        repo.merge_message_rev = u64::from(merge_message.is_some());
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..Default::default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this._ui_model.update(cx, |model, cx| {
                model.set_state(make_state(None), cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this._ui_model.update(cx, |model, cx| {
                model.set_state(make_state(Some("Merge branch 'feature'")), cx);
            });
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).text(),
            "Merge branch 'feature'"
        );
    });
}

#[gpui::test]
fn commit_click_dispatches_after_state_update_without_intermediate_redraw(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let make_state = |staged_count: usize, local_actions_in_flight: u32| {
        let mut repo = gitcomet_state::model::RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-commit-click"),
            },
        );
        repo.status = gitcomet_state::model::Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: (0..staged_count)
                    .map(|ix| gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from(format!("staged-{ix}.txt")),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    })
                    .collect(),
                unstaged: Vec::new(),
            }
            .into(),
        );
        repo.local_actions_in_flight = local_actions_in_flight;
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..Default::default()
        })
    };

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this._ui_model.update(cx, |model, cx| {
                model.set_state(make_state(0, 0), cx);
            });
        });
        let _ = window.draw(app);
    });

    let commit_center = cx
        .debug_bounds("commit_button")
        .expect("expected commit button bounds")
        .center();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this._ui_model.update(cx, |model, cx| {
                model.set_state(make_state(1, 0), cx);
            });
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input
                    .update(cx, |input, cx| input.set_text("hello".to_string(), cx));
                cx.notify();
            });
        });
    });

    cx.simulate_mouse_move(commit_center, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: commit_center,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: commit_center,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).text(),
            "",
            "expected first click to execute commit handler and clear the input"
        );
    });
}
