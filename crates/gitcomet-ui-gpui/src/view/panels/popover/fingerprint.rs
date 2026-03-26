use super::*;
use crate::view::fingerprint as view_fingerprint;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

pub(super) fn notify_fingerprint(state: &AppState, popover: &PopoverKind) -> u64 {
    let mut hasher = FxHasher::default();
    hash_popover_kind(popover, &mut hasher);

    match popover {
        PopoverKind::CloneRepo => match &state.clone {
            None => 0u8.hash(&mut hasher),
            Some(clone) => {
                1u8.hash(&mut hasher);
                clone.seq.hash(&mut hasher);
                clone.url.hash(&mut hasher);
                clone.dest.hash(&mut hasher);
                match &clone.status {
                    CloneOpStatus::Running => 0u8.hash(&mut hasher),
                    CloneOpStatus::FinishedOk => 1u8.hash(&mut hasher),
                    CloneOpStatus::FinishedErr(err) => {
                        2u8.hash(&mut hasher);
                        err.hash(&mut hasher);
                    }
                }
            }
        },
        PopoverKind::RecentRepositoryPicker => {
            state.active_repo.hash(&mut hasher);
            for path in session::load().recent_repos {
                path.hash(&mut hasher);
            }
        }
        PopoverKind::RepoPicker => {
            state.active_repo.hash(&mut hasher);
            state.repos.len().hash(&mut hasher);
            // Repo picker list is usually small; hashing all ids+workdirs is fine and avoids stale lists.
            for repo in &state.repos {
                repo.id.hash(&mut hasher);
                repo.spec.workdir.hash(&mut hasher);
                view_fingerprint::hash_loadable_kind(&repo.open, &mut hasher);
            }
        }
        PopoverKind::Settings
        | PopoverKind::SettingsThemeMenu
        | PopoverKind::SettingsDateFormatMenu
        | PopoverKind::SettingsTimezoneMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::OpenSourceLicenses
        | PopoverKind::AppMenu => {
            // Mostly local UI state; depend only on whether a repo is active/open.
            state.active_repo.hash(&mut hasher);
            if let Some(repo) = repo_for_popover(state, popover) {
                view_fingerprint::hash_loadable_kind(&repo.open, &mut hasher);
            }
        }
        _ => {
            if let Some(repo) = repo_for_popover(state, popover) {
                hash_repo_for_popover(repo, popover, &mut hasher);
            } else {
                state.active_repo.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

fn repo_for_popover<'a>(state: &'a AppState, popover: &PopoverKind) -> Option<&'a RepoState> {
    let repo_id = match popover {
        PopoverKind::RepoPicker
        | PopoverKind::RecentRepositoryPicker
        | PopoverKind::CloneRepo
        | PopoverKind::Settings
        | PopoverKind::SettingsThemeMenu
        | PopoverKind::SettingsDateFormatMenu
        | PopoverKind::SettingsTimezoneMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::OpenSourceLicenses => None,

        // Popovers that implicitly use the currently active repo.
        PopoverKind::BranchPicker
        | PopoverKind::CreateBranch
        | PopoverKind::StashPrompt
        | PopoverKind::PullPicker
        | PopoverKind::PushPicker
        | PopoverKind::AppMenu
        | PopoverKind::DiffHunks
        | PopoverKind::HistoryColumnSettings
        | PopoverKind::ConflictResolverInputRowMenu { .. }
        | PopoverKind::ConflictResolverChunkMenu { .. }
        | PopoverKind::ConflictResolverOutputMenu { .. } => state.active_repo,

        // Popovers that carry an explicit repo id.
        PopoverKind::CreateBranchFromRefPrompt { repo_id, .. }
        | PopoverKind::ResetPrompt { repo_id, .. }
        | PopoverKind::CheckoutRemoteBranchPrompt { repo_id, .. }
        | PopoverKind::StashDropConfirm { repo_id, .. }
        | PopoverKind::StashMenu { repo_id, .. }
        | PopoverKind::CreateTagPrompt { repo_id, .. }
        | PopoverKind::Repo { repo_id, .. }
        | PopoverKind::FileHistory { repo_id, .. }
        | PopoverKind::PushSetUpstreamPrompt { repo_id, .. }
        | PopoverKind::ForcePushConfirm { repo_id }
        | PopoverKind::MergeAbortConfirm { repo_id }
        | PopoverKind::ConflictSaveStageConfirm { repo_id, .. }
        | PopoverKind::ForceDeleteBranchConfirm { repo_id, .. }
        | PopoverKind::ForceRemoveWorktreeConfirm { repo_id, .. }
        | PopoverKind::DiscardChangesConfirm { repo_id, .. }
        | PopoverKind::PullReconcilePrompt { repo_id }
        | PopoverKind::DiffHunkMenu { repo_id, .. }
        | PopoverKind::DiffEditorMenu { repo_id, .. }
        | PopoverKind::CommitMenu { repo_id, .. }
        | PopoverKind::StatusFileMenu { repo_id, .. }
        | PopoverKind::BranchMenu { repo_id, .. }
        | PopoverKind::BranchSectionMenu { repo_id, .. }
        | PopoverKind::CommitFileMenu { repo_id, .. }
        | PopoverKind::TagMenu { repo_id, .. }
        | PopoverKind::HistoryBranchFilter { repo_id } => Some(*repo_id),
    }?;

    state.repos.iter().find(|r| r.id == repo_id)
}

fn hash_repo_for_popover<H: Hasher>(repo: &RepoState, popover: &PopoverKind, hasher: &mut H) {
    view_fingerprint::hash_loadable_kind(&repo.open, hasher);

    match popover {
        PopoverKind::BranchPicker
        | PopoverKind::CreateBranch
        | PopoverKind::CreateBranchFromRefPrompt { .. }
        | PopoverKind::BranchMenu { .. }
        | PopoverKind::BranchSectionMenu { .. }
        | PopoverKind::ForceDeleteBranchConfirm { .. }
        | PopoverKind::PushSetUpstreamPrompt { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Remote(_),
            ..
        } => {
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(_),
            ..
        } => {
            repo.worktrees_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(_),
            ..
        } => {
            repo.submodules_rev.hash(hasher);
        }

        PopoverKind::StashPrompt => {
            repo.stashes_rev.hash(hasher);
            view_fingerprint::hash_loadable_arc(&repo.status, hasher);
        }
        PopoverKind::StashDropConfirm { .. } | PopoverKind::StashMenu { .. } => {
            repo.stashes_rev.hash(hasher);
        }

        PopoverKind::FileHistory { .. } => {
            repo.history_state.file_history_path.hash(hasher);
            view_fingerprint::hash_loadable_arc(&repo.history_state.file_history, hasher);
        }

        PopoverKind::DiffHunks
        | PopoverKind::DiffHunkMenu { .. }
        | PopoverKind::DiffEditorMenu { .. }
        | PopoverKind::DiscardChangesConfirm { .. } => {
            repo.diff_state.diff_rev.hash(hasher);
            if let Some(t) = repo.diff_state.diff_target.as_ref() {
                view_fingerprint::hash_diff_target(t, hasher)
            }
            view_fingerprint::hash_loadable_arc(&repo.diff_state.diff, hasher);
            repo.diff_state.diff_file_rev.hash(hasher);
            view_fingerprint::hash_loadable_kind(&repo.diff_state.diff_file, hasher);
            view_fingerprint::hash_loadable_kind(&repo.diff_state.diff_file_image, hasher);

            // Working tree diff popovers need status for file-kind/conflict decisions.
            if matches!(
                repo.diff_state.diff_target,
                Some(DiffTarget::WorkingTree { .. })
            ) {
                view_fingerprint::hash_loadable_arc(&repo.status, hasher);
            }
        }

        PopoverKind::HistoryBranchFilter { .. } => {
            repo.history_state.history_scope.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
        }

        PopoverKind::PullPicker
        | PopoverKind::PushPicker
        | PopoverKind::PullReconcilePrompt { .. }
        | PopoverKind::ForcePushConfirm { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
        }

        PopoverKind::TagMenu { .. } => {
            repo.tags_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_tags_rev.hash(hasher);
        }

        // Most prompt-style popovers don't require live state updates.
        PopoverKind::MergeAbortConfirm { .. }
        | PopoverKind::ConflictSaveStageConfirm { .. }
        | PopoverKind::ResetPrompt { .. }
        | PopoverKind::CheckoutRemoteBranchPrompt { .. }
        | PopoverKind::CreateTagPrompt { .. }
        | PopoverKind::ForceRemoveWorktreeConfirm { .. }
        | PopoverKind::CommitMenu { .. }
        | PopoverKind::CommitFileMenu { .. }
        | PopoverKind::StatusFileMenu { .. }
        | PopoverKind::HistoryColumnSettings
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::ConflictResolverInputRowMenu { .. }
        | PopoverKind::ConflictResolverChunkMenu { .. }
        | PopoverKind::ConflictResolverOutputMenu { .. }
        | PopoverKind::AppMenu
        | PopoverKind::Settings
        | PopoverKind::SettingsThemeMenu
        | PopoverKind::SettingsDateFormatMenu
        | PopoverKind::SettingsTimezoneMenu
        | PopoverKind::OpenSourceLicenses
        | PopoverKind::RepoPicker
        | PopoverKind::RecentRepositoryPicker
        | PopoverKind::CloneRepo => {}
    }
}

fn hash_popover_kind<H: Hasher>(kind: &PopoverKind, hasher: &mut H) {
    match kind {
        PopoverKind::RepoPicker => 0u8.hash(hasher),
        PopoverKind::RecentRepositoryPicker => 65u8.hash(hasher),
        PopoverKind::BranchPicker => 1u8.hash(hasher),
        PopoverKind::CreateBranch => 2u8.hash(hasher),
        PopoverKind::CreateBranchFromRefPrompt { repo_id, target } => {
            66u8.hash(hasher);
            repo_id.hash(hasher);
            target.hash(hasher);
        }
        PopoverKind::CheckoutRemoteBranchPrompt {
            repo_id,
            remote,
            branch,
        } => {
            50u8.hash(hasher);
            repo_id.hash(hasher);
            remote.hash(hasher);
            branch.hash(hasher);
        }
        PopoverKind::StashPrompt => 3u8.hash(hasher),
        PopoverKind::StashDropConfirm {
            repo_id,
            index,
            message,
        } => {
            55u8.hash(hasher);
            repo_id.hash(hasher);
            index.hash(hasher);
            message.hash(hasher);
        }
        PopoverKind::StashMenu {
            repo_id,
            index,
            message,
        } => {
            56u8.hash(hasher);
            repo_id.hash(hasher);
            index.hash(hasher);
            message.hash(hasher);
        }
        PopoverKind::CloneRepo => 4u8.hash(hasher),
        PopoverKind::Settings => 5u8.hash(hasher),
        PopoverKind::SettingsThemeMenu => 62u8.hash(hasher),
        PopoverKind::SettingsDateFormatMenu => 63u8.hash(hasher),
        PopoverKind::SettingsTimezoneMenu => 64u8.hash(hasher),
        PopoverKind::ChangeTrackingSettings => 66u8.hash(hasher),
        PopoverKind::OpenSourceLicenses => 60u8.hash(hasher),

        PopoverKind::ResetPrompt {
            repo_id,
            target,
            mode,
        } => {
            6u8.hash(hasher);
            repo_id.hash(hasher);
            target.hash(hasher);
            hash_reset_mode(*mode, hasher);
        }
        PopoverKind::CreateTagPrompt { repo_id, target } => {
            8u8.hash(hasher);
            repo_id.hash(hasher);
            target.hash(hasher);
        }
        PopoverKind::Repo { repo_id, kind } => {
            hash_repo_popover_kind(*repo_id, kind, hasher);
        }

        PopoverKind::FileHistory { repo_id, path } => {
            28u8.hash(hasher);
            repo_id.hash(hasher);
            path.hash(hasher);
        }
        PopoverKind::PushSetUpstreamPrompt { repo_id, remote } => {
            30u8.hash(hasher);
            repo_id.hash(hasher);
            remote.hash(hasher);
        }
        PopoverKind::ForcePushConfirm { repo_id } => {
            31u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::ForceDeleteBranchConfirm { repo_id, name } => {
            32u8.hash(hasher);
            repo_id.hash(hasher);
            name.hash(hasher);
        }
        PopoverKind::ForceRemoveWorktreeConfirm { repo_id, path } => {
            61u8.hash(hasher);
            repo_id.hash(hasher);
            path.hash(hasher);
        }
        PopoverKind::DiscardChangesConfirm {
            repo_id,
            area,
            path,
        } => {
            34u8.hash(hasher);
            repo_id.hash(hasher);
            hash_diff_area(*area, hasher);
            path.hash(hasher);
        }
        PopoverKind::PullReconcilePrompt { repo_id } => {
            35u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::PullPicker => 36u8.hash(hasher),
        PopoverKind::PushPicker => 37u8.hash(hasher),
        PopoverKind::AppMenu => 38u8.hash(hasher),
        PopoverKind::DiffHunks => 39u8.hash(hasher),
        PopoverKind::DiffHunkMenu { repo_id, src_ix } => {
            40u8.hash(hasher);
            repo_id.hash(hasher);
            src_ix.hash(hasher);
        }
        PopoverKind::DiffEditorMenu {
            repo_id,
            area,
            path,
            hunks_count,
            lines_count,
            ..
        } => {
            41u8.hash(hasher);
            repo_id.hash(hasher);
            hash_diff_area(*area, hasher);
            path.hash(hasher);
            hunks_count.hash(hasher);
            lines_count.hash(hasher);
        }
        PopoverKind::ConflictResolverInputRowMenu {
            line_label,
            line_target,
            chunk_label,
            chunk_target,
        } => {
            53u8.hash(hasher);
            line_label.hash(hasher);
            line_target.hash(hasher);
            chunk_label.hash(hasher);
            chunk_target.hash(hasher);
        }
        PopoverKind::ConflictResolverChunkMenu {
            conflict_ix,
            has_base,
            is_three_way,
            selected_choices,
            output_line_ix,
        } => {
            59u8.hash(hasher);
            conflict_ix.hash(hasher);
            has_base.hash(hasher);
            is_three_way.hash(hasher);
            selected_choices.hash(hasher);
            output_line_ix.hash(hasher);
        }
        PopoverKind::ConflictResolverOutputMenu {
            cursor_line,
            selected_text,
            has_source_a,
            has_source_b,
            has_source_c,
            is_three_way,
        } => {
            54u8.hash(hasher);
            cursor_line.hash(hasher);
            selected_text.hash(hasher);
            has_source_a.hash(hasher);
            has_source_b.hash(hasher);
            has_source_c.hash(hasher);
            is_three_way.hash(hasher);
        }
        PopoverKind::CommitMenu { repo_id, commit_id } => {
            42u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
        }
        PopoverKind::StatusFileMenu {
            repo_id,
            area,
            path,
        } => {
            43u8.hash(hasher);
            repo_id.hash(hasher);
            hash_diff_area(*area, hasher);
            path.hash(hasher);
        }
        PopoverKind::BranchMenu {
            repo_id,
            section,
            name,
        } => {
            44u8.hash(hasher);
            repo_id.hash(hasher);
            hash_branch_section(*section, hasher);
            name.hash(hasher);
        }
        PopoverKind::BranchSectionMenu { repo_id, section } => {
            45u8.hash(hasher);
            repo_id.hash(hasher);
            hash_branch_section(*section, hasher);
        }
        PopoverKind::CommitFileMenu {
            repo_id,
            commit_id,
            path,
        } => {
            46u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
            path.hash(hasher);
        }
        PopoverKind::TagMenu { repo_id, commit_id } => {
            47u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
        }
        PopoverKind::HistoryBranchFilter { repo_id } => {
            48u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::HistoryColumnSettings => 49u8.hash(hasher),
        PopoverKind::MergeAbortConfirm { repo_id } => {
            51u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::ConflictSaveStageConfirm {
            repo_id,
            path,
            has_conflict_markers,
            unresolved_blocks,
        } => {
            52u8.hash(hasher);
            repo_id.hash(hasher);
            path.hash(hasher);
            has_conflict_markers.hash(hasher);
            unresolved_blocks.hash(hasher);
        }
    }
}

fn hash_repo_popover_kind<H: Hasher>(repo_id: RepoId, kind: &RepoPopoverKind, hasher: &mut H) {
    match kind {
        RepoPopoverKind::Remote(remote_kind) => match remote_kind {
            RemotePopoverKind::AddPrompt => {
                9u8.hash(hasher);
                repo_id.hash(hasher);
            }
            RemotePopoverKind::EditUrlPrompt { name, kind } => {
                13u8.hash(hasher);
                repo_id.hash(hasher);
                name.hash(hasher);
                hash_remote_url_kind(*kind, hasher);
            }
            RemotePopoverKind::RemoveConfirm { name } => {
                14u8.hash(hasher);
                repo_id.hash(hasher);
                name.hash(hasher);
            }
            RemotePopoverKind::Menu { name } => {
                15u8.hash(hasher);
                repo_id.hash(hasher);
                name.hash(hasher);
            }
            RemotePopoverKind::DeleteBranchConfirm { remote, branch } => {
                33u8.hash(hasher);
                repo_id.hash(hasher);
                remote.hash(hasher);
                branch.hash(hasher);
            }
        },
        RepoPopoverKind::Worktree(worktree_kind) => match worktree_kind {
            WorktreePopoverKind::SectionMenu => {
                16u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::Menu { path } => {
                17u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
            WorktreePopoverKind::AddPrompt => {
                20u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::OpenPicker => {
                21u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::RemovePicker => {
                22u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::RemoveConfirm { path } => {
                23u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
        },
        RepoPopoverKind::Submodule(submodule_kind) => match submodule_kind {
            SubmodulePopoverKind::SectionMenu => {
                18u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::Menu { path } => {
                19u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
            SubmodulePopoverKind::AddPrompt => {
                24u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::OpenPicker => {
                25u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::RemovePicker => {
                26u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::RemoveConfirm { path } => {
                27u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
        },
    }
}

fn hash_diff_area<H: Hasher>(area: DiffArea, hasher: &mut H) {
    match area {
        DiffArea::Staged => 0u8.hash(hasher),
        DiffArea::Unstaged => 1u8.hash(hasher),
    }
}

fn hash_branch_section<H: Hasher>(section: BranchSection, hasher: &mut H) {
    match section {
        BranchSection::Local => 0u8.hash(hasher),
        BranchSection::Remote => 1u8.hash(hasher),
    }
}

fn hash_remote_url_kind<H: Hasher>(kind: RemoteUrlKind, hasher: &mut H) {
    match kind {
        RemoteUrlKind::Fetch => 0u8.hash(hasher),
        RemoteUrlKind::Push => 1u8.hash(hasher),
    }
}

fn hash_reset_mode<H: Hasher>(mode: ResetMode, hasher: &mut H) {
    match mode {
        ResetMode::Soft => 0u8.hash(hasher),
        ResetMode::Mixed => 1u8.hash(hasher),
        ResetMode::Hard => 2u8.hash(hasher),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, Upstream};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";

    fn hash_kind(kind: PopoverKind) -> u64 {
        let mut hasher = FxHasher::default();
        hash_popover_kind(&kind, &mut hasher);
        hasher.finish()
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-ui-popover-fingerprint-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn run_subtest_with_session_env(filter: &str, session_file: &Path) {
        let current_exe = std::env::current_exe().expect("locate current test binary");
        let output = Command::new(current_exe)
            .arg(filter)
            .arg("--nocapture")
            .env(SESSION_FILE_ENV, session_file)
            .output()
            .expect("spawn subtest process");
        assert!(
            output.status.success(),
            "subtest {filter} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn grouped_repo_popover_hash_changes_with_nested_payload() {
        let repo_id = RepoId(7);
        let hash_origin = hash_kind(PopoverKind::remote(
            repo_id,
            RemotePopoverKind::Menu {
                name: "origin".to_string(),
            },
        ));
        let hash_upstream = hash_kind(PopoverKind::remote(
            repo_id,
            RemotePopoverKind::Menu {
                name: "upstream".to_string(),
            },
        ));

        assert_ne!(hash_origin, hash_upstream);
    }

    #[test]
    fn grouped_repo_popover_resolves_explicit_repo_id() {
        let repo_id = RepoId(42);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_repo_popover_repo_for_popover"),
            },
        );
        let state = AppState {
            repos: vec![repo],
            active_repo: None,
            ..Default::default()
        };

        let popover = PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu);
        let resolved = repo_for_popover(&state, &popover).expect("expected repo lookup to work");

        assert_eq!(resolved.id, repo_id);
    }

    #[test]
    fn pull_picker_fingerprint_changes_when_branches_rev_changes() {
        let repo_id = RepoId(9);
        let mut repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_pull_picker_fingerprint"),
            },
        );
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            divergence: None,
        }]));

        let mut state = AppState::default();
        state.active_repo = Some(repo_id);
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::PullPicker);
        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        let after = notify_fingerprint(&state, &PopoverKind::PullPicker);

        assert_ne!(before, after);
    }

    #[test]
    fn recent_repository_picker_fingerprint_changes_with_active_repo() {
        let base = AppState::default();
        let with_active_repo = AppState {
            active_repo: Some(RepoId(99)),
            ..Default::default()
        };

        let before = notify_fingerprint(&base, &PopoverKind::RecentRepositoryPicker);
        let after = notify_fingerprint(&with_active_repo, &PopoverKind::RecentRepositoryPicker);

        assert_ne!(before, after);
    }

    #[test]
    fn recent_repository_picker_fingerprint_changes_with_recent_repos_wrapper() {
        if std::env::var_os(SESSION_FILE_ENV).is_some() {
            return;
        }

        let dir = unique_temp_dir("recent-repo-wrapper");
        let session_file = dir.join("session.json");
        session::persist_recent_repo_to_path(Path::new("/tmp/repo-a"), &session_file)
            .expect("seed recent repo session");

        run_subtest_with_session_env(
            "recent_repository_picker_fingerprint_changes_with_recent_repos_subprocess",
            &session_file,
        );
    }

    #[test]
    fn recent_repository_picker_fingerprint_changes_with_recent_repos_subprocess() {
        if std::env::var_os(SESSION_FILE_ENV).is_none() {
            return;
        }

        let before = notify_fingerprint(&AppState::default(), &PopoverKind::RecentRepositoryPicker);
        let session_file =
            PathBuf::from(std::env::var_os(SESSION_FILE_ENV).expect("session file env"));
        session::persist_recent_repo_to_path(Path::new("/tmp/repo-b"), &session_file)
            .expect("append recent repo");
        let after = notify_fingerprint(&AppState::default(), &PopoverKind::RecentRepositoryPicker);

        assert_ne!(before, after);
    }
}
