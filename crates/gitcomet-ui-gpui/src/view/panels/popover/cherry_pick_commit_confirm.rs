use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    commit_id: CommitId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let sha = commit_id.as_ref();
    let short = sha.get(0..7).unwrap_or(sha).to_string();
    let summary = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.log {
            Loadable::Ready(page) => page
                .commits
                .iter()
                .find(|commit| commit.id == commit_id)
                .map(|commit| commit.summary.to_string()),
            _ => None,
        })
        .unwrap_or_default();
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let dispatch =
        move |this: &mut PopoverHost, commit_now: bool, cx: &mut gpui::Context<PopoverHost>| {
            this.store.dispatch(Msg::CherryPickCommit {
                repo_id,
                commit_id: commit_id.clone(),
                commit: commit_now,
                summary: summary.clone(),
            });
            this.popover = None;
            this.popover_anchor = None;
            cx.notify();
        };

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(380.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Commit cherry-picked commit?"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!("Apply {short} to the current branch?")),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Commit the cherry-picked change immediately?"),
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
                    super::cancel_button(
                        "cherry_pick_commit_cancel",
                        "cherry_pick_commit_cancel_hint",
                        theme,
                    )
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.popover = None;
                        this.popover_anchor = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            components::Button::new("cherry_pick_commit_no", "No")
                                .style(components::ButtonStyle::Outlined)
                                .on_click(theme, cx, {
                                    let dispatch = dispatch.clone();
                                    move |this, _e, _w, cx| dispatch(this, false, cx)
                                }),
                        )
                        .child(
                            components::Button::new("cherry_pick_commit_yes", "Yes")
                                .style(components::ButtonStyle::Filled)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    dispatch(this, true, cx)
                                }),
                        ),
                ),
        )
}
