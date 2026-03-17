use std::fmt;

#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFailureId {
    CommandFailed,
    Timeout,
    StashApplyConflict,
    UntrackedRestoreConflict,
    WorktreeWouldBeOverwritten,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFailure {
    command: String,
    id: GitFailureId,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    detail: Option<String>,
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
        }
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn id(&self) -> GitFailureId {
        self.id
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

#[derive(Debug, thiserror::Error)]
pub enum ErrorKind {
    #[error("I/O error: {0}")]
    Io(std::io::ErrorKind),
    #[error("Not a repository")]
    NotARepository,
    #[error("Unsupported: {0}")]
    Unsupported(&'static str),
    #[error("{0}")]
    Git(GitFailure),
    #[error("{0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::{Error, ErrorKind, GitFailure, GitFailureId};

    #[test]
    fn backend_error_kind_display_is_human_readable() {
        let kind = ErrorKind::Backend("message".to_string());
        assert_eq!(kind.to_string(), "message");
    }

    #[test]
    fn error_display_uses_error_kind_display() {
        let error = Error::new(ErrorKind::Backend("message".to_string()));
        assert_eq!(error.to_string(), "message");
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
