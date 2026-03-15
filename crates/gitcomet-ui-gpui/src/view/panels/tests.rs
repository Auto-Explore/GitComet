use super::main::{
    next_conflict_diff_split_ratio, show_conflict_save_stage_action,
    show_external_mergetool_actions,
};
use super::*;
use crate::test_support::{lock_clipboard_test, lock_visual_test};
use crate::view::panes::main::PreparedSyntaxViewMode;
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

fn set_ready_worktree_preview(
    pane: &mut MainPaneView,
    path: std::path::PathBuf,
    lines: Arc<Vec<String>>,
    source_len: usize,
    cx: &mut gpui::Context<MainPaneView>,
) {
    pane.set_worktree_preview_ready_rows(path, lines.as_ref(), source_len, cx);
    pane.worktree_preview_scroll
        .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
    cx.notify();
}

fn highlights_include_range(
    highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    target: std::ops::Range<usize>,
) -> bool {
    highlights.iter().any(|(range, _)| *range == target)
}

fn styled_debug_info(
    styled: &super::CachedDiffStyledText,
) -> (gpui::SharedString, Vec<std::ops::Range<usize>>) {
    (
        styled.text.clone(),
        styled
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect(),
    )
}

fn styled_debug_info_with_styles(
    styled: &super::CachedDiffStyledText,
) -> (
    gpui::SharedString,
    Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )>,
) {
    (
        styled.text.clone(),
        styled
            .highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect(),
    )
}

fn file_diff_split_row_ix(
    pane: &MainPaneView,
    region: DiffTextRegion,
    text: &str,
) -> Option<usize> {
    pane.file_diff_cache_rows
        .iter()
        .position(|row| match region {
            DiffTextRegion::SplitLeft => row.old.as_deref() == Some(text),
            DiffTextRegion::SplitRight => row.new.as_deref() == Some(text),
            DiffTextRegion::Inline => false,
        })
}

fn file_diff_split_cached_styled<'a>(
    pane: &'a MainPaneView,
    region: DiffTextRegion,
    text: &str,
) -> Option<&'a super::CachedDiffStyledText> {
    let row_ix = file_diff_split_row_ix(pane, region, text)?;
    let key = pane.file_diff_split_cache_key(row_ix, region)?;
    let epoch = pane.file_diff_split_style_cache_epoch(region);
    pane.diff_text_segments_cache_get(key, epoch)
}

fn file_diff_split_cached_debug(
    pane: &MainPaneView,
    region: DiffTextRegion,
    text: &str,
) -> Option<(gpui::SharedString, Vec<std::ops::Range<usize>>)> {
    file_diff_split_cached_styled(pane, region, text).map(styled_debug_info)
}

fn file_diff_inline_ix(
    pane: &MainPaneView,
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> Option<usize> {
    pane.file_diff_inline_cache
        .iter()
        .position(|line| line.kind == kind && line.text.as_ref() == text)
}

fn file_diff_inline_cached_styled<'a>(
    pane: &'a MainPaneView,
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> Option<&'a super::CachedDiffStyledText> {
    let inline_ix = file_diff_inline_ix(pane, kind, text)?;
    let line = pane.file_diff_inline_cache.get(inline_ix)?;
    let epoch = pane.file_diff_inline_style_cache_epoch(line);
    pane.diff_text_segments_cache_get(inline_ix, epoch)
}

fn file_diff_inline_cached_debug(
    pane: &MainPaneView,
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> Option<(gpui::SharedString, Vec<std::ops::Range<usize>>)> {
    file_diff_inline_cached_styled(pane, kind, text).map(styled_debug_info)
}

fn seed_file_diff_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    path: &std::path::Path,
    old_text: &str,
    new_text: &str,
) {
    seed_file_diff_state_with_rev(cx, view, repo_id, workdir, path, 1, old_text, new_text);
}

fn seed_file_diff_state_with_rev(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    path: &std::path::Path,
    diff_file_rev: u64,
    old_text: &str,
    new_text: &str,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = gitcomet_state::model::RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.to_path_buf(),
                },
            );
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: path.to_path_buf(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: path.to_path_buf(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff_file_rev = diff_file_rev;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path: path.to_path_buf(),
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
}

#[gpui::test]
fn worktree_preview_ready_rows_preserve_trailing_empty_line(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let preview_path = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_trailing_empty_row.rs",
        std::process::id()
    ));
    let preview_lines = Arc::new(vec!["alpha".to_string(), "beta".to_string()]);
    let preview_text = "alpha\nbeta\n";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let preview_lines = Arc::clone(&preview_lines);
            let preview_path = preview_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_path,
                    preview_lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.worktree_preview_line_count(), Some(3));
        assert_eq!(pane.worktree_preview_line_starts.as_ref(), &[0, 6, 11]);
        assert_eq!(pane.worktree_preview_line_text(0), Some("alpha"));
        assert_eq!(pane.worktree_preview_line_text(1), Some("beta"));
        assert_eq!(pane.worktree_preview_line_text(2), Some(""));
    });
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
                set_ready_worktree_preview(
                    pane,
                    workdir.join(&file_rel),
                    lines,
                    expected.len(),
                    cx,
                );
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown file preview activation",
        |pane| {
            let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            );
            let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
                false,
                pane.is_file_preview_active(),
                rendered_preview_kind,
            );
            pane.is_file_preview_active()
                && toggle_kind == Some(RenderedPreviewKind::Markdown)
                && pane
                    .rendered_preview_modes
                    .get(RenderedPreviewKind::Markdown)
                    == RenderedPreviewMode::Rendered
        },
        |pane| {
            let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            );
            let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
                false,
                pane.is_file_preview_active(),
                rendered_preview_kind,
            );
            format!(
                "active_repo={:?} diff_target={:?} is_file_preview_active={} toggle_kind={toggle_kind:?} markdown_mode={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.is_file_preview_active(),
                pane.rendered_preview_modes
                    .get(RenderedPreviewKind::Markdown),
            )
        },
    );

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

fn disable_view_poller_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
    });
}

const DEFAULT_MAIN_PANE_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);
const BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(20);

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
    wait_for_main_pane_condition_with_timeout(
        cx,
        view,
        description,
        DEFAULT_MAIN_PANE_WAIT_TIMEOUT,
        is_ready,
        snapshot,
    );
}

