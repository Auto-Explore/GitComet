//! A hosted pane's API and renderer use the same prepared file generation.
//! The cell is evaluated by their background workers, never while rendering.

use super::file_diff::{
    FileDiffCacheError, FileDiffCacheRebuild, build_file_diff_cache_with_engine,
};
use super::*;
use gitcomet_core::domain::{Diff, DiffLineKind, FileDiffText};
use gitcomet_core::services::{CancellationToken, GitRepository};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(in crate::view) struct SharedFileDiffCache {
    pub(in crate::view) file: Arc<FileDiffText>,
    patch: Option<Arc<Diff>>,
    workdir: PathBuf,
    prepared: OnceLock<Result<FileDiffCacheRebuild, FileDiffCacheError>>,
    pub(in crate::view) engine: DiffEngine,
    pub(in crate::view) whitespace: DiffWhitespaceMode,
    cancellation: gitcomet_core::services::CancellationToken,
    prefetch: Option<Arc<PrefetchedFileDiff>>,
}

fn alignment_patch(patch: Option<&Arc<Diff>>) -> Option<&Arc<Diff>> {
    patch.filter(|patch| {
        patch
            .lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Hunk)
    })
}

impl SharedFileDiffCache {
    #[cfg(test)]
    pub(in crate::view) fn new(
        file: Arc<FileDiffText>,
        patch: Option<&Arc<Diff>>,
        workdir: PathBuf,
    ) -> Self {
        Self::with_engine(
            file,
            patch,
            workdir,
            DiffEngine::LineByLine,
            DiffWhitespaceMode::Show,
        )
    }

    pub(in crate::view) fn with_engine(
        file: Arc<FileDiffText>,
        patch: Option<&Arc<Diff>>,
        workdir: PathBuf,
        engine: DiffEngine,
        whitespace: DiffWhitespaceMode,
    ) -> Self {
        Self {
            file,
            patch: alignment_patch(patch).cloned(),
            workdir,
            prepared: OnceLock::new(),
            engine,
            whitespace,
            cancellation: gitcomet_core::services::CancellationToken::new(),
            prefetch: None,
        }
    }

    pub(in crate::view) fn with_prefetch(
        mut self,
        prefetch: Option<Arc<PrefetchedFileDiff>>,
    ) -> Self {
        self.prefetch = prefetch;
        self
    }

    pub(in crate::view) fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub(in crate::view) fn matches_engine(
        &self,
        file: &Arc<FileDiffText>,
        patch: Option<&Arc<Diff>>,
        workdir: &Path,
        engine: DiffEngine,
        whitespace: DiffWhitespaceMode,
    ) -> bool {
        self.engine == engine
            && self.whitespace == whitespace
            && !self.cancellation.is_cancelled()
            && self.matches(file, patch, workdir)
    }

    pub(in crate::view) fn matches(
        &self,
        file: &Arc<FileDiffText>,
        patch: Option<&Arc<Diff>>,
        workdir: &Path,
    ) -> bool {
        Arc::ptr_eq(&self.file, file)
            && self.workdir == workdir
            && match (&self.patch, alignment_patch(patch)) {
                (None, None) => true,
                (Some(cached), Some(patch)) => Arc::ptr_eq(cached, patch),
                _ => false,
            }
    }

    pub(in crate::view) fn build(&self) -> Result<&FileDiffCacheRebuild, &FileDiffCacheError> {
        self.build_for(gitcomet_difftastic::Request::Visible(&self.cancellation))
    }

    fn build_for(
        &self,
        request: gitcomet_difftastic::Request<'_>,
    ) -> Result<&FileDiffCacheRebuild, &FileDiffCacheError> {
        self.prepared
            .get_or_init(|| {
                if self.cancellation.is_cancelled() {
                    return Err(FileDiffCacheError::Cancelled);
                }
                if let Some(prepared) = self.prefetch.as_ref().and_then(|hint| hint.rows_for(self))
                {
                    return Ok(prepared.clone());
                }
                build_file_diff_cache_with_engine(
                    &self.file,
                    &self.workdir,
                    self.patch.as_deref(),
                    self.whitespace,
                    self.engine,
                    request,
                )
            })
            .as_ref()
    }
}

