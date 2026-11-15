//! Visible-side recovery must be bounded when a source cannot be prepared.

use super::syntax_cache_findings::{
    View, file_diff_state, fixture_text, idle_frames, new_view, set_eager, show, sides_present,
    wait_settled,
};
use super::*;
use gitcomet_state::model::RepoId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn prepare_fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    tempfile::TempDir,
    View,
    &mut gpui::VisualTestContext,
    PathBuf,
    PathBuf,
) {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.rs");
    let new = directory.path().join("new.rs");
    std::fs::write(&old, fixture_text(0)).unwrap();
    std::fs::write(&new, fixture_text(1)).unwrap();
    let (view, cx) = new_view(cx);
    show(
        cx,
        &view,
        &file_diff_state(
            RepoId(7190),
            directory.path(),
            "src/retry.rs",
            1,
            &old,
            &new,
            0,
            1,
        ),
    );
    wait_settled(cx, &view, 1, "src/retry.rs");
    assert_eq!(
        (sides_present(cx, &view).0, sides_present(cx, &view).1),
        (true, true)
    );
    set_eager(cx, &view, false);
    (directory, view, cx, old, new)
}

fn make_right_cold(pane: &mut DiffRenderer, completions: Arc<AtomicUsize>) {
    let key = pane
        .file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight)
        .unwrap();
    pane.prepared_syntax_documents.remove(&key);
    pane.file_diff_pair_syntax_text
        .remove(&DiffTextRegion::SplitRight);
    pane.file_diff_click_syntax_before_complete_hook = Some(Arc::new(move |_| {
        completions.fetch_add(1, Ordering::SeqCst);
    }));
}

fn fail_first_read(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    new: &std::path::Path,
    completions: Arc<AtomicUsize>,
) -> PathBuf {
    let hidden = new.with_extension("hidden");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                make_right_cold(pane, completions);
                let pos = DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::SplitRight,
                    offset: 13,
                };
                pane.diff_text_head = Some(pos);
                pane.diff_text_pending_syntax_click = Some((pos, DiffTextRegion::SplitRight));
                pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitRight, cx);
            })
        });
        std::fs::rename(new, &hidden).unwrap();
        std::fs::create_dir(new).unwrap();
    });
    cx.run_until_parked();
    hidden
}

#[gpui::test]
fn retry_timer_uses_a_document_repaired_before_it_fires(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (_directory, view, cx, _old, new) = prepare_fixture(cx);
        let completions = Arc::new(AtomicUsize::new(0));
        let hidden = fail_first_read(cx, &view, &new, Arc::clone(&completions));
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        assert!(!sides_present(cx, &view).1);

        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, _| {
                    assert_eq!(
                        gitcomet_core::domain::FileDiffTextSource::new(new.clone()).identity,
                        pane.file_diff_new_source_identity.clone().unwrap(),
                    );
                    // A small-file click can repair the document synchronously
                    // while the earlier failed read still owns a retry timer.
                    pane.file_diff_pair_syntax_document(DiffTextRegion::SplitRight)
                        .expect("the restored small source should prepare synchronously");
                    assert!(
                        pane.file_diff_side_syntax_attempts[&DiffTextRegion::SplitRight]
                            .retry_pending
                    );
                });
            });
        });
        cx.executor().advance_clock(Duration::from_millis(249));
        cx.run_until_parked();
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(pane.file_diff_side_syntax_attempts[&DiffTextRegion::SplitRight].retry_pending);
            assert!(pane.file_diff_click_syntax_inflight.is_empty());
        });

        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(
            completions.load(Ordering::SeqCst),
            1,
            "the timer must use the repaired document without starting another worker"
        );
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(
                pane.file_diff_side_syntax_attempts.is_empty(),
                "a cache hit must retire the completed retry state"
            );
            assert!(pane.file_diff_click_syntax_inflight.is_empty());
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
            );
        });
    });
}

