//! The errors on screen in full: what failed, the command and its output,
//! and what can be done about each.

use super::*;
use crate::kit::interaction as controls;
use crate::view::terminal_alacritty::{terminal_default_background, terminal_default_foreground};

/// Selectable text fields, kept across renders so a selection survives.
#[derive(Default)]
pub(super) struct TextState {
    fields: rustc_hash::FxHashMap<String, (Entity<components::TextInput>, bool)>,
}

impl TextState {
    #[cfg(test)]
    pub(super) fn input_for_test(&self, key: &str) -> Option<Entity<components::TextInput>> {
        self.fields.get(key).map(|(input, _)| input.clone())
    }
}

fn text(
    this: &mut PopoverHost,
    key: impl Into<String>,
    value: impl Into<SharedString>,
    multiline: bool,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    let key = key.into();
    let (input, seen) = this
        .error_details_text
        .fields
        .entry(key.clone())
        .or_insert_with(|| {
            let input = cx.new(|cx| {
                let mut input = components::TextInput::new_inert(
                    components::TextInputOptions {
                        read_only: true,
                        chromeless: true,
                        multiline,
                        soft_wrap: multiline,
                        ..Default::default()
                    },
                    cx,
                );
                input.set_display_text(cx);
                if !multiline {
                    input.set_display_truncation(Some(components::TextTruncationProfile::End), cx);
                }
                input
            });
            (input, true)
        });
    *seen = true;
    let value = value.into();
    let theme = this.theme;
    input.update(cx, |input, cx| {
        input.set_theme(theme, cx);
        input.set_text(value, cx);
    });
    let selector = format!("error_details_text_{key}");
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .min_w(px(0.0))
        .max_w_full()
        .child(input.clone())
}

const DIALOG_WIDTH_PX: f32 = 720.0;
const DIALOG_HEIGHT_PX: f32 = 420.0;
const DIALOG_MAX_WIDTH_PX: f32 = 1200.0;
const DIALOG_MAX_HEIGHT_PX: f32 = 860.0;
const DIALOG_WIDTH_FRACTION: f32 = 0.6;
const DIALOG_HEIGHT_FRACTION: f32 = 0.6;
const DIALOG_MARGIN_PX: f32 = 16.0;
const RAIL_WIDTH_PX: f32 = 240.0;

fn extent(available: Pixels, preferred: Pixels, max: Pixels, fraction: f32) -> Pixels {
    (available * fraction)
        .max(preferred)
        .min(max)
        .min(available)
}

fn repo_name(this: &PopoverHost, repo_id: Option<RepoId>) -> Option<SharedString> {
    let repo_id = repo_id?;
    this.state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| crate::view::path_display::repo_path_name(&repo.spec.workdir))
}

fn time_label(this: &PopoverHost, notice: &ErrorNotice) -> String {
    let mut label = String::with_capacity(24);
    format_datetime_into(
        &mut label,
        notice.time,
        this.date_time_format,
        this.timezone,
        this.show_timezone,
    );
    label
}

/// The errors the toast host holds, newest first.
fn notices(this: &PopoverHost, cx: &App) -> Vec<(u64, std::rc::Rc<ErrorNotice>)> {
    this.root_view
        .upgrade()
        .map(|root| root.read(cx).toast_host.read(cx).error_notices())
        .unwrap_or_default()
}

pub(super) fn select(this: &mut PopoverHost, toast_id: u64, cx: &mut gpui::Context<PopoverHost>) {
    if this.error_details_selected == Some(toast_id) {
        return;
    }
    this.error_details_selected = Some(toast_id);
    this.error_details_text = TextState::default();
    this.error_details_scroll = ScrollHandle::new();
    cx.notify();
}

