use super::*;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};

/// The focus entry's label and the action it sends.
fn focus_entry(model: &ContextMenuModel) -> (String, ContextMenuAction) {
    model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if matches!(**action, ContextMenuAction::ToggleHistoryFocus { .. }) =>
            {
                Some((label.to_string(), (**action).clone()))
            }
            _ => None,
        })
        .expect("expected a focus entry in the menu")
}

/// Builds a one-repo app state focused on `focus`, then reads the menu `kind`
/// produces.
fn menu_with_focus(
    cx: &mut gpui::TestAppContext,
    focus: HistoryFocusSet,
    kind: PopoverKind,
) -> ContextMenuModel {
    menu_with_repo(cx, |repo| repo.history_state.history_focus = focus, kind)
}

/// [`menu_with_focus`] with the repository shaped by `setup`.
fn menu_with_repo(
    cx: &mut gpui::TestAppContext,
    setup: impl FnOnce(&mut RepoState),
    kind: PopoverKind,
) -> ContextMenuModel {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(7);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_focus_menu",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.head_branch = Loadable::Ready("main".to_string());
            setup(&mut repo);

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, cx| host.context_menu_model(&kind, cx))
        })
        .expect("expected a context menu model")
    })
}

fn local_branch_menu(repo_id: RepoId, name: &str) -> PopoverKind {
    PopoverKind::BranchMenu {
        repo_id,
        target: BranchMenuTarget::local(name),
    }
}

/// An unfocused ref offers "Focus".
#[gpui::test]
fn local_branch_menu_focuses_that_branch(cx: &mut gpui::TestAppContext) {
    let model = menu_with_focus(
        cx,
        HistoryFocusSet::default(),
        local_branch_menu(RepoId(7), "feature"),
    );

    let (label, action) = focus_entry(&model);
    assert_eq!(label, "Focus");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistoryFocus {
            repo_id: RepoId(7),
            target: HistoryFocus::LocalBranch { ref name },
        } if name.as_ref() == "feature"
    ));
}

/// A focused ref offers "Stop focus", and sends the same toggle -- which is
/// what takes it back out of the set.
#[gpui::test]
fn the_focused_branchs_own_entry_stops_its_focus(cx: &mut gpui::TestAppContext) {
    let model = menu_with_focus(
        cx,
        HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]),
        local_branch_menu(RepoId(7), "feature"),
    );

    let (label, action) = focus_entry(&model);
    assert_eq!(label, "Stop focus");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistoryFocus {
            repo_id: RepoId(7),
            target: HistoryFocus::LocalBranch { ref name },
        } if name.as_ref() == "feature"
    ));
}

/// A focus on one ref must not make every other ref's entry read as active.
#[gpui::test]
fn another_branchs_entry_still_offers_focus(cx: &mut gpui::TestAppContext) {
    let model = menu_with_focus(
        cx,
        HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]),
        local_branch_menu(RepoId(7), "main"),
    );

    let (label, action) = focus_entry(&model);
    assert_eq!(label, "Focus");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistoryFocus {
            target: HistoryFocus::LocalBranch { ref name },
            ..
        } if name.as_ref() == "main"
    ));
}

#[gpui::test]
fn remote_branch_menu_focuses_the_remote_tracking_ref(cx: &mut gpui::TestAppContext) {
    let model = menu_with_focus(
        cx,
        HistoryFocusSet::default(),
        PopoverKind::BranchMenu {
            repo_id: RepoId(7),
            target: BranchMenuTarget::remote("origin", "feature/awesome"),
        },
    );

    let (label, action) = focus_entry(&model);
    assert_eq!(label, "Focus");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistoryFocus {
            target: HistoryFocus::RemoteBranch {
                ref remote,
                ref branch,
            },
            ..
        } if remote.as_ref() == "origin" && branch.as_ref() == "feature/awesome"
    ));
}

