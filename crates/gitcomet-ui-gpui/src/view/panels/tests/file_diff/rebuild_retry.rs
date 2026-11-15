use super::super::syntax_cache_findings::{
    file_diff_state, fixture_text, idle_frames, new_view, show, wait_settled,
};
use super::*;
use gitcomet_core::domain::{Diff, DiffArea, FileDiffText, FileDiffTextSource, FileStatusKind};
use gitcomet_state::model::{Loadable, RepoId};
use std::time::Duration;

type View = gpui::Entity<super::super::super::GitCometView>;

fn failed_rows_stay_idle(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    revision: u64,
    expected_sequence: u64,
) {
    for _ in 0..100 {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer
                    .update(cx, |pane, cx| pane.ensure_file_diff_cache(cx));
            });
        });
        cx.run_until_parked();
    }
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.file_diff_cache_rev, revision);
        assert_eq!(
            pane.file_diff_cache_seq, expected_sequence,
            "an unchanged failed revision must not start another rebuild"
        );
        assert!(pane.file_diff_cache_inflight.is_none());
        assert!(pane.file_diff_cache_error.is_some());
        assert!(pane.file_diff_row_provider.is_none());
    });
}

#[gpui::test]
fn failed_file_diff_rebuild_retries_once_per_reselection(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("failed rebuild fixture");
        let workdir = directory.path();
        let old = workdir.join("old.rs");
        let new = workdir.join("new.rs");
        let hidden = workdir.join("new.rs.hidden");
        std::fs::write(&old, "fn value() -> i32 { 1 }\n").unwrap();
        std::fs::write(&new, "fn value() -> i32 { 2 }\n").unwrap();
        let path = PathBuf::from("src/fixture.rs");
        let file = Arc::new(FileDiffText::new_sources(
            path.clone(),
            Some(FileDiffTextSource::new(old)),
            Some(FileDiffTextSource::new(new.clone())),
        ));

        // Capture the normal payload before its source temporarily becomes
        // unreadable. Keep that exact payload across each re-selection.
        std::fs::rename(&new, &hidden).unwrap();
        std::fs::create_dir(&new).unwrap();

        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::super::super::GitCometView::new(store, events, None, window, cx)
        });
        let repo_id = RepoId(75506);
        let show_revision = |cx: &mut gpui::VisualTestContext, revision| {
            let mut repo = opening_repo_state(repo_id, workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                FileStatusKind::Modified,
                DiffArea::Unstaged,
            );
            let target = repo.diff_state.diff_target.clone().unwrap();
            repo.diff_state.diff_rev = revision;
            repo.diff_state.diff_state_rev = revision;
            repo.diff_state.diff = Loadable::Ready(Arc::new(Diff::from_unified(
                target,
                "@@ -1 +1 @@\n-fn value() -> i32 { 1 }\n+fn value() -> i32 { 2 }\n",
            )));
            repo.diff_state.diff_file_rev = revision;
            repo.diff_state.diff_file = Loadable::Ready(Some(Arc::clone(&file)));
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    push_test_state(this, app_state_with_repo(repo, repo_id), cx);
                });
            });
        };

        let mut previous_sequence: Option<u64> = None;
        for revision in 1..=3 {
            show_revision(cx, revision);
            wait_for_main_pane_condition(
                cx,
                &view,
                "failed source-backed row rebuild",
                |pane| {
                    pane.file_diff_cache_rev == revision
                        && pane.file_diff_cache_error.is_some()
                        && pane.file_diff_cache_inflight.is_none()
                },
                |pane| {
                    (
                        pane.file_diff_cache_rev,
                        pane.file_diff_cache_seq,
                        pane.file_diff_cache_error.is_some(),
                    )
                },
            );
            let sequence = cx
                .update(|_window, app| view.read(app).diff_renderer.read(app).file_diff_cache_seq);
            if let Some(previous) = previous_sequence {
                assert_eq!(
                    sequence,
                    previous.wrapping_add(1),
                    "re-selection must start exactly one row rebuild"
                );
            }
            // Measure re-selections from the first settled build, and idle
            // checks from each completion.
            failed_rows_stay_idle(cx, &view, revision, sequence);
            previous_sequence = Some(sequence);
        }
        let previous_sequence = previous_sequence.unwrap();

        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();
        // Restoring the bytes/identity alone must not cause a retry every frame.
        // With the clock standing still no retry timer fires either, so the
        // explicit re-selection below is what grants the next row rebuild.
        failed_rows_stay_idle(cx, &view, 3, previous_sequence);
        show_revision(cx, 4);
        wait_for_main_pane_condition(
            cx,
            &view,
            "successful row rebuild after re-selection",
            |pane| {
                pane.file_diff_cache_rev == 4
                    && pane.file_diff_cache_inflight.is_none()
                    && pane.file_diff_cache_error.is_none()
                    && pane.file_diff_row_provider.is_some()
            },
            |pane| {
                (
                    pane.file_diff_cache_rev,
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_error.is_some(),
                )
            },
        );
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(pane.file_diff_cache_seq, previous_sequence.wrapping_add(1));
        });
    });
}

