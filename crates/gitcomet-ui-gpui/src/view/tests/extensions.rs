//! Extension hosting: the example product's extension against the real host.

use super::*;
use crate::view::extension_host;
use gitcomet_extension_api::{HostError, Registry};
use gitcomet_extension_example::review::{self, ReviewExtension};

const MARK_REVIEWED: &str = "com.example.review/mark-reviewed";
const SHOW_SUMMARY: &str = "com.example.review/show-summary";

fn install_example(app: &mut gpui::App) {
    let registry = Registry::build(vec![Box::new(ReviewExtension)]).expect("valid registration");
    extension_host::install(registry, app);
}

fn state_with_repo(repo_id: RepoId, workdir: &Path) -> Arc<AppState> {
    let mut state = AppState {
        active_repo: Some(repo_id),
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    };
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.to_path_buf(),
        },
    ));
    Arc::new(state)
}

fn empty_state() -> Arc<AppState> {
    Arc::new(AppState {
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    })
}

/// Named, so the empty window keeps it until its repository arrives.
fn named_workspace(name: &str) -> gitcomet_state::session::Workspace {
    let mut workspace = gitcomet_state::session::Workspace::new(Vec::new());
    workspace.custom_name = Some(name.into());
    workspace
}

fn open_window_with_repo(
    cx: &mut gpui::TestAppContext,
    workspace: gitcomet_state::session::Workspace,
    workdir: &Path,
) -> gpui::WindowHandle<GitCometView> {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let handle = cx.add_window(|window, cx| GitCometView::new(store, events, None, window, cx));
    handle
        .update(cx, |view, _window, cx| {
            view.adopt_workspace(workspace, cx);
            test_support::push_test_state(view, state_with_repo(RepoId(1), workdir), cx);
        })
        .expect("window is open");
    let view = handle.root(cx).expect("root view");
    cx.update(|app| extension_host::window_opened(&view, app));
    cx.run_until_parked();
    // Drawn through the untyped handle: a typed update leases the root view.
    cx.update_window(handle.into(), |_, window, app| {
        let _ = window.draw(app);
    })
    .expect("window is open");
    handle
}

/// A view config for each window kind the host opens.
fn config_for(mode: GitCometViewMode) -> GitCometViewConfig {
    match mode {
        GitCometViewMode::FocusedDiff => GitCometViewConfig {
            view_mode: mode,
            focused_diff: Some(crate::FocusedDiffConfig {
                label_left: "before".into(),
                label_right: "after".into(),
                display_path: Some("example.rs".into()),
                diff_text: "diff --git a/example.rs b/example.rs\n--- a/example.rs\n+++ b/example.rs\n@@ -1 +1 @@\n-old\n+new\n".into(),
            }),
            workspace: WorkspaceBootstrap::Empty,
            ..Default::default()
        },
        GitCometViewMode::FocusedMergetool => GitCometViewConfig {
            view_mode: mode,
            focused_mergetool: Some(FocusedMergetoolViewConfig {
                repo_path: PathBuf::from("/tmp/without-extensions-mergetool"),
                conflicted_file_path: PathBuf::from("conflicted.txt"),
                labels: FocusedMergetoolLabels {
                    local: "LOCAL".into(),
                    remote: "REMOTE".into(),
                    base: "BASE".into(),
                },
            }),
            ..Default::default()
        },
        GitCometViewMode::Normal => GitCometViewConfig::default(),
    }
}

