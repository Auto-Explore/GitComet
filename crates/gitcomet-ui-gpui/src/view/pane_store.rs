//! A pane routes diff messages to its binding; it never owns the window store.
use super::*;
use crate::view::documents::buffers::SaveOrigin;
use gitcomet_extension_api::DiffPanePolicy;
use gitcomet_state::diff_session::{DiffSessionMsg, DiffViewId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffBinding {
    pub repo_id: RepoId,
    pub lifetime: u64,
    pub view: DiffViewId,
}

impl DiffBinding {
    pub(in crate::view) fn repository(self, state: &AppState) -> Option<&RepoState> {
        state.repos.iter().find(|repo| {
            repo.id == self.repo_id
                && repo.lifetime() == self.lifetime
                && repo.diff_sessions.contains_key(&self.view)
        })
    }

    pub(in crate::view) fn diff(
        self,
        repo: &RepoState,
    ) -> Option<&gitcomet_state::model::DiffState> {
        if repo.id != self.repo_id || repo.lifetime() != self.lifetime {
            return None;
        }
        Some(&repo.diff_sessions.get(&self.view)?.diff_state)
    }
}

#[derive(Clone)]
pub(crate) struct PaneStore {
    store: std::sync::Weak<AppStore>,
    read_only: bool,
    filesystem: &'static std::sync::Mutex<gitcomet_core::filesystem::Filesystem>,
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
            filesystem: store.filesystem(),
            store: Arc::downgrade(&store),
            read_only: false,
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
    pub fn repository(
        &self,
        repo_id: RepoId,
        lifetime: u64,
    ) -> Option<Arc<dyn gitcomet_core::services::GitRepository>> {
        self.store.upgrade()?.repository(repo_id, lifetime)
    }

    pub fn read_only() -> Self {
        Self {
            store: std::sync::Weak::new(),
            read_only: true,
            // Never written: a read-only pane saves nothing.
            filesystem: gitcomet_core::filesystem::global(),
            binding: None,
            policy: DiffPanePolicy::read_only(),
            owner_policy: DiffPanePolicy::read_only(),
            linked: false,
        }
    }

    /// The window store's filesystem engine, kept so it outlives the weak store.
    pub fn filesystem(&self) -> &'static std::sync::Mutex<gitcomet_core::filesystem::Filesystem> {
        self.filesystem
    }

    pub fn disable_commands(&mut self) {
        self.read_only = true;
    }
    pub fn bind(&mut self, binding: DiffBinding, policy: DiffPanePolicy) {
        self.binding = Some(binding);
        self.set_policy(policy);
    }

    /// Whether the pane admits a worktree save: none with its commands off,
    /// every save edits, and one that stages is staging too.
    pub(in crate::view) fn admits_save(&self, stage: bool) -> bool {
        !self.read_only && self.policy.allow_edit && (!stage || self.policy.allow_stage)
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

    pub(in crate::view) fn document_save_origin(
        &self,
        repo_id: RepoId,
        lifetime: u64,
    ) -> Option<SaveOrigin> {
        if !self.admits_save(false) {
            return None;
        }
        let state = self.snapshot()?;
        let repo = state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)?;
        if !repo.has_working_tree() || repo.bare_mode_conversion.is_busy() {
            return None;
        }
        let session = if let Some(binding) = self.binding {
            if binding.repo_id != repo_id || binding.lifetime != lifetime {
                return None;
            }
            Some(binding.view)
        } else {
            None
        };
        Some(SaveOrigin::Repository {
            repo_id,
            lifetime,
            session,
        })
    }

    pub fn snapshot(&self) -> Option<Arc<AppState>> {
        self.store.upgrade().map(|store| store.snapshot())
    }

    pub fn dispatch(&self, msg: Msg) {
        if self.read_only {
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
            Msg::ApplyPreparedDiffPatch {
                repo_id: id,
                lifetime: origin,
                view: origin_view,
                file_rev,
                generation,
                prepared,
            } if id == repo_id && origin == lifetime && origin_view == Some(view) && !linked => {
                let allowed = match prepared.operation() {
                    gitcomet_core::patch_action::PatchOperation::Stage
                    | gitcomet_core::patch_action::PatchOperation::Unstage => {
                        self.policy.allow_stage
                    }
                    gitcomet_core::patch_action::PatchOperation::Discard => self.policy.allow_edit,
                };
                if allowed {
                    store.dispatch(Msg::ApplyPreparedDiffPatch {
                        repo_id,
                        lifetime,
                        view: Some(view),
                        file_rev,
                        generation,
                        prepared,
                    });
                }
                return;
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
            msg @ Msg::SaveWorktreeFile {
                repo_id: id, stage, ..
            } if id == repo_id && self.admits_save(stage) && !linked => {
                store.dispatch(msg);
                return;
            }
            // Navigation and unrelated repository actions belong to the owner.
            _ => return,
        };
        store.dispatch(Msg::DiffSession(session));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_store() -> PaneStore {
        let (store, _events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
        PaneStore::from(Arc::new(store))
    }

    #[test]
    fn a_save_that_stages_needs_both_permissions() {
        let store = |allow_edit, allow_stage| {
            let mut policy = DiffPanePolicy::default();
            policy.allow_edit = allow_edit;
            policy.allow_stage = allow_stage;
            let mut store = live_store();
            store.set_policy(policy);
            store
        };
        let admits = |store: PaneStore| [false, true].map(|stage| store.admits_save(stage));
        assert_eq!(admits(store(true, true)), [true, true]);
        assert_eq!(admits(store(true, false)), [true, false]);
        assert_eq!(admits(store(false, true)), [false, false]);
    }

    /// A pane whose commands are off drops every save, whatever its policy.
    #[test]
    fn a_pane_without_commands_admits_no_save() {
        let mut read_only = PaneStore::read_only();
        read_only.set_policy(DiffPanePolicy::default());
        let mut disabled = live_store();
        disabled.disable_commands();
        for store in [read_only, disabled] {
            assert_eq!(
                [false, true].map(|stage| store.admits_save(stage)),
                [false, false]
            );
        }
    }
}
