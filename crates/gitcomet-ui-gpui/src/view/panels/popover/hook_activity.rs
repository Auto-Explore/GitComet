use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::view::terminal_alacritty::{terminal_default_background, terminal_default_foreground};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum TextSection {
    Header,
    History,
    Detail,
    Hooks,
    Output,
}

struct TextField {
    input: Entity<components::TextInput>,
    section: TextSection,
    seen: bool,
}

/// Entities survive streaming updates, but only the current dialog's fields
/// are retained. History fields also survive switching the detail run.
#[derive(Default)]
pub(super) struct TextState {
    repo_id: Option<RepoId>,
    fields: rustc_hash::FxHashMap<String, TextField>,
    run_clicks:
        rustc_hash::FxHashMap<GitOperationId, crate::kit::click::SubtargetClick<GitOperationId>>,
}

impl TextState {
    #[cfg(test)]
    pub(super) fn input_for_test(&self, key: &str) -> Entity<components::TextInput> {
        self.fields
            .get(key)
            .expect("hook activity text field")
            .input
            .clone()
    }

    pub(super) fn is_interacting(&self, section: TextSection, cx: &App) -> bool {
        self.fields
            .values()
            .any(|field| field.section == section && field.input.read(cx).has_selection_or_drag())
    }

    fn clear_detail(&mut self) {
        self.fields
            .retain(|_, field| matches!(field.section, TextSection::Header | TextSection::History));
    }
}

fn text(
    this: &mut PopoverHost,
    key: impl Into<String>,
    value: impl Into<SharedString>,
    section: TextSection,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    let key = key.into();
    let value = value.into();
    let field = this
        .hook_activity_text
        .fields
        .entry(key.clone())
        .or_insert_with(|| {
            let input = cx.new(|cx| {
                let mut input = components::TextInput::new_inert(
                    components::TextInputOptions {
                        read_only: true,
                        chromeless: true,
                        multiline: section == TextSection::Output,
                        soft_wrap: section == TextSection::Output,
                        ..Default::default()
                    },
                    cx,
                );
                input.set_display_text(cx);
                if section != TextSection::Output {
                    input.set_display_truncation(Some(components::TextTruncationProfile::End), cx);
                }
                input
            });
            TextField {
                input,
                section,
                seen: true,
            }
        });
    field.seen = true;
    field.input.update(cx, |input, cx| {
        input.set_theme(this.theme, cx);
        if section == TextSection::Output {
            input.set_vertical_scroll_handle(Some(this.hook_activity_output_scroll.clone()));
            input.set_text_preserving_selection_on_append(value, cx);
        } else {
            input.set_text(value, cx);
            input.set_vertical_scroll_handle(match section {
                TextSection::History => Some(this.hook_activity_history_scroll.clone()),
                TextSection::Hooks => Some(this.hook_activity_hooks_scroll.clone()),
                _ => None,
            });
        }
    });
    let selector = format!("hook_activity_text_{key}");
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .min_w(px(0.0))
        .max_w_full()
        .child(field.input.clone())
}

fn select_run(
    this: &mut PopoverHost,
    operation_id: GitOperationId,
    cx: &mut gpui::Context<PopoverHost>,
) {
    if this.hook_activity_selected == Some(operation_id) {
        return;
    }
    this.hook_activity_selected = Some(operation_id);
    this.hook_activity_text.clear_detail();
    this.hook_activity_hooks_scroll = ScrollHandle::new();
    this.hook_activity_hooks_scroll.scroll_to_bottom();
    this.hook_activity_output_scroll = ScrollHandle::new();
    this.hook_activity_output_scroll.scroll_to_bottom();
    cx.notify();
}

/// One side of the dialog.
fn hook_activity_dialog_extent(
    available: Pixels,
    preferred: Pixels,
    max: Pixels,
    fraction: f32,
) -> Pixels {
    (available * fraction)
        .max(preferred)
        .min(max)
        .min(available)
}

/// A hook run is a two-line list row; the per-hook lines under it are
/// ordinary rows.
const HOOK_ACTIVITY_RUN_ROW_HEIGHT_PX: f32 = 48.0;
const HOOK_ACTIVITY_RUN_ROW_COMFORTABLE_HEIGHT_PX: f32 = 56.0;
const HOOK_ACTIVITY_HOOK_ROW_HEIGHT_PX: f32 = 24.0;
const HOOK_ACTIVITY_HOOK_ROW_COMFORTABLE_HEIGHT_PX: f32 = 32.0;

