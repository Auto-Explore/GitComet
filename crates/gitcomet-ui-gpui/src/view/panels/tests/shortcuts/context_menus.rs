//! Context-menu shortcuts and the history author filter.

use super::*;

fn copied_path_ends_with(text: &str, suffix: &std::path::Path) -> bool {
    let normalize = |value: &str| value.replace('\\', "/");
    normalize(text).ends_with(&normalize(&suffix.to_string_lossy()))
}

fn declared_shortcuts(model: &ContextMenuModel) -> Vec<String> {
    model
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { shortcut, .. } => shortcut.as_ref().map(|s| s.to_string()),
            _ => None,
        })
        .collect()
}

fn assert_declared_shortcuts(model: &ContextMenuModel, expected: &[impl AsRef<str>]) {
    let expected = expected
        .iter()
        .map(|s| s.as_ref().to_string())
        .collect::<Vec<_>>();
    assert_eq!(declared_shortcuts(model), expected);
}

fn context_menu_entry_disabled_by_label(model: &ContextMenuModel, expected: &str) -> bool {
    model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label, disabled, ..
            } if label.as_ref() == expected => Some(*disabled),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected `{expected}` entry to exist"))
}

/// Like [`context_menu_entry_disabled_by_label`], but matches on a label
/// prefix — for entries whose labels embed branch names or commit shas.
fn context_menu_entry_disabled_by_label_prefix(model: &ContextMenuModel, prefix: &str) -> bool {
    model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label, disabled, ..
            } if label.as_ref().starts_with(prefix) => Some(*disabled),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected entry starting with `{prefix}` to exist"))
}

/// Platform-aware label for `secondary`-modifier shortcuts, matching what the
/// context menus declare.
fn sec(suffix: &str) -> String {
    crate::view::shortcut_labels::secondary_shortcut(suffix)
}

fn context_menu_model_for(
    view: &gpui::Entity<super::super::super::GitCometView>,
    app: &mut gpui::App,
    kind: PopoverKind,
) -> ContextMenuModel {
    view.update(app, |this, cx| {
        this.popover_host.update(cx, |host, cx| {
            host.context_menu_model(&kind, cx)
                .unwrap_or_else(|| panic!("expected context menu model for {kind:?}"))
        })
    })
}

#[gpui::test]
fn recent_repository_shortcut_does_not_select_diff_content(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70512);
    let commit_id = CommitId("fedcba0987654323".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_recent_repo_diff_shortcut",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let target = DiffTarget::commit(commit_id.clone(), "src/lib.rs".into());
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(target).into());
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        let _ = window.draw(app);
        window.activate();
    });
    focus_diff_panel(cx, &view);
    draw_and_drain_test_window(cx);
    assert!(
        !diff_text_has_selection(cx, &view),
        "the fixture must start without a text selection"
    );

    cx.simulate_keystrokes("secondary-shift-a");
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "Ctrl/Cmd+Shift+A must open the recent-repositories picker"
    );
    assert!(
        !diff_text_has_selection(cx, &view),
        "opening recent repositories must not also select the diff contents"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn history_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(700);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_settings_history_shortcuts",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let history_filter_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::HistoryBranchFilter { repo_id })
    });
    assert_declared_shortcuts(&history_filter_model, &["F", "P", "N", "M", "A"]);
    assert_shortcut_action!(
        history_filter_model,
        "F",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::FullReachable
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "P",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::FirstParent
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "N",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::NoMerges
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "M",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::HistoryMode::MergesOnly
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        history_filter_model,
        "A",
        ContextMenuAction::SetHistoryScope {
            repo_id: rid,
            scope: gitcomet_core::domain::LogScope::AllBranches
        } if *rid == repo_id
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.history_view.update(cx, |history, cx| {
                    history.history_show_author = true;
                    history.history_show_date = true;
                    history.history_show_sha = true;
                    cx.notify();
                });
            });
        });
    });

    let change_tracking_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::ChangeTrackingSettings)
    });
    assert_declared_shortcuts(&change_tracking_model, &["C", "S"]);
    assert_shortcut_action!(
        change_tracking_model,
        "C",
        ContextMenuAction::SetChangeTrackingView {
            view: ChangeTrackingView::Combined
        }
    );
    assert_shortcut_action!(
        change_tracking_model,
        "S",
        ContextMenuAction::SetChangeTrackingView {
            view: ChangeTrackingView::SplitUntracked
        }
    );
}

