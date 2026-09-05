use super::branch::{create_tracking_store, wait_until};
use super::*;
use gitcomet_core::domain::{Branch, CommitId};

#[gpui::test]
fn repo_picker_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::RepoPicker,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(is_open, "expected Repository popover to open");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close Repository popover");
}

#[gpui::test]
fn branch_picker_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("branch-picker-escape");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::BranchPicker {
                        purpose: BranchPickerPurpose::Checkout,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(is_open, "expected Branch popover to open");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close Branch popover");
}

#[gpui::test]
fn branch_picker_escape_closes_while_branches_unavailable(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) =
        create_tracking_store("branch-picker-escape-unavailable");
    let repo_id = store
        .snapshot()
        .active_repo
        .expect("expected active repo for branch picker test");
    store.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Err(Error::new(ErrorKind::Unsupported(
                "branches unavailable in test",
            ))),
        },
    ));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::BranchPicker {
                        purpose: BranchPickerPurpose::Checkout,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(is_open, "expected Branch popover to open");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        !is_open,
        "expected Escape to close unavailable Branch popover"
    );
}

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";

/// Seeds a session with a repository that is *not* open, then checks the
/// repository picker splits its rows into the open and recently-closed
/// sections. Runs in a subprocess so the session-file override is set before
/// the session is first read.
#[test]
fn repo_picker_lists_recently_closed_repositories_wrapper() {
    if std::env::var_os(SESSION_FILE_ENV).is_some() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let closed_repo = dir.path().join("closed-repo");
    std::fs::create_dir_all(&closed_repo).expect("create closed repo dir");
    let session_file = dir.path().join("session.json");
    gitcomet_state::session::persist_recent_repo_to_path(&closed_repo, &session_file)
        .expect("seed recent repo session");

    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = gitcomet_core::process::background_command(current_exe)
        .arg("repo_picker_lists_recently_closed_repositories_subprocess")
        .arg("--nocapture")
        .env(SESSION_FILE_ENV, &session_file)
        .output()
        .expect("spawn subtest process");
    assert!(
        output.status.success(),
        "subtest failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[gpui::test]
fn repo_picker_lists_recently_closed_repositories_subprocess(cx: &mut gpui::TestAppContext) {
    if std::env::var_os(SESSION_FILE_ENV).is_none() {
        return;
    }

    let _visual_guard = crate::test_support::lock_visual_test();
    let closed_repo = gitcomet_state::session::load()
        .recent_repos
        .into_iter()
        .next()
        .expect("seeded recent repo");
    let closed_repo = gitcomet_core::path_utils::canonicalize_or_original(closed_repo);

    let (store, events, _repo, workdir) = create_tracking_store("repo-picker-recently-closed");
    let open_workdir = gitcomet_core::path_utils::canonicalize_or_original(workdir);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::RepoPicker,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    let entries = cx.update(|_window, app| {
        let host = view.read(app).popover_host.clone();
        let host = host.read(app);
        repo_picker::entries(host)
            .into_iter()
            .map(|(entry, _)| entry)
            .collect::<Vec<_>>()
    });

    let open_paths = entries
        .iter()
        .filter_map(|entry| match entry {
            repo_picker::RepoPickerEntry::Open(_) => Some(()),
            _ => None,
        })
        .count();
    assert_eq!(open_paths, 1, "expected the tracked repository to be open");

    let recently_closed = entries
        .iter()
        .filter_map(|entry| match entry {
            repo_picker::RepoPickerEntry::Closed(path) => Some(path.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        recently_closed,
        vec![closed_repo],
        "expected only the non-open session recent to be listed as recently closed"
    );
    assert!(
        !recently_closed.contains(&open_workdir),
        "an open repository must not also appear under recently closed"
    );

    let expected_badge_size = gpui::size(
        gpui::px(components::REPOSITORY_BADGE_SIZE_PX),
        gpui::px(components::REPOSITORY_BADGE_SIZE_PX),
    );
    assert_eq!(
        cx.debug_bounds("picker_prompt_repository_badge_0")
            .expect("expected an initials badge for the open repository")
            .size,
        expected_badge_size,
    );
    assert_eq!(
        cx.debug_bounds("picker_prompt_repository_badge_1")
            .expect("expected an initials badge for the recently closed repository")
            .size,
        expected_badge_size,
    );
}

/// Drives the picker's sort menu over a seeded set of closed repositories.
/// Runs in a subprocess so the session-file override is set before the session
/// is first read.
#[test]
fn repo_picker_sort_menu_reorders_rows_wrapper() {
    if std::env::var_os(SESSION_FILE_ENV).is_some() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");
    // Persisted newest-last so the session's MRU order ends up alpha, zulu, mike.
    for repo in ["c-parent/mike", "a-parent/zulu", "b-parent/Alpha"] {
        let path = dir.path().join(repo);
        std::fs::create_dir_all(&path).expect("create seeded repo dir");
        gitcomet_state::session::persist_recent_repo_to_path(&path, &session_file)
            .expect("seed recent repo session");
    }

    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = gitcomet_core::process::background_command(current_exe)
        .arg("repo_picker_sort_menu_reorders_rows_subprocess")
        .arg("--nocapture")
        .env(SESSION_FILE_ENV, &session_file)
        .output()
        .expect("spawn subtest process");
    assert!(
        output.status.success(),
        "subtest failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[gpui::test]
fn repo_picker_sort_menu_reorders_rows_subprocess(cx: &mut gpui::TestAppContext) {
    if std::env::var_os(SESSION_FILE_ENV).is_none() {
        return;
    }

    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::RepoPicker,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());
    let row_names = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            repo_picker::entries(popover_host.read(app))
                .into_iter()
                .filter_map(|(entry, _)| match entry {
                    repo_picker::RepoPickerEntry::Closed(path) => Some(
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    repo_picker::RepoPickerEntry::Open(_) => None,
                })
                .collect::<Vec<_>>()
        })
    };
    let set_sort = |cx: &mut gpui::VisualTestContext, sort: repo_picker::RepoPickerSort| {
        cx.update(|_window, app| {
            popover_host.update(app, |host, cx| {
                repo_picker::apply_sort(host, sort, cx);
            });
        });
    };

    assert_eq!(
        row_names(cx),
        vec!["Alpha", "zulu", "mike"],
        "expected the session's most-recent-first order by default"
    );

    set_sort(cx, repo_picker::RepoPickerSort::Oldest);
    assert_eq!(row_names(cx), vec!["mike", "zulu", "Alpha"]);

    set_sort(cx, repo_picker::RepoPickerSort::Name);
    assert_eq!(row_names(cx), vec!["Alpha", "mike", "zulu"]);

    set_sort(cx, repo_picker::RepoPickerSort::Path);
    assert_eq!(row_names(cx), vec!["zulu", "Alpha", "mike"]);

    // The chosen sort is written back to the session file.
    assert_eq!(
        gitcomet_state::session::load().repo_picker_sort.as_deref(),
        Some("path")
    );
}

#[gpui::test]
fn repo_picker_sort_menu_takes_over_navigation_and_escape(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::RepoPicker,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            repo_picker::toggle_sort_menu(host, cx);
            assert_eq!(
                repo_picker::nav_targets(host, "", cx).len(),
                repo_picker::RepoPickerSort::ALL.len(),
                "arrow keys should walk the sort options while the menu is open"
            );

            // Escape backs out of the menu before it closes the picker.
            repo_picker::dismiss(host, cx);
            assert!(!host.repo_picker_sort_menu_open);
            assert!(host.is_open(), "expected the picker to stay open");

            repo_picker::dismiss(host, cx);
            assert!(!host.is_open(), "expected Escape to close the picker");
        });
    });
}

