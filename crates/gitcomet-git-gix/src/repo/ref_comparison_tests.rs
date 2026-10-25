use super::*;
use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::ref_comparison::*;

static COMPARISON_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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

#[test]
fn strict_content_evidence_keeps_unique_commit_counts_separate() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "file", "base\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "base"]);
    let base = oid(dir, "HEAD");
    write_file(dir, "file", "topic\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "topic"]);
    let topic = oid(dir, "HEAD");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "file", "topic\n");
    write_file(dir, "unrelated", "target only\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "squashed with unrelated changes"]);
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let request = RefComparisonRequest {
        target: repo.ref_comparison_target(Some("HEAD"), &token).unwrap(),
        subjects: vec![topic],
        depth: RefComparisonDepth::Evidence,
        budget: RefComparisonBudget::automatic(),
    };
    let batch = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        batch.results[0].counts,
        RefComparisonCounts::Exact {
            ahead: 1,
            behind: 1
        }
    );
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::ChangesMatch)
    );
}

fn commit_all(dir: &Path, message: &str) -> CommitId {
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", message]);
    oid(dir, "HEAD")
}

#[test]
fn squash_patch_evidence_is_historical_and_reuses_completed_counts() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed", "base\n");
    let base = commit_all(dir, "base");
    write_file(dir, "a", "a\n");
    commit_all(dir, "first topic commit");
    write_file(dir, "b", "b\n");
    let topic = commit_all(dir, "second topic commit");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "a", "a\n");
    write_file(dir, "b", "b\n");
    commit_all(dir, "squash");
    write_file(dir, "a", "edited later\n");
    commit_all(dir, "later target edit");
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let mut request = RefComparisonRequest {
        target: repo.ref_comparison_target(Some("HEAD"), &token).unwrap(),
        subjects: vec![topic],
        depth: RefComparisonDepth::Counts,
        budget: RefComparisonBudget::automatic(),
    };
    let counts = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        counts.results[0].counts,
        RefComparisonCounts::Exact {
            ahead: 2,
            behind: 2
        }
    );
    request.depth = RefComparisonDepth::Evidence;
    let batch = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(batch.results[0].counts, counts.results[0].counts);
    assert_eq!(
        batch.graph_reads, 0,
        "evidence must reuse the completed shared count walk"
    );
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::SquashPatch)
    );
}

fn evidence_at_head(repo: &GixRepo, subjects: Vec<CommitId>) -> RefComparisonBatch {
    let token = CancellationToken::new();
    repo.analyze_ref_comparisons(
        &RefComparisonRequest {
            target: repo.ref_comparison_target(Some("HEAD"), &token).unwrap(),
            subjects,
            depth: RefComparisonDepth::Evidence,
            budget: RefComparisonBudget::automatic(),
        },
        &token,
    )
    .unwrap()
}

#[test]
fn individual_patch_evidence_requires_every_unique_commit_and_preserves_later_edits() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed", "base\n");
    let base = commit_all(dir, "base");
    write_file(dir, "a", "a\n");
    let first = commit_all(dir, "first");
    write_file(dir, "b", "b\n");
    let topic = commit_all(dir, "second");
    write_file(dir, "c", "later local\n");
    let later = commit_all(dir, "later local");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "unrelated", "base movement\n");
    commit_all(dir, "target first");
    git_success(dir, &["cherry-pick", first.as_ref()]);
    write_file(dir, "a", "edited later\n");
    commit_all(dir, "target edit");
    let partial = evidence_at_head(&open_repo(dir), vec![topic.clone()]);
    assert_eq!(
        partial.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
    git_success(dir, &["cherry-pick", topic.as_ref()]);
    let batch = evidence_at_head(&open_repo(dir), vec![topic, later]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::IndividualCommits)
    );
    assert_eq!(
        batch.results[1].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
}

#[test]
fn whitespace_sensitive_patches_and_reverted_target_patches_are_not_confirmed() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "file", "before\n");
    let base = commit_all(dir, "base");
    write_file(dir, "file", " value\n");
    let topic = commit_all(dir, "topic");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "file", "  value\n");
    commit_all(dir, "different whitespace");
    let batch = evidence_at_head(&open_repo(dir), vec![topic.clone()]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "target-only", "different parent\n");
    commit_all(dir, "different target parent");
    git_success(dir, &["cherry-pick", topic.as_ref()]);
    git_success(dir, &["revert", "--no-edit", "HEAD"]);
    let batch = evidence_at_head(&open_repo(dir), vec![topic]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
}

