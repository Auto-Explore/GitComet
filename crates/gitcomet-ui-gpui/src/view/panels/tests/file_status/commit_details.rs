//! Commit details: selectable fields, message links, signatures, input focus.

use super::*;

/// A window showing one commit's details, with the sha/date/parent fields
/// populated. Returns the fields' expected text.
fn commit_details_metadata_fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<crate::view::GitCometView>,
    &mut gpui::VisualTestContext,
    String,
    String,
    String,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(33);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let parent_sha = "89abcdef0123456789abcdef0123456789abcdef".to_string();
    let commit_date = "2026-03-08 12:34:56 +0200".to_string();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-metadata-copy"));
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: commit_date.clone(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(parent_sha.clone().into())],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
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

    (view, cx, commit_sha, commit_date, parent_sha)
}

#[gpui::test]
fn commit_details_metadata_fields_are_selectable(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, commit_date, parent_sha) = commit_details_metadata_fixture(cx);

    // One field at a time: only one surface may hold a selection, so selecting
    // all three at once would leave just the last one highlighted.
    for (label, expected) in [
        ("sha", commit_sha),
        ("date", commit_date),
        ("parent", parent_sha),
    ] {
        cx.update(|window, app| {
            let details_pane = view.read(app).details_pane.clone();
            details_pane.update(app, |pane, cx| {
                let input = match label {
                    "sha" => pane.commit_details_sha_input.clone(),
                    "date" => pane.commit_details_date_input.clone(),
                    _ => pane.commit_details_parent_input.clone(),
                };
                input.update(cx, |input, cx| input.select_all_text(window, cx));
            });
        });

        cx.update(|_window, app| {
            let details_pane = view.read(app).details_pane.clone();
            let pane = details_pane.read(app);
            let actual = match label {
                "sha" => pane.commit_details_sha_input.read(app).selected_text(),
                "date" => pane.commit_details_date_input.read(app).selected_text(),
                _ => pane.commit_details_parent_input.read(app).selected_text(),
            };
            assert_eq!(actual, Some(expected), "{label} field must be selectable");
        });
    }
}

/// Any other surface taking the window's selection clears a text input's
/// highlight, not just another input. Stands in for the diff pane, the
/// terminal, and any future selectable surface.
#[gpui::test]
fn another_surface_taking_the_selection_clears_a_text_input(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, _date, _parent) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha)
        );
    });

    cx.update(|window, app| {
        let mut elsewhere = crate::text_selection_owner::SelectionOwnerToken::default();
        elsewhere.adopt(window, app);
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            None,
            "the input must drop its highlight once another surface owns the selection"
        );
    });
}

/// The browser rule end to end: a real press on a surface that does not own the
/// selection collapses it. This is the counterpart to
/// `a_preserved_press_leaves_a_text_input_selection_alone` -- together they show
/// the capture-phase invalidator actually fires.
#[gpui::test]
fn an_ordinary_press_elsewhere_clears_a_text_input_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, _date, _parent) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha),
            "precondition: the input holds a selection"
        );
    });

    // Mid-window, clear of the titlebar and of any selectable text surface.
    let size = cx.update(|window, _app| window.viewport_size());
    let elsewhere = gpui::point(size.width * 0.5, size.height * 0.25);
    cx.simulate_mouse_move(elsewhere, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: elsewhere,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            None,
            "a press outside the owning surface must collapse the selection"
        );
    });
}

/// The overlay flag must fall back to closed on every dismiss path: one that
/// forgot would latch it on and disable click-away blur for the session.
#[gpui::test]
fn blur_still_works_after_a_stale_overlay_flag(cx: &mut gpui::TestAppContext) {
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);

    let input = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .clone()
    });

    // Stand in for a dismiss path that forgot to clear the flag: no popover is
    // open, but the flag says one is. The next render must correct it.
    cx.update(|window, app| crate::text_selection_owner::set_overlay_open(window, true, app));
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let handle = input.read(app).focus_handle();
        handle.focus(window, app);
    });
    cx.run_until_parked();

    let size = cx.update(|window, _app| window.viewport_size());
    let sidebar = gpui::point(size.width * 0.05, size.height * 0.5);
    cx.simulate_mouse_move(sidebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: sidebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    assert!(
        cx.update(|window, app| !input.read(app).focus_handle().is_focused(window)),
        "a stale overlay flag must be corrected by render, so blur still works"
    );
}

