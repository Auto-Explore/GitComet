use super::*;
use crate::view::terminal_alacritty::{terminal_default_background, terminal_default_foreground};

const DIALOG_WIDTH_PX: f32 = 900.0;
const DIALOG_HEIGHT_PX: f32 = 680.0;
const DIALOG_MARGIN_PX: f32 = 16.0;
const HISTORY_RAIL_WIDTH_PX: f32 = 220.0;

fn status_label(status: GitHookOperationStatus) -> &'static str {
    match status {
        GitHookOperationStatus::Running => "Running",
        GitHookOperationStatus::Cancelling => "Stopping",
        GitHookOperationStatus::Succeeded => "Passed",
        GitHookOperationStatus::SucceededWithHookFailure => "Warning",
        GitHookOperationStatus::Failed => "Failed",
        GitHookOperationStatus::Cancelled => "Stopped",
        GitHookOperationStatus::TimedOut => "Timed out",
    }
}

fn status_color(theme: AppTheme, status: GitHookOperationStatus) -> gpui::Rgba {
    match status {
        GitHookOperationStatus::Succeeded => theme.colors.status.success.foreground,
        GitHookOperationStatus::SucceededWithHookFailure | GitHookOperationStatus::Cancelled => {
            theme.colors.status.warning.foreground
        }
        GitHookOperationStatus::Failed | GitHookOperationStatus::TimedOut => {
            theme.colors.status.danger.foreground
        }
        GitHookOperationStatus::Running | GitHookOperationStatus::Cancelling => {
            theme.colors.accent.foreground
        }
    }
}

fn duration_label(duration: Option<Duration>) -> String {
    let Some(duration) = duration else {
        return String::new();
    };
    if duration.as_secs() >= 60 {
        format!(
            "{}m {:02}s",
            duration.as_secs() / 60,
            duration.as_secs() % 60
        )
    } else if duration.as_secs() > 0 {
        format!("{:.1}s", duration.as_secs_f32())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn hook_status_label(hook: &gitcomet_state::model::GitHookRun) -> String {
    match hook.status {
        GitHookRunStatus::Running => "running".to_string(),
        GitHookRunStatus::Succeeded => "passed".to_string(),
        GitHookRunStatus::Failed => hook
            .exit_code
            .map_or_else(|| "failed".to_string(), |code| format!("failed ({code})")),
        GitHookRunStatus::Cancelled => "stopped".to_string(),
    }
}

fn operation_timestamp_label(this: &PopoverHost, operation: &GitHookOperation) -> String {
    let mut timestamp = String::with_capacity(24);
    format_datetime_into(
        &mut timestamp,
        operation.time,
        this.date_time_format,
        this.timezone,
        this.show_timezone,
    );
    timestamp
}

fn history_row(
    this: &mut PopoverHost,
    operation: &GitHookOperation,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    let theme = this.theme;
    let ui_scale = popover_ui_scale(cx);
    let scaled_px = |value: f32| popover_scaled_px(value, ui_scale);
    let operation_id = operation.id;
    let selected = this.hook_activity_selected == Some(operation_id);
    let color = status_color(theme, operation.status);
    let duration = duration_label(operation.duration);
    let status = if duration.is_empty() {
        status_label(operation.status).to_string()
    } else {
        format!("{} · {duration}", status_label(operation.status))
    };
    let timestamp = operation_timestamp_label(this, operation);
    let selector = format!("hook_activity_run_{}", operation_id.0);
    let timestamp_selector = format!("hook_activity_run_timestamp_{}", operation_id.0);

    div()
        .id(("hook_activity_run", operation_id.0))
        .debug_selector(move || selector.clone())
        .w_full()
        .h(scaled_px(48.0))
        .px_2()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .when(selected, |row| {
            row.bg(theme.colors.interaction.pressed_background)
        })
        .when(!selected, |row| {
            row.hover(move |style| style.bg(theme.colors.interaction.hover_background))
                .active(move |style| style.bg(theme.colors.interaction.pressed_background))
        })
        .child(
            div()
                .when(selected, |dot| {
                    dot.debug_selector(|| "hook_activity_selected_run".to_string())
                })
                .w(scaled_px(8.0))
                .h(scaled_px(8.0))
                .flex_none()
                .rounded(px(999.0))
                .bg(color),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                                .child(operation.label.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(color)
                                .whitespace_nowrap()
                                .child(status),
                        ),
                )
                .child(
                    div()
                        .debug_selector(move || timestamp_selector.clone())
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(theme.colors.foreground.secondary)
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(timestamp),
                ),
        )
        .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
            if this.hook_activity_selected == Some(operation_id) {
                return;
            }
            this.hook_activity_selected = Some(operation_id);
            this.hook_activity_hooks_scroll = ScrollHandle::new();
            this.hook_activity_hooks_scroll.scroll_to_bottom();
            this.hook_activity_output_scroll = ScrollHandle::new();
            this.hook_activity_output_scroll.scroll_to_bottom();
            cx.notify();
        }))
}

