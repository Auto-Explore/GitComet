use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::git_dir_for_workdir;
use std::path::Path;

/// Open the repository backing the worktree at `workdir`.
///
/// This is the single point in the crate that turns a worktree path into an
/// open [`gix::Repository`]. It routes through [`git_dir_for_workdir`] so that a
/// worktree whose directory ends in `.git` (e.g. `/path/myrepo.git`) is opened
/// via its inner `.git` entry rather than being misread by gix as a bare git
/// directory.
///
/// gix 0.85 offers no open-option to force this: when a path ends in `.git` it
/// sets `looks_like_git_dir` and refuses to append `.git`, and `open_path_as_is`
/// only governs the opposite branch. Resolving the path ourselves is therefore
/// the intended fix — so keep every worktree open going through here.
///
/// The raw [`gix::Error`] is returned so callers can map it to their own
/// error type or treat "not a repository" as absence.
pub(crate) fn open_worktree_repo(workdir: &Path) -> gix::Result<gix::Repository> {
    gix::open(git_dir_for_workdir(workdir))
}

/// Translate a failed [`open_worktree_repo`] into the crate's error type.
///
/// `context` names the operation that was opening the repository and is only
/// used for the catch-all `Backend` message; the two cases callers act on —
/// "not a repository" and I/O — map to their own kinds so they stay
/// distinguishable. Callers that treat a missing repository as absence rather
/// than an error check [`gix::Error::is_not_found`] themselves instead.
pub(crate) fn map_open_error(error: gix::Error, context: &str) -> Error {
    // gix classifies "not a git repository" (and a missing path) as not-found.
    if error.is_not_found() {
        return Error::new(ErrorKind::NotARepository);
    }
    match error.classify().find_map(|class| class.io_kind()) {
        Some(kind) => Error::new(ErrorKind::Io(kind)),
        None => Error::new(ErrorKind::Backend(format!("{context}: {error}"))),
    }
}
