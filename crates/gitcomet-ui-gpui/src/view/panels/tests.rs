use super::main::{
    next_conflict_diff_split_ratio, show_conflict_save_stage_action,
    show_external_mergetool_actions,
};
use super::*;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_state::store::AppStore;
use gpui::px;
use std::path::Path;
use std::sync::Arc;

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
    let expected = lines.join("\n");
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

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
            repo.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
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
            repo.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
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
            repo.diff_target = Some(target);
            repo.diff_rev = 1;
            repo.diff = gitcomet_state::model::Loadable::Ready(diff.into());

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
            repo.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Staged,
            });
            repo.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
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
                pane.try_populate_worktree_preview_from_diff_file();
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
fn added_file_preview_ctrl_a_ctrl_c_copies_all_content(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(31);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_added_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("added.rs");
    let lines: Arc<Vec<String>> =
        Arc::new(vec!["alpha".into(), "beta".into(), "gamma".into()]);
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
fn switching_active_repo_clears_commit_message_input(cx: &mut gpui::TestAppContext) {
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
