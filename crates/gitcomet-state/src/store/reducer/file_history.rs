use super::prepared::ReducerInputs;
use super::util::push_diagnostic;
use crate::model::{AppState, DiagnosticKind, Loadable, RepoId};
use crate::msg::Effect;
use gitcomet_core::domain::{LogCursor, LogPage};
use gitcomet_core::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

/// "Everything after the cursor" for the rest of a file's history: cursor
/// pages come from one cached follow walk, so one request costs the same as
/// many and the picker gets a complete, searchable list.
const FILE_HISTORY_REMAINDER_LIMIT: usize = usize::MAX;

pub(super) fn file_history_loaded(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    request: crate::model::FileHistoryRequestId,
    path: PathBuf,
    cursor: Option<LogCursor>,
    result: std::result::Result<Arc<LogPage>, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !repo_state
        .history_state
        .accepts_file_history(request, &path, cursor.as_ref())
    {
        return Vec::new();
    }

    let page = match cursor {
        None => match result {
            Ok(page) => page,
            Err(e) => {
                push_diagnostic(inputs, repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.history_state.file_history = Loadable::Error(e.to_string());
                return Vec::new();
            }
        },
        Some(_) => {
            let Loadable::Ready(current) = &repo_state.history_state.file_history else {
                return Vec::new();
            };
            let mut commits = current.commits.clone();
            let next_cursor = match result {
                Ok(rest) => {
                    let rest = Arc::unwrap_or_clone(rest);
                    commits.extend(rest.commits);
                    rest.next_cursor
                }
                Err(e) => {
                    // Keep what is loaded; clearing the cursor stops the
                    // picker from reporting that older commits are coming.
                    push_diagnostic(inputs, repo_state, DiagnosticKind::Error, e.to_string());
                    None
                }
            };
            Arc::new(LogPage {
                commits,
                next_cursor,
            })
        }
    };

    let effects = page
        .next_cursor
        .clone()
        .map(|cursor| Effect::LoadFileHistory {
            request,
            repo_id,
            path,
            limit: FILE_HISTORY_REMAINDER_LIMIT,
            cursor: Some(cursor),
        })
        .into_iter()
        .collect();
    repo_state.history_state.file_history = Loadable::Ready(page);
    effects
}

pub(super) fn load_file_history(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    limit: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let request = repo_state.history_state.begin_file_history(path.clone());
    vec![Effect::LoadFileHistory {
        request,
        repo_id,
        path,
        limit,
        cursor: None,
    }]
}
