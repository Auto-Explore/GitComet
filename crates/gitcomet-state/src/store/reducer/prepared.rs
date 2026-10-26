use super::{diff_selection, util};
use crate::model::{
    AppState, AuthRetryOperation, GitOperationOuterOutcome, Loadable, RepoId, RepoLifetime,
    RepoState, ViewNavDir,
};
use crate::msg::{InternalMsg, Msg, RepoActionKind, RepoCommandKind, RepoExternalChange};
use crate::session::RepoSessionPreferences;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{DiffArea, DiffTarget, FileStatusKind};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::GitRepository;
use smallvec::SmallVec;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

pub(in crate::store) struct PreparedReduction {
    message: Msg,
    inputs: ReducerInputs,
}

enum SavedPreferences {
    Unneeded,
    Loaded(Box<RepoSessionPreferences>),
}

struct PathFact<T> {
    lifetime: RepoLifetime,
    path: PathBuf,
    value: T,
}

pub(in crate::store) struct ReducerInputs {
    now: SystemTime,
    preferences: SavedPreferences,
    head: SmallVec<[PathFact<Option<bool>>; 1]>,
    markers: SmallVec<[PathFact<bool>; 1]>,
    auth_worktree_path: Option<PathBuf>,
}

impl ReducerInputs {
    pub(super) fn normalize_auth_retry(&self, state: &AppState, message: &mut Msg) {
        if !state.git_runtime.is_available() {
            return;
        }
        if let Msg::RemoveWorktree { repo_id, path } | Msg::ForceRemoveWorktree { repo_id, path } =
            message
            && state.repos.iter().any(|repo| repo.id == *repo_id)
        {
            *path = self
                .auth_worktree_path
                .as_ref()
                .expect("auth retry worktree path was not prepared")
                .clone();
        }
    }

    pub(super) fn now(&self) -> SystemTime {
        self.now
    }

    pub(super) fn session_preferences(&self) -> &RepoSessionPreferences {
        match &self.preferences {
            SavedPreferences::Loaded(preferences) => preferences,
            SavedPreferences::Unneeded => panic!("repository preferences were not prepared"),
        }
    }

    pub(super) fn head_gitlink(&self, repo: &RepoState, path: &Path) -> Option<bool> {
        self.head
            .iter()
            .find(|fact| fact.lifetime == repo.lifetime_id() && fact.path == path)
            .expect("required HEAD classification was not prepared")
            .value
    }

    pub(super) fn worktree_marker(&self, repo: &RepoState, path: &Path) -> bool {
        self.markers
            .iter()
            .find(|fact| fact.lifetime == repo.lifetime_id() && fact.path == path)
            .expect("required worktree marker classification was not prepared")
            .value
    }
}

#[derive(Default)]
struct InputQueries {
    needs_preferences: bool,
    head: SmallVec<[HeadQuery; 1]>,
    markers: SmallVec<[PathFact<PathBuf>; 1]>,
}

struct HeadQuery {
    lifetime: RepoLifetime,
    path: PathBuf,
    repository: Option<Arc<dyn GitRepository>>,
}

impl InputQueries {
    fn resolve(self, now: SystemTime) -> ReducerInputs {
        ReducerInputs {
            now,
            auth_worktree_path: None,
            preferences: if self.needs_preferences {
                SavedPreferences::Loaded(Box::new(crate::session::load_repo_session_preferences()))
            } else {
                SavedPreferences::Unneeded
            },
            head: self
                .head
                .into_iter()
                .map(|fact| {
                    let value = fact
                        .repository
                        .and_then(|repo| repo.head_path_is_gitlink(&fact.path).ok());
                    PathFact {
                        lifetime: fact.lifetime,
                        path: fact.path,
                        value,
                    }
                })
                .collect(),
            markers: self
                .markers
                .into_iter()
                .map(|fact| {
                    let value = std::fs::metadata(fact.value)
                        .is_ok_and(|metadata| metadata.is_file() || metadata.is_dir());
                    PathFact {
                        lifetime: fact.lifetime,
                        path: fact.path,
                        value,
                    }
                })
                .collect(),
        }
    }

    fn load_preferences(&mut self) {
        self.needs_preferences = true;
    }

    fn head(
        &mut self,
        repos: &RepositoryRuntimes,
        repo: &RepoState,
        target: &DiffTarget,
        status_will_reset: bool,
    ) {
        let DiffTarget::WorkingTree { path, area, .. } = target else {
            return;
        };
        if !status_will_reset && !super::head_gitlink_lookup_is_worth_it_for(repo, *area, path) {
            return;
        }
        self.queue_head(repos, repo, path);
    }

    fn queue_head(&mut self, repos: &RepositoryRuntimes, repo: &RepoState, path: &Path) {
        if self
            .head
            .iter()
            .any(|fact| fact.lifetime == repo.lifetime_id() && fact.path == path)
        {
            return;
        }
        self.head.push(HeadQuery {
            lifetime: repo.lifetime_id(),
            path: path.to_path_buf(),
            repository: repos.repository(&repo.id).cloned(),
        });
    }

