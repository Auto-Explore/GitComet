//! Scrollbar change markers follow a same-target diff reload, including one
//! that changes a line in place and so keeps the row count.

use super::*;
use gitcomet_core::services::GitBackend;

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn lines(edits: &[(usize, &str)]) -> String {
    (0..40)
        .map(|ix| match edits.iter().find(|(at, _)| *at == ix) {
            Some((_, text)) => format!("{text}\n"),
            None if ix == 5 => "\n".to_string(),
            None => format!("line {ix}\n"),
        })
        .collect()
}

fn wait_store(store: &AppStore, what: &str, ready: impl Fn(&AppState) -> bool) -> Arc<AppState> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let snapshot = store.snapshot();
        if ready(&snapshot) {
            return snapshot;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn show(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    snapshot: Arc<AppState>,
) {
    let (diff_rev, diff_file_rev) = {
        let diff = &snapshot.repos[0].diff_state;
        (diff.diff_rev, diff.diff_file_rev)
    };
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.ui_model
                .update(cx, |model, cx| model.set_state(snapshot, cx))
        });
    });
    // Until the pane has built rows for exactly this snapshot: the patch cache,
    // and the file-diff rows, which are swapped in by a background rebuild.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        draw_and_drain_test_window(cx);
        let caught_up = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            if pane
                .active_repo()
                .is_some_and(|repo| repo.diff_state.edit_mode)
            {
                return true;
            }
            pane.diff_cache_rev == diff_rev
                && pane.file_diff_cache_rev == diff_file_rev
                && pane.file_diff_cache_inflight.is_none()
        });
        if caught_up {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the pane never caught up with the diff"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }
}

/// Where the markers say changes are, as marker start fractions.
fn marker_starts(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> (Vec<f32>, Vec<f32>) {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let starts = |markers: Vec<components::ScrollbarMarker>| {
            markers
                .into_iter()
                .map(|marker| marker.start)
                .collect::<Vec<_>>()
        };
        (
            starts(pane.diff_scrollbar_markers_cache.clone()),
            starts(pane.compute_diff_scrollbar_markers()),
        )
    })
}

fn markers_follow_an_in_place_external_edit(
    cx: &mut gpui::TestAppContext,
    view_mode: DiffViewMode,
    content_mode: DiffContentMode,
) {
    let _guard = lock_visual_test();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "T"]);
    std::fs::write(root.join("a.rs"), lines(&[])).expect("write");
    git(root, &["add", "a.rs"]);
    git(root, &["commit", "-q", "-m", "init"]);
    // An unstaged change near the end of the file.
    std::fs::write(root.join("a.rs"), lines(&[(35, "changed near the end")])).expect("write");

    let repo = gitcomet_git_gix::GixBackend.open(root).expect("open repo");
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store.clone(), events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(1);
    let mut repo_state = opening_repo_state(repo_id, root);
    repo_state.spec = repo.spec().clone();
    repo_state.open = gitcomet_state::model::Loadable::Ready(());
    store.replace_snapshot_for_test(app_state_with_repo(repo_state, repo_id));
    store.insert_repo_for_test(repo_id, repo);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, _| pane.diff_view = view_mode)
        });
    });

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: "a.rs".into(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    store.dispatch(Msg::SelectDiff {
        repo_id,
        target: target.clone(),
    });
    let snapshot = wait_store(&store, "the diff", |state| {
        let diff = &state.repos[0].diff_state;
        diff.diff_target.as_ref() == Some(&target)
            && matches!(diff.diff, gitcomet_state::model::Loadable::Ready(_))
            && matches!(diff.diff_file, gitcomet_state::model::Loadable::Ready(_))
    });
    show(cx, &view, snapshot);
    set_diff_content_mode_for_test(cx, &view, content_mode);
    let (cached, fresh) = marker_starts(cx, &view);
    assert_eq!(cached, fresh);
    assert!(!cached.is_empty(), "the change near the end is marked");
    let markers_before = cached.len();

    // Another program fills the blank line 6: same line count, so the split
    // view gains a modified row but no new row.
    std::fs::write(
        root.join("a.rs"),
        lines(&[(5, "asdads;"), (35, "changed near the end")]),
    )
    .expect("external write");
    let (rev_before, file_rev_before) = {
        let diff = &store.snapshot().repos[0].diff_state;
        (diff.diff_rev, diff.diff_file_rev)
    };
    store.dispatch(Msg::RepoExternallyChanged {
        repo_id,
        change: gitcomet_state::msg::RepoExternalChange::worktree(),
    });
    // Both halves: the patch (which the change kinds come from) and the text.
    let snapshot = wait_store(&store, "the reload", |state| {
        let diff = &state.repos[0].diff_state;
        diff.diff_rev != rev_before
            && diff.diff_file_rev != file_rev_before
            && !diff.diff_reload_in_flight
    });
    show(cx, &view, snapshot);

    let (cached, fresh) = marker_starts(cx, &view);
    assert!(
        fresh.len() > markers_before,
        "the diff on screen has a second change: {fresh:?}"
    );
    assert_eq!(
        cached, fresh,
        "{view_mode:?}/{content_mode:?}: the scrollbar must mark the new change at the top, not only the old one"
    );
}

