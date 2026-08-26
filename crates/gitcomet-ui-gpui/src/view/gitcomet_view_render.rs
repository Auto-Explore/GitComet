use super::*;
use chrome::{cursor_style_for_resize_edge, resize_edge};

#[cfg(test)]
use tooltip::clear_visible_tooltip_text_for_test;

impl Render for GitCometView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        clear_visible_tooltip_text_for_test();

        let external_repo_drop_enabled =
            renders_full_chrome(self.view_mode) && !self.state.repos.is_empty();
        if self.external_drag_paths.is_some()
            && (!external_repo_drop_enabled
                || (!cx.has_active_drag() && !self.external_drag_drop_pending))
        {
            self.clear_external_drag_state(false, cx);
        }

        let theme = self.theme;
        let font_preferences = crate::font_preferences::current(cx);
        debug_assert!(matches!(
            self.view_mode,
            GitCometViewMode::Normal | GitCometViewMode::FocusedMergetool
        ));
        self.last_window_size = window.viewport_size();
        self.clamp_pane_widths_to_window();
        if self.last_window_size != self.ui_window_size_last_seen {
            self.ui_window_size_last_seen = self.last_window_size;
            self.schedule_ui_settings_persist(cx);
        }

        if self
            .pending_branch_exists_prompt
            .as_ref()
            .is_some_and(|prompt| self.active_repo_id() == Some(prompt.repo_id))
        {
            let prompt = self
                .pending_branch_exists_prompt
                .take()
                .expect("branch-exists prompt checked above");
            self.open_popover_centered(
                PopoverKind::BranchExistsPrompt {
                    repo_id: prompt.repo_id,
                    name: prompt.name,
                    target: prompt.target,
                    operation: prompt.operation,
                },
                window,
                cx,
            );
        }

        if let Some(repo_id) = self.pending_pull_reconcile_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::PullReconcilePrompt { repo_id },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        if let Some(prompt) = self.pending_unsaved_file_edits_prompt.take() {
            let anchor = point(
                self.last_window_size.width / 2.0,
                self.last_window_size.height / 2.0,
            );
            self.open_popover_at(
                PopoverKind::UnsavedFileEditsConfirm(prompt),
                anchor,
                window,
                cx,
            );
        }

        if let Some(prompt) = self.pending_terminal_shutdown_prompt.take() {
            let anchor = point(
                self.last_window_size.width / 2.0,
                self.last_window_size.height / 2.0,
            );
            self.open_popover_at(
                PopoverKind::TerminalShutdownConfirm(prompt),
                anchor,
                window,
                cx,
            );
        }

        if let Some((repo_id, name)) = self.pending_force_delete_branch_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            if self.pending_force_delete_branch_centered {
                self.open_popover_centered(
                    PopoverKind::ForceDeleteBranchConfirm { repo_id, name },
                    window,
                    cx,
                );
            } else {
                self.open_popover_at(
                    PopoverKind::ForceDeleteBranchConfirm { repo_id, name },
                    self.last_mouse_pos,
                    window,
                    cx,
                );
            }
        }

        if let Some((repo_id, path, branch)) = self.pending_force_remove_worktree_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::ForceRemoveWorktreeConfirm {
                    repo_id,
                    path,
                    branch,
                },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        // A trust check just started: open the trust popover immediately in its
        // pending/spinner state so there is no dead gap while the background
        // check runs. It fills in with the real sources (or is closed on a
        // silent proceed) when the check resolves — see `apply_state_snapshot`.
        if let Some(check) = self.pending_submodule_trust_check.take()
            && self.active_repo_id() == Some(check.repo_id)
        {
            self.open_popover_at(
                PopoverKind::submodule(check.repo_id, SubmodulePopoverKind::TrustConfirm),
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        if let Some(prompt) = self.pending_submodule_trust_prompt.take()
            && self.active_repo_id() == Some(prompt.repo_id)
        {
            self.open_popover_at(
                PopoverKind::submodule(prompt.repo_id, SubmodulePopoverKind::TrustConfirm),
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        let decorations = window.window_decorations();
        let (tiling, client_inset) = match decorations {
            Decorations::Client { tiling } => (
                Some(tiling),
                chrome::client_side_decoration_inset(self.ui_scale_percent),
            ),
            Decorations::Server => (None, px(0.0)),
        };
        window.set_client_inset(client_inset);

        let cursor = self
            .hover_resize_edge
            .map(cursor_style_for_resize_edge)
            .unwrap_or(CursorStyle::Arrow);

        let center_content = self.center_content(window, cx);
        let font_features = crate::font_preferences::current_font_features(cx);
        let show_custom_window_chrome =
            crate::linux_gui_env::LinuxGuiEnvironment::should_render_custom_window_chrome(
                decorations,
            );

        let mut body = div()
            .flex()
            .flex_col()
            .size_full()
            .font(gpui::Font {
                family: crate::font_preferences::applied_ui_font_family(
                    &font_preferences.ui_font_family,
                )
                .into(),
                features: font_features,
                fallbacks: None,
                weight: gpui::FontWeight::default(),
                style: gpui::FontStyle::default(),
            })
            .text_color(theme.colors.foreground.primary)
            // Any click anywhere hides visible tooltips (both gpui-managed
            // bubbles and the canvas-driven TooltipHost overlay).
            .capture_any_mouse_down(cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                tooltip::dismiss_tooltips_on_mouse_down(cx);
                this.tooltip_host.update(cx, |host, cx| {
                    host.clear_tooltip(cx);
                });
                this.commit_message_hover_host
                    .update(cx, |host, cx| host.dismiss(cx));
            }));

        if show_custom_window_chrome {
            body = body.child(stable_cached_fixed_height_view(
                self.title_bar.clone(),
                chrome::title_bar_height(self.ui_scale_percent),
            ));
        }

        body = body.child(center_content);

        if let Some(report) = self.startup_crash_report.clone()
            && self.view_mode == GitCometViewMode::Normal
        {
            let summary = report.summary.clone();

            let report_button =
                components::Button::new("startup_crash_report_open", "Report Issue")
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.report_startup_crash_report(cx);
                    });

            let ignore_button =
                components::Button::new("startup_crash_report_ignore", "Ignore Crash")
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        if let Err(err) = this.ignore_startup_crash_report() {
                            this.push_toast(
                                components::ToastKind::Error,
                                format!("Could not clear crash report: {err}"),
                                cx,
                            );
                        }
                        cx.notify();
                    });

            body = body.child(
                div()
                    .id("startup_crash_report")
                    .debug_selector(|| "startup_crash_report".to_string())
                    .relative()
                    .px_2()
                    .py_1()
                    // Light's `status.*.background` is a saturated cream that
                    // reads as a coloured card rather than a notification. The
                    // status colour stays in the border; the panel is neutral,
                    // like the toasts and the progress shell.
                    .bg(if theme.is_dark {
                        with_alpha(theme.colors.status.warning.foreground, 0.13)
                    } else {
                        theme.colors.surface.raised
                    })
                    .border_1()
                    .border_color(if theme.is_dark {
                        with_alpha(theme.colors.status.warning.foreground, 0.30)
                    } else {
                        theme.colors.status.warning.border
                    })
                    .rounded(px(theme.radii.panel))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child("GitComet recovered from program crash"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(
                                        "Would you like to contribute by reporting issue to GitComet GitHub repository?",
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(format!("Summary: {summary}")),
                            )
                            .child(
                                div()
                                    .pt_1()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(report_button)
                                    .child(ignore_button),
                            ),
                    ),
            );
        }

        if let Some(prompt) = self.state.auth_prompt.clone() {
            let prompt_key = format!("{:?}:{:?}", prompt.kind, prompt.operation);
            if self.auth_prompt_key.as_ref() != Some(&prompt_key) {
                self.auth_prompt_key = Some(prompt_key);
                self.auth_prompt_username_input
                    .update(cx, |input, cx| input.set_text("", cx));
                self.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("", cx));
            }

            self.auth_prompt_username_input
                .update(cx, |input, cx| input.set_theme(theme, cx));
            let is_host_verification = prompt.kind == AuthPromptKind::HostVerification;
            self.auth_prompt_secret_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_masked(!is_host_verification, cx);
            });

            let requires_username = prompt.kind == AuthPromptKind::UsernamePassword;
            let title = match prompt.kind {
                AuthPromptKind::UsernamePassword => "Repository authentication required",
                AuthPromptKind::Passphrase => "Passphrase required",
                AuthPromptKind::HostVerification => "Host authenticity confirmation required",
            };
            let subtitle = match prompt.kind {
                AuthPromptKind::UsernamePassword => {
                    "Enter username and password, then confirm to retry."
                }
                AuthPromptKind::Passphrase => "Enter your key passphrase, then confirm to retry.",
                AuthPromptKind::HostVerification => {
                    "Enter `yes` to trust this host key, or paste the shown fingerprint."
                }
            };

            let confirm_button = components::Button::new("auth_prompt_confirm", "Confirm")
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.try_auth_prompt_submit(cx);
                });

            let cancel_button = components::Button::new("auth_prompt_cancel", "Cancel")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.store.dispatch(Msg::CancelAuthPrompt);
                    cx.notify();
                });

            let prompt_form = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().font_weight(FontWeight::BOLD).child(title))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.colors.foreground.secondary)
                        .child(subtitle),
                )
                .when(requires_username, |this| {
                    this.child(self.auth_prompt_username_input.clone())
                })
                .child(self.auth_prompt_secret_input.clone())
                .when(is_host_verification, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .child("Use Cancel if you do not trust this host."),
                    )
                })
                .when(!prompt.reason.trim().is_empty(), |this| {
                    this.child(
                        restrict_scroll_to_vertical_axis(
                            div()
                                .id("auth_prompt_reason_scroll")
                                .max_h(px(96.0))
                                .overflow_y_scroll(),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(prompt.reason.clone()),
                        ),
                    )
                })
                .child(
                    div()
                        .pt_1()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(confirm_button)
                        .child(cancel_button),
                );

            let (prompt_bg, prompt_border) = Self::auth_prompt_banner_colors(theme);
            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .bg(prompt_bg)
                    .border_1()
                    .border_color(prompt_border)
                    .rounded(px(theme.radii.panel))
                    .child(prompt_form),
            );
        } else {
            self.auth_prompt_key = None;
        }

        let banner_error =
            if Self::should_render_generic_error_banner(self.state.auth_prompt.is_some()) {
                self.state
                    .banner_error
                    .as_ref()
                    .map(|banner| banner.message.clone())
            } else {
                None
            };
        if let Some(err_text) = banner_error {
            let (error_command, display_error) =
                Self::split_error_banner_message(err_text.as_ref());
            let show_overflow_hint =
                Self::should_show_error_banner_overflow_hint(err_text.as_ref());
            self.error_banner_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text(display_error.clone(), cx);
                input.set_read_only(true, cx);
            });

            let dismiss = components::Button::new("repo_error_banner_close", "")
                .start_slot(svg_icon(
                    "icons/generic_close.svg",
                    theme.colors.foreground.secondary,
                    px(12.0),
                ))
                .style(components::ButtonStyle::Transparent)
                .on_click(theme, cx, move |this, _e, _w, _cx| {
                    this.store.dispatch(Msg::DismissBannerError);
                });

            let command_block = error_command.as_ref().map(|command| {
                div()
                    .id("repo_error_banner_command")
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .bg(with_alpha(
                        theme.colors.surface.canvas,
                        if theme.is_dark { 0.28 } else { 0.75 },
                    ))
                    .rounded(px(theme.radii.row))
                    .px_2()
                    .py_1()
                    .child(command.clone())
            });

            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .pr(px(40.0))
                    .bg(if theme.is_dark {
                        with_alpha(theme.colors.status.danger.foreground, 0.15)
                    } else {
                        theme.colors.surface.raised
                    })
                    .border_1()
                    .border_color(if theme.is_dark {
                        with_alpha(theme.colors.status.danger.foreground, 0.3)
                    } else {
                        theme.colors.status.danger.border
                    })
                    .rounded(px(theme.radii.panel))
                    .child(
                        restrict_scroll_to_vertical_axis(
                            div()
                                .id("repo_error_banner_scroll")
                                .max_h(px(140.0))
                                .overflow_y_scroll(),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .when_some(command_block, |this, command_block| {
                                    this.child(command_block)
                                })
                                .child(self.error_banner_input.clone()),
                        ),
                    )
                    .when(show_overflow_hint, |this| {
                        this.child(
                            div()
                                .mt_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child("Scroll for full output"),
                        )
                    })
                    .child(div().absolute().top(px(6.0)).right(px(6.0)).child(dismiss)),
            );
        }

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.foreground.primary);
        root = root.relative();
        if external_repo_drop_enabled {
            root = root.on_drag_move(cx.listener(
                |this, event: &gpui::DragMoveEvent<gpui::ExternalPaths>, _window, cx| {
                    this.begin_external_drag_classification(event.drag(cx).clone(), false, cx);
                },
            ));
        }
        root = root.child(UiScaleScrollCapture { view: cx.entity() });
        root = root
            .on_action(cx.listener(|this, _: &OpenActiveViewSearch, window, cx| {
                let handled = this
                    .main_pane
                    .update(cx, |pane, cx| pane.open_search_for_active_view(window, cx));
                if handled {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, window, cx| {
                if !command_palette_available(this.view_mode) {
                    cx.stop_propagation();
                    return;
                }
                this.toggle_command_palette(window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &LocateFileInExplorer, _window, cx| {
                this.locate_open_file_in_explorer(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &CommandPaletteDismiss, window, cx| {
                if this.command_palette_open {
                    this.close_command_palette(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &TextInputCommitSubmit, window, cx| {
                let handled = this.details_pane.update(cx, |pane, cx| {
                    pane.handle_commit_submit_shortcut(window, cx)
                });
                if handled {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &TextInputDiffPrevFile, _window, cx| {
                if !show_diff_file_navigation(this.view_mode) {
                    cx.stop_propagation();
                    return;
                }
                this.defer_text_input_adjacent_diff_file_navigation(-1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &TextInputDiffNextFile, _window, cx| {
                if !show_diff_file_navigation(this.view_mode) {
                    cx.stop_propagation();
                    return;
                }
                this.defer_text_input_adjacent_diff_file_navigation(1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(
                |this, _: &TextInputDiffPrevSearchMatchOrChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_prev_search_match_or_diff_change(cx)
                    });
                    cx.stop_propagation();
                },
            ))
            .on_action(cx.listener(
                |this, _: &TextInputDiffNextSearchMatchOrChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_next_search_match_or_diff_change(cx)
                    });
                    cx.stop_propagation();
                },
            ))
            .on_action(cx.listener(|this, _: &DiffPrevFile, _window, cx| {
                if !show_diff_file_navigation(this.view_mode) {
                    cx.stop_propagation();
                    return;
                }
                this.defer_adjacent_diff_file_navigation(-1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &DiffNextFile, _window, cx| {
                if !show_diff_file_navigation(this.view_mode) {
                    cx.stop_propagation();
                    return;
                }
                this.defer_adjacent_diff_file_navigation(1, cx);
                cx.stop_propagation();
            }))
            .on_action(
                cx.listener(|this, _: &DiffPrevSearchMatchOrChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_prev_search_match_or_diff_change(cx)
                    });
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this, _: &DiffNextSearchMatchOrChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_next_search_match_or_diff_change(cx)
                    });
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this, _: &TextInputDiffPrevChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_prev_diff_change(cx)
                    });
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this, _: &TextInputDiffNextChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_next_diff_change(cx)
                    });
                    cx.stop_propagation();
                }),
            );

        root = root.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            this.last_mouse_pos = e.position;
            this.history_refs_hover_host
                .update(cx, |host, cx| host.on_mouse_moved(e.position, cx));
            this.commit_message_hover_host
                .update(cx, |host, cx| host.on_mouse_moved(e.position, cx));
            this.tooltip_host
                .update(cx, |tooltip, cx| tooltip.on_mouse_moved(e.position, cx));

            let Decorations::Client { tiling } = window.window_decorations() else {
                if this.hover_resize_edge.is_some() {
                    this.hover_resize_edge = None;
                    cx.notify();
                }
                return;
            };

            let size = window.viewport_size();
            let next = resize_edge(
                e.position,
                chrome::client_side_decoration_inset(this.ui_scale_percent),
                size,
                tiling,
            );
            if next != this.hover_resize_edge {
                this.hover_resize_edge = next;
                cx.notify();
            }
        }));
        root = root.on_any_mouse_down(cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
            this.dismiss_history_refs_menus(cx);
        }));
        root = root
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.dispatch_global_nav(false, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    this.dispatch_global_nav(true, cx);
                }),
            );
        if tiling.is_some() {
            root = root.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    let Decorations::Client { tiling } = window.window_decorations() else {
                        return;
                    };

                    let size = window.viewport_size();
                    let edge = resize_edge(
                        e.position,
                        chrome::client_side_decoration_inset(this.ui_scale_percent),
                        size,
                        tiling,
                    );
                    let Some(edge) = edge else {
                        return;
                    };

                    cx.stop_propagation();
                    crate::app::begin_window_resize(window, edge);
                }),
            );
        } else if self.hover_resize_edge.is_some() {
            self.hover_resize_edge = None;
        }

        let framed_content = div().relative().size_full().child(body);

        let frame_overlay = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(self.command_palette.clone())
            .child(stable_overlay_view(self.history_refs_hover_host.clone()))
            .child(stable_overlay_view(self.commit_message_hover_host.clone()))
            .child(stable_overlay_view(self.popover_host.clone()))
            .child(stable_overlay_view(self.toast_host.clone()))
            .child(stable_overlay_view(self.tooltip_host.clone()));

        root = root.child(chrome::window_frame(
            theme,
            decorations,
            framed_content.into_any_element(),
            Some(frame_overlay.into_any_element()),
            self.ui_scale_percent,
        ));

        if crate::startup_probe::is_enabled() {
            root = root.on_children_prepainted(|_children_bounds, window, _cx| {
                if crate::startup_probe::mark_first_paint() {
                    window.on_next_frame(|_window, cx| {
                        crate::startup_probe::mark_first_interactive();
                        if crate::startup_probe::should_exit_after_first_interactive() {
                            crate::app::mark_clean_shutdown_requested(cx);
                            cx.quit();
                        }
                    });
                }
            });
        }

        root
    }
}
