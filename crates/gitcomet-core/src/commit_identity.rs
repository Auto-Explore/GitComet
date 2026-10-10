//! An identity for one Git operation, without changing repository or process
//! configuration. Backends must refuse unsupported overrides rather than
//! silently creating a commit as somebody else.
use crate::error::{Error, ErrorKind};
use crate::services::Result;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
}

impl CommitIdentity {
    pub fn validate(&self) -> Result<()> {
        if [&self.name, &self.email].iter().any(|value| {
            value.trim().is_empty()
                || value
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '<' | '>'))
        }) {
            return Err(Error::new(ErrorKind::Backend(
                "invalid operation commit identity".into(),
            )));
        }
        Ok(())
    }
    /// Sets only this child command's environment; never the parent process.
    pub fn apply(&self, command: &mut std::process::Command) -> Result<()> {
        self.validate()?;
        command
            .env("GIT_AUTHOR_NAME", &self.name)
            .env("GIT_AUTHOR_EMAIL", &self.email)
            .env("GIT_COMMITTER_NAME", &self.name)
            .env("GIT_COMMITTER_EMAIL", &self.email);
        Ok(())
    }
}

/// Called outside the store's state lock before scheduling a commit. Each
/// window has its own resolver. Return None to use Git's own configuration.
pub type CommitIdentityResolver = Arc<dyn Fn(&Path) -> Option<CommitIdentity> + Send + Sync>;