/// A selection-neutral gesture -- a scrollbar drag, a splitter -- must leave the
/// highlight alone even though it is a press outside the owning surface.
#[gpui::test]
fn a_preserved_press_leaves_a_text_input_selection_alone(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, _date, _parent) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });

    // The titlebar, a real `preserve` site -- dragging the window is chrome,
    // not content. Pressed for real so the whole capture/bubble/resolve chain
    // runs, exactly as in the clearing test above.
    let titlebar = gpui::point(gpui::px(2.0), gpui::px(2.0));
    cx.simulate_mouse_move(titlebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: titlebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha),
            "a preserved press must not collapse the selection"
        );
    });
}

/// Selecting one commit-details field must drop the highlight on the previous
/// one, the way a browser only ever shows one selection.
#[gpui::test]
fn selecting_a_second_commit_details_field_clears_the_first(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, commit_date, _parent_sha) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha)
        );
    });

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_date_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_details_date_input.read(app).selected_text(),
            Some(commit_date),
            "the newly selected field keeps its selection"
        );
        assert_eq!(
            pane.commit_details_sha_input.read(app).selected_text(),
            None,
            "the previously selected field must have been cleared"
        );
    });
}

/// Wait for the store worker to apply the selection a reveal dispatched.
///
/// GPUI's executor does not drive the store's worker thread, so
/// `run_until_parked` can return before `Msg::SelectCommit` has been reduced.
fn wait_for_store_selected_commit(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    expected: &str,
) {
    super::super::shortcuts::wait_until(cx, "store to select the revealed commit", |cx| {
        cx.update(|_window, app| {
            view.read(app)
                .store
                .snapshot()
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .and_then(|repo| repo.history_state.selected_commit.as_ref())
                .is_some_and(|id| id.as_ref() == expected)
        })
    });
}

/// Click inside a commit-details text input and report which menu, if any, the
/// popover host opened for it.
fn click_commit_details_link(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    click: gpui::Point<Pixels>,
    click_count: usize,
) -> Option<PopoverKind> {
    simulate_counted_click(cx, click, click_count);
    cx.run_until_parked();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

/// The first point inside the commit message, which every fixture below puts a
/// link at.
fn commit_details_message_link_point(cx: &mut gpui::VisualTestContext) -> gpui::Point<Pixels> {
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected commit details message bounds");
    point(bounds.left() + px(4.0), bounds.top() + px(8.0))
}

/// Show a single commit whose message is `message`, so its links can be clicked.
fn show_commit_details_message(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &str,
    message: &str,
) {
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new(workdir));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: message.to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                }));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

/// The message scrolls inside a container capped at
/// `COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX`; selecting a commit used to shape
/// every line of its message anyway, which on long pull-request descriptions
/// made that frame take 30-70 ms.
#[gpui::test]
fn selecting_a_commit_shapes_only_the_visible_part_of_a_long_message(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let message: String = std::iter::once("Long pull request\n\n".to_string())
        .chain((0..400).map(|line| format!("Body line {line} explains one more detail.\n")))
        .collect();

    let _ = gitcomet_ui_kit::test_support::take_wrapped_lines_shaped();
    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(43),
        "/tmp/repo-commit-message-long",
        &message,
    );
    let shaped = gitcomet_ui_kit::test_support::take_wrapped_lines_shaped();

    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected commit details message bounds");
    assert!(bounds.size.height <= px(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX + 1.0));
    assert!(
        (1..=60).contains(&shaped),
        "a 402-line message in a {:?} tall container shaped {shaped} lines",
        bounds.size.height
    );
}

#[gpui::test]
fn commit_details_message_url_click_opens_the_web_link_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(41),
        "/tmp/repo-commit-message-url",
        "https://example.com/issues/42 is fixed",
    );

    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::WebLinkMenu { ref url, .. })
                if url.as_ref() == "https://example.com/issues/42"
        ),
        "clicking a URL should open the same menu the markdown preview shows, got {popover:?}"
    );

    // The same menu the markdown preview offers, entry for entry.
    assert!(
        cx.debug_bounds("context_menu_open_in_web_browser")
            .is_some(),
        "expected an entry that opens the link"
    );
    assert!(
        cx.debug_bounds("context_menu_copy_link_address").is_some(),
        "expected an entry that copies the address"
    );

    // Reached from the details pane, so closing it must not hand the keyboard
    // to the diff panel the way a preview link does.
    cx.update(|_window, app| {
        assert!(
            !view
                .read(app)
                .popover_host
                .read(app)
                .popover_opened_from_diff_panel_for_tests(),
            "a commit message is not a diff-panel invoker"
        );
    });

    // The menu hangs off the link's own box, not off the row or the panel, so
    // it opens flush under the words it describes.
    cx.update(|_window, app| {
        let anchor = view
            .read(app)
            .popover_host
            .read(app)
            .popover_anchor_bounds_for_tests()
            .expect("a link menu anchors on the link's box");
        let details_pane = view.read(app).details_pane.clone();
        let expected = details_pane
            .read(app)
            .commit_details_message_input
            .read(app)
            .hotspot_bounds(&(0.."https://example.com/issues/42".len()))
            .expect("expected bounds for the link");
        assert_eq!(anchor, expected);
        assert!(anchor.contains(&link), "the click landed inside that box");
    });
}

