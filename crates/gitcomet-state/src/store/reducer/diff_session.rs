//! Diff sessions: open, retarget, reload, blame, and completions. Each
//! session's work is stopped by its own token; completions from an older
//! generation or repository lifetime are dropped.

use super::*;
use crate::diff_session::{
    ChangeListSession, DiffSession, DiffSessionContent, DiffSessionEffect, DiffSessionLoads,
    DiffSessionMsg as Event, DiffSessionWork, DiffViewId, Refreshable,
};
use crate::model::RepoState;
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::error::{Error, ErrorKind};
use std::sync::Arc;

pub(super) fn reduce(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    event: Event,
) -> Vec<Effect> {
    let explicit = match &event {
        Event::Open {
            repo_id,
            lifetime,
            view,
            ..
        }
        | Event::OpenEditor {
            repo_id,
            lifetime,
            view,
            ..
        }
        | Event::ExitEditor {
            repo_id,
            lifetime,
            view,
        }
        | Event::SetEncoding {
            repo_id,
            lifetime,
            view,
            ..
        }
        | Event::SetTextOverride {
            repo_id,
            lifetime,
            view,
            ..
        }
        | Event::SetContentMode {
            repo_id,
            lifetime,
            view,
            ..
        }
        | Event::Reload {
            repo_id,
            lifetime,
            view,
        }
        | Event::LoadBlame {
            repo_id,
            lifetime,
            view,
        } => Some((*repo_id, *lifetime, *view, false)),
        Event::OpenChanges {
            repo_id,
            lifetime,
            view,
            ..
        } => Some((*repo_id, *lifetime, *view, true)),
        Event::Clear { .. }
        | Event::Close { .. }
        | Event::CloseChanges { .. }
        | Event::Loaded { .. }
        | Event::ChangesLoaded { .. } => None,
    };
    let mut effects = reduce_event(state, event);
    if let Some((repo_id, lifetime, view, list)) = explicit
        && let Some(repo) = state.repos.iter_mut().find(|repo| {
            repo.id == repo_id
                && repo.lifetime() == lifetime
                && if list {
                    repo.change_lists.contains_key(&view)
                } else {
                    repo.diff_sessions.contains_key(&view)
                }
        })
    {
        repo.last_active_at = Some(inputs.now());
        if !repos.has_repository(&repo_id) {
            super::repo_management::append_open_repo_effect_if_not_loaded(repo, &mut effects);
        }
    }
    // Reads for a repository without a backend wait for its open. With no
    // open in flight, they are answered with why the repository is closed.
    let mut unavailable = Vec::new();
    effects.retain(|effect| {
        let Effect::DiffSession(work) = effect else {
            return true;
        };
        let Some(repo) = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == work.repo_id && repo.lifetime() == work.lifetime)
        else {
            return false;
        };
        if repos.has_repository(&repo.id)
            || (!repo.backend_parked && matches!(repo.open, Loadable::Ready(())))
        {
            return true;
        }
        await_backend(repo, work);
        if !repo.open.is_loading() && !unavailable.iter().any(|(id, _)| *id == repo.id) {
            unavailable.push((repo.id, unavailable_error(repo)));
        }
        false
    });
    for (repo_id, error) in unavailable {
        effects.extend(settle_awaiting_backend(state, repo_id, &error));
    }
    effects
}

fn await_backend(repo: &mut RepoState, work: &DiffSessionEffect) {
    if matches!(work.work, DiffSessionWork::Changes { .. }) {
        if let Some(list) = Arc::make_mut(&mut repo.change_lists).get_mut(&work.view)
            && list.generation == work.generation
        {
            list.awaiting_backend = true;
        }
    } else if let Some(session) = Arc::make_mut(&mut repo.diff_sessions).get_mut(&work.view)
        && session.generation == work.generation
    {
        session.awaiting_backend = true;
    }
}

fn unavailable_error(repo: &RepoState) -> Error {
    Error::new(ErrorKind::Backend(match &repo.open {
        Loadable::Error(message) => message.clone(),
        _ => "The repository is not open.".to_string(),
    }))
}

