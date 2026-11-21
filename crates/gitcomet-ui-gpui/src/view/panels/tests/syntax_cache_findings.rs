//! Regression coverage for file-diff syntax cache retention and recovery.

use super::*;
use crate::view::panes::diff::input::DiffPaneInput;
use gitcomet_core::domain::{Diff, DiffArea, FileDiffText, FileDiffTextSource, FileStatusKind};
use gitcomet_state::model::{Loadable, RepoId};
use palette::IntoColor;
use std::path::{Path, PathBuf};

pub(super) type View = gpui::Entity<super::super::GitCometView>;

pub(super) fn fixture_text(value: usize) -> String {
    format!(
        "fn fixture() {{\n    /* a comment\n       across lines */\n    let value = {value};\n}}\n"
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn file_diff_state(
    repo_id: RepoId,
    workdir: &Path,
    path: &str,
    rev: u64,
    old_source: &Path,
    new_source: &Path,
    old_value: usize,
    new_value: usize,
) -> Arc<AppState> {
    let path = PathBuf::from(path);
    let mut repo = opening_repo_state(repo_id, workdir);
    set_test_file_status(
        &mut repo,
        path.clone(),
        FileStatusKind::Modified,
        DiffArea::Unstaged,
    );
    let target = repo.diff_state.diff_target.clone().unwrap();
    repo.diff_state.diff_rev = rev;
    repo.diff_state.diff_state_rev = rev;
    repo.diff_state.diff = Loadable::Ready(Arc::new(Diff::from_unified(
        target,
        &format!("@@ -4 +4 @@\n-    let value = {old_value};\n+    let value = {new_value};\n"),
    )));
    repo.diff_state.diff_file_rev = rev;
    repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new_sources(
        path,
        Some(FileDiffTextSource::new(old_source.to_path_buf())),
        Some(FileDiffTextSource::new(new_source.to_path_buf())),
    ))));
    app_state_with_repo(repo, repo_id)
}

fn file_of(state: &AppState) -> Arc<FileDiffText> {
    match &state.repos[0].diff_state.diff_file {
        Loadable::Ready(Some(file)) => Arc::clone(file),
        _ => unreachable!(),
    }
}

pub(super) fn new_view(cx: &mut gpui::TestAppContext) -> (View, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Split;
            });
        });
    });
    (view, cx)
}

pub(super) fn show(cx: &mut gpui::VisualTestContext, view: &View, state: &Arc<AppState>) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| push_test_state(this, Arc::clone(state), cx));
    });
}

pub(super) fn set_eager(cx: &mut gpui::VisualTestContext, view: &View, enabled: bool) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _| {
                pane.set_eager_source_backed_syntax_prepare_for_tests(enabled);
            });
        });
    });
}

/// Rows for `rev` are installed and no side worker is pending.
pub(super) fn wait_settled(cx: &mut gpui::VisualTestContext, view: &View, rev: u64, path: &str) {
    let path = PathBuf::from(path);
    wait_for_main_pane_condition(
        cx,
        view,
        "file diff settled",
        move |pane| {
            pane.file_diff_cache_rev == rev
                && pane.file_diff_cache_inflight.is_none()
                && pane
                    .file_diff_cache_path
                    .as_deref()
                    .is_some_and(|cached| cached.ends_with(&path))
                && pane.file_diff_click_syntax_inflight.is_empty()
        },
        |pane| {
            format!(
                "rev={} rows_pending={:?} path={:?} syntax_pending={:?} error={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path,
                pane.file_diff_click_syntax_inflight,
                pane.file_diff_cache_error.is_some()
            )
        },
    );
}

/// (left document present, right document present, view map entries)
pub(super) fn sides_present(cx: &mut gpui::VisualTestContext, view: &View) -> (bool, bool, usize) {
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        (
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some(),
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            pane.prepared_syntax_documents.len(),
        )
    })
}