#[gpui::test]
fn repo_picker_search_selects_first_result_and_enter_activates_it(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-auto-select");
    let snapshot = store.snapshot();
    let repo_id = snapshot.active_repo.expect("expected an active repository");
    let open_workdir = snapshot
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("expected active repository state")
        .spec
        .workdir
        .clone();
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());
    let input = cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            // Two matching rows in different sections prove index zero is the
            // first displayed result across the whole picker, not merely the
            // first Recently Closed match.
            host.cached_pinned_repos = vec![open_workdir.clone()];
            host.cached_recent_repos = vec![std::path::PathBuf::from(
                "/tmp/repo-picker-auto-select-closed",
            )];
            repo_picker::toggle_sort_menu(host, cx);
            host.repo_picker_search_input
                .clone()
                .expect("repository picker search input")
        })
    });

    input.update(cx, |input, cx| input.set_text("repo-picker", cx));
    cx.run_until_parked();
    cx.update(|_window, app| {
        let host = popover_host.read(app);
        assert!(
            !host.repo_picker_sort_menu_open,
            "typing should dismiss the Sort menu and reveal repository results"
        );
        assert_eq!(
            host.repo_picker_selected_index,
            Some(0),
            "a non-empty query should select its first visible match"
        );
        assert!(matches!(
            repo_picker::filtered_layout(host, "repo-picker").0.first(),
            Some(repo_picker::RepoPickerEntry::Open(id)) if *id == repo_id
        ));
    });

    // Navigation remains manual until the text changes again. Refining the
    // query then returns selection and scroll to its first result.
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    cx.update(|_window, app| {
        popover_host.update(app, |host, _cx| {
            assert_eq!(host.repo_picker_selected_index, Some(1));
            host.picker_prompt_scroll
                .set_offset(gpui::point(gpui::px(0.0), gpui::px(-100.0)));
        });
    });
    input.update(cx, |input, cx| input.set_text("repo-picker-auto", cx));
    cx.run_until_parked();
    cx.update(|_window, app| {
        let host = popover_host.read(app);
        assert_eq!(host.repo_picker_selected_index, Some(0));
        assert_eq!(
            host.picker_prompt_scroll.offset(),
            gpui::point(gpui::px(0.0), gpui::px(0.0)),
            "a query edit should bring the newly selected first row into view"
        );
    });

    input.update(cx, |input, cx| input.set_text("definitely-no-match", cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| popover_host.read(app).repo_picker_selected_index),
        None,
        "a query with no results should not leave an Enter target"
    );

    input.update(cx, |input, cx| input.set_text("", cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| popover_host.read(app).repo_picker_selected_index),
        None,
        "clearing the query should restore neutral selection"
    );

    input.update(cx, |input, cx| input.set_text("repo-picker-auto", cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        !cx.update(|_window, app| popover_host.read(app).is_open()),
        "Enter should activate the automatically selected first repository"
    );
}

