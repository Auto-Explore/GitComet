//! git-annex entries: status rows, the Pull and Push menus, and the Annex
//! sidebar section's header and repository menus. Every entry stays listed but
//! disabled, with "(install git-annex)", when Git cannot run `git annex`.

use super::*;
use gitcomet_core::large_files::{
    AnnexAdjustMode, AnnexRepository, AnnexTrust, LargeFileCommand, LargeFilePointer,
    LargeFileSupport,
};
use gitcomet_state::model::{Loadable, RepoState};
use std::path::PathBuf;

/// Enough targets for real setups without turning the menu into a list.
const MAX_REMOTE_ENTRIES: usize = 4;

fn annex_missing(state: &AppState) -> bool {
    state.large_file_tools.git_annex.is_not_found()
}

fn label(text: &str, missing: bool) -> SharedString {
    if missing {
        format!("{text} (install git-annex)").into()
    } else {
        SharedString::from(text.to_string())
    }
}

fn entry(
    text: &str,
    icon: &'static str,
    missing: bool,
    repo_id: RepoId,
    command: LargeFileCommand,
) -> ContextMenuItem {
    ContextMenuItem::Entry {
        label: label(text, missing),
        icon: Some(icon.into()),
        shortcut: None,
        disabled: missing,
        action: Box::new(ContextMenuAction::RunLargeFileCommand { repo_id, command }),
    }
}

fn prompt_entry(
    text: &str,
    icon: &'static str,
    missing: bool,
    repo_id: RepoId,
    prompt: AnnexPrompt,
) -> ContextMenuItem {
    ContextMenuItem::Entry {
        label: label(text, missing),
        icon: Some(icon.into()),
        shortcut: None,
        disabled: missing,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::annex(repo_id, AnnexPopoverKind::Prompt(prompt)),
        }),
    }
}

fn support(repo: &RepoState) -> Option<&LargeFileSupport> {
    match &repo.large_file_support {
        Loadable::Ready(support) => Some(support.as_ref()),
        _ => None,
    }
}

fn initialized(repo: &RepoState) -> Option<&LargeFileSupport> {
    support(repo).filter(|support| support.annex.initialized())
}

/// Other repositories this clone can move content to or from by name.
fn content_remotes(support: &LargeFileSupport) -> impl Iterator<Item = &AnnexRepository> {
    support
        .annex
        .repositories
        .iter()
        .filter(|repo| !repo.here && !repo.is_builtin() && repo.remote_name.is_some())
        .take(MAX_REMOTE_ENTRIES)
}

fn everything() -> Vec<PathBuf> {
    vec![PathBuf::from(".")]
}

/// Entries for a status row. `paths` is the multi-selection or the row itself.
pub(super) fn status_file_items(
    state: &AppState,
    repo: &RepoState,
    area: DiffArea,
    path: &std::path::Path,
    paths: &[PathBuf],
    is_untracked: bool,
) -> Vec<ContextMenuItem> {
    let Some(support) = initialized(repo) else {
        return Vec::new();
    };
    let missing = annex_missing(state);
    let repo_id = repo.id;
    let mut items = Vec::new();
    match repo.large_file_state(area, path) {
        Some(row) if matches!(row.pointer, LargeFilePointer::Annex(_)) => {
            if row.in_local_store != Some(true) {
                items.push(entry(
                    "Get content",
                    "icons/arrow_down_to_line.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::AnnexGet {
                        paths: paths.to_vec(),
                        from: None,
                    },
                ));
            }
            if row.in_local_store != Some(false) {
                // git-annex refuses when too few other copies are verified.
                items.push(entry(
                    "Drop local content",
                    "icons/trash.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::AnnexDrop {
                        paths: paths.to_vec(),
                        from: None,
                        force: false,
                    },
                ));
            }
            for remote in content_remotes(support) {
                let name = remote.display_name().to_string();
                items.push(entry(
                    &format!("Copy to {name}"),
                    "icons/arrow_up_to_line.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::AnnexCopy {
                        paths: paths.to_vec(),
                        to: name.clone(),
                    },
                ));
                items.push(entry(
                    &format!("Move to {name}"),
                    "icons/arrow_right.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::AnnexMove {
                        paths: paths.to_vec(),
                        to: name,
                    },
                ));
            }
            items.push(entry(
                "Unlock (make editable)",
                "icons/pencil.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexUnlock {
                    paths: paths.to_vec(),
                },
            ));
            items.push(entry(
                "Lock",
                "icons/pin.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexLock {
                    paths: paths.to_vec(),
                },
            ));
            if row.in_local_store != Some(false) {
                items.push(prompt_entry(
                    "Drop even without other copies…",
                    "icons/warning.svg",
                    missing,
                    repo_id,
                    AnnexPrompt::ForceDrop {
                        paths: paths.to_vec(),
                    },
                ));
            }
        }
        Some(_) => {}
        None if is_untracked => items.push(entry(
            "Add to git-annex",
            "icons/plus.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexAdd {
                paths: paths.to_vec(),
            },
        )),
        None => {}
    }
    if !items.is_empty() {
        items.insert(0, ContextMenuItem::Label("git-annex".into()));
    }
    items
}

