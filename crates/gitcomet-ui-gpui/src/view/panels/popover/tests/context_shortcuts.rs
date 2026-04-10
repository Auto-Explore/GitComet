use super::*;
use gitcomet_core::domain::RepoSpec;

fn assert_first_entry_has_no_enter_shortcut(model: &ContextMenuModel, expected_label: &str) {
    let (label, shortcut) = model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label, shortcut, ..
            } => Some((label.as_ref(), shortcut.as_ref().map(|s| s.as_ref()))),
            _ => None,
        })
        .expect("expected context menu to contain an entry");

    assert_eq!(label, expected_label);
    assert_eq!(shortcut, None);
    assert!(
        !model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry {
                shortcut: Some(shortcut),
                ..
            } if shortcut.as_ref() == "Enter"
        )),
        "context menu should not advertise Enter as a row shortcut"
    );
}

#[gpui::test]
fn context_menu_default_actions_do_not_render_enter_shortcuts(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(42);
    let commit_id = CommitId("0123456789abcdef".into());
    let path = std::path::PathBuf::from("src/lib.rs");
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_shortcuts",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(repo_id, RepoSpec { workdir });
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(RepoStatus::default().into());

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this._ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    let pull_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(&PopoverKind::PullPicker, cx)
                })
            })
        })
        .expect("expected pull context menu model");
    assert_first_entry_has_no_enter_shortcut(&pull_model, "Pull (default)");

    let push_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(&PopoverKind::PushPicker, cx)
                })
            })
        })
        .expect("expected push context menu model");
    assert_first_entry_has_no_enter_shortcut(&push_model, "Push");

    let branch_section_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::BranchSectionMenu {
                            repo_id,
                            section: BranchSection::Local,
                        },
                        cx,
                    )
                })
            })
        })
        .expect("expected branch section context menu model");
    assert_first_entry_has_no_enter_shortcut(&branch_section_model, "Switch branch");

    let branch_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Local,
                            name: "feature".to_string(),
                        },
                        cx,
                    )
                })
            })
        })
        .expect("expected branch context menu model");
    assert_first_entry_has_no_enter_shortcut(&branch_model, "Checkout");

    let commit_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                        cx,
                    )
                })
            })
        })
        .expect("expected commit context menu model");
    assert_first_entry_has_no_enter_shortcut(&commit_model, "Open diff");

    let commit_file_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitFileMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
        })
        .expect("expected commit file context menu model");
    assert_first_entry_has_no_enter_shortcut(&commit_file_model, "Open diff");

    let status_file_model = cx
        .update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::StatusFileMenu {
                            repo_id,
                            area: DiffArea::Unstaged,
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
        })
        .expect("expected status file context menu model");
    assert_first_entry_has_no_enter_shortcut(&status_file_model, "Open diff");
}