/// Seeds a session where one repository is pinned and has already fallen off
/// the recents list, then checks the picker still lists it — under Pinned, and
/// nowhere else. Runs in a subprocess so the session-file override is set
/// before the session is first read.
#[test]
fn repo_picker_pins_outlive_the_recents_list_wrapper() {
    if std::env::var_os(SESSION_FILE_ENV).is_some() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");

    let pinned = dir.path().join("pinned-repo");
    std::fs::create_dir_all(&pinned).expect("create pinned repo dir");
    gitcomet_state::session::persist_pinned_repo_to_path(&pinned, &session_file)
        .expect("seed pinned repo");

    // Also a recent, so the Recently Closed section exists to compare against.
    let recent = dir.path().join("recent-repo");
    std::fs::create_dir_all(&recent).expect("create recent repo dir");
    gitcomet_state::session::persist_recent_repo_to_path(&recent, &session_file)
        .expect("seed recent repo");

    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = gitcomet_core::process::background_command(current_exe)
        .arg("repo_picker_pins_outlive_the_recents_list_subprocess")
        .arg("--nocapture")
        .env(SESSION_FILE_ENV, &session_file)
        .output()
        .expect("spawn subtest process");
    assert!(
        output.status.success(),
        "subtest failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[gpui::test]
fn repo_picker_pins_outlive_the_recents_list_subprocess(cx: &mut gpui::TestAppContext) {
    if std::env::var_os(SESSION_FILE_ENV).is_none() {
        return;
    }

    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, workdir) = create_tracking_store("repo-picker-pins");
    let open_workdir = gitcomet_core::path_utils::canonicalize_or_original(workdir);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    // The pin was never opened and never recorded as recent, so only the pin
    // list can be keeping it in the picker.
    assert_eq!(
        sectioned_row_names(&popover_host, cx),
        vec![
            ("Pinned".to_string(), "pinned-repo".to_string()),
            (
                "Open Repositories".to_string(),
                open_repo_name(&open_workdir)
            ),
            ("Recently Closed".to_string(), "recent-repo".to_string()),
        ]
    );

    // Pinning the open repository lifts it out of Open Repositories rather than
    // listing it twice. Take the entry from `entries()` rather than building
    // one: an open repository is an `Open(repo_id)` row, so a hand-made
    // `Closed(path)` would skip the repo-id → workdir lookup that pinning it
    // actually goes through.
    let open_entry = cx.update(|_window, app| {
        repo_picker::entries(popover_host.read(app))
            .into_iter()
            .find(|(entry, _)| matches!(entry, repo_picker::RepoPickerEntry::Open(_)))
            .expect("the tracked repository should have an Open row")
            .0
    });
    cx.update(|window, app| {
        popover_host.update(app, |host, cx| {
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(open_entry.clone()),
                1,
                gpui::point(gpui::px(120.0), gpui::px(120.0)),
                cx,
            );
            picker_row_menu::activate(
                host,
                ContextMenuAction::PinRepository {
                    path: open_workdir.clone(),
                },
                window,
                cx,
            );
        });
    });

    let rows = sectioned_row_names(&popover_host, cx);
    assert_eq!(
        rows.iter()
            .filter(|(_, name)| name == &open_repo_name(&open_workdir))
            .count(),
        1,
        "a pinned repository must appear exactly once: {rows:?}"
    );
    assert_eq!(
        rows.iter()
            .find(|(_, name)| name == &open_repo_name(&open_workdir))
            .map(|(section, _)| section.as_str()),
        Some("Pinned"),
        "the pinned repository belongs to Pinned, not to its home section"
    );
    assert!(
        !rows
            .iter()
            .any(|(section, _)| section == "Open Repositories"),
        "the only open repository is pinned now, so its home section is gone: {rows:?}"
    );
}

/// A collapsed section keeps its header — the only way back to its rows — and a
/// query overrides collapse so filtering always searches everything.
#[gpui::test]
fn repo_picker_collapsing_a_section_hides_its_rows_but_keeps_its_header(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, workdir) = create_tracking_store("repo-picker-collapse");
    let open_workdir = gitcomet_core::path_utils::canonicalize_or_original(workdir);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    let layout_for = |cx: &mut gpui::VisualTestContext, query: &str| {
        cx.update(|_window, app| repo_picker::filtered_layout(popover_host.read(app), query).1)
    };

    assert_eq!(layout_for(cx, "").item_indices.len(), 1);

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            repo_picker::toggle_section(host, &"Open Repositories".into(), cx);
        });
    });

    let collapsed = layout_for(cx, "");
    assert!(
        collapsed.item_indices.is_empty(),
        "the collapsed section's rows leave the navigable list"
    );
    assert_eq!(
        collapsed
            .headers
            .iter()
            .map(|(_, header)| (
                header.label.to_string(),
                header.collapsed,
                header.hidden_count
            ))
            .collect::<Vec<_>>(),
        vec![("Open Repositories".to_string(), true, 1)],
        "the header stays, and says how much it is hiding"
    );

    // A query searches every section, collapsed or not.
    let name = open_repo_name(&open_workdir);
    assert_eq!(
        layout_for(cx, &name).item_indices.len(),
        1,
        "typing must reach rows inside a collapsed section"
    );

    // The header is the only way back, so it has to be clickable — and drawn at
    // all, now that the section it belongs to has no rows left.
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let header = cx
        .debug_bounds("picker_prompt_section_Open Repositories")
        .expect("a collapsed section keeps its header");
    assert!(
        cx.debug_bounds("picker_prompt_item_0").is_none(),
        "the collapsed section's row should not be drawn"
    );
    crate::view::panels::tests::simulate_counted_click(cx, header.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert_eq!(
        layout_for(cx, "").item_indices.len(),
        1,
        "clicking the header should unfold the section again"
    );
    assert!(
        cx.update(|_window, app| {
            popover_host
                .read(app)
                .cached_collapsed_picker_sections
                .is_empty()
        }),
        "unfolding drops the section from the persisted collapse set"
    );

    // While a query suspends collapse the headers must not be toggleable: a
    // click would flip the persisted fold with nothing moving on screen, so the
    // user would discover it only after clearing the query.
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            let input = host
                .repo_picker_search_input
                .clone()
                .expect("the picker owns a search input");
            input.update(cx, |input, cx| input.set_text(name.clone(), cx));
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        cx.debug_bounds("picker_prompt_section_Open Repositories")
            .is_none(),
        "a suspended section header should render as a plain label, not a toggle"
    );
}

