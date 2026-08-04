use super::branch::create_tracking_store;
use super::*;

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
            repo_picker::RepoPickerEntry::RecentlyClosed(path) => Some(path.clone()),
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
                    repo_picker::RepoPickerEntry::RecentlyClosed(path) => Some(
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
