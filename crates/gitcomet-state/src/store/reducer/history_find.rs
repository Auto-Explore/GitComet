use super::*;
use crate::history_find::{HistoryFindEffect, HistoryFindMsg, HistoryFindState};

pub(super) fn reduce(state: &mut AppState, event: HistoryFindMsg) -> Vec<Effect> {
    let repo_id = match &event {
        HistoryFindMsg::Find { repo_id, .. } | HistoryFindMsg::Found { repo_id, .. } => *repo_id,
        HistoryFindMsg::Close => {
            for repo in &mut state.repos {
                repo.history_state.find.close();
            }
            return Vec::new();
        }
    };
    // Keep decoded text for the active search only. Switching among large
    // repositories must not retain an unbounded cache per visited tab.
    if matches!(
        &event,
        HistoryFindMsg::Find {
            query: Some(_),
            index: Some(_),
            ..
        }
    ) {
        for repo in &mut state.repos {
            if repo.id != repo_id {
                repo.history_state.find.close();
            }
        }
    }
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let stashes = match &repo.stashes {
        Loadable::Ready(stashes) => Arc::clone(stashes),
        _ => Default::default(),
    };
    let find = &mut repo.history_state.find;
    match event {
        HistoryFindMsg::Find { query, index, .. } => {
            if let (Some(query), Some(index)) = (&query, &index)
                && find.is_for(query, index, repo.stashes_rev)
                && find.error.is_none()
            {
                return Vec::new();
            }
            find.cancellation.cancel();
            // Stopping keeps the commit text: the view stops for passing
            // states (a regex mid-typing, a scope change, a tab switch).
            *find = HistoryFindState {
                seq: find.seq.wrapping_add(1),
                rev: find.rev.wrapping_add(1),
                cache: Arc::clone(&find.cache),
                ..Default::default()
            };
            let (Some(query), Some(index)) = (query, index) else {
                return Vec::new();
            };
            find.query = Some(query.clone());
            find.stashes_rev = repo.stashes_rev;
            find.index = Some(Arc::downgrade(&index));
            // An invalid regex matches nothing.
            if index.is_empty() || query.regex_error().is_some() {
                find.done = true;
                return Vec::new();
            }
            vec![Effect::HistoryFind(HistoryFindEffect {
                repo_id,
                seq: find.seq,
                index,
                query,
                stashes,
                cancellation: find.cancellation.clone(),
                cache: Arc::clone(&find.cache),
            })]
        }
        HistoryFindMsg::Found { seq, result, .. } => {
            if seq != find.seq || find.done || find.error.is_some() {
                return Vec::new();
            }
            match result {
                Ok(chunk) => {
                    if !chunk.matches.is_empty() {
                        Arc::make_mut(&mut find.matches).push(chunk.matches.into());
                    }
                    find.done = chunk.done;
                }
                Err(error) => {
                    find.error = Some(error.to_string());
                }
            }
            find.rev = find.rev.wrapping_add(1);
            Vec::new()
        }
        // Handled for every repo above.
        HistoryFindMsg::Close => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