/// Pins are stored oldest-first while `recency` counts the other way, so the
/// Pinned section has to invert the pin index or it sorts against the two
/// sections below it.
#[gpui::test]
fn repo_picker_pinned_section_sorts_newest_pin_first(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-pin-order");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    // Pinned oldest-first, the order `persist_pinned_repo` appends in.
    cx.update(|_window, app| {
        popover_host.update(app, |host, _cx| {
            host.cached_pinned_repos = vec![
                std::path::PathBuf::from("/tmp/first-pinned"),
                std::path::PathBuf::from("/tmp/second-pinned"),
            ];
        });
    });

    let pinned_names = |cx: &mut gpui::VisualTestContext| {
        sectioned_row_names(&popover_host, cx)
            .into_iter()
            .filter(|(section, _)| section == "Pinned")
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
    };

    assert_eq!(
        pinned_names(cx),
        vec!["second-pinned", "first-pinned"],
        "Newest must lead with the most recent pin, like the sections below it"
    );

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            repo_picker::apply_sort(host, repo_picker::RepoPickerSort::Oldest, cx);
        });
    });
    assert_eq!(pinned_names(cx), vec!["first-pinned", "second-pinned"]);
}

/// Right-clicking a row layers its menu over the picker: the arrow keys walk the
/// menu's actions, and Escape backs out of the menu before it closes the picker.
#[gpui::test]
fn repo_picker_row_menu_takes_over_navigation_and_escape(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-row-menu");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    let entry = cx.update(|_window, app| {
        repo_picker::entries(popover_host.read(app))
            .into_iter()
            .next()
            .expect("expected the open repository to have a row")
            .0
    });
    assert_eq!(
        entry,
        repo_picker::RepoPickerEntry::Open(gitcomet_state::model::RepoId(1))
    );

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            let row_actions = repo_picker::nav_targets(host, "", cx).len();
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry.clone()),
                0,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
            assert!(
                repo_picker::nav_targets(host, "", cx).len() > row_actions,
                "the arrow keys should walk the menu's actions while it is open"
            );

            // Escape backs out of the menu before it closes the picker.
            repo_picker::dismiss(host, cx);
            assert!(host.picker_row_menu.is_none());
            assert!(host.is_open(), "expected the picker to stay open");

            repo_picker::dismiss(host, cx);
            assert!(!host.is_open(), "expected Escape to close the picker");
        });
    });

    cx.update(|_window, app| {
        let host = popover_host.read(app);
        let labels = |entry| row_menu_labels(host, entry);

        // The active repository cannot be activated again, so that entry is
        // present but disabled — a row menu never offers a no-op. No editor is
        // configured in tests, so that entry is absent rather than dead.
        assert_eq!(
            as_str_pairs(&labels(&entry)),
            vec![
                ("Pin repository", false),
                ("Activate", true),
                ("Open repository location", false),
                ("Copy absolute path", false),
                ("Close repository", false),
            ]
        );

        // A closed row opens instead of activating, and forgets instead of
        // closing.
        let closed = repo_picker::RepoPickerEntry::Closed("/tmp/not-open".into());
        assert_eq!(
            as_str_pairs(&labels(&closed)),
            vec![
                ("Pin repository", false),
                ("Open repository", false),
                ("Open repository location", false),
                ("Copy absolute path", false),
                ("Remove from recently closed", false),
            ]
        );
    });
}

