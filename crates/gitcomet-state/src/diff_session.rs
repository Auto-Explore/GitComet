//! Independently owned diff sessions: the target, loads, encoding, and blame
//! of one hosted diff pane, apart from History's selected diff.
//!
//! A session belongs to one repository and is named by a [`DiffViewId`].
//! Work carries the repository's lifetime and the session's generation, so a
//! completion for a retargeted, closed, or reopened session is dropped, and
//! each session's cancellation token stops only its own loads.

use crate::model::{Loadable, RepoId, Shared};
use gitcomet_core::domain::{BlameSource, Diff, DiffArea, DiffTarget, FileDiffImage, FileDiffText};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::BlameLine;
use gitcomet_core::services::{CancellationToken, Result};
use gitcomet_core::text_format::TextEncoding;
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
    pub diff: Loadable<Shared<Diff>>,
    pub file_text: Loadable<Option<Shared<FileDiffText>>>,
    pub file_image: Loadable<Option<Shared<FileDiffImage>>>,
    /// Loaded on request ([`DiffSessionMsg::LoadBlame`]), for the target's
    /// newer side; cleared on retarget.
    pub blame: Loadable<Shared<Vec<BlameLine>>>,
    pub(crate) cancellation: CancellationToken,
}

impl DiffSession {
    pub(crate) fn new(target: DiffTarget) -> Self {
        Self {
            target,
            encoding: None,
            generation: 0,
            rev: 0,
            diff: Loadable::NotLoaded,
            file_text: Loadable::NotLoaded,
            file_image: Loadable::NotLoaded,
            blame: Loadable::NotLoaded,
            cancellation: CancellationToken::new(),
        }
    }

    /// Starts a new generation: cancels the old one's work and returns the
    /// new generation's token.
    pub(crate) fn next_generation(&mut self) -> CancellationToken {
        self.cancellation.cancel();
        self.cancellation = CancellationToken::new();
        self.generation = self.generation.wrapping_add(1);
        self.rev = self.rev.wrapping_add(1);
        self.cancellation.clone()
    }

    /// Whether the target follows the working tree, so external edits
    /// reload it.
    pub fn follows_worktree(&self) -> bool {
        matches!(
            self.target,
            DiffTarget::WorkingTree { .. }
                | DiffTarget::CommitRange {
                    to_commit_id: None,
                    ..
                }
        )
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

/// Messages for diff sessions. `Loaded` comes from the store's own workers.
#[derive(Debug)]
pub enum DiffSessionMsg {
    /// Opens `view` on `target`, or retargets it if already open.
    Open {
        repo_id: RepoId,
        view: DiffViewId,
        target: DiffTarget,
    },
    SetEncoding {
        repo_id: RepoId,
        view: DiffViewId,
        encoding: Option<TextEncoding>,
    },
    /// Reloads the current target (a new generation).
    Reload {
        repo_id: RepoId,
        view: DiffViewId,
    },
    LoadBlame {
        repo_id: RepoId,
        view: DiffViewId,
    },
    /// Cancels the session's work and forgets it.
    Close {
        repo_id: RepoId,
        view: DiffViewId,
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
    Patch(Result<Diff>),
    FileText(Result<Option<FileDiffText>>),
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
    },
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
                patch,
                file_text,
                image,
                ..
            } => {
                // `Error` is not `Clone`; each part gets the same message.
                let text = error.to_string();
                let error = || Error::new(ErrorKind::Backend(text.clone()));
                let mut replies = Vec::new();
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
        }
    }
}
