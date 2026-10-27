use super::CancellationToken;
use super::{
    BlameLine, CheckoutRemoteBranchMode, CommandOutput, ConflictSide, GitBackend, GitRepository,
    PullMode, RemoteUrlKind, ResetMode, SafePushAfterCommitTarget, UnavailableGitBackend,
    decode_utf8_optional, validate_conflict_resolution_text,
};
use crate::domain::{
    Branch, CommitDetails, CommitId, DiffArea, DiffTarget, HistoryMode, LogCursor, LogPage,
    ReflogEntry, Remote, RemoteBranch, RepoSpec, RepoStatus, SignatureFormats, StashEntry,
    Upstream,
};
use crate::error::{Error, ErrorKind};
use crate::test_support::UnconfiguredRepository;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[test]
fn combined_steps_join_non_empty_streams_in_order() {
    let step = |stdout: &str, stderr: &str| CommandOutput {
        command: "step".into(),
        stdout: stdout.into(),
        stderr: stderr.into(),
        exit_code: Some(0),
    };
    let combined = CommandOutput::combine(
        "both",
        &[step("one\n", ""), step("", "warn\n"), step("two", "  \n")],
    );
    assert_eq!(combined.command, "both");
    assert_eq!(combined.stdout, "one\ntwo");
    assert_eq!(combined.stderr, "warn");
    assert_eq!(combined.exit_code, Some(0));
}

#[test]
fn history_ref_filters_match_names_prefixes_and_globs() {
    use super::HistoryRefFilter;
    let filter =
        HistoryRefFilter::excluding(["refs/pull", "refs/remotes/*/pr/**", "refs/heads/wip-*", ""]);
    assert_eq!(filter.patterns().len(), 3, "empty patterns are dropped");
    for excluded in [
        "refs/pull",
        "refs/pull/7/head",
        "refs/remotes/origin/pr/7",
        "refs/remotes/origin/pr/7/merge",
        "refs/heads/wip-parser",
    ] {
        assert!(filter.excludes(excluded), "{excluded}");
    }
    for kept in [
        "refs/pulled/7",
        "refs/heads/main",
        "refs/remotes/origin/main",
        "refs/remotes/origin/nested/pr/7",
        "refs/heads/wip",
        "refs/heads/wip-a/b",
    ] {
        assert!(!filter.excludes(kept), "{kept}");
    }
    assert!(HistoryRefFilter::default().is_empty());
    assert!(!HistoryRefFilter::default().excludes("refs/heads/main"));
    // Order and duplicates do not make two filters different.
    assert_eq!(
        HistoryRefFilter::excluding(["b", "a", "a"]),
        HistoryRefFilter::excluding(["a", "b"])
    );
}

#[test]
fn repository_options_are_refused_by_backends_that_cannot_honor_them() {
    use super::{ConfiguredBackend, HistoryRefFilter, RepositoryOptions};
    let options = RepositoryOptions::default()
        .with_history_ref_filter(HistoryRefFilter::excluding(["refs/pull"]));
    struct PlainBackend;
    impl GitBackend for PlainBackend {
        fn open(&self, workdir: &Path) -> super::Result<Arc<dyn GitRepository>> {
            Ok(Arc::new(UnconfiguredRepository::new(workdir)))
        }
    }
    let error = PlainBackend
        .open_with_options(Path::new("/tmp/repo"), &options)
        .err()
        .expect("a non-default option is refused");
    assert!(matches!(error.kind(), ErrorKind::Unsupported(_)));
    assert!(
        PlainBackend
            .open_with_options(Path::new("/tmp/repo"), &RepositoryOptions::default())
            .is_ok()
    );
    // Wrapped, every open carries the options and so is refused too.
    let configured = ConfiguredBackend::new(Arc::new(PlainBackend), options);
    assert!(configured.open(Path::new("/tmp/repo")).is_err());
    let cancel = super::CancellationToken::new();
    assert!(
        configured
            .open_cancellable(Path::new("/tmp/repo"), &cancel)
            .is_err()
    );
}