    fn marker(&mut self, repo: &RepoState, target: &DiffTarget) {
        let DiffTarget::WorkingTree { path, area, .. } = target else {
            return;
        };
        if !matches!(
            util::worktree_submodule_source(repo, *area, path),
            util::WorktreeSubmoduleSource::Marker
        ) {
            return;
        }
        self.queue_marker(repo, path);
    }

    fn queue_marker(&mut self, repo: &RepoState, path: &Path) {
        if self
            .markers
            .iter()
            .any(|fact| fact.lifetime == repo.lifetime_id() && fact.path == path)
        {
            return;
        }
        let value = repo.spec.workdir.join(path).join(".git");
        self.markers.push(PathFact {
            lifetime: repo.lifetime_id(),
            path: path.to_path_buf(),
            value,
        });
    }

    fn selected_marker(&mut self, state: &AppState, repo_id: RepoId, skip_conflicts: bool) {
        if let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id)
            && let Some(target) = &repo.diff_state.diff_target
            && !(skip_conflicts && util::selected_conflict_target(repo, target).is_some())
        {
            self.marker(repo, target);
        }
    }

    fn selection(
        &mut self,
        state: &AppState,
        repos: &RepositoryRuntimes,
        repo_id: RepoId,
        target: &DiffTarget,
        content: bool,
    ) {
        if let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) {
            self.head(repos, repo, target, false);
            if content || util::selected_conflict_target(repo, target).is_none() {
                self.marker(repo, target);
            }
        }
    }

    fn activation(&mut self, state: &AppState, repos: &RepositoryRuntimes, repo_id: RepoId) {
        if state.active_repo == Some(repo_id) {
            return;
        }
        if let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id)
            && let Some(target @ DiffTarget::WorkingTree { path, area, .. }) =
                &repo.diff_state.diff_target
        {
            let entries = repo.status_entries_for_area(*area);
            let entry =
                entries.and_then(|entries| entries.iter().find(|entry| entry.path == *path));
            if entries.is_none() || entry.is_some_and(|entry| entry.kind == FileStatusKind::Deleted)
            {
                self.queue_head(repos, repo, path);
            }
            if matches!(repo.open, Loadable::Ready(()))
                && util::selected_conflict_target(repo, target).is_none()
                && matches!(
                    util::worktree_submodule_source_for_entry(repo, path, entry),
                    util::WorktreeSubmoduleSource::Marker
                )
            {
                self.queue_marker(repo, path);
            }
        }
    }

    fn external_change(
        &mut self,
        state: &AppState,
        repos: &RepositoryRuntimes,
        repo_id: RepoId,
        change: &RepoExternalChange,
        finishing_maintenance: Option<RepoId>,
    ) {
        let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) else {
            return;
        };
        if super::maintenance::change_is_deferred(state, repo_id, finishing_maintenance) {
            return;
        }
        if let Some(target) = &repo.diff_state.diff_target {
            if change.git_state {
                self.head(repos, repo, target, false);
            }
            if super::external_and_history::external_change_reloads_diff(target, change)
                && util::selected_conflict_target(repo, target).is_none()
            {
                self.marker(repo, target);
            }
        }
    }
}

impl PreparedReduction {
    pub(in crate::store) fn for_repo_switch(
        repo_id: RepoId,
        state: &RwLock<Arc<AppState>>,
        repos: &RepositoryRuntimes,
        unchanged: bool,
    ) -> Self {
        let now = SystemTime::now();
        let queries = {
            let state = state.read().unwrap_or_else(|error| error.into_inner());
            let mut queries = InputQueries::default();
            if !super::repo_management::repo_switch_reuses_selected_diff(&state, repo_id, unchanged)
            {
                capture_message(&Msg::SetActiveRepo { repo_id }, &state, repos, &mut queries);
            }
            queries
        };
        Self {
            message: Msg::SetActiveRepo { repo_id },
            inputs: queries.resolve(now),
        }
    }

    pub(in crate::store) fn for_worker(
        message: Msg,
        state: &RwLock<Arc<AppState>>,
        repos: &RepositoryRuntimes,
    ) -> Self {
        let now = SystemTime::now();
        if !needs_preparation(&message) {
            return Self {
                message,
                inputs: InputQueries::default().resolve(now),
            };
        }
        let paths = {
            let state = state.read().unwrap_or_else(|error| error.into_inner());
            capture_paths(&message, &state)
        };
        let (message, auth_worktree_path) = paths.resolve(message);
        let queries = {
            let state = state.read().unwrap_or_else(|error| error.into_inner());
            let mut queries = InputQueries::default();
            capture_message(&message, &state, repos, &mut queries);
            queries
        };
        let mut inputs = queries.resolve(now);
        inputs.auth_worktree_path = auth_worktree_path;
        Self { message, inputs }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::store) fn new(
        message: Msg,
        state: &AppState,
        repos: &RepositoryRuntimes,
    ) -> Self {
        let now = SystemTime::now();
        let (message, auth_worktree_path) = capture_paths(&message, state).resolve(message);
        let mut queries = InputQueries::default();
        capture_message(&message, state, repos, &mut queries);
        let mut inputs = queries.resolve(now);
        inputs.auth_worktree_path = auth_worktree_path;
        Self { message, inputs }
    }