/// (rebuild sequence, a rebuild is pending, the error panel is up, rows exist)
fn row_build(cx: &mut gpui::VisualTestContext, view: &View) -> (u64, bool, bool, bool) {
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        (
            pane.file_diff_cache_seq,
            pane.file_diff_cache_inflight.is_some(),
            pane.file_diff_cache_error.is_some(),
            pane.file_diff_row_provider.is_some(),
        )
    })
}

/// Frames with no executor turn between them, so a queued build cannot finish.
fn frames_without_executor_turns(cx: &mut gpui::VisualTestContext, view: &View, frames: usize) {
    for _ in 0..frames {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |_, cx| cx.notify());
            });
            let _ = window.draw(app);
        });
    }
}

fn wait_for_failed_rows(cx: &mut gpui::VisualTestContext, view: &View, revision: u64) -> u64 {
    wait_for_main_pane_condition(
        cx,
        view,
        "failed row build",
        |pane| {
            pane.file_diff_cache_rev == revision
                && pane.file_diff_cache_error.is_some()
                && pane.file_diff_cache_inflight.is_none()
        },
        |pane| (pane.file_diff_cache_rev, pane.file_diff_cache_seq),
    );
    row_build(cx, view).0
}

/// A large file builds over many frames. Each frame re-checks the cache, and
/// an identical request must join the running build instead of replacing it.
#[gpui::test]
fn frames_do_not_restart_a_pending_row_build(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("pending build fixture");
        let workdir = directory.path();
        let (old, new) = (workdir.join("old.rs"), workdir.join("new.rs"));
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(75601);
        let path = "src/pending.rs";

        // First view of the file: there are no rows yet.
        let before = row_build(cx, &view).0;
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, path, 1, &old, &new, 0, 1),
        );
        frames_without_executor_turns(cx, &view, 6);
        assert_eq!(
            row_build(cx, &view),
            (before + 1, true, false, false),
            "the first view must start exactly one row build"
        );
        wait_settled(cx, &view, 1, path);
        assert_eq!(row_build(cx, &view), (before + 1, false, false, true));

        // New content for the same file: the old rows stay up meanwhile.
        std::fs::write(&new, fixture_text(2)).unwrap();
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, path, 2, &old, &new, 0, 2),
        );
        frames_without_executor_turns(cx, &view, 6);
        assert_eq!(
            row_build(cx, &view),
            (before + 2, true, false, true),
            "a same-file rebuild must start exactly once"
        );
        wait_settled(cx, &view, 2, path);
        assert_eq!(row_build(cx, &view), (before + 2, false, false, true));
    });
}

/// A source that is locked or briefly missing while its rows are indexed
/// usually comes back unchanged, so no new revision follows. The build is
/// retried on a timer; frames alone never retry it.
#[gpui::test]
fn transient_row_build_failure_recovers_without_reselection(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("transient build fixture");
        let workdir = directory.path();
        let (old, new) = (workdir.join("old.rs"), workdir.join("new.rs"));
        let hidden = workdir.join("new.rs.hidden");
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        let state = file_diff_state(RepoId(75602), workdir, "src/locked.rs", 1, &old, &new, 0, 1);
        std::fs::rename(&new, &hidden).unwrap();
        std::fs::create_dir(&new).unwrap();

        let (view, cx) = new_view(cx);
        show(cx, &view, &state);
        let failed = wait_for_failed_rows(cx, &view, 1);
        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();

        cx.executor().advance_clock(Duration::from_millis(249));
        for _ in 0..10 {
            idle_frames(cx, &view);
        }
        assert_eq!(row_build(cx, &view), (failed, false, true, false));

        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(
            row_build(cx, &view),
            (failed + 1, false, false, true),
            "the retry must build the rows once the source is readable"
        );
    });
}

