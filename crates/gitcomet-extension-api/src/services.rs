//! Revision-pinned reads and syntax services without importing the UI host.

use crate::{HostError, RepositoryHandle, StateSubscription, WindowHost};
use gitcomet_core::services::GitRepository;
use gitcomet_state::model::{AppState, RepoId};
use gitcomet_ui_kit::gpui::{App, HighlightStyle, WindowId};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

type ReadRepository = dyn Fn(RepoId, u64) -> Option<Arc<dyn GitRepository>> + Send + Sync;

/// A weak, thread-safe repository reader. Call `repository` on a worker thread:
/// it synchronizes with the store worker before returning the backend handle.
#[derive(Clone)]
pub struct RepositoryReader {
    window: WindowId,
    read: Arc<ReadRepository>,
}

impl RepositoryReader {
    #[doc(hidden)]
    pub fn new(
        window: WindowId,
        read: impl Fn(RepoId, u64) -> Option<Arc<dyn GitRepository>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            window,
            read: Arc::new(read),
        }
    }

    pub fn repository(
        &self,
        repository: &RepositoryHandle,
    ) -> Result<Arc<dyn GitRepository>, HostError> {
        if repository.window() != self.window {
            return Err(HostError::Unsupported);
        }
        (self.read)(repository.repo_id(), repository.lifetime()).ok_or(HostError::RepositoryClosed)
    }
}

/// A window's read-only state subscriptions. Select a small fingerprint so
/// unrelated store changes do not wake the owning view.
#[derive(Clone)]
pub struct StoreView(pub(crate) WindowHost);

impl StoreView {
    pub fn state(&self, cx: &App) -> Result<Arc<AppState>, HostError> {
        self.0.state(cx)
    }

    pub fn observe_fingerprint<K: PartialEq + 'static>(
        &self,
        fingerprint: impl Fn(&AppState) -> K + 'static,
        changed: impl Fn(&WindowHost, &K, &mut App) + 'static,
        cx: &App,
    ) -> Result<StateSubscription, HostError> {
        self.0.observe_selected(fingerprint, changed, cx)
    }
}

/// Syntax styling in the window's current theme.
#[derive(Clone)]
pub struct SyntaxService(pub(crate) WindowHost);

impl SyntaxService {
    pub fn highlight_line(
        &self,
        path: &Path,
        text: &str,
        cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.0.highlight_line(path, text, cx)
    }
}
