use super::*;
use gitcomet_core::domain::{Diff, DiffArea, FileDiffText, FileDiffTextSource, FileStatusKind};
use gitcomet_state::model::{Loadable, RepoId};

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
            // The cold window can enqueue superseded work before its first
            // result establishes the cached path. Measure re-selections from
            // that settled baseline, and idle checks from each completion.
            failed_rows_stay_idle(cx, &view, revision, sequence);
            previous_sequence = Some(sequence);
        }
        let previous_sequence = previous_sequence.unwrap();

        std::fs::remove_dir(&new).unwrap();
        std::fs::rename(&hidden, &new).unwrap();
        // Restoring the bytes/identity alone must not cause a retry every frame.
        // A later explicit re-selection is what grants the next row rebuild.
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
