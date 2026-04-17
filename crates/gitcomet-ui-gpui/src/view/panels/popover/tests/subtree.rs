use super::branch::create_tracking_store;
use super::*;

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
}

fn open_split_prompt(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    initial_path: Option<std::path::PathBuf>,
) -> RepoId {
    let repo_id = cx.update(|window, app| {
        let repo_id = view
            .read(app)
            .state
            .active_repo
            .expect("expected active repo");
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::Repo {
                        repo_id,
                        kind: RepoPopoverKind::Subtree(SubtreePopoverKind::SplitPrompt {
                            initial_path: initial_path.clone(),
                        }),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        repo_id
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    repo_id
}

fn open_merge_prompt(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    initial_path: Option<std::path::PathBuf>,
) -> RepoId {
    let repo_id = cx.update(|window, app| {
        let repo_id = view
            .read(app)
            .state
            .active_repo
            .expect("expected active repo");
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::Repo {
                        repo_id,
                        kind: RepoPopoverKind::Subtree(SubtreePopoverKind::MergePrompt {
                            initial_path: initial_path.clone(),
                        }),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        repo_id
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    repo_id
}

#[gpui::test]
fn split_prompt_prefills_stored_extract_defaults_and_enables_publish(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let repo_id = RepoId(42);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_subtree_split_remote",
        std::process::id()
    ));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.open = Loadable::Ready(());
            repo.subtrees = Loadable::Ready(
                vec![gitcomet_core::domain::Subtree {
                    path: std::path::PathBuf::from("libs/example"),
                    source: Some(gitcomet_core::domain::SubtreeSourceConfig {
                        local_repository: Some("/tmp/libs-example".to_string()),
                        repository: "https://example.com/org/libs-example.git".to_string(),
                        reference: "main".to_string(),
                        push_refspec: Some("refs/heads/main".to_string()),
                        squash: true,
                    }),
                }]
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.popover_host.update(cx, |host, _| {
                host.state = Arc::clone(&this.state);
            });
            cx.notify();
        });
    });

    open_split_prompt(&view, cx, Some(std::path::PathBuf::from("libs/example")));

    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.subtree_split_remote_enabled,
            "expected publish to remote to be restored on open"
        );
        assert_eq!(
            host.subtree_split_destination_repo_input
                .read_with(app, |input, _| input.text().to_string()),
            "/tmp/libs-example"
        );
        assert_eq!(
            host.subtree_split_destination_branch_input
                .read_with(app, |input, _| input.text().to_string()),
            "main"
        );
        assert_eq!(
            host.subtree_split_remote_repository_input
                .read_with(app, |input, _| input.text().to_string()),
            "https://example.com/org/libs-example.git"
        );
    });
    assert!(
        cx.debug_bounds("subtree_split_publish_remote_toggle")
            .is_some(),
        "expected publish toggle to render"
    );
    assert!(
        cx.debug_bounds("subtree_split_advanced_toggle").is_some(),
        "expected advanced toggle to render"
    );
    assert!(
        cx.debug_bounds("subtree_split_advanced_indicator_collapsed")
            .is_some(),
        "expected advanced options indicator to start collapsed"
    );
}

#[gpui::test]
fn split_prompt_prefills_local_destination_without_enabling_publish(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let repo_id = RepoId(43);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_subtree_split_local",
        std::process::id()
    ));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.open = Loadable::Ready(());
            repo.subtrees = Loadable::Ready(
                vec![gitcomet_core::domain::Subtree {
                    path: std::path::PathBuf::from("libs/example"),
                    source: Some(gitcomet_core::domain::SubtreeSourceConfig {
                        local_repository: Some("/tmp/libs-example".to_string()),
                        repository: "/tmp/libs-example".to_string(),
                        reference: "main".to_string(),
                        push_refspec: Some("refs/heads/main".to_string()),
                        squash: true,
                    }),
                }]
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.popover_host.update(cx, |host, _| {
                host.state = Arc::clone(&this.state);
            });
            cx.notify();
        });
    });

    open_split_prompt(&view, cx, Some(std::path::PathBuf::from("libs/example")));

    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            !host.subtree_split_remote_enabled,
            "expected publish to stay disabled for local-only source metadata"
        );
        assert_eq!(
            host.subtree_split_destination_repo_input
                .read_with(app, |input, _| input.text().to_string()),
            "/tmp/libs-example"
        );
        assert_eq!(
            host.subtree_split_destination_branch_input
                .read_with(app, |input, _| input.text().to_string()),
            "main"
        );
        assert_eq!(
            host.subtree_split_remote_repository_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
    });
}

#[gpui::test]
fn split_prompt_defaults_to_simple_extract_ui(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("subtree-split-simple-ui");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let repo_id = open_split_prompt(&view, cx, Some(std::path::PathBuf::from("libs/example")));

    cx.debug_bounds("subtree_split_publish_remote_toggle")
        .expect("expected publish toggle");
    cx.debug_bounds("subtree_split_advanced_toggle")
        .expect("expected advanced toggle");
    cx.debug_bounds("subtree_split_advanced_indicator_collapsed")
        .expect("expected collapsed advanced indicator");
    assert!(
        cx.debug_bounds("subtree_split_toggle_rejoin").is_none(),
        "expected advanced split actions to stay hidden by default"
    );

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Subtree(SubtreePopoverKind::SplitPrompt {
                    initial_path: Some(std::path::PathBuf::from("libs/example")),
                }),
            })
        );
        assert!(
            !host.subtree_split_advanced_enabled,
            "expected advanced options to be collapsed by default"
        );
        assert!(
            !host.subtree_split_remote_enabled,
            "expected publish to remote to stay disabled without stored remote state"
        );
        assert_eq!(
            host.subtree_path_input
                .read_with(app, |input, _| input.text().to_string()),
            "libs/example"
        );
        assert_eq!(
            host.subtree_split_destination_repo_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        assert_eq!(
            host.subtree_split_destination_branch_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        assert_eq!(
            host.subtree_split_branch_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        let focus = host
            .subtree_path_input
            .read_with(app, |input, _| input.focus_handle());
        assert!(
            focus.is_focused(window),
            "expected path input to receive initial focus"
        );
    });
}

