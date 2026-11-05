//! The linked commit panel in a worktree's commit form: one section per
//! changed first-level submodule above the parent's own.

use super::*;
use gitcomet_core::domain::{
    CommitId, FileStatus, FileStatusKind, HeadTracking, Submodule, SubmoduleChanges,
    SubmoduleStatus, Upstream,
};

type LinkedView = gpui::Entity<super::super::super::GitCometView>;

fn file(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: path.into(),
        kind,
        conflict: None,
    }
}

fn changed_row(path: &str) -> Submodule {
    let mut row = Submodule::new(
        path.into(),
        CommitId("a1b2c3d4e5f6a7b8".into()),
        Some(CommitId("a1b2c3d4e5f6a7b8".into())),
        SubmoduleStatus::UpToDate,
    );
    row.has_inner_changes = true;
    row
}

/// A worktree with two changed submodules: the first has a staged and an
/// unstaged file, the second only an unstaged one.
fn parent_with_changed_submodules(id: u64) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(
        gitcomet_state::model::RepoId(id),
        Path::new("/tmp/linked-commit/GitComet"),
    );
    repo.status = gitcomet_state::model::Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: Arc::new(vec![file("crates/lang.rs", FileStatusKind::Modified)]),
            unstaged: Arc::new(Vec::new()),
        }
        .into(),
    );
    repo.submodules = gitcomet_state::model::Loadable::Ready(Arc::new(vec![
        changed_row("fixtures/syntax_test"),
        changed_row("vendor/ui-kit"),
    ]));
    let tracking = HeadTracking::Tracking {
        branch: "main".to_string(),
        upstream: Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        },
        divergence: None,
    };
    repo.submodule_changes = gitcomet_state::model::Loadable::Ready(Arc::new(vec![
        SubmoduleChanges {
            path: "fixtures/syntax_test".into(),
            head: tracking,
            staged: Arc::new(vec![file("razor/layout.cshtml", FileStatusKind::Added)]),
            unstaged: Arc::new(vec![file("manifest.toml", FileStatusKind::Modified)]),
        },
        SubmoduleChanges {
            path: "vendor/ui-kit".into(),
            head: HeadTracking::Detached,
            staged: Arc::new(Vec::new()),
            unstaged: Arc::new(vec![file("theme.rs", FileStatusKind::Modified)]),
        },
    ]));
    repo
}

fn new_linked_view(cx: &mut gpui::TestAppContext) -> (LinkedView, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    })
}

fn draw_with(cx: &mut gpui::VisualTestContext, view: &LinkedView, state: Arc<AppState>) {
    cx.update(|window, app| {
        view.update(app, |this, cx| push_test_state(this, state, cx));
        let _ = window.draw(app);
    });
}

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

fn set_message(cx: &mut gpui::VisualTestContext, view: &LinkedView, text: &str) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input
                    .update(cx, |input, cx| input.set_text(text.to_string(), cx));
            });
        });
    });
}

fn set_submodule_message(
    cx: &mut gpui::VisualTestContext,
    view: &LinkedView,
    repo_id: gitcomet_state::model::RepoId,
    path: &str,
    text: &str,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let input = pane
                    .linked_message_inputs
                    .get(&(repo_id, std::path::PathBuf::from(path)))
                    .expect("the section's message box")
                    .clone();
                input.update(cx, |input, cx| input.set_text(text.to_string(), cx));
            });
        });
    });
}

fn can_submit(cx: &mut gpui::VisualTestContext, view: &LinkedView) -> bool {
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let message = pane.commit_message_input.read(app).text().to_string();
        match pane.active_linked_panel() {
            Some(panel) => {
                let children = pane.linked_children(&panel, app);
                pane.can_submit_linked_commit(&message, children.as_deref())
            }
            None => pane.can_submit_commit(&message),
        }
    })
}

