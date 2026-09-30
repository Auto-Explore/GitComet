//! Diff sessions: open, retarget, reload, blame, and completions. Each
//! session's work is stopped by its own token; completions from an older
//! generation or repository lifetime are dropped.

use super::*;
use crate::diff_session::{
    DiffSession, DiffSessionContent, DiffSessionEffect, DiffSessionMsg as Event, DiffSessionWork,
    DiffViewId,
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

/// Reloads the sessions that follow the working tree after an external edit.
pub(super) fn reload_worktree_sessions(repo: &mut RepoState) -> Vec<Effect> {
    if !repo
        .diff_sessions
        .values()
        .any(DiffSession::follows_worktree)
    {
        return Vec::new();
    }
    let (repo_id, lifetime) = (repo.id, repo.lifetime());
    let mut effects = Vec::new();
    for (view, session) in Arc::make_mut(&mut repo.diff_sessions).iter_mut() {
        if session.follows_worktree() {
            effects.extend(load(repo_id, lifetime, *view, session));
        }
    }
    effects
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