/// One upcoming committed file. The foreground still opens it normally, so
/// encoding, attributes and locally resolved large-file contents are checked
/// against the inputs the renderer actually received.
pub(in crate::view) struct PrefetchedFileDiff {
    target: DiffTarget,
    engine: DiffEngine,
    whitespace: DiffWhitespaceMode,
    cancellation: CancellationToken,
    cache: OnceLock<SharedFileDiffCache>,
}

impl PrefetchedFileDiff {
    pub(in crate::view) fn new(
        target: DiffTarget,
        engine: DiffEngine,
        whitespace: DiffWhitespaceMode,
    ) -> Option<Self> {
        if engine != DiffEngine::Difftastic {
            return None;
        }
        let fixed = |id: &gitcomet_core::domain::CommitId| {
            matches!(id.0.len(), 40 | 64)
                && id.0.bytes().all(|b| b.is_ascii_hexdigit())
                && !id.is_uncommitted()
        };
        let immutable = match &target {
            DiffTarget::Commit { commit_id, .. } => fixed(commit_id),
            DiffTarget::CommitRange {
                from_commit_id,
                to_commit_id: Some(to),
                path: Some(_),
                ..
            } => fixed(from_commit_id) && fixed(to),
            _ => false,
        };
        immutable.then(|| Self {
            target,
            engine,
            whitespace,
            cancellation: CancellationToken::new(),
            cache: OnceLock::new(),
        })
    }

    pub(in crate::view) fn targets(&self, target: &DiffTarget) -> bool {
        self.target == *target && self.target.old_file_path() == target.old_file_path()
    }

    pub(in crate::view) fn matches(
        &self,
        target: &DiffTarget,
        engine: DiffEngine,
        whitespace: DiffWhitespaceMode,
    ) -> bool {
        self.targets(target) && self.engine == engine && self.whitespace == whitespace
    }

    pub(in crate::view) fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub(in crate::view) fn prepare(&self, repo: &dyn GitRepository) {
        let cancel = &self.cancellation;
        if cancel.is_cancelled() {
            return;
        }
        let Ok(Some(file)) =
            repo.prefetch_file_text(&self.target, gitcomet_difftastic::BYTE_LIMIT as u64, cancel)
        else {
            return;
        };
        if cancel.is_cancelled()
            || file_diff_source_len(file.old_source.as_ref(), file.old.as_ref())
                > gitcomet_difftastic::BYTE_LIMIT
            || file_diff_source_len(file.new_source.as_ref(), file.new.as_ref())
                > gitcomet_difftastic::BYTE_LIMIT
        {
            return;
        }
        let Ok(patch) = repo.diff_parsed(&self.target, None, cancel) else {
            return;
        };
        if cancel.is_cancelled() {
            return;
        }
        let cache = self.cache.get_or_init(|| {
            let mut cache = SharedFileDiffCache::with_engine(
                Arc::new(file),
                Some(&Arc::new(patch)),
                repo.spec().workdir.clone(),
                self.engine,
                self.whitespace,
            );
            cache.cancellation = cancel.clone();
            cache
        });
        let _ = cache.build_for(gitcomet_difftastic::Request::Prefetch(cancel));
    }

    fn rows_for(&self, current: &SharedFileDiffCache) -> Option<&FileDiffCacheRebuild> {
        let Some(cache) = self.cache.get() else {
            self.cancel();
            return None;
        };
        if self.cancellation.is_cancelled()
            || cache.engine != current.engine
            || cache.whitespace != current.whitespace
            || cache.workdir != current.workdir
            || cache.file != current.file
            || cache.patch != current.patch
        {
            self.cancel();
            return None;
        }
        cache.build().ok()
    }

