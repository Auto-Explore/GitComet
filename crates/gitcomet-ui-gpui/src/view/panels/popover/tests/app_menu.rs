use super::*;

fn open_app_menu(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::AppMenu,
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
    (view, cx)
}

#[gpui::test]
fn app_menu_offers_close_window_rather_than_a_dismiss_entry(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    assert!(
        cx.debug_bounds("app_menu_close_window").is_some(),
        "app menu should offer Close Window"
    );
    assert!(
        cx.debug_bounds("app_menu_quit").is_some(),
        "app menu should offer Quit"
    );
    assert!(
        cx.debug_bounds("app_menu_close").is_none(),
        "the old popover-dismiss entry should be gone"
    );
}

#[gpui::test]
fn app_menu_places_quit_after_close_window(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);
    let close_window = cx
        .debug_bounds("app_menu_close_window")
        .expect("app menu should offer Close Window");
    let quit = cx
        .debug_bounds("app_menu_quit")
        .expect("app menu should offer Quit");

    assert!(
        quit.top() >= close_window.bottom(),
        "Quit should be the final row after Close Window"
    );
}

#[gpui::test]
fn app_menu_hides_desktop_integration_on_unsupported_platforms(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    let entry = cx.debug_bounds("app_menu_install_desktop");
    if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        assert!(
            entry.is_some(),
            "desktop integration should be offered where it is implemented"
        );
    } else {
        assert!(
            entry.is_none(),
            "desktop integration should not render where it cannot run"
        );
    }
}

#[gpui::test]
fn app_menu_shortcuts_use_command_palette_keycaps(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    assert!(
        cx.debug_bounds("shortcut_keycaps").is_some(),
        "App-menu shortcuts should use the shared Command Palette keycap badges"
    );
}

#[gpui::test]
fn app_menu_owns_keyboard_focus_and_escape_dismisses_it(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_app_menu(cx);

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.context_menu_focus_handle.is_focused(window),
            "opening the app menu should move keyboard focus into the menu"
        );
        assert_eq!(
            host.context_menu_selected_ix,
            Some(0),
            "the first application action should be selected"
        );
    });

    simulate_key_press(cx, "tab");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .popover_host
                .read(app)
                .context_menu_selected_ix,
            Some(1),
            "Tab should select Settings"
        );
    });

    simulate_key_press(cx, "shift-tab");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .popover_host
                .read(app)
                .context_menu_selected_ix,
            Some(0),
            "Shift+Tab should select Command Palette"
        );
    });

    simulate_key_press(cx, "escape");
    cx.update(|_window, app| {
        assert!(
            !view.read(app).popover_host.read(app).is_open(),
            "Escape should dismiss the app menu"
        );
    });
}

#[gpui::test]
fn app_menu_offers_the_reflog_panel(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    let reflog = cx
        .debug_bounds("app_menu_show_reflog")
        .expect("app menu should offer Reflog");
    let locate = cx
        .debug_bounds("app_menu_locate_file")
        .expect("app menu should offer Show file in explorer");
    let apply_patch = cx
        .debug_bounds("app_menu_apply_patch")
        .expect("app menu should offer Apply patch");

    // Grouped with the other repository-scoped views, above the separator that
    // starts the file-operation block.
    assert!(
        reflog.top() >= locate.bottom(),
        "Reflog should follow the file-explorer entry"
    );
    assert!(
        reflog.bottom() <= apply_patch.top(),
        "Reflog should sit above Apply patch"
    );
}

#[gpui::test]
fn app_menu_offers_open_remote_beside_the_file_explorer(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);
    let locate = cx
        .debug_bounds("app_menu_locate_file")
        .expect("app menu should offer Open in file explorer");
    let open_remote = cx
        .debug_bounds("app_menu_open_remote_in_browser")
        .expect("app menu should offer Open remote in web browser");
    let reflog = cx
        .debug_bounds("app_menu_show_reflog")
        .expect("app menu should offer Reflog");

    assert!(open_remote.top() >= locate.bottom());
    assert!(open_remote.bottom() <= reflog.top());
}

/// A view whose active repository has these remotes, with the app shortcuts
/// installed so a dispatched `OpenRemoteInBrowser` has somewhere to land.
fn app_menu_view_with_remotes(
    cx: &mut gpui::TestAppContext,
    remotes: Vec<gitcomet_core::domain::Remote>,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    app_menu_view_with(cx, Loadable::Ready(Arc::new(remotes)))
}