fn author_filter_repo_with_many_authors(repo_id: RepoId, count: usize) -> RepoState {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_author_filter_many",
        std::process::id()
    ));
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &CommitId("deadbeefdeadbeef".into()));
    let log_page: Loadable<std::sync::Arc<gitcomet_core::domain::LogPage>> = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits: (0..count)
                .map(|ix| gitcomet_core::domain::Commit {
                    id: CommitId(format!("{ix:016x}").into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "msg".into(),
                    author: format!("author {ix:04}").into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                })
                .collect(),
            next_cursor: None,
        }
        .into(),
    );
    repo.log = log_page.clone();
    repo.history_state.log = log_page;
    repo
}

/// Every author must be reachable by scrolling, however many there are. The
/// list is virtualized, so far-down rows are not built until they are scrolled
/// to — but they do exist, rather than being cut off the end of the list.
#[gpui::test]
fn history_author_filter_scrolls_to_every_author(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    const AUTHORS: usize = 500;
    // Row 0 is "All authors", so the last author sits at row `AUTHORS`.
    const LAST_ROW: &str = "picker_prompt_item_500";

    let repo_id = RepoId(716);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_repo_with_many_authors(repo_id, AUTHORS)),
    );
    open_popover_for_test(cx, &view, PopoverKind::HistoryAuthorFilter { repo_id });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("picker_prompt_item_1").is_some(),
        "the first author must render"
    );
    assert!(
        cx.debug_bounds(LAST_ROW).is_none(),
        "a row far below the viewport must not be built until it is scrolled to"
    );

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| {
            host.scroll_history_author_filter_to_item_for_test(AUTHORS, cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds(LAST_ROW).is_some(),
        "scrolling to the end of the list must render the last author"
    );
}

/// The AUTHOR column header stays highlighted while its dropdown is up. The
/// dropdown is a picker rather than a context menu, so it has to opt into
/// keeping the invoker active explicitly.
#[gpui::test]
fn history_author_filter_keeps_its_header_highlighted(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(714);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_fixture_repo(repo_id)),
    );

    let invoker: SharedString = "history_author_filter_header".into();
    open_popover_for_test(
        cx,
        &view,
        (PopoverKind::HistoryAuthorFilter { repo_id }).invoked_by(invoker),
    );
    draw_and_drain_test_window(cx);

    let active = cx.update(|_window, app| view.read(app).active_context_menu_invoker.clone());
    assert_eq!(
        active.as_deref(),
        Some("history_author_filter_header"),
        "the AUTHOR header must stay highlighted while its dropdown is open"
    );
}

/// The dropdown carries a search box and full author names, so it is wider than
/// the sibling column menus (which sit at 220 design px).
#[gpui::test]
fn history_author_filter_is_wide_enough_for_full_names(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(717);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_fixture_repo(repo_id)),
    );
    open_popover_for_test(cx, &view, PopoverKind::HistoryAuthorFilter { repo_id });
    draw_and_drain_test_window(cx);

    let width = debug_width(cx, "app_popover");
    assert!(
        width >= 300.0,
        "the author dropdown must stay wide enough for full author names, got {width}"
    );
}

/// Enter applies the highlighted suggestion, and closes the popover.
#[gpui::test]
fn history_author_filter_applies_the_selected_author(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(713);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_fixture_repo(repo_id)),
    );
    // Arrow keys and Enter reach the search box through actions, which need the
    // text-input keymap installed.
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    open_popover_for_test(cx, &view, PopoverKind::HistoryAuthorFilter { repo_id });
    draw_and_drain_test_window(cx);

    cx.simulate_keystrokes("b o");
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("down");
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("enter");
    wait_until(cx, "the author filter to be applied", |cx| {
        cx.update(|_window, _app| {
            store_for_assert
                .snapshot()
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .and_then(|repo| repo.history_state.history_author_filter.clone())
                == Some("Bob".to_string())
        })
    });

    let popover_open = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .is_kind_open(&PopoverKind::HistoryAuthorFilter { repo_id })
    });
    assert!(!popover_open, "applying a filter must close the dropdown");
}

