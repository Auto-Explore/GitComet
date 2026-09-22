//! Git LFS entries for the status-row, Pull and Push menus. Kept together so
//! each menu shows the same wording and the same "tool missing" rule.

use super::*;
use gitcomet_core::large_files::{LargeFileCommand, LargeFilePointer};
use gitcomet_state::model::RepoState;

/// Commands are listed but disabled, with the reason in the label, when Git
/// cannot run `git lfs`; hidden entirely only when the repo has no LFS.
fn lfs_missing(state: &AppState) -> bool {
    state.large_file_tools.git_lfs.is_not_found()
}

fn entry(
    label: impl Into<SharedString>,
    icon: &'static str,
    disabled: bool,
    repo_id: RepoId,
    command: LargeFileCommand,
) -> ContextMenuItem {
    ContextMenuItem::Entry {
        label: label.into(),
        icon: Some(icon.into()),
        shortcut: None,
        disabled,
        action: Box::new(ContextMenuAction::RunLargeFileCommand { repo_id, command }),
    }
}

fn with_missing_suffix(label: &str, missing: bool) -> String {
    if missing {
        format!("{label} (install git-lfs)")
    } else {
        label.to_string()
    }
}

/// Entries for a status row. `paths` is the multi-selection or the row itself.
pub(super) fn status_file_items(
    state: &AppState,
    repo: &RepoState,
    area: DiffArea,
    path: &std::path::Path,
    paths: &[std::path::PathBuf],
    is_untracked: bool,
) -> Vec<ContextMenuItem> {
    let lfs = matches!(
        &repo.large_file_support,
        gitcomet_state::model::Loadable::Ready(support) if support.lfs.filter_configured || support.lfs.in_use()
    );
    if !lfs {
        return Vec::new();
    }
    let missing = lfs_missing(state);
    let repo_id = repo.id;
    let row = repo.large_file_state(area, path);
    let mut items = Vec::new();
    match row {
        Some(state_row) if matches!(state_row.pointer, LargeFilePointer::Lfs(_)) => {
            if state_row.content_missing() {
                items.push(entry(
                    with_missing_suffix("Download LFS content", missing),
                    "icons/arrow_down_to_line.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::LfsPull {
                        paths: paths.to_vec(),
                    },
                ));
            }
            if state_row.lockable {
                match repo.lfs_lock_for(path) {
                    Some(lock) => {
                        let owner = lock.owner.as_deref().unwrap_or("someone");
                        items.push(entry(
                            with_missing_suffix(&format!("Unlock (locked by {owner})"), missing),
                            "icons/unlink.svg",
                            missing,
                            repo_id,
                            LargeFileCommand::LfsUnlock {
                                paths: vec![path.to_path_buf()],
                                force: false,
                            },
                        ));
                        items.push(entry(
                            with_missing_suffix("Force unlock", missing),
                            "icons/warning.svg",
                            missing,
                            repo_id,
                            LargeFileCommand::LfsUnlock {
                                paths: vec![path.to_path_buf()],
                                force: true,
                            },
                        ));
                    }
                    None => items.push(entry(
                        with_missing_suffix("Lock file", missing),
                        "icons/pin.svg",
                        missing,
                        repo_id,
                        LargeFileCommand::LfsLock {
                            paths: paths.to_vec(),
                        },
                    )),
                }
            }
        }
        Some(_) => {}
        None => {
            // Tracking converts the file on the next add; tracked files are
            // re-added now so the change shows up staged.
            let renormalize = if is_untracked {
                Vec::new()
            } else {
                paths.to_vec()
            };
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
                items.push(entry(
                    with_missing_suffix(&format!("Track *.{ext} in Git LFS"), missing),
                    "icons/plus.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::LfsTrack {
                        patterns: vec![format!("*.{ext}")],
                        filename: false,
                        lockable: false,
                        renormalize: renormalize.clone(),
                    },
                ));
            }
            if let Some(pattern) = path.to_str().filter(|_| paths.len() <= 1) {
                #[cfg(windows)]
                let pattern = pattern.replace('\\', "/");
                items.push(entry(
                    with_missing_suffix("Track this file in Git LFS", missing),
                    "icons/plus.svg",
                    missing,
                    repo_id,
                    LargeFileCommand::LfsTrack {
                        patterns: vec![format!("/{pattern}")],
                        filename: true,
                        lockable: false,
                        renormalize,
                    },
                ));
            }
        }
    }
    if !items.is_empty() {
        items.insert(0, ContextMenuItem::Label("Git LFS".into()));
    }
    items
}

/// Repository-wide downloads, under the Pull split button.
pub(super) fn pull_items(state: &AppState, repo: Option<&RepoState>) -> Vec<ContextMenuItem> {
    let Some(repo) = repo.filter(|repo| lfs_in_use(repo)) else {
        return Vec::new();
    };
    let missing = lfs_missing(state);
    vec![
        ContextMenuItem::Separator,
        entry(
            with_missing_suffix("Download all LFS content", missing),
            "icons/arrow_down_to_line.svg",
            missing,
            repo.id,
            LargeFileCommand::LfsPull { paths: Vec::new() },
        ),
        entry(
            with_missing_suffix("Fetch LFS objects for all refs", missing),
            "icons/cloud.svg",
            missing,
            repo.id,
            LargeFileCommand::LfsFetchAll,
        ),
    ]
}

/// Uploads, locks and setup, under the Push split button.
pub(super) fn push_items(
    state: &AppState,
    repo: Option<&RepoState>,
    remote: Option<&str>,
) -> Vec<ContextMenuItem> {
    let Some(repo) = repo else {
        return Vec::new();
    };
    let missing = lfs_missing(state);
    let support = match &repo.large_file_support {
        gitcomet_state::model::Loadable::Ready(support) => support.clone(),
        _ => return Vec::new(),
    };
    let mut items = vec![ContextMenuItem::Separator];
    if support.lfs.in_use() && !support.lfs.filter_configured {
        // Patterns are tracked but this clone has no filters: set them up.
        items.push(entry(
            with_missing_suffix("Enable Git LFS in this repository", missing),
            "icons/check.svg",
            missing,
            repo.id,
            LargeFileCommand::LfsInstall,
        ));
    }
    if support.lfs.in_use() {
        let remote = remote.unwrap_or("origin").to_string();
        items.push(entry(
            with_missing_suffix(&format!("Push all LFS objects to {remote}"), missing),
            "icons/arrow_up_to_line.svg",
            missing,
            repo.id,
            LargeFileCommand::LfsPushAll { remote },
        ));
    }
    if support.lfs.has_lockable_patterns() {
        items.push(ContextMenuItem::Entry {
            label: "Refresh LFS locks".into(),
            icon: Some("icons/refresh.svg".into()),
            shortcut: None,
            disabled: missing,
            action: Box::new(ContextMenuAction::LoadLfsLocks { repo_id: repo.id }),
        });
    }
    if items.len() == 1 {
        return Vec::new();
    }
    items
}

fn lfs_in_use(repo: &RepoState) -> bool {
    matches!(
        &repo.large_file_support,
        gitcomet_state::model::Loadable::Ready(support) if support.lfs.in_use()
    )
}