#[gpui::test]
fn transient_read_retry_replays_the_original_bracket_click(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (directory, view, cx, old, new) = prepare_fixture(cx);
        let filler: String = (0..48_000)
            .map(|index| format!("fn filler_{index}() {{ let value = {index}; }}\n"))
            .collect();
        let old_text = format!("{}{filler}", fixture_text(0));
        let new_text = format!("{}{filler}", fixture_text(1));
        assert!(new_text.len() > 1024 * 1024);
        assert!(new_text.len() <= rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES);
        std::fs::write(&old, old_text).unwrap();
        std::fs::write(&new, new_text).unwrap();
        show(
            cx,
            &view,
            &file_diff_state(
                RepoId(7190),
                directory.path(),
                "src/retry.rs",
                2,
                &old,
                &new,
                0,
                1,
            ),
        );
        wait_settled(cx, &view, 2, "src/retry.rs");
        set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
        // The real first row is `fn fixture() {`: click its `(` at byte 10.
        // The source exceeds the foreground read ceiling, so this click must
        // wait for the worker rather than preparing synchronously.
        let click = wait_for_diff_text_click_position_for_offset_range(
            cx,
            &view,
            0,
            DiffTextRegion::SplitRight,
            10..11,
            "transient retry bracket hitbox",
        );
        let completions = Arc::new(AtomicUsize::new(0));
        let hidden = new.with_extension("hidden");
        let indexed_identity = cx.update(|_, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    make_right_cold(pane, Arc::clone(&completions));
                    pane.begin_diff_text_selection(0, DiffTextRegion::SplitRight, click, cx);
                    assert!(pane.diff_text_pending_syntax_click.is_some());
                    assert!(
                        pane.file_diff_click_syntax_inflight
                            .contains_key(&DiffTextRegion::SplitRight)
                    );
                    let identity = pane.file_diff_new_source_identity.clone().unwrap();
                    // No executor yield between the real click and the failed
                    // read: this interleaving does not depend on parse timing.
                    std::fs::rename(&new, &hidden).unwrap();
                    std::fs::create_dir(&new).unwrap();
                    identity
                })
            })
        });
        cx.run_until_parked();
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(pane.diff_text_pending_syntax_click.is_some());
            assert!(pane.diff_text_pair_match_for_tests().is_none());
        });

        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();
        assert_eq!(
            gitcomet_core::domain::FileDiffTextSource::new(new).identity,
            indexed_identity
        );
        cx.executor().advance_clock(Duration::from_millis(250));
        cx.run_until_parked();
        assert_eq!(completions.load(Ordering::SeqCst), 2);
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(pane.diff_text_pending_syntax_click.is_none());
            let pair = pane
                .diff_text_pair_match_for_tests()
                .expect("the original click must resolve after the retry without another click");
            assert_eq!(
                pair.spans
                    .iter()
                    .map(|span| span.range.clone())
                    .collect::<Vec<_>>(),
                vec![10..11, 11..12],
                "both parentheses in fixture() must be highlighted"
            );
        });
    });
}

#[gpui::test]
fn failed_reads_stop_after_three_retries_despite_repeated_frames(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (_directory, view, cx, _old, new) = prepare_fixture(cx);
        let completions = Arc::new(AtomicUsize::new(0));
        fail_first_read(cx, &view, &new, Arc::clone(&completions));
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        cx.update(|_, app| {
            assert!(
                view.read(app)
                    .diff_renderer
                    .read(app)
                    .diff_text_pending_syntax_click
                    .is_some(),
                "a transient failure must retain the pending click during its retry window"
            );
        });
        set_eager(cx, &view, true);
        // Frames do not bypass the timer and produce a retry storm.
        for _ in 0..34 {
            idle_frames(cx, &view);
        }
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        for (delay, expected) in [
            (Duration::from_millis(250), 2),
            (Duration::from_secs(1), 3),
            (Duration::from_secs(4), 4),
        ] {
            cx.executor().advance_clock(delay);
            cx.run_until_parked();
            assert_eq!(completions.load(Ordering::SeqCst), expected);
        }
        cx.executor().advance_clock(Duration::from_secs(60));
        for _ in 0..34 {
            idle_frames(cx, &view);
        }
        assert_eq!(
            completions.load(Ordering::SeqCst),
            4,
            "initial read plus exactly three retries"
        );
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            let attempt = &pane.file_diff_side_syntax_attempts[&DiffTextRegion::SplitRight];
            assert!(attempt.stopped && !attempt.retry_pending);
            assert!(
                pane.diff_text_pending_syntax_click.is_none(),
                "the exhausted retry window must release the pending click"
            );
            assert!(pane.file_diff_click_syntax_inflight.is_empty());
        });
    });
}

