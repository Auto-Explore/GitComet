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
use std::sync::Arc;

pub(super) fn reduce(state: &mut AppState, event: Event) -> Vec<Effect> {
    match event {
        Event::Open {
            repo_id,
            lifetime,
            view,
            target,
        } => with_repo(state, repo_id, lifetime, |repo| {
            let (repo_id, lifetime) = (repo.id, repo.lifetime());
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
        if !session.follows_worktree() || session.worktree() != worktree {
            return false;
        }
        if change.index || change.git_state || change.text_attributes {
            return true;
        }
        if !change.worktree
            || matches!(
                session.target,
                gitcomet_core::domain::DiffTarget::WorkingTree {
                    area: gitcomet_core::domain::DiffArea::Staged,
                    ..
                }
            )
        {
            return false;
        }
        session
            .target
            .file_path()
            .is_none_or(|path| change.paths.may_contain(path))
            || session
                .target
                .old_file_path()
                .is_some_and(|path| change.paths.may_contain(path))
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
    );
    let effects = load(repo_id, lifetime, view, session);
    retain_ready(previous.0, &mut session.diff);
    retain_ready(previous.1, &mut session.diff_file);
    retain_ready(previous.2, &mut session.diff_file_image);
    retain_ready(previous.3, &mut session.blame);
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
    if session.diff_target.is_none() {
        return Vec::new();
    }
    let cancellation = session.next_generation();
    let preview = util::diff_target_preview_flags(&session.target);
    let has_file = session.target.file_path().is_some();
    let image = has_file && preview.wants_image;
    let file_text = has_file && (!preview.wants_image || preview.is_svg);
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
        },
        cancellation,
    })];
    if session.blame_requested {
        effects.extend(load_blame(repo_id, lifetime, view, session));
    }
    effects
}
