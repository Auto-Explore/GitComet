use super::*;

/// Stages the branch's assigned paths and opens the commit prompt so the user
/// can commit only those changes to the current branch.
fn commit_branch(
    this: &mut PopoverHost,
    repo_id: RepoId,
    paths: Vec<std::path::PathBuf>,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    if !paths.is_empty() {
        this.store.dispatch(Msg::StagePaths {
            repo_id,
            paths: paths.into(),
        });
    }
    this.open_popover_centered(PopoverKind::CommitPrompt { repo_id }, window, cx);
}

fn branch_row(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch: &gitcomet_core::domain::VirtualBranch,
    scaled_px: impl Fn(f32) -> gpui::Pixels + Copy,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let branch_id = branch.id;
    let name = branch.name.to_string();
    let count = branch.paths.len();
    let applied = branch.applied;
    let pending = branch.pending;
    let paths = branch.paths.clone();

    let status_label = if pending {
        "…"
    } else if applied {
        "applied"
    } else {
        "unapplied"
    };
    let status_color = if applied {
        theme.colors.success
    } else {
        theme.colors.text_muted
    };

    let toggle_button = if applied {
        components::Button::new(format!("vb_unapply_{branch_id}"), "Unapply")
            .style(components::ButtonStyle::Outlined)
            .disabled(pending || count == 0)
            .on_click(theme, cx, move |this, _e, _w, _cx| {
                this.store.dispatch(Msg::UnapplyVirtualBranch { repo_id, branch_id });
            })
    } else {
        components::Button::new(format!("vb_apply_{branch_id}"), "Apply")
            .style(components::ButtonStyle::Outlined)
            .disabled(pending)
            .on_click(theme, cx, move |this, _e, _w, _cx| {
                this.store.dispatch(Msg::ApplyVirtualBranch { repo_id, branch_id });
            })
    };
    let commit_button = components::Button::new(format!("vb_commit_{branch_id}"), "Commit")
        .style(components::ButtonStyle::Outlined)
        .disabled(count == 0)
        .on_click(theme, cx, move |this, _e, window, cx| {
            commit_branch(this, repo_id, paths.clone(), window, cx);
        });
    let delete_button =        components::Button::new(format!("vb_delete_{branch_id}"), "Delete")
            .style(components::ButtonStyle::Outlined)
            .disabled(pending)
            .on_click(theme, cx, move |this, _e, _w, _cx| {
                this.store.dispatch(Msg::DeleteVirtualBranch { repo_id, branch_id });
            });

    div()
        .flex()
        .items_center()
        .gap(scaled_px(8.0))
        .px(scaled_px(8.0))
        .py(scaled_px(6.0))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.text)
                        .child(name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(status_color)
                        .child(format!("{status_label} · {count} file(s)")),
                ),
        )
        .child(toggle_button)
        .child(commit_button)
        .child(delete_button)
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let width = super::LARGE_PICKER_WIDTH;
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let branches = repo
        .map(|r| &r.virtual_branches)
        .cloned()
        .unwrap_or_default();

    let header = div()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("Virtual Branches"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .line_height(scaled_px(14.0))
                        .child("Group worktree changes and commit them separately"),
                ),
        )
        .child(
            components::Button::new("vb_close", "Close")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| this.close_popover(cx)),
        );

    let list = if branches.is_empty() {
        components::context_menu_label(
            theme,
            ui_scale_percent,
            "No virtual branches yet. Right-click a changed file and pick “Assign to virtual branch…”.",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element()
    } else {
        let mut list = div().flex().flex_col().w_full();
        for branch in branches.iter() {
            list = list
                .child(div().border_t_1().border_color(theme.colors.border_variant))
                .child(branch_row(this, repo_id, branch, scaled_px, cx));
        }
        list.into_any_element()
    };

    let create_footer = div()
        .border_t_1()
        .border_color(theme.colors.border)
        .px(scaled_px(8.0))
        .py(scaled_px(6.0))
        .flex()
        .items_center()
        .gap(scaled_px(6.0))
        .child(this.virtual_branch_create_input.clone())
        .child(
            components::Button::new("vb_create_go", "Create")
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, |this, _e, _w, cx| this.submit_create_virtual_branch(cx)),
        );

    components::context_menu(
        theme,
        div()
            .flex()
            .flex_col()
            .w(width.preferred_px(ui_scale))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(list)
            .child(create_footer),
    )
}