#[gpui::test]
fn without_extensions_the_host_adds_nothing(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for mode in [
        GitCometViewMode::Normal,
        GitCometViewMode::FocusedDiff,
        GitCometViewMode::FocusedMergetool,
    ] {
        crate::view::perf::take_extension_dispatch_calls();
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_config(store, events, config_for(mode), window, cx)
        });
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                test_support::push_test_state(
                    this,
                    state_with_repo(RepoId(1), Path::new("/tmp/x")),
                    cx,
                )
            });
            extension_host::window_opened(&view, app);
        });
        cx.run_until_parked();
        test_support::redraw(cx);
        assert!(
            cx.debug_bounds("repository_view_strip").is_none(),
            "{mode:?}"
        );
        cx.update(|_window, app| {
            let view = view.read(app);
            assert_eq!(
                crate::view::perf::take_extension_dispatch_calls(),
                0,
                "{mode:?}"
            );
            assert!(extension_host::registry(app).is_none());
            // Only the focused difftool has a host: it builds its own pane.
            assert_eq!(
                view.extension_window.is_some(),
                mode == GitCometViewMode::FocusedDiff,
                "{mode:?}"
            );
            assert!(view.repository_views.is_none());
            assert!(view.details_tabs.is_none());
            assert!(view.sidebar_sections.is_none());
            assert!(view.window_gates.is_none());
            assert!(extension_host::palette_entries(app).is_empty());
            assert_eq!(view.bottom_status_bar.read(app).extension_item_count(), 0);
        });
    }
}

/// The example's chrome and rows come from the registry: its edition strip
/// replaces the default product row, its brand sits in the title bar, its
/// sidebar rows render, and its window gate covers the window until lifted.
#[gpui::test]
fn the_examples_chrome_rows_and_gate_reach_a_main_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let workdir = tempfile::tempdir().unwrap();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, state_with_repo(RepoId(1), workdir.path()), cx)
        });
        extension_host::window_opened(&view, app);
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_edition_strip").is_some());
    assert!(cx.debug_bounds("bottom_status_bar_brand").is_none());
    assert!(cx.debug_bounds("bottom_status_bar_version").is_none());
    assert!(cx.debug_bounds("example_title_brand").is_some());
    assert!(cx.debug_bounds("sidebar_contribution_0_0").is_some());
    assert!(cx.debug_bounds("example_gate").is_none());

    cx.update(|_window, app| review::set_gated(true, app));
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_gate").is_some());
    cx.update(|_window, app| assert!(view.read(app).window_gated));

    cx.update(|_window, app| review::set_gated(false, app));
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_gate").is_none());
}

/// A host opens URLs and files once the calling update has ended, off the
/// UI thread (tests record the launch instead of starting a browser), and
/// refuses what must never reach the OS opener.
#[gpui::test]
fn hosts_open_urls_and_paths_after_the_update(cx: &mut gpui::TestAppContext) {
    use crate::view::platform_open::{Launch, take_recorded_launches};
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.run_until_parked();
    take_recorded_launches();
    let host = cx.update(|_, app| view.read(app).extension_window.as_ref().unwrap().host());
    cx.update(|_, app| {
        host.open_url("https://example.com/docs", app).unwrap();
        host.open_path(Path::new("/tmp/report.txt"), app).unwrap();
        assert!(
            take_recorded_launches().is_empty(),
            "nothing launches inside the calling update"
        );
        assert!(matches!(
            host.open_url("javascript:alert(1)", app),
            Err(HostError::InvalidRequest(_))
        ));
        assert!(matches!(
            host.open_url("file:///etc/passwd", app),
            Err(HostError::InvalidRequest(_))
        ));
        assert!(matches!(
            host.open_path(Path::new(""), app),
            Err(HostError::InvalidRequest(_))
        ));
    });
    cx.run_until_parked();
    assert_eq!(
        take_recorded_launches(),
        vec![
            Launch::Url("https://example.com/docs".into()),
            Launch::Path("/tmp/report.txt".into()),
        ]
    );
}

/// The product's own links go through the same deferred opener.
#[gpui::test]
fn status_bar_links_open_after_the_click(cx: &mut gpui::TestAppContext) {
    use crate::view::platform_open::{Launch, take_recorded_launches};
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, state_with_repo(RepoId(1), Path::new("/tmp/x")), cx)
        });
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    take_recorded_launches();
    click_debug_selector(cx, "bottom_status_bar_version");
    cx.run_until_parked();
    let launches = take_recorded_launches();
    assert!(
        matches!(launches.as_slice(), [Launch::Url(url)] if url.starts_with("https://")),
        "{launches:?}"
    );
}

