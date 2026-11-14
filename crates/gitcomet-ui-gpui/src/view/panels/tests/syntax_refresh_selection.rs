//! A real status selection reload must not abandon a source-backed syntax worker.

use super::*;
use crate::view::panes::diff::input::DiffPaneInput;
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_state::model::Loadable;
use palette::IntoColor;
use std::sync::atomic::AtomicBool;

#[gpui::test]
fn staged_reselection_keeps_source_backed_syntax(cx: &mut gpui::TestAppContext) {
    assert_reselection_keeps_source_backed_syntax(cx, DiffArea::Staged);
}

#[gpui::test]
fn unstaged_reselection_keeps_source_backed_syntax(cx: &mut gpui::TestAppContext) {
    assert_reselection_keeps_source_backed_syntax(cx, DiffArea::Unstaged);
}

/// Wait on the real store, without driving GPUI's queued syntax task. Store
/// barriers acknowledge reductions; the predicate also waits for Gix effects.
fn wait_for_selected_file(
    store: &AppStore,
    target: &DiffTarget,
    previous_revision: Option<u64>,
) -> Arc<AppState> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        store
            .barrier_for_test()
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("store must process the selection");
        let snapshot = store.snapshot();
        let diff = &snapshot.repos[0].diff_state;
        if diff.diff_target.as_ref() == Some(target)
            && matches!(diff.diff, Loadable::Ready(_))
            && matches!(diff.diff_file, Loadable::Ready(Some(_)))
            && previous_revision.is_none_or(|revision| diff.diff_file_rev > revision)
        {
            return snapshot;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "selection did not load {target:?}: {diff:?}"
        );
        std::thread::yield_now();
    }
}

