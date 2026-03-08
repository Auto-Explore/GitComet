use gpui::SharedString;
use rustc_hash::FxHashMap as HashMap;
use std::path::{Path, PathBuf};

pub(super) fn path_display_string(path: &Path) -> String {
    format_windows_path_for_display(path.display().to_string())
}

pub(super) fn path_display_shared(path: &Path) -> SharedString {
    path_display_string(path).into()
}

pub(super) fn cached_path_display(
    cache: &mut HashMap<PathBuf, SharedString>,
    path: &PathBuf,
) -> SharedString {
    const MAX_ENTRIES: usize = 8_192;
    if cache.len() > MAX_ENTRIES {
        cache.clear();
    }
    if let Some(s) = cache.get(path) {
        return s.clone();
    }
    let s = path_display_shared(path);
    cache.insert(path.clone(), s.clone());
    s
}

#[cfg(windows)]
fn format_windows_path_for_display(mut path: String) -> String {
    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        path = format!(r"\\{stripped}");
    } else if let Some(stripped) = path.strip_prefix(r"\\?\") {
        path = stripped.to_string();
    }
    path.replace('\\', "/")
}

#[cfg(not(windows))]
fn format_windows_path_for_display(path: String) -> String {
    path
}

#[cfg(test)]
mod tests {
    use super::format_windows_path_for_display;

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_disk_prefix_and_uses_forward_slashes() {
        let formatted =
            format_windows_path_for_display(r"\\?\C:\Users\sanni\git\GitComet".to_string());
        assert_eq!(formatted, "C:/Users/sanni/git/GitComet");
    }

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_unc_prefix_and_uses_forward_slashes() {
        let formatted = format_windows_path_for_display(r"\\?\UNC\server\share\repo".to_string());
        assert_eq!(formatted, "//server/share/repo");
    }

    #[cfg(not(windows))]
    #[test]
    fn leaves_non_windows_path_unchanged() {
        let formatted = format_windows_path_for_display("/tmp/repo".to_string());
        assert_eq!(formatted, "/tmp/repo");
    }
}
