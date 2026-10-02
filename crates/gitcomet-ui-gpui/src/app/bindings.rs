//! App-wide actions and key bindings.

use super::*;

pub(super) fn install_app_actions(cx: &mut App, backend: Arc<dyn GitBackend>) {
    crate::window_focus::observe_tab_navigation(cx).detach();
    install_global_diff_shortcut_fallback(cx);

    let new_window_backend = Arc::clone(&backend);
    cx.on_action(move |_: &NewWindow, cx| {
        let backend = Arc::clone(&new_window_backend);
        cx.defer(move |cx| {
            let launch = normal_empty_launch_config(None);
            open_gitcomet_window(cx, backend, &launch);
            cx.activate(true);
        });
    });

    cx.on_action(|_: &OpenSettings, cx| {
        cx.defer(|cx| {
            crate::view::open_settings_window(cx);
        });
    });

    cx.on_action(|_: &OpenInCodeEditor, cx| {
        cx.defer(|cx| {
            let _ = update_active_or_existing_normal_gitcomet_window(cx, |view, cx| {
                view.open_active_repo_in_external_code_editor(cx);
            });
        });
    });

    let repo_backend = Arc::clone(&backend);
    cx.on_action(move |_: &OpenRepository, cx| {
        let backend = Arc::clone(&repo_backend);
        cx.defer(move |cx| {
            if existing_normal_gitcomet_window_blocks_repository_management_actions(cx) {
                return;
            }
            prompt_open_repository(cx, backend);
        });
    });

    let recent_picker_backend = Arc::clone(&backend);
    cx.on_action(move |_: &SwitchRepository, cx| {
        let backend = Arc::clone(&recent_picker_backend);
        cx.defer(move |cx| {
            if existing_normal_gitcomet_window_blocks_repository_management_actions(cx) {
                return;
            }
            open_repository_switcher_in_existing_or_new_window(cx, backend);
        });
    });
    let workspace_picker_backend = Arc::clone(&backend);
    cx.on_action(move |_: &OpenWorkspace, cx| {
        let backend = Arc::clone(&workspace_picker_backend);
        cx.defer(move |cx| open_workspace_picker_in_existing_or_new_window(cx, backend));
    });
    let command_palette_backend = Arc::clone(&backend);
    cx.on_action(move |_: &ToggleCommandPalette, cx| {
        let backend = Arc::clone(&command_palette_backend);
        cx.defer(move |cx| toggle_command_palette_in_active_existing_or_new_window(cx, backend));
    });
    // Reaches the window even with nothing focused inside it — the same reason
    // the palette needs an app-level handler alongside its window one.
    cx.on_action(|_: &crate::view::ToggleRevealCommit, cx| {
        cx.defer(toggle_reveal_commit_in_active_window);
    });
    cx.on_action(|_: &LocateFileInExplorer, cx| {
        cx.defer(locate_file_in_active_or_existing_normal_window);
    });
    cx.on_action(|_: &OpenRemoteInBrowser, cx| {
        cx.defer(open_remote_in_browser_in_active_window);
    });
    cx.on_action(|_: &ShowReflog, cx| {
        cx.defer(|cx| {
            let _ = update_active_normal_gitcomet_window(cx, |view, cx| {
                view.open_reflog_panel_for_active_repo(cx);
            });
        });
    });

    cx.on_action(|_: &Close, cx| {
        cx.defer(|cx| {
            let handled =
                update_active_normal_gitcomet_window(cx, |view, cx| view.close_active_repo_tab(cx))
                    .unwrap_or(false);
            if !handled {
                close_active_window_or_warn(cx);
            }
        });
    });
    cx.on_action(|_: &CloseWindow, cx| {
        cx.defer(close_active_window_or_warn);
    });
    cx.on_action(|_: &PreviousRepository, cx| {
        cx.defer(|cx| {
            let _ = update_active_normal_gitcomet_window(cx, |view, cx| {
                view.activate_previous_repo_tab(cx)
            });
        });
    });
    cx.on_action(|_: &NextRepository, cx| {
        cx.defer(|cx| {
            let _ = update_active_normal_gitcomet_window(cx, |view, cx| {
                view.activate_next_repo_tab(cx)
            });
        });
    });
    cx.on_action(|_: &MinimizeWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_root, window, _cx| {
                    window.minimize_window();
                });
            }
        });
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_root, window, _cx| {
                    toggle_window_zoom(window);
                });
            }
        });
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_root, window, _cx| {
                    window.toggle_fullscreen();
                });
            }
        });
    });
    // Zoom is per window: each step acts on the window the shortcut came from.
    cx.on_action(|_: &IncreaseUiScale, cx| step_window_ui_scale(cx, ui_scale::step_up));
    cx.on_action(|_: &DecreaseUiScale, cx| step_window_ui_scale(cx, ui_scale::step_down));
    cx.on_action(|_: &ResetUiScale, cx| {
        if let Some(window_id) = zoom_target_window(cx) {
            cx.defer(move |cx| set_window_ui_scale_percent(cx, window_id, None));
        }
    });
    cx.on_window_closed(ui_scale::forget_window).detach();
    cx.on_action(|_: &Hide, cx| cx.defer(|cx| cx.hide()));
    cx.on_action(|_: &HideOthers, cx| cx.defer(|cx| cx.hide_other_apps()));
    cx.on_action(|_: &ShowAll, cx| cx.defer(|cx| cx.unhide_other_apps()));
    cx.on_action(|_: &Quit, cx| cx.defer(quit_app_or_warn));
}