/// Suggestions only cover the commits loaded so far, and the backend filter is
/// a substring match, so a name that is not in the list is still applied as
/// typed rather than being a dead end.
#[gpui::test]
fn history_author_filter_applies_free_form_text(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(715);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_fixture_repo(repo_id)),
    );
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    open_popover_for_test(cx, &view, PopoverKind::HistoryAuthorFilter { repo_id });
    draw_and_drain_test_window(cx);

    // Neither loaded author matches, so nothing is highlighted to apply.
    cx.simulate_keystrokes("c a r");
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("enter");

    wait_until(cx, "the typed author filter to be applied", |cx| {
        cx.update(|_window, _app| {
            store_for_assert
                .snapshot()
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .and_then(|repo| repo.history_state.history_author_filter.clone())
                == Some("car".to_string())
        })
    });
}

/// Typing narrows the list without moving the selection, so the index can end up
/// past the end. The dropdown clamps it when it decides which row to highlight,
/// and Enter has to land on that same row rather than falling back to the raw
/// query — which would apply a filter the user never highlighted.
#[gpui::test]
fn history_author_filter_enter_applies_the_row_the_list_highlights(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(718);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_repo_with_authors(
            repo_id,
            &["Barb", "Bob", "boberta"],
        )),
    );
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    open_popover_for_test(cx, &view, PopoverKind::HistoryAuthorFilter { repo_id });
    draw_and_drain_test_window(cx);

    // "b" matches all three; three Downs land on the last of them.
    cx.simulate_keystrokes("b");
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("down down down");
    draw_and_drain_test_window(cx);

    // "bo" narrows to two, leaving the selection past the end.
    cx.simulate_keystrokes("o");
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("enter");

    wait_until(cx, "the highlighted author to be applied", |cx| {
        cx.update(|_window, _app| {
            store_for_assert
                .snapshot()
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .and_then(|repo| repo.history_state.history_author_filter.clone())
                == Some("boberta".to_string())
        })
    });
}