#[gpui::test]
fn accepting_a_new_generation_cancels_a_pending_syntax_retry(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (directory, view, cx, old, new) = prepare_fixture(cx);
        let completions = Arc::new(AtomicUsize::new(0));
        fail_first_read(cx, &view, &new, Arc::clone(&completions));
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        let generation = cx.update(|_, app| {
            view.read(app)
                .diff_renderer
                .read(app)
                .file_diff_syntax_generation
        });
        let replacement = directory.path().join("replacement.rs");
        std::fs::write(&replacement, fixture_text(2)).unwrap();
        show(
            cx,
            &view,
            &file_diff_state(
                RepoId(7190),
                directory.path(),
                "src/retry.rs",
                2,
                &old,
                &replacement,
                0,
                2,
            ),
        );
        wait_settled(cx, &view, 2, "src/retry.rs");
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert_eq!(
            completions.load(Ordering::SeqCst),
            1,
            "the old timer must not start a worker in the new generation"
        );
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_ne!(pane.file_diff_syntax_generation, generation);
            assert!(pane.file_diff_side_syntax_attempts.is_empty());
            assert!(pane.file_diff_click_syntax_inflight.is_empty());
        });
    });
}

#[gpui::test]
fn oversized_source_does_not_schedule_a_request_every_frame(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (_directory, view, cx, _old, new) = prepare_fixture(cx);
        let completions = Arc::new(AtomicUsize::new(0));
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, _| {
                    make_right_cold(pane, Arc::clone(&completions));
                })
            });
        });
        // Preserve the indexed prefix while making a whole-document read too
        // large. The first prepare rejects on metadata, without reading it.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&new)
            .unwrap()
            .set_len((rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES + 1) as u64)
            .unwrap();
        set_eager(cx, &view, true);
        for _ in 0..34 {
            idle_frames(cx, &view);
        }
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        assert!(!sides_present(cx, &view).1);
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert!(pane.file_diff_side_syntax_attempts[&DiffTextRegion::SplitRight].stopped);
            assert!(pane.file_diff_click_syntax_inflight.is_empty());
        });
    });
}

#[gpui::test]
fn a_visible_side_removed_from_the_view_map_recovers_without_a_click(
    cx: &mut gpui::TestAppContext,
) {
    assert_visible_side_recovers(cx, false);
}

#[gpui::test]
fn a_resident_side_removed_from_the_view_map_recovers_without_a_click(
    cx: &mut gpui::TestAppContext,
) {
    assert_visible_side_recovers(cx, true);
}

fn assert_visible_side_recovers(cx: &mut gpui::TestAppContext, resident: bool) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (directory, view, cx, old, new) = prepare_fixture(cx);
        if resident {
            let mut state = (*file_diff_state(
                RepoId(7190),
                directory.path(),
                "src/retry.rs",
                2,
                &old,
                &new,
                0,
                1,
            ))
            .clone();
            state.repos[0].diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::FileDiffText::new(
                    "src/retry.rs".into(),
                    Some(fixture_text(0)),
                    Some(fixture_text(1)),
                )),
            ));
            show(cx, &view, &Arc::new(state));
            wait_settled(cx, &view, 2, "src/retry.rs");
            cx.update(|_, app| {
                let pane = view.read(app).diff_renderer.read(app);
                assert!(pane.file_diff_new_source_path.is_none());
                assert!(!pane.file_diff_new_text.is_empty());
                assert!(!pane.file_diff_resident_syntax_inflight);
            });
        }
        let completions = Arc::new(AtomicUsize::new(0));
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, _| {
                    make_right_cold(pane, Arc::clone(&completions));
                })
            });
        });
        assert!(!sides_present(cx, &view).1);
        set_eager(cx, &view, true);
        idle_frames(cx, &view);
        assert!(sides_present(cx, &view).0 && sides_present(cx, &view).1);
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        cx.update(|_, app| {
            assert!(
                view.read(app)
                    .diff_renderer
                    .read(app)
                    .file_diff_side_syntax_attempts
                    .is_empty()
            );
        });
    });
}