#[gpui::test]
fn contributions_run_per_window_persist_and_forget_closed_windows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let first_workspace = named_workspace("First");
    let second_workspace = named_workspace("Second");
    cx.update(|app| {
        crate::workspaces::initialize_for_test(
            app,
            vec![first_workspace.clone(), second_workspace.clone()],
        );
        install_example(app);
    });
    let first_repo = PathBuf::from("/tmp/extension-review-first");
    let second_repo = PathBuf::from("/tmp/extension-review-second");
    let first = open_window_with_repo(cx, first_workspace.clone(), &first_repo);
    let second = open_window_with_repo(cx, second_workspace.clone(), &second_repo);

    for handle in [first, second] {
        handle
            .update(cx, |view, _, cx| {
                assert_eq!(
                    view.bottom_status_bar.read(cx).extension_item_count(),
                    1,
                    "each window builds its own status item"
                );
            })
            .unwrap();
    }

    let first_host = first
        .update(cx, |view, _, _| {
            view.extension_window.as_ref().unwrap().host()
        })
        .unwrap();
    first
        .update(cx, |view, _, cx| {
            assert!(view.run_extension_command(MARK_REVIEWED, cx));
        })
        .unwrap();
    cx.run_until_parked();

    let first_id = first_host.id();
    let second_id = second.update(cx, |_, window, _| window.window_handle().window_id());
    let second_id = second_id.unwrap();
    cx.update(|app| {
        let reviews = review::reviews(app);
        let reviews = reviews.read(app);
        assert_eq!(reviews.count(first_id, &first_repo), 1);
        assert_eq!(reviews.count(second_id, &second_repo), 0);
        assert_eq!(reviews.windows(), 2);
        let saved = crate::workspaces::workspace(app, first_workspace.id)
            .and_then(|workspace| workspace.extensions.get(review::EXTENSION_ID).cloned());
        assert_eq!(
            saved,
            Some(serde_json::json!({ "reviews": { first_repo.to_str().unwrap(): 1 } })),
            "the count is saved in the window's workspace"
        );
    });
    first
        .update(cx, |view, _, cx| {
            let toasts = view.toast_host.read(cx).toasts_for_tests(cx);
            assert!(
                toasts
                    .iter()
                    .any(|(_, text)| text == "Marked extension-review-first reviewed"),
                "{toasts:?}"
            );
        })
        .unwrap();

    first
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update(|app| {
        assert_eq!(review::reviews(app).read(app).windows(), 1);
        assert!(!first_host.is_open(app));
        assert_eq!(
            first_host.active_repository(app),
            Err(HostError::WindowClosed)
        );
    });
}

#[gpui::test]
fn saved_workspace_state_is_restored_when_a_window_opens(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let repo = PathBuf::from("/tmp/extension-review-restored");
    let mut workspace = named_workspace("Restored");
    workspace
        .extensions
        .set(
            review::EXTENSION_ID,
            Some(serde_json::json!({ "reviews": { repo.to_str().unwrap(): 3 } })),
        )
        .unwrap();
    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, vec![workspace.clone()]);
        install_example(app);
    });
    let window = open_window_with_repo(cx, workspace, &repo);
    let window_id = window
        .update(cx, |_, window, _| window.window_handle().window_id())
        .unwrap();
    cx.update(|app| assert_eq!(review::reviews(app).read(app).count(window_id, &repo), 3));
}

/// Both strips show an icon on every tab: the built-in one's own and each
/// contribution's.
#[gpui::test]
fn view_and_details_tabs_show_their_icons(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(
                this,
                state_with_repo(RepoId(1), Path::new("/tmp/extension-tab-icons")),
                cx,
            )
        });
    });
    test_support::redraw(cx);
    for selector in [
        "repository_view_history_icon",
        "repository_view_0_icon",
        "repository_view_1_icon",
        "details_tab_details_icon",
        "details_tab_0_icon",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector}");
    }
}

