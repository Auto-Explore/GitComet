//! The one layout icon every changed-file list carries: Flat list, Tree and
//! Groups, by click or by its right-click menu.

use super::*;
use crate::view::FileListLayout;
use crate::view::rows::FileListId;
use gitcomet_core::domain::FileStatusKind;

/// A commit whose files are of mixed kinds, under one folder.
fn commit_with_kinds(
    repo_id: gitcomet_state::model::RepoId,
    files: &[(&str, FileStatusKind)],
) -> gitcomet_state::model::RepoState {
    commit_with(
        repo_id,
        files
            .iter()
            .map(|(path, kind)| {
                gitcomet_core::domain::CommitFileChange::new((*path).into(), *kind)
                    .with_line_counts(Some(1), Some(1))
            })
            .collect(),
    )
}

fn commit_with(
    repo_id: gitcomet_state::model::RepoId,
    files: Vec<gitcomet_core::domain::CommitFileChange>,
) -> gitcomet_state::model::RepoState {
    let commit_id = gitcomet_core::domain::CommitId("groups0123456789".into());
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-list-layouts"));
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details =
        gitcomet_state::model::Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
            id: commit_id,
            message: "subject".to_string(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "2026-10-04 12:00:00 +0300".to_string(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files,
        }));
    repo.history_state.commit_details_rev = 1;
    repo.history_state.selected_commit_rev = 1;
    repo
}

fn mount(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    repo: gitcomet_state::model::RepoState,
) -> (
    gpui::Entity<crate::view::GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    (view, cx)
}

fn layout_of(
    view: &gpui::Entity<crate::view::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: gitcomet_state::model::RepoId,
    list: FileListId,
) -> FileListLayout {
    cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .file_list_layout_for(repo_id, list)
    })
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
}

fn right_click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector}"))
        .center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
}

/// A click steps Flat list → Tree → Groups → Flat list, and the icon takes
/// each layout's shape. Groups puts the files under their change kinds.
#[gpui::test]
fn the_layout_icon_steps_through_every_layout_in_its_shape(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = gitcomet_state::model::RepoId(640);
    let repo = commit_with_kinds(
        repo_id,
        &[
            ("src/a.rs", FileStatusKind::Modified),
            ("src/b.rs", FileStatusKind::Added),
            ("src/c.rs", FileStatusKind::Deleted),
        ],
    );
    let (view, cx) = mount(cx, repo_id, repo);
    let group = |kind: &str| leaked(format!("commit_file_group_{}_{kind}", repo_id.0));

    assert!(cx.debug_bounds("commit_file_layout_button_flat").is_some());
    assert!(cx.debug_bounds(group("Added")).is_none());

    click(cx, "commit_file_layout_button");
    assert_eq!(
        layout_of(&view, cx, repo_id, FileListId::CommitFiles),
        FileListLayout::Tree
    );
    assert!(cx.debug_bounds("commit_file_layout_button_tree").is_some());
    assert!(cx.debug_bounds("commit_file_layout_button_flat").is_none());

    click(cx, "commit_file_layout_button");
    assert_eq!(
        layout_of(&view, cx, repo_id, FileListId::CommitFiles),
        FileListLayout::Groups
    );
    assert!(
        cx.debug_bounds("commit_file_layout_button_groups")
            .is_some()
    );
    for kind in ["Added", "Modified", "Deleted"] {
        assert!(cx.debug_bounds(group(kind)).is_some(), "{kind} header");
    }
    // Drawn and navigated in group order: Added, Modified, Deleted.
    let drawn = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .active_commit_file_source_indices(repo_id)
            .expect("a projection")
    });
    assert_eq!(drawn.as_ref(), &[1, 0, 2]);

    click(cx, "commit_file_layout_button");
    assert!(cx.debug_bounds("commit_file_layout_button_flat").is_some());
    assert!(cx.debug_bounds(group("Added")).is_none());
}

/// A right click on the icon offers the three layouts, the list's own
/// checked, and choosing one applies it without stepping.
#[gpui::test]
fn right_clicking_the_layout_icon_offers_every_layout(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = gitcomet_state::model::RepoId(641);
    let repo = commit_with_kinds(
        repo_id,
        &[
            ("a.rs", FileStatusKind::Modified),
            ("b.rs", FileStatusKind::Added),
        ],
    );
    let (view, cx) = mount(cx, repo_id, repo);

    right_click(cx, "commit_file_layout_button");
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()),
        Some(PopoverKind::FileListLayoutMenu {
            repo_id,
            list: FileListId::CommitFiles,
        })
    );
    for entry in [
        "context_menu_flat_list",
        "context_menu_tree",
        "context_menu_groups",
    ] {
        assert!(cx.debug_bounds(entry).is_some(), "{entry}");
    }
    assert_eq!(
        layout_of(&view, cx, repo_id, FileListId::CommitFiles),
        FileListLayout::Flat,
        "opening the menu does not step"
    );

    click(cx, "context_menu_groups");
    assert_eq!(
        layout_of(&view, cx, repo_id, FileListId::CommitFiles),
        FileListLayout::Groups
    );
    assert!(cx.debug_bounds("context_menu_groups").is_none(), "closed");
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_group_{}_Added", repo_id.0)))
            .is_some()
    );
}

