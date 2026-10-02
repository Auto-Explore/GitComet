use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The most worktree paths one change reports; past it the change says
/// [`ChangedPaths::Unknown`].
pub const MAX_CHANGED_PATHS: usize = 256;

/// Which worktree paths (relative to the workdir) a change touched.
///
/// `Unknown` means "anything may have changed": the watcher overflowed its
/// bound, saw an event it could not attribute (a rescan, an ignore-rule or
/// control-file change), or the change did not come from the watcher at all
/// (a window activation). Callers must refresh broadly then; a list is never
/// silently truncated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChangedPaths {
    #[default]
    Unknown,
    Known(Arc<[PathBuf]>),
}

impl ChangedPaths {
    /// Sorted and deduplicated; `Unknown` past [`MAX_CHANGED_PATHS`].
    pub fn known(mut paths: Vec<PathBuf>) -> Self {
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_CHANGED_PATHS {
            Self::Unknown
        } else {
            Self::Known(paths.into())
        }
    }

    pub fn none() -> Self {
        Self::Known(Arc::from([]))
    }

    pub fn paths(&self) -> Option<&[PathBuf]> {
        match self {
            Self::Unknown => None,
            Self::Known(paths) => Some(paths),
        }
    }

    /// Whether `path` (relative) may have changed: always, when unknown. A
    /// known path covers everything below it, since a moved or removed
    /// directory is reported as one path with no event per file inside.
    pub fn may_contain(&self, path: &Path) -> bool {
        match self {
            Self::Unknown => true,
            Self::Known(paths) => path.ancestors().any(|ancestor| {
                paths
                    .binary_search_by(|known| known.as_path().cmp(ancestor))
                    .is_ok()
            }),
        }
    }

    /// Both changes' paths; unknown if either is, or past the bound.
    pub fn merge(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => {
                if b.is_empty() {
                    return self.clone();
                }
                if a.is_empty() {
                    return other.clone();
                }
                Self::known(a.iter().chain(b.iter()).cloned().collect())
            }
            _ => Self::Unknown,
        }
    }
}

/// Equality compares `paths` only when `worktree` is set: they describe the
/// worktree part of a change and mean nothing without it.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct RepoExternalChange {
    pub worktree: bool,
    pub index: bool,
    pub git_state: bool,
    pub tags: bool,
    /// Attributes changed; ordinary worktree edits do not need support detection.
    pub large_file_support: bool,
    /// Configuration/watch-policy inputs changed. A routine full refresh does
    /// not imply a changed verification context.
    pub verification_context: bool,
    /// Attribute inputs changed; independent of ordinary content/index edits.
    pub text_attributes: bool,
    /// The worktree paths behind `worktree`, when the watcher knows them.
    pub paths: ChangedPaths,
}

impl PartialEq for RepoExternalChange {
    fn eq(&self, other: &Self) -> bool {
        self.worktree == other.worktree
            && self.index == other.index
            && self.git_state == other.git_state
            && self.tags == other.tags
            && self.verification_context == other.verification_context
            && self.text_attributes == other.text_attributes
            && (!self.worktree || self.paths == other.paths)
    }
}

impl Eq for RepoExternalChange {}

impl RepoExternalChange {
    #[allow(non_upper_case_globals)]
    pub const Worktree: Self = Self::worktree();

    /// Every kind of change either side reports.
    pub fn union(self, other: Self) -> Self {
        Self {
            worktree: self.worktree || other.worktree,
            index: self.index || other.index,
            git_state: self.git_state || other.git_state,
            tags: self.tags || other.tags,
            large_file_support: self.large_file_support || other.large_file_support,
            verification_context: self.verification_context || other.verification_context,
            text_attributes: self.text_attributes || other.text_attributes,
            paths: self.paths.merge(&other.paths),
        }
    }

    #[allow(non_upper_case_globals)]
    pub const Index: Self = Self::index();
    #[allow(non_upper_case_globals)]
    pub const GitState: Self = Self::git_state();
    #[allow(non_upper_case_globals)]
    pub const Both: Self = Self::all();

    pub const fn worktree() -> Self {
        Self {
            worktree: true,
            index: false,
            git_state: false,
            tags: false,
            large_file_support: false,
            verification_context: false,
            text_attributes: false,
            paths: ChangedPaths::Unknown,
        }
    }

    pub const fn index() -> Self {
        Self {
            worktree: false,
            index: true,
            git_state: false,
            tags: false,
            large_file_support: false,
            verification_context: false,
            text_attributes: false,
            paths: ChangedPaths::Unknown,
        }
    }

    pub const fn git_state() -> Self {
        Self {
            worktree: false,
            index: false,
            git_state: true,
            tags: false,
            large_file_support: false,
            verification_context: false,
            text_attributes: false,
            paths: ChangedPaths::Unknown,
        }
    }

    pub const fn all() -> Self {
        Self {
            worktree: true,
            index: true,
            git_state: true,
            tags: true,
            large_file_support: true,
            verification_context: false,
            text_attributes: true,
            paths: ChangedPaths::Unknown,
        }
    }

    pub const fn is_empty(&self) -> bool {
        !self.worktree
            && !self.index
            && !self.git_state
            && !self.tags
            && !self.verification_context
            && !self.large_file_support
            && !self.text_attributes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_paths_are_sorted_bounded_and_merge_toward_unknown() {
        let known = ChangedPaths::known(vec!["b".into(), "a".into(), "b".into()]);
        assert_eq!(
            known.paths(),
            Some(&[PathBuf::from("a"), PathBuf::from("b")][..])
        );
        assert!(known.may_contain(Path::new("a")));
        assert!(!known.may_contain(Path::new("c")));
        assert!(ChangedPaths::Unknown.may_contain(Path::new("c")));
        // `mv src lib` is two directory paths; the files inside get no event.
        let moved = ChangedPaths::known(vec!["lib".into(), "src".into()]);
        assert!(moved.may_contain(Path::new("src/main.rs")));
        assert!(moved.may_contain(Path::new("lib/deep/mod.rs")));
        assert!(!moved.may_contain(Path::new("srcs/main.rs")));
        assert!(!moved.may_contain(Path::new("other.rs")));

        let too_many = (0..=MAX_CHANGED_PATHS)
            .map(|n| PathBuf::from(format!("f{n}")))
            .collect();
        assert_eq!(
            ChangedPaths::known(too_many),
            ChangedPaths::Unknown,
            "past the bound a list is never truncated"
        );

        assert_eq!(known.merge(&ChangedPaths::Unknown), ChangedPaths::Unknown);
        assert_eq!(ChangedPaths::none().merge(&known), known);
        let half: Vec<PathBuf> = (0..MAX_CHANGED_PATHS / 2 + 1)
            .map(|n| PathBuf::from(format!("x{n}")))
            .collect();
        let other: Vec<PathBuf> = (0..MAX_CHANGED_PATHS / 2 + 1)
            .map(|n| PathBuf::from(format!("y{n}")))
            .collect();
        assert_eq!(
            ChangedPaths::known(half).merge(&ChangedPaths::known(other)),
            ChangedPaths::Unknown,
            "a merge past the bound is unknown"
        );
        // Changes built without paths claim to know nothing.
        assert_eq!(RepoExternalChange::worktree().paths, ChangedPaths::Unknown);
    }
}
