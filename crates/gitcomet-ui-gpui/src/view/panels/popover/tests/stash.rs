use super::*;

#[gpui::test]
fn stash_menu_has_apply_pop_and_drop_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(8);
    let index = 3usize;
    let message = "WIP".to_string();

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::StashMenu {
                            repo_id,
                            index,
                            message: message.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected stash context menu model");

        let apply_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Apply stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            apply_action,
            Some(ContextMenuAction::ApplyStash {
                repo_id: rid,
                index: ix
            }) if rid == repo_id && ix == index
        ));

        let pop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Pop stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            pop_action,
            Some(ContextMenuAction::PopStash {
                repo_id: rid,
                index: ix
            }) if rid == repo_id && ix == index
        ));

        let drop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Drop stash…" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            drop_action,
            Some(ContextMenuAction::DropStashConfirm {
                repo_id: rid,
                index: ix,
                message: ref msg
            }) if rid == repo_id && ix == index && msg == &message
        ));
    });
}

#[gpui::test]
fn stash_menu_drop_action_opens_drop_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(9);
    let index = 1usize;
    let message = "Drop me".to_string();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.popover_anchor = Some(PopoverAnchor::Point(point(px(32.0), px(48.0))));
                host.context_menu_activate_action(
                    ContextMenuAction::DropStashConfirm {
                        repo_id,
                        index,
                        message: message.clone(),
                    },
                    window,
                    cx,
                );

                match host.popover.as_ref() {
                    Some(PopoverKind::StashDropConfirm {
                        repo_id: rid,
                        index: ix,
                        message: msg,
                    }) => {
                        assert_eq!(*rid, repo_id);
                        assert_eq!(*ix, index);
                        assert_eq!(msg, &message);
                    }
                    _ => panic!("expected stash drop confirm popover to open"),
                }
            });
        });
    });
}
