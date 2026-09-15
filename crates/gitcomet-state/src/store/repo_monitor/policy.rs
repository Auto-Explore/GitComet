//! Immutable path classification and the filesystem inputs used to build it.
use super::*;
use gitcomet_core::services::RepositoryWatchInfo;
use std::sync::RwLock;

pub(super) type PolicyCell = Arc<RwLock<Arc<PolicySnapshot>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PathClass {
    Cache,
    Control,
    ControlEntry,
    Index,
    Git { tags: bool },
    Excluded,
    Worktree,
    Outside,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct PolicySnapshot {
    pub workdir: PathBuf,
    pub git_roots: FxHashSet<PathBuf>,
    pub cache_roots: FxHashSet<PathBuf>,
    pub excluded_roots: FxHashSet<PathBuf>,
    pub control_files: FxHashSet<PathBuf>,
    pub control_entries: FxHashSet<PathBuf>,
    pub index_files: FxHashSet<PathBuf>,
    tag_roots: FxHashSet<PathBuf>,
    packed_refs: FxHashSet<PathBuf>,
    input_ancestors: FxHashSet<PathBuf>,
}

impl PolicySnapshot {
    pub fn new(workdir: &Path, inputs: &WatchInputs) -> Self {
        let mut policy = Self {
            workdir: workdir.to_path_buf(),
            ..Self::default()
        };
        policy
            .git_roots
            .extend(inputs.info.git_dirs.iter().map(|path| normalized(path)));
        policy
            .cache_roots
            .extend(inputs.cache_roots.iter().cloned());
        for root in &policy.git_roots {
            policy.cache_roots.insert(root.join("index.lock"));
            policy.index_files.insert(root.join("index"));
            policy.tag_roots.insert(root.join("refs/tags"));
            policy.packed_refs.insert(root.join("packed-refs"));
        }
        for path in &inputs.inputs {
            if inputs.stamps.is_directory(path)
                && (policy.git_roots.contains(path)
                    || path.file_name().is_some_and(|name| name == ".git"))
            {
                policy.control_entries.insert(path.clone());
            } else {
                policy.control_files.insert(path.clone());
            }
            policy
                .input_ancestors
                .extend(path.ancestors().skip(1).map(Path::to_path_buf));
        }
        policy
    }

    /// One allocation-free ancestors walk, with cache precedence even for
    /// explicitly configured inputs inside private Git/LFS storage.
    pub fn classify(&self, path: &Path) -> PathClass {
        let mut git = false;
        let mut tags = self.packed_refs.contains(path);
        let mut excluded = false;
        let mut worktree = false;
        for ancestor in path.ancestors() {
            if self.cache_roots.contains(ancestor) {
                return PathClass::Cache;
            }
            git |= self.git_roots.contains(ancestor);
            tags |= self.tag_roots.contains(ancestor);
            excluded |= self.excluded_roots.contains(ancestor);
            worktree |= ancestor == self.workdir;
        }
        // Watchman fsmonitor queries create cookies directly in Git directories.
        // Filter that read-side effect in every discovered administrative root,
        // without hiding similarly named worktree files or branch/tag refs.
        if git
            && path
                .parent()
                .is_some_and(|parent| self.git_roots.contains(parent))
            && path
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b".watchman-cookie-"))
        {
            return PathClass::Cache;
        }
        if self.control_files.contains(path) {
            PathClass::Control
        } else if self.control_entries.contains(path) {
            PathClass::ControlEntry
        } else if self.index_files.contains(path) {
            PathClass::Index
        } else if git {
            PathClass::Git { tags }
        } else if excluded {
            PathClass::Excluded
        } else if worktree {
            PathClass::Worktree
        } else {
            PathClass::Outside
        }
    }

    pub fn with_excluded(&self, dir: PathBuf) -> Self {
        let mut next = self.clone();
        next.excluded_roots.insert(dir);
        next
    }

    pub fn input_below(&self, path: &Path) -> bool {
        self.input_ancestors.contains(path)
    }

    pub fn is_git_directory_modify(&self, path: &Path, event: &notify::Event) -> bool {
        matches!(event.kind, notify::EventKind::Modify(kind) if !matches!(kind, notify::event::ModifyKind::Name(_)))
            && self.git_roots.contains(path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Triage {
    Drop,
    Rescan,
    Relevant,
}

pub(super) fn triage(policy: &PolicySnapshot, event: &notify::Event) -> Triage {
    if event.need_rescan() {
        return Triage::Rescan;
    }
    if should_ignore_event_kind(event) {
        return Triage::Drop;
    }
    if event.paths.is_empty() {
        return Triage::Rescan;
    }
    if event.paths.iter().any(|path| {
        // Recursive backends can echo cache/cookie writes as a timestamp
        // modification of the Git directory. Child events cover actual state;
        // structural events and input identity stamps cover root replacement.
        if policy.is_git_directory_modify(path, event) {
            return false;
        }
        match policy.classify(path) {
            PathClass::Cache => false,
            PathClass::Excluded | PathClass::Outside => {
                structural_event(event) && policy.input_below(path)
            }
            _ => true,
        }
    }) {
        Triage::Relevant
    } else {
        Triage::Drop
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Stamp {
    Missing,
    Failed(std::io::ErrorKind),
    Present {
        len: u64,
        modified: Option<std::time::SystemTime>,
        created: Option<std::time::SystemTime>,
        directory: bool,
        link: Option<PathBuf>,
        #[cfg(unix)]
        inode: u64,
        #[cfg(unix)]
        ctime: (i64, i64),
    },
}

fn stamp(path: &Path) -> Stamp {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let directory = metadata.is_dir();
            // A .git directory changes mtime whenever a lock file changes.
            // Revalidate its identity, not its entries, to avoid self-reloads.
            Stamp::Present {
                len: if directory { 0 } else { metadata.len() },
                modified: (!directory).then(|| metadata.modified().ok()).flatten(),
                created: metadata.created().ok(),
                directory,
                link: metadata
                    .file_type()
                    .is_symlink()
                    .then(|| fs::read_link(path).ok())
                    .flatten(),
                #[cfg(unix)]
                inode: std::os::unix::fs::MetadataExt::ino(&metadata),
                #[cfg(unix)]
                ctime: if directory {
                    (0, 0)
                } else {
                    (
                        std::os::unix::fs::MetadataExt::ctime(&metadata),
                        std::os::unix::fs::MetadataExt::ctime_nsec(&metadata),
                    )
                },
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Stamp::Missing,
        Err(error) => Stamp::Failed(error.kind()),
    }
}

#[derive(Default)]
pub(super) struct Stamps(FxHashMap<PathBuf, Stamp>);
impl Stamps {
    fn is_directory(&self, path: &Path) -> bool {
        matches!(
            self.0.get(path),
            Some(Stamp::Present {
                directory: true,
                ..
            })
        )
    }
    pub fn capture(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self(
            paths
                .into_iter()
                .map(|path| {
                    let value = stamp(&path);
                    (path, value)
                })
                .collect(),
        )
    }
    pub fn changed(&self) -> bool {
        self.0.iter().any(|(path, old)| stamp(path) != *old)
    }
    pub fn add(&mut self, path: PathBuf) {
        self.0.entry(path).or_insert_with_key(|path| stamp(path));
    }
    pub fn recapture(&mut self) {
        for (path, previous) in &mut self.0 {
            *previous = stamp(path);
        }
    }
}

#[derive(Default)]
pub(super) struct WatchInputs {
    pub info: RepositoryWatchInfo,
    pub inputs: Vec<PathBuf>,
    pub stamps: Stamps,
    pub indexes: Stamps,
    cache_roots: Vec<PathBuf>,
}
impl WatchInputs {
    pub fn load(workdir: &Path, backend: &dyn GitBackend) -> gitcomet_core::services::Result<Self> {
        // These inputs bracket discovery itself, including the unborn index.
        let root_git = resolve_git_dir(workdir);
        let mut stamps = Stamps::capture([workdir.join(".git"), workdir.join(".gitignore")]);
        stamps.add(workdir.to_path_buf());
        let mut indexes = Stamps::default();
        if let Some(root) = &root_git {
            stamps.add(root.join("config"));
            indexes.add(root.join("index"));
        }
        let info = backend.repository_watch_info(workdir)?.unwrap_or_else(|| {
            let mut info = RepositoryWatchInfo::default();
            if let Some(root) = root_git {
                info.cache_dirs
                    .extend([root.join("objects"), root.join("lfs")]);
                info.ignore_inputs
                    .extend([root.join("config"), root.join("info/exclude")]);
                info.git_dirs.push(root);
            }
            info
        });
        let mut inputs = Self {
            info,
            stamps,
            indexes,
            ..Self::default()
        };
        let mut links = Vec::new();
        let mut incomplete = inputs.info.discovery_incomplete;
        inputs.cache_roots =
            expand_watch_paths(inputs.info.cache_dirs.clone(), &mut links, &mut incomplete);
        let mut controls = inputs.info.ignore_inputs.clone();
        controls.extend(links);
        controls.extend([workdir.join(".git"), workdir.join(".gitignore")]);
        controls.extend(
            inputs
                .info
                .worktrees
                .iter()
                .map(|root| root.join(".gitignore")),
        );
        inputs.add_inputs(controls);
        for root in &inputs.info.git_dirs {
            inputs.indexes.add(root.join("index"));
            inputs.stamps.add(root.clone());
        }
        inputs.info.discovery_incomplete |= incomplete;
        Ok(inputs)
    }

    pub fn add_inputs(&mut self, paths: Vec<PathBuf>) {
        let mut links = Vec::new();
        let expanded = expand_watch_paths(paths, &mut links, &mut self.info.discovery_incomplete);
        for path in expanded.into_iter().chain(links) {
            if fs::metadata(&path).is_ok_and(|metadata| !metadata.is_file() && !metadata.is_dir()) {
                continue;
            }
            if !self.inputs.contains(&path) {
                self.stamps.add(path.clone());
                self.inputs.push(path);
            }
        }
        self.inputs.sort();
    }
}

pub(super) fn structural_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

pub(super) fn normalized(path: &Path) -> PathBuf {
    // Lexical normalization also works for removal events and missing inputs.
    let path = gitcomet_core::path_utils::strip_windows_verbatim_prefix(path.to_path_buf());
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

/// Resolve only the components of explicitly requested inputs. Retain missing
/// targets and every link in a chain; canonicalize alone loses both of those.
fn expand_watch_paths(
    inputs: Vec<PathBuf>,
    links: &mut Vec<PathBuf>,
    incomplete: &mut bool,
) -> Vec<PathBuf> {
    let mut paths = FxHashSet::default();
    let mut visited = FxHashSet::default();
    for input in inputs {
        let mut current = input;
        for hop in 0..=64 {
            // Preserve parent components until links have been resolved: two
            // lexically equal inputs can refer to different physical files.
            if !visited.insert(current.clone()) {
                break;
            }
            paths.insert(normalized(&current));
            // Native events use filesystem spelling, including Windows casing.
            // Missing inputs still need the spelling of their existing parent.
            for ancestor in current.ancestors() {
                if let Ok(resolved) = fs::canonicalize(ancestor) {
                    paths.insert(normalized(
                        &resolved.join(current.strip_prefix(ancestor).unwrap()),
                    ));
                    break;
                }
            }
            if hop == 64 {
                *incomplete = true;
                break;
            }
            let mut prefix = PathBuf::new();
            let mut components = current.components();
            let mut target = None;
            while let Some(component) = components.next() {
                prefix.push(component);
                match fs::symlink_metadata(&prefix) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        links.push(normalized(&prefix));
                        match fs::read_link(&prefix) {
                            Ok(path) => {
                                let resolved = if path.is_absolute() {
                                    path
                                } else {
                                    prefix.parent().unwrap_or(Path::new("")).join(path)
                                };
                                target = Some(resolved.join(components.as_path()));
                            }
                            Err(_) => *incomplete = true,
                        }
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if error.kind() != std::io::ErrorKind::NotFound {
                            *incomplete = true;
                        }
                        break;
                    }
                }
            }
            match target {
                Some(path) => current = path,
                None => break,
            }
        }
    }
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_links_are_resolved_before_lexical_deduplication() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalized(&temp.path().canonicalize().unwrap());
        let destination = root.join("destination");
        fs::create_dir_all(destination.join("child")).unwrap();
        let link = root.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(destination.join("child"), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(destination.join("child"), &link).unwrap();
        let direct = root.join("ignore");
        let mut links = Vec::new();
        let mut incomplete = false;
        let paths = expand_watch_paths(
            vec![direct.clone(), link.join("../ignore")],
            &mut links,
            &mut incomplete,
        );
        // link/../ignore resolves against the link's destination. It is a
        // different input from root/ignore despite the same lexical spelling.
        assert!(paths.contains(&destination.join("ignore")));
        assert!(paths.contains(&direct));
        assert!(links.contains(&link));
        assert!(!incomplete);
    }
}