/// Re-selecting a file reloads the same content under a new revision. Like a
/// click, that may retry a side whose automatic attempts are exhausted.
#[gpui::test]
fn reselecting_the_file_retries_an_exhausted_side(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let (directory, view, cx, old, new) = prepare_fixture(cx);
        let completions = Arc::new(AtomicUsize::new(0));
        let hidden = fail_first_read(cx, &view, &new, Arc::clone(&completions));
        for delay in [250, 1_000, 4_000] {
            cx.executor().advance_clock(Duration::from_millis(delay));
            cx.run_until_parked();
        }
        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();
        set_eager(cx, &view, true);
        idle_frames(cx, &view);
        assert_eq!(completions.load(Ordering::SeqCst), 4);
        assert!(
            !sides_present(cx, &view).1,
            "frames must not retry an exhausted side"
        );

        let rows = cx.update(|_, app| view.read(app).diff_renderer.read(app).file_diff_cache_seq);
        show(
            cx,
            &view,
            &file_diff_state(
                RepoId(7190),
                directory.path(),
                "src/retry.rs",
                2,
                &old,
                &new,
                0,
                1,
            ),
        );
        wait_settled(cx, &view, 2, "src/retry.rs");
        idle_frames(cx, &view);
        assert!(sides_present(cx, &view).1);
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.file_diff_cache_seq, rows,
                "identical content must keep its rows"
            );
            assert!(pane.file_diff_side_syntax_attempts.is_empty());
        });
    });
}

/// A background parse of in-memory text that is still running when the same
/// content arrives under a new revision serves that revision: no second parse
/// starts, and its documents are not thrown away.
#[gpui::test]
fn revision_only_refresh_joins_the_running_in_memory_parse(cx: &mut gpui::TestAppContext) {
    use crate::view::panes::diff::input::DiffPaneInput;
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().unwrap();
        let (old, new) = (
            directory.path().join("old.rs"),
            directory.path().join("new.rs"),
        );
        let path = "src/resident.rs";
        let state = |rev: u64, value: usize| {
            let mut state = (*file_diff_state(
                RepoId(7191),
                directory.path(),
                path,
                rev,
                &old,
                &new,
                0,
                value,
            ))
            .clone();
            state.repos[0].diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::FileDiffText::new(
                    path.into(),
                    Some(fixture_text(0)),
                    Some(fixture_text(value)),
                )),
            ));
            Arc::new(state)
        };
        let (view, cx) = new_view(cx);
        show(cx, &view, &state(1, 1));
        wait_settled(cx, &view, 1, path);
        let apply = |cx: &mut gpui::VisualTestContext, state: Arc<AppState>| {
            let scheduler = Arc::clone(cx.dispatcher.scheduler());
            cx.update(|_, app| {
                view.update(app, |this, cx| {
                    push_test_state(this, Arc::clone(&state), cx);
                    this.diff_renderer.update(cx, |pane, cx| {
                        let queued = scheduler.pending_task_counts();
                        pane.apply_input(DiffPaneInput::selected(state), cx);
                        pane.ensure_file_diff_cache(cx);
                        (
                            scheduler.pending_task_counts() != queued,
                            pane.file_diff_cache_rev,
                            pane.file_diff_cache_seq,
                        )
                    })
                })
            })
        };

        // New content whose parse cannot finish inside a frame.
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, _| {
                    pane.set_full_document_syntax_budget_override_for_tests(
                        rows::DiffSyntaxBudget {
                            foreground_parse: Duration::ZERO,
                        },
                    );
                })
            })
        });
        let (queued_rows, _, rows) = apply(cx, state(2, 2));
        assert!(queued_rows, "new content starts a row build");
        let parse_running = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, app| {
                view.read(app)
                    .diff_renderer
                    .read(app)
                    .file_diff_resident_syntax_inflight
            })
        };
        while !parse_running(cx) {
            assert!(cx.dispatcher.tick(false), "the row build never finished");
        }

        let (queued_work, rev, same_rows) = apply(cx, state(3, 2));
        assert_eq!(
            (rev, same_rows),
            (3, rows),
            "identical content keeps its rows"
        );
        assert!(
            !queued_work,
            "the running parse must serve the new revision"
        );

        cx.run_until_parked();
        assert!(!parse_running(cx));
        let (left, right, _) = sides_present(cx, &view);
        assert!(
            left && right,
            "the parse must install under the revision the rows carry now"
        );
    });
}