/// The row menu floats above the picker inside the same popover layer, so it
/// has to win the hit test against the rows underneath it, and its own scrim
/// has to swallow the dismissing click before `popover_scrim` reads it as
/// "close the picker".
#[gpui::test]
fn repo_picker_row_menu_floats_above_the_picker_and_dismisses_on_its_own(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-row-menu-layer");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    let row = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected a repository row");
    let anchor = row.center();
    cx.simulate_mouse_move(anchor, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: anchor,
        modifiers: gpui::Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let menu = cx
        .debug_bounds("picker_row_menu")
        .expect("right-clicking a row should open its menu");
    // Within a pixel: the anchored element lands on a device-pixel boundary.
    assert!(
        (menu.origin.x - anchor.x).abs() <= gpui::px(1.0)
            && (menu.origin.y - anchor.y).abs() <= gpui::px(1.0),
        "the menu should be anchored at the pointer, got {:?} for {anchor:?}",
        menu.origin
    );
    assert!(
        cx.update(|_window, app| popover_host.read(app).is_open()),
        "the picker stays open underneath its row menu"
    );

    // A press outside the menu dismisses it — and only it.
    let outside = gpui::point(
        menu.origin.x - gpui::px(20.0),
        menu.origin.y - gpui::px(20.0),
    );
    cx.simulate_mouse_move(outside, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: outside,
        modifiers: gpui::Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(cx.debug_bounds("picker_row_menu").is_none());
    assert!(
        cx.update(|_window, app| popover_host.read(app).is_open()),
        "dismissing the row menu must not also close the picker"
    );
}

/// Typing re-filters the rows the menu is floating over, so the row its stored
/// index highlights is no longer the row it was opened on — and it would go on
/// owning the arrow keys over a list that had moved underneath it. Any edit to
/// the filter therefore dismisses it. A search edit then starts selection at
/// the first filtered result instead of preserving the menu's invoking row.
#[gpui::test]
fn repo_picker_row_menu_closes_when_the_filter_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-menu-filter");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    // Two closed rows below the open one, so the filter has something to drop.
    let first_match = std::path::PathBuf::from("/tmp/zebra-a");
    let kept = std::path::PathBuf::from("/tmp/zebra-z");
    cx.update(|_window, app| {
        popover_host.update(app, |host, _cx| {
            host.cached_recent_repos = vec![first_match.clone(), kept.clone()];
        });
    });

    let entry = repo_picker::RepoPickerEntry::Closed(kept.clone());
    let display_index = cx.update(|_window, app| {
        repo_picker::filtered_layout(popover_host.read(app), "")
            .0
            .iter()
            .position(|candidate| *candidate == entry)
            .expect("the seeded recent should have a row")
    });

    cx.update(|window, app| {
        popover_host.update(app, |host, cx| {
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry.clone()),
                display_index,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
            let _ = window;
        });
    });

    // An arrow key reaches the picker through the same input the filter uses, so
    // the menu has to survive one: it is the query changing that dismisses it,
    // not any notification at all.
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| popover_host.read(app).picker_row_menu.is_some()),
        "arrowing inside the menu must not dismiss it"
    );

    cx.simulate_keystrokes("z e b");
    cx.run_until_parked();

    cx.update(|_window, app| {
        let host = popover_host.read(app);
        assert!(
            host.picker_row_menu.is_none(),
            "editing the filter should dismiss the row menu"
        );
        let filtered = repo_picker::filtered_layout(host, "zeb").0;
        assert_eq!(
            filtered.len(),
            2,
            "both zebra rows should survive the filter"
        );
        assert_eq!(
            host.repo_picker_selected_index,
            Some(0),
            "editing the filter should restart selection at the first result"
        );
        assert_ne!(
            host.repo_picker_selected_index,
            filtered.iter().position(|candidate| *candidate == entry),
            "the menu's invoking row should not override the first-result selection"
        );
    });
}

/// The rows can move while the menu floats over them — a background close, or
/// another surface closing a repository. Escape therefore has to look its row up
/// again instead of restoring the index it stored when the menu opened.
#[gpui::test]
fn repo_picker_row_menu_escape_reanchors_to_its_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-menu-reanchor");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    let dropped = std::path::PathBuf::from("/tmp/dropped-repo");
    let last = std::path::PathBuf::from("/tmp/last-repo");
    cx.update(|_window, app| {
        popover_host.update(app, |host, _cx| {
            host.cached_recent_repos = vec![dropped.clone(), last.clone()];
        });
    });

    let entry = repo_picker::RepoPickerEntry::Closed(last.clone());
    let stale_index = cx.update(|_window, app| {
        repo_picker::filtered_layout(popover_host.read(app), "")
            .0
            .iter()
            .position(|candidate| *candidate == entry)
            .expect("the seeded recent should have a row")
    });

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry.clone()),
                stale_index,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
            // A row above the menu's own disappears, so its index now names a
            // different repository than the one the menu was opened on.
            host.cached_recent_repos = vec![last.clone()];
            repo_picker::dismiss(host, cx);

            let moved_to = repo_picker::filtered_layout(host, "")
                .0
                .iter()
                .position(|candidate| *candidate == entry);
            assert_ne!(moved_to, Some(stale_index), "the row should have moved up");
            assert_eq!(
                host.repo_picker_selected_index, moved_to,
                "Escape should land on the row the menu belonged to, wherever it is now"
            );

            // And when the row is gone altogether there is nothing to restore.
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry.clone()),
                0,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
            host.cached_recent_repos = Vec::new();
            repo_picker::dismiss(host, cx);
            assert_eq!(host.repo_picker_selected_index, None);
        });
    });
}

/// A pin is what keeps a closed repository listed at all, so forgetting one
/// would drop it out of the picker with nothing left to bring it back. The menu
/// offers no such entry — and `forget` refuses it in its own right, so a future
/// caller cannot reintroduce the hole.
#[gpui::test]
fn repo_picker_forget_refuses_a_pinned_repository(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-forget-pinned");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    let pinned = std::path::PathBuf::from("/tmp/pinned-and-closed");
    let entry = repo_picker::RepoPickerEntry::Closed(pinned.clone());
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            host.cached_pinned_repos = vec![pinned.clone()];
            host.cached_recent_repos = vec![pinned.clone()];

            assert!(
                !row_menu_labels(host, &entry)
                    .iter()
                    .any(|(label, _)| *label == "Remove from recently closed"),
                "a pinned row must not offer the forget action"
            );

            repo_picker::forget(host, &entry, cx);
            assert_eq!(
                host.cached_recent_repos,
                vec![pinned.clone()],
                "forgetting a pinned repository must be a no-op"
            );
        });
    });
}

