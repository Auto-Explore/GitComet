use crate::conflict_session::ConflictSession;
use crate::domain::*;
use crate::error::{Error, ErrorKind};
use crate::text_format::TextAttributes;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub type Result<T> = std::result::Result<T, Error>;

pub struct RepositoryOperationLease {
    _guard: Box<dyn Send>,
}

impl RepositoryOperationLease {
    pub fn new(guard: impl Send + 'static) -> Self {
        Self {
            _guard: Box::new(guard),
        }
    }
}

impl Default for RepositoryOperationLease {
    fn default() -> Self {
        Self::new(())
    }
}

/// Which commit a comparison measures its newer side against.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum ComparisonBase {
    /// The older endpoint itself (`git diff from to`).
    #[default]
    Direct,
    /// The endpoints' merge base (`git diff from...to`): what the newer side
    /// adds since it diverged.
    MergeBase,
}

/// How [`GitRepository::compare_files`] compares. The default measures from
/// `from` itself and lists tracked files only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub struct ComparisonOptions {
    pub base: ComparisonBase,
    /// With a working-tree tip, also list untracked files (as added). Off by
    /// default; it has no effect when both sides are commits.
    pub include_untracked: bool,
}

impl ComparisonOptions {
    pub fn direct() -> Self {
        Self::default()
    }

    pub fn merge_base() -> Self {
        Self {
            base: ComparisonBase::MergeBase,
            ..Self::default()
        }
    }

    pub fn with_untracked(mut self, include_untracked: bool) -> Self {
        self.include_untracked = include_untracked;
        self
    }
}

/// The result of [`GitRepository::compare_files`]: the files, and the commit
/// the comparison actually measured from (the merge base, when asked for).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Comparison {
    pub base: CommitId,
    pub files: Vec<CommitFileChange>,
}

impl Comparison {
    pub fn new(base: CommitId, files: Vec<CommitFileChange>) -> Self {
        Self { base, files }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    parent: Option<Arc<CancellationToken>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self
                .parent
                .as_ref()
                .is_some_and(|parent| parent.is_cancelled())
    }

    /// Also stop when the repository closes without cancelling other requests
    /// when this request is superseded.
    pub fn with_parent(mut self, parent: CancellationToken) -> Self {
        self.parent = Some(Arc::new(parent));
        self
    }

    pub fn check_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::new(ErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }
}

/// A partially built log page, reported while a walk is still running.
///
/// `commits` is the page so far — every chunk is a prefix of the next one and
/// of the final page — and `scanned` counts the commits the walk has visited,
/// matching or not, so a filter that is finding nothing still shows progress.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LogChunk {
    pub commits: Vec<crate::domain::Commit>,
    pub scanned: u64,
}

/// Opaque identity of the exact inputs used to traverse history. It is scoped to
/// a repository and query, shared by all its pages, and never persisted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistorySnapshot(pub Arc<str>);

