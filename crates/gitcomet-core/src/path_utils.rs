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
        #[cfg(windows)]
        crate::filesystem::validate_name(name)?;
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
///
/// Every platform checks every spelling, as Git does (`core.protectNTFS` is on
/// everywhere by default): a Linux build can write to an NTFS volume, and a
/// macOS share is case-insensitive HFS+ or APFS.
pub fn is_git_metadata_component(component: &std::ffi::OsStr) -> bool {
    let bytes = component.as_encoded_bytes();
    // NTFS: a stream after `:`, ignored trailing dots and spaces, and `git~1`.
    let ntfs = bytes.split(|&byte| byte == b':').next().unwrap_or(bytes);
    let end = ntfs
        .iter()
        .rposition(|&byte| byte != b'.' && byte != b' ')
        .map_or(0, |index| index + 1);
    let ntfs = &ntfs[..end];
    if ntfs.eq_ignore_ascii_case(b".git") || ntfs.eq_ignore_ascii_case(b"git~1") {
        return true;
    }
    // HFS+ ignores some code points, so `.g\u{200c}it` is `.git` there.
    matches!(
        gix_validate::path::component(
            bytes.into(),
            None,
            gix_validate::path::component::Options {
                protect_windows: false,
                protect_hfs: true,
                protect_ntfs: true,
            },
        ),
        Err(gix_validate::path::component::Error::DotGitDir)
    )
}

/// Name prefix of the filesystem service's staging and undo areas.
pub const OPERATION_AREA_PREFIX: &str = ".gitcomet-operation-";
/// Name prefix of an editor save's temporary file beside its target.
pub const SAVE_STAGING_PREFIX: &str = ".gitcomet-save-";

const SERVICE_OWNED_PREFIXES: [&str; 2] = [OPERATION_AREA_PREFIX, SAVE_STAGING_PREFIX];

/// A staging entry the filesystem service owns; never user content.
pub fn is_service_owned_name(name: &[u8]) -> bool {
    SERVICE_OWNED_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix.as_bytes()))
}

/// Whether a service prefix occurs anywhere in `bytes`. A service-owned
/// component puts one there, and walking the components of the many paths
/// without one is the cost.
fn mentions_service_prefix(bytes: &[u8]) -> bool {
    static FINDERS: std::sync::LazyLock<[memchr::memmem::Finder<'static>; 2]> =
        std::sync::LazyLock::new(|| SERVICE_OWNED_PREFIXES.map(memchr::memmem::Finder::new));
    FINDERS.iter().any(|finder| finder.find(bytes).is_some())
}

pub fn has_service_owned_component(path: &Path) -> bool {
    mentions_service_prefix(path.as_os_str().as_encoded_bytes())
        && path
            .components()
            .any(|component| is_service_owned_name(component.as_os_str().as_encoded_bytes()))
}

/// [`has_service_owned_component`] for a repository-relative Git path, whose
/// separator is always `/`.
pub fn has_service_owned_git_component(path: &[u8]) -> bool {
    mentions_service_prefix(path) && path.split(|byte| *byte == b'/').any(is_service_owned_name)
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
    if (workdir.extension().is_some_and(|ext| ext == "git")
        || workdir.file_name().is_some_and(|name| name == ".git"))
        && dot_git.exists()
    {
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
        if component == Component::CurDir {
            continue;
        }
        #[cfg(windows)]
        crate::filesystem::validate_name(component.as_os_str())?;
        candidate.push(component.as_os_str());
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if is_path_redirect(&metadata) => {
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

fn is_path_redirect(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

pub fn worktree_entry_metadata(
    workdir: &Path,
    path: &Path,
) -> io::Result<Option<(PathBuf, std::fs::Metadata)>> {
    let relative = if path.is_absolute() {
        path.strip_prefix(workdir)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
    } else {
        path
    };
    let relative = validated_repo_relative_path(relative)?;
    let parent = relative.parent().expect("validated path has a parent");
    let parent = symlink_free_write_target(workdir, parent)?;
    let full = parent.join(relative.file_name().expect("validated path has a name"));
    match std::fs::symlink_metadata(&full) {
        Ok(metadata) if is_path_redirect(&metadata) && !metadata.is_symlink() => {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worktree path is a filesystem redirect",
            ))
        }
        Ok(metadata) => Ok(Some((full, metadata))),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub const MAX_CONFLICT_WORKTREE_BYTES: u64 = 64 * 1024 * 1024;

pub fn open_worktree_regular_file(
    workdir: &Path,
    path: &Path,
) -> io::Result<Option<std::fs::File>> {
    let Some((full, metadata)) = worktree_entry_metadata(workdir, path)? else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worktree path is not a regular file",
        ));
    }
    let file = open_worktree_file(workdir, &full)?;
    let opened = file.metadata()?;
    if !opened.is_file() || is_path_redirect(&opened) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worktree path is not a regular file",
        ));
    }
    Ok(Some(file))
}

#[cfg(unix)]
fn open_worktree_file(workdir: &Path, full: &Path) -> io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags, open, openat};

    let relative = full
        .strip_prefix(workdir)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let directory = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut parent = open(workdir, directory, Mode::empty())?;
    for component in relative.parent().unwrap().components() {
        parent = openat(
            &parent,
            component.as_os_str(),
            directory | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
    }
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = openat(&parent, relative.file_name().unwrap(), flags, Mode::empty())?;
    Ok(std::fs::File::from(file))
}