fn visible_scroll_surface(
    theme: AppTheme,
    container_id: &'static str,
    surface_id: &'static str,
    scrollbar_id: &'static str,
    debug_selector: &'static str,
    scroll: ScrollHandle,
    child: impl IntoElement,
) -> AnyElement {
    let scrollbar = components::Scrollbar::new(scrollbar_id, scroll.clone()).always_visible();
    #[cfg(test)]
    let scrollbar = scrollbar.debug_selector(scrollbar_id);

    let surface = restrict_scroll_to_vertical_axis(
        div()
            .id(surface_id)
            .debug_selector(move || debug_selector.to_string())
            .w_full()
            .h_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .pr(components::Scrollbar::visible_gutter(
                scroll.clone(),
                components::ScrollbarAxis::Vertical,
            ))
            .overflow_y_scroll()
            .track_scroll(&scroll),
    )
    .child(child);

    div()
        .id(container_id)
        .relative()
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(surface)
        .child(scrollbar.render(theme))
        .into_any_element()
}

fn history_rail(
    this: &mut PopoverHost,
    operations: &[GitHookOperation],
    rail_width: Pixels,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let theme = this.theme;
    let rows = div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .p_2()
        .children(
            operations
                .iter()
                .map(|operation| history_row(this, operation, cx)),
        );
    let scroll = this.hook_activity_history_scroll.clone();

    div()
        .id("hook_activity_history_rail")
        .debug_selector(|| "hook_activity_history_rail".to_string())
        .w(rail_width)
        .h_full()
        .min_h(px(0.0))
        .flex_none()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(theme.colors.stroke.default)
        .bg(theme.colors.surface.chrome)
        .child(
            div()
                .flex_none()
                .px_3()
                .py_2()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.foreground.secondary)
                .child("RUNS"),
        )
        .child(div().flex_1().min_h(px(0.0)).child(visible_scroll_surface(
            theme,
            "hook_activity_history_scroll_container",
            "hook_activity_history_scroll",
            "hook_activity_history_scrollbar",
            "hook_activity_history_scroll",
            scroll,
            rows,
        )))
        .into_any_element()
}