#[gpui::test]
fn split_prompt_advanced_toggle_preserves_values_until_reopen(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("subtree-split-advanced-toggle");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let path = std::path::PathBuf::from("libs/example");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    open_split_prompt(&view, cx, Some(path.clone()));

    click_debug_selector(cx, "subtree_split_advanced_toggle");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.subtree_split_advanced_enabled,
            "expected advanced controls to be enabled after clicking the toggle"
        );
    });
    assert!(
        cx.debug_bounds("subtree_split_advanced_indicator_expanded")
            .is_some(),
        "expected advanced options indicator to switch to expanded"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.subtree_split_branch_input
                    .update(cx, |input, cx| input.set_text("split-history", cx));
            });
        });
    });

    click_debug_selector(cx, "subtree_split_advanced_toggle");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            !host.subtree_split_advanced_enabled,
            "expected advanced controls to collapse after clicking the toggle again"
        );
    });
    assert!(
        cx.debug_bounds("subtree_split_advanced_indicator_collapsed")
            .is_some(),
        "expected advanced options indicator to switch back to collapsed"
    );

    click_debug_selector(cx, "subtree_split_advanced_toggle");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        cx.debug_bounds("subtree_split_advanced_indicator_expanded")
            .is_some(),
        "expected advanced options indicator to expand again"
    );

    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.subtree_split_branch_input
                .read_with(app, |input, _| input.text().to_string()),
            "split-history"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    open_split_prompt(&view, cx, Some(path));

    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            !host.subtree_split_advanced_enabled,
            "expected advanced options to reset when reopening the prompt"
        );
        assert_eq!(
            host.subtree_split_branch_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
    });
    assert!(
        cx.debug_bounds("subtree_split_advanced_indicator_collapsed")
            .is_some(),
        "expected advanced options indicator to reset to collapsed on reopen"
    );
}

#[gpui::test]
fn split_prompt_allows_blank_initial_path_for_section_level_split(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("subtree-split-section-level");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let repo_id = open_split_prompt(&view, cx, None);

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Subtree(SubtreePopoverKind::SplitPrompt {
                    initial_path: None,
                }),
            })
        );
        assert_eq!(
            host.subtree_path_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        assert_eq!(
            host.subtree_split_destination_repo_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        let focus = host
            .subtree_path_input
            .read_with(app, |input, _| input.focus_handle());
        assert!(
            focus.is_focused(window),
            "expected path input to receive focus for section-level split"
        );
    });
}

#[gpui::test]
fn merge_prompt_defaults_to_blank_path_for_section_level_merge(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("subtree-merge-section-level");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let repo_id = open_merge_prompt(&view, cx, None);

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Subtree(SubtreePopoverKind::MergePrompt {
                    initial_path: None,
                }),
            })
        );
        assert_eq!(
            host.subtree_path_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        assert_eq!(
            host.subtree_merge_revision_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        let focus = host
            .subtree_path_input
            .read_with(app, |input, _| input.focus_handle());
        assert!(
            focus.is_focused(window),
            "expected path input to receive focus for section-level merge"
        );
    });
}

#[gpui::test]
fn merge_prompt_prefills_path_and_focuses_revision_for_row_level_merge(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let repo_id = RepoId(44);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_subtree_merge_row",
        std::process::id()
    ));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.open = Loadable::Ready(());
            repo.subtrees = Loadable::Ready(
                vec![gitcomet_core::domain::Subtree {
                    path: std::path::PathBuf::from("libs/example"),
                    source: Some(gitcomet_core::domain::SubtreeSourceConfig {
                        local_repository: Some("/tmp/libs-example".to_string()),
                        repository: "/tmp/libs-example".to_string(),
                        reference: "main".to_string(),
                        push_refspec: Some("refs/heads/main".to_string()),
                        squash: false,
                    }),
                }]
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.popover_host.update(cx, |host, _| {
                host.state = Arc::clone(&this.state);
            });
            cx.notify();
        });
    });

    open_merge_prompt(&view, cx, Some(std::path::PathBuf::from("libs/example")));

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Subtree(SubtreePopoverKind::MergePrompt {
                    initial_path: Some(std::path::PathBuf::from("libs/example")),
                }),
            })
        );
        assert_eq!(
            host.subtree_path_input
                .read_with(app, |input, _| input.text().to_string()),
            "libs/example"
        );
        assert_eq!(
            host.subtree_merge_revision_input
                .read_with(app, |input, _| input.text().to_string()),
            ""
        );
        assert!(
            !host.subtree_squash_enabled,
            "expected merge prompt to restore stored subtree squash mode"
        );
        let focus = host
            .subtree_merge_revision_input
            .read_with(app, |input, _| input.focus_handle());
        assert!(
            focus.is_focused(window),
            "expected revision input to receive focus for row-level merge"
        );
    });
}
