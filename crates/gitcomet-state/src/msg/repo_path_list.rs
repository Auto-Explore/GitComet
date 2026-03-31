use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoPathList(Arc<Vec<PathBuf>>);

impl RepoPathList {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self(Arc::new(paths))
    }

    pub fn from_shared(paths: Arc<Vec<PathBuf>>) -> Self {
        Self(paths)
    }

    pub fn as_slice(&self) -> &[PathBuf] {
        self.0.as_slice()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<PathBuf>> for RepoPathList {
    fn from(paths: Vec<PathBuf>) -> Self {
        Self::new(paths)
    }
}

impl From<Arc<Vec<PathBuf>>> for RepoPathList {
    fn from(paths: Arc<Vec<PathBuf>>) -> Self {
        Self::from_shared(paths)
    }
}