fn operation_detail(
    this: &mut PopoverHost,
    repo_id: RepoId,
    operation: &GitHookOperation,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let theme = this.theme;
    let operation_id = operation.id;
    let color = status_color(theme, operation.status);
    let terminal_background = terminal_default_background(theme);
    let terminal_foreground = terminal_default_foreground(theme);
    let output = operation.combined_output();
    let copy_output = output.clone();
    let active = operation.status.is_active();

    let hooks_content = div()
        .w_full()
        .min_w(px(0.0))
        .p_1()
        .flex()
        .flex_col()
        .gap_1()
        .children(operation.hooks.iter().enumerate().map(|(index, hook)| {
            let hook_color = match hook.status {
                GitHookRunStatus::Succeeded => theme.colors.status.success.foreground,
                GitHookRunStatus::Failed => theme.colors.status.danger.foreground,
                GitHookRunStatus::Cancelled => theme.colors.status.warning.foreground,
                GitHookRunStatus::Running => theme.colors.accent.foreground,
            };
            div()
                .id(("hook_activity_hook", index))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_between()
                .text_xs()
                .child(
                    div()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.foreground.primary)
                        .child(hook.name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(hook_color)
                        .child(hook_status_label(hook))
                        .child(duration_label(hook.duration)),
                )
        }));
    let hooks_scroll = this.hook_activity_hooks_scroll.clone();
    let mut hooks_view = div()
        .id("hook_activity_hooks_container")
        .debug_selector(|| "hook_activity_hooks_container".to_string())
        .relative()
        .w_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .rounded(px(theme.radii.row))
        .border_1()
        .border_color(theme.colors.stroke.default)
        .bg(with_alpha(
            theme.colors.surface.raised,
            if theme.is_dark { 0.52 } else { 0.78 },
        ))
        .overflow_hidden()
        .child(visible_scroll_surface(
            theme,
            "hook_activity_hooks_scroll_container",
            "hook_activity_hooks_scroll",
            "hook_activity_hooks_scrollbar",
            "hook_activity_hooks_scroll",
            hooks_scroll,
            hooks_content,
        ));
    hooks_view.style().flex_grow = Some(0.9);
    hooks_view.style().flex_shrink = Some(1.0);
    hooks_view.style().flex_basis = Some(relative(0.0).into());

    let output_content = div()
        .w_full()
        .min_w(px(0.0))
        .p_2()
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .text_color(if output.is_empty() {
            theme.colors.foreground.secondary
        } else {
            terminal_foreground
        })
        .when(operation.output_truncated, |content| {
            content.child(
                div()
                    .pb_2()
                    .text_color(theme.colors.status.warning.foreground)
                    .child("Earlier hook output was truncated."),
            )
        })
        .child(if output.is_empty() {
            "This hook did not write any output.".to_string()
        } else {
            output
        });
    let output_scroll = this.hook_activity_output_scroll.clone();
    let output_surface = visible_scroll_surface(
        theme,
        "hook_activity_output_scroll_container",
        "hook_activity_output_scroll",
        "hook_activity_output_scrollbar",
        "hook_activity_output_scroll",
        output_scroll,
        output_content,
    );
    let terminal_header = div()
        .debug_selector(|| "hook_activity_output_terminal_header".to_string())
        .w_full()
        .flex_none()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .bg(theme.colors.surface.panel)
        .border_b_1()
        .border_color(theme.colors.stroke.subtle)
        .child(svg_icon(
            "icons/terminal.svg",
            theme.colors.foreground.secondary,
            px(12.0),
        ))
        .child(
            div()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.colors.foreground.secondary)
                .child("Hook output"),
        );
    let mut output_view = div()
        .id("hook_activity_output_container")
        .debug_selector(|| "hook_activity_output_container".to_string())
        .relative()
        .w_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .rounded(px(theme.radii.row))
        .border_1()
        .border_color(theme.colors.stroke.control)
        .bg(terminal_background)
        .overflow_hidden()
        .child(terminal_header)
        .child(
            div()
                .w_full()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .child(output_surface),
        );
    output_view.style().flex_grow = Some(2.1);
    output_view.style().flex_shrink = Some(1.0);
    output_view.style().flex_basis = Some(relative(0.0).into());

    let main_area = div()
        .id("hook_activity_main_area")
        .debug_selector(|| "hook_activity_main_area".to_string())
        .w_full()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .gap_2()
        .child(hooks_view)
        .child(output_view);

    let actions = div()
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .child(
            components::Button::new(
                format!("hook_activity_copy_{}", operation_id.0),
                "Copy output",
            )
            .style(components::ButtonStyle::Outlined)
            .disabled(copy_output.is_empty())
            .on_click(theme, cx, move |_this, _e, _window, cx| {
                crate::clipboard::write_text(
                    cx,
                    copy_output.clone(),
                    crate::clipboard::CopySource::HookActivity,
                );
            }),
        )
        .when(active, |actions| {
            actions.child(
                components::Button::new(
                    format!("hook_activity_stop_{}", operation_id.0),
                    if operation.status == GitHookOperationStatus::Cancelling {
                        "Stopping…"
                    } else {
                        "Stop"
                    },
                )
                .style(components::ButtonStyle::Danger)
                .disabled(operation.status == GitHookOperationStatus::Cancelling)
                .on_click(theme, cx, move |this, _e, _window, _cx| {
                    this.store.dispatch(Msg::CancelGitOperation {
                        repo_id,
                        operation_id,
                    });
                })
                .debug_selector(move || format!("hook_activity_stop_{}", operation_id.0)),
            )
        });

    div()
        .id("hook_activity_detail")
        .debug_selector(|| "hook_activity_detail".to_string())
        .w_full()
        .h_full()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .bg(theme.colors.surface.canvas)
        .child(
            div()
                .w_full()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .min_w(px(0.0))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .child(operation.label.clone()),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(color)
                                        .child(status_label(operation.status)),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.colors.foreground.secondary)
                                .child(duration_label(operation.duration)),
                        ),
                )
                .when_some(operation.context.clone(), |header, context| {
                    header.child(
                        div()
                            .debug_selector(|| "hook_activity_operation_context".to_string())
                            .w_full()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(context),
                    )
                }),
        )
        .child(main_area)
        .child(actions)
        .into_any_element()
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    _operation_id: Option<GitOperationId>,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = popover_ui_scale(cx);
    let scaled_px = |value: f32| popover_scaled_px(value, ui_scale);
    let window_size = window.window_bounds().get_bounds().size;
    let margin = scaled_px(DIALOG_MARGIN_PX);
    let width = scaled_px(DIALOG_WIDTH_PX).min((window_size.width - margin * 2.0).max(px(0.0)));
    let height = scaled_px(DIALOG_HEIGHT_PX).min((window_size.height - margin * 2.0).max(px(0.0)));
    let rail_width = scaled_px(HISTORY_RAIL_WIDTH_PX).min(width * 0.34);

    let (repository_name, repository_path, operations) = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| {
            (
                crate::view::path_display::repo_path_name(&repo.spec.workdir),
                crate::view::path_display::path_display_shared(&repo.spec.workdir),
                repo.hook_activity
                    .iter()
                    .filter(|operation| operation.has_hooks())
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_else(|| {
            (
                format!("Repository {}", repo_id.0).into(),
                "Repository is no longer open".into(),
                Vec::new(),
            )
        });
    let header_title: SharedString =
        format!("Git hook activity — {}", repository_name.as_ref()).into();

    let selected_is_available = this
        .hook_activity_selected
        .is_some_and(|selected| operations.iter().any(|operation| operation.id == selected));
    if !selected_is_available {
        this.hook_activity_selected = operations.first().map(|operation| operation.id);
        this.hook_activity_hooks_scroll = ScrollHandle::new();
        this.hook_activity_hooks_scroll.scroll_to_bottom();
        this.hook_activity_output_scroll = ScrollHandle::new();
        this.hook_activity_output_scroll.scroll_to_bottom();
    }

    let selected = this
        .hook_activity_selected
        .and_then(|selected| operations.iter().find(|operation| operation.id == selected));

    let minimize_tooltip: SharedString =
        "Minimize to a toast and keep future hook activity minimized".into();
    let minimize_tooltip_for_move = minimize_tooltip.clone();
    let minimize_tooltip_host_for_move = this.tooltip_host.clone();
    let minimize_tooltip_host_for_hover = this.tooltip_host.clone();
    let minimize_button = components::Button::new("hook_activity_minimize", "")
        .start_slot(svg_icon(
            "icons/generic_minimize.svg",
            theme.colors.foreground.secondary,
            scaled_px(14.0),
        ))
        .style(components::ButtonStyle::Transparent)
        .on_click(theme, cx, |this, _e, _window, cx| {
            this.minimize_hook_activity(cx)
        })
        .debug_selector(|| "hook_activity_minimize".to_string())
        .on_mouse_move(
            cx.listener(move |_this, event: &MouseMoveEvent, _window, cx| {
                let _ = minimize_tooltip_host_for_move.update(cx, |host, cx| {
                    host.on_mouse_moved(event.position, cx);
                    host.set_tooltip_text_if_changed(Some(minimize_tooltip_for_move.clone()), cx);
                });
            }),
        )
        .on_hover(cx.listener(move |_this, hovering: &bool, _window, cx| {
            if !*hovering {
                let _ = minimize_tooltip_host_for_hover.update(cx, |host, cx| {
                    host.clear_tooltip_if_matches(&minimize_tooltip, cx);
                });
            }
        }));

    let close_tooltip: SharedString =
        "Close and automatically reopen when new hook activity starts".into();
    let close_tooltip_for_move = close_tooltip.clone();
    let close_tooltip_host_for_move = this.tooltip_host.clone();
    let close_tooltip_host_for_hover = this.tooltip_host.clone();
    let close_button = components::Button::new("hook_activity_close", "")
        .start_slot(svg_icon(
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            scaled_px(14.0),
        ))
        .style(components::ButtonStyle::Transparent)
        .on_click(theme, cx, |this, _e, _window, cx| {
            this.close_hook_activity(cx)
        })
        .debug_selector(|| "hook_activity_close".to_string())
        .on_mouse_move(
            cx.listener(move |_this, event: &MouseMoveEvent, _window, cx| {
                let _ = close_tooltip_host_for_move.update(cx, |host, cx| {
                    host.on_mouse_moved(event.position, cx);
                    host.set_tooltip_text_if_changed(Some(close_tooltip_for_move.clone()), cx);
                });
            }),
        )
        .on_hover(cx.listener(move |_this, hovering: &bool, _window, cx| {
            if !*hovering {
                let _ = close_tooltip_host_for_hover.update(cx, |host, cx| {
                    host.clear_tooltip_if_matches(&close_tooltip, cx);
                });
            }
        }));

    let header_actions = div()
        .flex()
        .items_center()
        .gap_1()
        .child(minimize_button)
        .child(close_button);

    let body = if operations.is_empty() {
        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .text_sm()
            .text_color(theme.colors.foreground.secondary)
            .bg(theme.colors.surface.canvas)
            .child("No Git hooks have run in this repository during this session.")
            .into_any_element()
    } else {
        let detail = selected.map_or_else(
            || div().into_any_element(),
            |operation| operation_detail(this, repo_id, operation, cx),
        );
        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .child(history_rail(this, &operations, rail_width, cx))
            .child(div().flex_1().min_w(px(0.0)).min_h(px(0.0)).child(detail))
            .into_any_element()
    };

    div()
        .debug_selector(|| "hook_activity_panel".to_string())
        .w(width)
        .h(height)
        .min_w(px(0.0))
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .justify_between()
                .bg(theme.colors.surface.chrome)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(svg_icon(
                            "icons/lightning.svg",
                            theme.colors.accent.foreground,
                            scaled_px(15.0),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .debug_selector(|| "hook_activity_title".to_string())
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .line_clamp(1)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(header_title),
                                )
                                .child(
                                    div()
                                        .debug_selector(move || {
                                            format!("hook_activity_repository_{}", repo_id.0)
                                        })
                                        .text_xs()
                                        .font_family("monospace")
                                        .text_color(theme.colors.foreground.secondary)
                                        .line_clamp(1)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(repository_path),
                                ),
                        ),
                )
                .child(header_actions),
        )
        .child(dialog_divider(theme))
        .child(body)
}
