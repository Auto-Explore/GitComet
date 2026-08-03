use super::*;

use crate::view::shortcut_labels::Shortcut;

fn push_entry(
    items: &mut Vec<ContextMenuItem>,
    debug_selectors: &mut std::collections::HashMap<usize, SharedString>,
    debug_selector: &'static str,
    label: &'static str,
    shortcut: Shortcut,
    disabled: bool,
    action: AppMenuAction,
) {
    let ix = items.len();
    items.push(ContextMenuItem::Entry {
        label: label.into(),
        icon: None,
        shortcut: shortcut.label().map(SharedString::from),
        disabled,
        action: Box::new(ContextMenuAction::AppMenu(action)),
    });
    debug_selectors.insert(ix, debug_selector.into());
}

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let active_repo_id = this.active_repo().map(|repo| repo.id);
    let active_repo_workdir = this.active_repo().map(|repo| repo.spec.workdir.clone());
    let external_editor_configured = crate::external_editor::configured_setting().is_some();
    let show_command_palette = command_palette_available(this.root_view_mode);

    let mut items = vec![ContextMenuItem::Header("Application".into())];
    let mut debug_selectors = std::collections::HashMap::new();

    if show_command_palette {
        push_entry(
            &mut items,
            &mut debug_selectors,
            "app_menu_command_palette",
            "Command Palette",
            Shortcut::Secondary("P"),
            false,
            AppMenuAction::CommandPalette,
        );
    }
    push_entry(
        &mut items,
        &mut debug_selectors,
        "app_menu_settings",
        "Settings…",
        Shortcut::Secondary(","),
        false,
        AppMenuAction::Settings,
    );
    if external_editor_configured {
        push_entry(
            &mut items,
            &mut debug_selectors,
            "app_menu_open_in_code_editor",
            "Open in code editor",
            Shortcut::Secondary("Shift+E"),
            active_repo_workdir.is_none(),
            AppMenuAction::OpenInCodeEditor {
                path: active_repo_workdir,
            },
        );
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Header("Patches".into()));
    push_entry(
        &mut items,
        &mut debug_selectors,
        "app_menu_apply_patch",
        "Apply patch…",
        Shortcut::None,
        active_repo_id.is_none(),
        AppMenuAction::ApplyPatch {
            repo_id: active_repo_id,
        },
    );
    items.push(ContextMenuItem::Separator);

    // Only platforms with a real desktop-entry story get the row at all; a
    // permanently inert entry is noise everywhere else.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    push_entry(
        &mut items,
        &mut debug_selectors,
        "app_menu_install_desktop",
        "Install desktop integration",
        Shortcut::None,
        false,
        AppMenuAction::InstallDesktopIntegration,
    );

    push_entry(
        &mut items,
        &mut debug_selectors,
        "app_menu_quit",
        "Quit",
        Shortcut::Secondary("Q"),
        false,
        AppMenuAction::Quit,
    );
    push_entry(
        &mut items,
        &mut debug_selectors,
        "app_menu_close_window",
        "Close Window",
        Shortcut::Secondary("Shift+W"),
        false,
        AppMenuAction::CloseWindow,
    );

    ContextMenuModel::new(items)
        .with_shortcut_keycaps()
        .with_entry_debug_selectors(debug_selectors)
}

pub(super) fn activate(
    this: &mut PopoverHost,
    action: AppMenuAction,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match action {
        AppMenuAction::CommandPalette => {
            this.close_popover_and_restore_focus(window, cx);
            window.dispatch_action(Box::new(ToggleCommandPalette), cx);
        }
        AppMenuAction::Settings => {
            this.close_popover_and_restore_focus(window, cx);
            cx.defer(crate::view::open_settings_window);
        }
        AppMenuAction::OpenInCodeEditor { path } => {
            if let Some(path) = path {
                let _ = this.root_view.update(cx, |root, cx| {
                    root.open_path_in_external_code_editor(path, cx);
                });
            }
            this.close_popover_and_restore_focus(window, cx);
        }
        AppMenuAction::ApplyPatch { repo_id } => {
            let Some(repo_id) = repo_id else {
                return;
            };
            cx.stop_propagation();
            this.close_popover_and_restore_focus(window, cx);
            let view = cx.weak_entity();
            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Select patch file".into()),
            });
            window
                .spawn(cx, async move |cx| {
                    let paths = match rx.await {
                        Ok(Ok(Some(paths))) => paths,
                        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => return,
                    };
                    let Some(patch) = paths.into_iter().next() else {
                        return;
                    };
                    let _ = view.update(cx, |this, cx| {
                        this.store.dispatch(Msg::ApplyPatch { repo_id, patch });
                        cx.notify();
                    });
                })
                .detach();
        }
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        AppMenuAction::InstallDesktopIntegration => {
            this.install_linux_desktop_integration(cx);
            this.close_popover_and_restore_focus(window, cx);
        }
        AppMenuAction::Quit => {
            this.close_popover_and_restore_focus(window, cx);
            crate::app::quit_app_or_warn(cx);
        }
        AppMenuAction::CloseWindow => {
            this.close_popover_and_restore_focus(window, cx);
            crate::app::close_window_or_warn(window, cx);
        }
    }
}
