use std::io;
use std::path::{Path, PathBuf};

/// When a workdir path ends with ".git" and contains a `.git` entry (e.g.
/// `/home/user/myrepo.git`), gix::open may misinterpret the workdir as the git
/// directory itself.  This helper returns the `.git` entry — whether a directory
/// (non-bare clone) or a `gitdir:` file (linked worktree) — so that gix can
/// open it correctly.
pub fn git_dir_for_workdir(workdir: &Path) -> PathBuf {
    let dot_git = workdir.join(".git");
    if workdir.extension().is_some_and(|ext| ext == "git") && dot_git.exists() {
        dot_git
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

/// Resolve `relative` under `workdir` for a write, refusing when any component
/// on the way is a symlink.
///
/// A lexical check on `relative` stops `..` from leaving the worktree, but the
/// filesystem can still redirect the write: a tracked symlink such as
/// `notes.md -> ~/.ssh/authorized_keys` lists like a plain file and
/// `fs::write` follows it. Git itself never writes *through* a symlink when it
/// checks files out, so refusing here keeps the editor at parity. Components
/// that do not exist yet are fine; the caller creates them.
pub fn symlink_free_write_target(workdir: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut candidate = workdir.to_path_buf();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "refusing to write through symlink '{}'",
                        candidate
                            .strip_prefix(workdir)
                            .unwrap_or(&candidate)
                            .display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => break,
            Err(err) => return Err(err),
        }
    }
    Ok(workdir.join(relative))
}

#[cfg(test)]
mod tests {
    use super::{git_dir_for_workdir, symlink_free_write_target};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn symlink_free_write_target_accepts_regular_and_missing_paths() {
        let dir = tempdir().expect("create temp dir");
        fs::create_dir_all(dir.path().join("docs")).expect("create docs");
        fs::write(dir.path().join("docs/notes.md"), "x").expect("write notes");

        assert_eq!(
            symlink_free_write_target(dir.path(), Path::new("docs/notes.md")).expect("regular"),
            dir.path().join("docs/notes.md")
        );
        assert_eq!(
            symlink_free_write_target(dir.path(), Path::new("new/dir/file.txt"))
                .expect("missing components are created later"),
            dir.path().join("new/dir/file.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_free_write_target_refuses_symlinked_file() {
        let dir = tempdir().expect("create temp dir");
        let outside = tempdir().expect("create outside dir");
        let target = outside.path().join("victim");
        fs::write(&target, "keep").expect("write victim");
        std::os::unix::fs::symlink(&target, dir.path().join("notes.md")).expect("symlink");

        let err = symlink_free_write_target(dir.path(), Path::new("notes.md"))
            .expect_err("a symlinked file must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("notes.md"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_free_write_target_refuses_symlinked_parent() {
        let dir = tempdir().expect("create temp dir");
        let outside = tempdir().expect("create outside dir");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("docs")).expect("symlink");

        let err = symlink_free_write_target(dir.path(), Path::new("docs/notes.md"))
            .expect_err("a symlinked parent must be refused");
        assert!(err.to_string().contains("docs"), "{err}");
    }

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
