use super::*;

/// Chooser shown when one compact history chip represents multiple exact refs.
/// Selecting a row hands off to the existing single-ref branch menu, keeping
/// all branch operations and their validation in one implementation.
pub(super) fn model(
    repo_id: RepoId,
    display_name: &str,
    targets: &[BranchMenuTarget],
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Branch references".into()),
        ContextMenuItem::Label(display_name.to_string().into()),
        ContextMenuItem::Description("Choose the exact reference to act on.".into()),
        ContextMenuItem::Separator,
    ];

    items.extend(targets.iter().map(|target| {
        ContextMenuItem::Entry {
            label: target.display_name().into(),
            icon: Some(
                match target.section() {
                    BranchSection::Local => "icons/computer.svg",
                    BranchSection::Remote => "icons/cloud.svg",
                }
                .into(),
            ),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: target.popover_kind(repo_id),
            }),
        }
    }));

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_preserves_full_ref_names_and_routes_each_section() {
        let repo_id = RepoId(7);
        let targets = vec![
            BranchMenuTarget::local("feature/x"),
            BranchMenuTarget::remote("origin", "feature/x"),
        ];

        let model = model(repo_id, "feature/x", &targets);
        let entries = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    label,
                    icon,
                    shortcut,
                    action,
                    ..
                } => Some((label, icon, shortcut, action.as_ref())),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0.as_ref(), "feature/x");
        assert_eq!(entries[0].1.as_deref(), Some("icons/computer.svg"));
        assert!(entries[0].2.is_none());
        assert!(matches!(
            entries[0].3,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::BranchMenu {
                    repo_id: routed_repo,
                    target: BranchMenuTarget::Local { name },
                }
            } if *routed_repo == repo_id && name == "feature/x"
        ));

        assert_eq!(entries[1].0.as_ref(), "origin/feature/x");
        assert_eq!(entries[1].1.as_deref(), Some("icons/cloud.svg"));
        assert!(entries[1].2.is_none());
        assert!(matches!(
            entries[1].3,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::BranchMenu {
                    repo_id: routed_repo,
                    target: BranchMenuTarget::Remote { remote, branch },
                }
            } if *routed_repo == repo_id && remote == "origin" && branch == "feature/x"
        ));
    }
}