    pub(in crate::store) fn into_parts(self) -> (Msg, ReducerInputs) {
        (self.message, self.inputs)
    }

    #[cfg(test)]
    pub(in crate::store) fn selected_input_counts(&self) -> (usize, usize) {
        (self.inputs.head.len(), self.inputs.markers.len())
    }
}

#[derive(Default)]
struct PathRequests(SmallVec<[(PathBuf, PathBase); 1]>);

enum PathBase {
    Process,
    Worktree(PathBuf),
}

impl PathRequests {
    fn repository(&mut self, path: &Path) {
        self.0.push((path.to_path_buf(), PathBase::Process));
    }

    /// A path git resolved inside `repo_id`'s workdir.
    fn in_workdir(&mut self, state: &AppState, repo_id: RepoId, path: &Path) {
        if let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) {
            self.0.push((
                path.to_path_buf(),
                PathBase::Worktree(repo.spec.workdir.clone()),
            ));
        }
    }

    fn resolve(self, mut message: Msg) -> (Msg, Option<PathBuf>) {
        if self.0.is_empty() {
            return (message, None);
        }
        let normalized: SmallVec<[(PathBuf, PathBuf); 1]> = self
            .0
            .into_iter()
            .map(|(path, base)| {
                let value = match base {
                    PathBase::Process => util::normalize_repo_path(path.clone()),
                    PathBase::Worktree(root) => util::canonicalize_path(if path.is_relative() {
                        root.join(&path)
                    } else {
                        path.clone()
                    }),
                };
                (path, value)
            })
            .collect();
        let auth_worktree_path = matches!(message, Msg::SubmitAuthPrompt { .. }).then(|| {
            normalized
                .first()
                .expect("captured auth worktree path")
                .1
                .clone()
        });
        normalize_paths(&mut message, &normalized);
        (message, auth_worktree_path)
    }
}

fn capture_paths(message: &Msg, state: &AppState) -> PathRequests {
    let mut paths = PathRequests::default();
    if super::msg_requires_available_git(message) && !state.git_runtime.is_available() {
        return paths;
    }
    match message {
        Msg::OpenRepo(path)
        | Msg::OpenRepoFromExternalDrop(path)
        | Msg::OpenDocumentRepository { path, .. } => paths.repository(path),
        Msg::RestoreSession {
            open_repos,
            active_repo,
        } => {
            if let Some(path) = active_repo {
                paths.repository(path);
            }
            for path in open_repos {
                if !paths.0.iter().any(|(previous, _)| previous == path) {
                    paths.repository(path);
                }
            }
        }
        Msg::RemoveWorktree { repo_id, path } | Msg::ForceRemoveWorktree { repo_id, path } => {
            paths.in_workdir(state, *repo_id, path)
        }
        Msg::SubmitAuthPrompt {
            username, secret, ..
        } => {
            if state.git_runtime.is_available()
                && let Some(prompt) = &state.auth_prompt
                && super::auth::prepare_staged_git_auth(prompt.kind, username.as_deref(), secret)
                    .is_ok()
                && let AuthRetryOperation::RepoCommand { repo_id, command } = &prompt.operation
                && let RepoCommandKind::RemoveWorktree { path }
                | RepoCommandKind::ForceRemoveWorktree { path } = command
            {
                paths.in_workdir(state, *repo_id, path)
            }
        }
        Msg::Internal(message) => capture_internal_paths(message, state, &mut paths),
        _ => {}
    }
    paths
}

fn capture_internal_paths(message: &InternalMsg, state: &AppState, paths: &mut PathRequests) {
    match message {
        InternalMsg::GitOperationFinished { message, .. } => {
            capture_internal_paths(message, state, paths)
        }
        InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch,
            message,
        } if state
            .repos
            .iter()
            .any(|repo| repo.id == *repo_id && repo.load_epoch == *load_epoch) =>
        {
            capture_internal_paths(message, state, paths)
        }
        InternalMsg::RepoOpenedOk { repo_id, spec, .. }
        | InternalMsg::RepoOpenedErr { repo_id, spec, .. }
            if state.repos.iter().any(|repo| repo.id == *repo_id) =>
        {
            paths.repository(&spec.workdir)
        }
        InternalMsg::RepoActionFinishedInWorktree {
            worktree_path,
            result,
            ..
        } if result.is_ok() => paths.repository(worktree_path),
        // Git ran in the workdir, so a relative path names a folder there.
        InternalMsg::RepoCommandFinished {
            repo_id,
            command,
            result: Ok(_),
        } => {
            if let Some(path) = command.opens_worktree() {
                paths.in_workdir(state, *repo_id, path)
            }
        }
        _ => {}
    }
}