#[derive(Clone, Debug)]
pub enum HistoryReadRequest {
    Page {
        limit: usize,
        cursor: Option<LogCursor>,
        snapshot: Option<HistorySnapshot>,
    },
    Refresh {
        previous: Arc<LogPage>,
        snapshot: Option<HistorySnapshot>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryReadResult {
    Page {
        page: Arc<LogPage>,
        snapshot: Option<HistorySnapshot>,
    },
    Unchanged,
    /// The continuation belongs to a different history. Refresh the retained
    /// page before allowing another append.
    Invalidated,
}

impl From<LogPage> for HistoryReadResult {
    fn from(page: LogPage) -> Self {
        Arc::new(page).into()
    }
}

impl From<Arc<LogPage>> for HistoryReadResult {
    fn from(page: Arc<LogPage>) -> Self {
        Self::Page {
            page,
            snapshot: None,
        }
    }
}

/// Rebuild a loaded extent without losing commits merely because new commits
/// pushed them beyond a numeric page limit. A removed commit requires walking
/// to EOF to establish its absence. Partial results stay private to the caller.
pub fn refresh_history_page(
    previous: &LogPage,
    cancellation: &CancellationToken,
    mut read: impl FnMut(usize, Option<&LogCursor>) -> Result<Arc<LogPage>>,
) -> Result<Arc<LogPage>> {
    let complete = previous.next_cursor.is_none();
    let mut remaining: rustc_hash::FxHashSet<_> =
        previous.commits.iter().map(|commit| &commit.id).collect();
    cancellation.check_cancelled()?;
    let mut page = read(previous.commits.len().max(200), None)?;
    for commit in &page.commits {
        remaining.remove(&commit.id);
    }
    loop {
        cancellation.check_cancelled()?;
        if page.next_cursor.is_none() || (!complete && remaining.is_empty()) {
            return Ok(page);
        }
        let cursor = page.next_cursor.as_ref().expect("checked continuation");
        let next = read(200, Some(cursor))?;
        if next
            .next_cursor
            .as_ref()
            .is_some_and(|next| next.last_seen == cursor.last_seen)
        {
            return Err(Error::new(ErrorKind::Backend(
                "history cursor did not advance".into(),
            )));
        }
        // Only inspect the newly read commits on subsequent iterations.
        for commit in &next.commits {
            remaining.remove(&commit.id);
        }
        let mut next = Arc::unwrap_or_clone(next);
        let page = Arc::make_mut(&mut page);
        page.commits.append(&mut next.commits);
        page.next_cursor = next.next_cursor;
    }
}

/// The label of the check a maintenance run returns when git found nothing
/// to do; the run itself writes nothing to a pipe, so its output can't tell.
pub const MAINTENANCE_CHECK_COMMAND: &str = "git maintenance is-needed --auto";

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

    /// One successful output for several steps run as one command: each
    /// stream is the non-empty steps' output, in order, one per line.
    pub fn combine(command: impl Into<String>, outputs: &[CommandOutput]) -> Self {
        let join = |stream: fn(&CommandOutput) -> &str| {
            outputs
                .iter()
                .map(|output| stream(output).trim_end())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Self {
            command: command.into(),
            stdout: join(|output| &output.stdout),
            stderr: join(|output| &output.stderr),
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
pub enum CheckoutRemoteBranchMode {
    Create,
    Overwrite,
}

/// What `git worktree add` checks out in the new worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeCheckout {
    /// An existing branch or commit: `git worktree add <path> <reference>`.
    Existing { reference: String },
    /// `git worktree add --no-track -b <name> <path> [<start>]`; no start means HEAD.
    NewBranch { name: String, start: Option<String> },
    /// `--track -b <name>` from the remote-tracking ref of `upstream`.
    TrackRemote { name: String, upstream: Upstream },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteUrlKind {
    Fetch,
    Push,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveRebaseAction {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

impl InteractiveRebaseAction {
    pub fn to_todo_str(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }

    /// Inverse of [`Self::to_todo_str`], also accepting git's single-letter
    /// abbreviations. `None` for todo commands that are not entry actions
    /// (`exec`, `merge`, `label`, …).
    pub fn from_todo_word(word: &str) -> Option<Self> {
        Some(match word {
            "pick" | "p" => Self::Pick,
            "reword" | "r" => Self::Reword,
            "squash" | "s" => Self::Squash,
            "fixup" | "f" => Self::Fixup,
            "drop" | "d" => Self::Drop,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SequencerState {
    #[default]
    None,
    RebaseOrApply,
    CherryPick,
    /// A revert stopped at a conflict or a failed commit step, or a revert
    /// sequence still pending.
    Revert,
}

/// Marker a revert puts in its command output when git applied nothing because
/// the branch no longer has the reverted changes, so no commit was created.
pub const REVERT_NOTHING_TO_REVERT_SENTINEL: &str = "GITCOMET_REVERT_NOTHING_TO_REVERT";

/// Marker an applied file change puts in its output when the branch already has
/// that change, so nothing was applied or committed.
pub const APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL: &str =
    "GITCOMET_APPLY_CHANGE_ALREADY_APPLIED";

/// The revision an applied change comes from, as messages name it:
/// `abc1234`, or `abc1234..def5678` for a comparison.
pub fn apply_change_revision(source: &ApplyChangeSource) -> String {
    match source {
        ApplyChangeSource::Commit(commit_id) => commit_id.short().to_owned(),
        ApplyChangeSource::Range { from, to } => format!("{}..{}", from.short(), to.short()),
    }
}

/// Commit message for file changes applied from a comparison, which has no
/// single source commit message to reuse. More than one file is listed in
/// the body.
pub fn apply_file_change_range_message(
    from: &CommitId,
    to: &CommitId,
    paths: &[PathBuf],
) -> String {
    let range = format!("{}..{}", from.short(), to.short());
    match paths {
        [path] => format!("Apply {} from {range}", path.display()),
        _ => {
            let mut message = format!("Apply {} files from {range}\n", paths.len());
            for path in paths {
                message.push_str(&format!("\n- {}", path.display()));
            }
            message
        }
    }
}

/// Command label of a Continue that skipped a revert its resolution left empty.
pub const REVERT_SKIP_COMMAND: &str = "git revert --skip";

/// Marker an abort puts in its output when git cleared a leftover sequence but
/// refused to rewind HEAD, so the summary cannot claim the previous state back.
pub const REVERT_ABORT_KEPT_HEAD_SENTINEL: &str = "GITCOMET_REVERT_ABORT_KEPT_HEAD";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractiveRebaseEntry {
    pub action: InteractiveRebaseAction,
    pub commit_id: String,
    /// Single-line original commit subject (git's `%s`), used for list display
    /// and autosquash grouping. Never reflects `new_message` — display code
    /// derives an edited subject via `squash::split_subject_body`.
    pub summary: String,
    /// Full original commit message (subject + body). Seeds the reword dialog
    /// (`squash::reword_seed_message`) and contributes to combined squash
    /// messages; never edited in place.
    pub message: String,
    /// Full replacement message (subject + body), set only when action is
    /// Reword. Its subject may differ from `summary`.
    pub new_message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustTarget {
    pub submodule_path: PathBuf,
    pub display_source: String,
    pub local_source_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustDecision {
    Proceed,
    Prompt { sources: Vec<SubmoduleTrustTarget> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlameLine {
    pub commit_id: Arc<str>,
    pub author: Arc<str>,
    pub author_time_unix: Option<i64>,
    pub summary: Arc<str>,
    pub body: Option<Arc<str>>,
    pub line: String,
    /// Whether the blamed file existed in the first parent of `commit_id`.
    /// When `false`, "view file at parent commit" is a dead end (this commit
    /// introduced the file), so the UI hides that affordance.
    pub prior_exists: bool,
    /// The file's path at `commit_id`, when it differs from the blamed path
    /// because the file was renamed at/after that commit. `None` means the path
    /// is the same as the blamed path. Used so "view file at this commit" and
    /// "prior revision" navigate using the historical name rather than the
    /// current one (which may not exist in that older tree).
    pub source_path: Option<PathBuf>,
    /// For uncommitted ("Not Committed Yet") lines, the commit the working-tree
    /// change is based on (git blame porcelain `previous`), i.e. the revision to
    /// open for "view file at parent commit". `None` for committed lines (which
    /// resolve their parent from `commit_id`) and for uncommitted lines with no
    /// base (newly added files).
    pub prior_commit: Option<Arc<str>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommitOperationOutcome {
    pub local_branch: Option<String>,
    pub pre_head: Option<CommitId>,
    pub post_head: Option<CommitId>,
}

/// What [`GitRepository::reword_commit_with_output`] did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RewordCommitOutcome {
    pub output: CommandOutput,
    /// The rewritten commit's new id, when the backend could tell it.
    pub rewritten_id: Option<CommitId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafePushAfterCommitContext {
    pub amend: bool,
    pub local_branch: Option<String>,
    pub pre_head: Option<CommitId>,
    pub post_head: Option<CommitId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafePushAfterCommitTarget {
    pub remote: String,
    pub branch: String,
    pub local_branch: String,
    pub local_head: CommitId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForcePushLease {
    pub remote: String,
    pub branch: String,
    pub expected: CommitId,
    pub local_branch: String,
    pub local_head: CommitId,
}

/// Replay only `upstream..expected_head` onto an immutable parent, without
/// updating other branches or stashing the caller's working files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebaseRange {
    pub onto: CommitId,
    pub upstream: CommitId,
    pub expected_head: CommitId,
    pub preserve_merges: bool,
}

/// A compare-and-swap ref update. `None` requires the ref to be absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefUpdate {
    pub reference: String,
    pub expected: Option<CommitId>,
    pub new: CommitId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchUpdate {
    pub update: RefUpdate,
    pub worktrees: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteRefUpdate {
    pub branch: String,
    pub expected: CommitId,
    pub new: CommitId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SafePushAfterCommitDecision {
    Push {
        target: SafePushAfterCommitTarget,
    },
    PushSetUpstream {
        target: SafePushAfterCommitTarget,
    },
    Blocked {
        summary: String,
        lease: Option<ForcePushLease>,
    },
}

/// Backend operations, including optional capabilities with conservative defaults.
///
/// Cancellable methods take the caller's token explicitly. Implementations that
/// perform work check it before starting and before returning a successful
/// result; a caller without a cancellable task can pass a fresh token. Keep the
/// same token when delegating so cancellation reaches the underlying work.
pub trait GitRepository: Send + Sync {
    fn acquire_operation_lease(&self) -> Result<RepositoryOperationLease> {
        Ok(RepositoryOperationLease::default())
    }

    fn repository_mode(&self) -> Result<RepositoryMode> {
        Ok(RepositoryMode::WorkingTree)
    }

    fn preview_disable_bare_mode(&self) -> Result<BareConversionPreview> {
        Err(Error::new(ErrorKind::Unsupported(
            "bare mode conversion is not implemented for this backend",
        )))
    }

    fn disable_bare_mode(&self, _preview: &BareConversionPreview) -> Result<BareModeDisabled> {
        Err(Error::new(ErrorKind::Unsupported(
            "bare mode conversion is not implemented for this backend",
        )))
    }

    fn disable_bare_mode_cancellable(
        &self,
        preview: &BareConversionPreview,
        cancellation: &CancellationToken,
    ) -> Result<BareModeDisabled> {
        cancellation.check_cancelled()?;
        self.disable_bare_mode(preview)
    }

    fn rebase_range_with_output(&self, _range: &RebaseRange) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "explicit-range rebase is not implemented",
        )))
    }
    fn update_refs_with_output(&self, _updates: &[RefUpdate]) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "ref transactions are not implemented",
        )))
    }
    /// Ref locks remain held while clean checked-out branches are synchronized.
    /// Callers must journal the transaction before invoking this operation.
    fn apply_branch_updates_with_output(&self, _updates: &[BranchUpdate]) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "branch transactions are not implemented",
        )))
    }
    /// Resume a journalled branch transaction after interruption. Every ref and
    /// clean index must still equal either its captured original or prepared
    /// revision. Unexpected tips or edits are preserved and reject recovery.
    fn recover_branch_updates_with_output(
        &self,
        _updates: &[BranchUpdate],
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "branch transaction recovery is not implemented",
        )))
    }
    fn push_refs_with_lease_with_output(
        &self,
        _remote: &str,
        _updates: &[RemoteRefUpdate],
        _atomic: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "multi-ref lease publication is not implemented",
        )))
    }
    fn spec(&self) -> &RepoSpec;

    /// Drops held object-store handles so their pack files can be deleted and
    /// packs written since become visible; later reads reopen the store.
    fn release_object_store(&self) {}

    /// Distinct author names across the walk's complete history, without an
    /// author filter. Called on demand, independently of the visible commit
    /// metadata cache.
    ///
    /// `focus` narrows the walk the way [`Self::read_history`] does, so the
    /// catalog offers the authors of the commits actually on screen.
    fn history_authors(
        &self,
        walk: &HistoryWalk,
        focus: &HistoryFocusSet,
        cancellation: &CancellationToken,
    ) -> Result<Arc<[Arc<str>]>> {
        let _ = focus;
        refuse_hidden_remotes(walk)?;
        let mode = walk.mode();
        let mut authors = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor = None;
        loop {
            cancellation.check_cancelled()?;
            let page = self.log_history_mode_page(mode, 256, cursor.as_ref(), cancellation)?;
            for commit in &page.commits {
                if seen.insert(commit.author.clone()) {
                    authors.push(commit.author.clone());
                }
            }
            cursor = page.next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }
        cancellation.check_cancelled()?;
        Ok(authors.into())
    }

    /// Optional indexed access to history. `None` keeps older backends on the
    /// paged reader. Construction is background work; range reads never walk
    /// from the branch tip to the requested offset.
    fn build_history_index(
        &self,
        _walk: &HistoryWalk,
        _author: Option<&str>,
        _focus: &HistoryFocusSet,
        cancellation: &CancellationToken,
        _on_progress: &mut dyn FnMut(crate::history_index::HistoryIndexProgress),
    ) -> Result<Option<crate::history_index::HistoryIndexHandle>> {
        cancellation.check_cancelled()?;
        Ok(None)
    }

    fn read_history_range(
        &self,
        _index: &crate::history_index::HistoryIndexHandle,
        _range: std::ops::Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<crate::history_index::HistoryRange> {
        cancellation.check_cancelled()?;
        Err(Error::new(ErrorKind::Backend(
            "indexed history is unavailable".into(),
        )))
    }

    /// Share immutable decoded blocks across consumers. Existing backends keep
    /// their value-returning implementation; indexed delivery retains this Arc.
    fn read_history_range_shared(
        &self,
        index: &crate::history_index::HistoryIndexHandle,
        range: std::ops::Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<crate::history_index::HistoryRange>> {
        self.read_history_range(index, range, cancellation)
            .map(Arc::new)
    }

    /// Read or refresh a history snapshot. Backends without snapshot support
    /// conservatively rebuild and never claim an unchanged result.
    ///
    /// A non-empty `focus` replaces the walk's tips with the refs it names, so
    /// the page holds only the commits those refs reach. The default
    /// implementation has no way to reseed the paged reader and ignores it, the
    /// same way it ignores `author`; backends that support focus override this.
    fn read_history(
        &self,
        walk: &HistoryWalk,
        author: Option<&str>,
        focus: &HistoryFocusSet,
        request: &HistoryReadRequest,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(LogChunk),
    ) -> Result<HistoryReadResult> {
        let _ = focus;
        refuse_hidden_remotes(walk)?;
        let mode = walk.mode();
        let page = match request {
            HistoryReadRequest::Page {
                limit,
                cursor: None,
                ..
            } => self.log_history_mode_page_streaming(
                mode,
                author,
                *limit,
                None,
                cancellation,
                on_chunk,
            )?,
            HistoryReadRequest::Page { limit, cursor, .. } => self.log_history_mode_page_filtered(
                mode,
                author,
                *limit,
                cursor.as_ref(),
                cancellation,
            )?,
            HistoryReadRequest::Refresh { previous, .. } => {
                refresh_history_page(previous, cancellation, |limit, cursor| {
                    self.log_history_mode_page_filtered(mode, author, limit, cursor, cancellation)
                })?
            }
        };
        cancellation.check_cancelled()?;
        Ok(HistoryReadResult::Page {
            page,
            snapshot: None,
        })
    }

    fn log_history_mode_page(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = {
            match mode {
                HistoryMode::AllBranches => self.log_all_branches_page(limit, cursor, cancellation),
                HistoryMode::FullReachable
                | HistoryMode::FirstParent
                | HistoryMode::NoMerges
                | HistoryMode::MergesOnly => self.log_head_page(limit, cursor, cancellation),
            }
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    /// Like [`Self::log_history_mode_page`], but restricted to commits whose
    /// author matches `author` (case-insensitive substring match against the
    /// author name shown in the UI), cancellable, and reporting the page as it
    /// is built.
    ///
    /// An author filter has to walk history until it has found `limit` matching
    /// commits, which for a rare author means walking all of it — over ten
    /// seconds on a repository with a million commits. `on_chunk` lets the
    /// caller show what has been found so far instead of nothing at all, and
    /// `cancellation` lets a filter the user has moved on from be dropped
    /// rather than waited out.
    ///
    /// Each chunk carries the whole page built up to that point, so chunks are
    /// prefixes of each other and of the returned page, and applying one is
    /// idempotent. The default implementation ignores the filter and reports
    /// nothing; backends that support filtering override this method.
    fn log_history_mode_page_streaming(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(LogChunk),
    ) -> Result<std::sync::Arc<LogPage>> {
        let _ = (author, on_chunk);
        cancellation.check_cancelled()?;
        let page = self.log_history_mode_page(mode, limit, cursor, cancellation)?;
        cancellation.check_cancelled()?;
        Ok(page)
    }

    /// A filtered, cancellable page without progress snapshots. Backends can
    /// override this to avoid constructing chunks that the caller will discard.
    fn log_history_mode_page_filtered(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        self.log_history_mode_page_streaming(mode, author, limit, cursor, cancellation, &mut |_| {})
    }

    fn log_head_page(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>>;

    fn log_all_branches_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "all-branches history is not implemented for this backend",
        )))
    }

    fn log_file_page(
        &self,
        _path: &Path,
        _limit: usize,
        _cursor: Option<&LogCursor>,
    ) -> Result<std::sync::Arc<LogPage>> {
        Err(Error::new(ErrorKind::Unsupported(
            "file history is not implemented for this backend",
        )))
    }
    fn commit_details(&self, id: &CommitId) -> Result<CommitDetails>;
    /// Verifies the signatures of `ids`, returning an entry only for commits
    /// that earn a badge. Unsigned commits, and signatures that cannot be
    /// checked because the key is missing, are simply omitted.
    /// Only signatures in `formats` earn a badge; other formats should cost no
    /// verifier run. An empty set returns no signatures without verification.
    ///
    /// Batched on purpose: verification shells out to `git`, and one process per
    /// commit costs roughly ten times a single batched call.
    fn verify_commit_signatures(
        &self,
        _ids: &[CommitId],
        formats: crate::domain::SignatureFormats,
        cancellation: &CancellationToken,
    ) -> Result<Vec<(CommitId, CommitSignature)>> {
        cancellation.check_cancelled()?;
        if formats.is_empty() {
            return Ok(Vec::new());
        }

        Err(Error::new(ErrorKind::Unsupported(
            "signature verification is not implemented for this backend",
        )))
    }

    /// Resolve a possibly abbreviated reference — or any revspec git accepts,
    /// such as a branch, tag, or `HEAD~3` — to the commit it names.
    ///
    /// Deliberately lighter than [`GitRepository::commit_details`], which also
    /// diffs the commit against its parent: this is meant to run per keystroke
    /// behind the Reveal Commit dialog. The returned [`Commit::id`] is the full
    /// oid, *not* the spec that was passed in, so callers can hand it straight
    /// to code that compares against loaded log rows.
    fn resolve_commit(&self, _reference: &CommitId) -> Result<Commit> {
        Err(Error::new(ErrorKind::Unsupported(
            "commit reference resolution is not implemented for this backend",
        )))
    }
    /// Files that differ between `from` and `to` (the working tree when
    /// `None`), honoring `options`, with rename sources, ids, and modes where
    /// the backend reports them. `from` is the older side, so the files read
    /// as what `to` changes; measured from the merge base when asked.
    fn compare_files(
        &self,
        _from: &CommitId,
        _to: Option<&CommitId>,
        _options: &ComparisonOptions,
        _cancellation: &CancellationToken,
    ) -> Result<Comparison> {
        Err(Error::new(ErrorKind::Unsupported(
            "file comparison is not implemented for this backend",
        )))
    }

    /// Added/removed line counts for every uncommitted change, both lanes.
    ///
    /// Separate from `status`, which decides most entries from stat data alone
    /// when possible; stale stat data can require content reads and clean
    /// filters. Counting reads both sides of changed files. Callers that have
    /// just loaded status should reuse it through the supplied-status method.
    fn uncommitted_line_stats(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLineStats> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "uncommitted line stats are not implemented for this backend",
        )))
    }

    /// Count the files from a status snapshot the caller just collected, avoiding
    /// another worktree traversal. Contents are read at call time, as with status.
    fn uncommitted_line_stats_for_status(
        &self,
        _status: &RepoStatus,
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLineStats> {
        self.uncommitted_line_stats(cancellation)
    }

    /// Repository-level Git LFS and git-annex facts from config, attributes and
    /// the local object stores. Must not run either tool.
    fn large_file_support(
        &self,
        _cancellation: &CancellationToken,
    ) -> Result<crate::large_files::LargeFileSupport> {
        Ok(Default::default())
    }

    /// Large-file state for the rows of a status snapshot the caller just
    /// collected. Must not run either tool.
    fn uncommitted_large_files_for_status(
        &self,
        _status: &RepoStatus,
        _cancellation: &CancellationToken,
    ) -> Result<crate::large_files::UncommittedLargeFiles> {
        Ok(Default::default())
    }

    /// Run a Git LFS or git-annex operation through the tool.
    fn run_large_file_command(
        &self,
        _command: &crate::large_files::LargeFileCommand,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "large-file commands are not supported by this backend",
        )))
    }

    /// Where this content key is, including historical or deleted versions.
    fn annex_whereis(
        &self,
        _key: &str,
        _cancellation: &CancellationToken,
    ) -> Result<crate::large_files::AnnexWhereis> {
        Err(Error::new(ErrorKind::Unsupported(
            "git-annex is not supported by this backend",
        )))
    }

    /// Local content no file uses any more, per `git annex unused`.
    fn annex_unused(
        &self,
        _cancellation: &CancellationToken,
    ) -> Result<crate::large_files::AnnexUnused> {
        Err(Error::new(ErrorKind::Unsupported(
            "git-annex is not supported by this backend",
        )))
    }

    /// Current Git LFS locks for the repository (a server round trip).
    fn lfs_locks(
        &self,
        _cancellation: &CancellationToken,
    ) -> Result<Vec<crate::large_files::LfsLock>> {
        Ok(Vec::new())
    }

    /// Full `%B` messages of the given commits, in input order. Message-only
    /// on purpose: callers like the cherry-pick editor need nothing else, and
    /// implementations should skip the per-commit tree diff `commit_details`
    /// pays for.
    fn commit_messages(&self, ids: &[CommitId]) -> Result<Vec<String>> {
        ids.iter()
            .map(|id| self.commit_details(id).map(|details| details.message))
            .collect()
    }
    /// Stable topological ordering of an arbitrary set of commits. Selected
    /// ancestors precede selected descendants even when unselected commits
    /// lie between them; unrelated commits retain their input order.
    fn topologically_order_commits(&self, _ids: &[CommitId]) -> Result<Vec<CommitId>> {
        Err(Error::new(ErrorKind::Unsupported(
            "topological commit ordering is not implemented for this backend",
        )))
    }
    fn recent_commit_messages(&self, _limit: usize) -> Result<Vec<RecentCommitMessage>> {
        Err(Error::new(ErrorKind::Unsupported(
            "recent commit messages are not implemented for this backend",
        )))
    }
    fn reflog_head(&self, limit: usize) -> Result<Vec<ReflogEntry>>;
    fn current_branch(&self, cancellation: &CancellationToken) -> Result<String>;

    fn head_commit_id(&self) -> Result<Option<CommitId>> {
        Err(Error::new(ErrorKind::Unsupported(
            "reading HEAD commit id is not implemented for this backend",
        )))
    }
    fn list_branches(&self, cancellation: &CancellationToken) -> Result<Vec<Branch>>;

    fn list_tags(&self, cancellation: &CancellationToken) -> Result<Vec<Tag>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "tag listing is not implemented for this backend",
        )))
    }

    fn list_remote_tags(&self, cancellation: &CancellationToken) -> Result<Vec<RemoteTag>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "remote tag listing is not implemented for this backend",
        )))
    }

    fn list_remotes(&self, cancellation: &CancellationToken) -> Result<Vec<Remote>>;

    fn list_remote_branches(&self, cancellation: &CancellationToken) -> Result<Vec<RemoteBranch>>;

    fn worktree_status(&self, cancellation: &CancellationToken) -> Result<Vec<FileStatus>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<FileStatus>> = {
            self.status(cancellation)
                .map(|status| std::sync::Arc::unwrap_or_clone(status.unstaged))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn staged_status(&self, cancellation: &CancellationToken) -> Result<Vec<FileStatus>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<FileStatus>> = {
            self.status(cancellation)
                .map(|status| std::sync::Arc::unwrap_or_clone(status.staged))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn status(&self, cancellation: &CancellationToken) -> Result<RepoStatus>;

    /// Whether `path` is a gitlink in the current HEAD tree.
    ///
    /// This is primarily needed for a staged deletion: the worktree marker is
    /// already gone, so the old tree is the only reliable way to distinguish a
    /// deleted submodule from a deleted regular file.
    fn head_path_is_gitlink(&self, _path: &Path) -> Result<bool> {
        Ok(false)
    }
    fn upstream_divergence(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<UpstreamDivergence>> {
        cancellation.check_cancelled()?;
        let result: Result<Option<UpstreamDivergence>> = { Ok(None) };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn diff_unified(&self, target: &DiffTarget) -> Result<String>;
    fn diff_action_snapshot(
        &self,
        _target: &DiffTarget,
        _cancellation: &CancellationToken,
    ) -> Result<crate::patch_action::DiffActionSnapshot> {
        Err(Error::new(ErrorKind::Unsupported(
            "exact partial patches are not available for this backend",
        )))
    }

    fn prepare_diff_patch(
        &self,
        target: &DiffTarget,
        operation: crate::patch_action::PatchOperation,
        selection: &crate::patch_action::SourceLineSelection,
        old: &str,
        new: &str,
        cancellation: &CancellationToken,
    ) -> Result<crate::patch_action::PreparedDiffPatch> {
        // A snapshot torn by a concurrent write cannot match the one
        // `apply_prepared_diff_patch` reads under the write gate, so one read
        // is enough here.
        let snapshot = self.diff_action_snapshot(target, cancellation)?;
        crate::patch_action::prepare(snapshot, operation, selection, old, new)
            .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))
    }

    fn apply_prepared_diff_patch(
        &self,
        prepared: &crate::patch_action::PreparedDiffPatch,
    ) -> Result<CommandOutput> {
        let current = self.diff_action_snapshot(prepared.target(), &CancellationToken::new())?;
        prepared
            .validate(&current)
            .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?;
        match prepared.operation() {
            crate::patch_action::PatchOperation::Stage => {
                self.apply_unified_patch_to_index_with_output(prepared.patch(), false)
            }
            crate::patch_action::PatchOperation::Unstage => {
                self.apply_unified_patch_to_index_with_output(prepared.patch(), true)
            }
            crate::patch_action::PatchOperation::Discard => {
                self.apply_unified_patch_to_worktree_with_output(prepared.patch(), true)
            }
        }
    }
    /// Load and parse unified diff rows for the target.
    ///
    /// Default implementation goes through `diff_unified`; backends may
    /// override for streaming parsing to avoid large monolithic allocations.
    /// `encoding` is the user's explicit decoding choice, when supported.
    fn diff_parsed(
        &self,
        target: &DiffTarget,
        _encoding: Option<crate::text_format::TextEncoding>,
        cancellation: &CancellationToken,
    ) -> Result<Diff> {
        cancellation.check_cancelled()?;
        let result: Result<Diff> = {
            self.diff_unified(target)
                .map(|text| Diff::from_unified(target.clone(), &text))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    /// Read both sides as UTF-8 content, using `encoding` when the user chose one.
    fn diff_file_text(
        &self,
        _target: &DiffTarget,
        _encoding: Option<crate::text_format::TextEncoding>,
        cancellation: &CancellationToken,
    ) -> Result<Option<FileDiffText>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "file diff view is not implemented for this backend",
        )))
    }

    /// `.gitattributes` and git config deciding how `path`'s content reads.
    fn text_attributes(&self, _path: &Path) -> Result<TextAttributes> {
        Ok(TextAttributes::default())
    }
    fn diff_preview_text_file(
        &self,
        _target: &DiffTarget,
        _side: DiffPreviewTextSide,
        cancellation: &CancellationToken,
    ) -> Result<Option<DiffPreviewTextFile>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "preview text file loading is not implemented for this backend",
        )))
    }

    fn diff_file_image(
        &self,
        _target: &DiffTarget,
        cancellation: &CancellationToken,
    ) -> Result<Option<FileDiffImage>> {
        cancellation.check_cancelled()?;

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
    /// `encoding` selects how every side is decoded when the user chose one.
    fn conflict_session(
        &self,
        _path: &Path,
        _encoding: Option<crate::text_format::TextEncoding>,
    ) -> Result<Option<ConflictSession>> {
        Err(Error::new(ErrorKind::Unsupported(
            "conflict session loading is not implemented for this backend",
        )))
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()>;
    /// Create the local branch `name` at `target`, or reset it there when a
    /// branch with that name already exists, and check it out. The git
    /// equivalent is `git checkout --no-track -B <name> <target>`; like a fresh
    /// branch the result tracks nothing, so an existing upstream is cleared.
    fn create_branch_force_and_checkout(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "force branch creation and checkout is not implemented for this backend",
        )))
    }
    fn rename_branch(&self, _old_name: &str, _new_name: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "branch renaming is not implemented for this backend",
        )))
    }
    /// `git branch -M <old> <new>`. When `new_name` is HEAD here (git refuses
    /// `-M` then) the branch is reset to `old_name`'s commit with a safe
    /// checkout and `old_name` is deleted, detaching any worktree still on it.
    /// Fails like git when `new_name` is checked out in another worktree; find
    /// it with `branch_checked_out_in_other_worktree` and rename from there.
    fn rename_branch_force(&self, _old_name: &str, _new_name: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "forced branch renaming is not implemented for this backend",
        )))
    }
    /// Canonical, validated path of the other worktree whose HEAD is `name`.
    /// Defaults to `Ok(None)` (like `head_path_is_gitlink`) so backends without
    /// worktrees fall through to a plain checkout. Implementations must not
    /// return a path that is outside this repository.
    fn branch_checked_out_in_other_worktree(&self, _name: &str) -> Result<Option<PathBuf>> {
        Ok(None)
    }
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
        _mode: CheckoutRemoteBranchMode,
    ) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "remote branch checkout is not implemented for this backend",
        )))
    }
    fn checkout_commit(&self, id: &CommitId) -> Result<()>;
    fn cherry_pick(&self, id: &CommitId) -> Result<()>;
    /// Runs a single cherry-pick. `mainline` is Git's 1-based parent number
    /// for a merge commit and must be `None` for non-merge commits.
    fn cherry_pick_with_output(
        &self,
        _id: &CommitId,
        _commit: bool,
        _mainline: Option<usize>,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git cherry-pick is not implemented for this backend",
        )))
    }
    /// The message git left for the next commit (MERGE_MSG), if any. Unlike
    /// [`Self::merge_commit_message`] this does not require a merge.
    fn commit_message_template(&self) -> Result<Option<String>> {
        Ok(None)
    }
    /// Reverts a single commit. `commit: false` only stages the inverse;
    /// `mainline` follows [`Self::cherry_pick_with_output`].
    fn revert_with_output(
        &self,
        _id: &CommitId,
        _commit: bool,
        _mainline: Option<usize>,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git revert is not implemented for this backend",
        )))
    }

    /// Applies the change to `target`'s files from a commit or comparison to
    /// the index and worktree with a 3-way fallback, then with `commit`
    /// commits just those paths.
    fn apply_file_change_with_output(
        &self,
        _target: &crate::domain::ApplyChangeTarget,
        _commit: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "applying a file change is not implemented for this backend",
        )))
    }

    /// Retry only the failed commit step, after checking that HEAD and the
    /// applied paths still match the checkpoint returned by that failure.
    fn commit_applied_file_change_with_output(
        &self,
        _retry: &crate::domain::ApplyFileChangeRetry,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "committing an applied file change is not implemented for this backend",
        )))
    }

    fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()>;
    fn stash_list(&self, cancellation: &CancellationToken) -> Result<Vec<StashEntry>>;

    fn stash_apply(&self, index: usize) -> Result<()>;
    fn stash_drop(&self, index: usize) -> Result<()>;

    fn stage(&self, paths: &[&Path]) -> Result<()>;
    fn unstage(&self, paths: &[&Path]) -> Result<()>;
    fn commit(&self, message: &str) -> Result<()>;
    fn commit_with_outcome(&self, message: &str) -> Result<CommitOperationOutcome> {
        self.commit(message)?;
        Ok(CommitOperationOutcome::default())
    }
    fn commit_amend(&self, _message: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported(
            "commit amend is not implemented for this backend",
        )))
    }
    fn commit_amend_with_outcome(&self, message: &str) -> Result<CommitOperationOutcome> {
        self.commit_amend(message)?;
        Ok(CommitOperationOutcome::default())
    }

    /// Identity override for this operation. Existing implementations remain
    /// compatible and refuse overrides until they implement this method.
    fn commit_with_identity(
        &self,
        message: &str,
        identity: Option<&crate::commit_identity::CommitIdentity>,
    ) -> Result<CommitOperationOutcome> {
        if identity.is_some() {
            return Err(Error::new(ErrorKind::Unsupported(
                "operation commit identity is not supported by this backend",
            )));
        }
        self.commit_with_outcome(message)
    }
    /// Git retains the original author when amending; the override selects the
    /// committer. No repository configuration is written.
    fn commit_amend_with_identity(
        &self,
        message: &str,
        identity: Option<&crate::commit_identity::CommitIdentity>,
    ) -> Result<CommitOperationOutcome> {
        if identity.is_some() {
            return Err(Error::new(ErrorKind::Unsupported(
                "operation commit identity is not supported by this backend",
            )));
        }
        self.commit_amend_with_outcome(message)
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
    fn list_commits_for_interactive_rebase(
        &self,
        _base: &str,
    ) -> Result<Vec<InteractiveRebaseEntry>> {
        Err(Error::new(ErrorKind::Unsupported(
            "listing commits for interactive rebase is not implemented for this backend",
        )))
    }
    fn interactive_rebase_with_output(
        &self,
        _base: &str,
        _entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git rebase -i is not implemented for this backend",
        )))
    }
    /// Replaces the message of `commit_id`, which must be HEAD or a commit on
    /// HEAD's linear first-parent history. Trees are unchanged: HEAD is
    /// amended without its staged changes, and an older commit is reworded
    /// by replaying its descendants on top of it.
    fn reword_commit_with_output(
        &self,
        _commit_id: &CommitId,
        _message: &str,
    ) -> Result<RewordCommitOutcome> {
        Err(Error::new(ErrorKind::Unsupported(
            "rewording a commit is not implemented for this backend",
        )))
    }
    /// Continue a paused message edit only when its persisted rebase belongs
    /// to this target and message. This retains the rewritten target's identity
    /// across signing retries, unlike a generic rebase continuation.
    fn continue_reword_commit_with_output(
        &self,
        _commit_id: &CommitId,
        _message: &str,
    ) -> Result<RewordCommitOutcome> {
        Err(Error::new(ErrorKind::Unsupported(
            "continuing a commit message edit is not implemented for this backend",
        )))
    }
    /// Whether the paused operation is the message edit identified by these
    /// arguments. An unrelated rebase must never be resumed as a signing retry.
    fn reword_commit_in_progress(&self, _commit_id: &CommitId, _message: &str) -> Result<bool> {
        Ok(false)
    }
    /// Picks `entries` in order. Without `commit` every pick only merges into
    /// the index and worktree, and reword/squash/fixup steps are refused.
    fn interactive_cherry_pick_with_output(
        &self,
        _entries: &[InteractiveRebaseEntry],
        _commit: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "interactive cherry-pick is not implemented for this backend",
        )))
    }
    fn merge_abort_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git merge --abort is not implemented for this backend",
        )))
    }
    fn rebase_in_progress(&self, cancellation: &CancellationToken) -> Result<bool> {
        cancellation.check_cancelled()?;
        let result: Result<bool> = { Ok(false) };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn sequencer_state(&self, cancellation: &CancellationToken) -> Result<SequencerState> {
        cancellation.check_cancelled()?;
        let result: Result<SequencerState> = {
            Ok(if self.rebase_in_progress(cancellation)? {
                SequencerState::RebaseOrApply
            } else {
                SequencerState::None
            })
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn merge_commit_message(&self, cancellation: &CancellationToken) -> Result<Option<String>> {
        cancellation.check_cancelled()?;
        let result: Result<Option<String>> = { Ok(None) };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn create_tag_with_output(
        &self,
        _name: &str,
        _target: &str,
        _message: Option<&str>,
        _annotated: bool,
    ) -> Result<CommandOutput> {
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
    /// The shared git directory (the main `.git` of a linked worktree),
    /// canonicalized; worktrees of one repository share its maintenance.
    fn common_dir(&self) -> Option<std::path::PathBuf> {
        None
    }
    /// Whether the repository needs substantial maintenance: git's gc
    /// thresholds (`gc.auto` loose objects, `gc.autoPackLimit` packs).
    fn maintenance_needed(&self) -> Result<bool> {
        Err(Error::new(ErrorKind::Unsupported(
            "maintenance checks are not implemented for this backend",
        )))
    }
    /// Runs the maintenance git recommends, in the foreground, reporting
    /// progress through the attached git operation. When git finds nothing
    /// to do it skips the run and returns the check, labelled
    /// [`MAINTENANCE_CHECK_COMMAND`].
    fn run_maintenance_with_output(&self) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "maintenance is not implemented for this backend",
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

    fn add_remote_with_output(
        &self,
        _name: &str,
        _url: &str,
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
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
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "git remote set-url is not implemented for this backend",
        )))
    }

    fn fetch_all(&self, _prune: bool) -> Result<CommandOutput>;
    fn pull(&self, mode: PullMode, _prune: bool) -> Result<CommandOutput>;
    fn push(&self) -> Result<()>;

    fn push_with_tags(&self, _request: &crate::tag_push::TagPushRequest) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "pushing with tags is not implemented for this backend",
        )))
    }

    fn preview_tag_push(
        &self,
        _request: &crate::tag_push::TagPushRequest,
        _cancellation: &CancellationToken,
    ) -> Result<crate::tag_push::TagPushPreview> {
        Err(Error::new(ErrorKind::Unsupported(
            "tag push preview is not implemented for this backend",
        )))
    }

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

    /// Fetches exactly `refspecs` from `remote` (for example
    /// `+refs/pull/7/head:refs/remotes/origin/pr/7`), pruning nothing.
    fn fetch_refspecs_with_output(
        &self,
        _remote: &str,
        _refspecs: &[String],
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "fetching refspecs is not implemented for this backend",
        )))
    }

    fn push_with_output(&self) -> Result<CommandOutput> {
        self.push()?;
        Ok(CommandOutput::empty_success("git push"))
    }

    fn push_force_with_output(&self) -> Result<CommandOutput> {
        self.push_force()?;
        Ok(CommandOutput::empty_success("git push --force-with-lease"))
    }

    fn safe_push_after_commit(
        &self,
        _context: &SafePushAfterCommitContext,
    ) -> Result<SafePushAfterCommitDecision> {
        Err(Error::new(ErrorKind::Unsupported(
            "safe push after commit is not implemented for this backend",
        )))
    }

    fn push_after_commit_with_output(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        validate_safe_push_after_commit_target(self, target)?;
        self.push_with_output()
    }

    fn push_after_commit_set_upstream_with_output(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        validate_safe_push_after_commit_target(self, target)?;
        self.push_set_upstream_with_output(&target.remote, &target.branch)
    }

    fn push_force_with_lease_with_output(&self, lease: &ForcePushLease) -> Result<CommandOutput> {
        let _ = lease;
        Err(Error::new(ErrorKind::Unsupported(
            "oid-specific force push with lease is not implemented for this backend",
        )))
    }

    fn push_set_upstream_with_output(&self, remote: &str, branch: &str) -> Result<CommandOutput> {
        self.push_set_upstream(remote, branch)?;
        Ok(CommandOutput::empty_success(format!(
            "git push --set-upstream {remote} HEAD:refs/heads/{branch}"
        )))
    }

    /// Set an exact upstream target. The remote and remote branch stay
    /// separate because either name may contain slashes. The target may be a
    /// new remote branch that has not been fetched or pushed yet.
    fn set_upstream_branch_with_output(
        &self,
        _branch: &str,
        _upstream: &Upstream,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "setting a branch upstream is not implemented for this backend",
        )))
    }

    fn unset_upstream_branch_with_output(&self, _branch: &str) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "unsetting a branch upstream is not implemented for this backend",
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

    /// Delete several branches on one remote.
    ///
    /// A batch method rather than a caller-side loop because deleting is a push:
    /// one invocation carrying every ref is a single network round trip, where
    /// the loop pays one per branch. The default keeps that loop so backends
    /// that only implement the single-branch call stay correct.
    fn delete_remote_branches_with_output(
        &self,
        remote: &str,
        branches: &[String],
    ) -> Result<CommandOutput> {
        let mut last = CommandOutput::empty_success("git push --delete");
        for branch in branches {
            last = self.delete_remote_branch_with_output(remote, branch)?;
        }
        Ok(last)
    }

    fn commit_amend_with_output(&self, message: &str) -> Result<CommandOutput> {
        self.commit_amend(message)?;
        Ok(CommandOutput::empty_success("git commit --amend"))
    }

    fn pull_branch_with_output(
        &self,
        _remote: &str,
        _branch: &str,
        _prune: bool,
    ) -> Result<CommandOutput> {
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

    /// Builds the default combined message for squashing the linear commit
    /// range `oldest..=head`: the oldest commit's full message first, younger
    /// messages appended as paragraphs.
    fn squash_message_preview(&self, _oldest: &CommitId, _head: &CommitId) -> Result<String> {
        Err(Error::new(ErrorKind::Unsupported(
            "squashing commits is not implemented for this backend",
        )))
    }

    /// Squashes the linear first-parent range `oldest..=expected_head` (which
    /// must end at the current HEAD) into a single commit carrying `message`,
    /// preserving the oldest commit's author. Must not touch the worktree or
    /// index, and must fail without changing refs when HEAD no longer equals
    /// `expected_head`.
    fn squash_commits_with_output(
        &self,
        _oldest: &CommitId,
        _expected_head: &CommitId,
        _message: &str,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "squashing commits is not implemented for this backend",
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

    /// Resolve the path of the file currently known as `path` (at some revision)
    /// to the name it has in `commit`'s tree, following renames. Returns `None`
    /// when it cannot be determined (the caller should then fall back to `path`).
    ///
    /// This lets "view file at this commit" navigate across renames: a file's
    /// name in an older/newer commit may differ from the path the caller holds,
    /// and opening the wrong name would fail because it is absent from that tree.
    fn resolve_file_path_at_commit(
        &self,
        _path: &Path,
        _commit: &CommitId,
    ) -> Result<Option<PathBuf>> {
        Ok(None)
    }

    /// Blame the working-tree content shown on the new side of a staged/unstaged
    /// diff. Lines matching committed history are attributed to their commit;
    /// lines not yet committed are returned as "Not Committed Yet" entries.
    fn blame_worktree_file(&self, _path: &Path, _area: DiffArea) -> Result<Vec<BlameLine>> {
        Err(Error::new(ErrorKind::Unsupported(
            "git blame of working-tree content is not implemented for this backend",
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
        _patch: &[u8],
        _reverse: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "index patch apply is not implemented for this backend",
        )))
    }

    fn apply_unified_patch_to_worktree_with_output(
        &self,
        _patch: &[u8],
        _reverse: bool,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree patch apply is not implemented for this backend",
        )))
    }

    fn list_worktrees(&self, cancellation: &CancellationToken) -> Result<Vec<Worktree>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "worktree listing is not implemented for this backend",
        )))
    }

    /// Tip-commit author/date/summary for every local and remote-tracking ref,
    /// as `(short refname, metadata)` pairs. Purely decorative — callers render
    /// name-only rows when this is unavailable, so backends may leave it
    /// unimplemented.
    fn list_ref_metadata(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Arc<rustc_hash::FxHashMap<String, RefMetadata>>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "ref metadata listing is not implemented for this backend",
        )))
    }

    fn add_worktree_with_output(
        &self,
        _path: &Path,
        _checkout: &WorktreeCheckout,
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

    fn list_submodules(&self, cancellation: &CancellationToken) -> Result<Vec<Submodule>> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "submodule listing is not implemented for this backend",
        )))
    }

    /// The working directory as it is on disk, not `HEAD`'s tree.
    fn list_worktree_files(&self) -> Result<Vec<FileEntry>> {
        Err(Error::new(ErrorKind::Unsupported(
            "worktree file listing is not implemented for this backend",
        )))
    }

    fn list_tree_files_at_commit(&self, _commit_id: &CommitId) -> Result<Vec<FileEntry>> {
        Err(Error::new(ErrorKind::Unsupported(
            "tree file listing at commit is not implemented for this backend",
        )))
    }

    fn submodule_diff_summary(
        &self,
        _target: &crate::domain::DiffTarget,
        cancellation: &CancellationToken,
    ) -> Result<crate::domain::SubmoduleDiffSummary> {
        cancellation.check_cancelled()?;

        Err(Error::new(ErrorKind::Unsupported(
            "submodule diff summary is not implemented for this backend",
        )))
    }

    fn check_submodule_add_trust(
        &self,
        _url: &str,
        _path: &Path,
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<SubmoduleTrustDecision> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule trust checks are not implemented for this backend",
        )))
    }

    fn check_submodule_update_trust(
        &self,
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<SubmoduleTrustDecision> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule trust checks are not implemented for this backend",
        )))
    }

    fn check_submodule_load_trust(
        &self,
        _path: &Path,
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<SubmoduleTrustDecision> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule trust checks are not implemented for this backend",
        )))
    }

    fn add_submodule_with_output(
        &self,
        _url: &str,
        _path: &Path,
        _branch: Option<&str>,
        _name: Option<&str>,
        _force: bool,
        _approved_sources: &[SubmoduleTrustTarget],
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule add is not implemented for this backend",
        )))
    }

    fn update_submodules_with_output(
        &self,
        _approved_sources: &[SubmoduleTrustTarget],
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule update is not implemented for this backend",
        )))
    }

    fn load_submodule_with_output(
        &self,
        _path: &Path,
        _approved_sources: &[SubmoduleTrustTarget],
        _remote_url_policy: crate::remote_url::RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule update is not implemented for this backend",
        )))
    }

    fn change_submodule_pointer_with_output(
        &self,
        _path: &Path,
        _reference: &str,
    ) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule pointer changes are not implemented for this backend",
        )))
    }

    fn remove_submodule_with_output(&self, _path: &Path) -> Result<CommandOutput> {
        Err(Error::new(ErrorKind::Unsupported(
            "submodule remove is not implemented for this backend",
        )))
    }

    fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()>;
}