fn wait_for_main_pane_condition_with_timeout<T, Ready, Snapshot>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    description: &str,
    timeout: std::time::Duration,
    is_ready: Ready,
    snapshot: Snapshot,
) where
    T: std::fmt::Debug,
    Ready: Fn(&MainPaneView) -> bool,
    Snapshot: Fn(&MainPaneView) -> T,
{
    let deadline = std::time::Instant::now() + timeout;
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
    let preview_text = lines.join("\n");

    // Create the file on disk so is_file_preview_active() can detect it.
    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(workdir.join(&file_rel), &preview_text).expect("write preview fixture file");

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
                set_ready_worktree_preview(
                    pane,
                    workdir.join(&file_rel),
                    lines,
                    preview_text.len(),
                    cx,
                );
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

        let Some(styled) = pane.worktree_preview_segments_cache_get(0) else {
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
fn html_file_preview_renders_injected_javascript_and_css_from_real_document(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(75);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_html_preview_injections",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_injections.html");
    let preview_abs_path = workdir.join(&file_rel);
    let script_line = "const previewValue = 7;";
    let style_line = "color: red;";
    let script_line_ix = 1usize;
    let style_line_ix = 4usize;
    let lines: Arc<Vec<String>> = Arc::new(vec![
        "<script>".to_string(),
        script_line.to_string(),
        "</script>".to_string(),
        "<style>".to_string(),
        style_line.to_string(),
        "</style>".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create HTML preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write HTML preview fixture");

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
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML preview injected syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Html)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(script_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == script_line
                            && highlights_include_range(styled.highlights.as_slice(), 0..5)
                            && highlights_include_range(styled.highlights.as_slice(), 21..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_slice(), 0..5)
                    })
        },
        |pane| {
            let script_cached = pane
                .worktree_preview_segments_cache_get(script_line_ix)
                .map(styled_debug_info);
            let style_cached = pane
                .worktree_preview_segments_cache_get(style_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} language={:?} prepared={:?} script_cached={script_cached:?} style_cached={style_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup HTML preview fixture");
}

#[gpui::test]
fn html_file_preview_renders_injected_attribute_syntax_from_real_document(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(76);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_html_preview_attribute_injections",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_attribute_injections.html");
    let preview_abs_path = workdir.join(&file_rel);
    let onclick_line = r#"<button onclick="const value = 1;">go</button>"#;
    let style_line = r#"<div style="color: red; display: block">ok</div>"#;
    let onclick_line_ix = 0usize;
    let style_line_ix = 1usize;
    let lines: Arc<Vec<String>> = Arc::new(vec![onclick_line.to_string(), style_line.to_string()]);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create HTML attribute preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write HTML attribute preview fixture");

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
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML preview attribute injection syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Html)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(onclick_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == onclick_line
                            && highlights_include_range(styled.highlights.as_slice(), 17..22)
                            && highlights_include_range(styled.highlights.as_slice(), 31..32)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_slice(), 12..17)
                            && highlights_include_range(styled.highlights.as_slice(), 24..31)
                    })
        },
        |pane| {
            let onclick_cached = pane
                .worktree_preview_segments_cache_get(onclick_line_ix)
                .map(styled_debug_info);
            let style_cached = pane
                .worktree_preview_segments_cache_get(style_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} language={:?} prepared={:?} onclick_cached={onclick_cached:?} style_cached={style_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup HTML attribute preview fixture");
}

#[gpui::test]
fn large_file_preview_keeps_prepared_syntax_document_above_old_line_gate(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(52);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_preview_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("large_preview.rs");
    let line_count = 4_001usize;
    let lines: Arc<Vec<String>> = Arc::new(
        (0..line_count)
            .map(|ix| format!("let preview_value_{ix}: usize = {ix};"))
            .collect(),
    );
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create large preview workdir");
    std::fs::write(workdir.join(&file_rel), &preview_text).expect("write large preview fixture");

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
        view.update(app, |this, cx| {
            let workdir = workdir.clone();
            let file_rel = file_rel.clone();
            let lines = Arc::clone(&lines);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    workdir.join(&file_rel),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "large file preview prepared syntax document",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_line_count() == Some(line_count)
                && pane.worktree_preview_text.len() == preview_text.len()
                && pane.worktree_preview_line_starts.len() == line_count
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Rust)
                && pane.worktree_preview_prepared_syntax_document().is_some()
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?} preview_path={:?} line_count={:?} text_len={} line_starts={} syntax_language={:?} prepared_document={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_line_count(),
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_starts.len(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup large preview fixture");
}

#[gpui::test]
fn large_file_preview_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(60);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_preview_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("large_preview_background.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let comment_line = "still inside block comment";
    let mut preview_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
    ];
    preview_lines.extend((3..4_001).map(|ix| format!("let preview_value_{ix}: usize = {ix};")));
    let lines: Arc<Vec<String>> = Arc::new(preview_lines);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create background preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write background preview fixture");

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
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
                assert!(
                    pane.worktree_preview_prepared_syntax_document().is_none(),
                    "zero foreground budget should force worktree preview syntax into the background"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    let (initial_epoch, initial_highlights_hash) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .worktree_preview_segments_cache_get(target_ix)
            .expect("initial draw should populate the visible fallback preview row cache");
        assert_eq!(
            styled.text.as_ref(),
            comment_line,
            "expected the cached preview row to match the multiline comment text"
        );
        assert!(
            styled.highlights.is_empty(),
            "before the background parse completes, the multiline comment row should render as plain text"
        );
        (pane.worktree_preview_style_cache_epoch, styled.highlights_hash)
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file preview background syntax upgrade",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.worktree_preview_prepared_syntax_document().is_some()
                && pane.worktree_preview_style_cache_epoch > initial_epoch
                && pane
                    .worktree_preview_segments_cache_get(target_ix)
                    .is_some_and(|styled| {
                        styled.highlights.iter().any(|(range, style)| {
                            range.start == 0
                                && range.end == comment_line.len()
                                && style.color == Some(pane.theme.colors.text_muted.into())
                        })
                    })
        },
        |pane| {
            let row_cache = pane
                .worktree_preview_segments_cache_get(target_ix)
                .map(styled_debug_info_with_styles);
            format!(
                "prepared_document={:?} style_epoch={} cache_path={:?} row_cache={row_cache:?}",
                pane.worktree_preview_prepared_syntax_document(),
                pane.worktree_preview_style_cache_epoch,
                pane.worktree_preview_segments_cache_path.clone(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .worktree_preview_segments_cache_get(target_ix)
            .expect("background syntax completion should repopulate the preview row cache");
        assert_ne!(
            styled.highlights_hash, initial_highlights_hash,
            "background syntax should replace the plain-text fallback row styling"
        );
        assert!(
            styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "multiline comment row should upgrade to comment highlighting after background parsing"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup background preview fixture");
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
            .and_then(|v| v.as_ref().map(|entry| &entry.styled))
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
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let stable_left_line = "    x += 1;";
    let stable_right_line = "    x += 1;";
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
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
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
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
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
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (left_epoch_before, right_epoch_before, left_hash_before, right_hash_before) =
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, _cx| {
                    let left_row_ix =
                        file_diff_split_row_ix(pane, DiffTextRegion::SplitLeft, stable_left_line)
                            .expect(
                                "expected left split row to exist before seeding the row cache",
                            );
                    let right_row_ix =
                        file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, stable_right_line)
                            .expect(
                                "expected right split row to exist before seeding the row cache",
                            );
                    let left_key = pane
                        .file_diff_split_cache_key(left_row_ix, DiffTextRegion::SplitLeft)
                        .expect("left split row should produce a cache key");
                    let right_key = pane
                        .file_diff_split_cache_key(right_row_ix, DiffTextRegion::SplitRight)
                        .expect("right split row should produce a cache key");
                    let left_epoch =
                        pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
                    let right_epoch =
                        pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
                    let make_seeded =
                        |text: &str, hue: f32, hash: u64| super::CachedDiffStyledText {
                            text: text.to_string().into(),
                            highlights: Arc::new(vec![(
                                0..text.len().min(4),
                                gpui::HighlightStyle {
                                    color: Some(gpui::hsla(hue, 1.0, 0.5, 1.0)),
                                    ..gpui::HighlightStyle::default()
                                },
                            )]),
                            highlights_hash: hash,
                            text_hash: hash.wrapping_mul(31),
                        };
                    pane.diff_text_segments_cache_set(
                        left_key,
                        left_epoch,
                        make_seeded(stable_left_line, 0.0, 0xA11CE),
                    );
                    pane.diff_text_segments_cache_set(
                        right_key,
                        right_epoch,
                        make_seeded(stable_right_line, 0.6, 0xBEEF),
                    );

                    let left_cached = file_diff_split_cached_styled(
                        pane,
                        DiffTextRegion::SplitLeft,
                        stable_left_line,
                    )
                    .expect("seeded left split row should be immediately readable");
                    let right_cached = file_diff_split_cached_styled(
                        pane,
                        DiffTextRegion::SplitRight,
                        stable_right_line,
                    )
                    .expect("seeded right split row should be immediately readable");
                    (
                        left_epoch,
                        right_epoch,
                        left_cached.highlights_hash,
                        right_cached.highlights_hash,
                    )
                })
            })
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
                pane.file_diff_cache_seq, baseline_seq,
                "identical diff payload should not trigger file-diff rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_diff_cache_inflight.is_none(),
                "file-diff cache should remain built with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_diff_cache_rev, rev,
                "identical payload refresh should still advance the active file-diff rev marker"
            );
            assert_eq!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                left_epoch_before,
                "identical payload refresh should preserve the left split style epoch"
            );
            assert_eq!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
                right_epoch_before,
                "identical payload refresh should preserve the right split style epoch"
            );
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some(),
                "identical payload refresh should keep the left prepared syntax document reachable"
            );
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
                "identical payload refresh should keep the right prepared syntax document reachable"
            );
            let left_cached =
                file_diff_split_cached_styled(&pane, DiffTextRegion::SplitLeft, stable_left_line)
                    .expect("identical payload refresh should preserve the cached left split row");
            let right_cached =
                file_diff_split_cached_styled(&pane, DiffTextRegion::SplitRight, stable_right_line)
                    .expect("identical payload refresh should preserve the cached right split row");
            assert_eq!(
                left_cached.highlights_hash, left_hash_before,
                "identical payload refresh should keep the cached left split styling intact"
            );
            assert_eq!(
                right_cached.highlights_hash, right_hash_before,
                "identical payload refresh should keep the cached right split styling intact"
            );
        });
    }
}