#[gpui::test]
fn failed_row_build_stops_after_three_retries(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("failing build fixture");
        let workdir = directory.path();
        let (old, new) = (workdir.join("old.rs"), workdir.join("new.rs"));
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::create_dir(&new).unwrap();
        let (view, cx) = new_view(cx);
        show(
            cx,
            &view,
            &file_diff_state(
                RepoId(75603),
                workdir,
                "src/missing.rs",
                1,
                &old,
                &new,
                0,
                1,
            ),
        );
        let failed = wait_for_failed_rows(cx, &view, 1);

        for (delay, retries) in [
            (Duration::from_millis(250), 1),
            (Duration::from_secs(1), 2),
            (Duration::from_secs(4), 3),
        ] {
            cx.executor().advance_clock(delay);
            cx.run_until_parked();
            for _ in 0..10 {
                idle_frames(cx, &view);
            }
            assert_eq!(row_build(cx, &view), (failed + retries, false, true, false));
        }
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert_eq!(
            row_build(cx, &view),
            (failed + 3, false, true, false),
            "one build plus exactly three retries"
        );
    });
}

/// A side that is not text never becomes text, and a retry that outlives its
/// selection has nothing left to rebuild.
#[gpui::test]
fn row_build_retries_only_for_readable_text_of_the_same_selection(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("retry scope fixture");
        let workdir = directory.path();
        let (old, new) = (workdir.join("old.rs"), workdir.join("new.rs"));
        let binary = workdir.join("binary.rs");
        let unreadable = workdir.join("unreadable.rs");
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        std::fs::create_dir(&unreadable).unwrap();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(75604);

        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, "src/binary.rs", 1, &old, &binary, 0, 1),
        );
        let not_text = wait_for_failed_rows(cx, &view, 1);
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert_eq!(row_build(cx, &view), (not_text, false, true, false));

        show(
            cx,
            &view,
            &file_diff_state(
                repo_id,
                workdir,
                "src/unreadable.rs",
                2,
                &old,
                &unreadable,
                0,
                1,
            ),
        );
        wait_for_failed_rows(cx, &view, 2);
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, "src/other.rs", 3, &old, &new, 0, 1),
        );
        wait_settled(cx, &view, 3, "src/other.rs");
        let other = row_build(cx, &view);
        assert_eq!((other.1, other.2, other.3), (false, false, true));
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert_eq!(
            row_build(cx, &view),
            other,
            "the unreadable file's retry must not touch the next selection"
        );
    });
}

/// Joining a running build is only sound while that build can still land. One
/// started for an earlier revision rejects its own result, so the same content
/// under the new revision needs a build of its own.
#[gpui::test]
fn reload_under_a_new_revision_does_not_join_a_superseded_build(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        let directory = tempfile::tempdir().expect("superseded build fixture");
        let workdir = directory.path();
        let (old, new) = (workdir.join("old.rs"), workdir.join("new.rs"));
        std::fs::write(&old, fixture_text(0)).unwrap();
        std::fs::write(&new, fixture_text(1)).unwrap();
        let (view, cx) = new_view(cx);
        let repo_id = RepoId(75605);
        let path = "src/superseded.rs";
        show(
            cx,
            &view,
            &file_diff_state(repo_id, workdir, path, 1, &old, &new, 0, 1),
        );
        wait_settled(cx, &view, 1, path);
        let settled = row_build(cx, &view).0;

        std::fs::write(&new, fixture_text(2)).unwrap();
        let edited = file_diff_state(repo_id, workdir, path, 2, &old, &new, 0, 2);
        show(cx, &view, &edited);
        frames_without_executor_turns(cx, &view, 2);
        assert_eq!(row_build(cx, &view), (settled + 1, true, false, true));

        // The store reloads: first without a file, then the same file again.
        let mut reloading = (*edited).clone();
        reloading.repos[0].diff_state.diff_file_rev = 3;
        reloading.repos[0].diff_state.diff_state_rev = 3;
        let reloaded = Arc::new(reloading.clone());
        reloading.repos[0].diff_state.diff_file = Loadable::Loading;
        show(cx, &view, &Arc::new(reloading));
        frames_without_executor_turns(cx, &view, 2);
        show(cx, &view, &reloaded);
        frames_without_executor_turns(cx, &view, 2);
        assert_eq!(
            row_build(cx, &view),
            (settled + 2, true, false, true),
            "the reload must start a build that can land under revision 3"
        );

        wait_settled(cx, &view, 3, path);
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(pane.file_diff_cache_seq, settled + 2);
            assert_eq!(
                pane.file_diff_new_source_identity.as_deref(),
                Some(FileDiffTextSource::new(new.clone()).identity.as_ref()),
                "the rows must show the edited file"
            );
        });
    });
}