fn validate_safe_push_after_commit_target<R: GitRepository + ?Sized>(
    repo: &R,
    target: &SafePushAfterCommitTarget,
) -> Result<()> {
    let current_branch = repo.current_branch(&CancellationToken::new())?;
    if current_branch != target.local_branch {
        return Err(Error::new(ErrorKind::Backend(format!(
            "stale push-after-commit target: expected branch {}, but current branch is {}",
            target.local_branch, current_branch
        ))));
    }

    let current_head = repo.head_commit_id()?.ok_or_else(|| {
        Error::new(ErrorKind::Backend(
            "stale push-after-commit target: current HEAD does not point to a commit".to_string(),
        ))
    })?;
    if current_head != target.local_head {
        return Err(Error::new(ErrorKind::Backend(format!(
            "stale push-after-commit target: expected HEAD {}, but current HEAD is {}",
            target.local_head, current_head
        ))));
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PullMode {
    Default,
    Merge,
    FastForwardIfPossible,
    FastForwardOnly,
    Rebase,
}

/// Filesystem kind supplied to a worktree ignore matcher.
///
/// Watcher events do not always carry a reliable kind, so callers can leave it
/// unknown and let the concrete Git implementation use its normal fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorktreePathKind {
    File,
    Directory,
    Unknown,
}

/// Stateful ignore matcher created by a concrete Git backend.
///
/// Keeping this narrow interface in core lets filesystem monitoring honor
/// backend-specific ignore semantics without depending on the backend crate.
pub trait WorktreeIgnoreMatcher: Send {
    fn is_ignored(&mut self, relative_path: &Path, kind: WorktreePathKind) -> Result<bool>;
}

/// Filesystem inputs for monitoring one repository, including linked worktrees.
/// Paths may name files which do not exist yet (for example `info/exclude`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepositoryWatchInfo {
    pub git_dirs: Vec<PathBuf>,
    /// Additional private cache directories, including the LFS-owned folders
    /// in configured storage. These can be absent until the first filter run.
    pub cache_dirs: Vec<PathBuf>,
    pub ignore_inputs: Vec<PathBuf>,
    /// Initialized checkouts that can supply ignore matchers. Administrative
    /// repositories retained after submodule deinit belong only in `git_dirs`.
    /// Includes indexed gitlinks even when they have no `.gitmodules` entry.
    pub worktrees: Vec<PathBuf>,
    /// Discovery hit a filesystem error or its bounded administrative walk.
    pub discovery_incomplete: bool,
}

