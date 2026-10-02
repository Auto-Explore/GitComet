//! Independently owned diff sessions: the target, loads, encoding, and blame
//! of one hosted diff pane, apart from History's selected diff.
//!
//! A session belongs to one repository and is named by a [`DiffViewId`].
//! Work carries the repository's lifetime and the session's generation, so a
//! completion for a retargeted, closed, or reopened session is dropped, and
//! each session's cancellation token stops only its own loads.

use crate::model::{Loadable, RepoId, Shared};
use gitcomet_core::domain::{
    BlameSource, CommitFileChange, CommitId, Diff, DiffArea, DiffTarget, FileDiffImage,
    FileDiffText,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::BlameLine;
use gitcomet_core::services::ComparisonOptions;
use gitcomet_core::services::{CancellationToken, Result};
use gitcomet_core::text_format::{TextAttributes, TextEncoding, TextOverride};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Names one diff session within its repository. Allocate with
/// [`DiffViewId::next`]; ids are never reused in a process.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DiffViewId(pub u64);

impl DiffViewId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// One session's target and loaded content.
#[derive(Clone, Debug)]
pub struct DiffSession {
    pub target: DiffTarget,
    /// The encoding the session reads its file with; `None` follows the
    /// file's attributes and detection.
    pub encoding: Option<TextEncoding>,
    /// Bumped on every retarget, encoding change, or reload; work for an
    /// older generation is dropped.
    pub generation: u64,
    /// Bumped on every change to the session, for fingerprints.
    pub rev: u64,
    /// The complete diff state, shared with the built-in pane.
    pub diff_state: crate::model::DiffState,
    /// Once requested, blame follows subsequent reloads and retargets.
    pub(crate) blame_requested: bool,
    /// An external change during a load requests one more load on completion.
    pub(crate) refresh_queued: bool,
    pub(crate) pending: DiffSessionLoads,
    pub(crate) cancellation: CancellationToken,
}

impl std::ops::Deref for DiffSession {
    type Target = crate::model::DiffState;
    fn deref(&self) -> &Self::Target {
        &self.diff_state
    }
}

impl std::ops::DerefMut for DiffSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.diff_state
    }
}

impl DiffSession {
    pub(crate) fn new(target: DiffTarget) -> Self {
        Self {
            target: target.clone(),
            encoding: None,
            generation: 0,
            rev: 0,
            diff_state: crate::model::DiffState {
                diff_target: Some(target.clone()),
                ..Default::default()
            },
            blame_requested: false,
            refresh_queued: false,
            pending: DiffSessionLoads::default(),
            cancellation: CancellationToken::new(),
        }
    }

    /// Starts a new generation: cancels the old one's work and returns the
    /// new generation's token.
    pub(crate) fn next_generation(&mut self) -> CancellationToken {
        self.refresh_queued = false;
        self.cancellation.cancel();
        self.cancellation = CancellationToken::new();
        self.generation = self.generation.wrapping_add(1);
        self.rev = self.rev.wrapping_add(1);
        self.cancellation.clone()
    }

    /// The linked worktree the target reads; `None` is the repository's own.
    pub fn worktree(&self) -> Option<&std::path::Path> {
        self.target.worktree()
    }

    /// Whether the target follows the working tree, so external edits
    /// reload it.
    pub fn follows_worktree(&self) -> bool {
        self.diff_target.is_some()
            && matches!(
                self.target,
                DiffTarget::WorkingTree { .. }
                    | DiffTarget::CommitRange {
                        to_commit_id: None,
                        ..
                    }
            )
    }

    pub fn is_loading(&self) -> bool {
        self.pending.patch
            || self.pending.file_text
            || self.pending.image
            || self.pending.blame
            || self.pending.attributes
    }

    /// The blame the target's newer side reads, if it names one file.
    pub fn blame_source(&self) -> Option<(PathBuf, BlameSource)> {
        let path = self.target.file_path()?.to_path_buf();
        let source = match &self.target {
            DiffTarget::WorkingTree { area, .. } => BlameSource::WorkingTree(*area),
            DiffTarget::Commit { commit_id, .. } => {
                BlameSource::Revision(Some(commit_id.as_ref().to_string()))
            }
            DiffTarget::CommitRange {
                to_commit_id: Some(to),
                ..
            } => BlameSource::Revision(Some(to.as_ref().to_string())),
            DiffTarget::CommitRange {
                to_commit_id: None, ..
            } => BlameSource::WorkingTree(DiffArea::Unstaged),
        };
        Some((path, source))
    }
}

