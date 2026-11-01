use gitcomet_core::domain::DiffArea;
use gitcomet_state::diff_session::DiffViewId;
use gitcomet_state::model::{AppState, DiffState, RepoId, RepoLifetime, RepoState};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct RepositoryBinding {
    repo_id: RepoId,
    lifetime: RepoLifetime,
    session: Option<DiffViewId>,
}

impl RepositoryBinding {
    fn repository(self, state: &AppState) -> Option<&RepoState> {
        let repo = state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id && repo.lifetime_id() == self.lifetime)?;
        if self
            .session
            .is_some_and(|view| !repo.diff_sessions.contains_key(&view))
        {
            return None;
        }
        Some(repo)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum DiffSourceId {
    Repository(RepoId),
    Snapshot,
}

impl From<RepoId> for DiffSourceId {
    fn from(value: RepoId) -> Self {
        Self::Repository(value)
    }
}

#[derive(Clone)]
pub(in crate::view) enum DiffPaneInput {
    Repository {
        state: Arc<AppState>,
        binding: Option<RepositoryBinding>,
    },
    Snapshot {
        state: Arc<DiffState>,
        path: PathBuf,
    },
}

impl DiffPaneInput {
    pub fn selected(state: Arc<AppState>) -> Self {
        let binding = state.active_repo.and_then(|id| {
            state
                .repos
                .iter()
                .find(|repo| repo.id == id)
                .map(|repo| RepositoryBinding {
                    repo_id: repo.id,
                    lifetime: repo.lifetime_id(),
                    session: None,
                })
        });
        Self::Repository { state, binding }
    }

    pub fn session(state: Arc<AppState>, binding: crate::view::pane_store::DiffBinding) -> Self {
        let checked = binding.repository(&state).map(|repo| RepositoryBinding {
            repo_id: repo.id,
            lifetime: repo.lifetime_id(),
            session: Some(binding.view),
        });
        Self::Repository {
            state,
            binding: checked,
        }
    }

    pub fn repository_state(&self) -> Option<&Arc<AppState>> {
        match self {
            Self::Repository { state, .. } => Some(state),
            Self::Snapshot { .. } => None,
        }
    }

    pub fn repository(&self) -> Option<&RepoState> {
        match self {
            Self::Repository { state, binding } => (*binding)?.repository(state),
            Self::Snapshot { .. } => None,
        }
    }

    pub fn diff(&self) -> Option<&DiffState> {
        match self {
            Self::Repository { binding, .. } => {
                let repo = self.repository()?;
                match (*binding)?.session {
                    Some(view) => Some(&repo.diff_sessions.get(&view)?.diff_state),
                    None => Some(&repo.diff_state),
                }
            }
            Self::Snapshot { state, .. } => Some(state),
        }
    }

    pub fn source_id(&self) -> Option<DiffSourceId> {
        match self {
            Self::Repository { .. } => self
                .repository()
                .map(|repo| DiffSourceId::Repository(repo.id)),
            Self::Snapshot { .. } => Some(DiffSourceId::Snapshot),
        }
    }

    pub fn workdir(&self) -> Option<&Path> {
        match self {
            Self::Repository { binding, .. } => {
                let repo = self.repository()?;
                Some(
                    (*binding)?
                        .session
                        .and_then(|view| repo.diff_sessions.get(&view))
                        .and_then(|session| session.worktree())
                        .unwrap_or(&repo.spec.workdir),
                )
            }
            // A snapshot already supplies its complete path relative to the current directory.
            Self::Snapshot { .. } => Some(Path::new("")),
        }
    }

    /// Status belongs to the main checkout; a linked session does not share it.
    pub fn status_entry_for_path(
        &self,
        area: DiffArea,
        path: &Path,
    ) -> Option<&gitcomet_core::domain::FileStatus> {
        let repo = self.repository()?;
        if self.shows_linked_worktree() {
            return None;
        }
        repo.status_entry_for_path(area, path)
    }

    pub fn shows_linked_worktree(&self) -> bool {
        let Self::Repository {
            binding: Some(binding),
            ..
        } = self
        else {
            return false;
        };
        self.repository()
            .and_then(|repo| repo.diff_sessions.get(&binding.session?))
            .is_some_and(|session| session.worktree().is_some())
    }

    pub fn same_snapshot(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Repository {
                    state: a,
                    binding: a_binding,
                },
                Self::Repository {
                    state: b,
                    binding: b_binding,
                },
            ) => a_binding == b_binding && Arc::ptr_eq(a, b),
            (
                Self::Snapshot {
                    state: a,
                    path: a_path,
                },
                Self::Snapshot {
                    state: b,
                    path: b_path,
                },
            ) => a_path == b_path && Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::DiffRenderer;
    use super::*;

    #[test]
    fn repository_input_keeps_the_original_snapshot_and_checks_its_lifetime() {
        let repo = RepoState::new_opening(
            RepoId(71),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/repository"),
            },
        );
        let state = Arc::new(AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..AppState::test_default()
        });
        let input = DiffPaneInput::selected(state.clone());
        assert!(Arc::ptr_eq(input.repository_state().unwrap(), &state));
        assert!(std::ptr::eq(input.repository().unwrap(), &state.repos[0]));
        assert!(std::ptr::eq(
            input.diff().unwrap(),
            &state.repos[0].diff_state
        ));
        let DiffPaneInput::Repository {
            binding: Some(binding),
            ..
        } = input
        else {
            panic!("selected repository")
        };
        let replacement = RepoState::new_opening(binding.repo_id, state.repos[0].spec.clone());
        let reopened = Arc::new(AppState {
            active_repo: Some(replacement.id),
            repos: vec![replacement],
            ..AppState::test_default()
        });
        let stale = DiffPaneInput::Repository {
            state: reopened,
            binding: Some(binding),
        };
        assert!(stale.repository().is_none());
        assert!(stale.diff().is_none());
        assert!(stale.workdir().is_none());
        let closed_session = DiffPaneInput::Repository {
            state,
            binding: Some(RepositoryBinding {
                session: Some(DiffViewId::next()),
                ..binding
            }),
        };
        assert!(closed_session.repository().is_none());
        assert!(closed_session.diff().is_none());
    }

    #[test]
    fn snapshot_identity_includes_its_path_and_revision_without_a_repository() {
        let state = Arc::new(DiffState::default());
        let first = DiffPaneInput::Snapshot {
            state: state.clone(),
            path: "first.rs".into(),
        };
        let second = DiffPaneInput::Snapshot {
            state: state.clone(),
            path: "second.rs".into(),
        };
        assert!(first.repository_state().is_none());
        assert!(first.repository().is_none());
        assert!(std::ptr::eq(first.diff().unwrap(), state.as_ref()));
        assert!(!first.same_snapshot(&second));
        assert_ne!(
            DiffRenderer::notify_fingerprint_for(&first),
            DiffRenderer::notify_fingerprint_for(&second)
        );
        let changed = DiffPaneInput::Snapshot {
            state: Arc::new(DiffState {
                diff_file_rev: 1,
                ..Default::default()
            }),
            path: "first.rs".into(),
        };
        assert_ne!(
            DiffRenderer::notify_fingerprint_for(&first),
            DiffRenderer::notify_fingerprint_for(&changed)
        );
    }
}