fn rail(
    this: &mut PopoverHost,
    notices: &[(u64, std::rc::Rc<ErrorNotice>)],
    width: Pixels,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let theme = this.theme;
    let danger = theme.colors.status.danger.foreground;
    let rows = notices
        .iter()
        .map(|(id, notice)| {
            let id = *id;
            let selected = this.error_details_selected == Some(id);
            let time = time_label(this, notice);
            let meta = if notice.count > 1 {
                format!("{time} · ×{}", notice.count)
            } else {
                time
            };
            div()
                .id(("error_details_row", id))
                .debug_selector(move || format!("error_details_row_{id}"))
                .w_full()
                .px_2()
                .py_1()
                .flex()
                .items_start()
                .gap_2()
                .rounded(px(theme.radii.row))
                .control_interaction(
                    controls::InteractionStyle::new(theme),
                    controls::InteractionState::default()
                        .selected(selected, theme.colors.interaction.selected_background),
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        select(this, id, cx);
                    }),
                )
                .child(
                    div()
                        .mt(px(6.0))
                        .w(px(8.0))
                        .h(px(8.0))
                        .flex_none()
                        .rounded(px(999.0))
                        .bg(danger),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(theme.ui_text(14.0))
                                .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                                .line_clamp(2)
                                .overflow_hidden()
                                .child(notice.text.summary.clone()),
                        )
                        .child(
                            div()
                                .text_size(theme.ui_text(12.0))
                                .text_color(theme.colors.foreground.secondary)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(meta),
                        ),
                )
        })
        .collect::<Vec<_>>();
    let scroll = this.error_details_rail_scroll.clone();
    div()
        .id("error_details_rail")
        .debug_selector(|| "error_details_rail".to_string())
        .w(width)
        .h_full()
        .min_h(px(0.0))
        .flex_none()
        .border_r_1()
        .border_color(theme.colors.stroke.default)
        .bg(theme.colors.surface.chrome)
        .child(super::hook_activity::visible_scroll_surface(
            theme,
            "error_details_rail_scroll_container",
            "error_details_rail_scroll",
            "error_details_rail_scrollbar",
            "error_details_rail_scroll",
            scroll,
            div().p_2().flex().flex_col().gap(px(1.0)).children(rows),
        ))
        .into_any_element()
}

fn block_heading(theme: AppTheme, label: &'static str) -> gpui::Div {
    div()
        .text_size(theme.ui_text(12.0))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.colors.foreground.secondary)
        .child(label)
}

fn code_block(theme: AppTheme, content: impl IntoElement) -> gpui::Div {
    div()
        .w_full()
        .min_w(px(0.0))
        .px_2()
        .py_1p5()
        .rounded(px(theme.radii.row))
        .border_1()
        .border_color(theme.colors.stroke.control)
        .bg(terminal_default_background(theme))
        .text_color(terminal_default_foreground(theme))
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_size(theme.ui_text(13.0))
        .child(content)
}

