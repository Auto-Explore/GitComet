use super::*;
use gitcomet_core::services::CommandOutput;

fn summary_for(branches: Vec<String>) -> String {
    let (_message, summary) = super::summarize_command(
        &RepoCommandKind::DeleteRemoteBranches {
            remote: "origin".into(),
            branches,
        },
        &CommandOutput::empty_success("git push --delete"),
        true,
        None,
    );
    summary
}

#[test]
fn summary_pluralises_on_the_branch_count() {
    assert_eq!(
        summary_for(vec!["feat/a".into()]),
        "1 remote branch on origin: Deleted"
    );
    assert_eq!(
        summary_for(vec!["feat/a".into(), "feat/b".into()]),
        "2 remote branches on origin: Deleted"
    );
}
