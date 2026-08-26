use super::*;

fn display_target(target: &str) -> SharedString {
    let is_full_object_id =
        matches!(target.len(), 40 | 64) && target.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_full_object_id {
        format!("{}…", &target[..7]).into()
    } else {
        target.to_owned().into()
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    name: String,
    target: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let target_display = display_target(&target);
    let target_label: SharedString = format!("Target: {target_display}").into();
    let mut target_text = components::TruncatedText::new(target_label)
        .id("branch_exists_target_text")
        .text_sm()
        .text_color(theme.colors.foreground.secondary);
    if target_display.as_ref() == target.as_str() {
        target_text = target_text.full_text_tooltip(this.tooltip_host.clone());
    }
    let dialog_width = DIALOG_540_WIDTH.preferred_px(popover_ui_scale(cx));

    ConfirmDialog::new("Branch already exists", DIALOG_540_WIDTH)
        .text(
            theme,
            format!("A local branch named '{name}' already exists."),
        )
        .section(
            div()
                .debug_selector(|| "branch_exists_target".to_string())
                .px_2()
                .py_1()
                .min_w(px(0.0))
                .max_w(dialog_width)
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .child(target_text.render(cx)),
        )
        .note(
            theme,
            "Overwriting moves the branch to the target commit and checks it out.",
        )
        .render(
            theme,
            cancel_button("branch_exists_cancel", "branch_exists_cancel_hint", theme).on_click(
                theme,
                cx,
                |this, _e, _w, cx| {
                    this.resolve_open_branch_exists_prompt(BranchExistsChoice::Cancel);
                    this.close_popover(cx);
                },
            ),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    components::Button::new("branch_exists_checkout_existing", "Checkout existing")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.resolve_open_branch_exists_prompt(
                                BranchExistsChoice::CheckoutExisting,
                            );
                            this.close_popover(cx);
                        })
                        .debug_selector(|| "branch_exists_checkout_existing".to_string()),
                )
                .child(
                    components::Button::new("branch_exists_overwrite", "Overwrite & checkout")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.resolve_open_branch_exists_prompt(
                                BranchExistsChoice::OverwriteAndCheckout,
                            );
                            this.close_popover(cx);
                        })
                        .debug_selector(|| "branch_exists_overwrite".to_string()),
                ),
            cx,
        )
}

#[cfg(test)]
mod tests {
    use super::display_target;

    #[test]
    fn abbreviates_only_full_object_ids() {
        assert_eq!(
            display_target("0123456789abcdef0123456789abcdef01234567").as_ref(),
            "0123456…"
        );
        assert_eq!(
            display_target("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .as_ref(),
            "0123456…"
        );
    }

    #[test]
    fn preserves_revision_names_and_short_hex_values() {
        assert_eq!(
            display_target("origin/feature-one").as_ref(),
            "origin/feature-one"
        );
        assert_eq!(
            display_target("origin/feature-two").as_ref(),
            "origin/feature-two"
        );
        assert_eq!(display_target("deadbeef").as_ref(), "deadbeef");
    }
}
