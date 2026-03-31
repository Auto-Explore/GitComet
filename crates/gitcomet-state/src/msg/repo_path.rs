use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RepoPath(Arc<PathBuf>);

impl RepoPath {
    pub fn new(path: PathBuf) -> Self {
        Self(Arc::new(path))
    }

    pub fn from_shared(path: Arc<PathBuf>) -> Self {
        Self(path)
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.0.as_ref().clone()
    }
}

impl AsRef<Path> for RepoPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl std::ops::Deref for RepoPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.as_path()
    }
}

impl From<PathBuf> for RepoPath {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

impl From<Arc<PathBuf>> for RepoPath {
    fn from(path: Arc<PathBuf>) -> Self {
        Self::from_shared(path)
    }
}

impl From<&Path> for RepoPath {
    fn from(path: &Path) -> Self {
        Self::new(path.to_path_buf())
    }
}

impl PartialEq<PathBuf> for RepoPath {
    fn eq(&self, other: &PathBuf) -> bool {
        self.as_path() == other.as_path()
    }
}

impl PartialEq<RepoPath> for PathBuf {
    fn eq(&self, other: &RepoPath) -> bool {
        self.as_path() == other.as_path()
    }
}

impl PartialEq<&Path> for RepoPath {
    fn eq(&self, other: &&Path) -> bool {
        self.as_path() == *other
    }
}

impl PartialEq<RepoPath> for &Path {
    fn eq(&self, other: &RepoPath) -> bool {
        *self == other.as_path()
    }
}