fn replace_path(path: &mut PathBuf, normalized: &[(PathBuf, PathBuf)]) {
    if let Some((_, resolved)) = normalized.iter().find(|(original, _)| original == path) {
        *path = resolved.clone();
    }
}

fn normalize_paths(message: &mut Msg, normalized: &[(PathBuf, PathBuf)]) {
    match message {
        Msg::OpenRepo(path)
        | Msg::OpenRepoFromExternalDrop(path)
        | Msg::OpenDocumentRepository { path, .. }
        | Msg::RemoveWorktree { path, .. }
        | Msg::ForceRemoveWorktree { path, .. } => replace_path(path, normalized),
        Msg::RestoreSession {
            open_repos,
            active_repo,
        } => {
            *open_repos = util::dedup_paths_in_order(std::mem::take(open_repos));
            for path in open_repos {
                replace_path(path, normalized);
            }
            if let Some(path) = active_repo {
                replace_path(path, normalized);
            }
        }
        Msg::Internal(message) => normalize_internal_paths(message, normalized),
        _ => {}
    }
}

fn normalize_internal_paths(message: &mut InternalMsg, normalized: &[(PathBuf, PathBuf)]) {
    match message {
        InternalMsg::RepoLoadFinished { message, .. }
        | InternalMsg::GitOperationFinished { message, .. } => {
            normalize_internal_paths(message, normalized)
        }
        InternalMsg::RepoOpenedOk { spec, .. } | InternalMsg::RepoOpenedErr { spec, .. } => {
            replace_path(&mut spec.workdir, normalized)
        }
        InternalMsg::RepoActionFinishedInWorktree {
            worktree_path,
            result,
            ..
        } if result.is_ok() => replace_path(worktree_path, normalized),
        InternalMsg::RepoCommandFinished {
            command,
            result: Ok(_),
            ..
        } => {
            if let Some(path) = command.opens_worktree_mut() {
                replace_path(path, normalized)
            }
        }
        _ => {}
    }
}

fn needs_preparation(message: &Msg) -> bool {
    matches!(
        message,
        Msg::OpenRepo(_)
            | Msg::OpenRepoFromExternalDrop(_)
            | Msg::OpenDocumentRepository { .. }
            | Msg::RestoreSession { .. }
            | Msg::SetActiveRepo { .. }
            | Msg::CloseRepo { .. }
            | Msg::MoveRepoOut { .. }
            | Msg::CloseRepos { .. }
            | Msg::RemoveWorktree { .. }
            | Msg::ForceRemoveWorktree { .. }
            | Msg::SubmitAuthPrompt { .. }
            | Msg::SelectDiff { .. }
            | Msg::OpenFileContent { .. }
            | Msg::OpenFileEditor { .. }
            | Msg::ExitDiffEditMode { .. }
            | Msg::ViewerNavBack { .. }
            | Msg::ViewerNavForward { .. }
            | Msg::GlobalNavBack { .. }
            | Msg::GlobalNavForward { .. }
            | Msg::ReloadRepo { .. }
            | Msg::RepoExternallyChanged { .. }
            | Msg::SetTextOverride { .. }
            | Msg::FilesystemPathsChanged(_)
            | Msg::FilesystemFinished(_)
    ) || matches!(message, Msg::Internal(message) if internal_needs_preparation(message))
}

fn internal_needs_preparation(message: &InternalMsg) -> bool {
    match message {
        InternalMsg::GitOperationFinished { .. } => true,
        InternalMsg::RepoLoadFinished { message, .. } => internal_needs_preparation(message),
        InternalMsg::RepoOpenedOk { .. }
        | InternalMsg::RepoOpenedErr { .. }
        | InternalMsg::RepoActionFinished { .. }
        | InternalMsg::RepoPathsActionFinished { .. }
        | InternalMsg::BranchAlreadyExists { .. }
        | InternalMsg::RepoActionFinishedInWorktree { .. }
        | InternalMsg::RepoCommandFinished { .. }
        | InternalMsg::DiffLoaded { .. }
        | InternalMsg::DiffFileLoaded { .. }
        | InternalMsg::DiffFileImageLoaded { .. }
        | InternalMsg::DiffPreviewTextFileLoaded { .. }
        | InternalMsg::SubmoduleSummaryLoaded { .. }
        | InternalMsg::TextAttributesLoaded { .. }
        | InternalMsg::CommitDetailsLoaded { .. }
        | InternalMsg::LargeFileSupportLoaded { .. }
        | InternalMsg::FileBrowserLoaded { .. } => true,
        _ => false,
    }
}