#[gpui::test]
fn commit_details_message_mailto_click_opens_the_web_link_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(42),
        "/tmp/repo-commit-message-mailto",
        "mailto:maintainer@example.com reported this",
    );

    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::WebLinkMenu { ref url, .. })
                if url.as_ref() == "mailto:maintainer@example.com"
        ),
        "clicking a mailto link should open the link menu, got {popover:?}"
    );
}

/// The message that motivated the stricter commit-id rules, end to end: build
/// ids, a Gerrit change id and URL path segments all used to linkify as commits.
#[gpui::test]
fn commit_details_message_trailers_do_not_linkify_as_commits(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(43),
        "/tmp/repo-commit-message-trailers",
        concat!(
            "Cr-Original-Build-Id: 8674534147806418049\n",
            "Change-Id: I7a5d480873e839444e4e188ffa87f9c635e2fb81\n",
        ),
    );

    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        popover.is_none(),
        "a build id is not a commit id, so clicking it should open nothing, got {popover:?}"
    );
}

/// A Chromium PGO roll: two build artifacts whose names are built out of two
/// full-length hashes each. Every one of them used to linkify.
#[gpui::test]
fn commit_details_message_filename_hashes_do_not_linkify_as_commits(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(44),
        "/tmp/repo-commit-message-profdata",
        concat!(
            "Roll Chrome Mac PGO profile from ",
            "chrome-mac-7922-1785736271-37240ae8aae5f01fc00cbf0b7ea19b73826e0dba",
            "-d9e99b2bafcc6df3c2a5bf803fcb5483d33dbdd0.profdata to ",
            "chrome-mac-7922-1785755104-c2eee60da6765f60eca833b7c5c0d85ddcbc2940",
            "-551a1e94b700524e479bd2d64ccaf8cdb71d43a6.profdata",
        ),
    );

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let links = details_pane
            .read(app)
            .commit_details_message_link_menu
            .read(app)
            .links_for_tests();
        assert!(
            links.is_empty(),
            "hashes joined into a filename are not commit ids, got {links:?}"
        );
    });
}

#[gpui::test]
fn commit_details_message_sha_click_menu_navigate_reveals_referenced_commit(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(34);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";
    let target_sha_upper = target_sha.to_ascii_uppercase();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-sha"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(current_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "current".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(target_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "target".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha_upper} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::CommitShaLinkMenu {
                ref commit_id,
                allow_navigate: true,
                ..
            }) if commit_id.as_ref() == target_sha
        ),
        // The message spells the SHA in upper case; the link resolves to the
        // lower-case id the repository uses.
        "clicking a commit id should open its menu, got {popover:?}"
    );

    let reveal_bounds = cx
        .debug_bounds("context_menu_reveal_commit")
        .expect("expected reveal commit entry");
    simulate_counted_click(cx, reveal_bounds.center(), 1);
    cx.run_until_parked();
    wait_for_store_selected_commit(cx, &view, repo_id, target_sha);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(target_sha.into());
        assert_eq!(selected, Some(&expected));
    });
}

/// Only the link opens a menu. Clicking the prose beside it must leave the
/// popover host alone, otherwise every caret placement would raise a menu.
#[gpui::test]
fn commit_details_message_click_beside_a_sha_opens_no_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(36);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-message-hover-close"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(current_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "current".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(target_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "target".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                }));
            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected commit details message bounds");
    // The SHA occupies the head of the line; " fixes the regression" follows it.
    let link = commit_details_message_link_point(cx);
    let beside_link = point(link.x + px(320.0), link.y);
    assert!(
        beside_link.x < bounds.right(),
        "the point past the SHA has to stay inside the message"
    );

    let popover = click_commit_details_link(cx, &view, beside_link, 1);
    assert!(
        popover.is_none(),
        "clicking plain message text should open no menu, got {popover:?}"
    );

    // …and the link itself still does, so the point above was not simply outside
    // the input.
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(popover, Some(PopoverKind::CommitShaLinkMenu { ref commit_id, .. })
            if commit_id.as_ref() == target_sha),
        "clicking the commit id should open its menu, got {popover:?}"
    );
}