/// Closing from the row menu keeps the picker up, so the list underneath has to
/// show the row reach Recently Closed straight away. The session file is the
/// reducer's job; this only keeps the picker's own snapshot in step with it.
#[gpui::test]
fn repo_picker_close_row_action_promotes_the_recents_cache(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, workdir) = create_tracking_store("repo-picker-close-row");
    let workdir = gitcomet_core::path_utils::canonicalize_or_original(workdir);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    let older = std::path::PathBuf::from("/tmp/older-repo");
    let entry = repo_picker::RepoPickerEntry::Open(gitcomet_state::model::RepoId(1));
    cx.update(|window, app| {
        popover_host.update(app, |host, cx| {
            // The repository is already on the list from when it was opened, so
            // closing it has to move that entry rather than add a second one.
            host.cached_recent_repos = vec![older.clone(), workdir.clone()];
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry.clone()),
                0,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
            picker_row_menu::activate(
                host,
                ContextMenuAction::CloseRepo {
                    repo_id: gitcomet_state::model::RepoId(1),
                },
                window,
                cx,
            );

            assert_eq!(
                host.cached_recent_repos,
                vec![workdir.clone(), older.clone()],
                "the closed repository should lead the list, exactly once"
            );
            assert!(host.picker_row_menu.is_none());
            assert_eq!(host.repo_picker_selected_index, None);
            assert!(
                host.is_open(),
                "closing keeps the picker up for the next one"
            );
        });
    });
}

/// The row a menu was opened on can leave the store while the menu is up — a
/// concurrent close from a repo tab, say. The menu is already down by the time
/// the action runs, so a quiet bail would land the click as nothing at all.
#[gpui::test]
fn repo_picker_row_action_says_so_when_the_repository_is_gone(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-row-gone");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());
    // Stands in for a repository that left the store while its menu was up — a
    // concurrent close from a repo tab — taking the workdir every path-shaped
    // action needs with it.
    let entry = repo_picker::RepoPickerEntry::Open(gitcomet_state::model::RepoId(999));

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            assert!(
                host.workdir_for_repo(gitcomet_state::model::RepoId(999))
                    .is_none()
            );
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry.clone()),
                0,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        let host = popover_host.read(app);
        let labels = row_menu_labels(host, &entry);
        assert!(
            labels
                .iter()
                .all(|(label, _)| label != "Pin repository" && label != "Copy absolute path"),
            "a row with no path must not offer the entries that need one, got {labels:?}"
        );
    });

    cx.update(|window, app| {
        popover_host.update(app, |host, cx| {
            picker_row_menu::activate(
                host,
                ContextMenuAction::CloseRepo {
                    repo_id: gitcomet_state::model::RepoId(999),
                },
                window,
                cx,
            );
            assert!(
                host.cached_pinned_repos.is_empty(),
                "nothing should have been pinned"
            );
        });
    });
    cx.run_until_parked();

    let toasts = cx.update(|_window, app| {
        view.read(app)
            .toast_host
            .read(app)
            .toasts_for_tests(app)
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>()
    });
    assert_eq!(
        toasts,
        vec!["That repository is no longer open.".to_string()],
        "the click has to land as something the user can see"
    );
}

/// The row menu is its own floating layer, so it gets none of the max-height
/// treatment `popover_view` gives the context menus that go through it. Without
/// a cap of its own it runs off a short window, taking the destructive entries
/// at the bottom with it and leaving no way to scroll to them.
#[gpui::test]
fn repo_picker_row_menu_stays_inside_a_short_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-menu-height");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    // Short enough that the menu does not fit on either side of its anchor.
    cx.simulate_resize(gpui::size(gpui::px(720.0), gpui::px(150.0)));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let window_h = cx.update(|window, _app| window.window_bounds().get_bounds().size.height);

    let entry = repo_picker::RepoPickerEntry::Open(gitcomet_state::model::RepoId(1));
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry),
                0,
                gpui::point(gpui::px(200.0), window_h / 2.0),
                cx,
            );
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("picker_row_menu_scroll").is_some(),
        "the menu needs a scroll container to reach what the cap cuts off"
    );
    let menu = cx
        .debug_bounds("picker_row_menu")
        .expect("the row menu should be on screen");
    assert!(
        menu.bottom() <= window_h,
        "the menu should be capped to the window, got {menu:?} in a window {window_h:?} tall"
    );
    assert!(
        menu.size.height > gpui::px(0.0),
        "the cap must not collapse the menu"
    );
}

/// Both halves of the row-menu layer occlude, which silences the root view's
/// mouse tracking. The scrim covers the whole window, so without its own
/// forwarding a truncated-path tooltip from the picker underneath stays painted
/// wherever the pointer was when the menu opened.
#[gpui::test]
fn repo_picker_row_menu_scrim_feeds_the_tooltip_host(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-menu-tooltip");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());
    let tooltip_host = cx.update(|_window, app| view.read(app).tooltip_host_for_test());

    let row = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected a repository row");
    let row_center = row.center();
    cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
    cx.run_until_parked();

    let entry = repo_picker::RepoPickerEntry::Open(gitcomet_state::model::RepoId(1));
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            picker_row_menu::open(
                host,
                picker_row_menu::PickerRowMenuTarget::Repo(entry),
                0,
                row_center,
                cx,
            );
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    // Over the scrim, clear of the menu itself.
    let menu = cx
        .debug_bounds("picker_row_menu")
        .expect("the row menu should be on screen");
    let over_scrim = gpui::point(
        menu.origin.x - gpui::px(40.0),
        menu.origin.y - gpui::px(40.0),
    );
    cx.simulate_mouse_move(over_scrim, None, gpui::Modifiers::default());
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_window, app| tooltip_host.read(app).anchor_for_test()),
        over_scrim,
        "the scrim has to hand the tooltip host the positions it swallows"
    );
}