#[gpui::test]
fn file_diff_view_renders_split_and_inline_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_syntax_view",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/file_diff_projection.rs");
    let removed_line = "struct Removed {}";
    let added_line = "fn added() { let value = 2; }";
    let removed_inline_text = format!("-{removed_line}");
    let added_inline_text = format!("+{added_line}");
    let old_text = format!("const KEEP: i32 = 1;\n{removed_line}\nconst AFTER: i32 = 2;\n");
    let new_text = format!("const KEEP: i32 = 1;\nconst AFTER: i32 = 2;\n{added_line}\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} repo_id={:?} cache_rev={} cache_target={:?} cache_path={:?} file_diff_active={} active_repo={:?} active_diff_file_rev={:?} active_diff_target={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target.clone(),
                pane.file_diff_cache_path.clone(),
                pane.is_file_diff_view_active(),
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff split syntax render",
        |pane| {
            let Some(remove_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, removed_line)
            else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_line
                && add_styled.text.as_ref() == added_line
                && highlights_include_range(remove_styled.highlights.as_slice(), 0..6)
                && highlights_include_range(add_styled.highlights.as_slice(), 0..2)
        },
        |pane| {
            let remove_row_ix =
                file_diff_split_row_ix(pane, DiffTextRegion::SplitLeft, removed_line);
            let add_row_ix = file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, added_line);
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_line);
            format!(
                "file_diff_active={} diff_view={:?} visible_len={} cache_path={:?} cache_repo_id={:?} cache_rev={} cache_target={:?} active_repo={:?} active_diff_file_rev={:?} active_diff_target={:?} remove_row_ix={remove_row_ix:?} add_row_ix={add_row_ix:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.diff_visible_len(),
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target.clone(),
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff inline syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_line
                && add_styled.text.as_ref() == added_line
                && highlights_include_range(remove_styled.highlights.as_slice(), 0..6)
                && highlights_include_range(add_styled.highlights.as_slice(), 0..2)
        },
        |pane| {
            let remove_inline_ix = file_diff_inline_ix(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_inline_ix = file_diff_inline_ix(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            format!(
                "file_diff_active={} diff_view={:?} visible_len={} remove_inline_ix={remove_inline_ix:?} add_inline_ix={add_inline_ix:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.diff_visible_len(),
            )
        },
    );
}