/// An ordinary cache check plus redraws: what the app does with no user input.
pub(super) fn idle_frames(cx: &mut gpui::VisualTestContext, view: &View) {
    for _ in 0..3 {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer
                    .update(cx, |pane, cx| pane.ensure_file_diff_cache(cx));
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
}

/// Paints the diff and reports, per split side, whether the block comment's
/// continuation line is painted in the comment colour. `fresh_rows` drops the
/// styled-row cache first, as rows scrolled into view would be built.
fn comment_continuation_coloured(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    fresh_rows: bool,
) -> (Option<bool>, Option<bool>) {
    if fresh_rows {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.clear_diff_text_style_caches();
                    cx.notify();
                });
            });
        });
    }
    cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        let _ = window.draw(app);
        let pane = view.read(app).diff_renderer.read(app);
        let comment: Option<gpui::Hsla> = Some(pane.theme.syntax.comment.into_color());
        let records = rows::diff_paint_log_for_tests();
        let side = |region: DiffTextRegion| {
            let continuation: Vec<_> = records
                .iter()
                .filter(|record| record.region == region && record.text.contains("across lines"))
                .collect();
            if continuation.is_empty() {
                return None;
            }
            Some(continuation.iter().all(|record| {
                let start = record.text.find("across lines").unwrap();
                let end = start + "across lines".len();
                record.highlights.iter().any(|(range, color, _)| {
                    range.start <= start && range.end >= end && *color == comment
                })
            }))
        };
        (
            side(DiffTextRegion::SplitLeft),
            side(DiffTextRegion::SplitRight),
        )
    })
}

#[gpui::test]
fn long_sessions_keep_both_file_diff_sides_coloured(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("session fixture");
        let workdir = directory.path().to_path_buf();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(7101);
        for i in 0..768 {
            let old = workdir.join(format!("old_{i}.rs"));
            let new = workdir.join(format!("new_{i}.rs"));
            std::fs::write(&old, fixture_text(i)).unwrap();
            std::fs::write(&new, fixture_text(i + 1)).unwrap();
            let path = format!("src/file_{i}.rs");
            let rev = i as u64 + 1;
            show(
                cx,
                &view,
                &file_diff_state(repo_id, &workdir, &path, rev, &old, &new, i, i + 1),
            );
            wait_settled(cx, &view, rev, &path);
            if i == 0 {
                set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
            }
            let (left, right, entries) = sides_present(cx, &view);
            assert!(
                left && right,
                "view {i} lost a prepared side: left={left}, right={right}"
            );
            assert!(
                entries <= 2,
                "view {i} retained {entries} unreachable file-diff documents"
            );
            draw_and_drain_test_window(cx);
            assert_eq!(
                comment_continuation_coloured(cx, &view, false),
                (Some(true), Some(true)),
                "view {i} must retain whole-document syntax on both sides"
            );
        }
    });
}

#[gpui::test]
fn content_edits_keep_only_current_file_diff_documents(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("edit fixture");
        let workdir = directory.path().to_path_buf();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(7102);
        let path = "src/edited.rs";
        let old = workdir.join("old.rs");
        std::fs::write(&old, fixture_text(0)).unwrap();
        for k in 0..20 {
            // Gix keeps each revision in an immutable source snapshot. Use a
            // fresh snapshot path so this check does not depend on mtime precision.
            let new = workdir.join(format!("new_{k}.rs"));
            std::fs::write(&new, fixture_text(100 + k)).unwrap();
            let rev = k as u64 + 1;
            show(
                cx,
                &view,
                &file_diff_state(repo_id, &workdir, path, rev, &old, &new, 0, 100 + k),
            );
            wait_settled(cx, &view, rev, path);
            let (left, right, entries) = sides_present(cx, &view);
            assert!(left && right, "edit {k} should prepare both sides");
            assert_eq!(entries, 2, "edit {k} retained an unreachable revision");
            cx.update(|_window, app| {
                let pane = view.read(app).diff_renderer.read(app);
                assert!(
                    pane.prepared_syntax_documents
                        .keys()
                        .all(|key| { key.file_path.ends_with(path) && key.target_rev == rev })
                );
            });
        }
    });
}