#[gpui::test]
fn commit_details_message_sha_retained_details_are_inert_after_selection_changes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(38);
    let retained_sha = "0123456789abcdef0123456789abcdef01234567";
    let selected_sha = "fedcba9876543210fedcba9876543210fedcba98";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-retained-commit-details-sha"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(retained_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "retained".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(selected_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "selected".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(target_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "target".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(selected_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(retained_sha.into()),
                    message: format!("{target_sha} should not reveal from retained details"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected retained commit details message bounds");
    let hover = point(bounds.left() + px(4.0), bounds.top() + px(8.0));
    cx.simulate_mouse_move(hover, None, Modifiers::default());
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(301));
    cx.run_until_parked();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(selected_sha.into());
        assert_eq!(selected, Some(&expected));
    });
    let popover = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        popover.is_none(),
        "expected retained details to stay inert, got {popover:?}"
    );
}

/// Opening the menu is not the same as acting on it: the click itself must not
/// move the history, only offer to.
#[gpui::test]
fn commit_details_message_sha_click_opens_the_menu_without_navigating(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(35);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-sha-select"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let click = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, click, 1);
    assert!(
        matches!(popover, Some(PopoverKind::CommitShaLinkMenu { ref commit_id, .. })
            if commit_id.as_ref() == target_sha),
        "expected the commit link menu, got {popover:?}"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_details_message_input.read(app).selected_text(),
            None
        );
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(current_sha.into());
        assert_eq!(selected, Some(&expected));
    });
}

/// Selecting the words of a link has to keep working, so only a plain click
/// follows it — a double click is a word selection like anywhere else.
#[gpui::test]
fn commit_details_message_sha_double_click_selects_instead_of_opening_the_menu(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(41);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-sha-focus"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let click = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, click, 2);
    assert!(
        popover.is_none(),
        "a double click on a link should select its text, not open a menu, got {popover:?}"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_details_message_input
                .read(app)
                .selected_text()
                .as_deref(),
            Some(target_sha),
            "the double click should have selected the whole commit id"
        );
    });
}

#[gpui::test]
fn commit_details_parent_sha_click_menu_navigate_reveals_referenced_commit(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(39);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let parent_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-parent-hover-menu"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(current_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "current".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(parent_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "parent".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: "subject".into(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(parent_sha.into())],
                    files: vec![],
                }));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let parent_bounds = cx
        .debug_bounds("commit_details_parent_link_menu")
        .expect("expected parent sha link target");
    let popover = click_commit_details_link(
        cx,
        &view,
        point(parent_bounds.left() + px(4.0), parent_bounds.center().y),
        1,
    );
    assert!(
        matches!(
            popover,
            Some(PopoverKind::CommitShaLinkMenu {
                ref commit_id,
                allow_navigate: true,
                ..
            }) if commit_id.as_ref() == parent_sha
        ),
        "clicking the parent id should open its menu, got {popover:?}"
    );

    let reveal_bounds = cx
        .debug_bounds("context_menu_reveal_commit")
        .expect("expected parent reveal commit entry");
    simulate_counted_click(cx, reveal_bounds.center(), 1);
    cx.run_until_parked();
    wait_for_store_selected_commit(cx, &view, repo_id, parent_sha);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(parent_sha.into());
        assert_eq!(selected, Some(&expected));
    });
}

#[gpui::test]
fn commit_details_parent_sha_dash_has_no_link_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(40);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-parent-dash-hover"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: "subject".into(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                }));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let parent_bounds = cx
        .debug_bounds("commit_details_parent_link_menu")
        .expect("expected parent sha link target");
    let popover = click_commit_details_link(
        cx,
        &view,
        point(parent_bounds.left() + px(4.0), parent_bounds.center().y),
        1,
    );

    assert!(
        popover.is_none(),
        "expected placeholder parent value to stay non-interactive, got {popover:?}"
    );
}