/// An extension's menu opens as a menu in the main window, with its own
/// icons (`extensions/<id>/…`), and runs its entries.
#[gpui::test]
fn extension_menus_draw_the_extensions_icons(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(
                this,
                state_with_repo(RepoId(1), Path::new("/tmp/extension-menu-icons")),
                cx,
            )
        });
    });
    test_support::redraw(cx);
    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    click_debug_selector(cx, "example_review_more");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("context_menu_entry_icon_Mark reviewed")
            .is_some()
    );
    // The entry runs its action.
    click_debug_selector(cx, "context_menu_mark_reviewed");
    cx.run_until_parked();
    let window_id = cx.update(|window, _| window.window_handle().window_id());
    cx.update(|_, app| {
        let workdir = Path::new("/tmp/extension-menu-icons");
        assert_eq!(review::reviews(app).read(app).count(window_id, workdir), 1);
    });
}

#[gpui::test]
fn repository_views_switch_with_history_and_are_kept(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(
                this,
                state_with_repo(RepoId(1), Path::new("/tmp/extension-review-router")),
                cx,
            )
        });
    });
    test_support::redraw(cx);
    assert!(cx.debug_bounds("repository_view_strip").is_some());
    assert!(cx.debug_bounds("example_review_view").is_none());

    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("example_review_view").is_some(),
        "the extension view replaces History"
    );
    let built = cx.update(|_window, app| {
        let router = view.read(app).repository_views.as_ref().unwrap();
        let repo = view.read(app).active_repo().unwrap();
        router.active_view(repo).unwrap().entity_id()
    });

    click_debug_selector(cx, "repository_view_history");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_view").is_none());

    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    let again = cx.update(|_window, app| {
        let router = view.read(app).repository_views.as_ref().unwrap();
        let repo = view.read(app).active_repo().unwrap();
        router.active_view(repo).unwrap().entity_id()
    });
    assert_eq!(built, again, "the view is built once and kept");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, empty_state(), cx)
        });
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        let router = view.read(app).repository_views.as_ref().unwrap();
        let repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/extension-review-router"),
            },
        );
        assert!(
            router.active_view(&repo).is_none(),
            "a closed repository's view is dropped"
        );
    });
}

#[gpui::test]
fn commands_reach_the_palette_key_bindings_and_menus(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo = PathBuf::from("/tmp/extension-review-commands");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, state_with_repo(RepoId(1), &repo), cx)
        });
        window.activate();
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        let ids: Vec<&str> = extension_host::palette_entries(app)
            .iter()
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "extension:com.example.review/mark-reviewed",
                "extension:com.example.review/show-summary",
                "extension:com.example.review/toggle-review-log"
            ]
        );
        let app_menu =
            extension_host::menu_entries(gitcomet_extension_api::MenuLocation::Application, app);
        assert_eq!(app_menu.len(), 1);
        assert_eq!(app_menu[0].id.as_ref(), SHOW_SUMMARY);
    });

    // The palette row runs the command in this window.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command(
                "extension:com.example.review/mark-reviewed",
                Some(window),
                cx,
            )
        });
    });
    cx.run_until_parked();
    let window_id = cx.update(|window, _| window.window_handle().window_id());
    cx.update(|_window, app| assert_eq!(review::reviews(app).read(app).count(window_id, &repo), 1));

    // So does its key binding, with nothing focused in the window.
    cx.simulate_keystrokes("secondary-alt-r");
    cx.run_until_parked();
    cx.update(|_window, app| assert_eq!(review::reviews(app).read(app).count(window_id, &repo), 2));

    // A stale explicit target must never turn into the active repository.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(this.run_extension_command_for(MARK_REVIEWED, Some(RepoId(999)), cx));
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| assert_eq!(review::reviews(app).read(app).count(window_id, &repo), 2));
}