#[gpui::test]
fn remote_menu_focuses_the_whole_remote(cx: &mut gpui::TestAppContext) {
    let model = menu_with_focus(
        cx,
        HistoryFocusSet::default(),
        PopoverKind::remote(
            RepoId(7),
            RemotePopoverKind::Menu {
                name: "origin".to_string(),
            },
        ),
    );

    let (label, action) = focus_entry(&model);
    assert_eq!(label, "Focus");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistoryFocus {
            target: HistoryFocus::Remote { ref name },
            ..
        } if name.as_ref() == "origin"
    ));
}

/// Several refs can be focused at once, and each one's entry reads its own
/// state -- the set is what the menu reads, not a single held target. A branch
/// under a focused remote is in view, so it reads as focused too.
#[gpui::test]
fn every_ref_the_focus_covers_offers_stop_focus(cx: &mut gpui::TestAppContext) {
    let focus = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("feature"),
        HistoryFocus::remote("origin"),
    ]);
    let remote_menu = PopoverKind::remote(
        RepoId(7),
        RemotePopoverKind::Menu {
            name: "origin".to_string(),
        },
    );
    let covered_remote_branch = PopoverKind::BranchMenu {
        repo_id: RepoId(7),
        target: BranchMenuTarget::remote("origin", "main"),
    };
    for (kind, expected) in [
        (local_branch_menu(RepoId(7), "feature"), "Stop focus"),
        (remote_menu, "Stop focus"),
        (covered_remote_branch, "Stop focus"),
        (local_branch_menu(RepoId(7), "main"), "Focus"),
    ] {
        let model = menu_with_focus(cx, focus.clone(), kind);
        let (label, _) = focus_entry(&model);
        assert_eq!(label, expected);
    }
}

/// The group menu's focus entries as `(label, focused)` pairs.
fn group_focus_entries(model: &ContextMenuModel) -> Vec<(String, bool)> {
    model
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } => match **action {
                ContextMenuAction::SetBranchGroupFocus { focused, .. } => {
                    Some((label.to_string(), focused))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The fixture's `feat/` group holds `feat/a` and `feat/b/c`.
fn feat_group_with_focus(
    cx: &mut gpui::TestAppContext,
    focus: HistoryFocusSet,
) -> Vec<(String, bool)> {
    group_focus_entries(&super::branch::branch_group_menu_model(
        cx,
        BranchSection::Local,
        None,
        "feat",
        move |repo| repo.history_state.history_focus = focus,
    ))
}

#[gpui::test]
fn an_unfocused_group_offers_only_focus_group(cx: &mut gpui::TestAppContext) {
    assert_eq!(
        feat_group_with_focus(cx, HistoryFocusSet::default()),
        vec![("Focus group".to_string(), true)]
    );
}

/// A partly focused group can go either way, so it offers both.
#[gpui::test]
fn a_partly_focused_group_offers_both_entries(cx: &mut gpui::TestAppContext) {
    assert_eq!(
        feat_group_with_focus(
            cx,
            HistoryFocusSet::from_iter([HistoryFocus::local_branch("feat/a")]),
        ),
        vec![
            ("Focus group".to_string(), true),
            ("Stop focusing group".to_string(), false),
        ]
    );
}

#[gpui::test]
fn a_fully_focused_group_offers_only_stop_focusing(cx: &mut gpui::TestAppContext) {
    assert_eq!(
        feat_group_with_focus(
            cx,
            HistoryFocusSet::from_iter([
                HistoryFocus::local_branch("feat/a"),
                HistoryFocus::local_branch("feat/b/c"),
                // Outside the group; must not change what the group offers.
                HistoryFocus::local_branch("features/x"),
            ]),
        ),
        vec![("Stop focusing group".to_string(), false)]
    );
}

/// A remote group under a focused remote is already in view.
#[gpui::test]
fn a_remote_group_under_a_focused_remote_reads_as_focused(cx: &mut gpui::TestAppContext) {
    let model = super::branch::branch_group_menu_model(
        cx,
        BranchSection::Remote,
        Some("origin"),
        "feat",
        |repo| {
            repo.history_state.history_focus =
                HistoryFocusSet::from_iter([HistoryFocus::remote("origin")])
        },
    );
    assert_eq!(
        group_focus_entries(&model),
        vec![("Stop focusing group".to_string(), false)]
    );
}

/// The menu entries wear the sidebar's eyes: open to focus, crossed out to
/// stop, so the menu and the eye column read as the same control.
#[gpui::test]
fn focus_entries_carry_the_sidebar_eye_icons(cx: &mut gpui::TestAppContext) {
    fn icon_of<'a>(model: &'a ContextMenuModel, wanted: &str) -> Option<&'a str> {
        model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, icon, .. } if label.as_ref() == wanted => {
                icon.as_ref().map(|icon| icon.as_ref())
            }
            _ => None,
        })
    }

    let unfocused = menu_with_focus(
        cx,
        HistoryFocusSet::default(),
        local_branch_menu(RepoId(7), "feature"),
    );
    assert_eq!(icon_of(&unfocused, "Focus"), Some("icons/eye.svg"));

    let focused = menu_with_focus(
        cx,
        HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]),
        local_branch_menu(RepoId(7), "feature"),
    );
    assert_eq!(icon_of(&focused, "Stop focus"), Some("icons/eye_off.svg"));

    let group =
        super::branch::branch_group_menu_model(cx, BranchSection::Local, None, "feat", |repo| {
            repo.history_state.history_focus =
                HistoryFocusSet::from_iter([HistoryFocus::local_branch("feat/a")])
        });
    assert_eq!(icon_of(&group, "Focus group"), Some("icons/eye.svg"));
    assert_eq!(
        icon_of(&group, "Stop focusing group"),
        Some("icons/eye_off.svg")
    );
}

