use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: TerminalShutdownPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let terminal_label = if prompt.summary.terminal_count == 1 {
        "terminal".to_string()
    } else {
        format!("{} terminals", prompt.summary.terminal_count)
    };
    let title = match prompt.action {
        TerminalShutdownAction::QuitApp => "Quit GitComet?",
        TerminalShutdownAction::CloseWindow => "Close window?",
        TerminalShutdownAction::CloseRepo { .. }
        | TerminalShutdownAction::CloseTerminalForRepo { .. }
        | TerminalShutdownAction::CloseTerminalTab { .. } => "Close terminal?",
    };
    let confirm_label = match prompt.action {
        TerminalShutdownAction::QuitApp => "Terminate and quit",
        TerminalShutdownAction::CloseWindow => "Terminate and close",
        TerminalShutdownAction::CloseRepo { .. }
        | TerminalShutdownAction::CloseTerminalForRepo { .. }
        | TerminalShutdownAction::CloseTerminalTab { .. } => "Terminate and close",
    };
    let detail = if prompt.summary.running_command_count == 1 {
        format!("1 running command is still active in {terminal_label}.")
    } else {
        format!(
            "{} running commands are still active in {terminal_label}.",
            prompt.summary.running_command_count
        )
    };

    let repo_names = &prompt.summary.repo_names;
    let show_repo_list = !repo_names.is_empty()
        && matches!(
            prompt.action,
            TerminalShutdownAction::CloseWindow | TerminalShutdownAction::QuitApp
        );

    div()
        .flex()
        .flex_col()
        .min_w(px(440.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(title),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!(
                    "{detail} Cancel to keep the terminal open, or terminate it to continue."
                )),
        )
        .when(show_repo_list, |d| {
            d.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(repo_names.iter().map(|name| {
                                div()
                                    .font_family(
                                        crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                    )
                                    .ml_2()
                                    .child(name.clone())
                            })),
                    ),
            )
        })
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("terminal_shutdown_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _window, cx| {
                            let root_view = this.root_view.clone();
                            let _ = root_view.update(cx, |root, cx| {
                                root.clear_pending_terminal_shutdown_prompt(cx);
                            });
                            this.close_popover(cx);
                        }),
                )
                .child(
                    components::Button::new("terminal_shutdown_confirm", confirm_label)
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            let root_view = this.root_view.clone();
                            let _ = root_view.update(cx, |root, cx| {
                                root.confirm_terminal_shutdown(prompt.clone(), window, cx);
                            });
                            this.close_popover(cx);
                        }),
                ),
        )
}