/// Grouped status rows: prev/next and ranges follow the groups' order, a
/// collapsed group keeps its files navigable, and revealing a file opens it.
#[gpui::test]
fn grouped_status_sections_keep_their_drawn_order_and_reveal_hidden_files(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let repo_id = gitcomet_state::model::RepoId(642);
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-status-groups"));
    repo.worktree_status = gitcomet_state::model::Loadable::Ready(Arc::new(
        [
            ("a.rs", FileStatusKind::Modified),
            ("b.rs", FileStatusKind::Deleted),
            ("c.rs", FileStatusKind::Modified),
            ("d.rs", FileStatusKind::Conflicted),
        ]
        .iter()
        .map(|(path, kind)| gitcomet_core::domain::FileStatus {
            path: (*path).into(),
            kind: *kind,
            conflict: None,
        })
        .collect(),
    ));
    repo.worktree_status_rev = 1;
    let (view, cx) = mount(cx, repo_id, repo);
    let list = FileListId::Status(crate::view::StatusSection::CombinedUnstaged);

    right_click(cx, "status_unstaged_layout_button");
    click(cx, "context_menu_groups");
    assert!(
        cx.debug_bounds(leaked(format!(
            "status_group_{}_unstaged_Conflicted",
            repo_id.0
        )))
        .is_some()
    );
    let order = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let pane = view.read(app).details_pane.read(app);
            let repo = pane.active_repo().expect("a repo");
            let entries = repo.worktree_status_entries().expect("entries");
            pane.active_status_section_order(repo_id, crate::view::StatusSection::CombinedUnstaged)
                .expect("an order")
                .iter()
                .map(|ix| entries[*ix].path.display().to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(order(cx), ["d.rs", "a.rs", "c.rs", "b.rs"]);

    // Collapse Modified: its rows go, its files stay in the order.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_group(repo_id, list, 2, cx)
            })
        })
    });
    draw_and_drain_test_window(cx);
    assert_eq!(order(cx), ["d.rs", "a.rs", "c.rs", "b.rs"]);
    let row = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.reveal_status_row(crate::view::StatusSection::CombinedUnstaged, 2, cx)
            })
        })
    });
    assert_eq!(row, Some(4), "Modified opens again; c.rs is its second row");
}

/// Scrolled into a long group, its header stays pinned above the rows.
#[gpui::test]
fn a_long_group_keeps_its_header_pinned(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = gitcomet_state::model::RepoId(643);
    let paths: Vec<String> = (0..80).map(|n| format!("src/file_{n:02}.rs")).collect();
    let files: Vec<(&str, FileStatusKind)> = paths
        .iter()
        .map(|path| (path.as_str(), FileStatusKind::Modified))
        .collect();
    let repo = commit_with_kinds(repo_id, &files);
    let (view, cx) = mount(cx, repo_id, repo);
    right_click(cx, "commit_file_layout_button");
    click(cx, "context_menu_groups");
    let scroll = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_files_scroll
            .clone()
    });
    scroll.scroll_to_item(60, gpui::ScrollStrategy::Top);
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds(leaked(format!(
            "commit_file_group_{}_sticky_Modified",
            repo_id.0
        )))
        .is_some()
    );
}

/// Layout and sort combine: Groups with the Edits sort keeps each kind's
/// files in edit order, the shared edit first.
#[gpui::test]
fn groups_and_the_edits_sort_mix_in_one_list(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = gitcomet_state::model::RepoId(644);
    let edited = |path: &str, kind, text: &str| {
        let mut edit = gitcomet_core::edit_signature::EditSignatureBuilder::default();
        edit.added(text.as_bytes());
        gitcomet_core::domain::CommitFileChange::new(path.into(), kind)
            .with_line_counts(Some(1), Some(0))
            .with_edit(edit.finish())
    };
    let repo = commit_with(
        repo_id,
        vec![
            edited("a.rs", FileStatusKind::Modified, "x"),
            edited("b.rs", FileStatusKind::Added, "x"),
            edited("c.rs", FileStatusKind::Modified, "y"),
            edited("d.rs", FileStatusKind::Modified, "x"),
            edited("e.rs", FileStatusKind::Added, "z"),
        ],
    );
    let (view, cx) = mount(cx, repo_id, repo);
    let drawn = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .details_pane
                .read(app)
                .active_commit_file_source_indices(repo_id)
                .expect("a projection")
        })
    };

    click(cx, "commit_file_sort_button");
    click(cx, "context_menu_edits_repeated_first");
    // The three files sharing "x" first, then the singles by path.
    assert_eq!(drawn(cx).as_ref(), &[0, 1, 3, 2, 4]);

    right_click(cx, "commit_file_layout_button");
    click(cx, "context_menu_groups");
    // Added: b (x), e; Modified: a, d (x), c.
    assert_eq!(drawn(cx).as_ref(), &[1, 4, 0, 3, 2]);
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .details_pane
            .read(app)
            .file_list_sort_for(FileListId::CommitFiles)),
        crate::view::rows::CommitFileSort::Edits,
        "the layout leaves the sort alone"
    );
}
