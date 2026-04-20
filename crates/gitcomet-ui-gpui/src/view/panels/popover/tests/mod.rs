pub(super) use super::*;
pub(super) use crate::test_support::lock_clipboard_test;
pub(super) use gitcomet_core::error::{Error, ErrorKind};
pub(super) use gitcomet_core::services::{GitBackend, GitRepository, Result};
pub(super) use std::path::Path;
pub(super) use std::sync::Arc;
pub(super) use std::time::SystemTime;

pub(super) struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

mod branch;
mod clone;
mod context_shortcuts;
mod file_actions;
mod layout;
mod picker;
mod recent_repo_picker;
mod refs;
mod stash;
mod status;
mod subtree;
