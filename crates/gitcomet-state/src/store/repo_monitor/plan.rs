//! Register parents before enumerating children; each stable setup walks once.
use super::*;
use std::collections::VecDeque;

#[derive(Default, Debug)]
pub(super) struct WatchPlan {
    pub dirs: FxHashSet<PathBuf>,
    pub worktree_dirs: FxHashSet<PathBuf>,
    pub boundaries: Vec<PathBuf>,
    pub skipped: Option<usize>,
    pub failures: usize,
}
impl WatchPlan {
    pub fn walk(
        &mut self,
        starts: impl IntoIterator<Item = PathBuf>,
        worktree: bool,
        policy: &PolicySnapshot,
        rules: &mut IgnoreRules,
        inputs: &mut WatchInputs,
        limit: usize,
        mut visit: impl FnMut(&Path) -> notify::Result<()>,
    ) {
        let mut pending: VecDeque<_> = starts.into_iter().collect();
        let mut visited = FxHashSet::default();
        while let Some(dir) = pending.pop_front() {
            if !visited.insert(dir.clone()) || self.dirs.contains(&dir) {
                continue;
            }
            let class = policy.classify(&dir);
            if matches!(class, PathClass::Cache) {
                continue;
            }
            if worktree && dir != policy.workdir {
                if matches!(class, PathClass::Git { .. } | PathClass::ControlEntry)
                    || dir.file_name().is_some_and(|name| name == ".git")
                {
                    continue;
                }
                if rules.failed && rules.matcher.is_none()
                    || dir
                        .strip_prefix(&policy.workdir)
                        .is_ok_and(|relative| rules.is_ignored_rel(relative, Some(true)))
                {
                    self.boundaries.push(dir);
                    continue;
                }
            }
            let count = if worktree {
                self.worktree_dirs.len()
            } else {
                self.dirs.len() - self.worktree_dirs.len()
            };
            if count >= limit {
                if worktree {
                    self.skipped = Some(count + 1);
                } else {
                    self.failures += 1;
                }
                break;
            }
            // This ordering closes the creation gap: an earlier child is found
            // below; a later one generates an event from the registered parent.
            if visit(&dir).is_err() {
                self.failures += 1;
            }
            self.dirs.insert(dir.clone());
            if worktree {
                self.worktree_dirs.insert(dir.clone());
            }
            match fs::read_dir(&dir) {
                Ok(entries) => {
                    let mut children = Vec::new();
                    for entry in entries {
                        match entry {
                            Ok(entry) => {
                                if worktree && entry.file_name() == ".gitignore" {
                                    inputs.add_inputs(vec![entry.path()]);
                                }
                                match entry.file_type() {
                                    Ok(kind) if kind.is_dir() => children.push(entry.path()),
                                    Ok(_) => {}
                                    Err(_) => self.failures += 1,
                                }
                            }
                            Err(_) => self.failures += 1,
                        }
                    }
                    children.sort(); // Stable BFS boundaries for native exclusion priority.
                    pending.extend(children);
                }
                Err(_) => self.failures += 1,
            }
        }
    }

    pub fn outcome(&self, rules: &IgnoreRules, inputs: &WatchInputs) -> WatchSetupOutcome {
        if rules.failed {
            WatchSetupOutcome::PolicyFailed
        } else if let Some(dir_count) = self.skipped {
            WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count }
        } else {
            WatchSetupOutcome::Watching {
                failed_dirs: self.failures + usize::from(inputs.info.discovery_incomplete),
            }
        }
    }
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn native_exclusions(
    root: &Path,
    policy: &PolicySnapshot,
    boundaries: &[PathBuf],
) -> Vec<PathBuf> {
    let mut caches: Vec<_> = policy
        .cache_roots
        .iter()
        .filter(|path| path.as_path() != root && path.starts_with(root))
        .cloned()
        .collect();
    caches.sort_by_key(|path| (path.components().count(), path.clone()));
    let mut exclusions: Vec<PathBuf> = Vec::new();
    for path in caches.into_iter().chain(boundaries.iter().cloned()) {
        if path == root
            || !path.starts_with(root)
            || exclusions.iter().any(|parent| path.starts_with(parent))
        {
            continue;
        }
        // FSEvents exclusions are directories. A missing cache root is still a
        // valid boundary, but index.lock is a file and must only be filtered.
        if path.file_name().is_some_and(|name| name == "index.lock") {
            continue;
        }
        if exclusions.len() == 8 {
            break;
        }
        exclusions.push(path);
    }
    exclusions
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_exclusions_prefer_caches_then_shallowest_boundaries() {
        let root = std::env::temp_dir().join("exclusions");
        let mut policy = PolicySnapshot::default();
        policy.workdir = root.clone();
        policy.cache_roots.extend([
            root.join(".git/objects"),
            root.join(".git/lfs"),
            root.join(".git/index.lock"),
        ]);
        let mut boundaries = vec![root.join("vendor"), root.join("vendor/nested")];
        boundaries.extend((0..20).map(|index| root.join(format!("ignored-{index}"))));
        let paths = native_exclusions(&root, &policy, &boundaries);
        assert_eq!(paths.len(), 8);
        assert!(paths[..2].contains(&root.join(".git/objects")));
        assert!(paths[..2].contains(&root.join(".git/lfs")));
        assert_eq!(paths[2], root.join("vendor"));
        assert!(!paths.contains(&root.join("vendor/nested")));
    }
}