#[gpui::test]
fn failed_rebuild_drops_documents_for_discarded_rows(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("failed rebuild fixture");
        let workdir = directory.path().to_path_buf();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(7103);
        let path = "src/failing.rs";
        let old = workdir.join("old.rs");
        let new = workdir.join("new.rs");
        let unreadable = workdir.join("unreadable.rs");
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        std::fs::create_dir(&unreadable).unwrap();
        set_eager(cx, &view, false);
        show(
            cx,
            &view,
            &file_diff_state(repo_id, &workdir, path, 1, &old, &new, 0, 1),
        );
        wait_settled(cx, &view, 1, path);

        // A worker starts for the visible rows; the next payload cannot be indexed.
        let failing_rev2 = file_diff_state(repo_id, &workdir, path, 2, &old, &unreadable, 0, 1);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitRight, cx);
                    assert!(
                        pane.file_diff_click_syntax_inflight
                            .contains_key(&DiffTextRegion::SplitRight)
                    );
                });
                push_test_state(this, Arc::clone(&failing_rev2), cx);
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.apply_input(DiffPaneInput::selected(Arc::clone(&failing_rev2)), cx);
                    pane.ensure_file_diff_cache(cx);
                    assert_eq!(pane.file_diff_cache_rev, 2);
                    assert!(pane.file_diff_cache_inflight.is_some(), "keep-rows rebuild");
                });
            });
        });
        cx.run_until_parked();
        assert_failed_rows_have_no_documents(cx, &view, path, 2);

        // A re-selection retries the same failing payload, without carrying the
        // old rows' document forward under the next revision.
        let failing_rev3 = file_diff_state(repo_id, &workdir, path, 3, &old, &unreadable, 0, 1);
        assert_eq!(
            file_of(&failing_rev3).content_signature(),
            file_of(&failing_rev2).content_signature()
        );
        show(cx, &view, &failing_rev3);
        wait_settled(cx, &view, 3, path);
        assert_failed_rows_have_no_documents(cx, &view, path, 3);
    });
}

fn assert_failed_rows_have_no_documents(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    path: &str,
    revision: u64,
) {
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_diff_cache_rev, revision);
        assert!(pane.file_diff_cache_error.is_some());
        assert!(pane.file_diff_row_provider.is_none());
        assert!(
            !pane.prepared_syntax_documents.keys().any(|key| {
                key.file_path.ends_with(path)
                    && matches!(
                        key.view_mode,
                        PreparedSyntaxViewMode::FileDiffSplitLeft
                            | PreparedSyntaxViewMode::FileDiffSplitRight
                    )
            }),
            "discarded rows must not leave syntax under revision {revision} or its predecessors"
        );
    });
}

#[gpui::test]
fn globally_evicted_syntax_recovers_and_recolours_cached_rows(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("global eviction fixture");
        let workdir = directory.path().to_path_buf();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(7104);
        let path = "src/evicted.rs";
        let old = workdir.join("old.rs");
        let new = workdir.join("new.rs");
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        show(
            cx,
            &view,
            &file_diff_state(repo_id, &workdir, path, 1, &old, &new, 0, 1),
        );
        wait_settled(cx, &view, 1, path);
        set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
        draw_and_drain_test_window(cx);
        assert_eq!(
            comment_continuation_coloured(cx, &view, false),
            (Some(true), Some(true))
        );
        let original = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight]
                .map(|side| pane.file_diff_split_prepared_syntax_document(side).unwrap())
        });

        // With oldest-first seed eviction, 128 distinct documents exceed both
        // the 8-entry local LRU and 64-entry shared seed store deterministically.
        // Do not probe the visible handles during churn, which could rehydrate
        // a document from its seed and change local-cache recency.
        set_eager(cx, &view, false);
        for i in 0..128 {
            let source = format!("fn unrelated_{i}() {{ let x = {i}; }}\n");
            let (text, line_starts) = shared_text_and_line_starts(&source);
            let document = rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                rows::DiffSyntaxLanguage::Rust,
                rows::DiffSyntaxMode::Auto,
                text,
                line_starts,
                None,
                None,
            )
            .expect("unrelated document");
            let _ = rows::inject_background_prepared_diff_syntax_document(document);
        }
        assert!(
            original
                .into_iter()
                .all(|document| !rows::prepared_diff_syntax_document_is_available(document))
        );
        let epochs_before = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.prepared_syntax_documents.len(),
                2,
                "view map still holds the dead handles"
            );
            [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight]
                .map(|side| pane.file_diff_split_style_cache_epoch(side))
        });
        assert_eq!(
            comment_continuation_coloured(cx, &view, true),
            (Some(false), Some(false))
        );

        // A normal frame must recover both sides. Keep the fallback rows cached
        // so recolouring requires the document install to invalidate their styles.
        set_eager(cx, &view, true);
        idle_frames(cx, &view);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            for (index, side) in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    pane.file_diff_split_prepared_syntax_document(side),
                    Some(original[index]),
                    "identical bytes must revive the existing handle"
                );
                assert!(
                    pane.file_diff_split_style_cache_epoch(side) > epochs_before[index],
                    "recovery must invalidate {side:?} fallback styles"
                );
            }
        });
        assert_eq!(
            comment_continuation_coloured(cx, &view, false),
            (Some(true), Some(true))
        );
    });
}

