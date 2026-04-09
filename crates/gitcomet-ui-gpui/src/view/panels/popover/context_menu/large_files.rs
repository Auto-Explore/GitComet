use super::*;
use gitcomet_core::services::RepoLargeFileCapabilities;

fn repo_capabilities(this: &PopoverHost) -> Option<(RepoId, RepoLargeFileCapabilities, bool)> {
    let repo = this.active_repo()?;
    let capabilities = match &repo.large_file_capabilities {
        Loadable::Ready(capabilities) => *capabilities,
        _ => RepoLargeFileCapabilities::default(),
    };
    Some((repo.id, capabilities, repo.large_file_capabilities.is_loading()))
}

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let repo_id = this.active_repo_id().unwrap_or(RepoId(0));
    let (capabilities, capabilities_loading) = repo_capabilities(this)
        .map(|(_, capabilities, loading)| (capabilities, loading))
        .unwrap_or((RepoLargeFileCapabilities::default(), false));
    let disabled = this.active_repo_id().is_none() || capabilities_loading;

    let lfs_tool_available = !disabled && capabilities.git_lfs_available;
    let annex_tool_available = !disabled && capabilities.git_annex_available;

    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Large files".into()),
        ContextMenuItem::Label(
            if capabilities_loading {
                "Detecting git-lfs / git-annex support…"
            } else {
                "git-lfs"
            }
            .into(),
        ),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "LFS fetch".into(),
            icon: Some("icons/cloud.svg".into()),
            shortcut: Some("F".into()),
            disabled: !lfs_tool_available || !capabilities.uses_git_lfs,
            action: Box::new(ContextMenuAction::LfsFetch { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "LFS pull".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("P".into()),
            disabled: !lfs_tool_available || !capabilities.uses_git_lfs,
            action: Box::new(ContextMenuAction::LfsPull { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "LFS track…".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: Some("T".into()),
            disabled: !lfs_tool_available,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::LfsPatternPrompt {
                    repo_id,
                    kind: LfsPatternPromptKind::Track,
                },
            }),
        },
        ContextMenuItem::Entry {
            label: "LFS untrack…".into(),
            icon: Some("icons/unlink.svg".into()),
            shortcut: Some("U".into()),
            disabled: !lfs_tool_available || !capabilities.uses_git_lfs,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::LfsPatternPrompt {
                    repo_id,
                    kind: LfsPatternPromptKind::Untrack,
                },
            }),
        },
        ContextMenuItem::Entry {
            label: "LFS prune".into(),
            icon: Some("icons/broom.svg".into()),
            shortcut: None,
            disabled: !lfs_tool_available || !capabilities.uses_git_lfs,
            action: Box::new(ContextMenuAction::LfsPrune { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "LFS migrate import…".into(),
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("M".into()),
            disabled: !lfs_tool_available,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::LfsPatternPrompt {
                    repo_id,
                    kind: LfsPatternPromptKind::MigrateImport,
                },
            }),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Label("git-annex".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Annex init".into(),
            icon: Some("icons/box.svg".into()),
            shortcut: Some("I".into()),
            disabled: !annex_tool_available,
            action: Box::new(ContextMenuAction::AnnexInit { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Annex sync".into(),
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("S".into()),
            disabled: !annex_tool_available || !capabilities.uses_git_annex,
            action: Box::new(ContextMenuAction::AnnexSync { repo_id }),
        },
    ])
}
