//! Where a Browse dialog opens: at the path its text field names, when that
//! exists, else wherever the platform picks.

use std::path::{Path, PathBuf};

/// For folder pickers: the typed folder, if it exists. A relative path counts
/// from `base`; with no base it names nothing.
pub(crate) fn existing_folder(typed: &str, base: Option<&Path>) -> Option<PathBuf> {
    let typed = Path::new(typed.trim());
    if typed.as_os_str().is_empty() {
        return None;
    }
    let path = if typed.is_absolute() {
        typed.to_path_buf()
    } else {
        base?.join(typed)
    };
    path.is_dir().then_some(path)
}

/// For file pickers: the folder holding the typed entry, if the entry exists.
/// A macOS `.app` bundle opens among its siblings, not inside.
pub(crate) fn folder_of_existing_entry(typed: &str) -> Option<PathBuf> {
    let typed = Path::new(typed.trim());
    if !typed.is_absolute() || !typed.exists() {
        return None;
    }
    typed.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().to_path_buf();
        std::fs::create_dir(root.join("sub")).expect("sub folder");
        std::fs::write(root.join("tool"), "").expect("file");
        (dir, root)
    }

    fn text(path: &Path) -> String {
        path.display().to_string()
    }

    #[test]
    fn a_folder_picker_opens_at_an_existing_typed_folder() {
        let (_dir, root) = fixture();
        assert_eq!(existing_folder(&text(&root), None), Some(root.clone()));
        assert_eq!(
            existing_folder(&format!("  {}  ", text(&root.join("sub"))), None),
            Some(root.join("sub")),
            "surrounding spaces are not part of the path"
        );
    }

    #[test]
    fn a_folder_picker_keeps_the_default_for_anything_else() {
        let (_dir, root) = fixture();
        assert_eq!(existing_folder(&text(&root.join("missing")), None), None);
        assert_eq!(
            existing_folder(&text(&root.join("tool")), None),
            None,
            "a file is no folder to open"
        );
        assert_eq!(existing_folder("", Some(&root)), None);
        assert_eq!(existing_folder("   ", Some(&root)), None);
    }

    #[test]
    fn a_relative_folder_counts_from_the_base() {
        let (_dir, root) = fixture();
        assert_eq!(existing_folder("sub", Some(&root)), Some(root.join("sub")));
        assert_eq!(existing_folder("sub", None), None);
    }

    #[test]
    fn a_file_picker_opens_beside_an_existing_entry() {
        let (_dir, root) = fixture();
        assert_eq!(
            folder_of_existing_entry(&text(&root.join("tool"))),
            Some(root.clone())
        );
        assert_eq!(
            folder_of_existing_entry(&text(&root.join("sub"))),
            Some(root.clone()),
            "a bundle folder opens among its siblings"
        );
    }

    #[test]
    fn a_file_picker_keeps_the_default_for_a_missing_or_bare_entry() {
        let (_dir, root) = fixture();
        assert_eq!(folder_of_existing_entry(&text(&root.join("missing"))), None);
        assert_eq!(folder_of_existing_entry("git"), None, "a PATH lookup");
        assert_eq!(folder_of_existing_entry(""), None);
    }
}