#[gpui::test]
fn transient_syntax_read_failure_recovers_without_a_click(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("transient read fixture");
        let workdir = directory.path().to_path_buf();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(7105);
        let path = "src/locked.rs";
        let old = workdir.join("old.rs");
        let new = workdir.join("new.rs");
        let hidden = workdir.join("new.rs.hidden");
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        set_eager(cx, &view, false);
        show(
            cx,
            &view,
            &file_diff_state(repo_id, &workdir, path, 1, &old, &new, 0, 1),
        );
        wait_settled(cx, &view, 1, path);
        set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
        let indexed = cx.update(|_window, app| {
            view.read(app)
                .diff_renderer
                .read(app)
                .file_diff_new_source_identity
                .clone()
                .unwrap()
        });

        // The queued worker finds an unreadable directory. Restore the original
        // file only after that read fails, keeping its path, length and mtime.
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitRight, cx);
                    assert!(
                        pane.file_diff_click_syntax_inflight
                            .contains_key(&DiffTextRegion::SplitRight)
                    );
                });
            });
            std::fs::rename(&new, &hidden).unwrap();
            std::fs::create_dir(&new).unwrap();
        });
        cx.run_until_parked();
        assert!(!sides_present(cx, &view).1);
        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();
        assert_eq!(FileDiffTextSource::new(new.clone()).identity, indexed);

        // Eager rendering stays disabled, so only the bounded retry timer can
        // recover the failed request. No second click or revision change occurs.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(249));
        cx.run_until_parked();
        assert!(
            !sides_present(cx, &view).1,
            "retry must respect its backoff"
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(1));
        cx.run_until_parked();
        assert!(
            sides_present(cx, &view).1,
            "the first retry must recover the readable source"
        );
        draw_and_drain_test_window(cx);
        assert_eq!(
            comment_continuation_coloured(cx, &view, false).1,
            Some(true)
        );
    });
}

#[gpui::test]
fn same_content_reselection_recovers_from_indexing_failure(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("sticky error fixture");
        let workdir = directory.path().to_path_buf();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(7106);
        let path = "src/sticky.rs";
        let old = workdir.join("old.rs");
        let new = workdir.join("new.rs");
        let hidden = workdir.join("new.rs.hidden");
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        let first = file_diff_state(repo_id, &workdir, path, 1, &old, &new, 0, 1);

        std::fs::rename(&new, &hidden).unwrap();
        std::fs::create_dir(&new).unwrap();
        show(cx, &view, &first);
        wait_for_main_pane_condition(
            cx,
            &view,
            "indexing error",
            |pane| pane.file_diff_cache_rev == 1 && pane.file_diff_cache_error.is_some(),
            |pane| {
                (
                    pane.file_diff_cache_rev,
                    pane.file_diff_cache_error.is_some(),
                )
            },
        );
        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();

        let reselected = file_diff_state(repo_id, &workdir, path, 2, &old, &new, 0, 1);
        assert_eq!(
            file_of(&reselected).content_signature(),
            file_of(&first).content_signature()
        );
        show(cx, &view, &reselected);
        wait_settled(cx, &view, 2, path);
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(
                pane.file_diff_cache_error.is_none(),
                "re-selection must clear a transient indexing error"
            );
            assert!(
                pane.file_diff_row_provider.is_some(),
                "re-selection must restore the rows"
            );
        });
        assert_eq!(sides_present(cx, &view), (true, true, 2));
        set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
        draw_and_drain_test_window(cx);
        assert_eq!(
            comment_continuation_coloured(cx, &view, false),
            (Some(true), Some(true))
        );
    });
}
