use super::*;
use crate::msg::{ReferenceEffect, RepoCommandEffect};
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::Upstream;

fn upstream_target(remote: &str, branch: &str) -> Upstream {
    Upstream {
        remote: remote.to_string(),
        branch: branch.to_string(),
    }
}

fn test_force_push_lease() -> gitcomet_core::services::ForcePushLease {
    gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: CommitId("1111111111111111111111111111111111111111".into()),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    }
}

fn test_recent_commit_message() -> gitcomet_core::domain::RecentCommitMessage {
    test_recent_commit_message_with_summary(
        "1111111111111111111111111111111111111111",
        "old message",
    )
}

fn test_recent_commit_message_with_summary(
    id: &str,
    summary: &str,
) -> gitcomet_core::domain::RecentCommitMessage {
    gitcomet_core::domain::RecentCommitMessage {
        id: CommitId(id.into()),
        summary: Arc::from(summary),
        message: format!("{summary}\n\nbody"),
    }
}

fn repo_with_head_dependent_cached_state(repo_id: RepoId) -> RepoState {
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.pending.force_push_lease = Some(test_force_push_lease());
    repo_state.set_recent_commit_messages(Loadable::Ready(vec![test_recent_commit_message()]));
    repo_state
}

// --- Revision counter regression tests ---

fn ready_working_tree_blame() -> Loadable<std::sync::Arc<Vec<gitcomet_core::services::BlameLine>>> {
    Loadable::Ready(std::sync::Arc::new(vec![
        gitcomet_core::services::BlameLine {
            commit_id: Arc::from("1111111111111111111111111111111111111111"),
            author: Arc::from("Ada"),
            author_time_unix: Some(1_700_000_000),
            summary: Arc::from("initial"),
            body: None,
            line: "let x = 1;".to_string(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        },
    ]))
}

fn repo_state_with_tags_loaded(repo_id: RepoId) -> RepoState {
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.local_actions_in_flight = 1;
    repo_state.set_tags(Loadable::Ready(vec![gitcomet_core::domain::Tag {
        name: "v1.0.0".to_string(),
        target: CommitId("abc123".into()),
    }]));
    repo_state
}

mod large_files;

mod command_completion;
mod head_changes;
mod remote_commands;
mod reword;
mod sequencers;
mod submodules;
mod tags;
mod worktrees;
