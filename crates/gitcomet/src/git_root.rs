use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn is_git_root_marker(dot_git: &Path) -> bool {
    if dot_git.is_dir() {
        return dot_git.join("HEAD").is_file();
    }

    if !dot_git.is_file() {
        return false;
    }

    fs::read(dot_git).is_ok_and(|contents| {
        let line = contents
            .split(|byte| matches!(byte, b'\n' | b'\r'))
            .next()
            .unwrap_or_default();
        line.strip_prefix(b"gitdir:")
            .is_some_and(|path| !path.trim_ascii().is_empty())
    })
}

/// Walks `start`'s ancestors looking for a `.git` root marker.
///
/// Pure filesystem traversal: no git process is spawned, so this works for
/// paths outside a repository too. `rev-parse --show-toplevel` remains the
/// authoritative probe where git is available (see `cli/git_config.rs`).
pub(crate) fn find_git_root(start: &Path) -> Option<PathBuf> {
    let start_dir = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    for candidate in start_dir.ancestors() {
        if is_git_root_marker(&candidate.join(".git")) {
            return Some(fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_git_directory_is_not_a_root_marker() {
        let temp = tempfile::tempdir().unwrap();
        let dot_git = temp.path().join(".git");
        fs::create_dir_all(&dot_git).unwrap();

        assert!(!is_git_root_marker(&dot_git));
    }

    #[test]
    fn git_directory_with_head_is_a_root_marker() {
        let temp = tempfile::tempdir().unwrap();
        let dot_git = temp.path().join(".git");
        fs::create_dir_all(&dot_git).unwrap();
        fs::write(dot_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        assert!(is_git_root_marker(&dot_git));
    }

    #[test]
    fn gitdir_file_with_target_is_a_root_marker() {
        let temp = tempfile::tempdir().unwrap();
        let dot_git = temp.path().join(".git");
        fs::write(&dot_git, b"gitdir: ../.git/worktrees/example\n").unwrap();

        assert!(is_git_root_marker(&dot_git));
    }

    #[test]
    fn gitdir_file_without_target_is_not_a_root_marker() {
        let temp = tempfile::tempdir().unwrap();
        let dot_git = temp.path().join(".git");
        fs::write(&dot_git, b"gitdir:   \n").unwrap();

        assert!(!is_git_root_marker(&dot_git));
    }
}
