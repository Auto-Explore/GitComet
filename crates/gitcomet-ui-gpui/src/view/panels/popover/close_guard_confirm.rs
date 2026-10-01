use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: CloseGuardPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let (title, confirm_label) = match prompt.action {
        TerminalShutdownAction::QuitApp => (
            format!("Quit {}?", crate::view::product_name()),
            "Quit anyway",
        ),
        TerminalShutdownAction::DeleteWorkspace { .. } => {
            ("Delete workspace?".to_string(), "Delete anyway")
        }
        TerminalShutdownAction::CloseRepo { .. } => {
            ("Close repository?".to_string(), "Close anyway")
        }
        TerminalShutdownAction::CloseRepos { .. } => {
            ("Close repositories?".to_string(), "Close anyway")
        }
        TerminalShutdownAction::CloseWindow
        | TerminalShutdownAction::MoveRepo { .. }
        | TerminalShutdownAction::CloseTerminalForRepo { .. }
        | TerminalShutdownAction::CloseTerminalTab { .. } => {
            ("Close window?".to_string(), "Close anyway")
        }
    };
    let dialog = ConfirmDialog::new(title, DIALOG_440_WIDTH).section(
        div()
            .id("close_guard_reasons")
            .debug_selector(|| "close_guard_reasons".to_string())
            .px_2()
            .pb_1()
            .flex()
            .flex_col()
            .gap_1()
            .text_size(theme.ui_text(14.0))
            .text_color(theme.colors.foreground.secondary)
            .children(
                prompt
                    .reasons
                    .iter()
                    .map(|reason| div().child(reason.clone())),
            ),
    );
    dialog.render(
        theme,
        cancel_button("close_guard_cancel", "close_guard_cancel_hint", theme).on_click(
            theme,
            cx,
            |this, _e, _window, cx| {
                this.close_popover(cx);
            },
        ),
        components::Button::new("close_guard_confirm", confirm_label)
            .style(components::ButtonStyle::Danger)
            .on_click(theme, cx, move |this, _e, window, cx| {
                let root_view = this.root_view.clone();
                let action = prompt.action.clone();
                let _ = root_view.update(cx, |root, cx| {
                    root.perform_close_action(action, window, cx);
                });
                this.close_popover(cx);
            }),
        cx,
    )
}