/// Outstanding parts of this generation, independent of retained content.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DiffSessionLoads {
    pub attributes: bool,
    pub patch: bool,
    pub file_text: bool,
    pub image: bool,
    pub blame: bool,
}

/// Where a hosted file list's changes come from.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChangeSource {
    /// Changes in this repository's index or working directory.
    #[non_exhaustive]
    Worktree {
        area: DiffArea,
        include_untracked: bool,
    },
    /// Changes in one of this repository's linked worktrees.
    #[non_exhaustive]
    LinkedWorktree {
        path: PathBuf,
        area: DiffArea,
        include_untracked: bool,
    },
    /// What a commit changed, against its first parent.
    Commit(CommitId),
    /// `from` to `to` (the working tree when `None`).
    #[non_exhaustive]
    Comparison {
        from: CommitId,
        to: Option<CommitId>,
        options: ComparisonOptions,
    },
}

impl ChangeSource {
    pub fn worktree(area: DiffArea, include_untracked: bool) -> Self {
        Self::Worktree {
            area,
            include_untracked,
        }
    }

    pub fn comparison(from: CommitId, to: Option<CommitId>, options: ComparisonOptions) -> Self {
        Self::Comparison { from, to, options }
    }

    /// The linked worktree at `path`; it must be one of the repository's.
    /// Its files open read-only: staging and saving act on the main checkout.
    pub fn linked_worktree(path: PathBuf, area: DiffArea, include_untracked: bool) -> Self {
        Self::LinkedWorktree {
            path: gitcomet_core::domain::normalize_worktree_path(&path),
            area,
            include_untracked,
        }
    }

    /// The linked worktree the changes are in; `None` is the repository's own.
    pub fn linked_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::LinkedWorktree { path, .. } => Some(path),
            Self::Worktree { .. } | Self::Commit(_) | Self::Comparison { .. } => None,
        }
    }

    /// The diff target for one listed change, carrying its rename source.
    pub fn target_for(&self, change: &CommitFileChange, base: Option<&CommitId>) -> DiffTarget {
        match self {
            Self::Worktree { area, .. } => DiffTarget::working_tree(change.path.clone(), *area),
            Self::LinkedWorktree { path, area, .. } => {
                DiffTarget::working_tree(change.path.clone(), *area).in_worktree(path.clone())
            }
            Self::Commit(id) => DiffTarget::commit_change(id.clone(), change),
            Self::Comparison { from, to, .. } => {
                DiffTarget::commit_range(base.unwrap_or(from).clone(), to.clone(), None)
                    .for_change(change)
            }
        }
    }

    /// Whether the working tree is one side, so external edits reload it.
    pub fn follows_worktree(&self) -> bool {
        matches!(
            self,
            Self::Comparison { to: None, .. } | Self::Worktree { .. } | Self::LinkedWorktree { .. }
        )
    }
}

/// A hosted file list's changes, loaded like a diff session.
#[derive(Clone, Debug)]
pub struct ChangeListSession {
    pub source: ChangeSource,
    pub generation: u64,
    pub rev: u64,
    pub files: Loadable<Shared<Vec<CommitFileChange>>>,
    /// The commit a comparison measured from (its merge base when asked).
    pub base: Option<CommitId>,
    pub(crate) refresh_queued: bool,
    pub(crate) loading: bool,
    pub(crate) cancellation: CancellationToken,
}

impl ChangeListSession {
    pub(crate) fn new(source: ChangeSource) -> Self {
        Self {
            source,
            generation: 0,
            rev: 0,
            files: Loadable::NotLoaded,
            base: None,
            refresh_queued: false,
            loading: false,
            cancellation: CancellationToken::new(),
        }
    }

    pub(crate) fn next_generation(&mut self) -> CancellationToken {
        self.refresh_queued = false;
        self.cancellation.cancel();
        self.cancellation = CancellationToken::new();
        self.generation = self.generation.wrapping_add(1);
        self.rev = self.rev.wrapping_add(1);
        self.cancellation.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }
}

/// What refresh scheduling needs from either session kind.
pub(crate) trait Refreshable {
    fn is_loading(&self) -> bool;
    /// One more load once the one in flight completes.
    fn queue_refresh(&mut self);
}

impl Refreshable for DiffSession {
    fn is_loading(&self) -> bool {
        DiffSession::is_loading(self)
    }

    fn queue_refresh(&mut self) {
        self.refresh_queued = true;
    }
}

impl Refreshable for ChangeListSession {
    fn is_loading(&self) -> bool {
        self.loading
    }

