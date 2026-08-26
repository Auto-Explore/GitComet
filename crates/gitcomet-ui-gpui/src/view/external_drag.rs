use gpui::ExternalPaths;
use std::path::PathBuf;

/// The subset of an operating-system path drag that GitComet understands.
///
/// Files are identified separately as groundwork for future drop targets, but
/// the repository bar currently accepts only one existing directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ClassifiedExternalPaths {
    SingleDirectory(PathBuf),
    SingleFile(PathBuf),
    Unsupported,
}

impl ClassifiedExternalPaths {
    pub(super) fn directory(&self) -> Option<&PathBuf> {
        match self {
            Self::SingleDirectory(path) => Some(path),
            Self::SingleFile(_) | Self::Unsupported => None,
        }
    }
}

/// Performs filesystem I/O and must only be called from GPUI's background
/// executor in production code.
pub(super) fn classify_external_paths_blocking(paths: &ExternalPaths) -> ClassifiedExternalPaths {
    let [path] = paths.paths() else {
        return ClassifiedExternalPaths::Unsupported;
    };
    let Ok(metadata) = std::fs::metadata(path) else {
        return ClassifiedExternalPaths::Unsupported;
    };

    if metadata.is_dir() {
        ClassifiedExternalPaths::SingleDirectory(path.clone())
    } else if metadata.is_file() {
        ClassifiedExternalPaths::SingleFile(path.clone())
    } else {
        ClassifiedExternalPaths::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn external_paths(paths: impl IntoIterator<Item = PathBuf>) -> ExternalPaths {
        ExternalPaths(paths.into_iter().collect())
    }

    #[test]
    fn classifies_one_directory() {
        let temp = tempfile::tempdir().expect("create temp directory");
        assert_eq!(
            classify_external_paths_blocking(&external_paths([temp.path().to_path_buf()])),
            ClassifiedExternalPaths::SingleDirectory(temp.path().to_path_buf())
        );
    }

    #[test]
    fn classifies_one_file_for_future_drop_targets() {
        let temp = tempfile::NamedTempFile::new().expect("create temp file");
        assert_eq!(
            classify_external_paths_blocking(&external_paths([temp.path().to_path_buf()])),
            ClassifiedExternalPaths::SingleFile(temp.path().to_path_buf())
        );
    }

    #[test]
    fn rejects_empty_multiple_and_missing_payloads() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let file = tempfile::NamedTempFile::new().expect("create temp file");
        let missing = temp.path().join("missing");

        assert_eq!(
            classify_external_paths_blocking(&external_paths([])),
            ClassifiedExternalPaths::Unsupported
        );
        assert_eq!(
            classify_external_paths_blocking(&external_paths([
                temp.path().to_path_buf(),
                file.path().to_path_buf(),
            ])),
            ClassifiedExternalPaths::Unsupported
        );
        assert_eq!(
            classify_external_paths_blocking(&external_paths([missing])),
            ClassifiedExternalPaths::Unsupported
        );
    }

    #[cfg(unix)]
    #[test]
    fn follows_a_symlink_to_a_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("create temp directory");
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::create_dir(&target).expect("create target directory");
        symlink(&target, &link).expect("create directory symlink");

        assert_eq!(
            classify_external_paths_blocking(&external_paths([link.clone()])),
            ClassifiedExternalPaths::SingleDirectory(link)
        );
    }
}