#[gpui::test]
fn html_file_diff_renders_injected_attribute_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(77);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_html_attribute_injections",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/file_diff_attribute_injections.html");
    let removed_onclick_line = r#"<button onclick="const value = 1;">go</button>"#;
    let added_onclick_line = r#"<button onclick="const value = 2;">go</button>"#;
    let added_style_line = r#"<div style="color: red; display: block">ok</div>"#;
    let removed_inline_text = format!("-{removed_onclick_line}");
    let added_inline_text = format!("+{added_onclick_line}");
    let style_inline_text = format!("+{added_style_line}");
    let old_text = format!("<p>keep</p>\n{removed_onclick_line}\n<p>after</p>\n");
    let new_text = format!("<p>keep</p>\n<p>after</p>\n{added_onclick_line}\n{added_style_line}\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_onclick_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_onclick_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_style_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == style_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff split attribute injection syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_split_cached_styled(
                pane,
                DiffTextRegion::SplitLeft,
                removed_onclick_line,
            ) else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_onclick_line)
            else {
                return false;
            };
            let Some(style_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_style_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_onclick_line
                && add_styled.text.as_ref() == added_onclick_line
                && style_styled.text.as_ref() == added_style_line
                && highlights_include_range(remove_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(add_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(add_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(style_styled.highlights.as_slice(), 12..17)
                && highlights_include_range(style_styled.highlights.as_slice(), 24..31)
        },
        |pane| {
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_onclick_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_onclick_line);
            let style_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_style_line);
            format!(
                "diff_view={:?} remove_cached={remove_cached:?} add_cached={add_cached:?} style_cached={style_cached:?}",
                pane.diff_view,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff inline attribute injection syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };
            let Some(style_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &style_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_onclick_line
                && add_styled.text.as_ref() == added_onclick_line
                && style_styled.text.as_ref() == added_style_line
                && highlights_include_range(remove_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(add_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(add_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(style_styled.highlights.as_slice(), 12..17)
                && highlights_include_range(style_styled.highlights.as_slice(), 24..31)
        },
        |pane| {
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            let style_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &style_inline_text,
            );
            format!(
                "diff_view={:?} remove_cached={remove_cached:?} add_cached={add_cached:?} style_cached={style_cached:?}",
                pane.diff_view,
            )
        },
    );
}

#[gpui::test]
fn xml_file_diff_renders_syntax_highlights_from_real_documents(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(79);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_xml_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("config/settings.xml");
    let removed_tag_line = r#"<server port="8080">"#;
    let added_tag_line = r#"<server port="9090" mode="prod">"#;
    let comment_line = "<!-- configuration -->";
    let removed_inline_text = format!("-{removed_tag_line}");
    let added_inline_text = format!("+{added_tag_line}");
    let old_text = format!("{comment_line}\n{removed_tag_line}\n  <name>app</name>\n</server>\n");
    let new_text = format!("{comment_line}\n{added_tag_line}\n  <name>app</name>\n</server>\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Xml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_tag_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_tag_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} language={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff split syntax render",
        |pane| {
            let Some(remove_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, removed_tag_line)
            else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_tag_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_tag_line
                && add_styled.text.as_ref() == added_tag_line
                && highlights_include_range(remove_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(add_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 20..24)
        },
        |pane| {
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_tag_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_tag_line);
            format!(
                "diff_view={:?} language={:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.diff_view, pane.file_diff_cache_language,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff inline syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_tag_line
                && add_styled.text.as_ref() == added_tag_line
                && highlights_include_range(remove_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(add_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 20..24)
        },
        |pane| {
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            format!(
                "diff_view={:?} language={:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.diff_view, pane.file_diff_cache_language,
            )
        },
    );
}

#[gpui::test]
fn xml_file_preview_renders_syntax_highlights_from_real_document(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(78);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_xml_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("config.xml");
    let preview_abs_path = workdir.join(&file_rel);
    let tag_line = r#"<server port="8080">"#;
    let comment_line = "<!-- configuration -->";
    let tag_line_ix = 1usize;
    let comment_line_ix = 0usize;
    let lines: Arc<Vec<String>> = Arc::new(vec![
        comment_line.to_string(),
        tag_line.to_string(),
        "  <name>app</name>".to_string(),
        "</server>".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create XML preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write XML preview fixture");

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
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML preview syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Xml)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(comment_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == comment_line
                            && highlights_include_range(styled.highlights.as_slice(), 0..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(tag_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == tag_line
                            && highlights_include_range(styled.highlights.as_slice(), 1..7)
                            && highlights_include_range(styled.highlights.as_slice(), 8..12)
                    })
        },
        |pane| {
            let comment_cached = pane
                .worktree_preview_segments_cache_get(comment_line_ix)
                .map(styled_debug_info);
            let tag_cached = pane
                .worktree_preview_segments_cache_get(tag_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} language={:?} prepared={:?} comment_cached={comment_cached:?} tag_cached={tag_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup XML preview fixture");
}

#[gpui::test]
fn large_file_diff_keeps_prepared_syntax_documents_above_old_line_gate(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(53);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_diff_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/large_file_diff.rs");
    let line_count = 4_001usize;
    let changed_old_line = format!(
        "let diff_value_{}: usize = {};",
        line_count - 1,
        line_count - 1
    );
    let changed_new_line = format!(
        "let diff_value_{}: usize = {};",
        line_count - 1,
        line_count * 2
    );
    let old_text = (0..line_count)
        .map(|ix| format!("let diff_value_{ix}: usize = {ix};"))
        .collect::<Vec<_>>()
        .join("\n");
    let new_text = (0..line_count)
        .map(|ix| {
            if ix + 1 == line_count {
                changed_new_line.clone()
            } else {
                format!("let diff_value_{ix}: usize = {ix};")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "large file-diff prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Rust)
                && pane.file_diff_old_text.len() == old_text.len()
                && pane.file_diff_old_line_starts.len() == line_count
                && pane.file_diff_new_text.len() == new_text.len()
                && pane.file_diff_new_line_starts.len() == line_count
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(changed_old_line.as_str()))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(changed_new_line.as_str()))
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?} cache_inflight={:?} cache_path={:?} language={:?} old_text_len={} old_line_starts={} new_text_len={} new_line_starts={} left_doc={:?} right_doc={:?} row_count={}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_old_text.len(),
                pane.file_diff_old_line_starts.len(),
                pane.file_diff_new_text.len(),
                pane.file_diff_new_line_starts.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
            )
        },
    );
}

#[gpui::test]
fn large_file_diff_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(61);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_diff_background_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/large_file_diff_bg.rs");
    let line_count = 4_001usize;
    let mut old_lines = vec![
        "/* start block comment".to_string(),
        "still inside block comment".to_string(),
        "end */".to_string(),
    ];
    old_lines.extend((3..line_count).map(|ix| format!("let diff_bg_{ix}: usize = {ix};")));
    let comment_line = old_lines[1].clone();
    let comment_inline_text = format!(" {comment_line}");
    let old_text = old_lines.join("\n");
    let mut new_lines = old_lines.clone();
    *new_lines.last_mut().unwrap() = format!(
        "let diff_bg_{}: usize = {};",
        line_count - 1,
        line_count * 2
    );
    let new_text = new_lines.join("\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });
    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    // Wait for the file-diff cache rows to be built. The zero foreground budget
    // means syntax timed out and a background parse has been spawned.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file-diff cache build (rows populated, syntax pending)",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && !pane.file_diff_cache_rows.is_empty()
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} rows={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    // Right after the cache build, the foreground syntax timed out (zero budget),
    // so the prepared syntax documents should not yet exist.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none(),
            "zero foreground budget should force left syntax into the background"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_none(),
            "zero foreground budget should force right syntax into the background"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let (split_epoch_after_first_draw, fallback_split_highlights_hash) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let styled = file_diff_split_cached_styled(
                &pane,
                DiffTextRegion::SplitLeft,
                comment_line.as_str(),
            )
            .expect("initial wait should populate the visible fallback split row cache");
            assert_eq!(
                styled.text.as_ref(),
                comment_line,
                "expected the cached split row to match the multiline comment text"
            );
            if styled.highlights.is_empty() {
                assert!(
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                        .is_none(),
                    "the first split draw should still be using the plain-text fallback before the background parse is applied"
                );
                assert!(
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                        .is_none(),
                    "the first split draw should still be using the plain-text fallback before the background parse is applied"
                );
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    Some(styled.highlights_hash),
                )
            } else {
                assert!(
                    styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    }),
                    "if the background parse wins the race before the first split draw, the cached split row should already be syntax highlighted"
                );
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    None,
                )
            }
        });

    // Wait for the background syntax parse to complete.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file-diff background syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            let left_epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, &comment_line)
                    .is_some_and(|styled| {
                        let upgraded_from_fallback = fallback_split_highlights_hash
                            .map(|hash| {
                                left_epoch > split_epoch_after_first_draw
                                    && styled.highlights_hash != hash
                            })
                            .unwrap_or(true);
                        upgraded_from_fallback
                            && styled.highlights.iter().any(|(range, style)| {
                                range.start == 0
                                    && range.end == comment_line.len()
                                    && style.color == Some(pane.theme.colors.text_muted.into())
                            })
                    })
        },
        |pane| {
            let left_epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, &comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_epoch={} split_epoch_after_first_draw={split_epoch_after_first_draw} fallback_split_highlights_hash={fallback_split_highlights_hash:?} split_cached={split_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                left_epoch,
            )
        },
    );

    // Verify both old and new sides have valid document-backed syntax sessions.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let split_styled = file_diff_split_cached_styled(
            &pane,
            DiffTextRegion::SplitLeft,
            comment_line.as_str(),
        )
            .expect("background syntax completion should repopulate the split row cache");
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some(),
            "background parse should produce the left (old) prepared syntax document"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            "background parse should produce the right (new) prepared syntax document"
        );
        if let Some(initial_split_highlights_hash) = fallback_split_highlights_hash {
            assert!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft)
                    > split_epoch_after_first_draw,
                "background syntax completion should bump the left style cache epoch after the plain-text fallback draw"
            );
            assert_ne!(
                split_styled.highlights_hash, initial_split_highlights_hash,
                "background syntax should replace the plain-text split row styling"
            );
        }
        assert!(
            split_styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "split comment row should upgrade to comment highlighting after background parsing"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file-diff inline projection after background syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .is_some_and(|styled| {
                styled.text.as_ref() == comment_line
                    && styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    })
            })
        },
        |pane| {
            let inline_cached = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "inline_doc_left={:?} inline_doc_right={:?} inline_cached={inline_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );
}

