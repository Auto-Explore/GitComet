use super::*;
use gitcomet_core::domain::{Branch, Remote, RemoteBranch, RepoSpec, Tag};
use gitcomet_core::tag_push::TagPushMode;

fn ref_repo() -> RepoState {
    let commit = CommitId("0123456789abcdef0123456789abcdef01234567".into());
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/combined-refs".into(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "main".into(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "release".into(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "team/alice".into(),
        name: "release".into(),
        target: commit.clone(),
    }]));
    repo.tags = Loadable::Ready(Arc::new(vec![Tag {
        name: "release".into(),
        target: commit.clone(),
    }]));
    repo.remote_tags = Loadable::Ready(Arc::new(vec![]));
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".into(),
        url: None,
    }]));
    repo
}

#[gpui::test]
fn combined_history_ref_groups_keep_exact_targets_and_toggle_with_keyboard(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo = ref_repo();
    let commit_id = repo.branches.ready().unwrap()[0].target.clone();
    let state = Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..Default::default()
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.state = state.clone();
            this.ui_model.update(cx, |model, cx| model.set_state(state.clone(), cx));
            this.popover_host.update(cx, |host, cx| {
                host.state = state.clone();
                host.open_popover_at(PopoverKind::CommitMenu { repo_id: RepoId(1), commit_id: commit_id.clone() }, point(px(10.0), px(10.0)), window, cx);
                let model = host.context_menu_model(host.popover.as_ref().unwrap(), cx).unwrap();
                let groups = model.items.iter().filter_map(|item| match item {
                    ContextMenuItem::Entry { label, action, .. } if matches!(action.as_ref(), ContextMenuAction::ToggleHistoryRefGroup { .. }) => Some(label.as_ref()), _ => None,
                }).collect::<Vec<_>>();
                assert_eq!(groups, ["Local branch main", "Local branch release", "Remote branch team/alice/release", "Tag release"]);
                host.context_menu_selected_ix = model.items.iter().position(|item| matches!(item, ContextMenuItem::Entry { label, .. } if label == "Local branch release"));
            });
        });
        let _ = window.draw(app);
    });
    simulate_key_press(cx, "right");
    cx.update(|_, app| {
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.expanded_history_ref,
            Some(HistoryMenuRef::Branch(BranchMenuTarget::local("release")))
        );
    });
    simulate_key_press(cx, "left");
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .popover_host
                .read(app)
                .expanded_history_ref
                .is_none()
        )
    });
    simulate_key_press(cx, "space");
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .popover_host
                .read(app)
                .expanded_history_ref
                .is_some()
        )
    });
    simulate_key_press(cx, "escape");
    cx.update(|_, app| assert!(view.read(app).popover_host.read(app).popover.is_none()));
}

#[gpui::test]
fn tag_push_menu_keeps_actions_enabled_when_preview_is_unavailable_and_preserves_upstream_mode(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut repo = ref_repo();
    let request = super::super::tag_push::request(&repo, TagPushMode::All).unwrap();
    repo.tag_push_previews[1] = Some(gitcomet_state::model::TagPushPreviewState {
        request,
        generation: 1,
        cancellation: gitcomet_core::services::CancellationToken::new(),
        result: Loadable::Error("authentication required".into()),
    });
    let state = Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..Default::default()
    });
    cx.update(|window, app| view.update(app, |this, cx| {
        this.state = state.clone();
        this.popover_host.update(cx, |host, cx| {
            host.state = state;
            let model = host.context_menu_model(&PopoverKind::PushPicker, cx).unwrap();
            assert!(model.items.iter().any(|item| matches!(item, ContextMenuItem::Entry { label, disabled: false, action, .. }
                if label.contains("Preview unavailable") && matches!(action.as_ref(), ContextMenuAction::PushWithTags { mode: TagPushMode::All, .. }))));
            host.context_menu_activate_action(ContextMenuAction::PushWithTags { repo_id: RepoId(1), mode: TagPushMode::All }, window, cx);
            assert_eq!(host.push_upstream_tag_mode, Some(TagPushMode::All));
            assert!(matches!(host.popover, Some(PopoverKind::PushSetUpstreamPrompt { .. })));
            host.close_popover(cx);
            assert!(host.push_upstream_tag_mode.is_none(), "tag mode must not stick to later pushes");
        });
    }));
}
