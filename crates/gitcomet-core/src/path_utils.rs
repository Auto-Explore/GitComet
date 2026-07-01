use std::path::{Path, PathBuf};

/// When a workdir path ends with ".git" and contains a `.git` entry (e.g.
/// `/home/user/myrepo.git`), gix::open may misinterpret the workdir as the git
/// directory itself.  This helper returns the `.git` entry — whether a directory
/// (non-bare clone) or a `gitdir:` file (linked worktree) — so that gix can
/// open it correctly.
pub fn git_dir_for_workdir(workdir: &Path) -> PathBuf {
    let is_dot_git_suffixed_dir =
        workdir.extension().is_some_and(|ext| ext == "git") && workdir.join(".git").exists();
    if is_dot_git_suffixed_dir {
        workdir.join(".git")
    } else {
        workdir.to_path_buf()
    }
}

/// Canonicalize a path when it exists, otherwise keep the original path unchanged.
pub fn canonicalize_or_original(path: PathBuf) -> PathBuf {
    strip_windows_verbatim_prefix(std::fs::canonicalize(&path).unwrap_or(path))
}

#[cfg(windows)]
pub fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };

    let mut out = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => PathBuf::from(format!("{}:", char::from(letter))),
        Prefix::VerbatimUNC(server, share) => {
            let mut out = PathBuf::from(r"\\");
            out.push(server);
            out.push(share);
            out
        }
        Prefix::Verbatim(raw) => PathBuf::from(raw),
        _ => return path,
    };

    for component in components {
        out.push(component.as_os_str());
    }
    out
}

#[cfg(not(windows))]
pub fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
mod tests {
    use super::git_dir_for_workdir;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn normal_workdir_returns_unchanged() {
        let dir = tempdir().expect("create temp dir");
        let result = git_dir_for_workdir(dir.path());
        assert_eq!(result, dir.path());
    }

    #[test]
    fn dot_git_suffixed_dir_with_git_subdir_returns_git_dir() {
        let dir = tempdir().expect("create temp dir");
        let inner = dir.path().join("repo.git");
        fs::create_dir(&inner).expect("create repo.git");
        fs::create_dir(inner.join(".git")).expect("create .git subdir");
        let result = git_dir_for_workdir(&inner);
        assert_eq!(result, inner.join(".git"));
    }

    #[test]
    fn dot_git_suffixed_dir_without_git_subdir_returns_unchanged() {
        let dir = tempdir().expect("create temp dir");
        let inner = dir.path().join("repo.git");
        fs::create_dir(&inner).expect("create repo.git");
        let result = git_dir_for_workdir(&inner);
        assert_eq!(result, inner);
    }

    #[test]
    fn dot_git_suffixed_linked_worktree_returns_git_file() {
        let dir = tempdir().expect("create temp dir");
        let inner = dir.path().join("repo.git");
        fs::create_dir(&inner).expect("create repo.git");
        fs::write(inner.join(".git"), "gitdir: /nonexistent/actual/dir\n")
            .expect("write .git file");
        let result = git_dir_for_workdir(&inner);
        assert_eq!(result, inner.join(".git"));
    }

    #[test]
    fn dot_git_suffixed_nonexistent_path_returns_unchanged() {
        let path = std::path::Path::new("/nonexistent/path/repo.git");
        let result = git_dir_for_workdir(path);
        assert_eq!(result, path);
    }

    #[test]
    fn no_extension_directory_preserved() {
        let dir = tempdir().expect("create temp dir");
        let inner = dir.path().join("myrepo");
        fs::create_dir(&inner).expect("create dir");
        let result = git_dir_for_workdir(&inner);
        assert_eq!(result, inner);
    }
}