#[cfg(windows)]
fn open_worktree_file(workdir: &Path, full: &Path) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x00200000;
    let root = strip_windows_verbatim_prefix(std::fs::canonicalize(workdir)?);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(full)?;
    let opened = strip_windows_verbatim_prefix(gitcomet_filesystem_native::final_file_path(&file)?);
    let relative = opened
        .strip_prefix(root)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    validated_repo_relative_path(relative)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_worktree_file(_workdir: &Path, full: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(full)
}

pub fn read_worktree_file_bounded(
    workdir: &Path,
    path: &Path,
    byte_limit: u64,
) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;

    let Some(file) = open_worktree_regular_file(workdir, path)? else {
        return Ok(None);
    };
    let too_large = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("worktree file exceeds the {byte_limit} byte limit"),
        )
    };
    if file.metadata()?.len() > byte_limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(byte_limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > byte_limit {
        return Err(too_large());
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_or_original, git_dir_for_workdir, has_service_owned_component,
        has_service_owned_git_component, is_git_metadata_component, is_service_owned_name,
        open_worktree_regular_file, read_worktree_file_bounded, resolved_git_dir,
        symlink_free_write_target, validated_repo_relative_path, worktree_entry_metadata,
    };
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// Git refuses these names everywhere (`core.protectNTFS` is on by default
    /// on every platform; HFS+ ignores some code points). A Linux build can
    /// still write to an NTFS volume or a macOS share, so the check must not
    /// depend on where GitComet was compiled.
    #[test]
    fn git_metadata_component_matches_every_spelling_git_refuses() {
        for name in [
            ".git",
            ".GIT",
            "git~1",
            "GIT~1",
            ".git.",
            ".git ",
            ".git . .",
            ".git::$INDEX_ALLOCATION",
            ".g\u{200c}it",
            ".GI\u{200d}T",
            "\u{feff}.git",
        ] {
            assert!(
                is_git_metadata_component(OsStr::new(name)),
                "{name:?} names .git"
            );
            assert!(
                validated_repo_relative_path(&Path::new(name).join("config")).is_err(),
                "{name:?}/config must not validate as a repository path"
            );
        }
        for name in [".github", ".gitignore", "git", ".gitz", "git~2", "a.git"] {
            assert!(
                !is_git_metadata_component(OsStr::new(name)),
                "{name:?} does not name .git"
            );
        }
    }

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
        // The prefix inside a component name is not a service-owned component.
        assert!(!has_service_owned_component(Path::new(
            "src/x.gitcomet-save-y/item"
        )));
        assert!(has_service_owned_git_component(
            b"src/.gitcomet-save-y/item"
        ));
        assert!(has_service_owned_git_component(b".gitcomet-operation-x"));
        assert!(!has_service_owned_git_component(
            b"src/x.gitcomet-save-y/item"
        ));
        assert!(!has_service_owned_git_component(b"src/main.rs"));
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
    fn worktree_file_reads_enforce_size_and_preserve_missing_files() {
        let root = tempdir().unwrap();
        let path = Path::new("note.txt");
        assert_eq!(
            read_worktree_file_bounded(root.path(), path, 4).unwrap(),
            None
        );
        fs::write(root.path().join(path), b"note").unwrap();
        assert_eq!(
            read_worktree_file_bounded(root.path(), path, 4).unwrap(),
            Some(b"note".to_vec())
        );
        assert_eq!(
            read_worktree_file_bounded(root.path(), path, 3)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        assert!(open_worktree_regular_file(root.path(), Path::new(".git/HEAD")).is_err());
        assert!(open_worktree_regular_file(root.path(), Path::new("../note.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn worktree_metadata_exposes_links_without_opening_their_targets() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("note.txt"), b"note").unwrap();
        let alias = Path::new("alias.txt");
        std::os::unix::fs::symlink("note.txt", root.path().join(alias)).unwrap();
        let (_, metadata) = worktree_entry_metadata(root.path(), alias)
            .unwrap()
            .unwrap();
        assert!(metadata.is_symlink());
        assert!(open_worktree_regular_file(root.path(), alias).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn worktree_paths_refuse_directory_junctions() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("docs")).unwrap();
        fs::write(root.path().join("docs/note.txt"), b"note").unwrap();
        let result = std::process::Command::new("cmd.exe")
            .current_dir(root.path())
            .args(["/d", "/c", "mklink", "/J", "alias", "docs"])
            .output()
            .unwrap();
        assert!(result.status.success(), "junction fixture: {result:?}");

        let path = Path::new("alias/note.txt");
        assert!(worktree_entry_metadata(root.path(), path).is_err());
        assert!(symlink_free_write_target(root.path(), path).is_err());
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
    fn dot_git_named_checkout_returns_its_inner_git_directory() {
        let dir = tempdir().expect("create temp dir");
        let checkout = dir.path().join(".git");
        fs::create_dir_all(checkout.join(".git")).expect("create checkout metadata");
        assert_eq!(git_dir_for_workdir(&checkout), checkout.join(".git"));
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
