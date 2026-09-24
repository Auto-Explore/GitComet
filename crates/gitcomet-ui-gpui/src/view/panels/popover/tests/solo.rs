use super::*;
use gitcomet_core::domain::{HistorySolo, HistorySoloSet};

/// The solo entry's label, whether it is ticked, and the action it sends.
fn solo_entry(model: &ContextMenuModel) -> (String, bool, ContextMenuAction) {
    model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                icon,
                action,
                ..
            } if matches!(**action, ContextMenuAction::ToggleHistorySolo { .. }) => Some((
                label.to_string(),
                icon.as_ref()
                    .is_some_and(|icon| icon.as_ref() == "icons/check.svg"),
                (**action).clone(),
            )),
            _ => None,
        })
        .expect("expected a solo entry in the menu")
}

/// Builds a one-repo app state soloed on `solo`, then reads the menu `kind`
/// produces.
fn menu_with_solo(
    cx: &mut gpui::TestAppContext,
    solo: HistorySoloSet,
    kind: PopoverKind,
) -> ContextMenuModel {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(7);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_solo_menu", std::process::id()));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.head_branch = Loadable::Ready("main".to_string());
            repo.history_state.history_solo = solo;

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

/// The entry never renames itself: it reads "Solo" whether or not the ref is
/// soloed, and the tick is what says which.
#[gpui::test]
fn local_branch_menu_solos_that_branch(cx: &mut gpui::TestAppContext) {
    let model = menu_with_solo(
        cx,
        HistorySoloSet::default(),
        local_branch_menu(RepoId(7), "feature"),
    );

    let (label, ticked, action) = solo_entry(&model);
    assert_eq!(label, "Solo");
    assert!(!ticked, "an unsoloed ref must not be ticked");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistorySolo {
            repo_id: RepoId(7),
            target: HistorySolo::LocalBranch { ref name },
        } if name.as_ref() == "feature"
    ));
}

/// The soloed ref's own entry is ticked, and sends the same toggle -- which is
/// what takes it back out of the set.
#[gpui::test]
fn the_soloed_branchs_own_entry_is_ticked(cx: &mut gpui::TestAppContext) {
    let model = menu_with_solo(
        cx,
        HistorySoloSet::from_iter([HistorySolo::local_branch("feature")]),
        local_branch_menu(RepoId(7), "feature"),
    );

    let (label, ticked, action) = solo_entry(&model);
    assert_eq!(label, "Solo");
    assert!(ticked, "the soloed ref's entry must be ticked");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistorySolo {
            repo_id: RepoId(7),
            target: HistorySolo::LocalBranch { ref name },
        } if name.as_ref() == "feature"
    ));
}

/// A solo on one ref must not make every other ref's entry read as active.
#[gpui::test]
fn another_branchs_entry_is_not_ticked(cx: &mut gpui::TestAppContext) {
    let model = menu_with_solo(
        cx,
        HistorySoloSet::from_iter([HistorySolo::local_branch("feature")]),
        local_branch_menu(RepoId(7), "main"),
    );

    let (label, ticked, action) = solo_entry(&model);
    assert_eq!(label, "Solo");
    assert!(!ticked);
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistorySolo {
            target: HistorySolo::LocalBranch { ref name },
            ..
        } if name.as_ref() == "main"
    ));
}

#[gpui::test]
fn remote_branch_menu_solos_the_remote_tracking_ref(cx: &mut gpui::TestAppContext) {
    let model = menu_with_solo(
        cx,
        HistorySoloSet::default(),
        PopoverKind::BranchMenu {
            repo_id: RepoId(7),
            target: BranchMenuTarget::remote("origin", "feature/awesome"),
        },
    );

    let (label, _, action) = solo_entry(&model);
    assert_eq!(label, "Solo");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistorySolo {
            target: HistorySolo::RemoteBranch {
                ref remote,
                ref branch,
            },
            ..
        } if remote.as_ref() == "origin" && branch.as_ref() == "feature/awesome"
    ));
}

#[gpui::test]
fn remote_menu_solos_the_whole_remote(cx: &mut gpui::TestAppContext) {
    let model = menu_with_solo(
        cx,
        HistorySoloSet::default(),
        PopoverKind::remote(
            RepoId(7),
            RemotePopoverKind::Menu {
                name: "origin".to_string(),
            },
        ),
    );

    let (label, _, action) = solo_entry(&model);
    assert_eq!(label, "Solo");
    assert!(matches!(
        action,
        ContextMenuAction::ToggleHistorySolo {
            target: HistorySolo::Remote { ref name },
            ..
        } if name.as_ref() == "origin"
    ));
}

/// Several refs can be soloed at once, and each one's entry carries its own
/// tick -- the set is what the menu reads, not a single held target.
#[gpui::test]
fn every_soloed_ref_in_a_set_is_ticked(cx: &mut gpui::TestAppContext) {
    let solo = HistorySoloSet::from_iter([
        HistorySolo::local_branch("feature"),
        HistorySolo::remote("origin"),
    ]);
    let remote_menu = PopoverKind::remote(
        RepoId(7),
        RemotePopoverKind::Menu {
            name: "origin".to_string(),
        },
    );
    for (kind, expect_ticked) in [
        (local_branch_menu(RepoId(7), "feature"), true),
        (remote_menu, true),
        (local_branch_menu(RepoId(7), "main"), false),
    ] {
        let model = menu_with_solo(cx, solo.clone(), kind);
        let (_, ticked, _) = solo_entry(&model);
        assert_eq!(ticked, expect_ticked);
    }
}
