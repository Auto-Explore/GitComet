//! Diff sessions: open, retarget, reload, blame, and completions. Each
//! session's work is stopped by its own token; completions from an older
//! generation or repository lifetime are dropped.

use super::*;
use crate::diff_session::{
    ChangeListSession, DiffSession, DiffSessionContent, DiffSessionEffect, DiffSessionMsg as Event,
    DiffSessionWork, DiffViewId,
};
use crate::model::RepoState;
use std::sync::Arc;

pub(super) fn reduce(state: &mut AppState, event: Event) -> Vec<Effect> {
    match event {
        Event::Open {
            repo_id,
            view,
            target,
        } => with_repo(state, repo_id, |repo| {
            let (repo_id, lifetime) = (repo.id, repo.lifetime());
            let sessions = Arc::make_mut(&mut repo.diff_sessions);
            let session = sessions
                .entry(view)
                .and_modify(|session| {
                    session.target = target.clone();
                    session.blame = Loadable::NotLoaded;
                })
                .or_insert_with(|| DiffSession::new(target));
            load(repo_id, lifetime, view, session)
        }),
        Event::SetEncoding {
            repo_id,
            view,
            encoding,
        } => with_session(state, repo_id, view, |lifetime, session| {
            if session.encoding == encoding {
                return Vec::new();
            }
            session.encoding = encoding;
            load(repo_id, lifetime, view, session)
        }),
        Event::Reload { repo_id, view } => {
            with_session(state, repo_id, view, |lifetime, session| {
                load(repo_id, lifetime, view, session)
            })
        }
        Event::LoadBlame { repo_id, view } => {
            with_session(state, repo_id, view, |lifetime, session| {
                let Some((path, source)) = session.blame_source() else {
                    return Vec::new();
                };
                if matches!(session.blame, Loadable::Loading | Loadable::Ready(_)) {
                    return Vec::new();
                }
                session.blame = Loadable::Loading;
                session.rev = session.rev.wrapping_add(1);
                vec![Effect::DiffSession(DiffSessionEffect {
                    repo_id,
                    view,
                    lifetime,
                    generation: session.generation,
                    work: DiffSessionWork::Blame { path, source },
                    cancellation: session.cancellation.clone(),
                })]
            })
        }
        Event::Close { repo_id, view } => {
            if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
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
            view,
            source,
        } => with_repo(state, repo_id, |repo| {
            let (repo_id, lifetime) = (repo.id, repo.lifetime());
            let lists = Arc::make_mut(&mut repo.change_lists);
            let list = lists
                .entry(view)
                .and_modify(|list| list.source = source.clone())
                .or_insert_with(|| ChangeListSession::new(source));
            load_changes(repo_id, lifetime, view, list)
        }),
        Event::CloseChanges { repo_id, view } => {
            if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
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
            match result {
                Ok((base, files)) => {
                    list.base = base;
                    list.files = Loadable::Ready(Arc::new(files));
                }
                Err(error) => list.files = Loadable::Error(error.to_string()),
            }
            list.rev = list.rev.wrapping_add(1);
            Vec::new()
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
            match content {
                DiffSessionContent::Patch(result) => {
                    session.diff = loadable(result.map(Arc::new));
                }
                DiffSessionContent::FileText(result) => {
                    session.file_text = loadable(result.map(|text| text.map(Arc::new)));
                }
                DiffSessionContent::Image(result) => {
                    session.file_image = loadable(result.map(|image| image.map(Arc::new)));
                }
                DiffSessionContent::Blame(result) => {
                    session.blame = loadable(result.map(Arc::new));
                }
            }
            session.rev = session.rev.wrapping_add(1);
            Vec::new()
        }
    }
}

/// Reloads the sessions and lists that follow the working tree after an
/// external edit.
pub(super) fn reload_worktree_sessions(repo: &mut RepoState) -> Vec<Effect> {
    let (repo_id, lifetime) = (repo.id, repo.lifetime());
    let mut effects = Vec::new();
    if repo
        .diff_sessions
        .values()
        .any(DiffSession::follows_worktree)
    {
        for (view, session) in Arc::make_mut(&mut repo.diff_sessions).iter_mut() {
            if session.follows_worktree() {
                effects.extend(load(repo_id, lifetime, *view, session));
            }
        }
    }
    if repo
        .change_lists
        .values()
        .any(|list| list.source.follows_worktree())
    {
        for (view, list) in Arc::make_mut(&mut repo.change_lists).iter_mut() {
            if list.source.follows_worktree() {
                effects.extend(load_changes(repo_id, lifetime, *view, list));
            }
        }
    }
    effects
}

fn load_changes(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    list: &mut ChangeListSession,
) -> Vec<Effect> {
    let cancellation = list.next_generation();
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

fn loadable<T>(result: gitcomet_core::services::Result<T>) -> Loadable<T> {
    match result {
        Ok(value) => Loadable::Ready(value),
        Err(error) => Loadable::Error(error.to_string()),
    }
}

fn with_repo(
    state: &mut AppState,
    repo_id: RepoId,
    f: impl FnOnce(&mut RepoState) -> Vec<Effect>,
) -> Vec<Effect> {
    match state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        Some(repo) => f(repo),
        None => Vec::new(),
    }
}

fn with_session(
    state: &mut AppState,
    repo_id: RepoId,
    view: DiffViewId,
    f: impl FnOnce(u64, &mut DiffSession) -> Vec<Effect>,
) -> Vec<Effect> {
    with_repo(state, repo_id, |repo| {
        if !repo.diff_sessions.contains_key(&view) {
            return Vec::new();
        }
        let lifetime = repo.lifetime();
        let session = Arc::make_mut(&mut repo.diff_sessions)
            .get_mut(&view)
            .expect("checked above");
        f(lifetime, session)
    })
}

/// Starts a new generation and loads what the target shows. The plan is the
/// selected diff's: a file target reads its patch and text (or image), and a
/// whole-commit target its patch.
fn load(
    repo_id: RepoId,
    lifetime: u64,
    view: DiffViewId,
    session: &mut DiffSession,
) -> Vec<Effect> {
    let cancellation = session.next_generation();
    let preview = util::diff_target_preview_flags(&session.target);
    let has_file = session.target.file_path().is_some();
    let image = has_file && preview.wants_image;
    let file_text = has_file && (!preview.wants_image || preview.is_svg);
    session.diff = Loadable::Loading;
    session.file_text = if file_text {
        Loadable::Loading
    } else {
        Loadable::Ready(None)
    };
    session.file_image = if image {
        Loadable::Loading
    } else {
        Loadable::Ready(None)
    };
    session.blame = Loadable::NotLoaded;
    vec![Effect::DiffSession(DiffSessionEffect {
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
        },
        cancellation,
    })]
}