fn app_menu_view_with(
    cx: &mut gpui::TestAppContext,
    remotes: Loadable<Arc<Vec<gitcomet_core::domain::Remote>>>,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(23);
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, backend);
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: std::env::temp_dir().join("gitcomet_ui_test_app_menu_remote"),
                },
            );
            repo.open = Loadable::Ready(());
            repo.remotes = remotes;
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
        });
        let _ = window.draw(app);
    });
    (view, cx)
}

fn open_remote_entry_disabled(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
) -> Option<bool> {
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        super::super::app_menu::model(host)
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry {
                    label, disabled, ..
                } if label.as_ref() == crate::menu_labels::OPEN_REMOTE_IN_BROWSER => {
                    Some(*disabled)
                }
                _ => None,
            })
    })
}

fn web_remote(name: &str, url: &str) -> gitcomet_core::domain::Remote {
    gitcomet_core::domain::Remote {
        name: name.to_string(),
        url: Some(url.to_string()),
    }
}

#[gpui::test]
fn app_menu_disables_open_remote_without_a_repository(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_app_menu(cx);
    assert_eq!(open_remote_entry_disabled(cx, &view), Some(true));
}

#[gpui::test]
fn app_menu_disables_open_remote_while_remotes_load(cx: &mut gpui::TestAppContext) {
    let (view, cx) = app_menu_view_with(cx, Loadable::Loading);
    assert_eq!(open_remote_entry_disabled(cx, &view), Some(true));
}

#[gpui::test]
fn app_menu_disables_open_remote_without_remotes(cx: &mut gpui::TestAppContext) {
    let (view, cx) = app_menu_view_with_remotes(cx, Vec::new());
    assert_eq!(open_remote_entry_disabled(cx, &view), Some(true));
}

#[gpui::test]
fn app_menu_open_remote_shows_the_chord(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_app_menu(cx);
    let shortcut = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        super::super::app_menu::model(host)
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry {
                    label, shortcut, ..
                } if label.as_ref() == crate::menu_labels::OPEN_REMOTE_IN_BROWSER => {
                    shortcut.clone()
                }
                _ => None,
            })
    });
    let expected = if cfg!(target_os = "macos") {
        "Cmd+K"
    } else {
        "Ctrl+K"
    };
    assert_eq!(shortcut.as_deref(), Some(expected));
}

#[gpui::test]
fn app_menu_disables_open_remote_without_a_web_page(cx: &mut gpui::TestAppContext) {
    let (view, cx) =
        app_menu_view_with_remotes(cx, vec![web_remote("origin", "/srv/git/repo.git")]);
    assert_eq!(open_remote_entry_disabled(cx, &view), Some(true));
}

#[gpui::test]
fn app_menu_enables_open_remote_with_a_web_page(cx: &mut gpui::TestAppContext) {
    let (view, cx) = app_menu_view_with_remotes(
        cx,
        vec![web_remote("origin", "git@github.com:org/repo.git")],
    );
    assert_eq!(open_remote_entry_disabled(cx, &view), Some(false));
}

#[gpui::test]
fn app_menu_open_remote_hands_off_to_the_remote_picker(cx: &mut gpui::TestAppContext) {
    // Two web remotes, so the click can only open the picker, never a browser.
    let (view, cx) = app_menu_view_with_remotes(
        cx,
        vec![
            web_remote("origin", "git@github.com:org/repo.git"),
            web_remote("upstream", "https://gitlab.com/upstream/repo.git"),
        ],
    );
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::AppMenu,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    let center = cx
        .debug_bounds("app_menu_open_remote_in_browser")
        .expect("expected the Open remote row")
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();

    let picker = PopoverKind::remote(RepoId(23), RemotePopoverKind::OpenInBrowserMenu);
    cx.update(|_window, app| {
        assert!(
            view.read(app).popover_host.read(app).is_kind_open(&picker),
            "the app menu row should open the remote picker in its place"
        );
    });
}

#[gpui::test]
fn app_menu_places_update_check_immediately_after_apply_patch(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);
    let apply_patch = cx
        .debug_bounds("app_menu_apply_patch")
        .expect("app menu should offer Apply patch");
    let update_check = cx
        .debug_bounds("app_menu_check_for_updates")
        .expect("app menu should offer Check for updates");

    assert!(
        update_check.top() >= apply_patch.bottom(),
        "Check for updates should immediately follow Apply patch"
    );
}

#[gpui::test]
fn app_menu_disables_update_check_when_environment_override_is_present(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_app_menu(cx);
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let model = super::super::app_menu::model_with_update_checks_disabled(host, true);
        let disabled = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label, disabled, ..
            } if label.as_ref() == crate::menu_labels::CHECK_FOR_UPDATES => Some(*disabled),
            _ => None,
        });
        assert_eq!(disabled, Some(true));
    });
}