#[test]
fn no_net_changes_handles_empty_commits_and_branch_reverts() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "file", "base\n");
    let base = commit_all(dir, "base");
    git_success(dir, &["commit", "--allow-empty", "-m", "empty"]);
    let empty = oid(dir, "HEAD");
    write_file(dir, "file", "topic\n");
    commit_all(dir, "topic");
    git_success(dir, &["revert", "--no-edit", "HEAD"]);
    let reverted = oid(dir, "HEAD");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "other", "target\n");
    commit_all(dir, "target");
    let batch = evidence_at_head(&open_repo(dir), vec![empty, reverted]);
    for result in batch.results {
        assert_eq!(
            result.integration,
            RefIntegration::Confirmed(IntegrationEvidence::NoNetChanges)
        );
    }
}

#[cfg(unix)]
#[test]
fn content_evidence_compares_binary_rename_modes_symlink_gitlink_and_deletion_objects() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use std::os::unix::fs::{PermissionsExt, symlink};
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "original", "same\n");
    write_file(dir, "delete", "gone\n");
    std::fs::write(dir.join("binary"), [0, 1, 2]).unwrap();
    symlink("original", dir.join("link")).unwrap();
    let base = commit_all(dir, "base");
    let apply = || {
        git_success(dir, &["mv", "original", "renamed"]);
        std::fs::set_permissions(dir.join("renamed"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::remove_file(dir.join("delete")).unwrap();
        std::fs::write(dir.join("binary"), [0, 7, 8, 9]).unwrap();
        std::fs::remove_file(dir.join("link")).unwrap();
        symlink("renamed", dir.join("link")).unwrap();
        git_success(dir, &["add", "-A"]);
        git_success(
            dir,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},sub", base),
            ],
        );
    };
    apply();
    git_success(dir, &["commit", "-m", "topic"]);
    let topic = oid(dir, "HEAD");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    apply();
    write_file(dir, "unrelated", "target only\n");
    git_success(dir, &["add", "unrelated"]);
    git_success(dir, &["commit", "-m", "different history"]);
    let matched = evidence_at_head(&open_repo(dir), vec![topic.clone()]);
    assert_eq!(
        matched.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::ChangesMatch)
    );
    std::fs::set_permissions(dir.join("renamed"), std::fs::Permissions::from_mode(0o644)).unwrap();
    commit_all(dir, "mode differs");
    let different = evidence_at_head(&open_repo(dir), vec![topic]);
    assert_eq!(
        different.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
}

#[test]
fn historical_limits_defer_once_and_deeper_checks_expand_shared_coverage() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use std::io::Write as _;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "file", "base\n");
    let base = commit_all(dir, "base");
    write_file(dir, "file", "topic\n");
    let topic = commit_all(dir, "topic");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "file", "topic\n");
    commit_all(dir, "squash with different identity");
    write_file(dir, "file", "later edit\n");
    let tip = commit_all(dir, "later edit");
    let mut input = String::new();
    for index in 0..1_005 {
        let message = format!("empty {index}");
        let parent = if index == 0 {
            tip.to_string()
        } else {
            format!(":{index}")
        };
        input.push_str(&format!("commit refs/heads/long-history\nmark :{}\ncommitter A <a@example.com> {} +0000\ndata {}\n{message}\nfrom {parent}\n\n", index + 1, 1_000_000_000 + index, message.len()));
    }
    let mut child = crate::test_support::git_command()
        .current_dir(dir)
        .args(["fast-import", "--quiet"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let mut request = RefComparisonRequest {
        target: repo
            .ref_comparison_target(Some("long-history"), &token)
            .unwrap(),
        subjects: vec![topic],
        depth: RefComparisonDepth::Evidence,
        budget: RefComparisonBudget::automatic(),
    };
    let automatic = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        automatic.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::Budget)
    );
    let warm = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(warm.cache_hits, 1);
    request.depth = RefComparisonDepth::Deep;
    request.budget = RefComparisonBudget::deep();
    let deep = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        deep.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::SquashPatch)
    );
    assert_eq!(deep.results[0].counts, automatic.results[0].counts);
    assert_eq!(deep.graph_reads, 0);
}

