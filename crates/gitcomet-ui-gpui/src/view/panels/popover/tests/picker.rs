use super::branch::create_tracking_store;
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
                repo_picker::nav_targets(host, "").len(),
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
            repo_picker::open_row_menu(
                host,
                open_entry.clone(),
                1,
                gpui::point(gpui::px(120.0), gpui::px(120.0)),
                cx,
            );
            repo_picker::activate_row_action(
                host,
                repo_picker::RepoPickerRowAction::Pin,
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
            let row_actions = repo_picker::nav_targets(host, "").len();
            repo_picker::open_row_menu(
                host,
                entry.clone(),
                0,
                gpui::point(gpui::px(140.0), gpui::px(120.0)),
                cx,
            );
            assert!(
                repo_picker::nav_targets(host, "").len() > row_actions,
                "the arrow keys should walk the menu's actions while it is open"
            );

            // Escape backs out of the menu before it closes the picker.
            repo_picker::dismiss(host, cx);
            assert!(host.repo_picker_row_menu.is_none());
            assert!(host.is_open(), "expected the picker to stay open");

            repo_picker::dismiss(host, cx);
            assert!(!host.is_open(), "expected Escape to close the picker");
        });
    });

    cx.update(|_window, app| {
        let host = popover_host.read(app);
        let labels = |entry| {
            repo_picker::row_menu_items(host, entry)
                .into_iter()
                .filter_map(|item| match item {
                    repo_picker::RepoPickerRowMenuItem::Entry {
                        label, disabled, ..
                    } => Some((label, disabled)),
                    repo_picker::RepoPickerRowMenuItem::Separator => None,
                })
                .collect::<Vec<_>>()
        };

        // The active repository cannot be activated again, so that entry is
        // present but disabled — a row menu never offers a no-op. No editor is
        // configured in tests, so that entry is absent rather than dead.
        assert_eq!(
            labels(&entry),
            vec![
                ("Pin repository", false),
                ("Activate", true),
                ("Open repository location", false),
                ("Copy path", false),
                ("Close repository", false),
            ]
        );

        // A closed row opens instead of activating, and forgets instead of
        // closing.
        let closed = repo_picker::RepoPickerEntry::Closed("/tmp/not-open".into());
        assert_eq!(
            labels(&closed),
            vec![
                ("Pin repository", false),
                ("Open repository", false),
                ("Open repository location", false),
                ("Copy path", false),
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
        .debug_bounds("repo_picker_row_menu")
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

    assert!(cx.debug_bounds("repo_picker_row_menu").is_none());
    assert!(
        cx.update(|_window, app| popover_host.read(app).is_open()),
        "dismissing the row menu must not also close the picker"
    );
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