/// Smallest the dialog opens at, and the share of the window it takes when
/// there is more room than that. Hook output is wide and long, so give it the
/// space rather than scrolling a fixed box, but stop short of the whole window.
const DIALOG_WIDTH_PX: f32 = 900.0;
const DIALOG_HEIGHT_PX: f32 = 680.0;
const DIALOG_MAX_WIDTH_PX: f32 = 1600.0;
const DIALOG_MAX_HEIGHT_PX: f32 = 1100.0;
const DIALOG_WIDTH_FRACTION: f32 = 0.72;
const DIALOG_HEIGHT_FRACTION: f32 = 0.8;
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
    let scaled_px = crate::ui_scale::scaler(ui_scale);
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
    let press_view = cx.entity();
    let move_view = cx.entity();
    let release_view = cx.entity();

    div()
        .id(("hook_activity_run", operation_id.0))
        .debug_selector(move || selector.clone())
        .w_full()
        .h(ui_scale.row_height(
            HOOK_ACTIVITY_RUN_ROW_HEIGHT_PX,
            HOOK_ACTIVITY_RUN_ROW_COMFORTABLE_HEIGHT_PX,
        ))
        .px_2()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(theme.radii.row))
        .on_mouse_down_all(move |event, phase, hitbox, window, cx| {
            if phase == gpui::DispatchPhase::Capture {
                press_view.update(cx, |this, _| {
                    this.hook_activity_text
                        .run_clicks
                        .entry(operation_id)
                        .or_default()
                        .press(hitbox.is_hovered(window).then_some(operation_id), event);
                });
            }
        })
        .on_mouse_move_all(move |event, phase, _, _, cx| {
            if phase == gpui::DispatchPhase::Capture {
                move_view.update(cx, |this, _| {
                    if let Some(click) = this.hook_activity_text.run_clicks.get_mut(&operation_id) {
                        click.moved(event);
                    }
                });
            }
        })
        .on_mouse_up_all(move |event, phase, hitbox, window, cx| {
            let hovered = hitbox.is_hovered(window);
            if (phase == gpui::DispatchPhase::Capture && !hovered)
                || (phase == gpui::DispatchPhase::Bubble && hovered)
            {
                release_view.update(cx, |this, cx| {
                    let completed = this
                        .hook_activity_text
                        .run_clicks
                        .get_mut(&operation_id)
                        .and_then(|click| click.release(hovered.then_some(&operation_id), event))
                        .is_some();
                    let prefix = format!("run_{}_", operation_id.0);
                    let selected_text =
                        this.hook_activity_text.fields.iter().any(|(key, field)| {
                            key.starts_with(&prefix)
                                && !field.input.read(cx).selected_range().is_empty()
                        });
                    if completed && event.click_count == 1 && !selected_text {
                        select_run(this, operation_id, cx);
                    }
                });
            }
        })
        .control_interaction(
            controls::InteractionStyle::new(theme),
            controls::InteractionState::default()
                .selected(selected, theme.colors.interaction.selected_background),
        )
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
                                .text_size(theme.ui_text(14.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                                .child(text(
                                    this,
                                    format!("run_{}_label", operation_id.0),
                                    operation.label.clone(),
                                    TextSection::History,
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(theme.ui_text(12.0))
                                .text_color(color)
                                .whitespace_nowrap()
                                .child(text(
                                    this,
                                    format!("run_{}_status", operation_id.0),
                                    status,
                                    TextSection::History,
                                    cx,
                                )),
                        ),
                )
                .child(
                    div()
                        .debug_selector(move || timestamp_selector.clone())
                        .min_w(px(0.0))
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(text(
                            this,
                            format!("run_{}_timestamp", operation_id.0),
                            timestamp,
                            TextSection::History,
                            cx,
                        )),
                ),
        )
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                select_run(this, operation_id, cx);
            }),
        )
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
                .text_size(theme.ui_text(12.0))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.foreground.secondary)
                .child(text(this, "runs_heading", "RUNS", TextSection::Header, cx)),
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
    let ui_scale = popover_ui_scale(cx);
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
            let key = format!(
                "hook_{}_{}_{}",
                operation_id.0, hook.id.sid, hook.id.child_id
            );
            let hook_color = match hook.status {
                GitHookRunStatus::Succeeded => theme.colors.status.success.foreground,
                GitHookRunStatus::Failed => theme.colors.status.danger.foreground,
                GitHookRunStatus::Cancelled => theme.colors.status.warning.foreground,
                GitHookRunStatus::Running => theme.colors.accent.foreground,
            };
            div()
                .id(("hook_activity_hook", index))
                .h(ui_scale.row_height(
                    HOOK_ACTIVITY_HOOK_ROW_HEIGHT_PX,
                    HOOK_ACTIVITY_HOOK_ROW_COMFORTABLE_HEIGHT_PX,
                ))
                .flex()
                .items_center()
                .justify_between()
                .text_size(theme.ui_text(12.0))
                .child(
                    div()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.foreground.primary)
                        .child(text(
                            this,
                            format!("{key}_name"),
                            hook.name.clone(),
                            TextSection::Hooks,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(hook_color)
                        .child(text(
                            this,
                            format!("{key}_status"),
                            hook_status_label(hook),
                            TextSection::Hooks,
                            cx,
                        ))
                        .child(text(
                            this,
                            format!("{key}_duration"),
                            duration_label(hook.duration),
                            TextSection::Hooks,
                            cx,
                        )),
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
        .text_size(theme.ui_text(12.0))
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
                    .child(text(
                        this,
                        "output_truncation",
                        "Earlier hook output was truncated.",
                        TextSection::Output,
                        cx,
                    )),
            )
        })
        .child(text(
            this,
            format!("output_{}", operation_id.0),
            if output.is_empty() {
                "This hook did not write any output.".to_string()
            } else {
                output
            },
            TextSection::Output,
            cx,
        ));
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
            ui_scale.px(12.0),
        ))
        .child(
            div()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_size(theme.ui_text(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.colors.foreground.secondary)
                .child(text(
                    this,
                    "output_heading",
                    "Hook output",
                    TextSection::Detail,
                    cx,
                )),
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
                                        .text_size(theme.ui_text(18.0))
                                        .font_weight(FontWeight::BOLD)
                                        .child(text(
                                            this,
                                            format!("detail_{}_label", operation_id.0),
                                            operation.label.clone(),
                                            TextSection::Detail,
                                            cx,
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(theme.ui_text(14.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(color)
                                        .child(text(
                                            this,
                                            format!("detail_{}_status", operation_id.0),
                                            status_label(operation.status),
                                            TextSection::Detail,
                                            cx,
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .text_size(theme.ui_text(14.0))
                                .text_color(theme.colors.foreground.secondary)
                                .child(text(
                                    this,
                                    format!("detail_{}_duration", operation_id.0),
                                    duration_label(operation.duration),
                                    TextSection::Detail,
                                    cx,
                                )),
                        ),
                )
                .when_some(operation.context.clone(), |header, context| {
                    header.child(
                        div()
                            .debug_selector(|| "hook_activity_operation_context".to_string())
                            .w_full()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(text(
                                this,
                                format!("detail_{}_context", operation_id.0),
                                context,
                                TextSection::Detail,
                                cx,
                            )),
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
    if this.hook_activity_text.repo_id != Some(repo_id) {
        this.hook_activity_text = TextState {
            repo_id: Some(repo_id),
            ..Default::default()
        };
    }
    for field in this.hook_activity_text.fields.values_mut() {
        field.seen = false;
    }
    let ui_scale = popover_ui_scale(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    let window_size = window.window_bounds().get_bounds().size;
    let margin = scaled_px(DIALOG_MARGIN_PX);
    let available = gpui::size(
        (window_size.width - margin * 2.0).max(px(0.0)),
        (window_size.height - margin * 2.0).max(px(0.0)),
    );
    let width = hook_activity_dialog_extent(
        available.width,
        scaled_px(DIALOG_WIDTH_PX),
        scaled_px(DIALOG_MAX_WIDTH_PX),
        DIALOG_WIDTH_FRACTION,
    );
    let height = hook_activity_dialog_extent(
        available.height,
        scaled_px(DIALOG_HEIGHT_PX),
        scaled_px(DIALOG_MAX_HEIGHT_PX),
        DIALOG_HEIGHT_FRACTION,
    );
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
                repo.feedback
                    .hook_activity
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
    let fallback_selection = operations.first().map(|operation| operation.id);
    if !selected_is_available && this.hook_activity_selected != fallback_selection {
        this.hook_activity_selected = fallback_selection;
        this.hook_activity_text.clear_detail();
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
            .text_size(theme.ui_text(14.0))
            .text_color(theme.colors.foreground.secondary)
            .bg(theme.colors.surface.canvas)
            .child(text(
                this,
                "empty",
                "No Git hooks have run in this repository during this session.",
                TextSection::Detail,
                cx,
            ))
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

    let panel = div()
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
                                        .text_size(theme.ui_text(14.0))
                                        .font_weight(FontWeight::BOLD)
                                        .line_clamp(1)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(text(
                                            this,
                                            "title",
                                            header_title,
                                            TextSection::Header,
                                            cx,
                                        )),
                                )
                                .child(
                                    div()
                                        .debug_selector(move || {
                                            format!("hook_activity_repository_{}", repo_id.0)
                                        })
                                        .text_size(theme.ui_text(12.0))
                                        .font_family("monospace")
                                        .text_color(theme.colors.foreground.secondary)
                                        .line_clamp(1)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(text(
                                            this,
                                            "repository",
                                            repository_path,
                                            TextSection::Header,
                                            cx,
                                        )),
                                ),
                        ),
                )
                .child(header_actions),
        )
        .child(super::popover_rule(theme))
        .child(body);
    this.hook_activity_text.fields.retain(|_, field| field.seen);
    this.hook_activity_text
        .run_clicks
        .retain(|id, _| operations.iter().any(|operation| operation.id == *id));
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_grows_with_the_window_between_its_floor_and_cap() {
        let extent = |available: f32| {
            hook_activity_dialog_extent(px(available), px(900.0), px(1600.0), 0.72)
        };

        assert_eq!(extent(600.0), px(600.0), "never wider than the window");
        assert_eq!(extent(1000.0), px(900.0), "keeps its floor while it fits");
        assert_eq!(
            extent(2000.0),
            px(1440.0),
            "takes its share of a big window"
        );
        assert_eq!(extent(4000.0), px(1600.0), "and stops at the cap");
    }
}