fn tree_commit(dir: &Path, tree: &CommitId, parents: &[CommitId], message: &str) -> CommitId {
    let mut command = crate::test_support::git_command();
    command
        .current_dir(dir)
        .args(["commit-tree", tree.as_ref(), "-m", message]);
    for parent in parents {
        command.args(["-p", parent.as_ref()]);
    }
    let output = command.output().unwrap();
    assert!(output.status.success());
    CommitId(String::from_utf8(output.stdout).unwrap().trim().into())
}

#[test]
fn criss_cross_bases_do_not_prove_content_or_skip_unique_merge_commits() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "file", "base\n");
    let base = commit_all(dir, "base");
    let tree = oid(dir, "HEAD^{tree}");
    let left = tree_commit(dir, &tree, std::slice::from_ref(&base), "left");
    let right = tree_commit(dir, &tree, std::slice::from_ref(&base), "right");
    let subject = tree_commit(dir, &tree, &[left.clone(), right.clone()], "subject merge");
    let target = tree_commit(dir, &tree, &[right, left], "target merge");
    let bases = crate::test_support::git_output(
        dir,
        &["merge-base", "--all", target.as_ref(), subject.as_ref()],
    );
    assert!(bases.status.success());
    assert_eq!(String::from_utf8(bases.stdout).unwrap().lines().count(), 2);
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let batch = repo
        .analyze_ref_comparisons(
            &RefComparisonRequest {
                target: repo
                    .ref_comparison_target(Some(target.as_ref()), &token)
                    .unwrap(),
                subjects: vec![subject],
                depth: RefComparisonDepth::Evidence,
                budget: RefComparisonBudget::automatic(),
            },
            &token,
        )
        .unwrap();
    assert_eq!(
        batch.results[0].counts,
        RefComparisonCounts::Exact {
            ahead: 1,
            behind: 1
        }
    );
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::UniqueMerge)
    );
}

#[test]
fn repeated_branch_patches_cannot_reuse_one_historical_target_commit() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed", "base\n");
    let base = commit_all(dir, "base");
    write_file(dir, "a", "a\n");
    let first = commit_all(dir, "first patch");
    git_success(dir, &["revert", "--no-edit", "HEAD"]);
    write_file(dir, "a", "a\n");
    commit_all(dir, "same patch again");
    write_file(dir, "b", "b\n");
    let topic = commit_all(dir, "another patch");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "target-only", "different parent\n");
    commit_all(dir, "different parent");
    git_success(dir, &["cherry-pick", first.as_ref()]);
    git_success(dir, &["cherry-pick", topic.as_ref()]);
    write_file(dir, "a", "later target edit\n");
    commit_all(dir, "later edit");
    let batch = evidence_at_head(&open_repo(dir), vec![topic]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
}

#[cfg(unix)]
#[test]
fn historical_matching_disables_external_diff_text_conversion_and_index_writes() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    let canary = dir.join(".git/helper-called");
    let script = dir.join(".git/diff-helper");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf called >> '{}'\nprintf converted\n",
            canary.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    git_success(dir, &["config", "diff.external", script.to_str().unwrap()]);
    git_success(
        dir,
        &["config", "diff.canary.textconv", script.to_str().unwrap()],
    );
    write_file(dir, ".gitattributes", "*.txt diff=canary\n");
    write_file(dir, "seed", "base\n");
    let base = commit_all(dir, "base");
    write_file(dir, "a.txt", "a\n");
    commit_all(dir, "first");
    write_file(dir, "b.txt", "b\n");
    let topic = commit_all(dir, "second");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "a.txt", "a\n");
    write_file(dir, "b.txt", "b\n");
    commit_all(dir, "squash");
    write_file(dir, "a.txt", "later\n");
    commit_all(dir, "later");
    let index = std::fs::read(dir.join(".git/index")).unwrap();
    let config = std::fs::read(dir.join(".git/config")).unwrap();
    let batch = evidence_at_head(&open_repo(dir), vec![topic]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::Confirmed(IntegrationEvidence::SquashPatch)
    );
    assert!(!canary.exists());
    assert_eq!(std::fs::read(dir.join(".git/index")).unwrap(), index);
    assert_eq!(std::fs::read(dir.join(".git/config")).unwrap(), config);
    assert!(!dir.join(".git/FETCH_HEAD").exists());
}