/// Pins the fallback behavior of every provided [`GitRepository`] method.
///
/// These defaults are what a backend inherits when it does not implement an
/// operation, so a default that silently changes from "unsupported" to a
/// plausible-looking success value (`Ok(vec![])`, `Ok(false)`) would make
/// every partial backend quietly report the wrong thing instead of failing
/// loudly. Nothing else in the workspace exercises them.
#[test]
fn provided_trait_methods_use_unsupported_fallback_behavior() {
    fn assert_unsupported<T: std::fmt::Debug>(result: super::Result<T>) {
        let error = result.expect_err("provided default must be unsupported");
        assert!(
            matches!(error.kind(), ErrorKind::Unsupported(_)),
            "expected Unsupported, got {error:?}"
        );
    }

    let repo = UnconfiguredRepository::new("/tmp/provided-defaults");
    let commit = CommitId("abc123".into());
    let safe_push_target = SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        local_branch: "main".to_string(),
        local_head: commit.clone(),
    };
    let cursor = LogCursor {
        last_seen: CommitId("deadbeef".into()),
        resume_from: None,
        resume_token: None,
    };
    let diff_target = DiffTarget::working_tree(PathBuf::from("file.txt"), DiffArea::Staged);
    let path = Path::new("file.txt");

    // The three defaults that deliberately succeed: absence of an upstream,
    // of a rebase, and of a merge message are all legitimate answers a
    // backend can give without implementing anything.
    assert_eq!(
        repo.upstream_divergence(&CancellationToken::new()).unwrap(),
        None
    );
    assert!(!repo.rebase_in_progress(&CancellationToken::new()).unwrap());
    assert_eq!(
        repo.merge_commit_message(&CancellationToken::new())
            .unwrap(),
        None
    );
    // Likewise: "not a gitlink" is the safe answer for a backend that
    // cannot read HEAD trees.
    assert!(!repo.head_path_is_gitlink(path).unwrap());
    // A backend without a long-lived store has nothing to release.
    repo.release_object_store();

    assert_unsupported(repo.log_all_branches_page(25, Some(&cursor), &CancellationToken::new()));
    assert_unsupported(repo.log_file_page(path, 25, None));
    assert_unsupported(repo.list_tags(&CancellationToken::new()));
    assert_unsupported(repo.list_remote_tags(&CancellationToken::new()));
    assert_unsupported(repo.diff_file_text(&diff_target, None, &CancellationToken::new()));
    assert_unsupported(repo.diff_file_image(&diff_target, &CancellationToken::new()));
    assert_unsupported(repo.conflict_file_stages(path));
    assert_unsupported(repo.conflict_session(path, None));
    assert_unsupported(repo.delete_branch_force("feature"));
    assert_unsupported(repo.rename_branch_force("feature", "main"));
    assert_unsupported(repo.checkout_remote_branch(
        "origin",
        "main",
        "feature",
        CheckoutRemoteBranchMode::Create,
    ));
    assert_unsupported(repo.commit_amend("message"));
    assert_unsupported(repo.topologically_order_commits(std::slice::from_ref(&commit)));
    assert_unsupported(repo.compare_files(
        &commit,
        Some(&commit),
        &super::ComparisonOptions::direct(),
        &super::CancellationToken::new(),
    ));
    assert_unsupported(repo.cherry_pick_with_output(&commit, true, None));
    assert_unsupported(repo.revert_with_output(&commit, true, None));
    assert_unsupported(repo.apply_file_change_with_output(
        &crate::domain::ApplyChangeTarget::commit(commit.clone(), path.to_path_buf()),
        false,
    ));
    assert_unsupported(repo.rebase_with_output("main"));
    assert_unsupported(repo.rebase_continue_with_output());
    assert_unsupported(repo.rebase_abort_with_output());
    assert_unsupported(repo.merge_abort_with_output());
    assert_unsupported(repo.create_tag_with_output("v1.0.0", "HEAD", None, false));
    assert_unsupported(repo.delete_tag_with_output("v1.0.0"));
    assert_unsupported(repo.prune_merged_branches_with_output());
    assert_unsupported(repo.maintenance_needed());
    assert_unsupported(repo.run_maintenance_with_output());
    assert_eq!(repo.common_dir(), None);
    assert_unsupported(repo.prune_local_tags_with_output());
    assert_unsupported(repo.push_tag_with_output("origin", "v1.0.0"));
    assert_unsupported(repo.delete_remote_tag_with_output("origin", "v1.0.0"));
    assert_unsupported(repo.add_remote_with_output(
        "origin",
        "https://example.com/repo.git",
        crate::remote_url::RemoteUrlPolicy::default(),
    ));
    assert_unsupported(repo.remove_remote_with_output("origin"));
    assert_unsupported(repo.set_remote_url_with_output(
        "origin",
        "https://example.com/repo.git",
        RemoteUrlKind::Fetch,
        crate::remote_url::RemoteUrlPolicy::default(),
    ));
    assert_unsupported(repo.fetch_all(true));
    assert_unsupported(repo.pull(PullMode::FastForwardOnly));
    assert_unsupported(repo.push_with_output());
    assert_unsupported(repo.push_after_commit_with_output(&safe_push_target));
    assert_unsupported(repo.push_after_commit_set_upstream_with_output(&safe_push_target));
    assert_unsupported(repo.push_force());
    assert_unsupported(repo.push_force_with_output());
    assert_unsupported(repo.push_set_upstream("origin", "main"));
    assert_unsupported(repo.push_set_upstream_with_output("origin", "main"));
    assert_unsupported(repo.set_upstream_branch_with_output(
        "main",
        &Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        },
    ));
    assert_unsupported(repo.unset_upstream_branch_with_output("main"));
    assert_unsupported(repo.delete_remote_branch_with_output("origin", "main"));
    assert_unsupported(repo.commit_amend_with_output("message"));
    assert_unsupported(repo.pull_branch_with_output("origin", "main"));
    assert_unsupported(repo.merge_ref_with_output("origin/main"));
    assert_unsupported(repo.squash_ref_with_output("origin/main"));
    assert_unsupported(repo.reset_with_output("HEAD~1", ResetMode::Mixed));
    assert_unsupported(repo.blame_file(path, None));
    assert_unsupported(repo.checkout_conflict_side(path, ConflictSide::Ours));
    assert_unsupported(repo.accept_conflict_deletion(path));
    assert_unsupported(repo.checkout_conflict_base(path));
    assert_unsupported(repo.launch_mergetool(path));
    assert_unsupported(repo.export_patch_with_output(&commit, path));
    assert_unsupported(repo.apply_patch_with_output(path));
    assert_unsupported(repo.apply_unified_patch_to_index_with_output(b"@@ -1 +1 @@", false));
    assert_unsupported(repo.apply_unified_patch_to_worktree_with_output(b"@@ -1 +1 @@", true));
    assert_unsupported(repo.list_worktrees(&CancellationToken::new()));
    assert_unsupported(repo.add_worktree_with_output(
        path,
        &super::WorktreeCheckout::Existing {
            reference: "main".to_string(),
        },
    ));
    assert_unsupported(repo.remove_worktree_with_output(path));
    assert_unsupported(repo.force_remove_worktree_with_output(path));
    assert_unsupported(repo.list_submodules(&CancellationToken::new()));
    assert_unsupported(repo.check_submodule_add_trust(
        "https://example.com/repo.git",
        path,
        crate::remote_url::RemoteUrlPolicy::default(),
    ));
    assert_unsupported(
        repo.check_submodule_update_trust(crate::remote_url::RemoteUrlPolicy::default()),
    );
    assert_unsupported(repo.add_submodule_with_output(
        "https://example.com/repo.git",
        path,
        None,
        None,
        false,
        &[],
        crate::remote_url::RemoteUrlPolicy::default(),
    ));
    assert_unsupported(
        repo.update_submodules_with_output(&[], crate::remote_url::RemoteUrlPolicy::default()),
    );
    assert_unsupported(repo.remove_submodule_with_output(path));
}