fn capture_message(
    message: &Msg,
    state: &AppState,
    repos: &RepositoryRuntimes,
    inputs: &mut InputQueries,
) {
    if super::msg_requires_available_git(message) && !state.git_runtime.is_available() {
        return;
    }
    match message {
        Msg::OpenRepo(path)
        | Msg::OpenRepoFromExternalDrop(path)
        | Msg::OpenDocumentRepository { path, .. } => {
            if !state.repos.iter().any(|repo| repo.spec.workdir == *path) {
                inputs.load_preferences();
            }
            if matches!(
                message,
                Msg::OpenDocumentRepository {
                    activate: false,
                    ..
                }
            ) {
                return;
            }
            if let Some(repo) = state.repos.iter().find(|repo| repo.spec.workdir == *path) {
                inputs.activation(state, repos, repo.id);
            }
        }
        Msg::RestoreSession { .. } => inputs.load_preferences(),
        Msg::SetActiveRepo { repo_id } => inputs.activation(state, repos, *repo_id),
        Msg::CloseRepo { repo_id } | Msg::MoveRepoOut { repo_id } => {
            if state.active_repo == Some(*repo_id)
                && let Some(next) = adjacent_after_close(state, *repo_id)
            {
                inputs.activation(state, repos, next);
            }
        }
        Msg::CloseRepos {
            repo_ids,
            activate_after,
        } => {
            let close_ids = repo_ids
                .iter()
                .copied()
                .filter(|id| state.repos.iter().any(|repo| repo.id == *id))
                .collect::<rustc_hash::FxHashSet<_>>();
            if !close_ids.is_empty()
                && let Some(next) =
                    super::repo_management::active_after_closing(state, &close_ids, *activate_after)
            {
                inputs.activation(state, repos, next);
            }
        }
        Msg::SelectDiff { repo_id, target } if target.worktree().is_none() => {
            inputs.selection(state, repos, *repo_id, target, false);
        }
        Msg::OpenFileContent {
            repo_id,
            source,
            path,
        } => {
            if let Some(target) =
                diff_selection::content_view_target(source.clone(), path.clone(), None)
            {
                inputs.selection(state, repos, *repo_id, &target, true);
            }
        }
        Msg::OpenFileEditor { repo_id, path } => {
            inputs.selection(
                state,
                repos,
                *repo_id,
                &DiffTarget::working_tree(path.clone(), DiffArea::Unstaged),
                true,
            );
        }
        Msg::ExitDiffEditMode { repo_id } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && let Some((target, content)) = diff_selection::editor_return_target(repo)
            {
                inputs.selection(state, repos, *repo_id, target, content);
            }
        }
        Msg::ViewerNavBack { repo_id } | Msg::ViewerNavForward { repo_id } => {
            let direction = if matches!(message, Msg::ViewerNavBack { .. }) {
                ViewNavDir::Back
            } else {
                ViewNavDir::Forward
            };
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && let Some(entry) = repo.navigation.view_history.peek_step(direction)
                && let Some(target) = diff_selection::content_view_target(
                    entry.source.clone(),
                    entry.path.clone(),
                    entry.old_path.clone(),
                )
            {
                inputs.selection(state, repos, *repo_id, &target, true);
            }
        }
        Msg::GlobalNavBack { repo_id } | Msg::GlobalNavForward { repo_id } => {
            let direction = if matches!(message, Msg::GlobalNavBack { .. }) {
                ViewNavDir::Back
            } else {
                ViewNavDir::Forward
            };
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && let Some(destination) = repo.navigation.main_history.peek_step(direction)
                && let Some(target) = &destination.diff_target
            {
                inputs.selection(state, repos, *repo_id, target, destination.content_preview);
            }
        }
        Msg::ReloadRepo { repo_id } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && let Some(target) = &repo.diff_state.diff_target
            {
                inputs.head(repos, repo, target, true);
            }
        }
        Msg::RepoExternallyChanged { repo_id, change } => {
            inputs.external_change(state, repos, *repo_id, change, None);
        }
        Msg::SetTextOverride {
            repo_id,
            path,
            value,
        } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && let Some(target) = &repo.diff_state.diff_target
                && target.file_path() == Some(path.as_path())
                && repo
                    .diff_state
                    .text_override_for(path)
                    .and_then(|old| old.encoding)
                    != value.encoding
                && util::selected_conflict_target(repo, target).is_none()
            {
                inputs.marker(repo, target);
            }
        }
        Msg::FilesystemPathsChanged(changes) => capture_filesystem(state, inputs, changes),
        Msg::FilesystemFinished(result) => capture_filesystem(state, inputs, &result.changes),
        Msg::Internal(message) => capture_internal(message, state, repos, inputs),
        _ => {}
    }
}

fn adjacent_after_close(state: &AppState, repo_id: RepoId) -> Option<RepoId> {
    let index = state.repos.iter().position(|repo| repo.id == repo_id)?;
    if index > 0 {
        Some(state.repos[index - 1].id)
    } else {
        state.repos.get(1).map(|repo| repo.id)
    }
}

fn capture_filesystem(
    state: &AppState,
    inputs: &mut InputQueries,
    changes: &[gitcomet_core::filesystem::PathChange],
) {
    for repo in &state.repos {
        let target = super::filesystem::selected_target_after_changes(repo, changes);
        if target != repo.diff_state.diff_target
            && let Some(target) = target
        {
            inputs.marker(repo, &target);
        }
    }
}