#[gpui::test]
fn repo_operation_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(701);
    let commit_id = CommitId("feedfacefeedface".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_repo_shortcuts",
        std::process::id()
    ));
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
        name: "main".to_string(),
        target: commit_id.clone(),
        upstream: Some(gitcomet_core::domain::Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit_id.clone(),
    }]));
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let pull_model =
        cx.update(|_window, app| context_menu_model_for(&view, app, PopoverKind::PullPicker));
    assert_declared_shortcuts(&pull_model, &["F", "O", "R", "A"]);
    assert_shortcut_action!(
        pull_model,
        "Enter",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::Default
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "F",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::FastForwardIfPossible
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "O",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::FastForwardOnly
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "R",
        ContextMenuAction::Pull {
            repo_id: rid,
            mode: gitcomet_core::services::PullMode::Rebase
        } if *rid == repo_id
    );
    assert_shortcut_action!(
        pull_model,
        "A",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let push_model =
        cx.update(|_window, app| context_menu_model_for(&view, app, PopoverKind::PushPicker));
    assert_declared_shortcuts(&push_model, &["F"]);
    assert_shortcut_action!(
        push_model,
        "Enter",
        ContextMenuAction::Push { repo_id: rid } if *rid == repo_id
    );
    assert_shortcut_action!(
        push_model,
        "F",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::ForcePushConfirm { repo_id: rid }
        } if *rid == repo_id
    );

    let branch_section_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchSectionMenu {
                repo_id,
                section: BranchSection::Remote,
            },
        )
    });
    assert_declared_shortcuts(&branch_section_model, &["F"]);
    assert_shortcut_action!(
        branch_section_model,
        "Enter",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::Checkout,
            },
        }
    );
    assert_shortcut_action!(
        branch_section_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let local_branch_name = "feature".to_string();
    let local_branch_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchMenu {
                repo_id,
                target: BranchMenuTarget::local(local_branch_name.clone()),
            },
        )
    });
    assert_declared_shortcuts(&local_branch_model, &[] as &[&str]);
    assert_shortcut_action!(
        local_branch_model,
        "Enter",
        ContextMenuAction::CheckoutBranch { repo_id: rid, name } if *rid == repo_id && name == "feature"
    );
    assert!(local_branch_model.items.iter().any(|item| {
        matches!(
            item,
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Rename branch"
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::OpenPopover {
                            kind: PopoverKind::RenameBranchPrompt {
                                repo_id: rid,
                                name,
                                is_current_branch: false,
                            }
                        } if *rid == repo_id && name == "feature"
                    )
        )
    }));

    let remote_branch_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::BranchMenu {
                repo_id,
                target: BranchMenuTarget::remote("origin", "feature"),
            },
        )
    });
    assert!(!remote_branch_model.items.iter().any(|item| {
        matches!(
            item,
            ContextMenuItem::Entry { label, .. } if label.as_ref() == "Rename branch"
        )
    }));
    assert_declared_shortcuts(&remote_branch_model, &[] as &[&str]);
    assert_shortcut_action!(
        remote_branch_model,
        "Enter",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::CheckoutRemoteBranchPrompt {
                repo_id: rid,
                remote,
                branch
            }
        } if *rid == repo_id && remote == "origin" && branch == "feature"
    );

    let remote_menu_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::remote(
                repo_id,
                RemotePopoverKind::Menu {
                    name: "origin".into(),
                },
            ),
        )
    });
    assert_declared_shortcuts(&remote_menu_model, &["F"]);
    assert_shortcut_action!(
        remote_menu_model,
        "F",
        ContextMenuAction::FetchAll { repo_id: rid } if *rid == repo_id
    );

    let stash_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StashMenu {
                repo_id,
                index: 3,
                message: "WIP".into(),
            },
        )
    });
    assert_declared_shortcuts(&stash_model, &["A", "P"]);
    assert_shortcut_action!(
        stash_model,
        "A",
        ContextMenuAction::ApplyStash {
            repo_id: rid,
            index
        } if *rid == repo_id && *index == 3
    );
    assert_shortcut_action!(
        stash_model,
        "P",
        ContextMenuAction::PopStash {
            repo_id: rid,
            index
        } if *rid == repo_id && *index == 3
    );
}