#[gpui::test]
fn retained_panes_and_bottom_panels_do_not_keep_a_closed_windows_store_alive(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{ChangeSource, DiffPaneOptions};
    let _visual_guard = crate::test_support::lock_visual_test();
    let workspace = named_workspace("Store lifetime");
    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, vec![workspace.clone()]);
        install_example(app);
    });
    let window = open_window_with_repo(cx, workspace, Path::new("/tmp/host-store-lifetime"));
    let (host, weak_store, panels, key) = window
        .update(cx, |view, _, _| {
            let extensions = view.extension_window.as_ref().unwrap();
            (
                extensions.host(),
                Arc::downgrade(&view.store),
                extensions.bottom_panels(),
                crate::view::extension_panels::repo_key(view.active_repo().unwrap()),
            )
        })
        .unwrap();
    let (pane, list) = cx.update(|app| {
        let repository = host.active_repository(app).unwrap().unwrap();
        let pane = host
            .create_diff_pane(
                &repository,
                DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
                DiffPaneOptions::default(),
                app,
            )
            .unwrap();
        let list = host
            .create_file_list(
                &repository,
                ChangeSource::Commit(CommitId("HEAD".into())),
                |_, _, _| {},
                app,
            )
            .unwrap();
        host.open_bottom_panel(
            &repository,
            &review::extension_id()
                .contribution(review::REVIEW_LOG_PANEL)
                .unwrap(),
            app,
        )
        .unwrap();
        (pane, list)
    });
    cx.run_until_parked();
    assert!(!panels.borrow().shown(key).is_empty());
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(
        weak_store.upgrade().is_none(),
        "retained panes must own only a weak store"
    );
    assert!(
        panels.borrow().shown(key).is_empty(),
        "closing breaks the panel/host cycle"
    );
    cx.update(|app| {
        assert!(!host.is_open(app));
        pane.set_target(
            DiffTarget::working_tree("b.rs".into(), DiffArea::Unstaged),
            app,
        );
        list.set_source(ChangeSource::Commit(CommitId("other".into())), app);
    });
}

#[gpui::test]
fn different_close_requests_replace_pending_and_visible_confirmations(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = (*state_with_repo(RepoId(1), Path::new("/tmp/busy"))).clone();
    state.repos[0].push_in_flight = 1;
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, Arc::new(state), cx);
        });
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            assert!(this.request_late_close_guards(
                &TerminalShutdownAction::CloseRepo { repo_id: RepoId(1) },
                cx
            ));
            assert!(this.request_late_close_guards(&TerminalShutdownAction::CloseWindow, cx));
            assert_eq!(
                this.pending_close_guard_prompt.as_ref().unwrap().action,
                TerminalShutdownAction::CloseWindow
            );
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            assert_eq!(
                this.popover_host.read(cx).close_guard_action(),
                Some(&TerminalShutdownAction::CloseWindow)
            );
            assert!(this.request_late_close_guards(&TerminalShutdownAction::QuitApp, cx));
            assert_eq!(
                this.pending_close_guard_prompt.as_ref().unwrap().action,
                TerminalShutdownAction::QuitApp
            );
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            assert_eq!(
                this.popover_host.read(cx).close_guard_action(),
                Some(&TerminalShutdownAction::QuitApp)
            );
            assert!(this.request_late_close_guards(&TerminalShutdownAction::QuitApp, cx));
            assert!(
                this.pending_close_guard_prompt.is_none(),
                "the identical action is deduplicated"
            );
        })
    });
}

