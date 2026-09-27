use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{Result, WorktreeIgnoreMatcher, WorktreePathKind};
use gix::index::entry::Mode as GitIndexMode;
use std::path::Path;

pub(crate) fn repository_watch_info(
    workdir: &Path,
) -> Result<gitcomet_core::services::RepositoryWatchInfo> {
    use gitcomet_core::path_utils::strip_windows_verbatim_prefix;
    let normalize = |path: &Path| {
        strip_windows_verbatim_prefix(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
    };
    let repo = crate::open::open_worktree_repo(workdir)
        .map_err(|error| crate::open::map_open_error(error, "watch metadata"))?;
    let mut info = gitcomet_core::services::RepositoryWatchInfo::default();
    let home = gix::path::env::home_dir();
    if let Some(global) = std::env::var_os("GIT_CONFIG_GLOBAL") {
        info.ignore_inputs.push(global.into());
    } else {
        if let Some(home) = &home {
            info.ignore_inputs.push(home.join(".gitconfig"));
        }
        if let Some(path) = gix::path::env::xdg_config("config", &mut |key| std::env::var_os(key)) {
            info.ignore_inputs.push(path);
        }
    }
    let mut pending = vec![repo];
    let mut visited = std::collections::HashSet::new();
    while let Some(repo) = pending.pop() {
        if !visited.insert(normalize(repo.git_dir())) {
            continue;
        }
        if visited.len() > 4096 {
            return Err(Error::new(ErrorKind::Backend(
                "Too many submodules to monitor".into(),
            )));
        }
        if let Some(workdir) = repo.workdir() {
            // Submodule::open() also opens retained administrative repositories
            // after deinit. Their configured workdir is not an initialized
            // checkout and cannot be used to construct an ignore matcher.
            if workdir
                .join(".git")
                .try_exists()
                .map_err(|error| Error::new(ErrorKind::Io(error.kind())))?
            {
                info.worktrees.push(normalize(workdir));
                // `git add child` can record an embedded repository as a
                // gitlink without a .gitmodules entry. Follow the index too,
                // so its metadata and ignore rules survive worktree pruning.
                let index = repo.index_or_empty().map_err(|error| {
                    Error::new(ErrorKind::Backend(format!("watch gitlink index: {error}")))
                })?;
                for entry in index
                    .entries()
                    .iter()
                    .filter(|entry| entry.mode == GitIndexMode::COMMIT)
                {
                    let relative =
                        gix::path::try_from_bstr(entry.path(&index)).map_err(|error| {
                            Error::new(ErrorKind::Backend(format!("watch gitlink path: {error}")))
                        })?;
                    let child = workdir.join(relative);
                    info.ignore_inputs.push(child.join(".git"));
                    match crate::open::open_worktree_repo(&child) {
                        Ok(child) => pending.push(child),
                        // Missing/deinitialized checkouts still retain their
                        // .git input above, so initialization rebuilds coverage.
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(crate::open::map_open_error(error, "watch gitlink"));
                        }
                    }
                }
            }
            // Observe reinitialization even while the checkout is absent.
            info.ignore_inputs.push(workdir.join(".git"));
            info.ignore_inputs.push(workdir.join(".gitmodules"));
        }
        for dir in [repo.git_dir(), repo.common_dir()] {
            info.git_dirs.push(normalize(dir));
            for file in ["config", "config.worktree", "info/exclude", "commondir"] {
                info.ignore_inputs.push(dir.join(file));
            }
        }
        let config = repo.config_snapshot();
        if let Some(storage) = config
            .string("lfs.storage")
            .filter(|value| !value.is_empty())
        {
            // LFS treats this as a literal path (no ~/ interpolation). Relative
            // storage belongs to the common Git directory for linked worktrees.
            let storage = gix::path::try_from_bstr(&storage).map_err(|error| {
                Error::new(ErrorKind::Backend(format!("watch LFS storage: {error}")))
            })?;
            let storage = if storage.is_absolute() {
                storage.into_owned()
            } else {
                repo.common_dir().join(storage)
            };
            // Git LFS uses filepath.Join/Clean before opening the path. In
            // particular, link/../Storage must not follow link before popping it.
            let storage = gix::path::normalize_saturating(storage.into(), workdir);
            // Storage may share a directory with source files or Git metadata.
            // Only LFS-owned folders are private, not every path under storage.
            info.cache_dirs.extend(
                ["objects", "tmp", "logs", "incomplete"]
                    .into_iter()
                    .map(|name| storage.join(name)),
            );
        }
        for section in config.plumbing().sections() {
            if let Some(path) = &section.meta().path {
                // Keep the configured spelling too: replacing a config symlink
                // must be observed as well as editing its current target.
                info.ignore_inputs.push(path.clone());
            }
            let header = section.header();
            if header.name().eq_ignore_ascii_case(b"include")
                || header.name().eq_ignore_ascii_case(b"includeIf")
            {
                for value in section.values("path") {
                    let value = gix::config::Path::from(value);
                    let path = match value.interpolate(gix::config::path::interpolate::Context {
                        home_dir: home.as_deref(),
                        git_install_dir: gix::path::env::installation_config_prefix(),
                        ..Default::default()
                    }) {
                        Ok(path) => path,
                        // Inactive conditions may name users unavailable here.
                        // Opening the repo already processed active includes;
                        // keep observing this section's config even if its
                        // optional target cannot be resolved on this machine.
                        Err(_) if header.name().eq_ignore_ascii_case(b"includeIf") => continue,
                        Err(error) => {
                            return Err(Error::new(ErrorKind::Backend(format!(
                                "watch config include: {error}"
                            ))));
                        }
                    };
                    let parent = section
                        .meta()
                        .path
                        .as_ref()
                        .and_then(|path| path.parent())
                        .unwrap_or(workdir);
                    info.ignore_inputs.push(if path.is_absolute() {
                        path
                    } else {
                        parent.join(path)
                    });
                }
            }
        }
        if let Some(path) = config
            .trusted_path("core.excludesFile")
            .map_err(|error| Error::new(ErrorKind::Backend(format!("watch excludes: {error}"))))?
        {
            info.ignore_inputs.push(if path.is_absolute() {
                path
            } else {
                workdir.join(path)
            });
        } else if let Some(path) =
            gix::path::env::xdg_config("ignore", &mut |key| std::env::var_os(key))
        {
            info.ignore_inputs.push(path);
        }
        if let Some(submodules) = repo
            .submodules()
            .map_err(|error| Error::new(ErrorKind::Backend(format!("watch submodules: {error}"))))?
        {
            for submodule in submodules {
                if let Some(child) = submodule.open().map_err(|error| {
                    Error::new(ErrorKind::Backend(format!("watch submodule: {error}")))
                })? {
                    pending.push(child);
                }
            }
        }
    }
    // Retained administrative repositories need metadata coverage even when no
    // initialized checkout or .gitmodules entry leads to them. Own this bounded
    // discovery here so every consumer uses the same Git and cache roots.
    let mut namespaces = Vec::new();
    for dir in &info.git_dirs {
        namespaces.extend([dir.join("modules"), dir.join("worktrees")]);
    }
    let mut seen = std::collections::HashSet::new();
    while let Some(dir) = namespaces.pop() {
        let dir = normalize(&dir);
        if !seen.insert(dir.clone()) {
            continue;
        }
        if seen.len() > 4096 {
            info.discovery_incomplete = true;
            break;
        }
        if dir.join("HEAD").is_file()
            && (dir.join("config").is_file() || dir.join("commondir").is_file())
        {
            info.git_dirs.push(dir.clone());
            if let Ok(common) = std::fs::read_to_string(dir.join("commondir")) {
                let common = normalize(&dir.join(common.trim()));
                if !info.git_dirs.contains(&common) {
                    namespaces.extend([common.join("modules"), common.join("worktrees")]);
                    info.git_dirs.push(common);
                }
            }
            namespaces.extend([dir.join("modules"), dir.join("worktrees")]);
            continue;
        }
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => match entry.file_type() {
                            Ok(kind) if kind.is_dir() => namespaces.push(entry.path()),
                            Ok(_) => {}
                            Err(_) => info.discovery_incomplete = true,
                        },
                        Err(_) => info.discovery_incomplete = true,
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => info.discovery_incomplete = true,
        }
    }
    for dir in &info.git_dirs {
        info.cache_dirs
            .extend([dir.join("objects"), dir.join("lfs")]);
        info.ignore_inputs.extend(
            ["config", "config.worktree", "commondir", "info/exclude"]
                .into_iter()
                .map(|name| dir.join(name)),
        );
    }
    info.git_dirs.sort();
    info.git_dirs.dedup();
    info.cache_dirs.sort();
    info.cache_dirs.dedup();
    info.ignore_inputs.sort();
    info.ignore_inputs.dedup();
    info.worktrees
        .sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(info)
}