fn step_window_ui_scale(cx: &mut App, step: fn(u32) -> u32) {
    let Some(window_id) = zoom_target_window(cx) else {
        return;
    };
    cx.defer(move |cx| {
        let next = step(ui_scale::percent_for_window(cx, window_id));
        set_window_ui_scale_percent(cx, window_id, Some(next));
    });
}

pub(super) fn install_global_diff_shortcut_fallback(cx: &mut App) {
    cx.observe_keystrokes(|event, window, cx| {
        // Observers also run after a bound action handled the keystroke (gpui
        // passes that action here); acting again would run F3 twice and skip
        // every other change block.
        if event.action.is_some()
            || !is_diff_shortcut_candidate(&event.keystroke)
            || event.context_stack.iter().any(|context| {
                context.contains("TextInput")
                    || context.contains("Terminal")
                    || context.contains("ContextMenu")
                    || context.contains("PopoverPrompt")
                    || (context.contains("StatusSection")
                        && crate::view::is_status_section_shortcut(&event.keystroke))
            })
        {
            return;
        }

        let window_id = window.window_handle().window_id();
        let Some(entry) = cx
            .try_global::<GitCometWindowRegistry>()
            .and_then(|registry| registry.windows.get(&window_id))
            .filter(|entry| entry.diff_fallback_enabled)
            .cloned()
        else {
            return;
        };

        let handled = entry
            .main_pane
            .update(cx, |pane, cx| {
                let handled = pane.handle_diff_shortcut(&event.keystroke, window, cx);
                if handled {
                    cx.notify();
                    window.refresh();
                }
                handled
            })
            .unwrap_or(false);
        if handled {
            cx.stop_propagation();
        }
    })
    .detach();
}

#[cfg(test)]
pub(crate) fn install_global_diff_shortcut_fallback_for_test(cx: &mut App) {
    install_global_diff_shortcut_fallback(cx);
}

pub(super) fn bind_app_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewWindow, None),
        KeyBinding::new("secondary-shift-n", NewWindow, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-o", OpenRepository, None),
        KeyBinding::new("secondary-shift-o", SwitchRepository, None),
        KeyBinding::new("secondary-shift-a", SwitchRepository, None),
        KeyBinding::new("secondary-shift-r", OpenWorkspace, None),
        KeyBinding::new("secondary-f", OpenActiveViewSearch, None),
        KeyBinding::new("secondary-p", ToggleCommandPalette, None),
        KeyBinding::new("secondary-g", crate::view::ToggleRevealCommit, None),
        KeyBinding::new("secondary-shift-l", LocateFileInExplorer, None),
        KeyBinding::new("secondary-k", OpenRemoteInBrowser, None),
        KeyBinding::new("secondary-w", Close, None),
        KeyBinding::new("secondary-shift-w", CloseWindow, None),
        KeyBinding::new("secondary-pageup", PreviousRepository, None),
        KeyBinding::new("secondary-pagedown", NextRepository, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-tab", PreviousRepository, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-tab", NextRepository, None),
        KeyBinding::new("secondary-+", IncreaseUiScale, None),
        KeyBinding::new("secondary-=", IncreaseUiScale, None),
        KeyBinding::new("secondary--", DecreaseUiScale, None),
        KeyBinding::new("secondary-0", ResetUiScale, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("f1", DiffPrevFile, None),
        KeyBinding::new("f4", DiffNextFile, None),
        KeyBinding::new("f2", DiffPrevSearchMatchOrChange, None),
        KeyBinding::new("f3", DiffNextSearchMatchOrChange, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-o", SwitchRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-{", PreviousRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-left", PreviousRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-}", NextRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-right", NextRepository, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("f11", ToggleFullScreen, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-h", Hide, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-h", HideOthers, None),
    ]);
    refresh_external_editor_key_binding(cx);
}