#[gpui::test]
fn split_full_markers_follow_an_in_place_external_edit(cx: &mut gpui::TestAppContext) {
    markers_follow_an_in_place_external_edit(cx, DiffViewMode::Split, DiffContentMode::Full);
}

#[gpui::test]
fn inline_full_markers_follow_an_in_place_external_edit(cx: &mut gpui::TestAppContext) {
    markers_follow_an_in_place_external_edit(cx, DiffViewMode::Inline, DiffContentMode::Full);
}

#[gpui::test]
fn split_collapsed_markers_follow_an_in_place_external_edit(cx: &mut gpui::TestAppContext) {
    markers_follow_an_in_place_external_edit(cx, DiffViewMode::Split, DiffContentMode::Collapsed);
}

#[gpui::test]
fn inline_collapsed_markers_follow_an_in_place_external_edit(cx: &mut gpui::TestAppContext) {
    markers_follow_an_in_place_external_edit(cx, DiffViewMode::Inline, DiffContentMode::Collapsed);
}

/// The reported path: edit the file from its diff, another program changes
/// it, Reload, close the diff, open the file's diff again.
#[gpui::test]
fn markers_are_current_after_editing_reloading_and_reopening_the_diff(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "T"]);
    std::fs::write(root.join("a.rs"), lines(&[])).expect("write");
    git(root, &["add", "a.rs"]);
    git(root, &["commit", "-q", "-m", "init"]);
    std::fs::write(root.join("a.rs"), lines(&[(35, "changed near the end")])).expect("write");

    let repo = gitcomet_git_gix::GixBackend.open(root).expect("open repo");
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store.clone(), events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(1);
    let mut repo_state = opening_repo_state(repo_id, root);
    repo_state.spec = repo.spec().clone();
    repo_state.open = gitcomet_state::model::Loadable::Ready(());
    store.replace_snapshot_for_test(app_state_with_repo(repo_state, repo_id));
    store.insert_repo_for_test(repo_id, repo);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, _| pane.diff_view = DiffViewMode::Split)
        });
    });

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: "a.rs".into(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let open_diff = |cx: &mut gpui::VisualTestContext| {
        store.dispatch(Msg::SelectDiff {
            repo_id,
            target: target.clone(),
        });
        let snapshot = wait_store(&store, "the diff", |state| {
            let diff = &state.repos[0].diff_state;
            diff.diff_target.as_ref() == Some(&target)
                && !diff.edit_mode
                && !diff.diff_reload_in_flight
                && matches!(diff.diff, gitcomet_state::model::Loadable::Ready(_))
                && matches!(diff.diff_file, gitcomet_state::model::Loadable::Ready(_))
        });
        show(cx, &view, snapshot);
    };
    open_diff(cx);
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
    let markers_before = marker_starts(cx, &view).0.len();

    // Edit from the diff.
    store.dispatch(Msg::OpenFileEditor {
        repo_id,
        path: "a.rs".into(),
    });
    let snapshot = wait_store(&store, "the editor", |state| {
        state.repos[0].diff_state.edit_mode
    });
    show(cx, &view, snapshot);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx))
        });
    });
    for _ in 0..5 {
        draw_and_drain_test_window(cx);
    }

    // Another program fills the blank line 6; Reload.
    std::fs::write(
        root.join("a.rs"),
        lines(&[(5, "asdads;"), (35, "changed near the end")]),
    )
    .expect("external write");
    store.dispatch(Msg::RepoExternallyChanged {
        repo_id,
        change: gitcomet_state::msg::RepoExternalChange::worktree(),
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    let snapshot = wait_store(&store, "the refresh", |state| {
        !state.repos[0].diff_state.diff_reload_in_flight
    });
    show(cx, &view, snapshot);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, cx| pane.reload_file_from_disk_notice(cx))
        });
    });
    for _ in 0..5 {
        draw_and_drain_test_window(cx);
    }

    // Close the diff, then open the file's diff again.
    store.dispatch(Msg::ExitDiffEditMode { repo_id });
    store.dispatch(Msg::ClearDiffSelection { repo_id });
    let snapshot = wait_store(&store, "the diff to close", |state| {
        state.repos[0].diff_state.diff_target.is_none()
    });
    show(cx, &view, snapshot);
    open_diff(cx);

    let (cached, fresh) = marker_starts(cx, &view);
    assert!(
        fresh.len() > markers_before,
        "two changes on screen: {fresh:?}"
    );
    assert_eq!(
        cached, fresh,
        "the reopened diff must mark the new change too"
    );
}