    #[cfg(test)]
    pub(in crate::view) fn prepared_rows(&self) -> Option<&Arc<super::PagedFileDiffRows>> {
        Some(
            &self
                .cache
                .get()?
                .prepared
                .get()?
                .as_ref()
                .ok()?
                .row_provider,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_requires_a_committed_file_and_the_same_rename_source_and_options() {
        let id = gitcomet_core::domain::CommitId("a".repeat(40).into());
        let commit = DiffTarget::commit(id.clone(), "new.rs".into());
        let renamed = commit.clone().with_old_path(Some("old.rs".into()));
        let hint = |target| {
            PrefetchedFileDiff::new(target, DiffEngine::Difftastic, DiffWhitespaceMode::Show)
        };
        let next = hint(renamed.clone()).unwrap();
        assert!(next.matches(&renamed, DiffEngine::Difftastic, DiffWhitespaceMode::Show));
        assert!(!next.targets(&commit));
        assert!(!next.matches(&renamed, DiffEngine::LineByLine, DiffWhitespaceMode::Show));
        assert!(
            hint(DiffTarget::working_tree(
                "new.rs".into(),
                DiffArea::Unstaged
            ))
            .is_none()
        );
        assert!(
            hint(DiffTarget::commit_range(
                id.clone(),
                None,
                Some("new.rs".into())
            ))
            .is_none()
        );
        assert!(hint(DiffTarget::commit_range(id.clone(), Some(id), None)).is_none());
        assert!(
            hint(DiffTarget::commit(
                gitcomet_core::domain::CommitId("HEAD".into()),
                "new.rs".into()
            ))
            .is_none()
        );
    }

    #[test]
    fn changed_inputs_cancel_speculative_rows_and_build_the_current_file() {
        let target = DiffTarget::commit(
            gitcomet_core::domain::CommitId("b".repeat(40).into()),
            "file.rs".into(),
        );
        let hint = Arc::new(
            PrefetchedFileDiff::new(target, DiffEngine::Difftastic, DiffWhitespaceMode::Show)
                .unwrap(),
        );
        let cache = |text: &str| {
            SharedFileDiffCache::with_engine(
                Arc::new(FileDiffText::new_shared(
                    "file.rs".into(),
                    None,
                    Some(Arc::from(text)),
                )),
                None,
                "/repo".into(),
                DiffEngine::Difftastic,
                DiffWhitespaceMode::Show,
            )
        };
        let prior = hint.cache.get_or_init(|| cache("fn before() {}\n"));
        let current = cache("fn after() {}\n").with_prefetch(Some(hint.clone()));
        let shown = current.build().unwrap();
        assert!(hint.cancellation.is_cancelled());
        assert!(prior.prepared.get().is_none());
        assert_eq!(
            shown.row_provider.source_byte_lengths().1,
            "fn after() {}\n".len()
        );
    }

    #[test]
    fn workers_share_prepared_rows_for_exactly_one_file_generation() {
        let file = Arc::new(FileDiffText::new_shared(
            "file.rs".into(),
            Some(Arc::from("old\n")),
            Some(Arc::from("new\n")),
        ));
        let cache = SharedFileDiffCache::new(file.clone(), None, "/repo".into());
        std::thread::scope(|scope| {
            let first = scope.spawn(|| cache.build().unwrap().row_provider.clone());
            let second = scope.spawn(|| cache.build().unwrap().row_provider.clone());
            assert!(Arc::ptr_eq(&first.join().unwrap(), &second.join().unwrap()));
        });
        assert!(cache.matches(&file, None, Path::new("/repo")));
        // Even an equal payload can name a different on-disk generation.
        assert!(!cache.matches(&Arc::new((*file).clone()), None, Path::new("/repo")));
        assert!(!cache.matches(&file, None, Path::new("/other")));

        let target = DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged);
        let empty = Arc::new(Diff::from_unified(target.clone(), ""));
        assert!(cache.matches(&file, Some(&empty), Path::new("/repo")));
        let patch = Arc::new(Diff::from_unified(target, "@@ -1 +1 @@\n-old\n+new\n"));
        assert!(!cache.matches(&file, Some(&patch), Path::new("/repo")));
        let patched = SharedFileDiffCache::new(file.clone(), Some(&patch), "/repo".into());
        assert!(patched.matches(&file, Some(&patch), Path::new("/repo")));
        assert!(!patched.matches(&file, None, Path::new("/repo")));
    }
}
