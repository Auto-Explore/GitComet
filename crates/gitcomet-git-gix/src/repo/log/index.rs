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
        let (store, generation) = self.thread_safe_repo();
        let repo = store.to_thread_local();
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
            &store,
            tips.iter().copied(),
            mode,
            &shallow,
            Some(cancellation),
            None,
            Some((&self.log_topology_cache, generation)),
        )?;
        let mut decode_buf = Vec::new();
        let mut scanned = 0u64;
        let mut last_progress = Instant::now();
        for info in &mut walk.walk {
            cancellation.check_cancelled()?;
            let info = info.map_err(|error| {
                crate::repo::object_store::gix_error("gix history index", &*error)
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
            // Only author filtering needs object headers during traversal.
            // Defer stash classification until the walk's maps are released.
            let mut stash_candidate = (2..=3).contains(&info.parent_ids.len());
            if let Some(author) = &author {
                gitcomet_core::history_perf::record(
                    gitcomet_core::history_perf::Work::IndexObjectRead,
                );
                let commit = repo
                    .objects
                    .find_commit(info.id.as_ref(), &mut decode_buf)
                    .map_err(|error| {
                        crate::repo::object_store::gix_error(
                            "gix history index object",
                            &gix::Error::from(error),
                        )
                    })?;
                if !commit
                    .author()
                    .ok()
                    .is_some_and(|signature| author.matches(signature.name.as_ref()))
                {
                    continue;
                }
                stash_candidate = stash_candidate && has_stash_summary(commit.message);
            }
            builder.push(
                info.id.as_bytes(),
                info.parent_ids.iter().map(|id| id.as_bytes()),
                stash_candidate,
            )?;
        }
        on_progress(HistoryIndexProgress {
            scanned,
            matched: builder.len() as u64,
        });
        // Release the traversal's maps or shared topology before stash
        // classification opens a graph and reads candidate messages.
        drop(walk);
        drop(decode_buf);

        // Author-filtered walks have already checked their messages and no
        // longer need the potentially large author-decoding scratch buffer.
        let mut decode_buf = Vec::new();
        let topology = std::cell::OnceCell::new();
        builder.retain_probable_stashes(|id, parents| {
            cancellation.check_cancelled()?;
            if let Some(graph) =
                topology.get_or_init(|| repo.commit_graph_if_enabled().ok().flatten())
            {
                let parents: smallvec::SmallVec<[gix::ObjectId; 3]> =
                    parents.map(gix::ObjectId::from_bytes_or_panic).collect();
                if stash_shape(graph, &parents) == Some(false) {
                    return Ok(false);
                }
            }
            if author.is_some() {
                // Author filtering already decoded and checked this message.
                return Ok(true);
            }
            gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::IndexObjectRead);
            let commit = repo
                .objects
                .find_commit(gix::oid::from_bytes_unchecked(id), &mut decode_buf)
                .map_err(|error| {
                    Error::new(ErrorKind::Backend(format!(
                        "gix history index object: {error}"
                    )))
                })?;
            Ok(has_stash_summary(commit.message))
        })?;
        drop(topology);
        drop(decode_buf);
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
        let repo = self.range_reader_repo()?;
        let mut decode = CommitDecodeState::default();
        let mut header_buf = Vec::new();
        let mut commits = Vec::with_capacity(range.len());
        for row in range.clone() {
            cancellation.check_cancelled()?;
            let id =
                gix::ObjectId::from_bytes_or_panic(index.id_bytes(row).expect("validated range"));
            gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::RangeObjectRead);
            let object = repo
                .objects
                .find_commit(id.as_ref(), &mut header_buf)
                .map_err(|error| {
                    crate::repo::object_store::gix_error(
                        "gix history range",
                        &gix::Error::from(error),
                    )
                })?;
            // Use precisely the indexed topology (including first-parent,
            // shallow boundaries and parents excluded by an author filter).
            let parents = (0..index.parents(row).len()).map(|parent| {
                gix::oid::from_bytes_unchecked(index.parent_id_bytes(row, parent).unwrap())
            });
            let commit = commit_from_decoded(
                &object,
                id.as_ref(),
                parents,
                None,
                &mut decode.author_cache,
                &mut decode.next_commit_id_cache,
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

fn has_stash_summary(message: &[u8]) -> bool {
    let summary = message.lines().next().unwrap_or_default();
    (summary.starts_with(b"WIP on ") || summary.starts_with(b"On "))
        && summary.windows(2).any(|pair| pair == b": ")
}

/// Only reject when unfiltered commit-graph topology proves the shape impossible.
/// Missing graph entries or malformed edges retain the message-check fallback.
fn stash_shape(graph: &gix::commitgraph::Graph, parents: &[gix::ObjectId]) -> Option<bool> {
    let base = graph.lookup(parents.first()?)?;
    let index = graph.commit_by_id(parents.get(1)?)?;
    let mut edges = index.iter_parents();
    let parent = edges.next().transpose().ok()?;
    if parent != Some(base) || edges.next().transpose().ok()?.is_some() {
        return Some(false);
    }
    if let Some(untracked) = parents.get(2) {
        return Some(
            graph
                .commit_by_id(untracked)?
                .iter_parents()
                .next()
                .transpose()
                .ok()?
                .is_none(),
        );
    }
    Some(true)
}
