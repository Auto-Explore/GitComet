use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    target: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let target_short: SharedString = {
        let sha = target.as_str();
        let short = sha.get(0..7).unwrap_or(sha);
        if sha.len() > short.len() {
            format!("{short}…").into()
        } else {
            short.into()
        }
    };

    let overwrite_name = name.clone();
    let overwrite_target = target.clone();
    let overwrite = move |this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>| {
        this.store.dispatch(Msg::CreateBranchAndCheckout {
            repo_id,
            name: overwrite_name.clone(),
            target: overwrite_target.clone(),
            force: true,
        });
        this.close_popover(cx);
    };
    let checkout_existing_name = name.clone();
    let checkout_existing = move |this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>| {
        this.store.dispatch(Msg::CheckoutBranch {
            repo_id,
            name: checkout_existing_name.clone(),
        });
        this.close_popover(cx);
    };

    ConfirmDialog::new("Branch already exists", DIALOG_380_WIDTH)
        .text(
            theme,
            format!("A local branch named '{name}' already exists."),
        )
        .mono_value(theme, format!("Target: {target_short}"))
        .note(
            theme,
            "Overwriting moves the branch to the target commit and checks it out.",
        )
        .render(
            theme,
            dialog_cancel_button("branch_exists_cancel", "branch_exists_cancel_hint", theme, cx),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    components::Button::new("branch_exists_checkout_existing", "Checkout existing")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, {
                            let checkout_existing = checkout_existing.clone();
                            move |this, _e, _w, cx| checkout_existing(this, cx)
                        }),
                )
                .child(
                    components::Button::new("branch_exists_overwrite", "Overwrite & checkout")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| overwrite(this, cx)),
                ),
            cx,
        )
}