fn assert_reselection_keeps_source_backed_syntax(cx: &mut gpui::TestAppContext, area: DiffArea) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("selection syntax fixture");
        let workdir = directory.path();
        let git = |args: &[&str]| crate::test_support::git(workdir, args);
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        let text = |value| {
            format!(
                "fn fixture() {{\n    /* a comment\n       across lines */\n    let value = {value};\n}}\n"
            )
        };
        std::fs::write(workdir.join("fixture.rs"), text(0)).unwrap();
        git(&["add", "fixture.rs"]);
        git(&["commit", "-q", "-m", "baseline"]);
        std::fs::write(workdir.join("fixture.rs"), text(1)).unwrap();
        git(&["add", "fixture.rs"]);
        std::fs::write(workdir.join("fixture.rs"), text(2)).unwrap();

        let repository = gitcomet_git_gix::GixBackend.open(workdir).unwrap();
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        store.disable_repo_monitors_for_test();
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::super::GitCometView::new(store.clone(), events, None, window, cx)
        });
        let repo_id = gitcomet_state::model::RepoId(754);
        let mut repo = opening_repo_state(repo_id, workdir);
        repo.open = Loadable::Ready(());
        repo.status = Loadable::Ready(Arc::new(
            repository.status(&CancellationToken::new()).unwrap(),
        ));
        store.replace_snapshot_for_test(app_state_with_repo(repo, repo_id));
        store.insert_repo_for_test(repo_id, repository);
        let target = DiffTarget::working_tree("fixture.rs".into(), area);
        store.dispatch(Msg::SelectDiff {
            repo_id,
            target: target.clone(),
        });
        let initial = wait_for_selected_file(&store, &target, None);
        let initial_diff = &initial.repos[0].diff_state;
        let original_revision = initial_diff.diff_file_rev;
        let original_target_revision = initial_diff.diff_target_rev;
        let Loadable::Ready(Some(original)) = &initial_diff.diff_file else {
            unreachable!()
        };
        assert!(original.old.is_none() && original.new.is_none());
        assert!(original.old_source.is_some() && original.new_source.is_some());

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, _| {
                    pane.diff_view = DiffViewMode::Split;
                    pane.set_eager_source_backed_syntax_prepare_for_tests(false);
                });
                push_test_state(this, Arc::clone(&initial), cx);
            });
        });
        wait_for_main_pane_condition(
            cx,
            &view,
            "real source-backed selection rows",
            |pane| {
                pane.file_diff_cache_rev == original_revision
                    && pane.file_diff_cache_path.is_some()
                    && pane.file_diff_cache_inflight.is_none()
            },
            |pane| {
                format!(
                    "revision={} pending={:?}",
                    pane.file_diff_cache_rev, pane.file_diff_cache_inflight
                )
            },
        );

        // Finish the old side first. The visible failure is asymmetric: one
        // side has a whole-document tree while the other's task is pending.
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitLeft, cx);
                });
            });
        });
        wait_for_main_pane_condition(
            cx,
            &view,
            "completed old-side syntax",
            |pane| {
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
            },
            |pane| format!("pending={:?}", pane.file_diff_click_syntax_inflight),
        );

        let worker_prepared = Arc::new(AtomicBool::new(false));
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let (rows_generation, syntax_generation) =
                    this.diff_renderer.update(cx, |pane, cx| {
                        pane.set_eager_source_backed_syntax_prepare_for_tests(true);
                        let prepared = Arc::clone(&worker_prepared);
                        pane.file_diff_click_syntax_after_prepare_hook =
                            Some(Arc::new(move || {
                                prepared.store(true, Ordering::SeqCst);
                            }));
                        pane.request_file_diff_click_syntax_document(
                            DiffTextRegion::SplitRight,
                            cx,
                        );
                        assert!(
                            pane.file_diff_click_syntax_inflight
                                .contains_key(&DiffTextRegion::SplitRight)
                        );
                        (pane.file_diff_cache_seq, pane.file_diff_syntax_generation)
                    });

                // The same action dispatched by a status-row selection reloads
                // through Loading. Real Gix completion creates the new revision;
                // the test never edits revisions or fabricates completion data.
                // Remain in this GPUI turn until that snapshot is applied, so
                // the syntax worker cannot win a machine-dependent timing race.
                store.dispatch(Msg::SelectDiff {
                    repo_id,
                    target: target.clone(),
                });
                let reloaded = wait_for_selected_file(&store, &target, Some(original_revision));
                let reloaded_diff = &reloaded.repos[0].diff_state;
                assert_eq!(reloaded_diff.diff_target_rev, original_target_revision);
                assert_eq!(reloaded_diff.diff_file_rev, original_revision + 1);
                let Loadable::Ready(Some(reloaded_file)) = &reloaded_diff.diff_file else {
                    unreachable!()
                };
                assert_eq!(
                    reloaded_file.content_signature(),
                    original.content_signature()
                );
                push_test_state(this, Arc::clone(&reloaded), cx);
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.apply_input(DiffPaneInput::selected(reloaded), cx);
                    pane.ensure_file_diff_cache(cx);
                    assert_eq!(pane.file_diff_cache_rev, original_revision + 1);
                    assert_eq!(pane.file_diff_cache_seq, rows_generation);
                    assert_eq!(pane.file_diff_syntax_generation, syntax_generation);
                });
            });
        });
        cx.run_until_parked();
        assert!(
            worker_prepared.load(Ordering::SeqCst),
            "queued syntax worker must actually parse"
        );
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            for side in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight] {
                assert!(
                    pane.file_diff_split_prepared_syntax_document(side).is_some(),
                    "{area:?} re-selection stranded {side:?} syntax after the real store reload; pending={:?}",
                    pane.file_diff_click_syntax_inflight
                );
            }
        });

        // Check rendered colour, not just cache population, in every supported
        // file-diff presentation. This continuation needs the preceding opener.
        for diff_view in [DiffViewMode::Split, DiffViewMode::Inline] {
            for content_mode in [DiffContentMode::Full, DiffContentMode::Collapsed] {
                cx.update(|_window, app| {
                    view.update(app, |this, cx| {
                        this.diff_renderer.update(cx, |pane, cx| {
                            pane.diff_view = diff_view;
                            pane.clear_diff_text_style_caches();
                            cx.notify();
                        });
                    });
                });
                set_diff_content_mode_for_test(cx, &view, content_mode);
                cx.run_until_parked();
                cx.update(|window, app| {
                    rows::clear_diff_paint_log_for_tests();
                    let _ = window.draw(app);
                    let pane = view.read(app).diff_renderer.read(app);
                    let records = rows::diff_paint_log_for_tests();
                    let continuation = records.iter().filter(|record| record.text.contains("across lines")).collect::<Vec<_>>();
                    assert!(!continuation.is_empty(), "{area:?} {diff_view:?} {content_mode:?} must render the comment");
                    let regions: &[DiffTextRegion] = match diff_view {
                        DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
                        DiffViewMode::Inline => &[DiffTextRegion::Inline],
                    };
                    for region in regions {
                        assert!(
                            continuation.iter().any(|record| record.region == *region),
                            "{area:?} {content_mode:?} must paint the {region:?} comment"
                        );
                    }
                    for record in continuation {
                        let start = record.text.find("across lines").unwrap();
                        let end = start + "across lines".len();
                        assert!(record.highlights.iter().any(|(range, color, _)| {
                            range.start <= start && range.end >= end
                                && *color == Some(pane.theme.syntax.comment.into_color())
                        }), "{area:?} {diff_view:?} {content_mode:?} lost comment syntax: {record:?}");
                    }
                });
            }
        }
    });
}