/// Ref names History leaves out of its all-branches walk, as patterns over
/// full names: a plain name matches itself and everything under it
/// (`refs/pull` covers `refs/pull/7/head`), `*` matches within one path
/// segment, and `**` across segments (`refs/remotes/*/pr/**`).
///
/// Immutable and cheap to clone; the default excludes nothing. It filters the
/// walk's starting refs only: HEAD is always walked, and a commit reachable
/// from a kept ref stays visible.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct HistoryRefFilter {
    excluded: Arc<[Arc<str>]>,
}

impl HistoryRefFilter {
    pub fn excluding<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Arc<str>>,
    {
        let mut excluded: Vec<Arc<str>> = patterns.into_iter().map(Into::into).collect();
        excluded.retain(|pattern| !pattern.is_empty());
        excluded.sort();
        excluded.dedup();
        Self {
            excluded: excluded.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.excluded.is_empty()
    }

    pub fn patterns(&self) -> &[Arc<str>] {
        &self.excluded
    }

    /// Whether the full ref name `name` (`refs/heads/main`) is left out.
    pub fn excludes(&self, name: &str) -> bool {
        self.excluded
            .iter()
            .any(|pattern| ref_pattern_matches(pattern, name))
    }
}

/// What a history walk starts from: its mode, and the remotes an all-branches
/// walk leaves out. A hidden remote's refs are the remote-tracking refs its
/// fetch refspecs write, unless a shown remote's refspecs write them too.
/// Other modes walk only HEAD, so they hide nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct HistoryWalk {
    mode: HistoryMode,
    hidden_remotes: Arc<[Arc<str>]>,
}

