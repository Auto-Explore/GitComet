use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::git_dir_for_workdir;
use std::path::Path;

/// [`open_worktree_repo_with`] with gix's default permissions.
pub(crate) fn open_worktree_repo(workdir: &Path) -> gix::Result<gix::Repository> {
    open_worktree_repo_with(workdir, gix::open::Permissions::default())
}

/// Open the repository backing the worktree at `workdir`, reading config,
/// environment and attribute files only from the sources `permissions` allow.
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
pub(crate) fn open_worktree_repo_with(
    workdir: &Path,
    permissions: gix::open::Permissions,
) -> gix::Result<gix::Repository> {
    let options = gix::open::Options::default().permissions(permissions);
    let mut repo = gix::open_opts(git_dir_for_workdir(workdir), options)?;
    crate::refs::reload_conditional_includes(&mut repo)?;
    Ok(repo)
}

/// Translate a failed [`open_worktree_repo_with`] into the crate's error type.
///
/// The operation and path remain diagnostic facts; "not a repository" and I/O
/// retain their existing classifications. Callers that treat a missing
/// repository as absence check [`gix::Error::is_not_found`] themselves instead.
pub(crate) fn map_open_error(error: gix::Error, context: &str, path: &Path) -> Error {
    // gix classifies "not a git repository" (and a missing path) as not-found.
    if error.is_not_found() {
        return Error::new(ErrorKind::NotARepository)
            .with_source(error)
            .with_context(context, Some(path));
    }
    let io_kind = error.iter_errors().find_map(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind)
    });
    let kind = match io_kind {
        Some(kind) => ErrorKind::Io(kind),
        None => {
            // The top frame is generic ("configuration could not be loaded");
            // the innermost names the offending key or file.
            let mut message = error.to_string();
            if let Some(cause) = error.iter_errors().last().map(ToString::to_string)
                && !message.contains(&cause)
            {
                message.push_str(": ");
                message.push_str(&cause);
            }
            ErrorKind::Backend(message)
        }
    };
    Error::new(kind)
        .with_source(error)
        .with_context(context, Some(path))
}

#[cfg(test)]
mod tests {
    #[test]
    fn open_error_preserves_io_kinds_without_semantic_classifications() {
        for kind in [std::io::ErrorKind::Other, std::io::ErrorKind::InvalidData] {
            let error = gix::Error::from_error(std::io::Error::new(kind, "repository read failed"));
            let path = std::path::Path::new("repository");
            let mapped = super::map_open_error(error, "gix open", path);
            assert!(
                matches!(mapped.kind(), gitcomet_core::error::ErrorKind::Io(actual) if *actual == kind)
            );
            let source = std::error::Error::source(&mapped)
                .expect("repository open errors retain their cause");
            let source = source.downcast_ref::<gix::Error>().unwrap();
            let io = source
                .iter_errors()
                .find_map(|cause| cause.downcast_ref::<std::io::Error>())
                .unwrap();
            assert_eq!(io.to_string(), "repository read failed");
            assert_eq!(mapped.context().unwrap().operation(), "gix open");
            assert_eq!(mapped.context().unwrap().path(), Some(path));
        }
    }

    #[test]
    fn open_error_displays_io_cause_beneath_repository_configuration_context() {
        use gix::error::{ErrorExt, message};

        let io = std::io::Error::from_raw_os_error(13);
        let kind = io.kind();
        let io_message = io.to_string();
        let error = io.and_raise(message("Repository configuration could not be loaded"));
        let mapped = super::map_open_error(error, "gix open", std::path::Path::new("repository"));

        assert!(
            matches!(mapped.kind(), gitcomet_core::error::ErrorKind::Io(actual) if *actual == kind)
        );
        let source = std::error::Error::source(&mapped)
            .unwrap()
            .downcast_ref::<gix::Error>()
            .unwrap();
        assert!(source.iter_errors().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.raw_os_error() == Some(13))
        }));
        let rendered = mapped.to_string();
        assert!(rendered.contains("gix open (repository)"), "{rendered}");
        assert!(
            rendered.contains("Repository configuration could not be loaded"),
            "{rendered}"
        );
        assert!(rendered.contains(&io_message), "{rendered}");
    }

    #[test]
    fn open_error_names_the_innermost_cause() {
        let dir = tempfile::tempdir().unwrap();
        let git = crate::util::git_workdir_cmd_for(dir.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(git.success());
        let config = dir.path().join(".git/config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text = text.replace("repositoryformatversion = 0", "repositoryformatversion = 1");
        text.push_str("[extensions]\n\tobjectFormat = sha512\n");
        std::fs::write(&config, text).unwrap();

        let error = super::open_worktree_repo(dir.path()).unwrap_err();
        let message = super::map_open_error(error, "gix open", dir.path()).to_string();
        assert!(message.contains("objectFormat"), "{message}");
        assert!(message.contains("sha512"), "{message}");
    }
}