/// The same-file row rebuild replaces the rows on screen. With background
/// compute, as in the app, a side requested for the old rows meanwhile would be
/// parsed and then thrown away; each rebuild must cost one parse per side.
#[gpui::test]
fn same_file_rebuild_parses_each_side_once(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let workdir = directory.path();
    let old = workdir.join("old.rs");
    let old_staged = workdir.join("old_staged.rs");
    let new = workdir.join("new.rs");
    std::fs::write(&old, fixture_text(0)).unwrap();
    std::fs::write(&old_staged, fixture_text(50)).unwrap();
    std::fs::write(&new, fixture_text(1)).unwrap();
    let repo_id = RepoId(7192);
    let path = "src/rebuild.rs";
    let (view, cx) =
        crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
            new_view(cx)
        });
    let parses = Arc::new(AtomicUsize::new(0));
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _| {
                let parses = Arc::clone(&parses);
                pane.file_diff_click_syntax_after_prepare_hook = Some(Arc::new(move || {
                    parses.fetch_add(1, Ordering::SeqCst);
                }));
            })
        })
    });
    // One frame, then background work is delivered before the next frame, as
    // the app's main loop does it.
    let settle = |cx: &mut gpui::VisualTestContext, rev: u64| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            let busy = |pane: &DiffRenderer| {
                pane.file_diff_cache_inflight.is_some()
                    || !pane.file_diff_click_syntax_inflight.is_empty()
                    || !pane.file_diff_side_syntax_deferred.is_empty()
            };
            loop {
                cx.run_until_parked();
                if !cx.update(|_, app| busy(view.read(app).diff_renderer.read(app))) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "work never finished");
                std::thread::sleep(Duration::from_millis(1));
            }
            let ready = cx.update(|_, app| {
                let pane = view.read(app).diff_renderer.read(app);
                pane.file_diff_cache_rev == rev
                    && [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight]
                        .into_iter()
                        .all(|side| {
                            pane.file_diff_split_prepared_syntax_document(side)
                                .is_some()
                        })
            });
            if ready {
                return parses.swap(0, Ordering::SeqCst);
            }
            assert!(std::time::Instant::now() < deadline, "sides never prepared");
        }
    };

    // Background compute completes on real threads.
    cx.executor().allow_parking();
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, path, 1, &old, &new, 0, 1),
        );
        assert_eq!(settle(cx, 1), 2, "first view of the file");

        // The worktree file is edited: only the new side changes on disk.
        std::fs::write(&new, fixture_text(2)).unwrap();
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, path, 2, &old, &new, 0, 2),
        );
        assert_eq!(settle(cx, 2), 2, "edit of the new side");

        // A hunk is staged: the old side becomes another blob, and both files
        // behind the rows on screen stay as they were.
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, path, 3, &old_staged, &new, 50, 2),
        );
        assert_eq!(settle(cx, 3), 2, "replacement of the old side");
    });
}