impl HistoryWalk {
    pub fn new<I, S>(mode: HistoryMode, hidden_remotes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Arc<str>>,
    {
        let mut hidden: Vec<Arc<str>> = if mode.is_all_branches() {
            hidden_remotes.into_iter().map(Into::into).collect()
        } else {
            Vec::new()
        };
        hidden.sort();
        hidden.dedup();
        Self {
            mode,
            hidden_remotes: hidden.into(),
        }
    }

    pub fn mode(&self) -> HistoryMode {
        self.mode
    }

    /// Sorted remote names.
    pub fn hidden_remotes(&self) -> &[Arc<str>] {
        &self.hidden_remotes
    }
}

impl From<HistoryMode> for HistoryWalk {
    fn from(mode: HistoryMode) -> Self {
        Self::new(mode, std::iter::empty::<Arc<str>>())
    }
}

fn ref_pattern_matches(pattern: &str, name: &str) -> bool {
    if !pattern.contains('*') {
        let pattern = pattern.trim_end_matches('/');
        return name == pattern
            || name
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/'));
    }
    glob_segments(
        &pattern.split('/').collect::<Vec<_>>(),
        &name.split('/').collect::<Vec<_>>(),
    )
}

fn glob_segments(pattern: &[&str], name: &[&str]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((&"**", rest)) => (0..=name.len()).any(|skip| glob_segments(rest, &name[skip..])),
        Some((segment, rest)) => name
            .split_first()
            .is_some_and(|(first, tail)| glob_segment(segment, first) && glob_segments(rest, tail)),
    }
}