#[gpui::test]
fn file_and_diff_context_menu_shortcuts_match_expected_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(702);
    let commit_id = CommitId("cafebabecafebabe".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_shortcuts",
        std::process::id()
    ));
    let commit_file_path = std::path::PathBuf::from("src/main.rs");
    let unstaged_path = std::path::PathBuf::from("unstaged.rs");
    let staged_path = std::path::PathBuf::from("staged_added.rs");
    let conflicted_path = std::path::PathBuf::from("conflicted.rs");
    let hunk_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: staged_path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Added,
                conflict: None,
            }]),
            unstaged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: unstaged_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: hunk_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: conflicted_path.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                    conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                },
            ]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        hunk_path.clone(),
        DiffArea::Unstaged,
    ));
    repo.diff_state.diff = Loadable::Ready(
        simple_hunk_diff(DiffTarget::working_tree(
            hunk_path.clone(),
            DiffArea::Unstaged,
        ))
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    let commit_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::CommitMenu {
                repo_id,
                commit_id: commit_id.clone(),
            },
        )
    });
    assert_declared_shortcuts(&commit_model, &["C", "T", "D", "P", "R", "B", "I", "M"]);
    assert_shortcut_action!(
        commit_model,
        "Enter",
        ContextMenuAction::BrowseRepositoryAtCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "C",
        ContextMenuAction::CopyText { text } if text == commit_id.as_ref()
    );
    assert_shortcut_action!(
        commit_model,
        "T",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateTagPrompt { repo_id: rid, target }
        } if *rid == repo_id && target == commit_id.as_ref()
    );
    assert_shortcut_action!(
        commit_model,
        "D",
        ContextMenuAction::CheckoutCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "P",
        ContextMenuAction::CherryPickCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "R",
        ContextMenuAction::RevertCommit {
            repo_id: rid,
            commit_id: cid
        } if *rid == repo_id && cid == &commit_id
    );
    assert_shortcut_action!(
        commit_model,
        "B",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::RebaseOntoConfirm { repo_id: rid, onto }
        } if *rid == repo_id && onto == commit_id.as_ref()
    );
    assert_shortcut_action!(
        commit_model,
        "I",
        ContextMenuAction::LoadInteractiveRebaseSetup { repo_id: rid, base }
            if *rid == repo_id && base == commit_id.as_ref()
    );
    assert_shortcut_action!(
        commit_model,
        "M",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::MergeCommitConfirm {
                repo_id: rid,
                commit_id: cid
            }
        } if *rid == repo_id && cid == &commit_id
    );

    let commit_file_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::CommitFileMenu {
                repo_id,
                commit_id: commit_id.clone(),
                path: commit_file_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&commit_file_model, &["H", "A", "C"]);
    assert_shortcut_action!(
        commit_file_model,
        "Enter",
        ContextMenuAction::OpenFile { repo_id: rid, path }
            if *rid == repo_id && path == &commit_file_path
    );
    assert_shortcut_action!(
        commit_file_model,
        "A",
        ContextMenuAction::ApplyFileChange {
            repo_id: rid,
            target,
        } if *rid == repo_id
            && target
                == &gitcomet_core::domain::ApplyChangeTarget::commit(
                    commit_id.clone(),
                    commit_file_path.clone(),
                )
    );
    assert_shortcut_action!(
        commit_file_model,
        "H",
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &commit_file_path
    );
    assert_shortcut_action!(
        commit_file_model,
        "C",
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &commit_file_path)
    );

    let unstaged_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: unstaged_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(
        &unstaged_status_model,
        &[&sec("H"), &sec("S"), &sec("D"), &sec("Shift+C")],
    );
    assert_shortcut_action!(
        unstaged_status_model,
        "Enter",
        ContextMenuAction::OpenFile { repo_id: rid, path }
            if *rid == repo_id && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        &sec("H"),
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        &sec("S"),
        ContextMenuAction::StageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        &sec("D"),
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &unstaged_path
    );
    assert_shortcut_action!(
        unstaged_status_model,
        &sec("Shift+C"),
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &unstaged_path)
    );

    let staged_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Staged,
                path: staged_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(
        &staged_status_model,
        &[&sec("H"), &sec("U"), &sec("D"), &sec("Shift+C")],
    );
    assert_shortcut_action!(
        staged_status_model,
        "Enter",
        ContextMenuAction::OpenFile { repo_id: rid, path }
            if *rid == repo_id && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        &sec("H"),
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        &sec("U"),
        ContextMenuAction::UnstageSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        &sec("D"),
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Staged && path == &staged_path
    );
    assert_shortcut_action!(
        staged_status_model,
        &sec("Shift+C"),
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &staged_path)
    );

    let conflicted_status_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusFileMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: conflicted_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(
        &conflicted_status_model,
        &[&sec("H"), &sec("O"), &sec("T"), &sec("D"), &sec("Shift+C")],
    );
    assert_shortcut_action!(
        conflicted_status_model,
        "Enter",
        ContextMenuAction::SelectConflictDiff { repo_id: rid, path }
            if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        &sec("H"),
        ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory { repo_id: rid, path }
        } if *rid == repo_id && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        &sec("O"),
        ContextMenuAction::CheckoutConflictSideSelectionOrPath {
            repo_id: rid,
            area,
            path,
            side
        } if *rid == repo_id
            && *area == DiffArea::Unstaged
            && path == &conflicted_path
            && *side == gitcomet_core::services::ConflictSide::Ours
    );
    assert_shortcut_action!(
        conflicted_status_model,
        &sec("T"),
        ContextMenuAction::CheckoutConflictSideSelectionOrPath {
            repo_id: rid,
            area,
            path,
            side
        } if *rid == repo_id
            && *area == DiffArea::Unstaged
            && path == &conflicted_path
            && *side == gitcomet_core::services::ConflictSide::Theirs
    );
    assert_shortcut_action!(
        conflicted_status_model,
        &sec("D"),
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
            repo_id: rid,
            area,
            path
        } if *rid == repo_id && *area == DiffArea::Unstaged && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflicted_status_model,
        &sec("Shift+C"),
        ContextMenuAction::CopyText { text } if copied_path_ends_with(text, &conflicted_path)
    );
    // Discarding a conflict keeps ours, so the entry is live.
    assert!(!context_menu_entry_disabled_by_label(
        &conflicted_status_model,
        "Discard changes"
    ));

    let conflict_menu_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::StatusConflictMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: conflicted_path.clone(),
            },
        )
    });
    assert_declared_shortcuts(&conflict_menu_model, &[&sec("O"), &sec("T"), &sec("D")]);
    assert_shortcut_action!(
        conflict_menu_model,
        &sec("O"),
        ContextMenuAction::CheckoutConflictSideSelectionOrPath { side, path, .. }
            if *side == gitcomet_core::services::ConflictSide::Ours && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflict_menu_model,
        &sec("T"),
        ContextMenuAction::CheckoutConflictSideSelectionOrPath { side, path, .. }
            if *side == gitcomet_core::services::ConflictSide::Theirs && path == &conflicted_path
    );
    assert_shortcut_action!(
        conflict_menu_model,
        &sec("D"),
        ContextMenuAction::DiscardWorktreeChangesSelectionOrPath { area, path, .. }
            if *area == DiffArea::Unstaged && path == &conflicted_path
    );

    let diff_editor_unstaged_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::DiffEditorMenu {
                repo_id,
                area: DiffArea::Unstaged,
                path: Some(unstaged_path.clone()),
                hunk_patch: Some("hunk patch".into()),
                hunks_count: 2,
                lines_patch: Some("line patch".into()),
                discard_lines_patch: Some("discard patch".into()),
                lines_count: 3,
                copy_text: Some("copied selection".into()),
                copy_target: None,
            },
        )
    });
    assert_declared_shortcuts(&diff_editor_unstaged_model, &["S", "D", "C"]);
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "S",
        ContextMenuAction::ApplyIndexPatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "line patch" && !*reverse
    );
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "D",
        ContextMenuAction::ApplyWorktreePatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "discard patch" && *reverse
    );
    assert_shortcut_action!(
        diff_editor_unstaged_model,
        "C",
        ContextMenuAction::CopyDiffSelection { text } if text == "copied selection"
    );

    let diff_editor_staged_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::DiffEditorMenu {
                repo_id,
                area: DiffArea::Staged,
                path: Some(staged_path.clone()),
                hunk_patch: Some("staged hunk".into()),
                hunks_count: 1,
                lines_patch: Some("staged line".into()),
                discard_lines_patch: None,
                lines_count: 1,
                copy_text: Some("staged copy".into()),
                copy_target: None,
            },
        )
    });
    assert_declared_shortcuts(&diff_editor_staged_model, &["U", "C"]);
    assert_shortcut_action!(
        diff_editor_staged_model,
        "U",
        ContextMenuAction::ApplyIndexPatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && patch == "staged line" && *reverse
    );
    assert_shortcut_action!(
        diff_editor_staged_model,
        "C",
        ContextMenuAction::CopyDiffSelection { text } if text == "staged copy"
    );

    let diff_hunk_unstaged_model = cx.update(|_window, app| {
        context_menu_model_for(&view, app, PopoverKind::DiffHunkMenu { repo_id, src_ix: 3 })
    });
    assert_declared_shortcuts(&diff_hunk_unstaged_model, &[&sec("S"), &sec("D")]);
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        &sec("S"),
        ContextMenuAction::StageHunk {
            repo_id: rid,
            src_ix
        } if *rid == repo_id && *src_ix == 3
    );
    assert_shortcut_action!(
        diff_hunk_unstaged_model,
        &sec("D"),
        ContextMenuAction::ApplyWorktreePatch {
            repo_id: rid,
            patch,
            reverse
        } if *rid == repo_id && !patch.is_empty() && *reverse
    );

    let conflict_output_model = cx.update(|_window, app| {
        context_menu_model_for(
            &view,
            app,
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line: 12,
                selected_text: Some("chosen text".into()),
                has_source_a: true,
                has_source_b: true,
                has_source_c: true,
                is_three_way: true,
            },
        )
    });
    assert_declared_shortcuts(&conflict_output_model, &[&sec("C"), &sec("X"), &sec("V")]);
    assert_shortcut_action!(
        conflict_output_model,
        &sec("C"),
        ContextMenuAction::CopyText { text } if text == "chosen text"
    );
    assert_shortcut_action!(
        conflict_output_model,
        &sec("X"),
        ContextMenuAction::ConflictResolverOutputCut { text } if text == "chosen text"
    );
    assert_shortcut_action!(
        conflict_output_model,
        &sec("V"),
        ContextMenuAction::ConflictResolverOutputPaste
    );
}

