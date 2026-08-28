use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    operation_id: GitOperationId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    ConfirmDialog::new("Stop Git operation?", DIALOG_460_WIDTH)
        .text(
            theme,
            "Stopping terminates Git and the running hook. Repository changes may already have been applied and are not rolled back.",
        )
        .render(
            theme,
            components::Button::new("git_operation_stop_cancel", "Cancel")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    this.open_popover_centered(
                        PopoverKind::HookActivity {
                            repo_id,
                            operation_id: Some(operation_id),
                        },
                        window,
                        cx,
                    );
                }),
            components::Button::new("git_operation_stop_confirm", "Stop Git operation")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    this.store.dispatch(Msg::CancelGitOperation {
                        repo_id,
                        operation_id,
                    });
                    this.open_popover_centered(
                        PopoverKind::HookActivity {
                            repo_id,
                            operation_id: Some(operation_id),
                        },
                        window,
                        cx,
                    );
                }),
            cx,
        )
}
