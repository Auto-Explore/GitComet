use super::super::host::upstream_prompt_submission;
use super::super::upstream_picker::{self, UpstreamNavTarget, UpstreamTarget};
use super::*;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state};
use crate::view::test_support::{push_test_state, redraw};
use gitcomet_core::domain::{Branch, CommitId, Remote, RemoteBranch, Upstream};
use gitcomet_state::model::{Loadable, RepoId, RepoState};

fn oid(value: &str) -> CommitId {
    CommitId(Arc::from(value))
}

fn tracked_repo(repo_id: RepoId) -> RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/upstream-picker"));
    repo.head_branch = Loadable::Ready("feature/current".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/current".to_string(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: Some(Upstream {
            remote: "mirror".to_string(),
            branch: "feature/current".to_string(),
        }),
        divergence: None,
    }]));
    repo.remotes = Loadable::Ready(Arc::new(vec![
        Remote {
            name: "origin".to_string(),
            url: None,
        },
        Remote {
            name: "mirror".to_string(),
            url: None,
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: oid("2222222222222222222222222222222222222222"),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "HEAD".to_string(),
            target: oid("2222222222222222222222222222222222222222"),
        },
        RemoteBranch {
            remote: "mirror".to_string(),
            name: "feature/current".to_string(),
            target: oid("1111111111111111111111111111111111111111"),
        },
        RemoteBranch {
            remote: "removed".to_string(),
            name: "stale".to_string(),
            target: oid("3333333333333333333333333333333333333333"),
        },
    ]));
    repo
}

fn untracked_repo(repo_id: RepoId) -> RepoState {
    let mut repo = tracked_repo(repo_id);
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/current".to_string(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: None,
        divergence: None,
    }]));
    repo
}

fn open_popover(
    cx: &mut gpui::TestAppContext,
    repo: RepoState,
    kind: PopoverKind,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let repo_id = repo.id;
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    kind,
                    gpui::point(gpui::px(160.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    redraw(cx);
    (view, cx)
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    redraw(cx);
}

#[gpui::test]
fn upstream_picker_lists_live_remote_branches_and_marks_current(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let (view, cx) = open_popover(
        cx,
        tracked_repo(repo_id),
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    let (targets, marked, first_section) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let built = upstream_picker::cached(host, repo_id, &branch, "");
        (
            built.payloads.to_vec(),
            built.marked_index,
            built.items[0].section_label().cloned(),
        )
    });
    assert_eq!(
        targets,
        vec![
            UpstreamTarget {
                remote: "mirror".to_string(),
                branch: "feature/current".to_string(),
            },
            UpstreamTarget {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            },
        ],
        "the current upstream must lead the checkout-style rows; HEAD symrefs and removed remotes must not be offered"
    );
    assert_eq!(marked, Some(0));
    assert_eq!(first_section.as_deref(), Some("Remote Branches"));
    assert!(cx.debug_bounds("upstream_unlink").is_some());
    let unlink = cx.debug_bounds("upstream_unlink").expect("unlink row");
    let first_branch = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("first remote branch row");
    assert!(
        unlink.bottom() < first_branch.top(),
        "the unlink section must precede the remote-branch list with a gap"
    );
}

#[gpui::test]
fn upstream_picker_filters_by_full_remote_branch_name(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let (view, cx) = open_popover(
        cx,
        tracked_repo(repo_id),
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    let targets = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        upstream_picker::nav_targets(host, repo_id, &branch, "mirror/feature")
    });
    assert_eq!(
        targets,
        vec![
            UpstreamNavTarget::Unlink,
            UpstreamNavTarget::Branch(UpstreamTarget {
                remote: "mirror".to_string(),
                branch: "feature/current".to_string(),
            }),
        ],
        "unlink stays keyboard-reachable even while branch rows are filtered"
    );
}

#[gpui::test]
fn untracked_upstream_picker_offers_create_new_instead_of_unlink(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let (view, cx) = open_popover(
        cx,
        untracked_repo(repo_id),
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    let targets = cx.update(|_window, app| {
        upstream_picker::nav_targets(view.read(app).popover_host.read(app), repo_id, &branch, "")
    });
    assert_eq!(targets.first(), Some(&UpstreamNavTarget::CreateNew));
    assert!(cx.debug_bounds("upstream_create_new").is_some());
    assert!(cx.debug_bounds("upstream_unlink").is_none());
    let create = cx
        .debug_bounds("upstream_create_new")
        .expect("create-new row");
    let first_branch = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("first remote branch row");
    assert!(
        create.bottom() < first_branch.top(),
        "the create-new section must precede the remote-branch list with a gap"
    );
}

#[gpui::test]
fn upstream_create_new_opens_the_set_only_prompt(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let (view, cx) = open_popover(
        cx,
        untracked_repo(repo_id),
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    click(cx, "upstream_create_new");

    let (kind, remote_branch) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        (
            host.popover_kind_for_tests(),
            host.push_upstream_branch_input.read(app).text().to_string(),
        )
    });
    assert_eq!(
        kind,
        Some(PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "origin".to_string(),
            configure_only_for: Some(branch.clone()),
        })
    );
    assert_eq!(remote_branch, branch);
    assert!(cx.debug_bounds("push_upstream_go").is_some());
}

