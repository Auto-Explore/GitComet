use crate::conflict_session::ConflictSession;
use crate::domain::*;
use crate::error::{Error, ErrorKind};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandOutput {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl CommandOutput {
    pub fn empty_success(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
        }
    }

    pub fn combined(&self) -> String {
        let mut out = String::new();
        if !self.stdout.trim().is_empty() {
            out.push_str(self.stdout.trim_end());
            out.push('\n');
        }
        if !self.stderr.trim().is_empty() {
            out.push_str(self.stderr.trim_end());
            out.push('\n');
        }
        out.trim_end().to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictSide {
    Ours,
    Theirs,
}

/// Result of launching an external mergetool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergetoolResult {
    /// The tool command that was invoked.
    pub tool_name: String,
    /// Whether the tool reported success (exit code 0 or trust-exit-code semantics).
    pub success: bool,
    /// The merged file contents read back after the tool exited, if available.
    pub merged_contents: Option<Vec<u8>>,
    /// Combined stdout/stderr from the tool invocation for diagnostics.
    pub output: CommandOutput,
}

/// Try to decode optional bytes as UTF-8. Returns `None` if the bytes are
/// `None` or not valid UTF-8.
pub fn decode_utf8_optional(bytes: Option<&[u8]>) -> Option<String> {
    bytes.and_then(|b| std::str::from_utf8(b).ok().map(str::to_owned))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictTextValidation {
    pub has_conflict_markers: bool,
    pub marker_lines: usize,
}

/// Validate merged text before staging by scanning for unresolved
/// conflict marker lines.
pub fn validate_conflict_resolution_text(text: &str) -> ConflictTextValidation {
    let marker_lines = text
        .lines()
        .filter(|line| {
            line.starts_with("<<<<<<<")
                || line.starts_with(">>>>>>>")
                || line.starts_with("=======")
                || line.starts_with("|||||||")
        })
        .count();

    ConflictTextValidation {
        has_conflict_markers: marker_lines > 0,
        marker_lines,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictFileStages {
    pub path: PathBuf,
    pub base_bytes: Option<Arc<[u8]>>,
    pub ours_bytes: Option<Arc<[u8]>>,
    pub theirs_bytes: Option<Arc<[u8]>>,
    pub base: Option<Arc<str>>,
    pub ours: Option<Arc<str>>,
    pub theirs: Option<Arc<str>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteUrlKind {
    Fetch,
    Push,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlameLine {
    pub commit_id: Arc<str>,
    pub author: Arc<str>,
    pub author_time_unix: Option<i64>,
    pub summary: Arc<str>,
    pub line: String,
}

pub trait GitRepository: Send + Sync {
    fn spec(&self) -> &RepoSpec;

    fn log_head_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Result<LogPage>;
    fn log_all_branches_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        Err(Error::new(ErrorKind::Unsupported(
            "all-branches history is not implemented for this backend",
        )))
    }
    fn log_file_page(
        &self,
        _path: &Path,
        _limit: usize,
        _cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        Err(Error::new(ErrorKind::Unsupported(
            "file history is not implemented for this backend",
        )))
    }
    fn commit_details(&self, id: &CommitId) -> Result<CommitDetails>;
    fn reflog_head(&self, limit: usize) -> Result<Vec<ReflogEntry>>;
    fn current_branch(&self) -> Result<String>;
    fn list_branches(&self) -> Result<Vec<Branch>>;
    fn list_tags(&self) -> Result<Vec<Tag>> {
        Err(Error::new(ErrorKind::Unsupported(
            "tag listing is not implemented for this backend",
        )))
    }
    fn list_remote_tags(&self) -> Result<Vec<RemoteTag>> {
        Err(Error::new(ErrorKind::Unsupported(
            "remote tag listing is not implemented for this backend",
        )))
    }
    fn list_remotes(&self) -> Result<Vec<Remote>>;
    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>>;
    fn status(&self) -> Result<RepoStatus>;
    fn upstream_divergence(&self) -> Result<Option<UpstreamDivergence>> {
        Ok(None)
    }
    fn diff_unified(&self, target: &DiffTarget) -> Result<String>;
    /// Load and parse unified diff rows for the target.
    ///
    /// Default implementation goes through `diff_unified`; backends may
    /// override for streaming parsing to avoid large monolithic allocations.
    fn diff_parsed(&self, target: &DiffTarget) -> Result<Diff> {
        self.diff_unified(target)
            .map(|text| Diff::from_unified(target.clone(), &text))
    }
    fn diff_file_text(&self, _target: &DiffTarget) -> Result<Option<FileDiffText>> {
        Err(Error::new(ErrorKind::Unsupported(
            "file diff view is not implemented for this backend",
        )))
    }
    fn diff_file_image(&self, _target: &DiffTarget) -> Result<Option<FileDiffImage>> {
        Err(Error::new(ErrorKind::Unsupported(
            "image diff view is not implemented for this backend",
        )))
    }

    fn conflict_file_stages(&self, _path: &Path) -> Result<Option<ConflictFileStages>> {
        Err(Error::new(ErrorKind::Unsupported(
            "conflict stage reading is not implemented for this backend",
        )))
    }

    /// Build a backend-native conflict session for a conflicted path.
    ///
    /// Backends that support conflict stages and conflict-kind detection should
    /// return a populated session; unsupported backends return Unsupported.
    fn conflict_session(&self, _path: &Path) -> Result<Option<ConflictSession>> {
        Err(Error::new(ErrorKind::Unsupported(
            "conflict session loading is not implemented for this backend",
        )))
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()>;
    fn delete_branch(&self, name: &str) -> Result<()>;
    fn delete_branch_force(&self, _name: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "force branch deletion is not implemented for this backend",
        )))
    }
    fn checkout_branch(&self, name: &str) -> Result<()>;
    fn checkout_remote_branch(
        &self,
        _remote: &str,
        _branch: &str,
        _local_branch: &str,
    ) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "remote branch checkout is not implemented for this backend",
        )))
    }
    fn checkout_commit(&self, id: &CommitId) -> Result<()>;
    fn cherry_pick(&self, id: &CommitId) -> Result<()>;
    fn revert(&self, id: &CommitId) -> Result<()>;

    fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()>;
    fn stash_list(&self) -> Result<Vec<StashEntry>>;
    fn stash_apply(&self, index: usize) -> Result<()>;
    fn stash_drop(&self, index: usize) -> Result<()>;

    fn stage(&self, paths: &[&Path]) -> Result<()>;
    fn unstage(&self, paths: &[&Path]) -> Result<()>;
    fn commit(&self, message: &str) -> Result<()>;
    fn commit_amend(&self, _message: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "commit amend is not implemented for this backend",
        )))
    }

    fn rebase_with_output(&self, _onto: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git rebase is not implemented for this backend",
        )))
    }
    fn rebase_continue_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git rebase --continue is not implemented for this backend",
        )))
    }
    fn rebase_abort_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git rebase --abort is not implemented for this backend",
        )))
    }
    fn merge_abort_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git merge --abort is not implemented for this backend",
        )))
    }
    fn rebase_in_progress(&self) -> Result<bool> {
        Ok(false)
    }

    fn merge_commit_message(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn create_tag_with_output(&self, _name: &str, _target: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git tag creation is not implemented for this backend",
        )))
    }
    fn delete_tag_with_output(&self, _name: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git tag deletion is not implemented for this backend",
        )))
    }
    fn prune_merged_branches_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "pruning merged branches is not implemented for this backend",
        )))
    }
    fn prune_local_tags_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "pruning local tags is not implemented for this backend",
        )))
    }
    fn push_tag_with_output(&self, _remote: &str, _name: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "pushing tags is not implemented for this backend",
        )))
    }
    fn delete_remote_tag_with_output(&self, _remote: &str, _name: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "remote tag deletion is not implemented for this backend",
        )))
    }

    fn add_remote_with_output(&self, _name: &str, _url: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git remote add is not implemented for this backend",
        )))
    }
    fn remove_remote_with_output(&self, _name: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git remote remove is not implemented for this backend",
        )))
    }
    fn set_remote_url_with_output(
        &self,
        _name: &str,
        _url: &str,
        _kind: RemoteUrlKind,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git remote set-url is not implemented for this backend",
        )))
    }

    fn fetch_all(&self) -> Result<()>;
    fn pull(&self, mode: PullMode) -> Result<()>;
    fn push(&self) -> Result<()>;
    fn push_force(&self) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "force push is not implemented for this backend",
        )))
    }
    fn push_set_upstream(&self, _remote: &str, _branch: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "pushing with --set-upstream is not implemented for this backend",
        )))
    }

    fn fetch_all_with_output(&self) -> Result<CommandOutput> {
        self.fetch_all()?;
        Ok(CommandOutput::empty_success("git fetch --all"))
    }

    fn fetch_all_with_output_prune(&self, prune: bool) -> Result<CommandOutput> {
        let _ = prune;
        self.fetch_all_with_output()
    }

    fn pull_with_output(&self, mode: PullMode) -> Result<CommandOutput> {
        self.pull(mode)?;
        Ok(CommandOutput::empty_success("git pull"))
    }

    fn push_with_output(&self) -> Result<CommandOutput> {
        self.push()?;
        Ok(CommandOutput::empty_success("git push"))
    }

    fn push_force_with_output(&self) -> Result<CommandOutput> {
        self.push_force()?;
        Ok(CommandOutput::empty_success("git push --force-with-lease"))
    }

    fn push_set_upstream_with_output(&self, remote: &str, branch: &str) -> Result<CommandOutput> {
        self.push_set_upstream(remote, branch)?;
        Ok(CommandOutput::empty_success(format!(
            "git push --set-upstream {remote} HEAD:refs/heads/{branch}"
        )))
    }

    fn delete_remote_branch_with_output(
        &self,
        _remote: &str,
        _branch: &str,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "remote branch deletion is not implemented for this backend",
        )))
    }

    fn commit_amend_with_output(&self, message: &str) -> Result<CommandOutput> {
        self.commit_amend(message)?;
        Ok(CommandOutput::empty_success("git commit --amend"))
    }

    fn pull_branch_with_output(&self, _remote: &str, _branch: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "pulling a specific remote branch is not implemented for this backend",
        )))
    }

    fn merge_ref_with_output(&self, _reference: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "merging a specific ref is not implemented for this backend",
        )))
    }

    fn squash_ref_with_output(&self, _reference: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "squashing a specific ref is not implemented for this backend",
        )))
    }

    fn reset_with_output(&self, _target: &str, _mode: ResetMode) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git reset is not implemented for this backend",
        )))
    }

    fn blame_file(&self, _path: &Path, _rev: Option<&str>) -> Result<Vec<BlameLine>> {
        Err(Error::new(ErrorKind::Unsupported(
            "git blame is not implemented for this backend",
        )))
    }

    fn checkout_conflict_side(&self, _path: &Path, _side: ConflictSide) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "conflict resolution is not implemented for this backend",
        )))
    }

    /// Accept a conflict by explicitly deleting the path and staging removal.
    ///
    /// Used by decision/keep-delete resolvers when the chosen outcome is
    /// "accept deletion" rather than selecting a side's content.
    fn accept_conflict_deletion(&self, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "conflict deletion is not implemented for this backend",
        )))
    }

    /// Restore a conflicted file from stage-1 (base) contents and stage it.
    ///
    /// Useful for decision-style conflicts where users want to explicitly
    /// recover the base version as the resolution result.
    fn checkout_conflict_base(&self, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "base conflict checkout is not implemented for this backend",
        )))
    }

    /// Launch an external mergetool for a conflicted file.
    ///
    /// Materializes BASE, LOCAL, REMOTE temp files from the conflict stages,
    /// invokes the configured (or specified) mergetool, reads back the merged
    /// output, writes it to the worktree, and stages the result.
    fn launch_mergetool(&self, _path: &Path) -> Result<MergetoolResult> {
        Err(Error::new(ErrorKind::Unsupported(
            "external mergetool is not implemented for this backend",
        )))
    }

    fn export_patch_with_output(
        &self,
        _commit_id: &CommitId,
        _dest: &Path,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "patch export is not implemented for this backend",
        )))
    }

    fn apply_patch_with_output(&self, _patch: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "patch apply is not implemented for this backend",
        )))
    }

    fn apply_unified_patch_to_index_with_output(
        &self,
        _patch: &str,
        _reverse: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "index patch apply is not implemented for this backend",
        )))
    }

    fn apply_unified_patch_to_worktree_with_output(
        &self,
        _patch: &str,
        _reverse: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree patch apply is not implemented for this backend",
        )))
    }

    fn list_worktrees(&self) -> Result<Vec<Worktree>> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree listing is not implemented for this backend",
        )))
    }

    fn add_worktree_with_output(
        &self,
        _path: &Path,
        _reference: Option<&str>,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree add is not implemented for this backend",
        )))
    }

    fn remove_worktree_with_output(&self, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree remove is not implemented for this backend",
        )))
    }

    fn force_remove_worktree_with_output(&self, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree force remove is not implemented for this backend",
        )))
    }

    fn list_submodules(&self) -> Result<Vec<Submodule>> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule listing is not implemented for this backend",
        )))
    }

    fn add_submodule_with_output(&self, _url: &str, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule add is not implemented for this backend",
        )))
    }

    fn update_submodules_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule update is not implemented for this backend",
        )))
    }

    fn remove_submodule_with_output(&self, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule remove is not implemented for this backend",
        )))
    }

    fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PullMode {
    Default,
    Merge,
    FastForwardIfPossible,
    FastForwardOnly,
    Rebase,
}

pub trait GitBackend: Send + Sync {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>>;
}

#[cfg(test)]
mod tests {
    use super::{
        BlameLine, CommandOutput, decode_utf8_optional, validate_conflict_resolution_text,
    };
    use std::sync::Arc;

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
            line: "hello".to_string(),
        };

        let cloned = line.clone();
        assert!(Arc::ptr_eq(&line.commit_id, &cloned.commit_id));
        assert!(Arc::ptr_eq(&line.author, &cloned.author));
        assert!(Arc::ptr_eq(&line.summary, &cloned.summary));
        assert_eq!(line.line, cloned.line);
    }
}
