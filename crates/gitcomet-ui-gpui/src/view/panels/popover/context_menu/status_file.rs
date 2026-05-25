use super::*;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    area: DiffArea,
    path: &std::path::Path,
    cx: &gpui::Context<PopoverHost>,
) -> ContextMenuModel {
    let (use_selection, selected_count) = {
        let pane = this.details_pane.read(cx);
        let selection = pane
            .status_multi_selection
            .get(&repo_id)
            .map(|sel| sel.selected_paths_for_area(area))
            .unwrap_or(&[]);

        let use_selection = selection.len() > 1 && selection.iter().any(|p| p.as_path() == path);
        let selected_count = if use_selection { selection.len() } else { 1 };
        (use_selection, selected_count)
    };

    let (is_conflicted, is_unstaged_conflicted, has_unstaged_for_path, is_staged_added) = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .map(|repo| {
            let unstaged_kind = repo
                .status_entry_for_path(DiffArea::Unstaged, path)
                .map(|status| status.kind);
            let staged_kind = repo
                .status_entry_for_path(DiffArea::Staged, path)
                .map(|status| status.kind);

            (
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
            )
        })
        .unwrap_or((false, false, false, false));

    let submodule_menu_state = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| {
            let status_entry = repo.status_entry_for_path(area, path)?;
            if status_entry.kind == gitcomet_core::domain::FileStatusKind::Untracked {
                return None;
            }

            let menu_state = submodule::menu_state(this, repo_id, path);
            if menu_state.status.is_some() || repo.spec.workdir.join(path).is_dir() {
                Some(menu_state)
            } else {
                None
            }
        });

    if let Some(submodule_menu_state) = submodule_menu_state {
        return submodule_status_model(
            this,
            repo_id,
            area,
            path,
            use_selection,
            selected_count,
            is_conflicted,
            has_unstaged_for_path,
            is_staged_added,
            submodule_menu_state,
        );
    }

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
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: "Open diff".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: if area == DiffArea::Unstaged && is_unstaged_conflicted {
            Box::new(ContextMenuAction::SelectConflictDiff {
                repo_id,
                path: path.to_path_buf(),
            })
        } else {
            Box::new(ContextMenuAction::SelectDiff {
                repo_id,
                target: DiffTarget::WorkingTree {
                    path: path.to_path_buf(),
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
            path: path.to_path_buf(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Open file location".into(),
        icon: Some("icons/folder.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFileLocation {
            repo_id,
            path: path.to_path_buf(),
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
                path: path.to_path_buf(),
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
                path: path.to_path_buf(),
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
                path: path.to_path_buf(),
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
                path: path.to_path_buf(),
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
                    path: path.to_path_buf(),
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
                    path: path.to_path_buf(),
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
                    path: path.to_path_buf(),
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
                path: path.to_path_buf(),
            }),
        });
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

#[allow(clippy::too_many_arguments)]
fn submodule_status_model(
    this: &PopoverHost,
    repo_id: RepoId,
    area: DiffArea,
    path: &std::path::Path,
    use_selection: bool,
    selected_count: usize,
    is_conflicted: bool,
    has_unstaged_for_path: bool,
    is_staged_added: bool,
    submodule_menu_state: submodule::SubmoduleMenuState,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Submodule".into())];
    items.push(ContextMenuItem::Label(path.display().to_string().into()));
    if let Some(status_label) = submodule::status_label(submodule_menu_state.status) {
        items.push(ContextMenuItem::Label(status_label.into()));
    }
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: "Open submodule".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: !submodule_menu_state.can_open,
        action: Box::new(ContextMenuAction::OpenRepo {
            path: submodule_menu_state.open_path.clone().unwrap_or_default(),
        }),
    });
    if submodule_menu_state.show_load {
        items.push(ContextMenuItem::Entry {
            label: "Load submodule".into(),
            icon: Some("icons/plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::LoadSubmodule {
                repo_id,
                path: path.to_path_buf(),
            }),
        });
    }
    items.push(ContextMenuItem::Entry {
        label: "Change pointer…".into(),
        icon: Some("icons/swap.svg".into()),
        shortcut: None,
        disabled: !submodule_menu_state.can_change_pointer,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(
                repo_id,
                SubmodulePopoverKind::ChangePointerPrompt {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Remove…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(
                repo_id,
                SubmodulePopoverKind::RemoveConfirm {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });

    if !is_conflicted {
        items.push(ContextMenuItem::Separator);
        match area {
            DiffArea::Unstaged => items.push(ContextMenuItem::Entry {
                label: if use_selection {
                    format!("Stage ({selected_count})").into()
                } else {
                    "Stage".into()
                },
                icon: Some("icons/plus.svg".into()),
                shortcut: Some("S".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::StageSelectionOrPath {
                    repo_id,
                    area,
                    path: path.to_path_buf(),
                }),
            }),
            DiffArea::Staged => items.push(ContextMenuItem::Entry {
                label: if use_selection {
                    format!("Unstage ({selected_count})").into()
                } else {
                    "Unstage".into()
                },
                icon: Some("icons/minus.svg".into()),
                shortcut: Some("U".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::UnstageSelectionOrPath {
                    repo_id,
                    area,
                    path: path.to_path_buf(),
                }),
            }),
        }
    }

    let can_discard_worktree_changes = if is_conflicted {
        false
    } else {
        match area {
            DiffArea::Unstaged => true,
            DiffArea::Staged => has_unstaged_for_path || is_staged_added,
        }
    };
    if !(is_conflicted && area == DiffArea::Staged) {
        items.push(ContextMenuItem::Entry {
            label: if use_selection {
                format!("Discard ({selected_count})").into()
            } else {
                "Discard changes".into()
            },
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("D".into()),
            disabled: !can_discard_worktree_changes,
            action: Box::new(ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
                repo_id,
                area,
                path: path.to_path_buf(),
            }),
        });
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