/// Answers the reads waiting for a backend whose open ended without one,
/// through the same completions the scheduler sends.
pub(super) fn settle_awaiting_backend(
    state: &mut AppState,
    repo_id: RepoId,
    error: &Error,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let lifetime = repo.lifetime();
    let mut replies = Vec::new();
    if repo
        .diff_sessions
        .values()
        .any(|session| session.awaiting_backend)
    {
        for (view, session) in Arc::make_mut(&mut repo.diff_sessions).iter_mut() {
            if !std::mem::take(&mut session.awaiting_backend) {
                continue;
            }
            session.refresh_queued = false;
            replies.extend(
                pending_failures(&session.pending, error)
                    .into_iter()
                    .map(|content| Event::Loaded {
                        repo_id,
                        view: *view,
                        lifetime,
                        generation: session.generation,
                        content,
                    }),
            );
        }
    }
    if repo.change_lists.values().any(|list| list.awaiting_backend) {
        for (view, list) in Arc::make_mut(&mut repo.change_lists).iter_mut() {
            if !std::mem::take(&mut list.awaiting_backend) {
                continue;
            }
            list.refresh_queued = false;
            replies.push(Event::ChangesLoaded {
                repo_id,
                view: *view,
                lifetime,
                generation: list.generation,
                result: Err(error.clone()),
            });
        }
    }
    replies
        .into_iter()
        .flat_map(|reply| reduce_event(state, reply))
        .collect()
}

fn pending_failures(pending: &DiffSessionLoads, error: &Error) -> Vec<DiffSessionContent> {
    let mut contents = Vec::new();
    if pending.attributes {
        contents.push(DiffSessionContent::Attributes(Err(error.clone())));
    }
    if pending.patch {
        contents.push(DiffSessionContent::Patch(Err(error.clone())));
    }
    if pending.file_text {
        contents.push(DiffSessionContent::FileText(Err(error.clone())));
    }
    if pending.image {
        contents.push(DiffSessionContent::Image(Err(error.clone())));
    }
    if pending.preview_text {
        contents.push(DiffSessionContent::PreviewText(Err(error.clone())));
    }
    if pending.blame {
        contents.push(DiffSessionContent::Blame(Err(error.clone())));
    }
    contents
}