#[test]
fn unavailable_backend_reports_unsupported() {
    let error = match UnavailableGitBackend.open(Path::new(".")) {
        Ok(_) => panic!("unavailable backend must reject repository opens"),
        Err(error) => error,
    };
    assert!(matches!(error.kind(), ErrorKind::Unsupported(_)));
}

fn unsupported<T>() -> super::Result<T> {
    Err(Error::new(ErrorKind::Unsupported(
        "unused in services history-mode delegation test",
    )))
}

struct RecordingHistoryModeRepo {
    spec: RepoSpec,
    calls: Mutex<Vec<(&'static str, usize, Option<String>)>>,
    cancel_after_read: Option<CancellationToken>,
}

impl RecordingHistoryModeRepo {
    fn new() -> Self {
        Self {
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/recording-history-mode-repo"),
            },
            calls: Mutex::new(Vec::new()),
            cancel_after_read: None,
        }
    }

    fn record(&self, method: &'static str, limit: usize, cursor: Option<&LogCursor>) {
        self.calls.lock().expect("recording mutex").push((
            method,
            limit,
            cursor.map(|cursor| cursor.last_seen.as_ref().to_string()),
        ));
        if let Some(cancellation) = &self.cancel_after_read {
            cancellation.cancel();
        }
    }

    fn calls(&self) -> Vec<(&'static str, usize, Option<String>)> {
        self.calls.lock().expect("recording mutex").clone()
    }
}