#[gpui::test]
fn commit_context_menu_disables_history_rewrites_during_active_operations(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(1702);
    let commit_id = CommitId("cafebabecafebabe".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_history_rewrite_guard",
        std::process::id()
    ));

    // Each in-flight operation must disable every history-rewriting entry:
    // they all contend for git's single sequencer slot.
    let busy_states: [(&str, fn(&mut RepoState)); 4] = [
        ("pending merge", |repo| {
            repo.merge_commit_message = Loadable::Ready(Some("merge message".to_string()));
        }),
        ("rebase in progress", |repo| {
            repo.rebase_in_progress = Loadable::Ready(true);
        }),
        ("cherry-pick sequencer", |repo| {
            repo.sequencer_state =
                Loadable::Ready(gitcomet_core::services::SequencerState::CherryPick);
        }),
        ("revert sequencer", |repo| {
            repo.sequencer_state = Loadable::Ready(gitcomet_core::services::SequencerState::Revert);
        }),
    ];
    let idle_model = {
        apply_state(
            cx,
            &view,
            app_state_with_active_repo(shortcut_fixture_repo(repo_id, &workdir, &commit_id)),
        );
        cx.update(|_window, app| {
            context_menu_model_for(
                &view,
                app,
                PopoverKind::CommitMenu {
                    repo_id,
                    commit_id: commit_id.clone(),
                },
            )
        })
    };
    assert!(
        !context_menu_entry_disabled_by_label(&idle_model, "Revert cafebabe…"),
        "Revert names the clicked commit and is enabled when idle"
    );
    for (state_name, make_busy) in busy_states {
        let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
        make_busy(&mut repo);
        apply_state(cx, &view, app_state_with_active_repo(repo));
        let model = cx.update(|_window, app| {
            context_menu_model_for(
                &view,
                app,
                PopoverKind::CommitMenu {
                    repo_id,
                    commit_id: commit_id.clone(),
                },
            )
        });
        assert!(
            context_menu_entry_disabled_by_label(&model, "Cherry-pick"),
            "Cherry-pick enabled during {state_name}"
        );
        assert!(
            context_menu_entry_disabled_by_label_prefix(&model, "Revert "),
            "Revert enabled during {state_name}"
        );
        assert!(
            context_menu_entry_disabled_by_label_prefix(&model, "Rebase "),
            "Rebase onto enabled during {state_name}"
        );
        assert!(
            context_menu_entry_disabled_by_label_prefix(&model, "Interactive rebase"),
            "Interactive rebase enabled during {state_name}"
        );

        let branch_model = cx.update(|_window, app| {
            context_menu_model_for(
                &view,
                app,
                PopoverKind::BranchMenu {
                    repo_id,
                    target: BranchMenuTarget::local("feature"),
                },
            )
        });
        assert!(
            context_menu_entry_disabled_by_label_prefix(&branch_model, "Rebase "),
            "branch menu Rebase onto enabled during {state_name}"
        );
    }
}

