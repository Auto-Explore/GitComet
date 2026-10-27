use super::*;
use gitcomet_core::services::{
    HistoryReadRequest, HistoryReadResult, HistoryWalk, refresh_history_page,
};

impl GixRepo {
    pub(in super::super) fn read_history_impl(
        &self,
        walk: &HistoryWalk,
        author: Option<&str>,
        focus: &HistoryFocusSet,
        request: &HistoryReadRequest,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(LogChunk),
    ) -> Result<HistoryReadResult> {
        cancellation.check_cancelled()?;
        let (store, shared) = self.fresh_history_store()?;
        let repo = store.to_thread_local();
        let query = self.resolve_history_query(
            &repo,
            shared.id,
            shared.common.id,
            walk,
            author,
            focus,
            cancellation,
        )?;
        let mode = walk.mode();
        let snapshot = shared.snapshot(&query);
        let tips = query.tips;
        let shallow = query.shallow;
        let author = query.author;
        cancellation.check_cancelled()?;
        match request {
            HistoryReadRequest::Refresh {
                snapshot: Some(known),
                ..
            } if known == &snapshot => {
                cancellation.check_cancelled()?;
                return Ok(HistoryReadResult::Unchanged);
            }
            HistoryReadRequest::Page {
                cursor: Some(_),
                snapshot: known,
                ..
            } if known.as_ref() != Some(&snapshot) => return Ok(HistoryReadResult::Invalidated),
            _ => {}
        }

        let _scope = gitcomet_core::git_ops_trace::scope(
            gitcomet_core::git_ops_trace::GitOpTraceKind::LogWalk,
        );
        let seed = if !focus.is_empty() || mode == HistoryMode::AllBranches {
            super::LogPageSeed::Tips(Arc::clone(&tips))
        } else {
            super::LogPageSeed::Head(tips.first().copied())
        };
        let read = |limit, cursor: Option<&LogCursor>, chunks| {
            let mut key = self.log_page_cache_key(
                mode,
                seed.clone(),
                &shallow,
                limit,
                cursor,
                author.as_ref(),
            );
            key.generation = shared.mapping_id;
            if let Some(page) = self.cached_log_page(&key) {
                return Ok(page);
            }
            let page = self.log_paged_page_from_store(
                &store,
                &shared,
                mode,
                Arc::clone(&tips),
                &shallow,
                limit,
                cursor,
                Some(cancellation),
                author.as_ref(),
                chunks,
            )?;
            self.finish_log_page(key, page, Some(cancellation))
        };
        let page = match request {
            HistoryReadRequest::Page { limit, cursor, .. } => {
                let mut chunks = ChunkEmitter::new(on_chunk);
                read(
                    *limit,
                    cursor.as_ref(),
                    cursor.is_none().then_some(&mut chunks),
                )?
            }
            HistoryReadRequest::Refresh { previous, .. } => {
                refresh_history_page(previous, cancellation, |limit, cursor| {
                    read(limit, cursor, None)
                })?
            }
        };
        cancellation.check_cancelled()?;
        Ok(HistoryReadResult::Page {
            page,
            snapshot: Some(snapshot),
        })
    }
}
