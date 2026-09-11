use super::*;
use gitcomet_core::history_index::{
    HistoryIndexBuilder, HistoryIndexHandle, HistoryIndexProgress, HistoryRange,
};
use gitcomet_core::services::HistorySnapshot;
use std::ops::Range;
use std::time::{Duration, Instant};

impl GixRepo {
    pub(in super::super) fn build_history_index_impl(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        cancellation: &CancellationToken,
        on_progress: &mut dyn FnMut(HistoryIndexProgress),
    ) -> Result<Option<HistoryIndexHandle>> {
        let _trace = gitcomet_core::git_ops_trace::scope(
            gitcomet_core::git_ops_trace::GitOpTraceKind::LogWalk,
        );
        cancellation.check_cancelled()?;
        let repo = self._repo.to_thread_local();
        let shallow = shallow_snapshot(&repo)?;
        let tips = if mode == HistoryMode::AllBranches {
            self.all_branches_tips(&repo, Some(cancellation))?
        } else {
            Arc::from(gix_head_id_or_none(&repo)?.into_iter().collect::<Vec<_>>())
        };
        let author = AuthorFilter::new(author);
        let snapshot = HistorySnapshot(format!("{mode:?}|{author:?}|{tips:?}|{shallow:?}").into());
        let mut builder =
            HistoryIndexBuilder::new(snapshot, mode, repo.object_hash().len_in_bytes())?;
        let mut walk = new_log_paged_walk(
            &self._repo,
            tips.iter().copied(),
            mode,
            &shallow,
            Some(cancellation),
            None,
        )?;
        let mut decode_buf = Vec::new();
        let mut scanned = 0u64;
        let mut last_progress = Instant::now();
        for info in &mut walk.walk {
            cancellation.check_cancelled()?;
            let info = info.map_err(|error| {
                Error::new(ErrorKind::Backend(format!("gix history index: {error}")))
            })?;
            scanned += 1;
            if scanned.is_multiple_of(1024) && last_progress.elapsed() >= Duration::from_millis(100)
            {
                on_progress(HistoryIndexProgress {
                    scanned,
                    matched: builder.len() as u64,
                });
                last_progress = Instant::now();
            }
            if !mode_includes(mode, info.parent_ids.len()) {
                continue;
            }
            // Author filtering requires object headers. The stash heuristic
            // only needs messages from commits with two or three parents.
            let stash_candidate = (2..=3).contains(&info.parent_ids.len());
            let mut probable_stash = false;
            let mut commit_time = info.commit_time;
            if author.is_some() || stash_candidate || commit_time.is_none() {
                let commit = repo
                    .objects
                    .find_commit(info.id.as_ref(), &mut decode_buf)
                    .map_err(|error| {
                        Error::new(ErrorKind::Backend(format!(
                            "gix history index object: {error}"
                        )))
                    })?;
                if commit_time.is_none() {
                    commit_time = Some(
                        commit
                            .committer()
                            .map(|signature| signature.seconds())
                            .unwrap_or(0),
                    );
                }
                if let Some(author) = &author
                    && !commit
                        .author()
                        .ok()
                        .is_some_and(|signature| author.matches(signature.name.as_ref()))
                {
                    continue;
                }
                if stash_candidate {
                    let summary = commit.message.lines().next().unwrap_or_default();
                    probable_stash = (summary.starts_with(b"WIP on ")
                        || summary.starts_with(b"On "))
                        && summary.windows(2).any(|pair| pair == b": ");
                }
            }
            builder.push(
                info.id.as_bytes(),
                info.parent_ids.iter().map(|id| id.as_bytes()),
                commit_time.unwrap_or(0),
                probable_stash,
            )?;
        }
        on_progress(HistoryIndexProgress {
            scanned,
            matched: builder.len() as u64,
        });
        builder.finish(cancellation).map(Some)
    }

    pub(in super::super) fn read_history_range_impl(
        &self,
        index: &HistoryIndexHandle,
        range: Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<HistoryRange> {
        cancellation.check_cancelled()?;
        if range.start > range.end || range.end > index.len() {
            return Err(Error::new(ErrorKind::Backend(
                "history range is outside its snapshot".into(),
            )));
        }
        let repo = self._repo.to_thread_local();
        let mut decode = CommitDecodeState::default();
        let mut header_buf = Vec::new();
        let mut commits = Vec::with_capacity(range.len());
        for row in range.clone() {
            cancellation.check_cancelled()?;
            let id =
                gix::ObjectId::from_bytes_or_panic(index.id_bytes(row).expect("validated range"));
            let object = repo
                .objects
                .find_commit(id.as_ref(), &mut header_buf)
                .map_err(|error| {
                    Error::new(ErrorKind::Backend(format!("gix history range: {error}")))
                })?;
            let parents: Vec<_> = object.parents().take(index.parents(row).len()).collect();
            // The original walk determines parent count (first-parent and
            // shallow histories can differ from the raw object's parents).
            let commit = commit_from_walk_parts(
                &repo,
                id.as_ref(),
                &parents,
                index.timestamp(row),
                &mut decode,
                None,
            )?
            .expect("unfiltered decoding produces a commit");
            commits.push(commit);
        }
        cancellation.check_cancelled()?;
        Ok(HistoryRange {
            snapshot: index.snapshot.clone(),
            start: range.start,
            commits,
        })
    }
}