#[gpui::test]
fn diff_action_menu_contains_whitespace_setting(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70546);
    let commit_id = CommitId("1122334455667746".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_action_menu",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("diff_whitespace_mode_header").is_none(),
        "expected whitespace setting to be removed from the diff action bar"
    );
    let menu_bounds = cx
        .debug_bounds("diff_action_menu")
        .expect("expected diff action menu button in the diff action bar");
    let close_bounds = cx
        .debug_bounds("diff_close")
        .expect("expected diff close button in the diff action bar");
    assert!(
        menu_bounds.right() <= close_bounds.left(),
        "expected diff action menu button to be before the close button"
    );

    focus_diff_panel(cx, &view);
    cx.simulate_click(menu_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let popover_kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert_eq!(
        popover_kind,
        Some(PopoverKind::DiffActionMenu),
        "expected clicking the cog to open the diff action menu"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected opening the diff action menu to move focus away from the diff panel"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(
        !popover_is_open(cx, &view),
        "expected Escape to close the diff action menu"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected closing the diff action menu to restore diff-panel focus"
    );

    cx.simulate_click(menu_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        Some(PopoverKind::DiffActionMenu),
        "expected reopening the cog menu to show diff actions"
    );

    let whitespace_bounds = cx
        .debug_bounds("context_menu_show_whitespace_changes")
        .expect("expected whitespace setting to be rendered in the diff action menu");
    assert!(
        cx.debug_bounds("context_menu_reveal_whitespace_characters")
            .is_some(),
        "expected reveal whitespace characters setting to be rendered in the diff action menu"
    );
    assert!(
        cx.debug_bounds("context_menu_word_wrap").is_some(),
        "expected word wrap setting to be rendered in the diff action menu"
    );
    cx.simulate_click(whitespace_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let whitespace_mode =
        cx.update(|_window, app| crate::view::test_support::diff_whitespace_mode(view.read(app)));
    assert_eq!(
        whitespace_mode,
        DiffWhitespaceMode::Ignore,
        "expected selecting the whitespace entry to toggle the global diff whitespace mode"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected the diff action menu to remain open after selecting whitespace mode"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting whitespace mode to restore diff-panel focus"
    );
    assert!(
        cx.debug_bounds("context_menu_show_whitespace_changes")
            .is_some(),
        "expected the whitespace setting to remain visible after toggling"
    );

    let reveal_bounds = cx
        .debug_bounds("context_menu_reveal_whitespace_characters")
        .expect("expected reveal whitespace setting to remain visible");
    cx.simulate_click(reveal_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.update(
            |_window, app| crate::view::test_support::diff_reveal_whitespace_chars(view.read(app))
        ),
        "expected selecting reveal whitespace to toggle the global reveal preference"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected the diff action menu to remain open after selecting reveal whitespace"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting reveal whitespace to restore diff-panel focus"
    );

    let word_wrap_bounds = cx
        .debug_bounds("context_menu_word_wrap")
        .expect("expected word wrap setting to remain visible");
    cx.simulate_click(word_wrap_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.update(|_window, app| crate::view::test_support::diff_word_wrap(view.read(app))),
        "expected selecting word wrap to toggle the global word wrap preference"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected the diff action menu to remain open after selecting word wrap"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting word wrap to restore diff-panel focus"
    );
}

#[gpui::test]
fn open_workspace_shortcut_opens_the_workspace_chooser_with_nothing_focused(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        let _ = window.draw(app);
        window.activate();
    });
    focus_detached_window_focus(cx);

    cx.simulate_keystrokes("secondary-shift-r");
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    // One press opens it: a chord handled twice would toggle it shut again.
    assert_eq!(
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app)),
        Some(PopoverKind::RepoPicker {
            scope: RepoPickerScope::WorkspacesOnly
        }),
        "Ctrl/Cmd+Shift+R opens the workspace chooser"
    );

    cx.simulate_keystrokes("secondary-shift-r");
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app)),
        None,
        "pressing it again closes the chooser"
    );
}
