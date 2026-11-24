use super::*;
use crate::view::conflict_markers::{MarkerCheck, StageIntent};
use gitcomet_state::msg::StageSelection;

const MARKERS: &str = "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n";

fn conflicted_repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    restore_repository(root, clean_repository_template());
    git(root, &["checkout", "-qb", "side"]);
    for (branch, contents) in [("side", "theirs\n"), ("main", "ours\n")] {
        git(root, &["checkout", "-q", branch]);
        for name in ["a.txt", "b.txt"] {
            std::fs::write(root.join(name), contents).unwrap();
        }
        commit(root, branch);
    }
    let merge = gitcomet_core::test_support::git_fixture::git_command()
        .current_dir(root)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "merge",
            "side",
        ])
        .output()
        .unwrap();
    assert_eq!(merge.status.code(), Some(1));
    assert_eq!(
        git_output(root, &["diff", "--name-only", "--diff-filter=U"]),
        b"a.txt\nb.txt\n"
    );
    dir
}

fn open_conflict_window<'a>(
    cx: &'a mut gpui::TestAppContext,
    root: &Path,
) -> (View, AppStore, &'a mut gpui::VisualTestContext) {
    let backend = gitcomet_git_gix::GixBackend.open(root).unwrap();
    let status = backend
        .status(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let mut repo = opening_repo_state(REPO, root);
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.worktree_status = Loadable::Ready(status.unstaged.clone());
    repo.staged_status = Loadable::Ready(status.staged.clone());
    repo.status = Loadable::Ready(Arc::new(status));
    repo.diff_state.diff_target =
        Some(DiffTarget::working_tree("a.txt".into(), DiffArea::Unstaged));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    store.disable_repo_monitors_for_test();
    let view_store = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(view_store, events, None, window, cx));
    crate::view::test_support::drain_store_worker(&view, cx);
    super::super::shortcuts::apply_state(cx, &view, app_state_with_repo(repo, REPO));
    store.insert_repo_for_test(REPO, backend);
    (view, store, cx)
}

#[gpui::test]
fn marker_edit_after_a_clean_scan_preserves_the_entire_index(cx: &mut gpui::TestAppContext) {
    let dir = conflicted_repository();
    let root = dir.path();
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.join(name), "manually resolved\n").unwrap();
    }
    let initial = git_output(root, &["ls-files", "--unmerged"]);
    let (view, store, cx) = open_conflict_window(cx, root);
    let first = root.join("a.txt");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::conflict_markers::check(
                this,
                REPO,
                vec!["a.txt".into(), "b.txt".into()],
                StageIntent::Command {
                    offer_confirmation: true,
                },
                window,
                cx,
                move |this, paths, checked, _, _| {
                    let MarkerCheck::Clear(guard) = checked else {
                        panic!("both worktree files were resolved when scanned")
                    };
                    std::fs::write(&first, MARKERS).unwrap();
                    crate::view::status_actions::stage_checked(
                        &this.store,
                        REPO,
                        StageSelection::Paths(paths.into()),
                        guard,
                    );
                },
            );
        })
    });
    cx.run_until_parked();
    let after = wait_for(cx, &view, &store, "guarded staging completes", |repo| {
        repo.ops_rev >= 2 && repo.local_actions_in_flight == 0
    });
    assert_eq!(git_output(root, &["ls-files", "--unmerged"]), initial);
    assert!(
        after.repos[0]
            .feedback
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("a.txt") && message.contains("Stage again"))
    );
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    std::fs::write(root.join(".git/info/attributes"), "a.txt text eol=lf\n").unwrap();
    std::fs::write(root.join("a.txt"), "second resolution\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        git(root, &["config", "core.filemode", "true"]);
        std::fs::set_permissions(root.join("a.txt"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let revision = after.repos[0].ops_rev;
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("stage-all", Some(window), cx)
        })
    });
    cx.run_until_parked();
    wait_for(cx, &view, &store, "clean resolutions stage", |repo| {
        repo.ops_rev >= revision + 2 && repo.local_actions_in_flight == 0
    });
    assert!(git_output(root, &["ls-files", "--unmerged"]).is_empty());
    assert_eq!(
        git_output(root, &["show", ":a.txt"]),
        b"second resolution\n"
    );
    #[cfg(unix)]
    assert!(git_output(root, &["ls-files", "--stage", "a.txt"]).starts_with(b"100755 "));
    assert_eq!(
        git_output(root, &["show", ":b.txt"]),
        b"manually resolved\n"
    );
}

#[gpui::test]
fn a_status_menu_stage_survives_navigation_away_from_a_pending_diff_shortcut(
    cx: &mut gpui::TestAppContext,
) {
    let dir = conflicted_repository();
    let (view, store, cx) = open_conflict_window(cx, dir.path());
    let renderer = super::super::diff_renderer(cx, &view);
    cx.update(|window, app| {
        assert!(renderer.update(app, |pane, cx| pane.toggle_stage_shown_file(window, cx)));
        let mut next = (*store.snapshot()).clone();
        next.repos[0].diff_state.diff_target =
            Some(DiffTarget::working_tree("b.txt".into(), DiffArea::Unstaged));
        next.repos[0].diff_state.diff_target_rev += 1;
        let next = Arc::new(next);
        store.replace_snapshot_for_test(next.clone());
        view.update(app, |this, cx| {
            crate::view::test_support::push_test_state(this, next, cx)
        });
        view.read(app).popover_host.clone().update(app, |host, cx| {
            host.context_menu_activate_action(
                ContextMenuAction::StageSelectionOrPath {
                    repo_id: REPO,
                    area: DiffArea::Unstaged,
                    path: "a.txt".into(),
                },
                window,
                cx,
            )
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(matches!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::StageConflictMarkersConfirm { unresolved, .. })
                if unresolved == [PathBuf::from("a.txt")]
        ));
    });
    assert_eq!(
        git_output(dir.path(), &["diff", "--name-only", "--diff-filter=U"]),
        b"a.txt\nb.txt\n"
    );
    assert_eq!(
        store.snapshot().repos[0].diff_state.diff_target,
        Some(DiffTarget::working_tree("b.txt".into(), DiffArea::Unstaged))
    );
}

#[gpui::test]
fn staging_while_status_loads_checks_the_actual_index_and_honors_stage_anyway(
    cx: &mut gpui::TestAppContext,
) {
    let dir = conflicted_repository();
    let (view, store, cx) = open_conflict_window(cx, dir.path());
    let mut state = (*store.snapshot()).clone();
    state.repos[0].worktree_status = Loadable::Loading;
    state.repos[0].status = Loadable::Loading;
    super::super::shortcuts::apply_state(cx, &view, Arc::new(state));
    let renderer = super::super::diff_renderer(cx, &view);
    cx.update(|window, app| {
        assert!(renderer.update(app, |pane, cx| pane.toggle_stage_shown_file(window, cx)))
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
    cx.update(|_, app| {
        assert!(matches!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::StageConflictMarkersConfirm { unresolved, .. })
                if unresolved == [PathBuf::from("a.txt")]
        ));
    });
    let initial = std::fs::read(dir.path().join("a.txt")).unwrap();
    click(cx, "stage_conflict_markers_go");
    wait_for(cx, &view, &store, "confirmed markers stage", |repo| {
        repo.ops_rev >= 2 && repo.local_actions_in_flight == 0
    });
    assert_eq!(git_output(dir.path(), &["show", ":a.txt"]), initial);
    assert_eq!(
        git_output(dir.path(), &["diff", "--name-only", "--diff-filter=U"]),
        b"b.txt\n"
    );
}
