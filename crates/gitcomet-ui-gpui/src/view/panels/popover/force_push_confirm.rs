use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let lease = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.pending_force_push_lease.clone());
    let body = if let Some(lease) = lease.as_ref() {
        format!(
            "This will update {}/{} only if it still points at {} and {} is still checked out at {}.",
            lease.remote, lease.branch, lease.expected, lease.local_branch, lease.local_head
        )
    } else {
        "This will overwrite remote history if your branch has diverged.".to_string()
    };
    let command = if let Some(lease) = lease.as_ref() {
        format!(
            "git push --force-with-lease=refs/heads/{}:{} {} {}:refs/heads/{}",
            lease.branch, lease.expected, lease.remote, lease.local_head, lease.branch
        )
    } else {
        "git push --force-with-lease".to_string()
    };
    let button_label = if lease.is_some() {
        "Force push with lease"
    } else {
        "Force push"
    };

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
                .child("Force push"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(body),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.text_muted)
                .child(command),
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
                    components::Button::new("force_push_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("force_push_go", button_label)
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            if let Some(lease) = lease.clone() {
                                this.store
                                    .dispatch(Msg::ForcePushWithLease { repo_id, lease });
                            } else {
                                this.store.dispatch(Msg::ForcePush { repo_id });
                            }
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