impl GitRepository for RecordingHistoryModeRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> super::Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: super::Result<std::sync::Arc<LogPage>> = {
            self.record("head", limit, cursor);
            Ok(std::sync::Arc::new(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn log_all_branches_page(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> super::Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: super::Result<std::sync::Arc<LogPage>> = {
            self.record("all", limit, cursor);
            Ok(std::sync::Arc::new(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn commit_details(&self, _id: &CommitId) -> super::Result<CommitDetails> {
        unsupported()
    }

    fn reflog_head(&self, _limit: usize) -> super::Result<Vec<ReflogEntry>> {
        unsupported()
    }

    fn current_branch(&self, cancellation: &CancellationToken) -> super::Result<String> {
        cancellation.check_cancelled()?;
        let result: super::Result<String> = { unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_branches(&self, cancellation: &CancellationToken) -> super::Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: super::Result<Vec<Branch>> = { unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remotes(&self, cancellation: &CancellationToken) -> super::Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: super::Result<Vec<Remote>> = { unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remote_branches(
        &self,
        cancellation: &CancellationToken,
    ) -> super::Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: super::Result<Vec<RemoteBranch>> = { unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn status(&self, cancellation: &CancellationToken) -> super::Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: super::Result<RepoStatus> = { unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn diff_unified(&self, _target: &DiffTarget) -> super::Result<String> {
        unsupported()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> super::Result<()> {
        unsupported()
    }

    fn delete_branch(&self, _name: &str) -> super::Result<()> {
        unsupported()
    }

    fn checkout_branch(&self, _name: &str) -> super::Result<()> {
        unsupported()
    }

    fn checkout_commit(&self, _id: &CommitId) -> super::Result<()> {
        unsupported()
    }

    fn cherry_pick(&self, _id: &CommitId) -> super::Result<()> {
        unsupported()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> super::Result<()> {
        unsupported()
    }

    fn stash_list(&self, cancellation: &CancellationToken) -> super::Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: super::Result<Vec<StashEntry>> = { unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn stash_apply(&self, _index: usize) -> super::Result<()> {
        unsupported()
    }

    fn stash_drop(&self, _index: usize) -> super::Result<()> {
        unsupported()
    }

    fn stage(&self, _paths: &[&Path]) -> super::Result<()> {
        unsupported()
    }

    fn unstage(&self, _paths: &[&Path]) -> super::Result<()> {
        unsupported()
    }

    fn commit(&self, _message: &str) -> super::Result<()> {
        unsupported()
    }

    fn fetch_all(&self, _prune: bool) -> super::Result<CommandOutput> {
        let result: super::Result<()> = { unsupported() };
        result.map(|()| CommandOutput::empty_success("git fetch --all"))
    }

    fn pull(&self, _mode: super::PullMode) -> super::Result<CommandOutput> {
        let result: super::Result<()> = { unsupported() };
        result.map(|()| CommandOutput::empty_success("git pull"))
    }

    fn push(&self) -> super::Result<()> {
        unsupported()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> super::Result<()> {
        unsupported()
    }
}

// ── validate_conflict_resolution_text ────────────────────────────

#[test]
fn validate_conflict_resolution_text_reports_no_markers() {
    let validation = validate_conflict_resolution_text("line 1\nline 2\n");
    assert!(!validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 0);
}

#[test]
fn validate_conflict_resolution_text_counts_marker_lines() {
    let text = "<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n";
    let validation = validate_conflict_resolution_text(text);
    assert!(validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 3);
}

#[test]
fn validate_empty_text_reports_no_markers() {
    let validation = validate_conflict_resolution_text("");
    assert!(!validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 0);
}

#[test]
fn validate_diff3_markers_detected() {
    let text = "<<<<<<< ours\na\n||||||| base\nb\n=======\nc\n>>>>>>> theirs\n";
    let validation = validate_conflict_resolution_text(text);
    assert!(validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 4);
}

#[test]
fn validate_markers_with_branch_annotations_detected() {
    let text = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature/my-branch\n";
    let validation = validate_conflict_resolution_text(text);
    assert!(validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 3);
}

#[test]
fn validate_partial_marker_set_detected() {
    // Only start marker — still detects it
    let text = "some code\n<<<<<<< HEAD\nmore code\n";
    let validation = validate_conflict_resolution_text(text);
    assert!(validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 1);
}

#[test]
fn validate_markers_not_at_start_of_line_ignored() {
    // Markers must be at line start to count
    let text = "  <<<<<<< not a marker\n  ======= not a marker\n";
    let validation = validate_conflict_resolution_text(text);
    assert!(!validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 0);
}

#[test]
fn validate_multiple_conflicts_counts_all_markers() {
    let text = "\
<<<<<<< HEAD\na\n=======\nb\n>>>>>>> branch1\n\
<<<<<<< HEAD\nc\n=======\nd\n>>>>>>> branch2\n";
    let validation = validate_conflict_resolution_text(text);
    assert!(validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 6);
}

// ── decode_utf8_optional ─────────────────────────────────────────

#[test]
fn decode_utf8_none_returns_none() {
    assert_eq!(decode_utf8_optional(None), None);
}

#[test]
fn decode_utf8_valid_returns_string() {
    let bytes = b"hello world";
    assert_eq!(
        decode_utf8_optional(Some(bytes.as_slice())),
        Some("hello world".to_string())
    );
}

#[test]
fn decode_utf8_invalid_returns_none() {
    let bytes = &[0xff, 0xfe, 0x00, 0x01];
    assert_eq!(decode_utf8_optional(Some(bytes.as_slice())), None);
}

#[test]
fn decode_utf8_empty_bytes_returns_empty_string() {
    let bytes: &[u8] = b"";
    assert_eq!(decode_utf8_optional(Some(bytes)), Some(String::new()));
}

#[test]
fn decode_utf8_multibyte_chars_preserved() {
    let text = "héllo wörld 日本語";
    assert_eq!(
        decode_utf8_optional(Some(text.as_bytes())),
        Some(text.to_string())
    );
}

// ── CommandOutput ────────────────────────────────────────────────

#[test]
fn command_output_empty_success_has_zero_exit_code() {
    let out = CommandOutput::empty_success("git status");
    assert_eq!(out.command, "git status");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "");
    assert_eq!(out.exit_code, Some(0));
}

#[test]
fn command_output_combined_stdout_only() {
    let out = CommandOutput {
        command: "test".into(),
        stdout: "output line\n".into(),
        stderr: String::new(),
        exit_code: Some(0),
    };
    assert_eq!(out.combined(), "output line");
}

#[test]
fn command_output_combined_stderr_only() {
    let out = CommandOutput {
        command: "test".into(),
        stdout: String::new(),
        stderr: "error message\n".into(),
        exit_code: Some(1),
    };
    assert_eq!(out.combined(), "error message");
}

#[test]
fn command_output_combined_both_streams() {
    let out = CommandOutput {
        command: "test".into(),
        stdout: "output\n".into(),
        stderr: "warning\n".into(),
        exit_code: Some(0),
    };
    assert_eq!(out.combined(), "output\nwarning");
}

#[test]
fn command_output_combined_empty_when_both_blank() {
    let out = CommandOutput {
        command: "test".into(),
        stdout: "   \n".into(),
        stderr: "  \n".into(),
        exit_code: Some(0),
    };
    assert_eq!(out.combined(), "");
}

#[test]
fn command_output_combined_trims_trailing_whitespace() {
    let out = CommandOutput {
        command: "test".into(),
        stdout: "line1\nline2\n\n".into(),
        stderr: "err\n\n".into(),
        exit_code: Some(0),
    };
    assert_eq!(out.combined(), "line1\nline2\nerr");
}

#[test]
fn command_output_default_has_no_exit_code() {
    let out = CommandOutput::default();
    assert_eq!(out.command, "");
    assert_eq!(out.exit_code, None);
}

#[test]
fn blame_line_clone_shares_arc_metadata() {
    let line = BlameLine {
        commit_id: "deadbeef".into(),
        author: "Alice".into(),
        author_time_unix: Some(1_700_000_000),
        summary: "Initial import".into(),
        body: Some("detailed body".into()),
        line: "hello".to_string(),
        prior_exists: true,
        source_path: None,
        prior_commit: None,
    };

    let cloned = line.clone();
    assert!(Arc::ptr_eq(&line.commit_id, &cloned.commit_id));
    assert!(Arc::ptr_eq(&line.author, &cloned.author));
    assert!(Arc::ptr_eq(&line.summary, &cloned.summary));
    assert_eq!(line.line, cloned.line);
}

#[test]
fn signature_default_preserves_empty_formats_and_cancellation() {
    let repo = UnconfiguredRepository::new("/tmp/signature-defaults");
    let commits = [CommitId("abc123".into())];
    let cancellation = CancellationToken::new();

    assert!(
        repo.verify_commit_signatures(&commits, SignatureFormats::NONE, &cancellation)
            .unwrap()
            .is_empty()
    );
    let error = repo
        .verify_commit_signatures(&commits, SignatureFormats::ALL, &cancellation)
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Unsupported(_)));

    cancellation.cancel();
    let error = repo
        .verify_commit_signatures(&commits, SignatureFormats::NONE, &cancellation)
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
}

#[test]
fn canonical_read_cancelled_before_call_does_no_backend_work() {
    let repo = RecordingHistoryModeRepo::new();
    let backend: &dyn GitRepository = &repo;
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = backend.log_head_page(7, None, &cancellation).unwrap_err();

    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert!(repo.calls().is_empty());
    backend
        .log_head_page(7, None, &CancellationToken::new())
        .expect("a live request reaches the backend");
    assert_eq!(repo.calls(), vec![("head", 7, None)]);
}

#[test]
fn canonical_read_cancelled_during_call_suppresses_success() {
    let cancellation = CancellationToken::new();
    let mut repo = RecordingHistoryModeRepo::new();
    repo.cancel_after_read = Some(cancellation.clone());
    let backend: &dyn GitRepository = &repo;

    let error = backend.log_head_page(7, None, &cancellation).unwrap_err();

    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert_eq!(repo.calls(), vec![("head", 7, None)]);
}

#[test]
fn log_history_mode_page_delegates_current_branch_modes_to_head_log() {
    let repo = RecordingHistoryModeRepo::new();
    let cursor = LogCursor {
        last_seen: CommitId("cursor".into()),
        resume_from: Some(CommitId("resume".into())),
        resume_token: Some(Arc::from("token")),
    };

    for mode in [
        HistoryMode::FullReachable,
        HistoryMode::FirstParent,
        HistoryMode::NoMerges,
        HistoryMode::MergesOnly,
        HistoryMode::AllBranches,
    ] {
        repo.log_history_mode_page(mode, 7, Some(&cursor), &CancellationToken::new())
            .expect("history mode delegation should succeed");
    }

    assert_eq!(
        repo.calls(),
        vec![
            ("head", 7, Some("cursor".to_string())),
            ("head", 7, Some("cursor".to_string())),
            ("head", 7, Some("cursor".to_string())),
            ("head", 7, Some("cursor".to_string())),
            ("all", 7, Some("cursor".to_string())),
        ]
    );
}

#[test]
fn provided_history_readers_refuse_hidden_remotes_they_cannot_apply() {
    use super::{HistoryReadRequest, HistoryWalk};
    let repo = RecordingHistoryModeRepo::new();
    let hidden = HistoryWalk::new(HistoryMode::AllBranches, ["origin"]);
    let first_page = HistoryReadRequest::Page {
        limit: 7,
        cursor: None,
        snapshot: None,
    };
    let cancel = CancellationToken::new();

    let error = repo
        .read_history(
            &hidden,
            None,
            &Default::default(),
            &first_page,
            &cancel,
            &mut |_| {},
        )
        .unwrap_err();
    assert!(
        matches!(error.kind(), ErrorKind::Unsupported(_)),
        "{error:?}"
    );
    let error = repo
        .history_authors(&hidden, &Default::default(), &cancel)
        .unwrap_err();
    assert!(
        matches!(error.kind(), ErrorKind::Unsupported(_)),
        "{error:?}"
    );
    assert!(repo.calls().is_empty(), "{:?}", repo.calls());
    assert!(
        repo.build_history_index(&hidden, None, &Default::default(), &cancel, &mut |_| {})
            .unwrap()
            .is_none(),
        "no index still means the paged reader"
    );

    repo.read_history(
        &HistoryMode::AllBranches.into(),
        None,
        &Default::default(),
        &first_page,
        &cancel,
        &mut |_| {},
    )
    .expect("nothing to hide");
    assert_eq!(repo.calls(), vec![("all", 7, None)]);
}

#[test]
fn only_an_all_branches_walk_hides_remotes() {
    use super::HistoryWalk;
    let walk = HistoryWalk::new(HistoryMode::AllBranches, ["upstream", "origin", "upstream"]);
    assert_eq!(
        walk.hidden_remotes()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["origin", "upstream"]
    );
    for mode in [
        HistoryMode::FullReachable,
        HistoryMode::FirstParent,
        HistoryMode::NoMerges,
        HistoryMode::MergesOnly,
    ] {
        let walk = HistoryWalk::new(mode, ["origin"]);
        assert!(walk.hidden_remotes().is_empty(), "{mode:?}");
        assert_eq!(walk, HistoryWalk::from(mode), "{mode:?}");
    }
}
