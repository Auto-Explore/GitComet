//! One decision for shell commands, independent of how an action was invoked.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellAction {
    Application,
    RepositoryEntry,
    Repository,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionDisposition {
    Allow,
    Block,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShellPolicy {
    pub gated: bool,
    pub repository_window: bool,
    pub git_available: bool,
    pub has_repository: bool,
}

pub(crate) fn action_disposition(policy: ShellPolicy, action: ShellAction) -> ActionDisposition {
    use ActionDisposition::*;
    match action {
        ShellAction::Application => Allow,
        _ if policy.gated => Block,
        ShellAction::RepositoryEntry if policy.repository_window && policy.git_available => Allow,
        ShellAction::Repository if policy.has_repository && policy.git_available => Allow,
        _ => Block,
    }
}

impl super::GitCometView {
    pub(crate) fn shell_action_allowed(&self, action: ShellAction) -> bool {
        action_disposition(
            ShellPolicy {
                gated: self.window_gated,
                repository_window: self.view_mode == super::GitCometViewMode::Normal,
                git_available: self.state.git_runtime.is_available(),
                has_repository: self.active_repo_id().is_some(),
            },
            action,
        ) == ActionDisposition::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_block_repository_actions_but_preserve_settings_and_close() {
        for repository_window in [false, true] {
            for has_repository in [false, true] {
                let policy = ShellPolicy {
                    gated: true,
                    repository_window,
                    has_repository,
                    git_available: true,
                };
                assert_eq!(
                    action_disposition(policy, ShellAction::Application),
                    ActionDisposition::Allow
                );
                assert_eq!(
                    action_disposition(policy, ShellAction::Repository),
                    ActionDisposition::Block
                );
                assert_eq!(
                    action_disposition(policy, ShellAction::RepositoryEntry),
                    ActionDisposition::Block
                );
            }
        }
    }

    #[test]
    fn home_can_open_repositories_and_focused_tools_cannot() {
        let policy = ShellPolicy {
            gated: false,
            repository_window: true,
            has_repository: false,
            git_available: true,
        };
        assert_eq!(
            action_disposition(policy, ShellAction::RepositoryEntry),
            ActionDisposition::Allow
        );
        assert_eq!(
            action_disposition(policy, ShellAction::Repository),
            ActionDisposition::Block
        );
        assert_eq!(
            action_disposition(
                ShellPolicy {
                    repository_window: false,
                    ..policy
                },
                ShellAction::RepositoryEntry
            ),
            ActionDisposition::Block
        );
        assert_eq!(
            action_disposition(
                ShellPolicy {
                    git_available: false,
                    ..policy
                },
                ShellAction::RepositoryEntry
            ),
            ActionDisposition::Block
        );
    }
}
