use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Focus and range anchor are independent of selection and of the open preview.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub paths: BTreeSet<PathBuf>,
    pub focused: Option<PathBuf>,
    pub anchor: Option<PathBuf>,
}

impl Selection {
    pub fn click(
        &mut self,
        path: PathBuf,
        visible: &[PathBuf],
        toggle: bool,
        range: bool,
        context_menu: bool,
    ) {
        self.focused = Some(path.clone());
        if context_menu && self.paths.contains(&path) {
            return;
        }
        if range
            && let Some(start) = self
                .anchor
                .as_ref()
                .and_then(|p| visible.iter().position(|v| v == p))
            && let Some(end) = visible.iter().position(|v| v == &path)
        {
            if !toggle {
                self.paths.clear();
            }
            self.paths
                .extend(visible[start.min(end)..=start.max(end)].iter().cloned());
            return;
        }
        if toggle {
            if !self.paths.remove(&path) {
                self.paths.insert(path.clone());
            }
        } else {
            self.paths.clear();
            self.paths.insert(path.clone());
        }
        self.anchor = Some(path);
    }

    pub fn select_all(&mut self, visible: &[PathBuf]) {
        self.paths = visible.iter().cloned().collect();
        if self.focused.is_none() {
            self.focused = visible.first().cloned();
        }
    }

    pub fn destination(path: Option<&Path>, is_directory: bool) -> PathBuf {
        match path {
            Some(path) if is_directory => path.to_path_buf(),
            Some(path) => path.parent().unwrap_or(Path::new("")).to_path_buf(),
            None => PathBuf::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_uses_visible_rows_and_right_click_preserves_selection() {
        let visible: Vec<_> = ["folder", "folder/a", "z"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let mut selection = Selection::default();
        selection.click(visible[0].clone(), &visible, false, false, false);
        selection.click(visible[2].clone(), &visible, false, true, false);
        assert_eq!(selection.paths.len(), 3);
        selection.click(visible[1].clone(), &visible, false, false, true);
        assert_eq!(selection.paths.len(), 3);
        assert_eq!(selection.focused.as_ref(), Some(&visible[1]));
        selection.click(visible[1].clone(), &visible, true, false, false);
        assert_eq!(selection.paths.len(), 2);
    }
    #[test]
    fn files_target_their_parent_and_empty_space_targets_root() {
        assert_eq!(
            Selection::destination(Some(Path::new("a/file")), false),
            Path::new("a")
        );
        assert_eq!(
            Selection::destination(Some(Path::new("a")), true),
            Path::new("a")
        );
        assert_eq!(Selection::destination(None, false), Path::new(""));
    }
}
