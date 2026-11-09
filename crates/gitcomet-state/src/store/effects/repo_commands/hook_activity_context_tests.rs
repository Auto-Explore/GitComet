use super::*;
use gitcomet_core::domain::CommitId;

#[test]
fn network_and_history_commands_have_specific_context() {
    assert_eq!(
        repo_command_context(&RepoCommandKind::Pull {
            mode: PullMode::Rebase,
        })
        .as_deref(),
        Some("Configured upstream → current branch · rebase")
    );
    assert_eq!(
        repo_command_context(&RepoCommandKind::PushAfterCommit {
            target: SafePushAfterCommitTarget {
                remote: "origin".to_string(),
                branch: "main".to_string(),
                local_branch: "feature/hooks".to_string(),
                local_head: CommitId("0123456789abcdef".into()),
            },
            set_upstream: false,
            hooks: GitHooks::Run,
        })
        .as_deref(),
        Some("feature/hooks → origin/main")
    );
    assert_eq!(
        repo_command_context(&RepoCommandKind::PushAfterCommit {
            target: SafePushAfterCommitTarget {
                remote: "origin".to_string(),
                branch: "main".to_string(),
                local_branch: "feature/hooks".to_string(),
                local_head: CommitId("0123456789abcdef".into()),
            },
            set_upstream: true,
            hooks: GitHooks::Skip,
        })
        .as_deref(),
        Some("feature/hooks → origin/main · skipping Git hooks")
    );
    assert_eq!(
        repo_command_context(&RepoCommandKind::SquashCommits {
            oldest: CommitId("1111111111111111".into()),
            expected_head: CommitId("2222222222222222".into()),
            message: "Squash subject\n\nBody".to_string(),
            count: 3,
        })
        .as_deref(),
        Some("3 commits · Squash subject")
    );
}

#[test]
fn remote_context_does_not_retain_a_credential_bearing_url() {
    let context = repo_command_context(&RepoCommandKind::AddRemote {
        name: "origin".to_string(),
        url: "https://token@example.invalid/private.git".to_string(),
    });
    assert_eq!(context.as_deref(), Some("origin"));
    assert!(!context.unwrap().contains("token"));
}