fn reduce_event(state: &mut AppState, event: Event) -> Vec<Effect> {
    let requires_worktree = match &event {
        Event::Open {
            target:
                DiffTarget::WorkingTree {
                    worktree: Some(path),
                    ..
                },
            ..
        }
        | Event::OpenChanges {
            source: crate::diff_session::ChangeSource::LinkedWorktree { path, .. },
            ..
        } => state
            .repos
            .iter()
            .find(|repo| &repo.spec.workdir == path)
            .map(|repo| repo.id),
        Event::Open {
            repo_id, target, ..
        } => matches!(
            target,
            DiffTarget::WorkingTree { worktree: None, .. }
                | DiffTarget::CommitRange {
                    to_commit_id: None,
                    ..
                }
        )
        .then_some(*repo_id),
        Event::OpenEditor { repo_id, .. }
        | Event::SetContentMode {
            repo_id,
            edit: true,
            ..
        } => Some(*repo_id),
        Event::OpenChanges {
            repo_id, source, ..
        } => matches!(
            source,
            crate::diff_session::ChangeSource::Worktree { .. }
                | crate::diff_session::ChangeSource::Comparison { to: None, .. }
        )
        .then_some(*repo_id),
        Event::SetEncoding { .. }
        | Event::SetTextOverride { .. }
        | Event::SetContentMode { edit: false, .. }
        | Event::Clear { .. }
        | Event::ExitEditor { .. }
        | Event::Reload { .. }
        | Event::LoadBlame { .. }
        | Event::Close { .. }
        | Event::CloseChanges { .. }
        | Event::ChangesLoaded { .. }
        | Event::Loaded { .. } => None,
    };
    if requires_worktree.is_some_and(|repo_id| super::refuse_bare_worktree_action(state, repo_id)) {
        return Vec::new();
    }
    match event {
        Event::Open {
            repo_id,
            lifetime,
            view,
            target,
            request,
        } => with_repo(state, repo_id, lifetime, |repo| {
            let (repo_id, lifetime) = (repo.id, repo.lifetime());
            // Opening clears content preview, so the side is known up front
            // and the first load already reads it.
            let side = session_side(repo, &target, false);
            let sessions = Arc::make_mut(&mut repo.diff_sessions);
            let session = sessions
                .entry(view)
                .and_modify(|session| {
                    if session.target.file_path() != target.file_path() {
                        session.encoding = None;
                        session.text_override = None;
                        session.text_override_rev = session.text_override_rev.wrapping_add(1);
                    }
                    session.content_preview = false;
                    session.edit_mode = false;
                    session.edit_return_view = None;
                    session.target = target.clone();
                    session.diff_state.diff_target = Some(target.clone());
                    session.diff_state.diff_target_rev =
                        session.diff_state.diff_target_rev.wrapping_add(1);
                    session.blame = Loadable::NotLoaded;
                })
                .or_insert_with(|| DiffSession::new(target));
            if request.is_some() {
                session.open_request = request;
            }
            session.diff_state.one_sided_side = side;
            load(repo_id, lifetime, view, session)
        }),
        Event::Clear {
            repo_id,
            lifetime,
            view,
        } => with_session(state, repo_id, lifetime, view, |_, session| {
            session.next_generation();
            session.diff_state = Default::default();
            session.pending = Default::default();
            Vec::new()
        }),
        Event::OpenEditor {
            repo_id,
            lifetime,
            view,
            path,
        } => with_session(state, repo_id, lifetime, view, |lifetime, session| {
            if session.edit_mode {
                return Vec::new();
            }
            session.edit_return_view = Some(crate::model::FileEditReturnView {
                target: session.target.clone(),
                content_preview: session.content_preview,
            });
            let target = DiffTarget::working_tree(path, gitcomet_core::domain::DiffArea::Unstaged);
            session.target = target.clone();
            session.diff_target = Some(target);
            session.diff_target_rev = session.diff_target_rev.wrapping_add(1);
            session.content_preview = true;
            session.edit_mode = true;
            load(repo_id, lifetime, view, session)
        }),
        Event::ExitEditor {
            repo_id,
            lifetime,
            view,
        } => with_session(state, repo_id, lifetime, view, |lifetime, session| {
            if !session.edit_mode {
                return Vec::new();
            }
            if let Some(previous) = session.edit_return_view.take() {
                session.target = previous.target.clone();
                session.diff_target = Some(previous.target);
                session.content_preview = previous.content_preview;
                session.diff_target_rev = session.diff_target_rev.wrapping_add(1);
            }
            session.edit_mode = false;
            load(repo_id, lifetime, view, session)
        }),
        Event::SetEncoding {
            repo_id,
            lifetime,
            view,
            encoding,
        } => with_session(state, repo_id, lifetime, view, |lifetime, session| {
            if session.encoding == encoding {
                return Vec::new();
            }
            let Some(path) = session.target.file_path().map(ToOwned::to_owned) else {
                return Vec::new();
            };
            let value = gitcomet_core::text_format::TextOverride {
                encoding,
                ..session.text_override_for(&path).unwrap_or_default()
            };
            session.text_override =
                (!value.is_empty()).then_some(crate::model::OpenFileTextOverride { path, value });
            session.text_override_rev = session.text_override_rev.wrapping_add(1);
            session.encoding = encoding;
            load(repo_id, lifetime, view, session)
        }),
        Event::SetTextOverride {
            repo_id,
            lifetime,
            view,
            path,
            value,
        } => with_session(state, repo_id, lifetime, view, |lifetime, session| {
            if session.target.file_path() != Some(path.as_path())
                || session.text_override_for(&path).unwrap_or_default() == value
            {
                return Vec::new();
            }
            let encoding_changed = session.encoding != value.encoding;
            session.text_override =
                (!value.is_empty()).then_some(crate::model::OpenFileTextOverride { path, value });
            session.text_override_rev = session.text_override_rev.wrapping_add(1);
            session.encoding = value.encoding;
            session.rev = session.rev.wrapping_add(1);
            session.diff_state_rev = session.rev;
            if encoding_changed {
                load(repo_id, lifetime, view, session)
            } else {
                Vec::new()
            }
        }),
        Event::SetContentMode {
            repo_id,
            lifetime,
            view,
            preview,
            edit,
        } => with_session(state, repo_id, lifetime, view, |_, session| {
            session.content_preview = preview;
            session.edit_mode =
                preview && edit && matches!(session.target, DiffTarget::WorkingTree { .. });
            session.rev = session.rev.wrapping_add(1);
            session.diff_state_rev = session.rev;
            Vec::new()
        }),
        Event::Reload {
            repo_id,
            lifetime,
            view,
        } => with_session(state, repo_id, lifetime, view, |lifetime, session| {
            load(repo_id, lifetime, view, session)
        }),
        Event::LoadBlame {
            repo_id,
            lifetime,
            view,
        } => with_session(state, repo_id, lifetime, view, |lifetime, session| {
            session.blame_requested = true;
            load_blame(repo_id, lifetime, view, session)
        }),
        Event::Close {
            repo_id,
            lifetime,
            view,
        } => {
            if let Some(repo) = state
                .repos
                .iter_mut()
                .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
                && repo.diff_sessions.contains_key(&view)
            {
                let sessions = Arc::make_mut(&mut repo.diff_sessions);
                if let Some(session) = sessions.remove(&view) {
                    session.cancellation.cancel();
                }
            }
            Vec::new()
        }
        Event::OpenChanges {
            repo_id,
            lifetime,
            view,
            source,
        } => with_repo(state, repo_id, lifetime, |repo| {
            let (repo_id, lifetime) = (repo.id, repo.lifetime());
            let lists = Arc::make_mut(&mut repo.change_lists);
            let list = lists
                .entry(view)
                .and_modify(|list| {
                    list.source = source.clone();
                    list.base = None;
                })
                .or_insert_with(|| ChangeListSession::new(source));
            load_changes(repo_id, lifetime, view, list)
        }),
        Event::CloseChanges {
            repo_id,
            lifetime,
            view,
        } => {
            if let Some(repo) = state
                .repos
                .iter_mut()
                .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
                && repo.change_lists.contains_key(&view)
                && let Some(list) = Arc::make_mut(&mut repo.change_lists).remove(&view)
            {
                list.cancellation.cancel();
            }
            Vec::new()
        }
        Event::ChangesLoaded {
            repo_id,
            view,
            lifetime,
            generation,
            result,
        } => {
            let Some(repo) = state
                .repos
                .iter_mut()
                .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
            else {
                return Vec::new();
            };
            if !repo
                .change_lists
                .get(&view)
                .is_some_and(|list| list.generation == generation)
            {
                return Vec::new();
            }
            let list = Arc::make_mut(&mut repo.change_lists)
                .get_mut(&view)
                .expect("checked above");
            let cancelled = settle(
                &mut list.files,
                result.map(|(base, files)| {
                    list.base = base;
                    Arc::new(files)
                }),
            );
            list.refresh_queued |= cancelled;
            list.rev = list.rev.wrapping_add(1);
            list.loading = false;
            if list.refresh_queued && !cancelled {
                refresh_changes(repo_id, lifetime, view, list)
            } else {
                Vec::new()
            }
        }
        Event::Loaded {
            repo_id,
            view,
            lifetime,
            generation,
            content,
        } => {
            let Some(repo) = state
                .repos
                .iter_mut()
                .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
            else {
                return Vec::new();
            };
            let current = repo
                .diff_sessions
                .get(&view)
                .is_some_and(|session| session.generation == generation);
            if !current {
                return Vec::new();
            }
            let session = Arc::make_mut(&mut repo.diff_sessions)
                .get_mut(&view)
                .expect("checked above");
            let cancelled = match content {
                DiffSessionContent::Attributes(result) => {
                    session.pending.attributes = false;
                    session.text_attributes_rev = session.text_attributes_rev.wrapping_add(1);
                    settle(&mut session.text_attributes, result.map(Arc::new))
                }
                DiffSessionContent::Patch(result) => {
                    session.pending.patch = false;
                    session.diff_rev = session.diff_rev.wrapping_add(1);
                    settle(&mut session.diff, result.map(Arc::new))
                }
                DiffSessionContent::FileText(result) => {
                    session.pending.file_text = false;
                    session.diff_file_rev = session.diff_file_rev.wrapping_add(1);
                    settle(
                        &mut session.diff_file,
                        result.map(|text| text.map(|text| Arc::new(*text))),
                    )
                }
                DiffSessionContent::Image(result) => {
                    session.pending.image = false;
                    settle(
                        &mut session.diff_file_image,
                        result.map(|image| image.map(Arc::new)),
                    )
                }
                DiffSessionContent::PreviewText(result) => {
                    session.pending.preview_text = false;
                    session.diff_preview_text_file_rev =
                        session.diff_preview_text_file_rev.wrapping_add(1);
                    settle(
                        &mut session.diff_preview_text_file,
                        result.map(|file| file.map(Arc::new)),
                    )
                }
                DiffSessionContent::Blame(result) => {
                    session.pending.blame = false;
                    settle(&mut session.blame, result.map(Arc::new))
                }
            };
            session.refresh_queued |= cancelled;
            session.rev = session.rev.wrapping_add(1);
            session.diff_state.diff_state_rev = session.rev;
            session.diff_reload_in_flight = session.is_loading();
            if session.refresh_queued && !session.is_loading() && !cancelled {
                refresh(repo_id, lifetime, view, session)
            } else {
                Vec::new()
            }
        }
    }
}

