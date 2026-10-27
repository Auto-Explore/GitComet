use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Error {
    data: Arc<ErrorData>,
}

#[derive(Clone, Debug)]
struct ErrorData {
    kind: ErrorKind,
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
    context: Option<ErrorContext>,
}

/// The operation and native path associated with a failure. Diagnostic context
/// does not change the classification callers use to choose recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorContext {
    operation: Arc<str>,
    path: Option<PathBuf>,
}

impl ErrorContext {
    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Error {
    pub fn new(kind: ErrorKind) -> Self {
        Self {
            data: Arc::new(ErrorData {
                kind,
                source: None,
                context: None,
            }),
        }
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.data.kind
    }

    /// Retains a concrete cause without making state snapshots clone it.
    pub fn with_source(
        mut self,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Arc::make_mut(&mut self.data).source = Some(Arc::from(source.into()));
        self
    }

    pub fn with_context(mut self, operation: impl Into<Arc<str>>, path: Option<&Path>) -> Self {
        Arc::make_mut(&mut self.data).context = Some(ErrorContext {
            operation: operation.into(),
            path: path.map(Path::to_path_buf),
        });
        self
    }

    pub fn context(&self) -> Option<&ErrorContext> {
        self.data.context.as_ref()
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Self::new(ErrorKind::Io(source.kind())).with_source(source)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.data.source.as_deref().map(|source| source as _)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(context) = self.context() {
            write!(f, "{}", context.operation())?;
            if let Some(path) = context.path() {
                write!(f, " ({})", path.display())?;
            }
            write!(f, ": ")?;
        }
        if matches!(self.kind(), ErrorKind::Io(_))
            && let Some(source) = std::error::Error::source(self)
        {
            fmt::Display::fmt(source, f)?;
            let mut cause = source.source();
            while let Some(source) = cause {
                write!(f, ": {source}")?;
                cause = source.source();
            }
            return Ok(());
        }
        self.kind().fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFailureId {
    CommandFailed,
    Timeout,
    BranchAlreadyExists,
    StashApplyConflict,
    /// `git apply --3way` left the file conflicted in the index and worktree.
    ApplyChangeConflict,
    /// An applied file change is staged, but committing it failed.
    ApplyChangeCommitFailed,
    UntrackedRestoreConflict,
    WorktreeWouldBeOverwritten,
    /// A filter or hook needs `git-lfs`, which is not on Git's PATH.
    LfsNotInstalled,
    /// A pull or merge left a merge or rebase in progress with unmerged paths.
    StoppedAtConflicts,
    /// Smudge could not download an LFS object (missing on the server, no remote).
    LfsObjectMissing,
    /// A push or edit was refused because another user holds an LFS lock.
    LfsLocked,
    /// The pre-push hook could not upload LFS objects.
    LfsUploadFailed,
}

/// Git refused for a local reason (working-tree changes, merge or branch
/// reconciliation). Any credential exchange earlier in the same command, such
/// as a pull's fetch, did its job and is not why the command failed.
pub fn git_output_reports_local_refusal(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    [
        "would be overwritten",
        "automatic merge failed",
        "cannot pull with rebase: you have unstaged changes",
        "cannot pull with rebase: your index contains uncommitted changes",
        "not possible to fast-forward",
        "need to specify how to reconcile divergent branches",
    ]
    .iter()
    .any(|refusal| lower.contains(refusal))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFailure {
    command: String,
    id: GitFailureId,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    detail: Option<String>,
    extras: Option<Box<GitFailureExtras>>,
}

/// Rarely present details, boxed together so [`Error`] stays small.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GitFailureExtras {
    apply_file_change_retry: Option<crate::domain::ApplyFileChangeRetry>,
    auth_outcome: Option<crate::auth::AuthOutcome>,
}

impl GitFailure {
    pub fn new(
        command: impl Into<String>,
        id: GitFailureId,
        exit_code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        detail: Option<String>,
    ) -> Self {
        Self {
            command: command.into(),
            id,
            exit_code,
            stdout,
            stderr,
            detail,
            extras: None,
        }
    }

    fn extras_mut(&mut self) -> &mut GitFailureExtras {
        self.extras.get_or_insert_default()
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn id(&self) -> GitFailureId {
        self.id
    }

    /// Authoritative authentication information, separate from Git's output.
    pub fn auth_outcome(&self) -> Option<&crate::auth::AuthOutcome> {
        self.extras.as_ref()?.auth_outcome.as_ref()
    }

    pub fn with_auth_outcome(mut self, outcome: crate::auth::AuthOutcome) -> Self {
        self.extras_mut().auth_outcome = Some(outcome);
        self
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub fn with_apply_file_change_retry(
        mut self,
        retry: crate::domain::ApplyFileChangeRetry,
    ) -> Self {
        self.extras_mut().apply_file_change_retry = Some(retry);
        self
    }

    pub fn apply_file_change_retry(&self) -> Option<&crate::domain::ApplyFileChangeRetry> {
        self.extras.as_ref()?.apply_file_change_retry.as_ref()
    }
}

impl fmt::Display for GitFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail() {
            Some(detail) if matches!(self.id, GitFailureId::Timeout) => {
                write!(f, "{} timed out {detail}", self.command)
            }
            Some(detail) => write!(f, "{} failed: {detail}", self.command),
            None if matches!(self.id, GitFailureId::Timeout) => {
                write!(f, "{} timed out", self.command)
            }
            None => write!(f, "{} failed", self.command),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum BareConversionRefusal {
    #[error("The repository is no longer bare. Refresh it before continuing.")]
    NotBare,
    #[error("The repository changed after the preview. Run the preview again.")]
    StalePreview,
    #[error("The selected folder does not own a supported Git directory layout: {path}")]
    UnsupportedLayout { path: PathBuf },
    #[error("Branch {branch} is checked out at {worktree}. Release that branch before continuing.")]
    BranchInUse { branch: String, worktree: PathBuf },
    #[error("Git has an active operation or lock at {path}. Finish it before converting.")]
    ActiveOperation { path: PathBuf },
    #[error("HEAD is missing, corrupt, or does not resolve to a commit.")]
    InvalidHead,
    #[error("The existing index cannot be safely preserved by this conversion.")]
    ExistingIndex,
    #[error("The checkout folder conflicts with core.worktree: {path}")]
    ConflictingWorktree { path: PathBuf },
    #[error("The bare directory contains an entry that cannot be relocated safely: {path}")]
    UnknownEntry { path: PathBuf },
    #[error(
        "The repository contains a path collision or symbolic link that cannot be relocated safely: {path}"
    )]
    PathCollision { path: PathBuf },
    #[error(
        "Configuration {key} depends on the current Git directory. Update it before converting."
    )]
    RelocationDependentConfig { key: String },
    #[error(
        "Linked worktree {path} is inaccessible or nested inside the repository. Move or repair it before converting."
    )]
    InaccessibleWorktree { path: PathBuf },
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ErrorKind {
    #[error("I/O error: {0}")]
    Io(std::io::ErrorKind),
    #[error("Not a repository")]
    NotARepository,
    #[error("Cancelled")]
    Cancelled,
    #[error("Unsupported: {0}")]
    Unsupported(&'static str),
    #[error("Requires a working tree; this repository is bare.")]
    BareUnsupported,
    #[error("The repository changed. Reopen it before continuing.")]
    RepositoryChanged,
    #[error("{0}")]
    BareConversionRefused(BareConversionRefusal),
    #[error(
        "Bare conversion recovery needs attention at {path}: {reason}. Review the remaining repository copies before continuing."
    )]
    BareConversionRecovery { path: PathBuf, reason: String },
    #[error("{0}")]
    Git(GitFailure),
    #[error("{0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::{Error, ErrorKind, GitFailure, GitFailureId};
    use std::error::Error as _;
    use std::path::Path;