#[gpui::test]
fn changed_submodules_get_a_section_each_above_the_parent(cx: &mut gpui::TestAppContext) {
    let (view, cx) = new_linked_view(cx);
    let repo_id = gitcomet_state::model::RepoId(71);
    draw_with(
        cx,
        &view,
        app_state_with_repo(parent_with_changed_submodules(71), repo_id),
    );

    for selector in [
        "linked_commit_toggle",
        "linked_commit_section_0",
        "linked_commit_section_1",
        "linked_commit_parent_section",
        "linked_commit_file_0_0",
        "linked_commit_file_0_1",
        "linked_commit_file_1_0",
        "linked_commit_message_0",
        "linked_commit_message_1",
        "linked_commit_gitlink_0",
        "linked_commit_parent_staged",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} is drawn");
    }
    // Nothing in the second submodule is staged, so the parent records only
    // the first one.
    assert!(cx.debug_bounds("linked_commit_gitlink_1").is_none());
    // Its own section replaces the submodule tab's record-in-parent box.
    assert!(cx.debug_bounds("record_in_parent_section").is_none());
}

#[gpui::test]
fn a_linked_commit_needs_a_message_for_each_staged_submodule(cx: &mut gpui::TestAppContext) {
    let (view, cx) = new_linked_view(cx);
    let repo_id = gitcomet_state::model::RepoId(72);
    draw_with(
        cx,
        &view,
        app_state_with_repo(parent_with_changed_submodules(72), repo_id),
    );

    set_message(cx, &view, "test: use new razor fixtures");
    redraw(cx);
    assert!(
        !can_submit(cx, &view),
        "the staged submodule has no message yet"
    );

    set_submodule_message(
        cx,
        &view,
        repo_id,
        "fixtures/syntax_test",
        "test: add razor view fixtures",
    );
    redraw(cx);
    assert!(can_submit(cx, &view));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                assert!(pane.submit_commit_past_submodule_guard(None, cx));
            });
        });
    });
    let used = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .pending_linked_messages
            .get(&repo_id)
            .cloned()
    });
    assert_eq!(
        used,
        Some(vec![gitcomet_state::model::LinkedChild {
            path: "fixtures/syntax_test".into(),
            message: "test: add razor view fixtures".to_string(),
        }]),
        "only the submodule with staged files commits"
    );
}

#[gpui::test]
fn committing_the_parent_only_hides_the_submodule_sections(cx: &mut gpui::TestAppContext) {
    let (view, cx) = new_linked_view(cx);
    let repo_id = gitcomet_state::model::RepoId(73);
    draw_with(
        cx,
        &view,
        app_state_with_repo(parent_with_changed_submodules(73), repo_id),
    );
    assert!(cx.debug_bounds("linked_commit_section_0").is_some());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.linked_commit_off.insert(repo_id);
                cx.notify();
            });
        });
    });
    redraw(cx);

    assert!(cx.debug_bounds("linked_commit_toggle").is_some());
    assert!(cx.debug_bounds("linked_commit_section_0").is_none());
    assert!(cx.debug_bounds("linked_commit_parent_section").is_none());
    set_message(cx, &view, "parent only");
    redraw(cx);
    assert!(
        can_submit(cx, &view),
        "a plain commit of the parent's staged file"
    );
}

#[gpui::test]
fn the_push_note_names_the_submodule_pushed_first(cx: &mut gpui::TestAppContext) {
    let (view, cx) = new_linked_view(cx);
    let repo_id = gitcomet_state::model::RepoId(74);
    draw_with(
        cx,
        &view,
        app_state_with_repo(parent_with_changed_submodules(74), repo_id),
    );
    assert!(cx.debug_bounds("linked_commit_push_note").is_none());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_push_after_enabled = true;
                cx.notify();
            });
        });
    });
    redraw(cx);
    assert!(cx.debug_bounds("linked_commit_push_note").is_some());
}