fn tag_menu(name: Option<&str>) -> PopoverKind {
    PopoverKind::TagMenu {
        repo_id: RepoId(7),
        commit_id: CommitId("a".repeat(40).into()),
        name: name.map(str::to_string),
    }
}

/// A tag's menu toggles that tag, and reads its state from the tag's own entry:
/// a branch with the same name being focused does not count.
#[gpui::test]
fn a_tag_menu_focuses_and_stops_focusing_that_tag(cx: &mut gpui::TestAppContext) {
    for (focus, expected) in [
        (HistoryFocusSet::default(), "Focus"),
        (
            HistoryFocusSet::from_iter([HistoryFocus::local_branch("v1.0")]),
            "Focus",
        ),
        (
            HistoryFocusSet::from_iter([HistoryFocus::tag("v1.0")]),
            "Stop focus",
        ),
    ] {
        let model = menu_with_focus(cx, focus, tag_menu(Some("v1.0")));
        let (label, action) = focus_entry(&model);
        assert_eq!(label, expected);
        assert!(matches!(
            action,
            ContextMenuAction::ToggleHistoryFocus {
                repo_id: RepoId(7),
                target: HistoryFocus::Tag { ref name },
            } if name.as_ref() == "v1.0"
        ));
    }
}

/// A commit with several tags gives each its own entry, named, since a bare
/// "Focus" would not say which tag it means.
#[gpui::test]
fn a_commit_with_several_tags_offers_a_named_entry_per_tag(cx: &mut gpui::TestAppContext) {
    let commit_id = CommitId("a".repeat(40).into());
    let model = menu_with_repo(
        cx,
        |repo| {
            repo.tags = Loadable::Ready(Arc::new(
                ["v1.0", "release"]
                    .map(|name| gitcomet_core::domain::Tag {
                        name: name.to_string(),
                        target: commit_id.clone(),
                    })
                    .to_vec(),
            ));
            repo.history_state.history_focus =
                HistoryFocusSet::from_iter([HistoryFocus::tag("v1.0")]);
        },
        tag_menu(None),
    );

    let entries = model
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } => match &**action {
                ContextMenuAction::ToggleHistoryFocus {
                    target: HistoryFocus::Tag { name },
                    ..
                } => Some((label.to_string(), name.to_string())),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        vec![
            ("Focus tag release".to_string(), "release".to_string()),
            ("Stop focusing tag v1.0".to_string(), "v1.0".to_string()),
        ]
    );
}