    #[test]
    fn backend_error_kind_display_is_human_readable() {
        let kind = ErrorKind::Backend("message".to_string());
        assert_eq!(kind.to_string(), "message");
    }

    #[test]
    fn error_display_uses_error_kind_display() {
        let error = Error::new(ErrorKind::Backend("message".to_string()));
        assert_eq!(error.to_string(), "message");
        assert!(error.source().is_none());
        assert!(error.context().is_none());
    }

    #[test]
    fn io_error_clones_share_the_original_cause_and_keep_context() {
        let source = std::io::Error::from_raw_os_error(13);
        let kind = source.kind();
        let message = source.to_string();
        let error = Error::from(source).with_context("read index", Some(Path::new("repo")));
        let cloned = error.clone();

        assert!(matches!(cloned.kind(), ErrorKind::Io(actual) if *actual == kind));
        assert!(std::ptr::eq(
            error.source().unwrap(),
            cloned.source().unwrap()
        ));
        drop(error);

        let source = cloned
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(source.raw_os_error(), Some(13));
        let context = cloned.context().unwrap();
        assert_eq!(context.operation(), "read index");
        assert_eq!(context.path(), Some(Path::new("repo")));
        assert_eq!(cloned.to_string(), format!("read index (repo): {message}"));
    }

    #[test]
    fn adding_context_to_a_clone_does_not_change_the_retained_error() {
        let original = Error::from(std::io::Error::other("read failed"))
            .with_context("read file", Some(Path::new("file")));
        let contextual = original.clone().with_context("load status", None);

        assert_eq!(original.to_string(), "read file (file): read failed");
        assert_eq!(contextual.to_string(), "load status: read failed");
        assert!(std::ptr::eq(
            original.source().unwrap(),
            contextual.source().unwrap()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn error_context_preserves_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"repo-\xff".to_vec()));
        let error = Error::from(std::io::Error::other("read failed"))
            .with_context("read file", Some(&path));

        assert_eq!(error.context().unwrap().path(), Some(path.as_path()));
    }

    #[test]
    fn git_failure_display_preserves_command_and_detail() {
        let failure = GitFailure::new(
            "git fetch --all",
            GitFailureId::CommandFailed,
            Some(128),
            b"out".to_vec(),
            b"err".to_vec(),
            Some("fatal: network down".to_string()),
        );
        assert_eq!(
            failure.to_string(),
            "git fetch --all failed: fatal: network down"
        );
        assert_eq!(failure.command(), "git fetch --all");
        assert_eq!(failure.id(), GitFailureId::CommandFailed);
        assert_eq!(failure.exit_code(), Some(128));
        assert_eq!(failure.stdout(), b"out");
        assert_eq!(failure.stderr(), b"err");
        assert_eq!(failure.detail(), Some("fatal: network down"));
    }

    #[test]
    fn git_error_kind_display_uses_structured_message() {
        let kind = ErrorKind::Git(GitFailure::new(
            "git push",
            GitFailureId::Timeout,
            None,
            Vec::new(),
            Vec::new(),
            Some("after 300 seconds".to_string()),
        ));
        assert_eq!(kind.to_string(), "git push timed out after 300 seconds");
    }
}
