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
/// The raw [`gix::open::Error`] is returned so callers can map it to their own
/// error type or treat "not a repository" as absence.
// This is a thin forwarder over `gix::open`, which itself returns the large
// `gix::open::Error` by value; boxing here would only complicate every caller's
// match without a real payoff.
#[allow(clippy::result_large_err)]
pub(crate) fn open_worktree_repo(
    workdir: &Path,
) -> std::result::Result<gix::Repository, gix::open::Error> {
    gix::open(git_dir_for_workdir(workdir))
}
