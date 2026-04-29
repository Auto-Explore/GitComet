use super::*;

pub(super) fn model(
    _this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
) -> ContextMenuModel {
    model_for_section(repo_id, section)
}

fn model_for_section(repo_id: RepoId, section: BranchSection) -> ContextMenuModel {
    let header: SharedString = match section {
        BranchSection::Local => "Local".into(),
        BranchSection::Remote => "Remote".into(),
    };
    let mut items = vec![ContextMenuItem::Header(header.into())];
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Switch branch".into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::BranchPicker,
        }),
    });

    if section == BranchSection::Remote {
        items.push(ContextMenuItem::Entry {
            label: "Add remote…".into(),
            icon: Some("icons/plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::remote(repo_id, RemotePopoverKind::AddPrompt),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Fetch all".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("F".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::FetchAll { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Prune merged branches".into(),
            icon: Some("icons/broom.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Prune local tags".into(),
            icon: Some("icons/tag.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
        });
    }

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_section_header_omits_remote_specific_actions() {
        let repo_id = RepoId(7);
        let model = super::model_for_section(repo_id, BranchSection::Remote);

        let labels: Vec<&str> = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.as_ref()),
                _ => None,
            })
            .collect();

        assert!(!labels.contains(&"Edit fetch URL…"));
        assert!(!labels.contains(&"Edit push URL…"));
        assert!(!labels.contains(&"Remove remote…"));
        assert!(!labels.contains(&"Delete remote branch…"));
    }
}