fn capture_action(
    state: &AppState,
    repos: &RepositoryRuntimes,
    inputs: &mut InputQueries,
    repo_id: RepoId,
    action: RepoActionKind,
    active_after: Option<RepoId>,
) {
    if let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id)
        && let Some(target) = &repo.diff_state.diff_target
    {
        if super::external_and_history::repo_action_clears_head_dependent_state(action) {
            inputs.head(repos, repo, target, false);
        }
        if active_after == Some(repo_id) && util::selected_conflict_target(repo, target).is_none() {
            inputs.marker(repo, target);
        }
    }
}

fn capture_internal(
    message: &InternalMsg,
    state: &AppState,
    repos: &RepositoryRuntimes,
    inputs: &mut InputQueries,
) {
    match message {
        InternalMsg::GitOperationFinished {
            repo_id,
            outer_outcome,
            message,
            ..
        } => {
            capture_internal(message, state, repos, inputs);
            if *outer_outcome == GitOperationOuterOutcome::Cancelled
                && let CancelledSelectionRefresh::Required {
                    finished_maintenance,
                } = cancelled_selection_refresh(state, *repo_id, message)
            {
                inputs.external_change(
                    state,
                    repos,
                    *repo_id,
                    &RepoExternalChange::all(),
                    finished_maintenance,
                );
            }
        }
        InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch,
            message,
        } => {
            if state
                .repos
                .iter()
                .any(|repo| repo.id == *repo_id && repo.load_epoch == *load_epoch)
            {
                capture_internal(message, state, repos, inputs);
            }
        }
        InternalMsg::RepoOpenedErr { repo_id, error, .. } => {
            if state.active_repo != Some(*repo_id) {
                return;
            }
            let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id) else {
                return;
            };
            let next = if repo.is_provisional_external_drop_open() {
                repo.external_drop_previous_active_repo()
                    .and_then(|previous| {
                        state
                            .repos
                            .iter()
                            .any(|other| other.id == previous && other.id != *repo_id)
                            .then_some(previous)
                            .or_else(|| adjacent_after_close(state, *repo_id))
                    })
            } else if matches!(error.kind(), ErrorKind::NotARepository) {
                adjacent_after_close(state, *repo_id)
            } else {
                None
            };
            if let Some(next) = next {
                inputs.activation(state, repos, next);
            }
        }
        InternalMsg::RepoActionFinished {
            repo_id, action, ..
        } => {
            capture_action(state, repos, inputs, *repo_id, *action, state.active_repo);
        }
        InternalMsg::RepoPathsActionFinished {
            repo_id,
            action,
            paths,
            result,
        } => {
            let cleared = result.is_ok()
                && state
                    .repos
                    .iter()
                    .find(|repo| repo.id == *repo_id)
                    .is_some_and(|repo| {
                        diff_selection::action_clears_selected_target(
                            repo,
                            *action,
                            paths.as_slice(),
                        )
                    });
            if !cleared {
                capture_action(state, repos, inputs, *repo_id, *action, state.active_repo);
            }
        }
        InternalMsg::BranchAlreadyExists { action, prompt } => {
            capture_action(
                state,
                repos,
                inputs,
                prompt.repo_id,
                *action,
                state.active_repo,
            );
        }
        InternalMsg::RepoActionFinishedInWorktree {
            repo_id,
            action,
            worktree_path,
            result,
        } => {
            let active = if result.is_ok() {
                capture_opened_worktree(state, repos, inputs, worktree_path)
            } else {
                state.active_repo
            };
            capture_action(state, repos, inputs, *repo_id, *action, active);
        }
        InternalMsg::RepoCommandFinished {
            repo_id,
            command,
            result,
        } => {
            if result.is_ok()
                && let Some(path) = command.opens_worktree()
            {
                capture_opened_worktree(state, repos, inputs, path);
            }
            capture_command(state, repos, inputs, *repo_id, command, result.is_ok());
        }
        InternalMsg::DiffLoaded {
            repo_id, target, ..
        }
        | InternalMsg::DiffFileLoaded {
            repo_id, target, ..
        }
        | InternalMsg::DiffFileImageLoaded {
            repo_id, target, ..
        } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && repo.diff_state.diff_target.as_ref() == Some(target)
                && util::selected_conflict_target(repo, target).is_none()
            {
                inputs.marker(repo, target);
            }
        }
        InternalMsg::DiffPreviewTextFileLoaded {
            repo_id, target, ..
        }
        | InternalMsg::SubmoduleSummaryLoaded {
            repo_id, target, ..
        } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && repo.diff_state.diff_target.as_ref() == Some(target)
            {
                inputs.marker(repo, target);
            }
        }
        InternalMsg::TextAttributesLoaded {
            repo_id,
            target,
            result,
        } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && let Some(selected) = &repo.diff_state.diff_target
                && selected.file_path().is_some()
                && selected.file_path() == target.file_path()
                && let (Loadable::Ready(current), Ok(next)) =
                    (&repo.diff_state.text_attributes, result)
                && current.decoding_encodings() != next.decoding_encodings()
                && util::selected_conflict_target(repo, selected).is_none()
            {
                inputs.marker(repo, selected);
            }
        }
        InternalMsg::CommitDetailsLoaded {
            repo_id, commit_id, ..
        } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && repo.history_state.selected_commit.as_ref() == Some(commit_id)
                && let Some(target) = &repo.diff_state.diff_target
            {
                inputs.marker(repo, target);
            }
        }
        InternalMsg::LargeFileSupportLoaded {
            repo_id,
            result: Ok(next),
        } => {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && super::effects::large_file_support_changes_content(repo, next)
                && (repo.large_file_support_active() || next.is_active())
            {
                inputs.selected_marker(state, *repo_id, false);
            }
        }
        InternalMsg::FileBrowserLoaded {
            repo_id,
            source,
            result: Ok(entries),
            cancellation,
        } if !cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled()) =>
        {
            if let Some(repo) = state.repos.iter().find(|repo| repo.id == *repo_id)
                && repo.file_browser.source == *source
                && let Some(reopen) = &repo.file_browser.pending_reopen
                && matches!(
                    super::effects::reopen_decision(repo, source, entries, reopen),
                    super::effects::ReopenDecision::Open
                )
                && let Some(target) =
                    diff_selection::content_view_target(source.clone(), reopen.path.clone(), None)
            {
                inputs.selection(state, repos, *repo_id, &target, true);
            }
        }
        _ => {}
    }
}

