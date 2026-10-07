//! Preferences shared by all worktrees of one Git repository.

use gitcomet_core::domain::HistoryMode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RepositoryKey {
    CommonDir(PathBuf),
    Worktree(PathBuf),
}

impl RepositoryKey {
    pub fn storage_key(&self) -> String {
        let (prefix, path) = match self {
            Self::CommonDir(path) => ("common:", path),
            Self::Worktree(path) => ("worktree:", path),
        };
        format!("{prefix}{}", crate::session::path_storage_key(path))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryListKind {
    CommitFiles,
    RangeFiles,
    WorktreeFiles,
    CombinedUnstaged,
    Untracked,
    Unstaged,
    Staged,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryFileSort {
    #[default]
    PathAscending,
    PathDescending,
    FileTypeAscending,
    FileTypeDescending,
    EditSizeAscending,
    EditSizeDescending,
    Edits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SharedRepositoryPreferences {
    #[serde(with = "history_mode_serde")]
    pub history_mode: HistoryMode,
    pub pinned_items: BTreeSet<String>,
    pub collapsed_items: BTreeSet<String>,
    pub file_sorts: BTreeMap<RepositoryListKind, RepositoryFileSort>,
    pub show_hidden_files: bool,
    pub show_ignored_files: bool,
}

impl Default for SharedRepositoryPreferences {
    fn default() -> Self {
        Self {
            history_mode: HistoryMode::default(),
            pinned_items: BTreeSet::new(),
            collapsed_items: BTreeSet::new(),
            file_sorts: BTreeMap::new(),
            show_hidden_files: true,
            show_ignored_files: false,
        }
    }
}

/// Updates contain only the user's changes, never a stale window's whole map.
#[derive(Clone, Debug)]
pub enum RepositoryPreferenceUpdate {
    HistoryMode(HistoryMode),
    Pin {
        key: String,
        pinned: bool,
    },
    RemovePins(BTreeSet<String>),
    CollapseItems {
        added: BTreeSet<String>,
        removed: BTreeSet<String>,
    },
    FileSort {
        list: RepositoryListKind,
        sort: RepositoryFileSort,
    },
    /// `None` leaves the other flag unchanged, including across stale menus.
    ExplorerVisibility {
        hidden: Option<bool>,
        ignored: Option<bool>,
    },
}

impl SharedRepositoryPreferences {
    pub fn apply(&mut self, update: &RepositoryPreferenceUpdate) {
        match update {
            RepositoryPreferenceUpdate::HistoryMode(mode) => self.history_mode = *mode,
            RepositoryPreferenceUpdate::Pin { key, pinned } => {
                if *pinned {
                    self.pinned_items.insert(key.clone());
                } else {
                    self.pinned_items.remove(key);
                }
            }
            RepositoryPreferenceUpdate::RemovePins(keys) => {
                self.pinned_items.retain(|key| !keys.contains(key))
            }
            RepositoryPreferenceUpdate::CollapseItems { added, removed } => {
                self.collapsed_items.retain(|key| !removed.contains(key));
                self.collapsed_items.extend(added.iter().cloned());
            }
            RepositoryPreferenceUpdate::FileSort { list, sort } => {
                self.file_sorts.insert(*list, *sort);
            }
            RepositoryPreferenceUpdate::ExplorerVisibility { hidden, ignored } => {
                if let Some(hidden) = hidden {
                    self.show_hidden_files = *hidden;
                }
                if let Some(ignored) = ignored {
                    self.show_ignored_files = *ignored;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RepositoryPreferencesSnapshot {
    pub key: RepositoryKey,
    pub revision: u64,
    pub preferences: Arc<SharedRepositoryPreferences>,
}

mod history_mode_serde {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        mode: &HistoryMode,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let name = match mode {
            HistoryMode::FullReachable => "full_reachable",
            HistoryMode::FirstParent => "first_parent",
            HistoryMode::NoMerges => "no_merges",
            HistoryMode::MergesOnly => "merges_only",
            HistoryMode::AllBranches => "all_branches",
        };
        serializer.serialize_str(name)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HistoryMode, D::Error> {
        let name = String::deserialize(deserializer)?;
        match name.as_str() {
            "full_reachable" => Ok(HistoryMode::FullReachable),
            "first_parent" => Ok(HistoryMode::FirstParent),
            "no_merges" => Ok(HistoryMode::NoMerges),
            "merges_only" => Ok(HistoryMode::MergesOnly),
            "all_branches" => Ok(HistoryMode::AllBranches),
            _ => Err(serde::de::Error::custom("unknown history mode")),
        }
    }
}