#[gpui::test]
fn edited_large_file_diff_reparses_incrementally_in_background_after_timeout(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(64);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_edited_large_file_diff_background_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/edited_large_file_diff_bg.rs");
    let comment_line = "still inside block comment";
    let comment_inline_text = format!(" {comment_line}");
    let inserted_prefix = format!("/* start block comment\n{comment_line}\nend */\n");
    let line_count = 8_001usize;

    let mut old_lines = vec![
        "fn edited_demo() {".to_string(),
        "    let kept = 1;".to_string(),
        "}".to_string(),
    ];
    old_lines.extend((3..line_count).map(|ix| format!("let edited_bg_{ix}: usize = {ix};")));
    let old_text_v1 = old_lines.join("\n");
    let mut new_lines = old_lines.clone();
    *new_lines
        .last_mut()
        .expect("fixture should have a tail line") = format!(
        "let edited_bg_{}: usize = {};",
        line_count - 1,
        line_count * 2
    );
    let new_text_v1 = new_lines.join("\n");
    let old_text_v2 = format!("{inserted_prefix}{old_text_v1}");
    let new_text_v2 = format!("{inserted_prefix}{new_text_v1}");

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        &old_text_v1,
        &new_text_v1,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff initial syntax ready",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Rust)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    let (initial_left_version, initial_right_version) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .expect("initial left syntax document should be ready");
        let right_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            .expect("initial right syntax document should be ready");
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(left_document),
            Some(rows::PreparedDiffSyntaxParseMode::Full),
            "the first file-diff prepare should start from a full parse without a prior document seed"
        );
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(right_document),
            Some(rows::PreparedDiffSyntaxParseMode::Full),
            "the first file-diff prepare should start from a full parse without a prior document seed"
        );
        (
            rows::prepared_diff_syntax_source_version(left_document)
                .expect("initial left document should have a source version"),
            rows::prepared_diff_syntax_source_version(right_document)
                .expect("initial right document should have a source version"),
        )
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        2,
        &old_text_v2,
        &new_text_v2,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff cache rebuild for new revision",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane
                    .file_diff_old_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str())
                && pane
                    .file_diff_new_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str())
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(comment_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(comment_line))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} old_prefix={} new_prefix={} row_count={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_old_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str()),
                pane.file_diff_new_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str()),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff split comment row cached",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line).is_some()
        },
        |pane| {
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_epoch={} split_cached={split_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
            )
        },
    );

    let (split_epoch_after_first_draw, fallback_split_highlights_hash) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let styled = file_diff_split_cached_styled(
                &pane,
                DiffTextRegion::SplitLeft,
                comment_line,
            )
            .expect("edited split comment row should be cached before background completion wait");
            assert_eq!(
                styled.text.as_ref(),
                comment_line,
                "expected the cached split row to match the edited multiline comment text"
            );
            if styled.highlights.is_empty() {
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    Some(styled.highlights_hash),
                )
            } else {
                assert!(
                    styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    }),
                    "if the background parse wins the race before the first observable split cache fill, the cached edited row should already be syntax highlighted"
                );
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    None,
                )
            }
        });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff background incremental syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            let Some(left_document) =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            else {
                return false;
            };
            let Some(right_document) =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            else {
                return false;
            };
            let left_epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
            rows::prepared_diff_syntax_parse_mode(left_document)
                == Some(rows::PreparedDiffSyntaxParseMode::Incremental)
                && rows::prepared_diff_syntax_parse_mode(right_document)
                    == Some(rows::PreparedDiffSyntaxParseMode::Incremental)
                && rows::prepared_diff_syntax_source_version(left_document)
                    .is_some_and(|version| version > initial_left_version)
                && rows::prepared_diff_syntax_source_version(right_document)
                    .is_some_and(|version| version > initial_right_version)
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .is_some_and(|styled| {
                        let upgraded_from_fallback = fallback_split_highlights_hash
                            .map(|hash| {
                                left_epoch > split_epoch_after_first_draw
                                    && styled.highlights_hash != hash
                            })
                            .unwrap_or(true);
                        upgraded_from_fallback
                            && styled.highlights.iter().any(|(range, style)| {
                                range.start == 0
                                    && range.end == comment_line.len()
                                    && style.color == Some(pane.theme.colors.text_muted.into())
                            })
                    })
        },
        |pane| {
            let left_document =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_document =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={left_document:?} right_doc={right_document:?} left_mode={:?} right_mode={:?} left_version={:?} right_version={:?} left_epoch={} split_epoch_after_first_draw={split_epoch_after_first_draw} fallback_split_highlights_hash={fallback_split_highlights_hash:?} split_cached={split_cached:?}",
                left_document.and_then(rows::prepared_diff_syntax_parse_mode),
                right_document.and_then(rows::prepared_diff_syntax_parse_mode),
                left_document.and_then(rows::prepared_diff_syntax_source_version),
                right_document.and_then(rows::prepared_diff_syntax_source_version),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .expect("background reparse should produce the edited left syntax document");
        let right_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            .expect("background reparse should produce the edited right syntax document");
        let split_styled = file_diff_split_cached_styled(
            &pane,
            DiffTextRegion::SplitLeft,
            comment_line,
        )
        .expect("background reparse should repopulate the edited split row cache");
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(left_document),
            Some(rows::PreparedDiffSyntaxParseMode::Incremental),
            "the edited left document should reuse the previous tree during background reparsing"
        );
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(right_document),
            Some(rows::PreparedDiffSyntaxParseMode::Incremental),
            "the edited right document should reuse the previous tree during background reparsing"
        );
        assert!(
            rows::prepared_diff_syntax_source_version(left_document)
                .is_some_and(|version| version > initial_left_version),
            "the edited left document should advance its source version after incremental reparsing"
        );
        assert!(
            rows::prepared_diff_syntax_source_version(right_document)
                .is_some_and(|version| version > initial_right_version),
            "the edited right document should advance its source version after incremental reparsing"
        );
        if let Some(initial_split_highlights_hash) = fallback_split_highlights_hash {
            assert!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft)
                    > split_epoch_after_first_draw,
                "background syntax completion should bump the edited left style cache epoch after the fallback draw"
            );
            assert_ne!(
                split_styled.highlights_hash, initial_split_highlights_hash,
                "background syntax should replace the fallback split row styling after the edited revision rebuild"
            );
        }
        assert!(
            split_styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "the edited split comment row should upgrade to comment highlighting after incremental background parsing"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff inline projection after incremental background syntax",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .is_some_and(|styled| {
                styled.text.as_ref() == comment_line
                    && styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    })
            })
        },
        |pane| {
            let inline_cached = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_mode={:?} right_mode={:?} inline_cached={inline_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .and_then(rows::prepared_diff_syntax_parse_mode),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .and_then(rows::prepared_diff_syntax_parse_mode),
            )
        },
    );
}

