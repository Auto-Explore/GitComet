use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    branch: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let remove_branch = branch.clone();
    let header: SharedString = if branch.is_some() {
        "Remove worktree and branch".into()
    } else {
        "Remove worktree".into()
    };
    let description: Option<SharedString> = branch.as_ref().map(|branch| {
        format!("This will remove the worktree folder and delete the local branch '{branch}'.")
            .into()
    });
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(420.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(header),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(path.display().to_string()),
        )
        .when_some(description, |this, description| {
            this.child(div().border_t_1().border_color(theme.colors.border))
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child(description),
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
                    components::Button::new("worktree_remove_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("worktree_remove_go", "Remove")
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            if let Some(branch) = remove_branch.clone() {
                                let root_view = this.root_view.clone();
                                let _ = root_view.update(cx, |root, _cx| {
                                    root.register_pending_worktree_branch_removal(
                                        repo_id,
                                        path.clone(),
                                        branch,
                                    );
                                });
                            }
                            this.store.dispatch(Msg::RemoveWorktree {
                                repo_id,
                                path: path.clone(),
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
