//! The built-in commands: what the palette lists and the window runs.
//!
//! Each command has a stable string id, which scenario scripts, tests and
//! the palette's rows use; the window dispatches on the enum, so a listed
//! command without a handler, or a handler for a command nobody lists, does
//! not compile.

macro_rules! commands {
    ($($variant:ident => $id:literal,)*) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(crate) enum Command {
            $($variant,)*
        }

        impl Command {
            #[cfg(test)]
            pub(crate) const ALL: &[Command] = &[$(Command::$variant,)*];

            pub(crate) const fn id(self) -> &'static str {
                match self {
                    $(Command::$variant => $id,)*
                }
            }

            pub(crate) fn from_id(id: &str) -> Option<Self> {
                match id {
                    $($id => Some(Command::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

commands! {
    Commit => "commit",
    StageAll => "stage-all",
    UnstageAll => "unstage-all",
    CreateBranch => "create-branch",
    CheckoutBranch => "checkout-branch",
    DeleteBranch => "delete-branch",
    RenameBranch => "rename-branch",
    Rebase => "rebase",
    PruneMergedBranches => "prune-merged-branches",
    AbortMerge => "abort-merge",
    ContinueRebase => "continue-rebase",
    AbortRebase => "abort-rebase",
    Pull => "pull",
    Push => "push",
    ForcePush => "force-push",
    PushWithAnnotatedTags => "push-with-annotated-tags",
    PushWithAllTags => "push-with-all-tags",
    Stash => "stash",
    StashPop => "stash-pop",
    StashApply => "stash-apply",
    StashDrop => "stash-drop",
    OpenRepository => "open-repository",
    SwitchRepository => "switch-repository",
    CloneRepository => "clone-repository",
    InitializeRepository => "initialize-repository",
    CloseRepoTab => "close-repo-tab",
    ReloadRepository => "reload-repository",
    FetchRef => "fetch-ref",
    FetchAll => "fetch-all",
    ApplyPatch => "apply-patch",
    OpenInCodeEditor => "open-in-code-editor",
    OpenExternalTerminal => "open-external-terminal",
    ToggleSidebar => "toggle-sidebar",
    ToggleDetails => "toggle-details",
    ToggleDiffView => "toggle-diff-view",
    ToggleDiffWordWrap => "toggle-diff-word-wrap",
    ToggleLineNumbers => "toggle-line-numbers",
    ToggleWhitespaceChars => "toggle-whitespace-chars",
    ToggleTerminal => "toggle-terminal",
    PreviousRepoTab => "previous-repo-tab",
    NextRepoTab => "next-repo-tab",
    LocateFileInExplorer => "locate-file-in-explorer",
    RevealCommit => "reveal-commit",
    OpenActiveViewSearch => "open-active-view-search",
    Back => "back",
    Forward => "forward",
    CreateTag => "create-tag",
    PruneLocalTags => "prune-local-tags",
    NewWindow => "new-window",
    OpenWorkspace => "open-workspace",
    OpenSettings => "open-settings",
    Quit => "quit",
    MinimizeWindow => "minimize-window",
    Hide => "hide",
    HideOthers => "hide-others",
    ZoomWindow => "zoom-window",
    ToggleFullscreen => "toggle-fullscreen",
    IncreaseUiScale => "increase-ui-scale",
    DecreaseUiScale => "decrease-ui-scale",
    ResetUiScale => "reset-ui-scale",
    CloseWindow => "close-window",
    CheckForUpdates => "check-for-updates",
    InstallDesktopIntegration => "install-desktop-integration",
    AddRemote => "add-remote",
    OpenRemoteInBrowser => "open-remote-in-browser",
    AddSubmodule => "add-submodule",
    UpdateSubmodules => "update-submodules",
    AddWorktree => "add-worktree",
    Blame => "blame",
    ShowReflog => "show-reflog",
    FileHistory => "file-history",
    LfsDownloadAll => "lfs-download-all",
    LfsFetchAll => "lfs-fetch-all",
    LfsPrune => "lfs-prune",
    LfsFsck => "lfs-fsck",
    LfsRefreshLocks => "lfs-refresh-locks",
    LfsInstall => "lfs-install",
    AnnexSync => "annex-sync",
    AnnexPull => "annex-pull",
    AnnexPush => "annex-push",
    AnnexGetAll => "annex-get-all",
    AnnexFsck => "annex-fsck",
    AnnexAdjustUnlocked => "annex-adjust-unlocked",
    AnnexLeaveAdjusted => "annex-leave-adjusted",
    AnnexUnused => "annex-unused",
    AnnexWebapp => "annex-webapp",
    AnnexRestage => "annex-restage",
    AnnexInit => "annex-init",
}

impl Command {
    pub(crate) fn repository_requirement(self) -> gitcomet_core::domain::RepositoryRequirement {
        use gitcomet_core::domain::RepositoryRequirement::{Repository, WorkingTree};
        match self {
            Self::Commit
            | Self::StageAll
            | Self::UnstageAll
            | Self::CheckoutBranch
            | Self::Rebase
            | Self::AbortMerge
            | Self::ContinueRebase
            | Self::AbortRebase
            | Self::Pull
            | Self::Stash
            | Self::StashPop
            | Self::StashApply
            | Self::StashDrop
            | Self::ApplyPatch
            | Self::AddSubmodule
            | Self::UpdateSubmodules
            | Self::LfsDownloadAll
            | Self::AnnexAdjustUnlocked
            | Self::AnnexLeaveAdjusted
            | Self::AnnexWebapp => WorkingTree,
            Self::CreateBranch
            | Self::DeleteBranch
            | Self::RenameBranch
            | Self::PruneMergedBranches
            | Self::Push
            | Self::ForcePush
            | Self::PushWithAnnotatedTags
            | Self::PushWithAllTags
            | Self::OpenRepository
            | Self::SwitchRepository
            | Self::CloneRepository
            | Self::InitializeRepository
            | Self::CloseRepoTab
            | Self::ReloadRepository
            | Self::FetchRef
            | Self::FetchAll
            | Self::OpenInCodeEditor
            | Self::OpenExternalTerminal
            | Self::ToggleSidebar
            | Self::ToggleDetails
            | Self::ToggleDiffView
            | Self::ToggleDiffWordWrap
            | Self::ToggleLineNumbers
            | Self::ToggleWhitespaceChars
            | Self::ToggleTerminal
            | Self::PreviousRepoTab
            | Self::NextRepoTab
            | Self::LocateFileInExplorer
            | Self::RevealCommit
            | Self::OpenActiveViewSearch
            | Self::Back
            | Self::Forward
            | Self::CreateTag
            | Self::PruneLocalTags
            | Self::NewWindow
            | Self::OpenWorkspace
            | Self::OpenSettings
            | Self::Quit
            | Self::MinimizeWindow
            | Self::Hide
            | Self::HideOthers
            | Self::ZoomWindow
            | Self::ToggleFullscreen
            | Self::IncreaseUiScale
            | Self::DecreaseUiScale
            | Self::ResetUiScale
            | Self::CloseWindow
            | Self::CheckForUpdates
            | Self::InstallDesktopIntegration
            | Self::AddRemote
            | Self::OpenRemoteInBrowser
            | Self::AddWorktree
            | Self::Blame
            | Self::ShowReflog
            | Self::FileHistory
            | Self::LfsFetchAll
            | Self::LfsPrune
            | Self::LfsFsck
            | Self::LfsRefreshLocks
            | Self::LfsInstall
            | Self::AnnexSync
            | Self::AnnexPull
            | Self::AnnexPush
            | Self::AnnexGetAll
            | Self::AnnexFsck
            | Self::AnnexUnused
            | Self::AnnexRestage
            | Self::AnnexInit => Repository,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Command;
    use crate::view::command_palette::COMMANDS;

    #[test]
    fn every_command_is_listed_once_and_its_id_round_trips() {
        for command in Command::ALL {
            let listed = COMMANDS
                .iter()
                .filter(|entry| entry.id == command.id())
                .count();
            assert_eq!(listed, 1, "{command:?} must have exactly one palette row");
            assert_eq!(Command::from_id(command.id()), Some(*command));
        }
        assert_eq!(COMMANDS.len(), Command::ALL.len());
    }

    #[test]
    fn extension_and_unknown_ids_are_not_commands() {
        assert_eq!(Command::from_id("extension:review/mark"), None);
        assert_eq!(Command::from_id("merge"), None);
    }
}