/// Pins are uncapped, so this list is the one that can grow without bound. Past
/// a couple of viewports it renders only what is on screen — otherwise every
/// frame, including the ones a hover between rows causes, builds an element per
/// repository.
#[gpui::test]
fn a_long_repo_list_renders_only_the_rows_in_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-windowed");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            host.cached_pinned_repos = (0..200)
                .map(|ix| std::path::PathBuf::from(format!("/tmp/pinned-{ix:03}")))
                .collect();
            cx.notify();
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let matched = cx.update(|_window, app| {
        repo_picker::cached(popover_host.read(app), "")
            .layout
            .item_indices
            .len()
    });
    assert!(matched > 200, "expected a long list, matched {matched}");

    assert!(
        cx.debug_bounds("picker_prompt_item_0").is_some(),
        "the first row is in view"
    );
    assert!(
        cx.debug_bounds("picker_prompt_item_150").is_none(),
        "a row 150 places down must not be built until it is scrolled to"
    );
}

/// The windowed list stands spacers in for the rows it does not render, sized
/// from the geometry alone — and this list has section headers in among those
/// rows. If the arithmetic drifted from what rows really paint at, scrolling
/// would drift with it.
#[gpui::test]
fn repo_row_geometry_matches_the_height_rows_paint_at(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-geometry");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());

    // Two pinned rows above the open one, so a section boundary falls between
    // the rows this compares.
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            host.cached_pinned_repos = vec![
                std::path::PathBuf::from("/tmp/pinned-one"),
                std::path::PathBuf::from("/tmp/pinned-two"),
            ];
            cx.notify();
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let geometry = cx.update(|_window, app| {
        let rows = repo_picker::cached(popover_host.read(app), "");
        components::PickerPromptGeometry::new(&rows.items, &rows.layout, 100u32)
    });

    let first = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected the first row to render");
    let second = cx
        .debug_bounds("picker_prompt_item_1")
        .expect("expected the second row to render");
    let third = cx
        .debug_bounds("picker_prompt_item_2")
        .expect("expected the row below the section boundary to render");

    assert_eq!(
        first.size.height,
        geometry.row_height(0),
        "a painted row must be exactly as tall as the geometry says"
    );
    assert_eq!(
        second.origin.y - first.origin.y,
        geometry.row_top(1) - geometry.row_top(0),
        "the stride between rows must match the geometry"
    );
    assert_eq!(
        third.origin.y - second.origin.y,
        geometry.row_top(2) - geometry.row_top(1),
        "and the stride across a section header must include the header"
    );
}

/// Arrowing up from nothing selects the *last* row, which in a long list is far
/// outside the window. Scrolling to it has to go through the row geometry: the
/// window has not built an element for it, so there is nothing for
/// `ScrollHandle::scroll_to_item` to find and scroll to.
#[gpui::test]
fn arrowing_to_the_last_row_scrolls_it_into_a_windowed_repo_list(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-window-nav");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            host.cached_pinned_repos = (0..200)
                .map(|ix| std::path::PathBuf::from(format!("/tmp/pinned-{ix:03}")))
                .collect();
            cx.notify();
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    // 200 pins plus the one open repository, so the last row is 200 —
    // `debug_bounds` takes a `&'static str`, so it is named by literal.
    const LAST_ROW: &str = "picker_prompt_item_200";
    let last = cx.update(|_window, app| {
        repo_picker::cached(popover_host.read(app), "")
            .layout
            .item_indices
            .len()
            - 1
    });
    assert_eq!(last, 200, "the seeded list should end at row 200");
    assert!(
        cx.debug_bounds(LAST_ROW).is_none(),
        "the row this arrows to must start outside the window"
    );

    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert_eq!(
        cx.update(|_window, app| popover_host.read(app).repo_picker_selected_index),
        Some(last),
        "arrowing up from nothing selects the last row"
    );
    assert!(
        cx.debug_bounds(LAST_ROW).is_some(),
        "the selected row has to be scrolled into the window, not left unbuilt"
    );
}

/// A named change to one of the inputs the picker's rows are built from, and the
/// label the assertion reports it under.
type RowsInputBump = (
    &'static str,
    fn(&mut PopoverHost, &mut gpui::Context<PopoverHost>),
);

/// `PopoverHost` is an uncached overlay view, so a hover moving between rows
/// re-renders the whole picker. The rows only change when the data behind them
/// does, so those frames have to reuse them.
#[gpui::test]
fn repo_picker_rows_are_reused_until_their_data_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = create_tracking_store("repo-picker-rows-cache");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    open_repo_picker(&view, cx);
    let popover_host = cx.update(|_window, app| view.read(app).popover_host.clone());
    cx.update(|_window, app| {
        popover_host.update(app, |host, cx| {
            host.cached_pinned_repos = vec![std::path::PathBuf::from("/tmp/pinned-one")];
            host.cached_recent_repos = vec![std::path::PathBuf::from("/tmp/closed-one")];
            cx.notify();
        });
    });

    let rows = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| repo_picker::cached(popover_host.read(app), ""))
    };

    let first = rows(cx);
    assert!(
        std::rc::Rc::ptr_eq(&first, &rows(cx)),
        "an unchanged picker must hand back the very same rows"
    );

    // Every input the rows are built from has to be in the key. A missing one
    // shows a stale list with nothing on screen to say it is stale, so each is
    // bumped in turn and has to force a rebuild.
    let bumps: [RowsInputBump; 6] = [
        ("sort", |host, cx| {
            repo_picker::apply_sort(host, repo_picker::RepoPickerSort::Oldest, cx);
        }),
        ("a pin", |host, _cx| {
            host.cached_pinned_repos
                .push(std::path::PathBuf::from("/tmp/pinned-two"));
        }),
        ("a recent", |host, _cx| {
            host.cached_recent_repos
                .push(std::path::PathBuf::from("/tmp/closed-two"));
        }),
        ("a collapsed section", |host, cx| {
            repo_picker::toggle_section(host, &"Open Repositories".into(), cx);
        }),
        ("the active repository", |host, _cx| {
            let mut state = (*host.state).clone();
            state.active_repo = None;
            host.state = std::sync::Arc::new(state);
        }),
        ("a repository's last activation", |host, _cx| {
            let mut state = (*host.state).clone();
            for repo in &mut state.repos {
                repo.last_active_at = Some(std::time::SystemTime::UNIX_EPOCH);
            }
            host.state = std::sync::Arc::new(state);
        }),
    ];

    let mut previous = rows(cx);
    for (label, bump) in bumps {
        cx.update(|_window, app| {
            popover_host.update(app, bump);
        });
        let rebuilt = rows(cx);
        assert!(
            !std::rc::Rc::ptr_eq(&previous, &rebuilt),
            "changing {label} must rebuild the rows"
        );
        previous = rebuilt;
    }

    // The query is part of the key too, and it arrives separately from the host.
    let filtered = cx.update(|_window, app| repo_picker::cached(popover_host.read(app), "pinned"));
    assert!(
        !std::rc::Rc::ptr_eq(&previous, &filtered),
        "a different query must rebuild the rows"
    );
}