/// Refreshes the entries `affected` selects: now when idle, else once the
/// load in flight completes. The map stays shared when none is.
fn refresh_affected<S: Refreshable + Clone>(
    map: &mut Arc<FxHashMap<DiffViewId, S>>,
    affected: impl Fn(&S) -> bool,
    mut refresh: impl FnMut(DiffViewId, &mut S) -> Vec<Effect>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if map.values().any(&affected) {
        for (view, session) in Arc::make_mut(map).iter_mut() {
            if !affected(session) {
                continue;
            }
            if session.is_loading() {
                session.queue_refresh();
            } else {
                effects.extend(refresh(*view, session));
            }
        }
    }
    effects
}

/// Loads cancelled with the repository's (a tab switch) are queued, not
/// retried into the same cancellation; its next activation runs them.
pub(super) fn resume_queued_loads(repo: &mut RepoState, effects: &mut impl Extend<Effect>) {
    let (repo_id, lifetime) = (repo.id, repo.lifetime());
    requeue_awaiting_backend(repo);
    effects.extend(refresh_affected(
        &mut repo.diff_sessions,
        |session| session.refresh_queued && !session.is_loading(),
        |view, session| refresh(repo_id, lifetime, view, session),
    ));
    effects.extend(refresh_affected(
        &mut repo.change_lists,
        |list| list.refresh_queued && !list.is_loading(),
        |view, list| refresh_changes(repo_id, lifetime, view, list),
    ));
}