pub(super) fn refresh_external_editor_key_binding(cx: &mut App) {
    refresh_external_editor_key_binding_for_configured(
        cx,
        crate::external_editor::configured_setting().is_some(),
    );
}

pub(super) fn refresh_external_editor_key_binding_for_configured(cx: &mut App, configured: bool) {
    if configured {
        cx.bind_keys([KeyBinding::new("secondary-shift-e", OpenInCodeEditor, None)]);
    } else {
        cx.bind_keys([KeyBinding::new(
            "secondary-shift-e",
            Unbind(OpenInCodeEditor.name().into()),
            None,
        )]);
    }
}

pub(crate) fn refresh_external_editor_app_surfaces_for_setting(
    setting: Option<&session::ExternalCodeEditorSetting>,
    cx: &mut App,
) {
    let configured = setting.is_some_and(crate::external_editor::setting_is_configured);
    refresh_external_editor_key_binding_for_configured(cx, configured);
    #[cfg(target_os = "macos")]
    refresh_macos_app_menus_for_external_editor(cx, configured);
}

pub(super) fn bind_text_input_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "enter",
            PushUpstreamRemoteOpenOrSelect,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "space",
            PushUpstreamRemoteOpenOrSelect,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "up",
            PushUpstreamRemotePrev,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "down",
            PushUpstreamRemoteNext,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new(
            "escape",
            PushUpstreamRemoteClose,
            Some("PushUpstreamRemoteSelector"),
        ),
        KeyBinding::new("escape", PopoverPromptDismiss, Some("PopoverPrompt")),
        KeyBinding::new("tab", PopoverPromptTabNext, Some("PopoverPrompt")),
        KeyBinding::new("shift-tab", PopoverPromptTabPrev, Some("PopoverPrompt")),
        KeyBinding::new("backspace", crate::kit::Backspace, Some("TextInput")),
        KeyBinding::new("shift-backspace", crate::kit::Backspace, Some("TextInput")),
        KeyBinding::new("delete", crate::kit::Delete, Some("TextInput")),
        KeyBinding::new(
            "ctrl-backspace",
            crate::kit::DeleteWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "ctrl-delete",
            crate::kit::DeleteWordRight,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "alt-backspace",
            crate::kit::DeleteWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new("alt-delete", crate::kit::DeleteWordRight, Some("TextInput")),
        KeyBinding::new(
            "cmd-backspace",
            crate::kit::DeleteToLineStart,
            Some("TextInput"),
        ),
        KeyBinding::new("cmd-delete", crate::kit::DeleteToLineEnd, Some("TextInput")),
        // The Windows/Linux counterpart, as in GTK text views. Plain
        // Ctrl-Backspace/Delete already delete a word there.
        KeyBinding::new(
            "ctrl-shift-backspace",
            crate::kit::DeleteToLineStart,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "ctrl-shift-delete",
            crate::kit::DeleteToLineEnd,
            Some("TextInput"),
        ),
        KeyBinding::new("enter", crate::kit::Enter, Some("TextInput")),
        KeyBinding::new(
            "shift-enter",
            crate::kit::ShiftEnter,
            Some("TextInput && !HistoryFind"),
        ),
        // Disjoint scopes keep this independent of registration order.
        KeyBinding::new(
            "shift-enter",
            HistoryFindPrevious,
            Some("HistoryFind > TextInput"),
        ),
        KeyBinding::new("secondary-enter", TextInputCommitSubmit, Some("TextInput")),
        KeyBinding::new("f1", TextInputDiffPrevFile, Some("TextInput")),
        KeyBinding::new("f4", TextInputDiffNextFile, Some("TextInput")),
        KeyBinding::new(
            "f2",
            TextInputDiffPrevSearchMatchOrChange,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "f3",
            TextInputDiffNextSearchMatchOrChange,
            Some("TextInput"),
        ),
        KeyBinding::new("shift-f7", TextInputDiffPrevChange, Some("TextInput")),
        KeyBinding::new("f7", TextInputDiffNextChange, Some("TextInput")),
        KeyBinding::new("alt-up", TextInputDiffPrevChange, Some("TextInput")),
        KeyBinding::new("alt-down", TextInputDiffNextChange, Some("TextInput")),
        KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
        KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
        KeyBinding::new("up", crate::kit::Up, Some("TextInput")),
        KeyBinding::new("down", crate::kit::Down, Some("TextInput")),
        // Word navigation (Ctrl on Windows/Linux, Option on macOS)
        KeyBinding::new("ctrl-left", crate::kit::WordLeft, Some("TextInput")),
        KeyBinding::new("ctrl-right", crate::kit::WordRight, Some("TextInput")),
        KeyBinding::new(
            "ctrl-shift-left",
            crate::kit::SelectWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "ctrl-shift-right",
            crate::kit::SelectWordRight,
            Some("TextInput"),
        ),
        KeyBinding::new("alt-left", crate::kit::WordLeft, Some("TextInput")),
        KeyBinding::new("alt-right", crate::kit::WordRight, Some("TextInput")),
        KeyBinding::new(
            "alt-shift-left",
            crate::kit::SelectWordLeft,
            Some("TextInput"),
        ),
        KeyBinding::new(
            "alt-shift-right",
            crate::kit::SelectWordRight,
            Some("TextInput"),
        ),
        KeyBinding::new("shift-left", crate::kit::SelectLeft, Some("TextInput")),
        KeyBinding::new("shift-right", crate::kit::SelectRight, Some("TextInput")),
        KeyBinding::new("shift-up", crate::kit::SelectUp, Some("TextInput")),
        KeyBinding::new("shift-down", crate::kit::SelectDown, Some("TextInput")),
        KeyBinding::new("home", crate::kit::Home, Some("TextInput")),
        KeyBinding::new("ctrl-home", crate::kit::DocumentHome, Some("TextInput")),
        KeyBinding::new("ctrl-end", crate::kit::DocumentEnd, Some("TextInput")),
        KeyBinding::new("cmd-home", crate::kit::DocumentHome, Some("TextInput")),
        KeyBinding::new("cmd-end", crate::kit::DocumentEnd, Some("TextInput")),
        KeyBinding::new("shift-home", crate::kit::SelectHome, Some("TextInput")),
        KeyBinding::new("end", crate::kit::End, Some("TextInput")),
        KeyBinding::new("shift-end", crate::kit::SelectEnd, Some("TextInput")),
        KeyBinding::new("cmd-left", crate::kit::Home, Some("TextInput")),
        KeyBinding::new("cmd-shift-left", crate::kit::SelectHome, Some("TextInput")),
        KeyBinding::new("cmd-right", crate::kit::End, Some("TextInput")),
        KeyBinding::new("cmd-shift-right", crate::kit::SelectEnd, Some("TextInput")),
        KeyBinding::new("pageup", crate::kit::PageUp, Some("TextInput")),
        KeyBinding::new("shift-pageup", crate::kit::SelectPageUp, Some("TextInput")),
        KeyBinding::new("pagedown", crate::kit::PageDown, Some("TextInput")),
        KeyBinding::new(
            "shift-pagedown",
            crate::kit::SelectPageDown,
            Some("TextInput"),
        ),
        KeyBinding::new("cmd-a", crate::kit::SelectAll, Some("TextInput")),
        KeyBinding::new("ctrl-a", crate::kit::SelectAll, Some("TextInput")),
        KeyBinding::new("cmd-v", crate::kit::Paste, Some("TextInput")),
        KeyBinding::new("ctrl-v", crate::kit::Paste, Some("TextInput")),
        KeyBinding::new("cmd-c", crate::kit::Copy, Some("TextInput")),
        KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        KeyBinding::new("cmd-x", crate::kit::Cut, Some("TextInput")),
        KeyBinding::new("ctrl-x", crate::kit::Cut, Some("TextInput")),
        KeyBinding::new("cmd-z", crate::kit::Undo, Some("TextInput")),
        KeyBinding::new("ctrl-z", crate::kit::Undo, Some("TextInput")),
        KeyBinding::new("cmd-shift-z", crate::kit::Redo, Some("TextInput")),
        KeyBinding::new("ctrl-shift-z", crate::kit::Redo, Some("TextInput")),
        #[cfg(target_os = "macos")]
        KeyBinding::new(
            "ctrl-cmd-space",
            crate::kit::ShowCharacterPalette,
            Some("TextInput"),
        ),
    ]);
}

pub(super) fn bind_terminal_keys(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", TerminalCopy, Some("Terminal")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", TerminalPaste, Some("Terminal")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", TerminalSelectAll, Some("Terminal")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("secondary-shift-a", TerminalSelectAll, Some("Terminal")),
    ]);
}
