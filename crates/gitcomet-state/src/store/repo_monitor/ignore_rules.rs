use super::*;
#[cfg(test)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct IgnoreCacheKey {
    pub rel: PathBuf,
    pub is_dir_hint: Option<bool>,
}

/// One slot per `is_dir_hint` value, keyed by path alone so a probe borrows
/// the event's path (`get(&Path)`) instead of allocating a key per event.
#[derive(Clone, Default)]
pub(super) struct IgnoreCacheSlots {
    by_hint: [Option<CachedIgnoreResult>; 3],
}

impl IgnoreCacheSlots {
    pub(super) fn slot(is_dir_hint: Option<bool>) -> usize {
        match is_dir_hint {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.by_hint.iter().all(Option::is_none)
    }

    pub(super) fn newest_cached_at(&self) -> Option<Instant> {
        self.by_hint
            .iter()
            .flatten()
            .map(|entry| entry.cached_at)
            .max()
    }
}

pub(super) const GITIGNORE_CACHE_MAX_ENTRIES: usize = 4_096;
pub(super) const GITIGNORE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
pub(super) const GITIGNORE_CACHE_PRUNE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct CachedIgnoreResult {
    ignored: bool,
    cached_at: Instant,
}

#[derive(Default)]
pub(super) struct IgnoreRules {
    pub workdir: Option<PathBuf>,
    pub matcher: Option<Box<dyn WorktreeIgnoreMatcher>>,
    pub submodule_matchers: Vec<(PathBuf, Box<dyn WorktreeIgnoreMatcher>)>,
    pub failed: bool,
    pub cache: FxHashMap<PathBuf, IgnoreCacheSlots>,
    last_prune_at: Option<Instant>,
}
impl IgnoreRules {
    pub fn reload(
        &mut self,
        workdir: &Path,
        backend: &dyn GitBackend,
        info: &gitcomet_core::services::RepositoryWatchInfo,
    ) -> bool {
        self.workdir = Some(workdir.to_path_buf());
        let loaded = (|| {
            let matcher = backend.worktree_ignore_matcher(workdir)?;
            let mut children = Vec::new();
            for child in &info.worktrees {
                if child != workdir
                    && let Some(matcher) = backend.worktree_ignore_matcher(child)?
                {
                    children.push((child.clone(), matcher));
                }
            }
            Ok::<_, gitcomet_core::error::Error>((matcher, children))
        })();
        match loaded {
            Ok((matcher, children)) => {
                self.matcher = matcher;
                self.submodule_matchers = children;
                self.failed = false;
                self.cache.clear();
                self.last_prune_at = None;
                true
            }
            Err(error) => {
                self.failed = true;
                record_monitor_failure(MonitorFailureKind::Start, "reload ignore rules", error);
                false // Keep the last valid matchers, including tracked exceptions.
            }
        }
    }

    pub fn any_boundary_unignored(&mut self, boundaries: &[PathBuf]) -> bool {
        let Some(root) = self.workdir.clone() else {
            return false;
        };
        boundaries.iter().any(|path| {
            path.strip_prefix(&root)
                .is_ok_and(|relative| !self.is_ignored_rel(relative, Some(true)))
        })
    }
    fn is_cache_entry_fresh(now: Instant, entry: &CachedIgnoreResult) -> bool {
        now.saturating_duration_since(entry.cached_at) <= GITIGNORE_CACHE_TTL
    }

    pub(super) fn prune_cache_if_due(&mut self, now: Instant) {
        let should_prune = match self.last_prune_at {
            Some(last_prune_at) => {
                now.saturating_duration_since(last_prune_at) >= GITIGNORE_CACHE_PRUNE_INTERVAL
                    || self.cache.len() > GITIGNORE_CACHE_MAX_ENTRIES
            }
            None => true,
        };

        if should_prune {
            self.prune_cache(now);
        }
    }

    pub(super) fn prune_cache(&mut self, now: Instant) {
        self.cache.retain(|_, slots| {
            for entry in &mut slots.by_hint {
                if entry
                    .as_ref()
                    .is_some_and(|entry| !Self::is_cache_entry_fresh(now, entry))
                {
                    *entry = None;
                }
            }
            !slots.is_empty()
        });

        if self.cache.len() > GITIGNORE_CACHE_MAX_ENTRIES {
            // Only the overflow's keys are cloned: the oldest `overflow`
            // entries are selected in place rather than by sorting a copy of
            // every key.
            let overflow = self.cache.len() - GITIGNORE_CACHE_MAX_ENTRIES;
            let mut by_age: Vec<(Instant, &PathBuf)> = self
                .cache
                .iter()
                .map(|(path, slots)| (slots.newest_cached_at().unwrap_or(now), path))
                .collect();
            by_age.select_nth_unstable_by_key(overflow - 1, |(cached_at, _)| *cached_at);
            let evict: Vec<PathBuf> = by_age[..overflow]
                .iter()
                .map(|(_, path)| (*path).clone())
                .collect();
            for path in evict {
                self.cache.remove(&path);
            }
        }

        self.last_prune_at = Some(now);
    }