pub(crate) struct GixWorktreeIgnoreMatcher {
    repo: gix::Repository,
    index: gix::worktree::Index,
    excludes: gix::worktree::Stack,
}

impl GixWorktreeIgnoreMatcher {
    pub(crate) fn load(workdir: &Path) -> Result<Self> {
        let repo = crate::open::open_worktree_repo(workdir)
            .map_err(|error| crate::open::map_open_error(error, "gix ignore matcher open"))?;
        repo.worktree().ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "gix ignore matcher: repository has no worktree".to_string(),
            ))
        })?;
        // Before the first staging operation there is no index on disk. Use
        // an empty in-memory snapshot in that case, while retaining errors for
        // unreadable or corrupt indexes. Monitoring must not create the index.
        let index = repo.index_or_empty().map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "gix ignore matcher index: {error}"
            )))
        })?;
        let excludes = repo
            .excludes(
                &index,
                None,
                gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(|error| {
                Error::new(ErrorKind::Backend(format!(
                    "gix ignore matcher excludes: {error}"
                )))
            })?
            .detach();

        Ok(Self {
            repo,
            index,
            excludes,
        })
    }

    /// Git never ignores a path with tracked content at or beneath it, whatever
    /// the watcher reported the path as: a file or symlink that replaced a
    /// tracked directory has just deleted every entry under it, so the caller's
    /// kind must not skip the lookup beneath the path.
    fn path_is_tracked(&self, relative_path: &Path) -> bool {
        let relative_path =
            gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative_path));
        self.index.entry_by_path(relative_path.as_ref()).is_some()
            || self
                .index
                .entry_closest_to_directory_or_directory(relative_path.as_ref())
                .is_some()
    }
}

impl WorktreeIgnoreMatcher for GixWorktreeIgnoreMatcher {
    fn is_ignored(&mut self, relative_path: &Path, kind: WorktreePathKind) -> Result<bool> {
        if self.path_is_tracked(relative_path) {
            return Ok(false);
        }

        let mode = match kind {
            WorktreePathKind::Directory => Some(GitIndexMode::DIR),
            WorktreePathKind::File => Some(GitIndexMode::FILE),
            WorktreePathKind::Unknown => None,
        };
        let platform = self
            .excludes
            .at_path(relative_path, mode, &self.repo.objects)
            .map_err(|error| {
                Error::new(ErrorKind::Backend(format!(
                    "gix ignore matcher path: {error}"
                )))
            })?;
        Ok(platform.is_excluded())
    }
}