    fn queue_refresh(&mut self) {
        self.refresh_queued = true;
    }
}

/// Messages for diff sessions. `Loaded` comes from the store's own workers.
// Inline like `Msg`, so a load does not allocate; Windows paths tip it over.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum DiffSessionMsg {
    /// Opens `view` on `target`, or retargets it if already open.
    Open {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
        target: DiffTarget,
    },
    SetEncoding {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
        encoding: Option<TextEncoding>,
    },
    SetTextOverride {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
        path: PathBuf,
        value: TextOverride,
    },
    /// Changes the content presentation without touching History's navigation.
    SetContentMode {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
        preview: bool,
        edit: bool,
    },
    Clear {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
    },
    OpenEditor {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
        path: PathBuf,
    },
    ExitEditor {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
    },
    /// Reloads the current target (a new generation).
    Reload {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
    },
    LoadBlame {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
    },
    /// Cancels the session's work and forgets it.
    Close {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
    },
    /// Opens (or re-sources) change list `view`.
    OpenChanges {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
        source: ChangeSource,
    },
    CloseChanges {
        repo_id: RepoId,
        lifetime: u64,
        view: DiffViewId,
    },
    ChangesLoaded {
        repo_id: RepoId,
        view: DiffViewId,
        lifetime: u64,
        generation: u64,
        /// The commit measured from, and the files.
        result: Result<(Option<CommitId>, Vec<CommitFileChange>)>,
    },
    Loaded {
        repo_id: RepoId,
        view: DiffViewId,
        lifetime: u64,
        generation: u64,
        content: DiffSessionContent,
    },
}

#[derive(Debug)]
pub enum DiffSessionContent {
    Attributes(Result<TextAttributes>),
    Patch(Result<Diff>),
    /// Boxed: the text carries its sources and large-file sides, and this
    /// enum travels through every session message.
    FileText(Result<Option<Box<FileDiffText>>>),
    Image(Result<Option<FileDiffImage>>),
    Blame(Result<Vec<BlameLine>>),
}

/// What one session load reads.
#[derive(Clone, Debug)]
pub enum DiffSessionWork {
    Content {
        target: DiffTarget,
        encoding: Option<TextEncoding>,
        patch: bool,
        file_text: bool,
        image: bool,
    },
    Blame {
        path: PathBuf,
        source: BlameSource,
        /// The linked worktree the file is in; `None` is the repository's own.
        worktree: Option<PathBuf>,
    },
    Changes {
        source: ChangeSource,
    },
}

impl DiffSessionWork {
    /// The linked worktree the work reads; `None` is the repository's own.
    pub fn linked_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Content { target, .. } => target.worktree(),
            Self::Blame { worktree, .. } => worktree.as_deref(),
            Self::Changes { source } => source.linked_path(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiffSessionEffect {
    pub repo_id: RepoId,
    pub view: DiffViewId,
    pub lifetime: u64,
    pub generation: u64,
    pub work: DiffSessionWork,
    pub cancellation: CancellationToken,
}

impl DiffSessionEffect {
    /// The replies for work that could not run.
    pub fn failed(self, error: Error) -> Vec<DiffSessionMsg> {
        let Self {
            repo_id,
            view,
            lifetime,
            generation,
            work,
            ..
        } = self;
        let reply = |content| DiffSessionMsg::Loaded {
            repo_id,
            view,
            lifetime,
            generation,
            content,
        };
        match work {
            DiffSessionWork::Content {
                target,
                patch,
                file_text,
                image,
                ..
            } => {
                // `Error` is not `Clone`; each part gets the same message.
                let text = error.to_string();
                let error = || Error::new(ErrorKind::Backend(text.clone()));
                let mut replies = Vec::new();
                if target.file_path().is_some() {
                    replies.push(reply(DiffSessionContent::Attributes(Err(error()))));
                }
                if patch {
                    replies.push(reply(DiffSessionContent::Patch(Err(error()))));
                }
                if file_text {
                    replies.push(reply(DiffSessionContent::FileText(Err(error()))));
                }
                if image {
                    replies.push(reply(DiffSessionContent::Image(Err(error()))));
                }
                replies
            }
            DiffSessionWork::Blame { .. } => vec![reply(DiffSessionContent::Blame(Err(error)))],
            DiffSessionWork::Changes { .. } => vec![DiffSessionMsg::ChangesLoaded {
                repo_id,
                view,
                lifetime,
                generation,
                result: Err(error),
            }],
        }
    }
}
