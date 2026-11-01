//! A hosted pane's API and renderer use the same prepared file generation.
//! The cell is evaluated by their background workers, never while rendering.

use super::file_diff::{
    FileDiffCacheError, FileDiffCacheRebuild, build_file_diff_cache_with_engine,
};
use super::*;
use gitcomet_core::domain::{Diff, DiffLineKind, FileDiffText};
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
        }
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
        self.prepared
            .get_or_init(|| {
                build_file_diff_cache_with_engine(
                    &self.file,
                    &self.workdir,
                    self.patch.as_deref(),
                    self.whitespace,
                    self.engine,
                    &self.cancellation,
                )
            })
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