/// Under the Pull split button.
pub(super) fn pull_items(state: &AppState, repo: Option<&RepoState>) -> Vec<ContextMenuItem> {
    let Some(repo) = repo.filter(|repo| initialized(repo).is_some()) else {
        return Vec::new();
    };
    let missing = annex_missing(state);
    let content = state.large_file_settings.annex_sync_content;
    vec![
        ContextMenuItem::Separator,
        entry(
            "Pull with git-annex",
            "icons/arrow_down.svg",
            missing,
            repo.id,
            LargeFileCommand::AnnexPull { content },
        ),
        entry(
            "Get all annexed content",
            "icons/arrow_down_to_line.svg",
            missing,
            repo.id,
            LargeFileCommand::AnnexGet {
                paths: everything(),
                from: None,
            },
        ),
    ]
}

/// Under the Push split button.
pub(super) fn push_items(state: &AppState, repo: Option<&RepoState>) -> Vec<ContextMenuItem> {
    let Some(repo) = repo.filter(|repo| initialized(repo).is_some()) else {
        return Vec::new();
    };
    let missing = annex_missing(state);
    let content = state.large_file_settings.annex_sync_content;
    vec![
        ContextMenuItem::Separator,
        entry(
            "Push with git-annex",
            "icons/arrow_up.svg",
            missing,
            repo.id,
            LargeFileCommand::AnnexPush { content },
        ),
        entry(
            "Sync with git-annex (pull and push)",
            "icons/refresh.svg",
            missing,
            repo.id,
            LargeFileCommand::AnnexSync { content },
        ),
    ]
}

/// The Annex sidebar section's header menu.
pub(super) fn section_model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let state = &this.state;
    let missing = annex_missing(state);
    let repo = state.repos.iter().find(|repo| repo.id == repo_id);
    let mut items = vec![ContextMenuItem::Header("git-annex".into())];
    let Some(support) = repo.and_then(support) else {
        return ContextMenuModel::new(items);
    };
    if !support.annex.initialized() {
        items.push(entry(
            "Initialize git-annex in this clone",
            "icons/check.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexInit,
        ));
        return ContextMenuModel::new(items);
    }
    if support.annex.restage_pending {
        items.push(entry(
            "Refresh annexed files left stale",
            "icons/refresh.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexRestage,
        ));
    }
    let content = state.large_file_settings.annex_sync_content;
    items.extend([
        entry(
            "Sync with remotes",
            "icons/refresh.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexSync { content },
        ),
        entry(
            "Get all annexed content",
            "icons/arrow_down_to_line.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexGet {
                paths: everything(),
                from: None,
            },
        ),
        ContextMenuItem::Separator,
    ]);
    match repo.and_then(RepoState::annex_adjusted_branch) {
        Some((base, _)) => items.push(entry(
            &format!("Leave adjusted branch (back to {base})"),
            "icons/git_branch.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexLeaveAdjusted {
                base: base.to_string(),
            },
        )),
        None => {
            for (text, mode) in [
                ("Adjusted branch: unlock all files", AnnexAdjustMode::Unlock),
                (
                    "Adjusted branch: hide missing files",
                    AnnexAdjustMode::HideMissing,
                ),
            ] {
                items.push(entry(
                    text,
                    "icons/git_branch.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::AnnexAdjust { mode },
                ));
            }
        }
    }
    let here = support.annex.repositories.iter().find(|repo| repo.here);
    items.extend([
        ContextMenuItem::Separator,
        prompt_entry(
            "Add special remote…",
            "icons/plus.svg",
            missing,
            repo_id,
            AnnexPrompt::AddSpecialRemote,
        ),
        prompt_entry(
            "Enable special remote…",
            "icons/link.svg",
            missing,
            repo_id,
            AnnexPrompt::EnableSpecialRemote {
                name: String::new(),
            },
        ),
        prompt_entry(
            "Set number of copies…",
            "icons/copy.svg",
            missing,
            repo_id,
            AnnexPrompt::Numcopies {
                current: support.annex.numcopies,
            },
        ),
        prompt_entry(
            "Describe this repository…",
            "icons/pencil.svg",
            missing,
            repo_id,
            AnnexPrompt::Describe {
                repository: "here".into(),
                current: here
                    .map(|repo| repo.description.clone())
                    .unwrap_or_default(),
            },
        ),
        entry(
            "Check annexed content",
            "icons/check.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexFsck,
        ),
        prompt_entry(
            "Find unused content…",
            "icons/broom.svg",
            missing,
            repo_id,
            AnnexPrompt::Unused,
        ),
        ContextMenuItem::Separator,
    ]);
    // A running assistant's webapp reopens in the browser.
    items.push(prompt_entry(
        "Open git-annex webapp…",
        "icons/open_external.svg",
        missing,
        repo_id,
        AnnexPrompt::Webapp,
    ));
    if support.annex.assistant_running {
        items.push(entry(
            "Stop git-annex assistant",
            "icons/generic_close.svg",
            missing,
            repo_id,
            LargeFileCommand::AnnexStopAssistant,
        ));
    }
    ContextMenuModel::new(items)
}