fn store_ops_rev(
    cx: &mut gpui::VisualTestContext,
    view: &LinkedView,
    repo_id: gitcomet_state::model::RepoId,
) -> u64 {
    cx.update(|_window, app| {
        let store = view.read(app).store.clone();
        store
            .barrier_for_test()
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("the store acknowledges the click");
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.ops_rev)
            .expect("the store holds the parent")
    })
}

#[gpui::test]
fn a_conflicted_submodule_file_cannot_be_staged_from_the_parent(cx: &mut gpui::TestAppContext) {
    let (view, cx) = new_linked_view(cx);
    let repo_id = gitcomet_state::model::RepoId(75);
    let mut repo = parent_with_changed_submodules(75);
    if let gitcomet_state::model::Loadable::Ready(changes) = &mut repo.submodule_changes {
        let mut changes = changes.as_ref().clone();
        changes[0].unstaged = Arc::new(vec![FileStatus {
            path: "manifest.toml".into(),
            kind: FileStatusKind::Conflicted,
            conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
        }]);
        repo.submodule_changes = gitcomet_state::model::Loadable::Ready(Arc::new(changes));
    }
    let state = app_state_with_repo(repo, repo_id);
    // The store holds the same state, so a stage would reach its reducer.
    cx.update(|_window, app| {
        view.read(app)
            .store
            .replace_snapshot_for_test(Arc::clone(&state));
    });
    draw_with(cx, &view, state);
    let before = store_ops_rev(cx, &view, repo_id);

    let conflicted = cx
        .debug_bounds("linked_commit_file_0_1")
        .expect("the conflicted row is drawn");
    cx.simulate_click(conflicted.center(), gpui::Modifiers::default());
    redraw(cx);
    assert_eq!(
        store_ops_rev(cx, &view, repo_id),
        before,
        "a conflicted file must not be staged from the parent's form"
    );

    // An ordinary row next to it still stages, so the click above landed.
    let plain = cx
        .debug_bounds("linked_commit_file_1_0")
        .expect("the other submodule's row is drawn");
    cx.simulate_click(plain.center(), gpui::Modifiers::default());
    assert_ne!(store_ops_rev(cx, &view, repo_id), before);
}

#[gpui::test]
fn a_submodule_message_is_forgotten_once_the_submodule_is_clean(cx: &mut gpui::TestAppContext) {
    let (view, cx) = new_linked_view(cx);
    let repo_id = gitcomet_state::model::RepoId(76);
    draw_with(
        cx,
        &view,
        app_state_with_repo(parent_with_changed_submodules(76), repo_id),
    );
    set_submodule_message(cx, &view, repo_id, "fixtures/syntax_test", "draft");

    // The submodules commit or discard their changes elsewhere: no section
    // shows, so none of them draws its message box.
    let mut clean = parent_with_changed_submodules(76);
    let gitcomet_state::model::Loadable::Ready(rows) = &clean.submodules else {
        unreachable!("the fixture has submodule rows");
    };
    let mut rows = rows.as_ref().clone();
    for row in &mut rows {
        row.has_inner_changes = false;
    }
    clean.submodules = gitcomet_state::model::Loadable::Ready(Arc::new(rows));
    clean.submodules_rev = 1;
    draw_with(cx, &view, app_state_with_repo(clean, repo_id));
    assert!(cx.debug_bounds("linked_commit_section_0").is_none());

    // It changes again: the old draft must not come back as its message.
    let mut changed = parent_with_changed_submodules(76);
    changed.submodules_rev = 2;
    draw_with(cx, &view, app_state_with_repo(changed, repo_id));
    assert!(cx.debug_bounds("linked_commit_section_0").is_some());
    let message = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .linked_message_inputs
            .get(&(repo_id, std::path::PathBuf::from("fixtures/syntax_test")))
            .map(|input| input.read(app).text().to_string())
    });
    assert_eq!(message.as_deref(), Some(""));
}