/// `*` within one segment.
fn glob_segment(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

/// Options fixed when a repository opens. The default changes nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub struct RepositoryOptions {
    pub history_ref_filter: HistoryRefFilter,
}

impl RepositoryOptions {
    pub fn with_history_ref_filter(mut self, filter: HistoryRefFilter) -> Self {
        self.history_ref_filter = filter;
        self
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn refuse_hidden_remotes(walk: &HistoryWalk) -> Result<()> {
    if !walk.hidden_remotes().is_empty() {
        return Err(Error::new(ErrorKind::Unsupported(
            "hiding remotes from history is not supported by this backend",
        )));
    }
    Ok(())
}

fn unsupported_repository_options() -> Error {
    Error::new(ErrorKind::Unsupported(
        "repository options are not supported by this backend",
    ))
}

pub trait GitBackend: Send + Sync {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>>;

    /// Releases the object stores of every repository this backend opened on
    /// `common_dir`, in every window, so git can delete the packs they map.
    fn release_object_stores(&self, _common_dir: &Path) {}
    /// Opens `workdir` with `options`. A backend that cannot honor a
    /// non-default option refuses instead of opening without it.
    fn open_with_options(
        &self,
        workdir: &Path,
        options: &RepositoryOptions,
    ) -> Result<Arc<dyn GitRepository>> {
        if options.is_default() {
            self.open(workdir)
        } else {
            Err(unsupported_repository_options())
        }
    }

    /// [`Self::open_with_options`], cancellable.
    fn open_cancellable_with_options(
        &self,
        workdir: &Path,
        options: &RepositoryOptions,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        if options.is_default() {
            self.open_cancellable(workdir, cancellation)
        } else {
            Err(unsupported_repository_options())
        }
    }

    /// Resolve metadata and ignore/configuration sources without running status or filters.
    fn repository_watch_info(&self, _workdir: &Path) -> Result<Option<RepositoryWatchInfo>> {
        Ok(None)
    }

    /// Build a worktree ignore matcher when the backend supports one.
    ///
    /// Backends without ignore support return `None`; callers must then use a
    /// safe not-ignored fallback so filesystem changes are never missed.
    fn worktree_ignore_matcher(
        &self,
        _workdir: &Path,
    ) -> Result<Option<Box<dyn WorktreeIgnoreMatcher>>> {
        Ok(None)
    }

    fn open_cancellable(
        &self,
        workdir: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        cancellation.check_cancelled()?;
        let repo = self.open(workdir)?;
        cancellation.check_cancelled()?;
        Ok(repo)
    }
}

/// A backend whose repositories all open with the same options (for example
/// a product's history ref filter).
pub struct ConfiguredBackend {
    inner: Arc<dyn GitBackend>,
    options: RepositoryOptions,
}

impl ConfiguredBackend {
    pub fn new(inner: Arc<dyn GitBackend>, options: RepositoryOptions) -> Self {
        Self { inner, options }
    }
}

impl GitBackend for ConfiguredBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        self.inner.open_with_options(workdir, &self.options)
    }