enum CancelledSelectionRefresh {
    AlreadyHandled,
    Required {
        finished_maintenance: Option<RepoId>,
    },
}

fn cancelled_selection_refresh(
    state: &AppState,
    repo_id: RepoId,
    message: &InternalMsg,
) -> CancelledSelectionRefresh {
    use CancelledSelectionRefresh::{AlreadyHandled, Required};
    let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return AlreadyHandled;
    };
    if repo.diff_state.diff_target.is_none() {
        return AlreadyHandled;
    }
    let starts_log = || {
        repo.loads_in_flight
            .log_request_starts(&util::refresh_log_request(repo))
    };
    match message {
        InternalMsg::GitOperationFinished { message, .. } => {
            cancelled_selection_refresh(state, repo_id, message)
        }
        InternalMsg::RepoLoadFinished {
            repo_id: loaded_repo,
            load_epoch,
            message,
        } if state
            .repos
            .iter()
            .any(|repo| repo.id == *loaded_repo && repo.load_epoch == *load_epoch) =>
        {
            cancelled_selection_refresh(state, repo_id, message)
        }
        InternalMsg::RepoActionFinished {
            repo_id: completed, ..
        }
        | InternalMsg::RepoPathsActionFinished {
            repo_id: completed, ..
        }
        | InternalMsg::RepoActionFinishedInWorktree {
            repo_id: completed, ..
        } if *completed == repo_id => AlreadyHandled,
        InternalMsg::BranchAlreadyExists { prompt, .. } if prompt.repo_id == repo_id => {
            AlreadyHandled
        }
        InternalMsg::RepoCommandFinished {
            repo_id: completed,
            command,
            result,
        } => {
            let removes_selected_repo = result.is_ok()
                && matches!(command,
                    RepoCommandKind::RemoveWorktree { path } | RepoCommandKind::ForceRemoveWorktree { path }
                        if *path == repo.spec.workdir
                );
            if removes_selected_repo
                || (*completed == repo_id
                    && (starts_log()
                        || (result.is_ok()
                            && super::actions_emit_effects::command_clears_diff_selection(
                                command,
                            ))))
            {
                AlreadyHandled
            } else {
                Required {
                    finished_maintenance: matches!(command, RepoCommandKind::RunMaintenance)
                        .then_some(*completed),
                }
            }
        }
        InternalMsg::CommitFinished {
            repo_id: completed,
            result,
        }
        | InternalMsg::CommitAmendFinished {
            repo_id: completed,
            result,
        } if *completed == repo_id && (result.is_ok() || starts_log()) => AlreadyHandled,
        InternalMsg::SafePushAfterCommitFinished {
            repo_id: completed,
            result,
            ..
        } if *completed == repo_id
            && starts_log()
            && matches!(
                result,
                Err(_) | Ok(gitcomet_core::services::SafePushAfterCommitDecision::Blocked { .. })
            ) =>
        {
            AlreadyHandled
        }
        _ => Required {
            finished_maintenance: None,
        },
    }
}

/// Inputs for opening `path` as a tab, as `repo_management::open_repo` reads
/// them; the tab's id when it is already open.
fn capture_opened_worktree(
    state: &AppState,
    repos: &RepositoryRuntimes,
    inputs: &mut InputQueries,
    path: &Path,
) -> Option<RepoId> {
    let opened = state
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == path)
        .map(|repo| repo.id);
    match opened {
        Some(repo_id) => inputs.activation(state, repos, repo_id),
        None => inputs.load_preferences(),
    }
    opened
}

