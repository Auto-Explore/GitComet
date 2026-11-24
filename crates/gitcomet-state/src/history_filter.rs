//! Which commits the history shows: its scope, an author filter and the refs
//! it is focused on.

use crate::model::RepoId;
use crate::msg::Msg;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet, LogScope};

#[derive(Clone, Debug, PartialEq)]
pub enum HistoryFilterMsg {
    SetScope {
        repo_id: RepoId,
        scope: LogScope,
    },
    /// Restricts the history to commits authored by `author` (case-insensitive
    /// match on the author name). `None` clears the filter.
    SetAuthorFilter {
        repo_id: RepoId,
        author: Option<String>,
    },
    /// Focuses the history on a set of refs: the walk is reseeded from them, so
    /// only the commits they reach remain. An empty set clears the focus.
    SetFocus {
        repo_id: RepoId,
        focus: HistoryFocusSet,
    },
    /// Focuses `target`, or unfocuses it when the focus already covers it. The
    /// per-ref menu entry and the sidebar's eye are toggles, and this is what
    /// they send.
    ToggleFocus {
        repo_id: RepoId,
        target: HistoryFocus,
    },
    /// Focuses every one of `targets`, or unfocuses every one of them. A branch
    /// group's menu entries and eye send this with the group's members.
    FocusRefs {
        repo_id: RepoId,
        targets: Vec<HistoryFocus>,
        focused: bool,
    },
}

impl HistoryFilterMsg {
    pub fn repo_id(&self) -> RepoId {
        match self {
            Self::SetScope { repo_id, .. }
            | Self::SetAuthorFilter { repo_id, .. }
            | Self::SetFocus { repo_id, .. }
            | Self::ToggleFocus { repo_id, .. }
            | Self::FocusRefs { repo_id, .. } => *repo_id,
        }
    }
}

impl From<HistoryFilterMsg> for Msg {
    fn from(message: HistoryFilterMsg) -> Self {
        Self::HistoryFilter(message)
    }
}