fn as_str_pairs(labels: &[(String, bool)]) -> Vec<(&str, bool)> {
    labels
        .iter()
        .map(|(label, disabled)| (label.as_str(), *disabled))
        .collect()
}

/// The `(label, disabled)` of every entry a row's menu offers, in menu order.
fn row_menu_labels(
    host: &PopoverHost,
    entry: &repo_picker::RepoPickerEntry,
) -> Vec<(String, bool)> {
    host.repo_picker_row_menu_model(entry)
        .items
        .into_iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry {
                label, disabled, ..
            } => Some((label.to_string(), disabled)),
            _ => None,
        })
        .collect()
}

fn open_repo_picker(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::RepoPicker,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
}

fn open_repo_name(workdir: &std::path::Path) -> String {
    workdir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Every picker row as `(section label, repository name)`, in render order.
fn sectioned_row_names(
    popover_host: &gpui::Entity<PopoverHost>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<(String, String)> {
    cx.update(|_window, app| {
        let host = popover_host.read(app);
        repo_picker::entries(host)
            .into_iter()
            .map(|(entry, item)| {
                let section = item
                    .section_label()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let workdir = match entry {
                    repo_picker::RepoPickerEntry::Open(repo_id) => host
                        .state
                        .repos
                        .iter()
                        .find(|repo| repo.id == repo_id)
                        .map(|repo| repo.spec.workdir.clone())
                        .unwrap_or_default(),
                    repo_picker::RepoPickerEntry::Closed(path) => path,
                };
                (section, open_repo_name(&workdir))
            })
            .collect()
    })
}

/// The popover occludes the root view's mouse tracking, so without its own
/// mouse-move forwarding the tooltip host keeps anchoring truncated-text
/// tooltips to wherever the pointer sat before the popover opened.
#[gpui::test]
fn popover_feeds_pointer_positions_to_the_tooltip_host(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("popover-tooltip-anchor");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::RepoPicker,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    // Anchor somewhere far from the picker rows, the way clicking the repo tab
    // above the popover leaves it.
    let tooltip_host = cx.update(|_window, app| view.read(app).tooltip_host_for_test());
    cx.update(|_window, app| {
        tooltip_host.update(app, |host, cx| {
            host.on_mouse_moved(gpui::point(gpui::px(4.0), gpui::px(4.0)), cx);
        });
    });

    let row_bounds = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected a repository row");
    let row_center = row_bounds.center();
    cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let anchor = cx.update(|_window, app| tooltip_host.read(app).anchor_for_test());
    assert_eq!(
        anchor, row_center,
        "expected the tooltip anchor to follow the pointer inside the popover"
    );
}

#[gpui::test]
fn rebase_onto_picker_excludes_current_branch_and_opens_confirm(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("rebase-onto-picker");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    // The store's own repo-load races the injected branches below and only
    // knows "main", and in gpui tests the view freezes the store snapshot it is
    // constructed with — so let the load settle first, and only then inject.
    wait_until("rebase-onto test repo to load", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.branches.ready().is_some() && repo.head_branch.ready().is_some()
            })
    });
    store.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![
                Branch {
                    name: "main".to_string(),
                    target: CommitId("HEAD".into()),
                    upstream: None,
                    divergence: None,
                },
                Branch {
                    name: "feature".to_string(),
                    target: CommitId("HEAD".into()),
                    upstream: None,
                    divergence: None,
                },
            ]),
        },
    ));
    // Dispatch is asynchronous; the view below must be constructed from a
    // snapshot that already contains the injected branches.
    wait_until("injected branches to reach the store", || {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.branches
                    .ready()
                    .is_some_and(|branches| branches.iter().any(|branch| branch.name == "feature"))
            })
    });
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::BranchPicker {
                        purpose: BranchPickerPurpose::RebaseOnto,
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                let search = host
                    .branch_picker_search_input
                    .as_ref()
                    .expect("branch picker search input");
                let focus = search.read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    // The current branch (main) must not be offered as a rebase target, so
    // the first picker item is "feature"; selecting it asks for confirmation
    // instead of checking it out.
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();

    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        match &host.popover {
            Some(PopoverKind::RebaseOntoConfirm { repo_id: rid, onto }) => {
                assert_eq!(*rid, repo_id);
                assert_eq!(onto, "feature");
            }
            other => panic!("expected RebaseOntoConfirm popover, got {other:?}"),
        }
    });
}
