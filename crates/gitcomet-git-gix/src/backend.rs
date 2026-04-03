use crate::repo::GixRepo;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::strip_windows_verbatim_prefix;
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use std::path::Path;
use std::sync::Arc;

pub struct GixBackend;

impl Default for GixBackend {
    fn default() -> Self {
        Self
    }
}

impl GitBackend for GixBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        let workdir = strip_windows_verbatim_prefix(
            workdir
                .canonicalize()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?,
        );

        let repo = gix::open(&workdir).map_err(|e| match e {
            gix::open::Error::NotARepository { .. } => Error::new(ErrorKind::NotARepository),
            gix::open::Error::Io(io) => Error::new(ErrorKind::Io(io.kind())),
            e => Error::new(ErrorKind::Backend(format!("gix open: {e}"))),
        })?;

        Ok(Arc::new(GixRepo::new(workdir, repo.into_sync())))
    }
}
