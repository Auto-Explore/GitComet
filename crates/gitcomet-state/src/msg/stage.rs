use super::{Effect, RepoPathList};
use crate::model::RepoId;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum StageSelection {
    Path(PathBuf),
    Paths(RepoPathList),
}

#[derive(Clone, Debug)]
pub enum StageGuard {
    Scanned(RepoPathList),
    CurrentStatus,
}

#[derive(Clone, Debug)]
pub struct StageRequest {
    pub selection: StageSelection,
    pub guard: Option<StageGuard>,
}

impl StageRequest {
    pub(crate) fn effect(self, repo_id: RepoId) -> Effect {
        Effect::Stage {
            repo_id,
            request: self,
        }
    }

    pub fn path(path: PathBuf) -> Self {
        Self {
            selection: StageSelection::Path(path),
            guard: None,
        }
    }
    pub fn as_slice(&self) -> &[PathBuf] {
        self.selection.paths()
    }
}

impl From<RepoPathList> for StageRequest {
    fn from(paths: RepoPathList) -> Self {
        Self {
            selection: StageSelection::Paths(paths),
            guard: None,
        }
    }
}
impl From<Vec<PathBuf>> for StageRequest {
    fn from(paths: Vec<PathBuf>) -> Self {
        Self::from(RepoPathList::new(paths))
    }
}
impl From<Arc<Vec<PathBuf>>> for StageRequest {
    fn from(paths: Arc<Vec<PathBuf>>) -> Self {
        Self::from(RepoPathList::from_shared(paths))
    }
}

pub fn conflicted_stage_paths(
    entries: &[gitcomet_core::domain::FileStatus],
    requested: &[PathBuf],
) -> Vec<PathBuf> {
    if !entries.iter().any(|entry| entry.conflict.is_some()) {
        return Vec::new();
    }
    let requested: rustc_hash::FxHashSet<_> = requested.iter().collect();
    entries
        .iter()
        .filter(|entry| {
            entry.conflict.is_some() && (requested.is_empty() || requested.contains(&entry.path))
        })
        .map(|entry| entry.path.clone())
        .collect()
}

impl StageSelection {
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Path(path) => std::slice::from_ref(path),
            Self::Paths(paths) => paths.as_slice(),
        }
    }

    pub(crate) fn action(&self) -> super::RepoActionKind {
        match self {
            Self::Path(_) => super::RepoActionKind::StagePath,
            Self::Paths(_) => super::RepoActionKind::StagePaths,
        }
    }
}