#[test]
fn set_only_prompt_builds_a_config_action_without_a_push() {
    let kind = PopoverKind::PushSetUpstreamPrompt {
        repo_id: RepoId(1),
        remote: "origin".to_string(),
        configure_only_for: Some("feature/current".to_string()),
    };

    let message =
        upstream_prompt_submission(&kind, "mirror".to_string(), "review/current".to_string())
            .expect("upstream prompt submission");
    assert!(matches!(
        message,
        Msg::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream: Upstream { remote, branch: remote_branch },
        } if branch == "feature/current"
            && remote == "mirror"
            && remote_branch == "review/current"
    ));
}

#[gpui::test]
fn upstream_create_new_is_keyboard_reachable(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let (view, cx) = open_popover(
        cx,
        untracked_repo(repo_id),
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    // The first branch row starts selected. Up reaches the fixed Create new
    // action above the scrollable remote-branch list.
    simulate_key_press(cx, "up");
    simulate_key_press(cx, "enter");
    redraw(cx);

    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        Some(PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "origin".to_string(),
            configure_only_for: Some(branch),
        })
    );
}

#[gpui::test]
fn upstream_picker_preserves_a_configured_remote_name_containing_slashes(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let mut repo = tracked_repo(repo_id);
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "forks/alice".to_string(),
        url: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "forks/alice".to_string(),
        name: "main".to_string(),
        target: oid("1111111111111111111111111111111111111111"),
    }]));
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: branch.clone(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: Some(Upstream {
            remote: "forks/alice".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));
    let (view, cx) = open_popover(
        cx,
        repo,
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    let (targets, marked) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let built = upstream_picker::cached(host, repo_id, &branch, "");
        (built.payloads.to_vec(), built.marked_index)
    });
    assert_eq!(
        targets,
        vec![UpstreamTarget {
            remote: "forks/alice".to_string(),
            branch: "main".to_string(),
        }]
    );
    assert_eq!(marked, Some(0));
}

#[gpui::test]
fn upstream_picker_distinguishes_overlapping_remote_names(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let mut repo = tracked_repo(repo_id);
    repo.remotes = Loadable::Ready(Arc::new(vec![
        Remote {
            name: "team".to_string(),
            url: None,
        },
        Remote {
            name: "team/alice".to_string(),
            url: None,
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "team".to_string(),
            name: "alice/main".to_string(),
            target: oid("1111111111111111111111111111111111111111"),
        },
        RemoteBranch {
            remote: "team/alice".to_string(),
            name: "main".to_string(),
            target: oid("1111111111111111111111111111111111111111"),
        },
    ]));
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: branch.clone(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: Some(Upstream {
            remote: "team/alice".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));
    let (view, cx) = open_popover(
        cx,
        repo,
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    let (targets, labels, marked) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let built = upstream_picker::cached(host, repo_id, &branch, "");
        (
            built.payloads.to_vec(),
            built
                .items
                .iter()
                .map(|item| item.debug_display_text().to_string())
                .collect::<Vec<_>>(),
            built.marked_index,
        )
    });
    assert_eq!(
        targets,
        vec![
            UpstreamTarget {
                remote: "team/alice".to_string(),
                branch: "main".to_string(),
            },
            UpstreamTarget {
                remote: "team".to_string(),
                branch: "alice/main".to_string(),
            },
        ],
        "the exact current upstream must be first"
    );
    assert_eq!(labels, vec!["team/alice / main", "team / alice/main"]);
    assert_eq!(marked, Some(0));
}

#[gpui::test]
fn upstream_unlink_is_reachable_from_the_search_box_with_the_keyboard(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let (view, cx) = open_popover(
        cx,
        tracked_repo(repo_id),
        PopoverKind::UpstreamPicker { repo_id, branch },
    );

    // The current upstream starts at index one; Up reaches the fixed unlink
    // row before the remote-branch list.
    simulate_key_press(cx, "up");
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .upstream_picker_selected_index
        }),
        Some(0)
    );
    simulate_key_press(cx, "enter");
    redraw(cx);
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        None,
        "Enter on the keyboard-selected unlink row should close the picker"
    );
}

#[gpui::test]
fn tracked_upstream_picker_never_preselects_unlink_while_rows_are_loading(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let branch = "feature/current".to_string();
    let mut loading = tracked_repo(repo_id);
    loading.remote_branches = Loadable::Loading;
    let (view, cx) = open_popover(
        cx,
        loading,
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: branch.clone(),
        },
    );

    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .upstream_picker_selected_index
        }),
        None,
        "a destructive fixed action must not become the loading fallback"
    );
    simulate_key_press(cx, "enter");
    assert!(matches!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        Some(PopoverKind::UpstreamPicker { .. })
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(tracked_repo(repo_id), repo_id),
                cx,
            );
        });
    });
    redraw(cx);
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .upstream_picker_selected_index
        }),
        Some(1),
        "once rows arrive, focus should move to the current upstream after Unlink"
    );
}

