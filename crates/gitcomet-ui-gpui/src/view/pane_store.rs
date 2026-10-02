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
    /// What the pane lets the user do now: the owner's policy, with staging
    /// and editing off while it shows a linked worktree. Set it with
    /// [`Self::set_policy`].
    pub policy: DiffPanePolicy,
    owner_policy: DiffPanePolicy,
    linked: bool,
}

impl From<Arc<AppStore>> for PaneStore {
    fn from(store: Arc<AppStore>) -> Self {
        Self {
            store: Arc::downgrade(&store),
            snapshot: None,
            binding: None,
            policy: DiffPanePolicy::default(),
            owner_policy: DiffPanePolicy::default(),
            linked: false,
        }
    }
}

/// The linked worktree a bound session reads, if any.
fn bound_worktree(state: &AppState, binding: DiffBinding) -> Option<std::path::PathBuf> {
    state
        .repos
        .iter()
        .find(|repo| repo.id == binding.repo_id && repo.lifetime() == binding.lifetime)?
        .diff_sessions
        .get(&binding.view)?
        .worktree()
        .map(std::path::Path::to_path_buf)
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
        self.set_policy(policy);
    }

    pub fn set_policy(&mut self, policy: DiffPanePolicy) {
        self.owner_policy = policy;
        self.apply_policy();
    }

    /// Follows the bound session onto or off a linked worktree. Staging and
    /// saving act on the main checkout, so a linked file offers neither.
    pub fn set_linked(&mut self, linked: bool) {
        self.linked = linked;
        self.apply_policy();
    }

    fn apply_policy(&mut self) {
        self.policy = self.owner_policy;
        if self.linked {
            self.policy.allow_stage = false;
            self.policy.allow_edit = false;
        }
    }

    /// Whether `state`'s bound session reads a linked worktree.
    pub fn shows_linked_worktree(&self, state: &AppState) -> bool {
        self.binding
            .is_some_and(|binding| bound_worktree(state, binding).is_some())
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
                // A linked worktree's file: reads resolve against its checkout,
                // and nothing is judged by the main checkout's status.
                if let Some(worktree) = bound_worktree(&state, binding) {
                    repo.spec.workdir = worktree;
                    repo.status = Loadable::NotLoaded;
                    repo.worktree_status = Loadable::NotLoaded;
                    repo.staged_status = Loadable::NotLoaded;
                }
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
        let live = store.snapshot();
        if !live
            .repos
            .iter()
            .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
        {
            return;
        }
        // Checked against the live session, not only the drawn policy.
        let worktree = bound_worktree(&live, binding);
        let linked = worktree.is_some();
        // The renderer builds working-tree targets for the main checkout; a
        // linked session keeps them in its own worktree.
        let in_session_worktree = |target: DiffTarget| match &worktree {
            Some(path) if target.worktree().is_none() => target.in_worktree(path.clone()),
            _ => target,
        };
        let session = match msg {
            Msg::SelectDiff {
                repo_id: id,
                target,
            } if id == repo_id => DiffSessionMsg::Open {
                repo_id,
                lifetime,
                view,
                target: in_session_worktree(target),
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
                if id == repo_id && self.policy.allow_edit && !linked =>
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
                        in_session_worktree(DiffTarget::working_tree(path, DiffArea::Unstaged))
                    }
                    gitcomet_core::domain::FileSource::Commit(commit) => {
                        DiffTarget::commit(commit, path)
                    }
                    gitcomet_core::domain::FileSource::Branch(branch) => {
                        DiffTarget::commit(gitcomet_core::domain::CommitId(branch.into()), path)
                    }
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
                if id == repo_id && self.policy.allow_stage && !linked =>
            {
                store.dispatch(msg);
                return;
            }
            msg @ Msg::SaveWorktreeFile { repo_id: id, .. }
                if id == repo_id && self.policy.allow_edit && !linked =>
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