fn detail(
    this: &mut PopoverHost,
    id: u64,
    notice: &ErrorNotice,
    error_count: usize,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let theme = this.theme;
    let mut meta = Vec::new();
    if let Some(name) = repo_name(this, notice.repo_id) {
        meta.push(name.to_string());
    }
    meta.push(time_label(this, notice));
    if notice.count > 1 {
        meta.push(format!("happened {} times", notice.count));
    }

    let mut body = div().w_full().min_w(px(0.0)).flex().flex_col().gap_3();
    if let Some(command) = notice.text.command.clone() {
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(block_heading(theme, "Command"))
                .child(code_block(
                    theme,
                    text(this, format!("{id}_command"), command, true, cx),
                )),
        );
    }
    if let Some(details) = notice.text.details.clone() {
        let heading = if notice.text.command.is_some() {
            "Output"
        } else {
            "Details"
        };
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(block_heading(theme, heading))
                .child(code_block(
                    theme,
                    text(this, format!("{id}_details"), details, true, cx),
                )),
        );
    }
    let body_scroll = this.error_details_scroll.clone();
    let body = super::hook_activity::visible_scroll_surface(
        theme,
        "error_details_body_scroll_container",
        "error_details_body_scroll",
        "error_details_body_scrollbar",
        "error_details_body_scroll",
        body_scroll,
        body,
    );

    let available = notice
        .actions
        .iter()
        .map(|action| this.main_pane.read(cx).error_action_available(action))
        .collect::<Vec<_>>();
    let mut actions = div().w_full().flex_none().flex().items_center().gap_2();
    for (ix, (action, available)) in notice.actions.iter().zip(available).enumerate() {
        let action_for_click = action.clone();
        let mut button =
            components::Button::new(format!("error_details_action_{id}_{ix}"), action.label())
                .style(if ix == 0 {
                    components::ButtonStyle::Filled
                } else {
                    components::ButtonStyle::Outlined
                })
                .disabled(!available)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    run_action(this, id, action_for_click.clone(), window, cx);
                })
                .debug_selector(move || format!("error_details_action_{ix}"));
        if !available {
            button = button.gitcomet_tooltip(
                theme,
                "Open the file in the editor again to use this".into(),
            );
        }
        actions = actions.child(button);
    }
    let copy = notice.details_for_copy();
    actions = actions
        .child(
            components::Button::new(format!("error_details_copy_{id}"), "Copy details")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |_this, _e, _window, cx| {
                    crate::clipboard::write_text(
                        cx,
                        copy.clone(),
                        crate::clipboard::CopySource::ErrorDetails,
                    );
                })
                .debug_selector(|| "error_details_copy".to_string()),
        )
        .child(div().flex_1())
        .child(
            components::Button::new(format!("error_details_dismiss_{id}"), "Dismiss")
                .style(components::ButtonStyle::Subtle)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    dismiss(this, Some(id), window, cx);
                })
                .debug_selector(|| "error_details_dismiss".to_string()),
        )
        .when(error_count > 1, |actions| {
            actions.child(
                components::Button::new("error_details_dismiss_all", "Dismiss all")
                    .style(components::ButtonStyle::Subtle)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        dismiss(this, None, window, cx);
                    })
                    .debug_selector(|| "error_details_dismiss_all".to_string()),
            )
        });

    div()
        .id("error_details_detail")
        .debug_selector(|| "error_details_detail".to_string())
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
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(theme.ui_text(16.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.status.danger.foreground)
                        .child(text(
                            this,
                            format!("{id}_summary"),
                            notice.text.summary.clone(),
                            true,
                            cx,
                        )),
                )
                .child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(text(
                            this,
                            format!("{id}_meta"),
                            meta.join(" · "),
                            false,
                            cx,
                        )),
                ),
        )
        .child(div().flex_1().min_h(px(0.0)).child(body))
        .child(actions)
        .into_any_element()
}

/// Dismiss one error (`Some`) or all of them. The dialog closes, through the
/// toast host's change, once none is left.
fn dismiss(
    this: &mut PopoverHost,
    toast_id: Option<u64>,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(root) = this.root_view.upgrade() else {
        return;
    };
    let toast_host = root.read(cx).toast_host.clone();
    let remaining = toast_host.update(cx, |host, cx| {
        match toast_id {
            Some(id) => host.remove_toast(id, cx),
            None => host.dismiss_all_errors(cx),
        }
        host.error_notices().len()
    });
    if remaining == 0 {
        this.close_popover_and_restore_focus(window, cx);
    }
}

