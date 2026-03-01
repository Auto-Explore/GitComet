//! Conflict marker label formatting helpers.
//!
//! These helpers model the base-label variants used by git conflict markers
//! and provide deterministic string formatting for merge output.

use std::path::PathBuf;

/// Default short SHA width for marker labels (matches git default behavior).
pub const DEFAULT_SHORT_SHA_LEN: usize = 7;

/// Base-side label scenario used when formatting conflict markers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaseLabelScenario {
    /// No merge base exists (for example, add/add).
    NoBase,
    /// A unique merge base commit and path are known.
    UniqueBase { commit_id: String, path: PathBuf },
    /// A unique merge base is known, but the path was renamed.
    UniqueBaseRename {
        commit_id: String,
        original_path: PathBuf,
    },
    /// Multiple merge bases were merged into a virtual ancestor.
    MergedCommonAncestors { path: PathBuf },
    /// Rebase-style marker label that points at parent context.
    RebaseParent { description: String },
}

impl BaseLabelScenario {
    /// Format the base-side marker label text.
    pub fn format_label(&self) -> String {
        match self {
            Self::NoBase => "empty tree".to_string(),
            Self::UniqueBase { commit_id, path } => {
                format!("{}:{}", short_commit_id(commit_id), format_git_path(path))
            }
            Self::UniqueBaseRename {
                commit_id,
                original_path,
            } => format!(
                "{}:{}",
                short_commit_id(commit_id),
                format_git_path(original_path)
            ),
            Self::MergedCommonAncestors { path } => {
                format!("merged common ancestors:{}", format_git_path(path))
            }
            Self::RebaseParent { description } => format!("parent of {}", description.trim()),
        }
    }
}

/// Format base-side conflict marker label text.
pub fn format_base_label(scenario: &BaseLabelScenario) -> String {
    scenario.format_label()
}

fn short_commit_id(commit_id: &str) -> String {
    let trimmed = commit_id.trim();
    trimmed.chars().take(DEFAULT_SHORT_SHA_LEN).collect()
}

fn format_git_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_commit_id_truncates_to_default_width() {
        assert_eq!(short_commit_id("0123456789"), "0123456");
    }

    #[test]
    fn short_commit_id_keeps_short_inputs() {
        assert_eq!(short_commit_id("abc"), "abc");
    }
}