fn requeue_awaiting_backend(repo: &mut RepoState) {
    if repo
        .diff_sessions
        .values()
        .any(|session| session.awaiting_backend)
    {
        for session in Arc::make_mut(&mut repo.diff_sessions).values_mut() {
            if std::mem::take(&mut session.awaiting_backend) {
                session.pending = DiffSessionLoads::default();
                session.refresh_queued = true;
            }
        }
    }
    if repo.change_lists.values().any(|list| list.awaiting_backend) {
        for list in Arc::make_mut(&mut repo.change_lists).values_mut() {
            if std::mem::take(&mut list.awaiting_backend) {
                list.loading = false;
                list.refresh_queued = true;
            }
        }
    }
}

/// Queue at most one refresh while a load is in flight. Known worktree
/// paths only invalidate panes for those files; index/HEAD changes are broad.
/// `worktree` names the linked worktree that changed (`None`: the main one);
/// only its sessions and lists reload.
pub(super) fn reload_worktree_sessions(
    repo: &mut RepoState,
    change: &crate::msg::RepoExternalChange,
    worktree: Option<&std::path::Path>,
) -> Vec<Effect> {
    let (repo_id, lifetime) = (repo.id, repo.lifetime());
    let affected = |session: &DiffSession| {
        session.follows_worktree()
            && session.worktree() == worktree
            && super::external_and_history::external_change_reloads_diff(&session.target, change)
    };
    let mut effects = refresh_affected(&mut repo.diff_sessions, affected, |view, session| {
        refresh(repo_id, lifetime, view, session)
    });
    // A list covers the whole repository, including files not listed yet.
    effects.extend(refresh_affected(
        &mut repo.change_lists,
        |list| list.source.follows_worktree() && list.source.linked_path() == worktree,
        |view, list| refresh_changes(repo_id, lifetime, view, list),
    ));
    effects
}