fn run_action(
    this: &mut PopoverHost,
    toast_id: u64,
    action: ErrorAction,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match action {
        ErrorAction::Hosted(action) => {
            this.close_popover_and_restore_focus(window, cx);
            action.invoke(cx);
        }
        ErrorAction::SaveEditorAs { format, .. } => {
            let saved = this
                .main_pane
                .update(cx, |pane, cx| pane.save_file_editor_as(format, cx));
            if saved {
                dismiss(this, Some(toast_id), window, cx);
                if this.popover.is_some() {
                    this.close_popover_and_restore_focus(window, cx);
                }
            }
        }
        ErrorAction::RevealInEditor { line, column, .. } => {
            this.close_popover(cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.reveal_in_file_editor(line, column, window, cx)
            });
        }
        ErrorAction::OpenUrl { url, .. } => {
            let root_view = this.root_view.clone();
            crate::view::platform_open::spawn_launch(
                cx,
                move || crate::view::platform_open::open_url_blocking(&url),
                move |_this, result, cx| {
                    if let Err(err) = result {
                        let _ = root_view.update(cx, |root, cx| {
                            root.report_error(
                                ErrorReport::message(None, format!("Failed to open link: {err}")),
                                cx,
                            );
                        });
                    }
                },
            );
        }
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    toast_id: u64,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    for (_, seen) in this.error_details_text.fields.values_mut() {
        *seen = false;
    }
    let notices = notices(this, cx);
    if this.error_details_selected.is_none() {
        this.error_details_selected = Some(toast_id);
    }
    let selected_is_available = this
        .error_details_selected
        .is_some_and(|selected| notices.iter().any(|(id, _)| *id == selected));
    if !selected_is_available && let Some((first, _)) = notices.first() {
        select(this, *first, cx);
    }

    let ui_scale = popover_ui_scale(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    let window_size = crate::view::chrome::window_surface_bounds(window).size;
    let margin = scaled_px(DIALOG_MARGIN_PX);
    let width = extent(
        (window_size.width - margin * 2.0).max(px(0.0)),
        scaled_px(DIALOG_WIDTH_PX),
        scaled_px(DIALOG_MAX_WIDTH_PX),
        DIALOG_WIDTH_FRACTION,
    );
    let height = extent(
        (window_size.height - margin * 2.0).max(px(0.0)),
        scaled_px(DIALOG_HEIGHT_PX),
        scaled_px(DIALOG_MAX_HEIGHT_PX),
        DIALOG_HEIGHT_FRACTION,
    );

    let selected = this
        .error_details_selected
        .and_then(|selected| notices.iter().find(|(id, _)| *id == selected))
        .cloned();
    let title: SharedString = match (notices.len(), selected.as_ref()) {
        (0 | 1, Some((_, notice))) => match repo_name(this, notice.repo_id) {
            Some(name) => format!("Error — {name}").into(),
            None => "Error".into(),
        },
        (count, _) => format!("Errors ({count})").into(),
    };

    let close_button = components::Button::new("error_details_close", "")
        .start_slot(svg_icon(
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            scaled_px(14.0),
        ))
        .style(components::ButtonStyle::Transparent)
        .on_click(theme, cx, |this, _e, window, cx| {
            this.close_popover_and_restore_focus(window, cx);
        })
        .debug_selector(|| "error_details_close".to_string())
        .gitcomet_tooltip(theme, "Close; the errors stay until dismissed".into());

    let body = match selected {
        Some((id, notice)) => {
            let rail = (notices.len() > 1).then(|| {
                rail(
                    this,
                    &notices,
                    scaled_px(RAIL_WIDTH_PX).min(width * 0.34),
                    cx,
                )
            });
            let detail = detail(this, id, &notice, notices.len(), cx);
            div()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .when_some(rail, |body, rail| body.child(rail))
                .child(div().flex_1().min_w(px(0.0)).min_h(px(0.0)).child(detail))
                .into_any_element()
        }
        None => div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.colors.foreground.secondary)
            .child("No errors")
            .into_any_element(),
    };

    let panel = div()
        .debug_selector(|| "error_details_panel".to_string())
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
                            "icons/warning.svg",
                            theme.colors.status.danger.foreground,
                            scaled_px(15.0),
                        ))
                        .child(
                            div()
                                .debug_selector(|| "error_details_title".to_string())
                                .text_size(theme.ui_text(14.0))
                                .font_weight(FontWeight::BOLD)
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(title),
                        ),
                )
                .child(close_button),
        )
        .child(super::popover_rule(theme))
        .child(body);
    this.error_details_text.fields.retain(|_, (_, seen)| *seen);
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_grows_with_the_window_between_its_floor_and_cap() {
        let extent = |available: f32| extent(px(available), px(720.0), px(1200.0), 0.6);
        assert_eq!(extent(500.0), px(500.0), "never wider than the window");
        assert_eq!(extent(1000.0), px(720.0), "keeps its floor while it fits");
        assert_eq!(
            extent(1800.0),
            px(1080.0),
            "takes its share of a big window"
        );
        assert_eq!(extent(4000.0), px(1200.0), "and stops at the cap");
    }
}