#[gpui::test]
fn file_diff_background_left_syntax_upgrade_preserves_right_cached_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(65);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_one_sided_file_diff_background_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/one_sided_file_diff_bg.rs");
    let next_rev = 2u64;
    let rebuild_timeout = std::time::Duration::from_secs(30);

    let initial_old_text = "fn before_change() {}\n";
    let top_right_line = "fn stable_top() { let keep_top: usize = 1; }";
    let cached_right_line = "let stable_cached_right_90: usize = 90;";
    let mut new_lines = vec![top_right_line.to_string()];
    new_lines.extend((1..120).map(|ix| {
        if ix == 90 {
            cached_right_line.to_string()
        } else {
            format!("let stable_right_{ix}: usize = {ix};")
        }
    }));
    let new_text = new_lines.join("\n");

    let comment_line = "still inside block comment";
    let mut updated_old_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
    ];
    updated_old_lines.extend((3..12_001).map(|ix| {
        format!(
            "let one_sided_background_{ix}: Option<Result<Vec<usize>, usize>> = Some(Ok(vec![{ix}, {ix} + 1, {ix} + 2]));"
        )
    }));
    let updated_old_text = updated_old_lines.join("\n");

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        initial_old_text,
        &new_text,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "initial one-sided file-diff syntax ready",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let right_document = pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .expect("initial right syntax document should be ready before preseeding");
                let original_rev = pane.file_diff_cache_rev;
                pane.file_diff_cache_rev = next_rev;
                let next_right_key = pane
                    .file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight)
                    .expect(
                        "future right key should be available while the file-diff cache is built",
                    );
                pane.file_diff_cache_rev = original_rev;
                pane.prepared_syntax_documents
                    .insert(next_right_key, right_document);
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        next_rev,
        &updated_old_text,
        &new_text,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff rebuild (left pending, right ready)",
        rebuild_timeout,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == next_rev
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane
                    .file_diff_old_text
                    .as_ref()
                    .starts_with("/* start block comment")
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(comment_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(top_right_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(cached_right_line))
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} left_doc={:?} right_doc={:?} rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    let cached_right_row_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        file_diff_split_row_ix(&pane, DiffTextRegion::SplitRight, cached_right_line)
            .expect("expected the cached right row to exist in the rebuilt split diff")
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(cached_right_row_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff cached lower right row",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none()
                && file_diff_split_cached_styled(
                    pane,
                    DiffTextRegion::SplitRight,
                    cached_right_line,
                )
                .is_some()
        },
        |pane| {
            let cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} cached_right={cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff cached top right row",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none()
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .is_some()
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .is_some()
        },
        |pane| {
            let top_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .map(styled_debug_info_with_styles);
            let lower_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} top_cached={top_cached:?} lower_cached={lower_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    let (
        left_epoch_before,
        right_epoch_before,
        top_right_hash,
        cached_right_hash,
        left_fallback_hash,
    ) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none(),
            "left syntax should still be pending while the right-side cache is warmed"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            "the preseeded right syntax document should stay ready"
        );

        let top_cached =
            file_diff_split_cached_styled(&pane, DiffTextRegion::SplitRight, top_right_line)
                .expect(
                    "expected the top right row to be cached before left background completion",
                );
        let lower_cached = file_diff_split_cached_styled(
            &pane,
            DiffTextRegion::SplitRight,
            cached_right_line,
        )
        .expect(
            "expected the offscreen right row to remain cached before left background completion",
        );
        let left_fallback =
            file_diff_split_cached_styled(&pane, DiffTextRegion::SplitLeft, comment_line).expect(
                "expected the pending left comment row to be cached before background completion",
            );
        assert!(
            !top_cached.highlights.is_empty(),
            "the preseeded top right row should already be syntax highlighted"
        );
        assert!(
            !lower_cached.highlights.is_empty(),
            "the preseeded offscreen right row should already be syntax highlighted"
        );

        (
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            top_cached.highlights_hash,
            lower_cached.highlights_hash,
            left_fallback.highlights_hash,
        )
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff background left syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some()
                && pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft)
                    > left_epoch_before
                && pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight)
                    == right_epoch_before
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .is_some_and(|styled| styled.highlights_hash == top_right_hash)
                && file_diff_split_cached_styled(
                    pane,
                    DiffTextRegion::SplitRight,
                    cached_right_line,
                )
                .is_some_and(|styled| styled.highlights_hash == cached_right_hash)
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .is_some_and(|styled| styled.highlights_hash != left_fallback_hash)
        },
        |pane| {
            let top_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .map(styled_debug_info_with_styles);
            let lower_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                    .map(styled_debug_info_with_styles);
            let left_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_epoch={} right_epoch={} top_cached={top_cached:?} lower_cached={lower_cached:?} left_cached={left_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let top_cached = file_diff_split_cached_styled(
            &pane,
            DiffTextRegion::SplitRight,
            top_right_line,
        )
        .expect("top right row should remain cached after left background completion");
        let lower_cached = file_diff_split_cached_styled(
            &pane,
            DiffTextRegion::SplitRight,
            cached_right_line,
        )
        .expect("offscreen right row should remain cached after left background completion");
        let left_cached = file_diff_split_cached_styled(
            &pane,
            DiffTextRegion::SplitLeft,
            comment_line,
        )
        .expect("left comment row should be cached after background completion");

        assert_eq!(
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            right_epoch_before,
            "left-only background syntax completion should not bump the right-side cache epoch"
        );
        assert_eq!(
            top_cached.highlights_hash, top_right_hash,
            "the visible right row should keep its cached styling when only the left side upgrades"
        );
        assert_eq!(
            lower_cached.highlights_hash, cached_right_hash,
            "the offscreen right row should survive left-only syntax completion without a cache clear"
        );
        assert_ne!(
            left_cached.highlights_hash, left_fallback_hash,
            "the left comment row should replace its pending fallback styling after the background parse"
        );
    });
}