/// A press on something that takes no focus of its own must still blur a
/// focused input, the way clicking page background does in a browser.
#[gpui::test]
fn clicking_outside_a_focused_input_blurs_it(cx: &mut gpui::TestAppContext) {
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);

    let input = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .clone()
    });
    cx.update(|window, app| {
        let handle = input.read(app).focus_handle();
        handle.focus(window, app);
    });
    cx.run_until_parked();
    assert!(
        cx.update(|window, app| input.read(app).focus_handle().is_focused(window)),
        "precondition: the input is focused"
    );

    // The branch sidebar: a real surface that takes no focus of its own, so
    // without an explicit blur the input stays focused behind the user's back.
    let size = cx.update(|window, _app| window.viewport_size());
    let sidebar = gpui::point(size.width * 0.05, size.height * 0.5);
    cx.simulate_mouse_move(sidebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: sidebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    assert!(
        cx.update(|window, app| !input.read(app).focus_handle().is_focused(window)),
        "a press on a surface that takes no focus must still blur the input"
    );
    // Deliberately nothing focused rather than a fallback surface: the press
    // landed on something that is not a pane, so handing focus to one would
    // silently redirect the user's next keystroke. Global bindings still
    // resolve at the window root.
    assert!(
        cx.update(|window, app| window.focused(app).is_none()),
        "blur leaves no element focused"
    );
}

/// ...but a selection-neutral gesture must not. Dragging a scrollbar or the
/// titlebar while typing is not "clicking away".
#[gpui::test]
fn a_preserved_press_does_not_blur_a_focused_input(cx: &mut gpui::TestAppContext) {
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);

    let input = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .clone()
    });
    cx.update(|window, app| {
        let handle = input.read(app).focus_handle();
        handle.focus(window, app);
    });
    cx.run_until_parked();

    // The titlebar: a real `preserve` site.
    let titlebar = gpui::point(gpui::px(2.0), gpui::px(2.0));
    cx.simulate_mouse_move(titlebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: titlebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    assert!(
        cx.update(|window, app| input.read(app).focus_handle().is_focused(window)),
        "dragging the window must not blur the input being typed in"
    );
}

#[gpui::test]
fn commit_details_shows_a_badge_for_a_verified_signature(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (_view, cx) = commit_details_signature_fixture(
        cx,
        Some(test_signature(gitcomet_core::domain::SignatureStatus::Good)),
    );

    assert!(
        cx.debug_bounds("commit_details_signature_badge").is_some(),
        "a verified commit should render the signature badge"
    );
}

#[gpui::test]
fn commit_details_shows_a_badge_for_a_bad_signature(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (_view, cx) = commit_details_signature_fixture(
        cx,
        Some(test_signature(gitcomet_core::domain::SignatureStatus::Bad)),
    );

    assert!(
        cx.debug_bounds("commit_details_signature_badge").is_some(),
        "a bad signature must be surfaced, not hidden"
    );
}

#[gpui::test]
fn commit_details_shows_no_badge_without_a_signature_verdict(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (_view, cx) = commit_details_signature_fixture(cx, None);

    assert!(
        cx.debug_bounds("commit_details_signature_badge").is_none(),
        "an unsigned or unverifiable commit must render no badge"
    );
}

#[gpui::test]
fn commit_links_cancel_cross_link_and_outside_releases(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let first_url = "https://example.com/a";
    let second_url = "https://example.com/b";
    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(174),
        "/tmp/commit_link_cancellation",
        &format!("{first_url}\n{second_url}"),
    );
    let [first, second] = cx.update(|_, app| {
        let input = view
            .read(app)
            .details_pane
            .read(app)
            .commit_details_message_input
            .read(app);
        let second_start = first_url.len() + 1;
        [
            0..first_url.len(),
            second_start..second_start + second_url.len(),
        ]
        .map(|range| input.hotspot_bounds(&range).unwrap().center())
    });
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .unwrap();
    let outside = point(bounds.left() - px(8.0), bounds.top() + px(8.0));
    for (from, to) in [(first, second), (first, outside), (outside, first)] {
        cx.simulate_event(MouseDownEvent {
            position: from,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position: to,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        draw_and_drain_test_window(cx);
        cx.update(|_, app| {
            assert!(
                !view.read(app).popover_host.read(app).is_open(),
                "only a press and release on the same link can open its menu"
            )
        });
    }
    let menu = click_commit_details_link(cx, &view, first, 1);
    assert!(
        matches!(menu, Some(PopoverKind::WebLinkMenu { ref url, .. }) if url.as_ref() == first_url)
    );
}

/// Chips arrive after the list (with line stats), so the row must pick them
/// up from the snapshot and the details pane must repaint for them.
#[gpui::test]
fn status_row_shows_large_file_chip_for_managed_paths(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(7431);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_chip",
        std::process::id()
    ));

    let push = |cx: &mut gpui::VisualTestContext, with_chip: bool| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    std::path::PathBuf::from("art/hero.psd"),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                if with_chip {
                    let mut files = gitcomet_core::large_files::UncommittedLargeFiles::default();
                    files.unstaged.insert(
                        std::path::PathBuf::from("art/hero.psd"),
                        gitcomet_core::large_files::LargeFileState {
                            pointer: gitcomet_core::large_files::LargeFilePointer::Lfs(
                                gitcomet_core::lfs::LfsPointer {
                                    oid: gitcomet_core::lfs::LfsOid([3; 32]),
                                    size: 42,
                                },
                            ),
                            in_local_store: Some(false),
                            worktree: Some(gitcomet_core::large_files::LargeFileWorktree::Pointer),
                            lockable: false,
                        },
                    );
                    repo.uncommitted_large_files = Arc::new(files);
                    repo.large_files_rev = 1;
                }
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
        draw_and_drain_test_window(cx);
    };

    push(cx, false);
    assert!(
        cx.debug_bounds("status_row_large_file_0").is_none(),
        "plain rows carry no chip"
    );
    push(cx, true);
    assert!(
        cx.debug_bounds("status_row_large_file_0").is_some(),
        "managed rows show the chip once their state arrives"
    );
}

