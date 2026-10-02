use std::io;
use std::path::{Component, Path, PathBuf};

/// Normalize a repository path while refusing paths that can escape the
/// worktree or address Git's private metadata.
pub fn validated_repo_relative_path(path: &Path) -> io::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        let name = match component {
            Component::CurDir => continue,
            Component::Normal(name) => name,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("repository path must be relative: {}", path.display()),
                ));
            }
        };
        if is_git_metadata_component(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("repository path cannot enter .git: {}", path.display()),
            ));
        }
        normalized.push(name);
    }

    if normalized.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "repository path cannot be empty",
        ));
    }
    Ok(normalized)
}

/// `.git` under any spelling the filesystem accepts, compared as bytes so a
/// name that is not valid UTF-8 cannot slip past.
pub fn is_git_metadata_component(component: &std::ffi::OsStr) -> bool {
    let bytes = component.as_encoded_bytes();
    #[cfg(windows)]
    // NTFS: a stream after `:`, ignored trailing dots and spaces, and `git~1`.
    let bytes = {
        let bytes = bytes.split(|&byte| byte == b':').next().unwrap_or(bytes);
        let end = bytes
            .iter()
            .rposition(|&byte| byte != b'.' && byte != b' ')
            .map_or(0, |index| index + 1);
        &bytes[..end]
    };
    bytes.eq_ignore_ascii_case(b".git") || (cfg!(windows) && bytes.eq_ignore_ascii_case(b"git~1"))
}

/// Name prefix of the filesystem service's staging and undo areas.
pub const OPERATION_AREA_PREFIX: &str = ".gitcomet-operation-";
/// Name prefix of an editor save's temporary file beside its target.
pub const SAVE_STAGING_PREFIX: &str = ".gitcomet-save-";

/// A staging entry the filesystem service owns; never user content.
pub fn is_service_owned_name(name: &[u8]) -> bool {
    name.starts_with(OPERATION_AREA_PREFIX.as_bytes())
        || name.starts_with(SAVE_STAGING_PREFIX.as_bytes())
}

pub fn has_service_owned_component(path: &Path) -> bool {
    path.components()
        .any(|component| is_service_owned_name(component.as_os_str().as_encoded_bytes()))
}

/// The Git directory of `workdir`: its `.git` directory, or the directory a
/// `gitdir:` file points at (linked worktrees, submodules).
pub fn resolved_git_dir(workdir: &Path) -> Option<PathBuf> {
    let dot_git = workdir.join(".git");
    let metadata = std::fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git);
    }
    if !metadata.is_file() {
        return None;
    }
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let target = contents
        .lines()
        .next()?
        .trim()
        .strip_prefix("gitdir:")?
        .trim();
    if target.is_empty() {
        return None;
    }
    let target = canonicalize_or_original(workdir.join(target));
    target.is_dir().then_some(target)
}

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
    use super::{
        canonicalize_or_original, git_dir_for_workdir, has_service_owned_component,
        is_service_owned_name, resolved_git_dir, symlink_free_write_target,
        validated_repo_relative_path,
    };
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn service_owned_names_cover_operation_areas_and_save_staging_only() {
        assert!(is_service_owned_name(b".gitcomet-operation-a1b2"));
        assert!(is_service_owned_name(b".gitcomet-save-x"));
        for name in [&b"gitcomet-operation-x"[..], b".gitcomet", b"notes.md", b""] {
            assert!(!is_service_owned_name(name), "{name:?}");
        }
        assert!(has_service_owned_component(Path::new(
            "src/.gitcomet-operation-x/item"
        )));
        assert!(!has_service_owned_component(Path::new(
            "src/gitcomet-operation-x/item"
        )));
    }

    #[test]
    fn resolved_git_dir_follows_directories_and_gitdir_files() {
        let dir = tempdir().unwrap();
        // Without the `\\?\` prefix `fs::canonicalize` adds on Windows.
        let root = canonicalize_or_original(dir.path().to_path_buf());
        let plain = root.join("plain");
        fs::create_dir_all(plain.join(".git")).unwrap();
        assert_eq!(resolved_git_dir(&plain), Some(plain.join(".git")));

        let target = root.join("gitdirs/linked");
        fs::create_dir_all(&target).unwrap();
        let absolute = root.join("absolute");
        fs::create_dir(&absolute).unwrap();
        fs::write(
            absolute.join(".git"),
            format!("gitdir: {}\n", target.display()),
        )
        .unwrap();
        assert_eq!(resolved_git_dir(&absolute), Some(target.clone()));

        let relative = root.join("relative");
        fs::create_dir(&relative).unwrap();
        fs::write(relative.join(".git"), "gitdir: ../gitdirs/linked\n").unwrap();
        assert_eq!(resolved_git_dir(&relative), Some(target));

        for (name, contents) in [
            ("empty", "gitdir:   \n"),
            ("dangling", "gitdir: ../missing\n"),
            ("garbage", "not a pointer\n"),
        ] {
            let workdir = root.join(name);
            fs::create_dir(&workdir).unwrap();
            fs::write(workdir.join(".git"), contents).unwrap();
            assert_eq!(resolved_git_dir(&workdir), None, "{name}");
        }
        assert_eq!(resolved_git_dir(&root.join("missing")), None);
    }

    #[test]
    fn repository_relative_paths_reject_escape_and_git_metadata() {
        assert_eq!(
            validated_repo_relative_path(Path::new("docs/notes.md")).unwrap(),
            Path::new("docs/notes.md")
        );
        assert_eq!(
            validated_repo_relative_path(Path::new("./docs/notes.md")).unwrap(),
            Path::new("docs/notes.md")
        );
        for path in [
            "",
            ".",
            "../outside",
            "docs/../../outside",
            "/absolute",
            ".git/config",
            "src/.GiT/config",
        ] {
            assert!(
                validated_repo_relative_path(Path::new(path)).is_err(),
                "{path:?} must be rejected"
            );
        }
    }

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
