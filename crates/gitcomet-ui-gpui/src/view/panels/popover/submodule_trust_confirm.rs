use super::*;

const SUBMODULE_TRUST_CVE_URL: &str =
    "https://github.blog/open-source/git/git-security-vulnerabilities-announced/#cve-2022-39253";

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let Some(prompt) = this
        .state
        .submodule_trust_prompt
        .as_ref()
        .filter(|prompt| prompt.repo_id == repo_id)
        .cloned()
    else {
        // No prompt yet: if the background trust check for this repo is still
        // running, show a spinner so the popover is not an empty box. Otherwise
        // the popover is mid-teardown and renders nothing.
        if this
            .state
            .submodule_trust_check_pending
            .as_ref()
            .is_some_and(|check| check.repo_id == repo_id)
        {
            return checking_panel(theme, cx);
        }
        return div();
    };

    let (title, confirm_label, cancel_label) = match &prompt.operation {
        SubmoduleTrustPromptOperation::Add { .. } => {
            ("Trust local submodule?", "Trust and add", "Back")
        }
        SubmoduleTrustPromptOperation::Update => (
            "Trust local submodule sources?",
            "Trust and update",
            "Cancel",
        ),
        SubmoduleTrustPromptOperation::Load { .. } => {
            ("Trust local submodule sources?", "Trust and load", "Cancel")
        }
    };
    let (add_branch, add_name, add_force) = match &prompt.operation {
        SubmoduleTrustPromptOperation::Add {
            branch,
            name,
            force,
            ..
        } => (branch.clone(), name.clone(), *force),
        SubmoduleTrustPromptOperation::Update | SubmoduleTrustPromptOperation::Load { .. } => {
            (None, None, false)
        }
    };
    let sources = prompt.sources.clone();
    let operation = prompt.operation.clone();
    let scaled_px = super::popover_scaled_px_fn(cx);

    let mut dialog = ConfirmDialog::new(title, DIALOG_460_WIDTH)
        .section(
            // `ConfirmDialog` only pins a `min_w`, so an unwrapped body line grows
            // the whole dialog to its natural width. Capping this long intro
            // paragraph at the dialog width forces it to wrap and keeps the dialog
            // at 460px. Source paths below have no wrap points, so a pathologically
            // long submodule path could still widen it — acceptable for this rare
            // dialog rather than hard-pinning every ConfirmDialog to a fixed width.
            div()
                .px_2()
                .pt_1()
                .max_w(scaled_px(460.0))
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("Git blocks local file transport for submodules by default. Trusting these sources will allow GitComet to enable file transport only for this repo/source pair."),
        )
        .section(
            div().px_2().pb_1().child(
                components::Button::new("submodule_trust_cve_link", "Read about CVE-2022-39253")
                    .style(components::ButtonStyle::Filled)
                    .borderless()
                    .no_hover_border()
                    .end_slot(svg_icon(
                        "icons/open_external.svg",
                        theme.colors.accent,
                        px(14.0),
                    ))
                    .on_click(theme, cx, |_this, _e, _window, cx| {
                        cx.open_url(SUBMODULE_TRUST_CVE_URL);
                    }),
            ),
        );

    for source in sources {
        dialog = dialog.section(
            div()
                .px_2()
                .pb_1()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(format!("Submodule: {}", source.submodule_path.display())),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .child(source.display_source),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.text_muted)
                        .child(format!(
                            "Local path: {}",
                            source.local_source_path.display()
                        )),
                ),
        );
    }

    if add_branch.is_some() || add_name.is_some() || add_force {
        dialog = dialog.section(
            div()
                .px_2()
                .pb_1()
                .flex()
                .flex_col()
                .gap_0p5()
                .when_some(add_branch.clone(), |details, branch| {
                    details.child(
                        div()
                            .text_xs()
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.text_muted)
                            .child(format!("Branch: {branch}")),
                    )
                })
                .when_some(add_name.clone(), |details, name| {
                    details.child(
                        div()
                            .text_xs()
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.text_muted)
                            .child(format!("Logical name: {name}")),
                    )
                })
                .when(add_force, |details| {
                    details.child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("Force: enabled"),
                    )
                }),
        );
    }

    dialog.render(
        theme,
        cancel_button_labeled(
            "submodule_trust_cancel",
            "submodule_trust_cancel_hint",
            cancel_label,
            theme,
        )
        .on_click(theme, cx, move |this, _e, window, cx| {
            this.store.dispatch(Msg::CancelSubmoduleTrustPrompt);
            match operation.clone() {
                SubmoduleTrustPromptOperation::Add {
                    url,
                    path,
                    branch,
                    name,
                    force,
                } => {
                    let theme = this.theme;
                    let restored_branch = branch.unwrap_or_default();
                    let restored_branch_for_input = restored_branch.clone();
                    let restored_name = name.unwrap_or_default();
                    let restored_name_for_input = restored_name.clone();
                    this.submodule_url_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(&url, cx);
                        cx.notify();
                    });
                    this.submodule_path_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(path.display().to_string(), cx);
                        cx.notify();
                    });
                    this.submodule_branch_input.update(cx, move |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(&restored_branch_for_input, cx);
                        cx.notify();
                    });
                    this.submodule_name_input.update(cx, move |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(&restored_name_for_input, cx);
                        cx.notify();
                    });
                    this.submodule_add_advanced_expanded = !restored_name.is_empty() || force;
                    this.submodule_force_enabled = force;
                    this.popover = Some(PopoverKind::submodule(
                        repo_id,
                        SubmodulePopoverKind::AddPrompt,
                    ));
                    let focus = this
                        .submodule_url_input
                        .read_with(cx, |input, _| input.focus_handle());
                    window.focus(&focus, cx);
                    cx.notify();
                }
                SubmoduleTrustPromptOperation::Update
                | SubmoduleTrustPromptOperation::Load { .. } => {
                    this.close_popover(cx);
                }
            }
        }),
        components::Button::new("submodule_trust_confirm", confirm_label)
            .style(components::ButtonStyle::Filled)
            .on_click(theme, cx, |this, _e, _window, cx| {
                this.store.dispatch(Msg::ConfirmSubmoduleTrustPrompt);
                this.close_popover(cx);
            }),
        cx,
    )
}

/// Pending state shown while the background trust check runs. Matches the trust
/// dialog's width so the popover does not jump in size when the check resolves
/// into the real dialog.
fn checking_panel(theme: AppTheme, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let scaled_px = super::popover_scaled_px_fn(cx);
    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(460.0))
        .child(popover_title("Checking submodule trust…"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_3()
                .flex()
                .items_center()
                .gap_2()
                .child(crate::view::icons::svg_spinner(
                    "submodule_trust_checking_spinner",
                    theme.colors.text_muted,
                    scaled_px(16.0),
                ))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child("Checking local submodule sources…"),
                ),
        )
}