#[gpui::test]
fn large_conflict_resolved_output_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_conflict_resolved_output_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/large_conflict_resolved_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let comment_line = "still inside block comment";
    let fixture_line_count = 20_001usize;

    let mut base_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
        "let chosen = 0;".to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count).map(|ix| format!("let base_bg_{ix}: usize = {ix};")),
    );
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[3] = "let chosen = 1;".to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[3] = "let chosen = 2;".to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
        "<<<<<<< ours".to_string(),
        "let chosen = 1;".to_string(),
        "=======".to_string(),
        "let chosen = 2;".to_string(),
        ">>>>>>> theirs".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count)
            .map(|ix| format!("let resolved_bg_{ix}: usize = {ix};")),
    );
    let current_text = current_lines.join("\n");
    let resolved_output = crate::view::conflict_resolver::generate_resolved_text(
        crate::view::conflict_resolver::parse_conflict_markers(&current_text).as_slice(),
    );
    let line_count = resolved_output.lines().count();
    assert!(
        fixture_line_count > rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING,
        "fixture should stay above the old syntax gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict resolver fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict resolver fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

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
                    base: Some(base_text.clone()),
                    ours: Some(ours_text.clone()),
                    theirs: Some(theirs_text.clone()),
                    current: Some(current_text.clone()),
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

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large conflict resolved output initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| {
            format!(
                "path={:?} line_count={} syntax_language={:?} prepared_document={:?} source_hash={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_syntax_language,
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_source_hash,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
                assert_eq!(
                    pane.conflict_resolved_preview_line_count, line_count,
                    "forced recompute should materialize the expected resolved output line count"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_syntax_language,
                    Some(rows::DiffSyntaxLanguage::Rust),
                    "resolved output should still use the file-derived Rust syntax language"
                );
                assert!(
                    pane.conflict_resolved_preview_prepared_syntax_document.is_none(),
                    "zero foreground budget should leave resolved-output syntax pending until the background parse completes"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    let (initial_epoch, initial_highlights_hash) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("initial draw should populate the visible fallback resolved-output row cache");
        assert_eq!(
            styled.text.as_ref(),
            comment_line,
            "expected the cached resolved-output row to match the multiline comment text"
        );
        assert!(
            styled.highlights.is_empty(),
            "before the background parse completes, the multiline comment row should render as plain text"
        );
        (pane.conflict_resolved_preview_style_cache_epoch, styled.highlights_hash)
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large conflict resolved output background syntax upgrade",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some()
                && pane.conflict_resolved_preview_style_cache_epoch > initial_epoch
                && pane
                    .conflict_resolved_preview_segments_cache_get(target_ix)
                    .is_some_and(|styled| {
                        styled.highlights.iter().any(|(range, style)| {
                            range.start == 0
                                && range.end == comment_line.len()
                                && style.color == Some(pane.theme.colors.text_muted.into())
                        })
                    })
        },
        |pane| {
            let row_cache = pane
                .conflict_resolved_preview_segments_cache_get(target_ix)
                .map(styled_debug_info_with_styles);
            format!(
                "prepared_document={:?} style_epoch={} line_count={} row_cache={row_cache:?}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("background syntax completion should repopulate the resolved-output row cache");
        assert_ne!(
            styled.highlights_hash, initial_highlights_hash,
            "background syntax should replace the plain-text fallback row styling"
        );
        assert!(
            styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "multiline comment row should upgrade to comment highlighting after background parsing"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict resolver fixture");
}

#[gpui::test]
fn edited_conflict_resolved_output_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_edited_conflict_resolved_output_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/edited_conflict_resolved_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let inserted_comment_line = "still inside block comment";
    let inserted_prefix = format!("/* start block comment\n{inserted_comment_line}\nend */\n");
    let fixture_line_count = 20_001usize;

    let mut base_lines = vec![
        "fn large_demo() {".to_string(),
        "    let chosen = 0;".to_string(),
        "    let tail = 9;".to_string(),
        "}".to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count).map(|ix| format!("let base_bg_{ix}: usize = {ix};")),
    );
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[1] = "    let chosen = 1;".to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[1] = "    let chosen = 2;".to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        "fn large_demo() {".to_string(),
        "<<<<<<< ours".to_string(),
        "    let chosen = 1;".to_string(),
        "=======".to_string(),
        "    let chosen = 2;".to_string(),
        ">>>>>>> theirs".to_string(),
        "    let tail = 9;".to_string(),
        "}".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count)
            .map(|ix| format!("let resolved_bg_{ix}: usize = {ix};")),
    );
    let current_text = current_lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict resolver fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict resolver fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
            });

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
                    base: Some(base_text.clone()),
                    ours: Some(ours_text.clone()),
                    theirs: Some(theirs_text.clone()),
                    current: Some(current_text.clone()),
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

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| {
            format!(
                "path={:?} line_count={} prepared_document={:?} source_hash={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_source_hash,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output initial syntax ready",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some()
                && pane.conflict_resolved_preview_syntax_language
                    == Some(rows::DiffSyntaxLanguage::Rust)
        },
        |pane| {
            format!(
                "prepared_document={:?} style_epoch={} syntax_language={:?} line_count={}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_syntax_language,
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    let initial_epoch = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some(),
            "initial recompute should build a prepared syntax document before the edit"
        );
        pane.conflict_resolved_preview_style_cache_epoch
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
                pane.conflict_resolver_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, &inserted_prefix, cx);
                });
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output falls back to plain text while background syntax reparses",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_text
                .as_ref()
                .starts_with(inserted_prefix.as_str())
                && pane
                    .conflict_resolved_preview_prepared_syntax_document
                    .is_none()
                && pane.conflict_resolved_preview_style_cache_epoch > initial_epoch
        },
        |pane| {
            let preview_prefix: Vec<&str> = pane
                .conflict_resolved_preview_text
                .as_ref()
                .lines()
                .take(3)
                .collect();
            format!(
                "preview_prefix={preview_prefix:?} prepared_document={:?} style_epoch={} initial_epoch={initial_epoch} inflight={:?}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_syntax_inflight,
            )
        },
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    let (pending_epoch, pending_highlights_hash) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("edit redraw should populate the visible fallback resolved-output row cache");
        assert_eq!(
            styled.text.as_ref(),
            inserted_comment_line,
            "expected the cached resolved-output row to reflect the inserted comment continuation line"
        );
        assert!(
            styled.highlights.is_empty(),
            "while the edited document reparses in the background, the continuation row should render as plain text"
        );
        (
            pane.conflict_resolved_preview_style_cache_epoch,
            styled.highlights_hash,
        )
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output background syntax upgrade",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some()
                && pane.conflict_resolved_preview_style_cache_epoch > pending_epoch
                && pane
                    .conflict_resolved_preview_segments_cache_get(target_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == inserted_comment_line
                            && styled.highlights.iter().any(|(range, style)| {
                                range.start == 0
                                    && range.end == inserted_comment_line.len()
                                    && style.color == Some(pane.theme.colors.text_muted.into())
                            })
                    })
        },
        |pane| {
            let row_cache = pane
                .conflict_resolved_preview_segments_cache_get(target_ix)
                .map(styled_debug_info_with_styles);
            format!(
                "prepared_document={:?} style_epoch={} pending_epoch={pending_epoch} inflight={:?} row_cache={row_cache:?}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_syntax_inflight,
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("background syntax completion should repopulate the edited resolved-output row cache");
        assert_ne!(
            styled.highlights_hash, pending_highlights_hash,
            "background syntax should replace the plain-text fallback row styling after the edit"
        );
        assert!(
            styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == inserted_comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "the inserted comment continuation row should upgrade to multiline comment highlighting after background reparsing"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict resolver fixture");
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
    disable_view_poller_for_test(cx, &view);

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

    seed_file_diff_state(cx, &view, repo_id, &workdir, &file_rel, old_text, new_text);

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
    cx.run_until_parked();
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
            let preview_lines = Arc::new(vec![
                "# Title".to_string(),
                "".to_string(),
                "preview body".to_string(),
            ]);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    preview_lines,
                    "# Title\n\npreview body".len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
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
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::new(vec![oversized_source]),
                    oversized_len,
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
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
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    row_limit_source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.ensure_single_markdown_preview_cache(cx);
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
                pane.worktree_preview_text = "preview".into();
                pane.worktree_preview_line_starts = Arc::from(vec![0usize]);
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
                && pane.worktree_preview_text.is_empty()
                && pane.worktree_preview_line_starts.is_empty()
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
                "worktree_path={:?} worktree_rev={} worktree_text_len={} worktree_line_starts={} worktree_markdown_path={:?} worktree_markdown_rev={} worktree_markdown_inflight={:?} worktree_markdown_not_loaded={}",
                pane.worktree_preview_path,
                pane.worktree_preview_content_rev,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_starts.len(),
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
    disable_view_poller_for_test(cx, &view);

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
            .and_then(|entry| entry.as_ref().map(|entry| &entry.styled))
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
            .and_then(|entry| entry.as_ref().map(|entry| &entry.styled))
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
            .and_then(|entry| entry.as_ref().map(|entry| &entry.styled))
            .expect("expected query overlay cache entry for searched context row");
        assert_ne!(
            query_overlay.highlights_hash, stable_after.highlights_hash,
            "query overlay should layer match highlighting on top of stable highlights"
        );
    });
}

