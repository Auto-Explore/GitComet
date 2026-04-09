use super::*;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    area: DiffArea,
    path: &std::path::PathBuf,
    cx: &gpui::Context<PopoverHost>,
) -> ContextMenuModel {
    let (use_selection, selected_count) = {
        let pane = this.details_pane.read(cx);
        let selection = pane
            .status_multi_selection
            .get(&repo_id)
            .map(|sel| sel.selected_paths_for_area(area))
            .unwrap_or(&[]);

        let use_selection = selection.len() > 1 && selection.iter().any(|p| p == path);
        let selected_count = if use_selection { selection.len() } else { 1 };
        (use_selection, selected_count)
    };

    let (is_conflicted, is_unstaged_conflicted, has_unstaged_for_path, is_staged_added) = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| match &r.status {
            Loadable::Ready(status) => {
                let unstaged_kind = status
                    .unstaged
                    .iter()
                    .find(|s| &s.path == path)
                    .map(|s| s.kind);
                let staged_kind = status
                    .staged
                    .iter()
                    .find(|s| &s.path == path)
                    .map(|s| s.kind);

                Some((
                    matches!(
                        unstaged_kind,
                        Some(gitcomet_core::domain::FileStatusKind::Conflicted)
                    ) || matches!(
                        staged_kind,
                        Some(gitcomet_core::domain::FileStatusKind::Conflicted)
                    ),
                    matches!(
                        unstaged_kind,
                        Some(gitcomet_core::domain::FileStatusKind::Conflicted)
                    ),
                    unstaged_kind.is_some(),
                    matches!(
                        staged_kind,
                        Some(gitcomet_core::domain::FileStatusKind::Added)
                    ),
                ))
            }
            _ => None,
        })
        .unwrap_or((false, false, false, false));

    let (
        large_file_capabilities,
        large_file_path_info,
    ): (
        Option<gitcomet_core::services::RepoLargeFileCapabilities>,
        Option<Loadable<gitcomet_core::services::PathLargeFileInfo>>,
    ) = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .map(|repo| {
            (
                match &repo.large_file_capabilities {
                    Loadable::Ready(capabilities) => Some(*capabilities),
                    _ => None,
                },
                repo.large_file_path_info(path).cloned(),
            )
        })
        .unwrap_or((None, None));

    // Keep context menu opening fast. Validate precisely when the action runs instead.
    let can_discard_worktree_changes = if is_conflicted {
        false
    } else {
        match area {
            DiffArea::Unstaged => true,
            DiffArea::Staged => has_unstaged_for_path || is_staged_added,
        }
    };

    let mut items = vec![ContextMenuItem::Header(
        path.file_name()
            .and_then(|p| p.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{path:?}"))
            .into(),
    )];
    items.push(ContextMenuItem::Label(path.display().to_string().into()));
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: "Open diff".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: if area == DiffArea::Unstaged && is_unstaged_conflicted {
            Box::new(ContextMenuAction::SelectConflictDiff {
                repo_id,
                path: path.clone(),
            })
        } else {
            Box::new(ContextMenuAction::SelectDiff {
                repo_id,
                target: DiffTarget::WorkingTree {
                    path: path.clone(),
                    area,
                },
            })
        },
    });
    items.push(ContextMenuItem::Entry {
        label: "Open file".into(),
        icon: Some("icons/file.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFile {
            repo_id,
            path: path.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Open file location".into(),
        icon: Some("icons/folder.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFileLocation {
            repo_id,
            path: path.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "File history".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: Some("H".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory {
                repo_id,
                path: path.clone(),
            },
        }),
    });
    if is_conflicted {
        items.push(ContextMenuItem::Separator);
        let n = selected_count;
        items.push(ContextMenuItem::Entry {
            label: if use_selection {
                format!("Resolve selected using ours ({n})").into()
            } else {
                "Resolve using ours".into()
            },
            icon: Some("icons/arrow_left.svg".into()),
            shortcut: Some("O".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::CheckoutConflictSideSelectionOrPath {
                repo_id,
                area,
                path: path.clone(),
                side: gitcomet_core::services::ConflictSide::Ours,
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: if use_selection {
                format!("Resolve selected using theirs ({n})").into()
            } else {
                "Resolve using theirs".into()
            },
            icon: Some("icons/arrow_right.svg".into()),
            shortcut: Some("T".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::CheckoutConflictSideSelectionOrPath {
                repo_id,
                area,
                path: path.clone(),
                side: gitcomet_core::services::ConflictSide::Theirs,
            }),
        });

        let can_manual = !use_selection;
        items.push(ContextMenuItem::Entry {
            label: if can_manual {
                "Resolve manually…".into()
            } else {
                "Resolve manually… (select 1 file)".into()
            },
            icon: Some("icons/pencil.svg".into()),
            shortcut: Some("M".into()),
            disabled: !can_manual,
            action: Box::new(ContextMenuAction::SelectConflictDiff {
                repo_id,
                path: path.clone(),
            }),
        });
        if area == DiffArea::Unstaged && is_unstaged_conflicted {
            let can_launch_external_mergetool = !use_selection;
            items.push(ContextMenuItem::Entry {
                label: if can_launch_external_mergetool {
                    "Open external mergetool".into()
                } else {
                    "Open external mergetool (select 1 file)".into()
                },
                icon: Some("icons/open_external.svg".into()),
                shortcut: None,
                disabled: !can_launch_external_mergetool,
                action: Box::new(ContextMenuAction::LaunchMergetool {
                    repo_id,
                    path: path.clone(),
                }),
            });
        }
    } else {
        match area {
            DiffArea::Unstaged => items.push(ContextMenuItem::Entry {
                label: if use_selection {
                    format!("Stage ({})", selected_count).into()
                } else {
                    "Stage".into()
                },
                icon: Some("icons/plus.svg".into()),
                shortcut: Some("S".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::StageSelectionOrPath {
                    repo_id,
                    area,
                    path: path.clone(),
                }),
            }),
            DiffArea::Staged => items.push(ContextMenuItem::Entry {
                label: if use_selection {
                    format!("Unstage ({})", selected_count).into()
                } else {
                    "Unstage".into()
                },
                icon: Some("icons/minus.svg".into()),
                shortcut: Some("U".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::UnstageSelectionOrPath {
                    repo_id,
                    area,
                    path: path.clone(),
                }),
            }),
        };
    }

    let show_discard_changes = !(is_conflicted && area == DiffArea::Staged);
    if show_discard_changes {
        items.push(ContextMenuItem::Entry {
            label: if use_selection {
                format!("Discard ({})", selected_count).into()
            } else {
                "Discard changes".into()
            },
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("D".into()),
            disabled: !can_discard_worktree_changes,
            action: Box::new(ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
                repo_id,
                area,
                path: path.clone(),
            }),
        });
    }

    let annex_available = large_file_capabilities.is_some_and(|cap| cap.git_annex_available);
    match large_file_path_info {
        Some(Loadable::Ready(info)) => match info.kind {
            gitcomet_core::services::LargeFilePathKind::Plain => {}
            gitcomet_core::services::LargeFilePathKind::GitLfs => {
                items.push(ContextMenuItem::Separator);
                items.push(ContextMenuItem::Label("Git LFS tracked file".into()));
            }
            gitcomet_core::services::LargeFilePathKind::GitAnnexLocked => {
                items.push(ContextMenuItem::Separator);
                items.push(ContextMenuItem::Label("git-annex locked file".into()));
                items.push(ContextMenuItem::Entry {
                    label: "Annex get".into(),
                    icon: Some("icons/cloud.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexGet {
                        repo_id,
                        path: path.clone(),
                    }),
                });
                items.push(ContextMenuItem::Entry {
                    label: "Annex unlock".into(),
                    icon: Some("icons/link.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexUnlock {
                        repo_id,
                        path: path.clone(),
                    }),
                });
                items.push(ContextMenuItem::Entry {
                    label: "Annex drop".into(),
                    icon: Some("icons/minus.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexDrop {
                        repo_id,
                        path: path.clone(),
                    }),
                });
            }
            gitcomet_core::services::LargeFilePathKind::GitAnnexUnlocked => {
                items.push(ContextMenuItem::Separator);
                items.push(ContextMenuItem::Label("git-annex unlocked file".into()));
                items.push(ContextMenuItem::Entry {
                    label: "Annex get".into(),
                    icon: Some("icons/cloud.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexGet {
                        repo_id,
                        path: path.clone(),
                    }),
                });
                items.push(ContextMenuItem::Entry {
                    label: "Annex lock".into(),
                    icon: Some("icons/unlink.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexLock {
                        repo_id,
                        path: path.clone(),
                    }),
                });
                items.push(ContextMenuItem::Entry {
                    label: "Annex add".into(),
                    icon: Some("icons/plus.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexAdd {
                        repo_id,
                        path: path.clone(),
                    }),
                });
                items.push(ContextMenuItem::Entry {
                    label: "Annex drop".into(),
                    icon: Some("icons/minus.svg".into()),
                    shortcut: None,
                    disabled: !annex_available,
                    action: Box::new(ContextMenuAction::AnnexDrop {
                        repo_id,
                        path: path.clone(),
                    }),
                });
            }
        },
        Some(Loadable::Loading)
            if large_file_capabilities.is_some_and(|cap| cap.uses_large_files()) =>
        {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Label("Detecting large-file info…".into()));
        }
        Some(Loadable::Error(_))
            if large_file_capabilities.is_some_and(|cap| cap.uses_large_files()) =>
        {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Label("Large-file info unavailable".into()));
        }
        _ => {}
    }

    items.push(ContextMenuItem::Separator);
    let copy_path_text = this
        .resolve_workdir_path(repo_id, path)
        .map(|p| path_text_for_copy(&p))
        .unwrap_or_else(|_| path_text_for_copy(path));
    items.push(ContextMenuItem::Entry {
        label: "Copy path".into(),
        icon: Some("icons/copy.svg".into()),
        shortcut: Some("C".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::CopyText {
            text: copy_path_text,
        }),
    });

    ContextMenuModel::new(items)
}