#[gpui::test]
fn extension_bottom_panels_share_the_strip_with_the_reflog(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(
                this,
                state_with_repo(repo_id, Path::new("/tmp/extension-bottom-panels")),
                cx,
            )
        });
    });
    test_support::redraw(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let repository = cx.update(|_window, app| host.active_repository(app).unwrap().unwrap());
    let panel = review::extension_id()
        .contribution(review::REVIEW_LOG_PANEL)
        .unwrap();
    let is_open = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| host.is_bottom_panel_open(&repository, &panel, app))
    };

    // Opened through its command; alone, it needs no strip.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command(
                "extension:com.example.review/toggle-review-log",
                Some(window),
                cx,
            )
        });
    });
    assert!(is_open(cx), "the open is recorded at once");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_log").is_some());
    assert!(cx.debug_bounds("bottom_panel_tab_extension_0").is_none());

    // With the reflog open both get tabs; the reflog opened last is in front.
    cx.update(|_window, app| view.update(app, |this, cx| this.open_reflog_panel(repo_id, cx)));
    test_support::redraw(cx);
    assert!(cx.debug_bounds("bottom_panel_tab_reflog").is_some());
    assert!(cx.debug_bounds("example_review_log").is_none());
    click_debug_selector(cx, "bottom_panel_tab_extension_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_log").is_some());

    // Its tab's close button closes it; the reflog shows alone again.
    click_debug_selector(cx, "bottom_panel_tab_extension_0_close");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(!is_open(cx));
    assert!(cx.debug_bounds("example_review_log").is_none());
    assert!(cx.debug_bounds("bottom_panel_tab_reflog").is_none());

    // Closing the repository forgets its panels.
    cx.update(|_window, app| {
        host.open_bottom_panel(&repository, &panel, app).unwrap();
    });
    cx.run_until_parked();
    assert!(is_open(cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, empty_state(), cx)
        });
    });
    test_support::redraw(cx);
    assert!(!is_open(cx));
    let unknown = review::extension_id().contribution("nope").unwrap();
    cx.update(|_window, app| {
        assert_eq!(
            host.open_bottom_panel(&repository, &unknown, app),
            Err(HostError::RepositoryClosed)
        );
    });
}

#[gpui::test]
fn details_tabs_and_sidebar_sections_sit_beside_the_built_ins(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(
                this,
                state_with_repo(RepoId(1), Path::new("/tmp/extension-details-sidebar")),
                cx,
            )
        });
    });
    test_support::redraw(cx);

    // The sidebar section is built for the repository and collapses.
    assert!(cx.debug_bounds("sidebar_pane").is_some());
    assert!(cx.debug_bounds("example_review_sidebar").is_some());
    click_debug_selector(cx, "sidebar_extension_section_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_sidebar").is_none());
    click_debug_selector(cx, "sidebar_extension_section_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_sidebar").is_some());

    // The details tab replaces the details pane's content until Details.
    assert!(cx.debug_bounds("details_tab_strip").is_some());
    assert!(cx.debug_bounds("example_review_details").is_none());
    click_debug_selector(cx, "details_tab_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_details").is_some());
    click_debug_selector(cx, "details_tab_details");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_details").is_none());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, empty_state(), cx)
        });
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        let this = view.read(app);
        let repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/extension-details-sidebar"),
            },
        );
        let details = this.details_tabs.as_ref().unwrap();
        assert!(
            details.built(&repo, 0).is_none(),
            "a closed repository's tab is dropped"
        );
    });
}

#[gpui::test]
fn hosted_dialogs_open_in_the_popover_host_and_close(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(this.run_extension_command(SHOW_SUMMARY, cx))
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("extension_dialog_content").is_some());
    assert!(cx.update(|_window, app| test_support::popover_is_open(view.read(app), app)));

    click_debug_selector(cx, "extension_dialog_close");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("extension_dialog_content").is_none());
    assert!(!cx.update(|_window, app| test_support::popover_is_open(view.read(app), app)));
}