#[gpui::test]
fn worktree_preview_search_query_clears_row_cache_without_dropping_source_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(23);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_search",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let lines: Arc<Vec<String>> = Arc::new(vec![
        "fn needle() { let value = 1; }".to_string(),
        "fn keep() { let other = 2; }".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(&preview_abs_path, &preview_text).expect("write preview fixture file");

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
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree preview row cache before enabling search",
        |pane| {
            pane.worktree_preview_segments_cache_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_segments_cache_get(0).is_some()
        },
        |pane| {
            format!(
                "preview_path={:?} cache_path={:?} row_cache_present={} line_count={:?}",
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_segments_cache_path.clone(),
                pane.worktree_preview_segments_cache_get(0).is_some(),
                pane.worktree_preview_line_count(),
            )
        },
    );

    let mut base_highlights_hash = 0u64;
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        assert_eq!(
            pane.worktree_preview_segments_cache_path.as_ref(),
            Some(&preview_abs_path),
            "initial draw should bind the preview row cache to the current path"
        );
        let base = pane
            .worktree_preview_segments_cache_get(0)
            .expect("expected worktree preview row cache before enabling search");
        base_highlights_hash = base.highlights_hash;
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_search_active = true;
            pane.diff_search_input.update(cx, |input, cx| {
                input.set_text("needle", cx);
            });
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "needle");
        assert_eq!(
            pane.worktree_preview_segments_cache_path.as_ref(),
            Some(&preview_abs_path),
            "search query changes should preserve the bound preview source path"
        );
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let searched = pane
            .worktree_preview_segments_cache_get(0)
            .expect("expected worktree preview row cache after search query rebuild");
        assert_ne!(
            searched.highlights_hash, base_highlights_hash,
            "search overlay should change the cached preview row highlights"
        );
        assert!(
            searched
                .highlights
                .iter()
                .any(|(_, style)| style.background_color.is_some()),
            "searched preview row should include a query highlight background"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn worktree_preview_identical_refresh_preserves_row_cache(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(24);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_refresh_preserves_cache",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_refresh.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let lines: Arc<Vec<String>> = Arc::new(vec![
        "fn keep() { let value = 1; }".to_string(),
        "fn also_keep() { let other = 2; }".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(&preview_abs_path, &preview_text).expect("write preview fixture file");

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
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree preview row cache before identical refresh",
        |pane| {
            pane.worktree_preview_segments_cache_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_segments_cache_get(0).is_some()
        },
        |pane| {
            format!(
                "preview_path={:?} cache_path={:?} row_cache_present={} style_epoch={}",
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_segments_cache_path.clone(),
                pane.worktree_preview_segments_cache_get(0).is_some(),
                pane.worktree_preview_style_cache_epoch,
            )
        },
    );

    let mut base_highlights_hash = 0u64;
    let mut base_style_epoch = 0u64;
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let base = pane
            .worktree_preview_segments_cache_get(0)
            .expect("expected worktree preview row cache before identical refresh");
        base_highlights_hash = base.highlights_hash;
        base_style_epoch = pane.worktree_preview_style_cache_epoch;
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let refreshed = pane
            .worktree_preview_segments_cache_get(0)
            .expect("identical refresh should preserve the cached preview row");
        assert_eq!(
            pane.worktree_preview_segments_cache_path.as_ref(),
            Some(&preview_abs_path),
            "identical refresh should keep the preview cache bound to the current source"
        );
        assert_eq!(
            pane.worktree_preview_style_cache_epoch, base_style_epoch,
            "identical refresh should not bump the preview syntax/style epoch"
        );
        assert_eq!(
            refreshed.highlights_hash, base_highlights_hash,
            "identical refresh should preserve the existing cached row styling"
        );
    });

    // Phase 2: refresh with different content — cache must be invalidated.
    let changed_lines: Arc<Vec<String>> = Arc::new(vec![
        "fn changed() { let x = 99; }".to_string(),
        "fn also_changed() { let y = 100; }".to_string(),
    ]);
    let changed_text = changed_lines.join("\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let changed_lines = Arc::clone(&changed_lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    changed_lines,
                    changed_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        assert_ne!(
            pane.worktree_preview_style_cache_epoch, base_style_epoch,
            "changed source should bump the preview syntax/style epoch"
        );
        assert!(
            pane.worktree_preview_segments_cache_get(0).is_none(),
            "changed source should clear the cached preview rows"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
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
        assert!(
            matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Ready(_)
            ),
            "expected worktree preview to be ready"
        );
        assert_eq!(pane.worktree_preview_line_count(), Some(3));
        assert_eq!(pane.worktree_preview_line_text(0), Some("one"));
        assert_eq!(pane.worktree_preview_line_text(1), Some("two"));
        assert_eq!(pane.worktree_preview_line_text(2), Some(""));
    });
}

#[gpui::test]
fn untracked_markdown_file_preview_defaults_to_preview_mode_and_renders_container(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
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

    wait_for_main_pane_condition(
        cx,
        &view,
        "untracked markdown preview activation",
        |pane| pane.is_file_preview_active() && pane.is_markdown_preview_active(),
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?} is_file_preview_active={} is_markdown_preview_active={}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.is_file_preview_active(),
                pane.is_markdown_preview_active(),
            )
        },
    );

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
