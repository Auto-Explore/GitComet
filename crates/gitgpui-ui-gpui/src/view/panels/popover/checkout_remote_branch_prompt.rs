use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    remote: String,
    branch: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let upstream = format!("{remote}/{branch}");
    let remote_for_action = remote.clone();
    let branch_for_action = branch.clone();

    div()
        .flex()
        .flex_col()
        .w(px(540.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Checkout remote branch"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!("Remote branch: {upstream}")),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Local branch name"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    zed::Button::new("checkout_remote_branch_cancel", "Cancel")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    zed::Button::new("checkout_remote_branch_go", "Checkout")
                        .style(zed::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let local_branch = this
                                .create_branch_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            if local_branch.is_empty() {
                                this.push_toast(
                                    zed::ToastKind::Error,
                                    "Branch name cannot be empty".to_string(),
                                    cx,
                                );
                                return;
                            }

                            let local_branch_exists = this
                                .state
                                .repos
                                .iter()
                                .find(|r| r.id == repo_id)
                                .and_then(|repo| match &repo.branches {
                                    Loadable::Ready(branches) => Some(
                                        branches.iter().any(|b| b.name == local_branch.as_str()),
                                    ),
                                    _ => None,
                                })
                                .unwrap_or(false);
                            if local_branch_exists {
                                this.push_toast(
                                    zed::ToastKind::Error,
                                    format!("Branch already exists: {local_branch}"),
                                    cx,
                                );
                                return;
                            }

                            this.store.dispatch(Msg::CheckoutRemoteBranch {
                                repo_id,
                                remote: remote_for_action.clone(),
                                branch: branch_for_action.clone(),
                                local_branch: local_branch.clone(),
                            });
                            this.main_pane.update(cx, |pane, cx| {
                                pane.rebuild_diff_cache(cx);
                                cx.notify();
                            });

                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
