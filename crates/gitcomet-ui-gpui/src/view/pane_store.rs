//! A pane routes diff messages to its binding; it never owns the window store.
use super::*;
use gitcomet_extension_api::DiffPanePolicy;
use gitcomet_state::diff_session::{DiffSessionMsg, DiffViewId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffBinding {
    pub repo_id: RepoId,
    pub lifetime: u64,
    pub view: DiffViewId,
}

#[derive(Clone)]
pub(crate) struct PaneStore {
    store: std::sync::Weak<AppStore>,
    snapshot: Option<std::rc::Rc<std::cell::RefCell<Arc<AppState>>>>,
    pub binding: Option<DiffBinding>,
    pub policy: DiffPanePolicy,
}

impl From<Arc<AppStore>> for PaneStore {
    fn from(store: Arc<AppStore>) -> Self {
        Self {
            store: Arc::downgrade(&store),
            snapshot: None,
            binding: None,
            policy: DiffPanePolicy::default(),
        }
    }
}

impl PaneStore {
    pub fn bind_snapshot(
        &mut self,
        state: Arc<AppState>,
        view: DiffViewId,
        policy: DiffPanePolicy,
    ) {
        let repo = &state.repos[0];
        self.bind(
            DiffBinding {
                repo_id: repo.id,
                lifetime: repo.lifetime(),
                view,
            },
            policy,
        );
        self.snapshot = Some(std::rc::Rc::new(std::cell::RefCell::new(state)));
    }

    pub fn replace_snapshot(&self, state: Arc<AppState>) {
        if let Some(snapshot) = &self.snapshot {
            *snapshot.borrow_mut() = state;
        }
    }
    #[cfg(test)]
    pub(crate) fn store_for_test(&self) -> Arc<AppStore> {
        self.store.upgrade().expect("window store")
    }
    pub fn bind(&mut self, binding: DiffBinding, policy: DiffPanePolicy) {
        self.binding = Some(binding);
        self.policy = policy;
    }

    pub fn snapshot(&self) -> Arc<AppState> {
        if let Some(snapshot) = &self.snapshot {
            return snapshot.borrow().clone();
        }
        let state = self
            .store
            .upgrade()
            .map(|store| store.snapshot())
            .unwrap_or_default();
        self.project(state)
    }

    pub fn project(&self, state: Arc<AppState>) -> Arc<AppState> {
        if let Some(snapshot) = &self.snapshot {
            return snapshot.borrow().clone();
        }
        let Some(binding) = self.binding else {
            return state;
        };
        let mut projected = (*state).clone();
        let repo = state
            .repos
            .iter()
            .find(|repo| repo.id == binding.repo_id && repo.lifetime() == binding.lifetime);
        projected.repos = repo
            .map(|repo| {
                let mut repo = repo.clone();
                repo.diff_state = repo
                    .diff_sessions
                    .get(&binding.view)
                    .map(|session| session.diff_state.clone())
                    .unwrap_or_default();
                // A bound pane never enters History's interactive editors or foreign diff.
                repo.interactive_rebase_setup = None;
                repo.interactive_cherry_pick_setup = None;
                vec![repo]
            })
            .unwrap_or_default();
        projected.active_repo = repo.map(|repo| repo.id);
        Arc::new(projected)
    }

    pub fn dispatch(&self, msg: Msg) {
        if self.snapshot.is_some() {
            return;
        }
        let Some(store) = self.store.upgrade() else {
            return;
        };
        let Some(binding) = self.binding else {
            store.dispatch(msg);
            return;
        };
        let DiffBinding {
            repo_id,
            lifetime,
            view,
        } = binding;
        if !store
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
        {
            return;
        }
        let session = match msg {
            Msg::SelectDiff {
                repo_id: id,
                target,
            } if id == repo_id => DiffSessionMsg::Open {
                repo_id,
                lifetime,
                view,
                target,
            },
            Msg::ClearDiffSelection { repo_id: id }
                if id == repo_id && self.policy.close_button =>
            {
                DiffSessionMsg::Clear {
                    repo_id,
                    lifetime,
                    view,
                }
            }
            Msg::SetTextOverride {
                repo_id: id,
                path,
                value,
            } if id == repo_id => DiffSessionMsg::SetTextOverride {
                repo_id,
                lifetime,
                view,
                path,
                value,
            },
            Msg::LoadBlame { repo_id: id, .. } if id == repo_id && self.policy.allow_annotate => {
                DiffSessionMsg::LoadBlame {
                    repo_id,
                    lifetime,
                    view,
                }
            }
            Msg::OpenFileEditor { repo_id: id, path }
                if id == repo_id && self.policy.allow_edit =>
            {
                DiffSessionMsg::OpenEditor {
                    repo_id,
                    lifetime,
                    view,
                    path,
                }
            }
            Msg::ExitDiffEditMode { repo_id: id } if id == repo_id => DiffSessionMsg::ExitEditor {
                repo_id,
                lifetime,
                view,
            },
            Msg::OpenFileContent {
                repo_id: id,
                source,
                path,
            } if id == repo_id => {
                let target = match source {
                    gitcomet_core::domain::FileSource::WorkingDirectory => {
                        DiffTarget::working_tree(path, DiffArea::Unstaged)
                    }
                    gitcomet_core::domain::FileSource::Commit(commit) => {
                        DiffTarget::commit(commit, Some(path))
                    }
                    gitcomet_core::domain::FileSource::Branch(branch) => DiffTarget::commit(
                        gitcomet_core::domain::CommitId(branch.into()),
                        Some(path),
                    ),
                };
                store.dispatch(Msg::DiffSession(DiffSessionMsg::Open {
                    repo_id,
                    lifetime,
                    view,
                    target,
                }));
                DiffSessionMsg::SetContentMode {
                    repo_id,
                    lifetime,
                    view,
                    preview: true,
                    edit: false,
                }
            }
            msg @ (Msg::StagePath { repo_id: id, .. }
            | Msg::UnstagePath { repo_id: id, .. }
            | Msg::StagePaths { repo_id: id, .. }
            | Msg::UnstagePaths { repo_id: id, .. }
            | Msg::StageHunk { repo_id: id, .. }
            | Msg::UnstageHunk { repo_id: id, .. })
                if id == repo_id && self.policy.allow_stage =>
            {
                store.dispatch(msg);
                return;
            }
            msg @ Msg::SaveWorktreeFile { repo_id: id, .. }
                if id == repo_id && self.policy.allow_edit =>
            {
                store.dispatch(msg);
                return;
            }
            // Navigation and unrelated repository actions belong to the owner.
            _ => return,
        };
        store.dispatch(Msg::DiffSession(session));
    }
}
