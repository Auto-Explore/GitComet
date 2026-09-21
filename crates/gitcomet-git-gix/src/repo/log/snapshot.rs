use super::*;
use gitcomet_core::services::{
    HistoryReadRequest, HistoryReadResult, HistorySnapshot, refresh_history_page,
};

impl GixRepo {
    pub(in super::super) fn read_history_impl(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        request: &HistoryReadRequest,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(LogChunk),
    ) -> Result<HistoryReadResult> {
        cancellation.check_cancelled()?;
        let repo = self._repo.to_thread_local();
        let shallow = shallow_snapshot(&repo)?;
        let tips = if mode == HistoryMode::AllBranches {
            self.all_branches_tips(&repo, Some(cancellation))?
        } else {
            Arc::from(gix_head_id_or_none(&repo)?.into_iter().collect::<Vec<_>>())
        };
        let author = AuthorFilter::new(author);
        // These are exact, unambiguous Debug encodings of typed inputs, not a
        // sampled hash or filesystem timestamp. The same captured values feed
        // every batch below, even if refs change while the walk is running.
        let snapshot = HistorySnapshot(format!("{mode:?}|{author:?}|{tips:?}|{shallow:?}").into());
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
        let seed = if mode == HistoryMode::AllBranches {
            super::super::LogPageSeed::Tips(Arc::clone(&tips))
        } else {
            super::super::LogPageSeed::Head(tips.first().copied())
        };
        let read = |limit, cursor: Option<&LogCursor>, chunks| {
            let key = self.log_page_cache_key(
                mode,
                seed.clone(),
                &shallow,
                limit,
                cursor,
                author.as_ref(),
            );
            if let Some(page) = self.cached_log_page(&key) {
                return Ok(page);
            }
            let page = self.log_paged_page(
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