#[gpui::test]
fn entry_gates_refuse_marked_repositories_with_a_notice(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let marked = tempfile::tempdir().unwrap();
    std::fs::write(marked.path().join(review::DENY_MARKER), "").unwrap();
    let plain = tempfile::tempdir().unwrap();
    cx.update(|app| {
        use gitcomet_extension_api::{EntryOrigin, GateDecision};
        assert!(matches!(
            extension_host::entry_decision(marked.path(), EntryOrigin::Chooser, app),
            GateDecision::Deny { .. }
        ));
        assert_eq!(
            extension_host::entry_decision(plain.path(), EntryOrigin::Chooser, app),
            GateDecision::Allow
        );
        let (allowed, denied) = extension_host::filter_entries(
            vec![marked.path().to_path_buf(), plain.path().to_path_buf()],
            EntryOrigin::CommandLine,
            app,
        );
        assert_eq!(allowed, vec![plain.path().to_path_buf()]);
        assert_eq!(denied.len(), 1);
    });

    // A window asked to open the marked repository opens nothing and says why.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let marked_path = marked.path().to_path_buf();
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(store, events, Some(marked_path), window, cx)
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(50));
    assert!(store_for_assert.snapshot().repos.is_empty());
    cx.update(|_window, app| {
        let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
        assert!(
            toasts
                .iter()
                .any(|(kind, text)| *kind == components::ToastKind::Warning
                    && text.contains(review::DENY_MARKER)),
            "{toasts:?}"
        );
    });
}

/// Opening a submodule's diff in its own tab is an entry like any other: a
/// gate that refuses the submodule stops it before a tab opens.
#[gpui::test]
fn opening_a_submodule_tab_passes_the_entry_gates(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let parent = tempfile::tempdir().unwrap();
    let submodule = parent.path().join("vendored");
    std::fs::create_dir_all(&submodule).unwrap();
    std::fs::write(submodule.join(review::DENY_MARKER), "").unwrap();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, state_with_repo(RepoId(1), parent.path()), cx);
        });
    });
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.open_submodule_inner_diff(
                submodule.clone(),
                DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
                cx,
            )
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let view = view.read(app);
        assert!(view.submodule_diff_bootstrap.is_none());
        let toasts = view.toast_host.read(app).toasts_for_tests(app);
        assert!(
            toasts
                .iter()
                .any(|(kind, text)| *kind == components::ToastKind::Warning
                    && text.contains(review::DENY_MARKER)),
            "{toasts:?}"
        );
    });
}

#[gpui::test]
fn close_guards_ask_once_after_the_host_guards(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo = PathBuf::from("/tmp/extension-review-close");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, state_with_repo(RepoId(1), &repo), cx)
        });
        review::reviews(app).update(app, |reviews, _| reviews.confirm_close_unreviewed = true);
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.request_close_repos(vec![RepoId(1)], None, cx);
            let prompt = this
                .pending_close_guard_prompt
                .clone()
                .expect("the extension asks before closing an unreviewed repository");
            assert_eq!(
                prompt.action,
                TerminalShutdownAction::CloseRepo { repo_id: RepoId(1) }
            );
            assert_eq!(prompt.reasons.len(), 1);
            assert!(prompt.reasons[0].contains("has not been reviewed"));
        });
    });
    test_support::redraw(cx);
    assert!(cx.debug_bounds("close_guard_reasons").is_some());

    // Asking again while the prompt is up keeps the one prompt.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.request_close_repos(vec![RepoId(1)], None, cx);
            assert!(this.pending_close_guard_prompt.is_none());
        });
    });
    test_support::redraw(cx);
    assert!(cx.debug_bounds("close_guard_reasons").is_some());

    // Confirming closes without asking again.
    click_debug_selector(cx, "close_guard_confirm");
    test_support::redraw(cx);
    cx.update(|_window, app| {
        assert!(view.read(app).pending_close_guard_prompt.is_none());
        assert!(!test_support::popover_is_open(view.read(app), app));
    });
}

#[gpui::test]
fn running_git_operations_ask_before_the_window_closes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = (*state_with_repo(RepoId(1), Path::new("/tmp/pushing-repo"))).clone();
    state.repos[0].push_in_flight = 1;
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, Arc::new(state), cx);
        });
    });
    test_support::redraw(cx);
    cx.update(|window, app| {
        let window_id = window.window_handle().window_id();
        view.update(app, |this, cx| {
            assert!(this.request_close_window_or_warn(window_id, cx));
            let prompt = this.pending_close_guard_prompt.clone().unwrap();
            assert_eq!(prompt.action, TerminalShutdownAction::CloseWindow);
            assert_eq!(
                prompt.reasons,
                vec![SharedString::from("pushing-repo is still running a push.")]
            );
        });
    });
}