    pub(super) fn cache_get_at(
        &mut self,
        rel: &Path,
        is_dir_hint: Option<bool>,
        now: Instant,
    ) -> Option<bool> {
        let slot = IgnoreCacheSlots::slot(is_dir_hint);
        let slots = self.cache.get_mut(rel)?;
        let entry = slots.by_hint[slot].as_ref()?;
        let (ignored, fresh) = (entry.ignored, Self::is_cache_entry_fresh(now, entry));

        if !fresh {
            slots.by_hint[slot] = None;
            if slots.is_empty() {
                self.cache.remove(rel);
            }
            return None;
        }

        Some(ignored)
    }

    pub(super) fn cache_insert_at(
        &mut self,
        rel: &Path,
        is_dir_hint: Option<bool>,
        ignored: bool,
        now: Instant,
    ) {
        let slot = IgnoreCacheSlots::slot(is_dir_hint);
        let entry = CachedIgnoreResult {
            ignored,
            cached_at: now,
        };
        match self.cache.get_mut(rel) {
            Some(slots) => slots.by_hint[slot] = Some(entry),
            None => {
                let mut slots = IgnoreCacheSlots::default();
                slots.by_hint[slot] = Some(entry);
                self.cache.insert(rel.to_path_buf(), slots);
            }
        }
        self.prune_cache_if_due(now);
    }

    #[cfg(test)]
    pub(super) fn cache_get(&mut self, key: &IgnoreCacheKey, now: Instant) -> Option<bool> {
        self.cache_get_at(&key.rel, key.is_dir_hint, now)
    }

    #[cfg(test)]
    pub(super) fn cache_insert(&mut self, key: IgnoreCacheKey, ignored: bool, now: Instant) {
        self.cache_insert_at(&key.rel, key.is_dir_hint, ignored, now);
    }

    pub(super) fn cached_ignore_lookup(
        &mut self,
        rel: &Path,
        is_dir_hint: Option<bool>,
        now: Instant,
    ) -> Option<bool> {
        let cached = self.cache_get_at(rel, is_dir_hint, now);
        record_ignore_lookup_cache_outcome(cached.is_some());
        cached
    }

    pub(super) fn resolve_uncached_ignore(
        &mut self,
        rel: &Path,
        is_dir_hint: Option<bool>,
    ) -> gitcomet_core::services::Result<bool> {
        let started_at = Instant::now();
        let kind = match is_dir_hint {
            Some(true) => WorktreePathKind::Directory,
            Some(false) => WorktreePathKind::File,
            None => WorktreePathKind::Unknown,
        };
        let absolute = self.workdir.as_ref().map(|root| root.join(rel));
        let child = absolute.as_ref().and_then(|path| {
            self.submodule_matchers
                .iter_mut()
                .find(|(root, _)| path.starts_with(root))
        });
        let (matcher, relative) = match child {
            Some((root, matcher)) => (
                Some(matcher),
                absolute.as_ref().unwrap().strip_prefix(root).unwrap(),
            ),
            None => (self.matcher.as_mut(), rel),
        };
        let missing_matcher = matcher.is_none();
        // A worktree or child-worktree root is always eligible. gix's ignore
        // stack expects a nonempty relative entry, not the root itself.
        let result = match matcher {
            _ if relative.as_os_str().is_empty() => Ok(false),
            Some(matcher) => matcher.is_ignored(relative, kind),
            // No matcher available — treat as not-ignored.
            None => Ok(false),
        };
        record_ignore_lookup_latency(started_at.elapsed(), missing_matcher || result.is_err());
        result
    }

    pub(super) fn is_ignored_rel(&mut self, rel: &Path, is_dir_hint: Option<bool>) -> bool {
        if self.workdir.is_none() {
            return false;
        }

        let now = Instant::now();
        self.prune_cache_if_due(now);

        if let Some(ignored) = self.cached_ignore_lookup(rel, is_dir_hint, now) {
            return ignored;
        }

        let ignored = match self.resolve_uncached_ignore(rel, is_dir_hint) {
            Ok(ignored) => ignored,
            Err(error) => {
                if !self.failed {
                    record_monitor_failure(MonitorFailureKind::Start, "ignore path lookup", error);
                }
                self.failed = true;
                // Suppress this uncertain path until recovery, but never cache
                // an error as a successful ignore decision for the cache TTL.
                return true;
            }
        };
        self.cache_insert_at(rel, is_dir_hint, ignored, now);
        ignored
    }
}
