use super::*;
use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::ref_comparison::*;

fn oid(dir: &Path, reference: &str) -> CommitId {
    let output = crate::util::git_workdir_cmd_for(dir)
        .args(["rev-parse", reference])
        .output()
        .unwrap();
    assert!(output.status.success());
    CommitId(String::from_utf8(output.stdout).unwrap().trim().into())
}

#[test]
fn shared_counts_equal_independent_immutable_oid_walks() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "base", "base\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "base"]);
    let base = oid(dir, "HEAD");
    git_success(dir, &["checkout", "-b", "topic"]);
    write_file(dir, "topic", "topic\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "topic"]);
    let topic = oid(dir, "HEAD");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "target", "target\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "target"]);
    let target = oid(dir, "HEAD");
    git_success(
        dir,
        &["update-ref", "refs/remotes/origin/dev", target.as_ref()],
    );
    git_success(
        dir,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/dev",
        ],
    );
    let repo = open_repo(dir);
    let cancellation = CancellationToken::new();
    let request = RefComparisonRequest {
        target: repo.ref_comparison_target(None, &cancellation).unwrap(),
        subjects: vec![base, topic, target.clone()],
        depth: RefComparisonDepth::Counts,
        budget: RefComparisonBudget::automatic(),
    };
    let result = repo
        .analyze_ref_comparisons(&request, &cancellation)
        .unwrap();
    assert_eq!(result.results.len(), 3);
    for result in &result.results {
        let range = format!("{}...{}", result.subject, target);
        let output = crate::util::git_workdir_cmd_for(dir)
            .args(["rev-list", "--left-right", "--count", &range])
            .output()
            .unwrap();
        assert!(output.status.success());
        let counts: Vec<u64> = String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(
            result.counts,
            RefComparisonCounts::Exact {
                ahead: counts[0],
                behind: counts[1]
            }
        );
    }
    assert_eq!(
        result.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::Merged)
    );
    assert_eq!(
        result.results[2].integration,
        RefIntegration::Confirmed(IntegrationEvidence::UpToDate)
    );
}

#[test]
fn shared_walk_handles_skewed_dates_merges_disconnected_roots_and_all_128_memberships() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    let tree = oid(dir, "HEAD^{tree}");
    let mut tips: Vec<CommitId> = Vec::new();
    for index in 0..150 {
        let mut command = crate::test_support::git_command();
        command.current_dir(dir).args([
            "commit-tree",
            tree.as_ref(),
            "-m",
            &format!("node {index}"),
        ]);
        command.env(
            "GIT_AUTHOR_DATE",
            format!("@{} +0000", 1_000_000_000 + (150 - index) * 86400),
        );
        command.env(
            "GIT_COMMITTER_DATE",
            format!("@{} +0000", 1_000_000_000 + (150 - index) * 86400),
        );
        if index > 0 && index != 75 {
            command.args(["-p", tips[index - 1].as_ref()]);
        }
        if index > 5 && index % 3 == 0 && index != 75 {
            command.args(["-p", tips[index / 3].as_ref()]);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        tips.push(CommitId(
            String::from_utf8(output.stdout).unwrap().trim().into(),
        ));
    }
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let target = repo
        .ref_comparison_target(Some(tips[149].as_ref()), &token)
        .unwrap();
    let request = RefComparisonRequest {
        target,
        subjects: tips[..128].to_vec(),
        depth: RefComparisonDepth::Counts,
        budget: RefComparisonBudget::automatic(),
    };
    let batch = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(batch.results.len(), 128);
    for result in batch.results {
        let output = crate::test_support::git_output(
            dir,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{}...{}", result.subject, request.target.oid),
            ],
        );
        assert!(output.status.success());
        let counts: Vec<u64> = String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(
            result.counts,
            RefComparisonCounts::Exact {
                ahead: counts[0],
                behind: counts[1]
            }
        );
    }
    let warm = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(warm.graph_reads, 0);
    assert_eq!(warm.cache_hits, 128);
}

#[test]
fn default_resolution_overrides_and_graph_context_changes_never_reuse_false_confirmation() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let missing = repo.ref_comparison_target(None, &token).unwrap_err();
    assert!(missing.to_string().contains("origin/HEAD is unavailable"));
    let target = repo.ref_comparison_target(Some("HEAD"), &token).unwrap();
    let request = RefComparisonRequest {
        target: target.clone(),
        subjects: vec![target.oid.clone()],
        depth: RefComparisonDepth::Counts,
        budget: RefComparisonBudget::automatic(),
    };
    let exact = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        exact.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::UpToDate)
    );
    std::fs::write(dir.join(".git/shallow"), format!("{}\n", target.oid)).unwrap();
    let incomplete = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        incomplete.results[0].counts,
        RefComparisonCounts::Unavailable(RefComparisonReason::IncompleteHistory)
    );
    std::fs::remove_file(dir.join(".git/shallow")).unwrap();
    std::fs::create_dir_all(dir.join(".git/info")).unwrap();
    std::fs::write(dir.join(".git/info/grafts"), format!("{}\n", target.oid)).unwrap();
    let grafted = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        grafted.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::Unsupported)
    );
    std::fs::remove_file(dir.join(".git/info/grafts")).unwrap();
    let restored = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        restored.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::UpToDate)
    );
}

#[test]
fn explicit_deeper_counts_retry_deferred_automatic_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let target = repo.ref_comparison_target(Some("HEAD"), &token).unwrap();
    let mut request = RefComparisonRequest {
        target: target.clone(),
        subjects: vec![target.oid.clone()],
        depth: RefComparisonDepth::Counts,
        budget: RefComparisonBudget {
            elapsed: std::time::Duration::ZERO,
            ..RefComparisonBudget::automatic()
        },
    };
    let deferred = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        deferred.results[0].counts,
        RefComparisonCounts::Unavailable(RefComparisonReason::Deadline)
    );
    request.budget = RefComparisonBudget::deep();
    let complete = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        complete.results[0].counts,
        RefComparisonCounts::Exact {
            ahead: 0,
            behind: 0
        }
    );
    token.cancel();
    assert!(matches!(
        repo.analyze_ref_comparisons(&request, &token)
            .unwrap_err()
            .kind(),
        ErrorKind::Cancelled
    ));
}
