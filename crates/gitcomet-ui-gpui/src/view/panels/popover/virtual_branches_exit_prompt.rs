use super::*;

/// The dialog between a checkout and a working directory the virtual branches owns.
///
/// The rule it states is the reducer's, not this dialog's: confirming
/// dispatches the checkout unchanged, and the reducer turns it into "leave
/// first" for as long as the virtual branches holds the working directory. The dialog
/// therefore cannot promise something the state does not do — it only says
/// what is about to happen, which is what a move of HEAD deserves.
pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: VirtualBranchesCheckoutTarget,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let destination = target.label();
    ConfirmDialog::new("Leave the virtual branches to change branch", DIALOG_440_WIDTH)
        .text(
            theme,
            format!(
                "The working directory is on {branch}, the branch the virtual branches rebuilds from \
                 your applied branches. Checking out '{destination}' takes it off that branch, so \
                 the virtual branches has to be left first.",
                branch = gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH,
            ),
        )
        .note(
            theme,
            "Nothing in the virtual branches is lost: applied branches and their changes stay as they \
             are, and opening the Virtual branches tab puts the working directory back on the virtual branches.",
        )
        .render(
            theme,
            cancel_button(
                "leave_virtual_branches_checkout_cancel",
                "leave_virtual_branches_checkout_cancel_hint",
                theme,
            )
            .on_click(theme, cx, |this, _e, _window, cx| {
                this.close_popover(cx);
            }),
            div().child(
                components::Button::new(
                    "leave_virtual_branches_checkout_go",
                    "Leave virtual branches and change branch",
                )
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, |this, _e, _window, cx| {
                    // Read the request back out of the popover rather than
                    // capturing it: the click handler may run again if the
                    // button is pressed twice before the close lands, and the
                    // checkout must be dispatched exactly as it was asked for.
                    if let Some(PopoverKind::LeaveVirtualBranchesToCheckoutPrompt { repo_id, target }) =
                        this.popover.as_ref()
                    {
                        let checkout = target.clone().into_msg(*repo_id);
                        this.store.dispatch(checkout);
                    }
                    this.close_popover(cx);
                }),
            ),
            cx,
        )
}
