use crate::repo::GixRepo;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::strip_windows_verbatim_prefix;
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository, Result};
use std::path::Path;
use std::sync::Arc;

pub struct GixBackend;

impl Default for GixBackend {
    fn default() -> Self {
        Self
    }
}

impl GixBackend {
    fn open_impl(
        &self,
        workdir: &Path,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Arc<dyn GitRepository>> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        let workdir = strip_windows_verbatim_prefix(
            workdir
                .canonicalize()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?,
        );
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        let repo = gix::open(&workdir).map_err(|e| match e {
            gix::open::Error::NotARepository { .. } => Error::new(ErrorKind::NotARepository),
            gix::open::Error::Io(io) => Error::new(ErrorKind::Io(io.kind())),
            e => Error::new(ErrorKind::Backend(format!("gix open: {e}"))),
        })?;
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        Ok(Arc::new(GixRepo::new(workdir, repo.into_sync())))
    }
}

impl GitBackend for GixBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, None)
    }

    fn open_cancellable(
        &self,
        workdir: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, Some(cancellation))
    }
}