/// Adding a submodule clones it; closing its repository mid-clone would kill
/// that clone, so it asks like a push does.
#[gpui::test]
fn a_running_submodule_clone_asks_before_its_repository_closes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = (*state_with_repo(RepoId(1), Path::new("/tmp/parent-repo"))).clone();
    state.repos[0].submodule_add_in_flight =
        Some(gitcomet_state::model::SubmoduleAddProgressState {
            url: "https://example.com/lib.git".into(),
            path: "vendor/lib".into(),
        });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, Arc::new(state), cx);
        });
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.request_close_repos(vec![RepoId(1)], None, cx);
            let prompt = this.pending_close_guard_prompt.clone().unwrap();
            assert_eq!(
                prompt.reasons,
                vec![SharedString::from(
                    "parent-repo is still running a submodule clone."
                )]
            );
        });
    });
}

#[gpui::test]
fn state_observers_are_notified_once_per_update_cycle(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = std::rc::Rc::clone(&calls);
    let subscription = host
        .observe_state(move |_, _| seen.set(seen.get() + 1))
        .expect("the window is open");

    // Two snapshots applied in one update are one notification.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            for repo in 1..=2 {
                let state = state_with_repo(RepoId(repo), Path::new("/tmp/observed"));
                test_support::apply_state_snapshot_for_test(this, state, cx);
            }
        });
    });
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::apply_state_snapshot_for_test(this, empty_state(), cx)
        });
    });
    cx.run_until_parked();
    assert_eq!(calls.get(), 2);

    drop(subscription);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let state = state_with_repo(RepoId(3), Path::new("/tmp/observed"));
            test_support::apply_state_snapshot_for_test(this, state, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(calls.get(), 2, "a dropped subscription is not called");
}

#[gpui::test]
fn selective_observers_hear_only_changes_to_their_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = std::rc::Rc::clone(&seen);
    let _subscription = cx
        .update(|_window, app| {
            host.observe_selected(
                |state| state.repos.len(),
                move |_, count, _| record.borrow_mut().push(*count),
                app,
            )
        })
        .expect("the window is open");
    let apply = |cx: &mut gpui::VisualTestContext, state: Arc<AppState>| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                test_support::apply_state_snapshot_for_test(this, state, cx)
            });
        });
        cx.run_until_parked();
    };

    apply(cx, state_with_repo(RepoId(1), Path::new("/tmp/selected")));
    // Another repository, same count: the selection did not change.
    apply(cx, state_with_repo(RepoId(2), Path::new("/tmp/selected-2")));
    apply(cx, empty_state());
    assert_eq!(*seen.borrow(), vec![1, 0]);
}

#[gpui::test]
fn the_syntax_service_highlights_known_languages_only(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    cx.update(|_window, app| {
        let rust = host.highlight_line(Path::new("src/lib.rs"), "fn main() {}", app);
        assert!(
            rust.iter().any(|(range, _)| *range == (0..2)),
            "the keyword is highlighted: {rust:?}"
        );
        assert!(
            host.highlight_line(Path::new("notes.unknown-ext"), "fn main() {}", app)
                .is_empty()
        );
    });
}

#[gpui::test]
fn repository_watches_lease_the_watcher_until_dropped(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(install_example);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/extension-watched-repo")));
    wait_until("the repository to open", || {
        !store_for_assert.snapshot().repos.is_empty()
    });
    sync_view_snapshot(cx, &view);
    let (host, repository) = cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host
            .active_repository(app)
            .unwrap()
            .expect("an open repository");
        (host, repository)
    });

    let watch = cx.update(|_window, app| host.watch_repository(&repository, app).unwrap());
    let repo_id = repository.repo_id();
    wait_until("the lease to count", || {
        store_for_assert.snapshot().watch_leases.get(&repo_id) == Some(&1)
    });
    drop(watch);
    wait_until("the lease to be released", || {
        store_for_assert.snapshot().watch_leases.is_empty()
    });
}