    fn release_object_stores(&self, common_dir: &Path) {
        self.inner.release_object_stores(common_dir);
    }

    fn open_with_options(
        &self,
        workdir: &Path,
        options: &RepositoryOptions,
    ) -> Result<Arc<dyn GitRepository>> {
        self.inner.open_with_options(workdir, options)
    }

    fn open_cancellable(
        &self,
        workdir: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.inner
            .open_cancellable_with_options(workdir, &self.options, cancellation)
    }

    fn open_cancellable_with_options(
        &self,
        workdir: &Path,
        options: &RepositoryOptions,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.inner
            .open_cancellable_with_options(workdir, options, cancellation)
    }

    fn repository_watch_info(&self, workdir: &Path) -> Result<Option<RepositoryWatchInfo>> {
        self.inner.repository_watch_info(workdir)
    }

    fn worktree_ignore_matcher(
        &self,
        workdir: &Path,
    ) -> Result<Option<Box<dyn WorktreeIgnoreMatcher>>> {
        self.inner.worktree_ignore_matcher(workdir)
    }
}

/// Backend used by builds that intentionally omit every concrete Git
/// implementation. Keeping this alongside the service contract avoids a whole
/// workspace crate whose only behavior was returning this error.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableGitBackend;

impl GitBackend for UnavailableGitBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "No Git backend enabled. Build with `--features gix`.",
        )))
    }
}

#[cfg(test)]
mod tests;
