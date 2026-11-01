//! Which repository a checkout is a first-level submodule of.
//!
//! Detection reads two small files and never runs Git, so views can ask for
//! every open tab, including restored tabs that have not been opened yet.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository a submodule checkout belongs to.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SubmoduleParent {
    /// Working tree of the parent repository (or of its linked worktree).
    pub workdir: PathBuf,
    /// The submodule's path inside `workdir`.
    pub path: PathBuf,
}

/// Returns the parent of `workdir` when `workdir` is a submodule checkout.
///
/// A submodule checkout has a `.git` file, and the nearest enclosing working
/// tree lists its path in `.gitmodules`. Linked worktrees also have a `.git`
/// file, but are not listed there, so a worktree placed inside its main
/// checkout is not mistaken for a submodule. Only the direct parent is
/// reported; its own parent is not followed.
pub fn detect_submodule_parent(workdir: &Path) -> Option<SubmoduleParent> {
    let metadata = fs::symlink_metadata(workdir.join(".git")).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let parent_workdir = workdir
        .ancestors()
        .skip(1)
        .find(|ancestor| fs::symlink_metadata(ancestor.join(".git")).is_ok())?;
    let path = workdir.strip_prefix(parent_workdir).ok()?.to_path_buf();
    let gitmodules = fs::read_to_string(parent_workdir.join(".gitmodules")).ok()?;
    gitmodules_lists_path(&gitmodules, &path).then(|| SubmoduleParent {
        workdir: parent_workdir.to_path_buf(),
        path,
    })
}

/// Whether a `.gitmodules` text has a `path = <path>` entry for `path`.
fn gitmodules_lists_path(gitmodules: &str, path: &Path) -> bool {
    let wanted = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if wanted.is_empty() {
        return false;
    }
    gitmodules.lines().any(|line| {
        let Some((key, value)) = line.trim().split_once('=') else {
            return false;
        };
        if !key.trim().eq_ignore_ascii_case("path") {
            return false;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        value.trim_end_matches('/') == wanted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn submodule_checkout_reports_its_direct_parent() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("app");
        fs::create_dir_all(parent.join(".git")).unwrap();
        write(
            &parent.join(".gitmodules"),
            "[submodule \"syntax\"]\n\tpath = fixtures/syntax_test\n\turl = ../syntax\n",
        );
        let child = parent.join("fixtures/syntax_test");
        write(
            &child.join(".git"),
            "gitdir: ../../.git/modules/fixtures/syntax_test\n",
        );

        assert_eq!(
            detect_submodule_parent(&child),
            Some(SubmoduleParent {
                workdir: parent.clone(),
                path: PathBuf::from("fixtures/syntax_test"),
            })
        );
        assert_eq!(detect_submodule_parent(&parent), None);
    }

    #[test]
    fn nested_submodule_reports_only_the_submodule_it_lives_in() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().join("top");
        fs::create_dir_all(top.join(".git")).unwrap();
        write(&top.join(".gitmodules"), "[submodule \"a\"]\n\tpath = a\n");
        let a = top.join("a");
        write(&a.join(".git"), "gitdir: ../.git/modules/a\n");
        write(
            &a.join(".gitmodules"),
            "[submodule \"b\"]\n\tpath = libs/b\n",
        );
        let b = a.join("libs/b");
        write(
            &b.join(".git"),
            "gitdir: ../../../.git/modules/a/modules/b\n",
        );

        let parent = detect_submodule_parent(&b).unwrap();
        assert_eq!(parent.workdir, a);
        assert_eq!(parent.path, PathBuf::from("libs/b"));
    }

    #[test]
    fn linked_worktree_inside_its_checkout_is_not_a_submodule() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("repo");
        fs::create_dir_all(main.join(".git/worktrees/feature")).unwrap();
        write(
            &main.join(".gitmodules"),
            "[submodule \"x\"]\n\tpath = vendor/x\n",
        );
        let worktree = main.join(".worktrees/feature");
        write(
            &worktree.join(".git"),
            &format!(
                "gitdir: {}\n",
                main.join(".git/worktrees/feature").display()
            ),
        );

        assert_eq!(detect_submodule_parent(&worktree), None);
    }

    #[test]
    fn gitmodules_paths_match_quoted_values_and_trailing_slashes() {
        let text = "[submodule \"a\"]\n  PATH = \"libs/a/\"\n";
        assert!(gitmodules_lists_path(text, Path::new("libs/a")));
        assert!(!gitmodules_lists_path(text, Path::new("libs")));
        assert!(!gitmodules_lists_path(text, Path::new("")));
    }
}
