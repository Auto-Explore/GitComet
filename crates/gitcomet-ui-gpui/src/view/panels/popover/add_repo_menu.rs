use super::*;

fn push_entry(
    items: &mut Vec<ContextMenuItem>,
    debug_selectors: &mut std::collections::HashMap<usize, SharedString>,
    debug_selector: &'static str,
    icon: &'static str,
    label: &'static str,
    action: AddRepoMenuAction,
) {
    let ix = items.len();
    items.push(ContextMenuItem::Entry {
        label: label.into(),
        icon: Some(icon.into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::AddRepoMenu(action)),
    });
    debug_selectors.insert(ix, debug_selector.into());
}

/// Menu behind the "+" button after the repository tabs: open, clone, or
/// initialize a repository.
pub(super) fn model() -> ContextMenuModel {
    let mut items = Vec::with_capacity(3);
    let mut debug_selectors = std::collections::HashMap::new();
    push_entry(
        &mut items,
        &mut debug_selectors,
        "add_repo_menu_open",
        "icons/disk.svg",
        crate::menu_labels::OPEN_REPOSITORY,
        AddRepoMenuAction::Open,
    );
    push_entry(
        &mut items,
        &mut debug_selectors,
        "add_repo_menu_clone",
        "icons/cloud.svg",
        crate::menu_labels::CLONE_REPOSITORY,
        AddRepoMenuAction::Clone,
    );
    push_entry(
        &mut items,
        &mut debug_selectors,
        "add_repo_menu_init",
        "icons/git_branch.svg",
        crate::menu_labels::INITIALIZE_REPOSITORY,
        AddRepoMenuAction::Initialize,
    );

    ContextMenuModel::new(items).with_entry_debug_selectors(debug_selectors)
}

pub(super) fn activate(
    this: &mut PopoverHost,
    action: AddRepoMenuAction,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match action {
        AddRepoMenuAction::Open => {
            this.close_popover_and_restore_focus(window, cx);
            let _ = this
                .root_view
                .update(cx, |root, cx| root.prompt_open_repo(window, cx));
        }
        AddRepoMenuAction::Clone => {
            let Some(anchor) = this.popover_anchor.clone() else {
                this.close_popover_and_restore_focus(window, cx);
                return;
            };
            this.open_popover(PopoverKind::CloneRepo, anchor, window, cx);
        }
        AddRepoMenuAction::Initialize => {
            this.close_popover_and_restore_focus(window, cx);
            let _ = this
                .root_view
                .update(cx, |root, cx| root.prompt_init_repo(window, cx));
        }
    }
}