#[gpui::test]
fn first_push_remote_selector_changes_remote_without_resetting_branch_name(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let mut repo = tracked_repo(repo_id);
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/current".to_string(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: None,
        divergence: None,
    }]));
    let (view, cx) = open_popover(
        cx,
        repo,
        PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "origin".to_string(),
            configure_only_for: None,
        },
    );

    assert!(cx.debug_bounds("push_upstream_remote_selector").is_some());
    click(cx, "push_upstream_remote_selector");
    assert!(
        cx.debug_bounds("push_upstream_remote_option_0").is_some(),
        "alphabetically first mirror option should be visible"
    );
    click(cx, "push_upstream_remote_option_0");

    let (kind, branch_text) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        (
            host.popover_kind_for_tests(),
            host.push_upstream_branch_input.read(app).text().to_string(),
        )
    });
    assert_eq!(
        kind,
        Some(PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "mirror".to_string(),
            configure_only_for: None,
        })
    );
    assert_eq!(branch_text, "feature/current");
}

#[gpui::test]
fn first_push_remote_selector_supports_arrow_and_enter_selection(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let mut repo = tracked_repo(repo_id);
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/current".to_string(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: None,
        divergence: None,
    }]));
    let (view, cx) = open_popover(
        cx,
        repo,
        PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "origin".to_string(),
            configure_only_for: None,
        },
    );

    // The branch input receives initial focus. Shift+Tab focuses the preceding
    // remote selector; all remaining interaction is keyboard-only.
    simulate_key_press(cx, "shift-tab");
    cx.update(|window, app| {
        assert!(
            view.read(app)
                .popover_host
                .read(app)
                .push_upstream_remote_focus_handle
                .is_focused(window),
            "Shift+Tab from the branch input should focus the remote selector"
        );
    });
    simulate_key_press(cx, "enter");
    redraw(cx);
    assert!(cx.debug_bounds("push_upstream_remote_option_0").is_some());
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .push_upstream_remote_selected_index
        }),
        Some(1),
        "origin is the second item in the sorted remote list"
    );

    simulate_key_press(cx, "down");
    simulate_key_press(cx, "enter");
    redraw(cx);
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        Some(PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "mirror".to_string(),
            configure_only_for: None,
        })
    );
}

#[gpui::test]
fn escape_dismisses_set_upstream_prompt_after_a_remote_is_selected(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let mut repo = tracked_repo(repo_id);
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/current".to_string(),
        target: oid("1111111111111111111111111111111111111111"),
        upstream: None,
        divergence: None,
    }]));
    let (view, cx) = open_popover(
        cx,
        repo,
        PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "origin".to_string(),
            configure_only_for: None,
        },
    );

    simulate_key_press(cx, "shift-tab");
    simulate_key_press(cx, "enter");
    simulate_key_press(cx, "down");
    simulate_key_press(cx, "enter");
    redraw(cx);
    assert!(matches!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        Some(PopoverKind::PushSetUpstreamPrompt { .. })
    ));

    simulate_key_press(cx, "escape");
    redraw(cx);
    assert_eq!(
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        }),
        None,
        "Escape must propagate to the enclosing prompt once the menu is closed"
    );
}

#[gpui::test]
fn first_push_with_one_remote_keeps_a_static_remote_label(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let mut repo = tracked_repo(repo_id);
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".to_string(),
        url: None,
    }]));
    let (_view, cx) = open_popover(
        cx,
        repo,
        PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote: "origin".to_string(),
            configure_only_for: None,
        },
    );

    assert!(cx.debug_bounds("push_upstream_remote_static").is_some());
    assert!(cx.debug_bounds("push_upstream_remote_selector").is_none());
}

#[gpui::test]
fn upstream_picker_empty_search_requires_navigation_before_unlink(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_popover(
        cx,
        tracked_repo(repo_id),
        PopoverKind::UpstreamPicker {
            repo_id,
            branch: "feature/current".to_string(),
        },
    );
    cx.update(|_window, app| {
        let input = view
            .read(app)
            .popover_host
            .read(app)
            .remote_picker_search_input
            .clone()
            .unwrap();
        input.update(app, |input, cx| input.set_text("no-matching-branch", cx));
    });
    redraw(cx);
    simulate_key_press(cx, "enter");
    redraw(cx);
    assert!(
        matches!(
            cx.update(|_window, app| view
                .read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()),
            Some(PopoverKind::UpstreamPicker { .. })
        ),
        "Enter after an unsuccessful search must leave the picker open"
    );
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .upstream_picker_selected_index),
        None
    );
    simulate_key_press(cx, "up");
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .upstream_picker_selected_index),
        Some(0)
    );
    simulate_key_press(cx, "enter");
    redraw(cx);
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()),
        None
    );
}