/// Frames caused elsewhere must not re-render the cached details pane. The
/// commit link menus used to notify on every `sync`, which the details render
/// calls, so after its first re-render the pane re-rendered on every frame
/// (5 -> 15 renders over these 10 frames).
#[gpui::test]
fn commit_details_pane_ignores_unrelated_frames(cx: &mut gpui::TestAppContext) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);
    // One legitimate re-render, as any status publication causes: the menus
    // are tracked by the window from here on.
    cx.update(|_window, app| {
        let details = view.read(app).details_pane.clone();
        details.update(app, |_, cx| cx.notify());
    });
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let render_count = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).details_pane.read(app).render_count)
    };
    let before = render_count(cx);

    for _ in 0..5 {
        cx.update(|_window, app| {
            let sidebar = view.read(app).sidebar_pane.clone();
            sidebar.update(app, |_, cx| cx.notify());
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }

    assert_eq!(
        render_count(cx),
        before,
        "an unrelated frame re-rendered the details pane"
    );
}

/// Frames caused elsewhere must not re-render the cached details pane while
/// its file list truncates paths. The list's path alignment group reset
/// whenever the uniform list's measure pass and its visible rows, or rows of
/// different widths (a binary file has no stat column), took turns, so the
/// anchor never resolved and each frame's layout notified the pane again
/// (5 -> 15 renders over these 10 frames).
#[gpui::test]
fn commit_file_list_with_truncated_paths_ignores_unrelated_frames(cx: &mut gpui::TestAppContext) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(700.0)));
    let repo_id = gitcomet_state::model::RepoId(36);
    let commit_id =
        gitcomet_core::domain::CommitId("0123456789abcdef0123456789abcdef01234567".into());
    let file = |path: &str, stats: Option<(u32, u32)>| {
        let mut change = gitcomet_core::domain::CommitFileChange::new(
            path.into(),
            gitcomet_core::domain::FileStatusKind::Modified,
        );
        change.additions = stats.map(|(added, _)| added);
        change.deletions = stats.map(|(_, removed)| removed);
        change
    };
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-mixed-widths"));
            repo.history_state.selected_commit = Some(commit_id.clone());
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![
                        file(
                            "assets/some/deeply/nested/directory/structure/image_resource.png",
                            None,
                        ),
                        file(
                            "src/some/deeply/nested/directory/structure/with/a/long/module_name.rs",
                            Some((12, 3)),
                        ),
                        file(
                            "tests/another/deeply/nested/directory/structure/integration_case.rs",
                            Some((1, 1)),
                        ),
                    ],
                },
            ));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|_window, app| {
        let details = view.read(app).details_pane.clone();
        details.update(app, |_, cx| cx.notify());
    });
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let render_count = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).details_pane.read(app).render_count)
    };
    let before = render_count(cx);
    for _ in 0..5 {
        cx.update(|_window, app| {
            let sidebar = view.read(app).sidebar_pane.clone();
            sidebar.update(app, |_, cx| cx.notify());
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let snapshot = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_files_path_alignment_group
            .snapshot_for_test()
    });
    assert_eq!(
        render_count(cx),
        before,
        "an unrelated frame re-rendered the details pane; alignment {snapshot:?}"
    );
}
