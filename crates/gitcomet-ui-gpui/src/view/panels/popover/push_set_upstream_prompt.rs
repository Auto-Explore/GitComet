use super::*;

pub(super) fn remote_names(repo: &RepoState) -> Vec<String> {
    let mut names: Vec<String> = repo
        .remotes
        .ready()
        .map(|remotes| remotes.iter().map(|remote| remote.name.clone()).collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

pub(super) fn selected_remote(repo: &RepoState, preferred: &str) -> Option<String> {
    let names = remote_names(repo);
    names
        .iter()
        .find(|name| name.as_str() == preferred)
        .or_else(|| names.iter().find(|name| name.as_str() == "origin"))
        .or_else(|| names.first())
        .cloned()
}

fn prompt_remote_names(this: &PopoverHost) -> Vec<String> {
    let Some(PopoverKind::PushSetUpstreamPrompt { repo_id, .. }) = this.popover.as_ref() else {
        return Vec::new();
    };
    this.state
        .repos
        .iter()
        .find(|repo| repo.id == *repo_id)
        .map(remote_names)
        .unwrap_or_default()
}

fn open_remote_menu(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    let names = prompt_remote_names(this);
    if names.len() <= 1 {
        return;
    }
    let selected = this.selected_push_upstream_remote();
    this.push_upstream_remote_selected_index = selected
        .as_deref()
        .and_then(|selected| names.iter().position(|name| name == selected))
        .or(Some(0));
    this.push_upstream_remote_menu_open = true;
    cx.notify();
}

pub(super) fn toggle_remote_menu(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    if this.push_upstream_remote_menu_open {
        this.push_upstream_remote_menu_open = false;
        this.push_upstream_remote_selected_index = None;
        cx.notify();
    } else {
        open_remote_menu(this, cx);
    }
}

fn move_remote_selection(
    this: &mut PopoverHost,
    delta: isize,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let names = prompt_remote_names(this);
    if names.len() <= 1 {
        return;
    }
    if !this.push_upstream_remote_menu_open {
        open_remote_menu(this, cx);
        return;
    }
    let count = names.len();
    let current = this.push_upstream_remote_selected_index.unwrap_or(0);
    this.push_upstream_remote_selected_index = Some(if delta < 0 {
        if current == 0 { count - 1 } else { current - 1 }
    } else if current + 1 == count {
        0
    } else {
        current + 1
    });
    cx.notify();
}

fn commit_or_open_remote_selection(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) {
    if !this.push_upstream_remote_menu_open {
        open_remote_menu(this, cx);
        return;
    }
    let names = prompt_remote_names(this);
    if let Some(selected) = this
        .push_upstream_remote_selected_index
        .and_then(|index| names.get(index))
        .cloned()
    {
        this.select_push_upstream_remote(selected, cx);
    } else {
        this.push_upstream_remote_menu_open = false;
        this.push_upstream_remote_selected_index = None;
        cx.notify();
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    remote: String,
    configure_only_for: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_submit = this.can_submit_push_set_upstream(cx);
    let configure_only = configure_only_for.is_some();
    let remotes = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(remote_names)
        .unwrap_or_default();
    let selected_remote = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| selected_remote(repo, &remote));
    let scaled_px = super::popover_scaled_px_fn(cx);

    let remote_control = if remotes.len() > 1 {
        let selected_label: SharedString = selected_remote
            .clone()
            .unwrap_or_else(|| "No remotes".to_string())
            .into();
        let picker_open = this.push_upstream_remote_menu_open;
        let mut selector = div().flex().flex_col().gap_1().child(
            components::Button::new("push_upstream_remote_selector", selected_label)
                .start_slot(crate::view::icons::svg_icon(
                    "icons/cloud.svg",
                    theme.colors.accent.foreground,
                    scaled_px(14.0),
                ))
                .end_slot(crate::view::icons::svg_icon(
                    "icons/chevron_down.svg",
                    theme.colors.foreground.secondary,
                    scaled_px(12.0),
                ))
                .focus_handle(this.push_upstream_remote_focus_handle.clone())
                .style(components::ButtonStyle::Outlined)
                .selected(picker_open)
                .on_click(theme, cx, |this, _e, window, cx| {
                    let focus = this.push_upstream_remote_focus_handle.clone();
                    window.focus(&focus, cx);
                    toggle_remote_menu(this, cx);
                })
                .debug_selector(|| "push_upstream_remote_selector".to_string())
                .key_context("PushUpstreamRemoteSelector")
                .on_action(
                    cx.listener(|this, _: &PushUpstreamRemoteOpenOrSelect, _window, cx| {
                        commit_or_open_remote_selection(this, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &PushUpstreamRemotePrev, _window, cx| {
                        move_remote_selection(this, -1, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &PushUpstreamRemoteNext, _window, cx| {
                        move_remote_selection(this, 1, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &PushUpstreamRemoteClose, window, cx| {
                        if this.push_upstream_remote_menu_open {
                            this.push_upstream_remote_menu_open = false;
                            this.push_upstream_remote_selected_index = None;
                            cx.notify();
                            cx.stop_propagation();
                        } else {
                            // The selector owns Escape while it has focus, so
                            // explicitly hand the already-closed case to the
                            // enclosing prompt's dismissal action.
                            window.dispatch_action(Box::new(crate::view::PopoverPromptDismiss), cx);
                        }
                    }),
                ),
        );
        if picker_open {
            let options = remotes.into_iter().enumerate().fold(
                div()
                    .id("push_upstream_remote_options")
                    .flex()
                    .flex_col()
                    .p_1()
                    .max_h(scaled_px(180.0))
                    .overflow_y_scroll()
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .bg(theme.colors.surface.raised),
                |options, (ix, name)| {
                    let is_selected = this.push_upstream_remote_selected_index == Some(ix)
                        || (this.push_upstream_remote_selected_index.is_none()
                            && selected_remote.as_deref() == Some(name.as_str()));
                    let selected_name = name.clone();
                    options.child(
                        components::ContextMenuEntry::new(
                            ("push_upstream_remote_option", ix),
                            components::ContextMenuText::new(name).max_lines(1),
                        )
                        .icon(components::ContextMenuIconSlot::Icon(
                            "icons/cloud.svg".into(),
                        ))
                        .selected(is_selected)
                        .tooltip_host(this.tooltip_host.clone())
                        .render(theme, super::popover_ui_scale(cx), cx)
                        .debug_selector(move || format!("push_upstream_remote_option_{ix}"))
                        .on_click(cx.listener(
                            move |this, _e: &ClickEvent, _window, cx| {
                                this.select_push_upstream_remote(selected_name.clone(), cx);
                            },
                        )),
                    )
                },
            );
            selector = selector.child(options);
        }
        div().px_2().pb_1().w_full().min_w(px(0.0)).child(selector)
    } else {
        let label = selected_remote.unwrap_or_else(|| "No remotes configured".to_string());
        div()
            .debug_selector(|| "push_upstream_remote_static".to_string())
            .px_2()
            .pb_1()
            .flex()
            .items_center()
            .gap_1()
            .text_sm()
            .text_color(theme.colors.foreground.secondary)
            .child(crate::view::icons::svg_icon(
                "icons/cloud.svg",
                theme.colors.foreground.secondary,
                scaled_px(13.0),
            ))
            .child(label)
    };

    div()
        .flex()
        .flex_col()
        .w(scaled_px(320.0))
        .child(popover_title(if configure_only {
            "Set new upstream"
        } else {
            "Set upstream and push"
        }))
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        .child(input_label(theme, "Remote"))
        .child(remote_control)
        .child(input_label(theme, "Remote branch"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.push_upstream_branch_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button("push_upstream_cancel", "push_upstream_cancel_hint", theme)
                        .focus_handle(this.push_upstream_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new(
                        "push_upstream_go",
                        if configure_only {
                            "Set upstream"
                        } else {
                            "Push"
                        },
                    )
                    .focus_handle(this.push_upstream_focus.submit.clone())
                    .disabled(!can_submit)
                    .separated_end_slot(super::hotkey_hint(theme, "push_upstream_go_hint", "Enter"))
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.submit_push_set_upstream(cx);
                    })
                    .debug_selector(|| "push_upstream_go".to_string()),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Remote, RepoSpec};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn repo_with_remotes(names: &[&str]) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/push-remote-picker"),
            },
        );
        repo.remotes = Loadable::Ready(Arc::new(
            names
                .iter()
                .map(|name| Remote {
                    name: (*name).to_string(),
                    url: None,
                })
                .collect(),
        ));
        repo
    }

    #[test]
    fn selected_remote_keeps_a_configured_preference() {
        let repo = repo_with_remotes(&["origin", "mirror"]);
        assert_eq!(selected_remote(&repo, "mirror").as_deref(), Some("mirror"));
    }

    #[test]
    fn selected_remote_falls_back_to_origin_then_alphabetical_first() {
        let with_origin = repo_with_remotes(&["zeta", "origin", "alpha"]);
        assert_eq!(
            selected_remote(&with_origin, "removed").as_deref(),
            Some("origin")
        );

        let without_origin = repo_with_remotes(&["zeta", "alpha"]);
        assert_eq!(
            selected_remote(&without_origin, "removed").as_deref(),
            Some("alpha")
        );
    }

    #[test]
    fn selected_remote_is_none_when_no_remotes_are_available() {
        let repo = repo_with_remotes(&[]);
        assert_eq!(selected_remote(&repo, "origin"), None);
    }
}