fn load_blame(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    session: &mut DiffSession,
) -> Vec<Effect> {
    if session.diff_target.is_none() {
        return Vec::new();
    }
    let Some((path, source)) = session.blame_source() else {
        return Vec::new();
    };
    if matches!(session.blame, Loadable::Loading | Loadable::Ready(_)) {
        return Vec::new();
    }
    session.blame_path = Some(path.clone());
    session.blame_source = Some(source.clone());
    session.blame = Loadable::Loading;
    session.pending.blame = true;
    session.rev = session.rev.wrapping_add(1);
    vec![Effect::DiffSession(DiffSessionEffect {
        repo_id,
        view,
        lifetime,
        generation: session.generation,
        work: DiffSessionWork::Blame {
            path,
            source,
            worktree: session.worktree().map(std::path::Path::to_path_buf),
        },
        cancellation: session.cancellation.clone(),
    })]
}

fn load_changes(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    list: &mut ChangeListSession,
) -> Vec<Effect> {
    let cancellation = list.next_generation();
    list.loading = true;
    list.files = Loadable::Loading;
    vec![Effect::DiffSession(DiffSessionEffect {
        repo_id,
        view,
        lifetime,
        generation: list.generation,
        work: DiffSessionWork::Changes {
            source: list.source.clone(),
        },
        cancellation,
    })]
}

/// Applies a reply to its slot. A cancelled load (its repository was
/// deactivated) is not an error: the slot keeps the content it retained,
/// or shows nothing.
fn settle<T>(slot: &mut Loadable<T>, result: gitcomet_core::services::Result<T>) -> bool {
    match result {
        Ok(value) => *slot = Loadable::Ready(value),
        Err(error) if matches!(error.kind(), gitcomet_core::error::ErrorKind::Cancelled) => {
            if !matches!(slot, Loadable::Ready(_)) {
                *slot = Loadable::NotLoaded;
            }
            return true;
        }
        Err(error) => *slot = Loadable::Error(error.to_string()),
    }
    false
}

// Publish completed content even when another refresh was queued. Otherwise
// a stream of edits could keep a newly opened pane empty indefinitely.
fn retain_ready<T>(previous: Loadable<T>, current: &mut Loadable<T>) {
    if matches!(previous, Loadable::Ready(_)) {
        *current = previous;
    }
}

fn refresh(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    session: &mut DiffSession,
) -> Vec<Effect> {
    let previous = (
        session.diff.clone(),
        session.diff_file.clone(),
        session.diff_file_image.clone(),
        session.blame.clone(),
        session.diff_preview_text_file.clone(),
    );
    let effects = load(repo_id, lifetime, view, session);
    retain_ready(previous.0, &mut session.diff);
    retain_ready(previous.1, &mut session.diff_file);
    retain_ready(previous.2, &mut session.diff_file_image);
    retain_ready(previous.3, &mut session.blame);
    if session.pending.preview_text {
        retain_ready(previous.4, &mut session.diff_preview_text_file);
    }
    effects
}

fn refresh_changes(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    list: &mut ChangeListSession,
) -> Vec<Effect> {
    let previous = list.files.clone();
    let effects = load_changes(repo_id, lifetime, view, list);
    retain_ready(previous, &mut list.files);
    effects
}