/// A special remote another clone set up but this one has not enabled: it
/// has a `remote.log` name and no local remote.
pub(in crate::view) fn enable_candidate(repo: &AnnexRepository) -> Option<&str> {
    if repo.remote_name.is_some() || repo.here || repo.is_builtin() {
        return None;
    }
    repo.special_name.as_deref()
}

/// Menu for one repository or special remote row.
pub(super) fn repository_model(
    this: &PopoverHost,
    repo_id: RepoId,
    uuid: &str,
) -> ContextMenuModel {
    let state = &this.state;
    let missing = annex_missing(state);
    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(support)
        .and_then(|support| {
            support
                .annex
                .repositories
                .iter()
                .find(|repo| repo.uuid == uuid)
        })
        .cloned();
    let Some(target) = target else {
        return ContextMenuModel::new(vec![ContextMenuItem::Header("Repository".into())]);
    };
    let mut items = vec![
        ContextMenuItem::Header(target.display_name().to_string().into()),
        ContextMenuItem::Label(
            format!(
                "{} · {}",
                target.special_type.as_deref().unwrap_or("git repository"),
                target.trust.label()
            )
            .into(),
        ),
        ContextMenuItem::Separator,
    ];
    if let Some(name) = target.remote_name.clone() {
        items.extend([
            entry(
                &format!("Get all content from {name}"),
                "icons/arrow_down_to_line.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexGet {
                    paths: everything(),
                    from: Some(name.clone()),
                },
            ),
            entry(
                &format!("Copy all content to {name}"),
                "icons/arrow_up_to_line.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexCopy {
                    paths: everything(),
                    to: name.clone(),
                },
            ),
            entry(
                &format!("Move all content to {name}"),
                "icons/arrow_right.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexMove {
                    paths: everything(),
                    to: name.clone(),
                },
            ),
            entry(
                &format!("Drop all content from {name}"),
                "icons/trash.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexDrop {
                    paths: everything(),
                    from: Some(name),
                    force: false,
                },
            ),
            ContextMenuItem::Separator,
        ]);
    } else if let Some(name) = enable_candidate(&target) {
        items.extend([
            // A prompt, since some types need local settings again.
            prompt_entry(
                &format!("Enable {name} in this clone…"),
                "icons/link.svg",
                missing,
                repo_id,
                AnnexPrompt::EnableSpecialRemote {
                    name: name.to_string(),
                },
            ),
            ContextMenuItem::Separator,
        ]);
    }
    let repository = if target.here {
        "here".to_string()
    } else {
        target.uuid.clone()
    };
    for (text, trust) in [
        ("Mark as trusted", AnnexTrust::Trusted),
        ("Mark as semitrusted", AnnexTrust::Semitrusted),
        ("Mark as untrusted", AnnexTrust::Untrusted),
    ] {
        if trust != target.trust {
            items.push(entry(
                text,
                "icons/check.svg",
                missing,
                repo_id,
                LargeFileCommand::AnnexTrust {
                    repository: repository.clone(),
                    trust,
                },
            ));
        }
    }
    items.push(prompt_entry(
        "Describe…",
        "icons/pencil.svg",
        missing,
        repo_id,
        AnnexPrompt::Describe {
            repository,
            current: target.description.clone(),
        },
    ));
    ContextMenuModel::new(items)
}
