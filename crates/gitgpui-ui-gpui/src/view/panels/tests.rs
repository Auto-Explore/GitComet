use super::*;
use gitgpui_core::error::{Error, ErrorKind};
use gitgpui_core::services::{GitBackend, GitRepository, Result};
use gitgpui_state::store::AppStore;
use gpui::px;
use std::path::Path;
use std::sync::Arc;

const _: () = {
    assert!(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX > 0.0);
    assert!(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX <= 400.0);
};

#[test]
fn conflict_resolver_strategy_maps_conflict_kinds() {
    use gitgpui_core::conflict_session::ConflictResolverStrategy as S;
    use gitgpui_core::domain::FileConflictKind as K;

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

#[gpui::test]
fn file_preview_renders_scrollable_syntax_highlighted_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitGpuiView::new(store, events, None, window, cx)
    });

    let repo_id = gitgpui_state::model::RepoId(1);
    let workdir = std::env::temp_dir().join(format!("gitgpui_ui_test_{}", std::process::id()));
    let file_rel = std::path::PathBuf::from("preview.rs");
    let lines: Arc<Vec<String>> = Arc::new(
        (0..300)
            .map(|_| "fn main() { let x = 1; }".to_string())
            .collect(),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitgpui_state::model::RepoState::new_opening(
                repo_id,
                gitgpui_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitgpui_state::model::Loadable::Ready(
                gitgpui_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitgpui_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitgpui_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_target = Some(gitgpui_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitgpui_core::domain::DiffArea::Unstaged,
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
                pane.worktree_preview = gitgpui_state::model::Loadable::Ready(lines);
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
            .max_offset()
            .height;
        assert!(
            max_offset > px(0.0),
            "expected file preview to overflow and be scrollable"
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
        super::super::GitGpuiView::new(store, events, None, window, cx)
    });

    let repo_id = gitgpui_state::model::RepoId(2);
    let workdir =
        std::env::temp_dir().join(format!("gitgpui_ui_test_{}_patch", std::process::id()));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let target = gitgpui_core::domain::DiffTarget::Commit {
                commit_id: gitgpui_core::domain::CommitId("deadbeef".to_string()),
                path: None,
            };

            let diff = gitgpui_core::domain::Diff {
                target: target.clone(),
                lines: vec![
                    gitgpui_core::domain::DiffLine {
                        kind: gitgpui_core::domain::DiffLineKind::Header,
                        text: "diff --git a/foo.rs b/foo.rs".into(),
                    },
                    gitgpui_core::domain::DiffLine {
                        kind: gitgpui_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitgpui_core::domain::DiffLine {
                        kind: gitgpui_core::domain::DiffLineKind::Context,
                        text: " fn main() { let x = 1; }".into(),
                    },
                ],
            };

            let mut repo = gitgpui_state::model::RepoState::new_opening(
                repo_id,
                gitgpui_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = gitgpui_state::model::Loadable::Ready(
                gitgpui_core::domain::RepoStatus::default().into(),
            );
            repo.diff_target = Some(target);
            repo.diff_rev = 1;
            repo.diff = gitgpui_state::model::Loadable::Ready(diff.into());

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
        super::super::GitGpuiView::new(store, events, None, window, cx)
    });

    let repo_id = gitgpui_state::model::RepoId(3);
    let workdir =
        std::env::temp_dir().join(format!("gitgpui_ui_test_{}_deleted", std::process::id()));
    let file_rel = std::path::PathBuf::from("deleted.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitgpui_state::model::RepoState::new_opening(
                repo_id,
                gitgpui_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            repo.status = gitgpui_state::model::Loadable::Ready(
                gitgpui_core::domain::RepoStatus {
                    staged: vec![gitgpui_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitgpui_core::domain::FileStatusKind::Deleted,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_target = Some(gitgpui_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitgpui_core::domain::DiffArea::Staged,
            });
            repo.diff_file = gitgpui_state::model::Loadable::Ready(Some(Arc::new(
                gitgpui_core::domain::FileDiffText {
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
        let gitgpui_state::model::Loadable::Ready(lines) = &pane.worktree_preview else {
            panic!("expected worktree preview to be ready");
        };
        assert_eq!(lines.as_ref(), &vec!["one".to_string(), "two".to_string()]);
    });
}
