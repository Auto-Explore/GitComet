use gpui::ExternalPaths;
use std::path::PathBuf;

/// The directories of an external drop, in drop order. Files, missing paths and
/// special files are skipped: each dropped item is judged on its own.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ClassifiedExternalPaths {
    directories: Vec<PathBuf>,
}

impl ClassifiedExternalPaths {
    pub(super) fn directories(&self) -> &[PathBuf] {
        &self.directories
    }

    pub(super) fn has_directory(&self) -> bool {
        !self.directories.is_empty()
    }
}

pub(super) fn classify_external_paths_blocking(paths: &ExternalPaths) -> ClassifiedExternalPaths {
    ClassifiedExternalPaths {
        directories: paths
            .paths()
            .iter()
            // `metadata` follows symlinks, so a link to a folder counts.
            .filter(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn external_paths(paths: impl IntoIterator<Item = PathBuf>) -> ExternalPaths {
        ExternalPaths(paths.into_iter().collect())
    }

    fn directories(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
        classify_external_paths_blocking(&external_paths(paths))
            .directories()
            .to_vec()
    }

    #[test]
    fn classifies_one_directory() {
        let temp = tempfile::tempdir().expect("create temp directory");
        assert_eq!(
            directories([temp.path().to_path_buf()]),
            vec![temp.path().to_path_buf()]
        );
    }

    #[test]
    fn keeps_every_directory_in_drop_order_and_skips_other_items() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let dir_a = temp.path().join("a");
        let dir_b = temp.path().join("b");
        std::fs::create_dir(&dir_a).expect("create first directory");
        std::fs::create_dir(&dir_b).expect("create second directory");
        let file = tempfile::NamedTempFile::new().expect("create temp file");
        let missing = temp.path().join("missing");

        assert_eq!(
            directories([
                dir_b.clone(),
                file.path().to_path_buf(),
                missing,
                dir_a.clone(),
            ]),
            vec![dir_b, dir_a]
        );
    }

    #[test]
    fn empty_and_file_only_payloads_have_no_directory() {
        let file = tempfile::NamedTempFile::new().expect("create temp file");
        assert!(!classify_external_paths_blocking(&external_paths([])).has_directory());
        assert!(
            !classify_external_paths_blocking(&external_paths([file.path().to_path_buf()]))
                .has_directory()
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

        assert_eq!(directories([link.clone()]), vec![link]);
    }
}