fn with_repo(
    state: &mut AppState,
    repo_id: RepoId,
    lifetime: u64,
    f: impl FnOnce(&mut RepoState) -> Vec<Effect>,
) -> Vec<Effect> {
    match state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
    {
        Some(repo) => f(repo),
        None => Vec::new(),
    }
}

fn with_session(
    state: &mut AppState,
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    f: impl FnOnce(u64, &mut DiffSession) -> Vec<Effect>,
) -> Vec<Effect> {
    with_repo(state, repo_id, lifetime, |repo| {
        if !repo.diff_sessions.contains_key(&view) {
            return Vec::new();
        }
        let lifetime = repo.lifetime();
        let session = Arc::make_mut(&mut repo.diff_sessions)
            .get_mut(&view)
            .expect("checked above");
        let mut effects = f(lifetime, session);
        effects.extend(record_one_sided_sides(repo));
        effects
    })
}

/// The one side a session's change has, classified as the selected diff's.
/// A linked worktree's changes are not in this repository's status.
fn session_side(
    repo: &RepoState,
    target: &DiffTarget,
    content_preview: bool,
) -> Option<gitcomet_core::domain::DiffPreviewTextSide> {
    if content_preview || target.worktree().is_some() {
        return None;
    }
    repo.one_sided_change_side(target)
}

/// Re-classifies every session once what that reads (status, commit details,
/// the comparison list) lands or its target changes, and reloads the ones
/// whose side changed so they read it.
pub(super) fn record_one_sided_sides(repo: &mut RepoState) -> Vec<Effect> {
    let changed: Vec<_> = repo
        .diff_sessions
        .iter()
        .filter_map(|(view, session)| {
            let side = session_side(repo, &session.target, session.content_preview);
            (session.diff_state.one_sided_side != side).then_some((*view, side))
        })
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let (repo_id, lifetime) = (repo.id, repo.lifetime());
    let sessions = Arc::make_mut(&mut repo.diff_sessions);
    let mut effects = Vec::new();
    for (view, side) in changed {
        let Some(session) = sessions.get_mut(&view) else {
            continue;
        };
        session.diff_state.one_sided_side = side;
        if session.is_loading() {
            session.queue_refresh();
        } else {
            effects.extend(refresh(repo_id, lifetime, view, session));
        }
    }
    effects
}

/// Starts a new generation and loads what the target shows. The plan is the
/// selected diff's: a file target reads its patch and text (or image), plus
/// the one side an added or deleted file has; a whole-commit target its patch.
fn load(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    session: &mut DiffSession,
) -> Vec<Effect> {
    if session.diff_target.is_none() {
        return Vec::new();
    }
    let cancellation = session.next_generation();
    let preview = util::diff_target_preview_flags(&session.target);
    let has_file = session.target.file_path().is_some();
    let image = has_file && preview.wants_image;
    let file_text = has_file && preview.reads_text();
    let preview_text_side = session.diff_state.one_sided_side.filter(|_| file_text);
    session.diff_state_rev = session.rev;
    session.diff_rev = session.diff_rev.wrapping_add(1);
    session.diff_file_rev = session.diff_file_rev.wrapping_add(1);
    session.diff_reload_in_flight = true;
    session.text_attributes = if has_file {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    session.pending = DiffSessionLoads {
        attributes: has_file,
        patch: true,
        file_text,
        image,
        preview_text: preview_text_side.is_some(),
        blame: false,
    };
    session.diff = Loadable::Loading;
    session.diff_file = if file_text {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    session.diff_file_image = if image {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    session.diff_preview_text_file = if preview_text_side.is_some() {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    session.blame = Loadable::NotLoaded;
    let mut effects = vec![Effect::DiffSession(DiffSessionEffect {
        repo_id,
        view,
        lifetime,
        generation: session.generation,
        work: DiffSessionWork::Content {
            target: session.target.clone(),
            encoding: session.encoding,
            patch: true,
            file_text,
            image,
            preview_text_side,
        },
        cancellation,
    })];
    if session.blame_requested {
        effects.extend(load_blame(repo_id, lifetime, view, session));
    }
    effects
}