#[cfg(unix)]
#[test]
fn missing_promisor_objects_do_not_trigger_lazy_fetch() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "file", "base\n");
    let base = commit_all(dir, "base");
    write_file(dir, "file", "topic\n");
    let topic = commit_all(dir, "topic");
    let blob = oid(dir, "HEAD:file");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    write_file(dir, "file", "different target\n");
    commit_all(dir, "target");
    let canary = dir.join(".git/fetch-called");
    let script = dir.join(".git/remote-helper");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf called >> '{}'\nexit 1\n",
            canary.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    git_success(dir, &["config", "protocol.ext.allow", "always"]);
    git_success(
        dir,
        &[
            "config",
            "remote.origin.url",
            &format!("ext::{}", script.display()),
        ],
    );
    git_success(dir, &["config", "remote.origin.promisor", "true"]);
    git_success(
        dir,
        &["config", "remote.origin.partialclonefilter", "blob:none"],
    );
    let object_path = dir
        .join(".git/objects")
        .join(&blob.as_ref()[..2])
        .join(&blob.as_ref()[2..]);
    let object = std::fs::read(&object_path).unwrap();
    std::fs::remove_file(&object_path).unwrap();
    let repo = open_repo(dir);
    let batch = evidence_at_head(&repo, vec![topic.clone()]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::IncompleteHistory)
    );
    assert!(!canary.exists());
    assert!(!dir.join(".git/FETCH_HEAD").exists());
    std::fs::write(&object_path, object).unwrap();
    let restored = evidence_at_head(&repo, vec![topic]);
    assert_eq!(
        restored.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
    assert_eq!(restored.graph_reads, 0);
    assert!(!canary.exists());
}

#[cfg(unix)]
#[test]
fn repository_replacement_cannot_reuse_cached_confirmation_at_the_same_path() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repository");
    std::fs::create_dir(&dir).unwrap();
    init_test_repo(&dir);
    write_file(&dir, "file", "base\n");
    commit_all(&dir, "base");
    let repo = open_repo(&dir);
    let token = CancellationToken::new();
    let request = RefComparisonRequest {
        target: repo.ref_comparison_target(Some("HEAD"), &token).unwrap(),
        subjects: vec![oid(&dir, "HEAD")],
        depth: RefComparisonDepth::Counts,
        budget: RefComparisonBudget::automatic(),
    };
    repo.analyze_ref_comparisons(&request, &token).unwrap();
    std::fs::rename(&dir, tmp.path().join("retired")).unwrap();
    std::fs::create_dir(&dir).unwrap();
    init_test_repo(&dir);
    write_file(&dir, "file", "replacement\n");
    commit_all(&dir, "replacement");
    let batch = repo.analyze_ref_comparisons(&request, &token).unwrap();
    assert_eq!(
        batch.results[0].counts,
        RefComparisonCounts::Unavailable(RefComparisonReason::IncompleteHistory)
    );
}

#[test]
fn a_deleted_file_replaced_by_a_target_directory_does_not_establish_content_evidence() {
    let _guard = COMPARISON_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "p", "base\n");
    let base = commit_all(dir, "base");
    std::fs::remove_file(dir.join("p")).unwrap();
    let topic = commit_all(dir, "delete p");
    git_success(dir, &["checkout", "--detach", base.as_ref()]);
    std::fs::remove_file(dir.join("p")).unwrap();
    std::fs::create_dir(dir.join("p")).unwrap();
    write_file(dir, "p/child", "target directory\n");
    commit_all(dir, "replace p by directory");
    let batch = evidence_at_head(&open_repo(dir), vec![topic]);
    assert_eq!(
        batch.results[0].integration,
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch)
    );
}