fn capture_command(
    state: &AppState,
    repos: &RepositoryRuntimes,
    inputs: &mut InputQueries,
    repo_id: RepoId,
    command: &RepoCommandKind,
    succeeded: bool,
) {
    if matches!(
        command,
        RepoCommandKind::CherryPick { .. }
            | RepoCommandKind::InteractiveCherryPick { .. }
            | RepoCommandKind::Revert { .. }
    ) && let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id)
        && let Some(target) = &repo.diff_state.diff_target
    {
        inputs.head(repos, repo, target, false);
    }

    if let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id)
        && let Some(target) = &repo.diff_state.diff_target
    {
        let submodule_command = succeeded
            && super::actions_emit_effects::command_changes_selected_submodule(target, command);
        let objects = matches!(command, RepoCommandKind::LargeFile { command } if command.changes_object_store());
        let patch = matches!(
            command,
            RepoCommandKind::StageHunk
                | RepoCommandKind::UnstageHunk
                | RepoCommandKind::ApplyWorktreePatch { .. }
        ) && util::selected_conflict_target(repo, target).is_none();
        if submodule_command || objects || patch {
            inputs.marker(repo, target);
        }
    }
    if matches!(command, RepoCommandKind::RunMaintenance)
        && let Some(common) = state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.common_dir.as_deref())
    {
        for repo in &state.repos {
            if repo.common_dir.as_deref() == Some(common)
                && let Some(change) = &repo.maintenance.deferred_change
            {
                inputs.external_change(state, repos, repo.id, change, Some(repo_id));
            }
        }
    }
    if succeeded
        && let RepoCommandKind::RemoveWorktree { path }
        | RepoCommandKind::ForceRemoveWorktree { path } = command
    {
        let mut order: Vec<_> = state.repos.iter().map(|repo| repo.id).collect();
        let mut active = state.active_repo;
        for removed in state.repos.iter().filter(|repo| repo.spec.workdir == *path) {
            let index = order
                .iter()
                .position(|id| *id == removed.id)
                .expect("open tab is in order");
            order.remove(index);
            if active == Some(removed.id) {
                active = order.get(index.saturating_sub(1)).copied();
                if let Some(next) = active {
                    inputs.activation(state, repos, next);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{Effect, RepoCommandEffect};
    use gitcomet_core::domain::{FileStatus, RepoSpec, RepoStatus};

    #[test]
    fn repo_activation_defers_selected_path_inputs_but_keeps_maintenance_clock() {
        let repo_id = RepoId(1);
        let repos = RepositoryRuntimes::default();
        for kind in [FileStatusKind::Deleted, FileStatusKind::Modified] {
            let path = PathBuf::from("vendor/submodule");
            let mut repo = RepoState::new_opening(
                repo_id,
                RepoSpec {
                    workdir: PathBuf::from("activation-preparation-test"),
                },
            );
            repo.set_open(Loadable::Ready(()));
            repo.set_status(Loadable::Ready(Arc::new(RepoStatus {
                staged: Arc::default(),
                unstaged: Arc::new(vec![FileStatus {
                    path: path.clone(),
                    kind,
                    conflict: None,
                }]),
            })));
            repo.diff_state.diff_target = Some(DiffTarget::working_tree(path, DiffArea::Unstaged));
            repo.common_dir = Some(Arc::from(repo.spec.workdir.join(".git")));
            let mut state = AppState::test_default();
            state.maintenance_settings.recommend = true;
            state.repos.push(repo);
            state.active_repo = Some(repo_id);
            let message = Msg::RepoActivated { repo_id };

            assert!(!needs_preparation(&message));
            assert!(capture_paths(&message, &state).0.is_empty());
            let mut queries = InputQueries::default();
            capture_message(&message, &state, &repos, &mut queries);
            assert!(queries.head.is_empty());
            assert!(queries.markers.is_empty());
            assert!(!queries.needs_preferences);

            let shared = RwLock::new(Arc::new(state));
            let before = SystemTime::now();
            let (_, inputs) = PreparedReduction::for_worker(message, &shared, &repos).into_parts();
            assert!(inputs.head.is_empty());
            assert!(inputs.markers.is_empty());
            assert!(inputs.now() >= before && inputs.now() <= SystemTime::now());
            let mut state = shared.into_inner().unwrap();
            let state = Arc::make_mut(&mut state);
            assert!(matches!(
                super::super::maintenance::request_check(&inputs, state, repo_id),
                Some(Effect::RepoCommand(RepoCommandEffect::CheckRepoMaintenance { repo_id: id }))
                    if id == repo_id
            ));
            assert_eq!(
                state.repos[0].maintenance.check_requested_at,
                Some(inputs.now())
            );
            assert!(super::super::maintenance::request_check(&inputs, state, repo_id).is_none());
        }
    }
}
