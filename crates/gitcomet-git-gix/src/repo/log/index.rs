use super::*;
use gitcomet_core::history_index::{
    HistoryIndex, HistoryIndexBuilder, HistoryIndexHandle, HistoryIndexProgress, HistoryRange,
};
use gitcomet_core::services::{HistorySnapshot, HistoryWalk};
use std::ops::Range;
use std::time::{Duration, Instant};

/// Everything a history walk reads. Excluded refs are already applied to
/// `tips`, so two walks that start from the same commits share results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct HistoryQuery {
    pub common: u64,
    pub generation: u64,
    pub mode: HistoryMode,
    pub author: Option<AuthorFilter>,
    pub focus: HistoryFocusSet,
    pub tips: Arc<[gix::ObjectId]>,
    pub shallow: ShallowSnapshot,
}

impl HistoryQuery {
    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot(format!("gix-history:{self:?}").into())
    }
}

impl GixRepo {
    pub(in super::super) fn build_history_index_impl(
        &self,
        walk: &HistoryWalk,
        author: Option<&str>,
        focus: &HistoryFocusSet,
        cancellation: &CancellationToken,
        on_progress: &mut dyn FnMut(HistoryIndexProgress),
    ) -> Result<Option<HistoryIndexHandle>> {
        cancellation.check_cancelled()?;
        let (store, shared) = self.current_history_store()?;
        let query = self.resolve_history_query(
            &store.to_thread_local(),
            shared.id,
            shared.common.id,
            walk,
            author,
            focus,
            cancellation,
        )?;
        shared
            .index(store, query, cancellation, on_progress)
            .map(Some)
    }

    pub(in super::super) fn resolve_history_query(
        &self,
        repo: &gix::Repository,
        generation: u64,
        common: u64,
        walk: &HistoryWalk,
        author: Option<&str>,
        focus: &HistoryFocusSet,
        cancellation: &CancellationToken,
    ) -> Result<HistoryQuery> {
        let shallow = shallow_snapshot(repo)?;
        let mode = walk.mode();
        let tips = self.history_tips(repo, walk, focus, Some(cancellation))?;
        Ok(HistoryQuery {
            common,
            generation,
            mode,
            author: AuthorFilter::new(author),
            focus: focus.clone(),
            tips,
            shallow,
        })
    }

    pub(in super::super) fn build_resolved_history_index(
        store: &gix::ThreadSafeRepository,
        query: &HistoryQuery,
        snapshot: HistorySnapshot,
        topology_cache: &TopologyCache,
        cancellation: &CancellationToken,
        on_progress: &mut dyn FnMut(HistoryIndexProgress),
    ) -> Result<HistoryIndexHandle> {
        let _trace = gitcomet_core::git_ops_trace::scope(
            gitcomet_core::git_ops_trace::GitOpTraceKind::LogWalk,
        );
        cancellation.check_cancelled()?;
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::HistoryIndexBuild);
        let repo = store.to_thread_local();
        let mode = query.mode;
        let author = &query.author;
        let hash_len = repo.object_hash().len_in_bytes();
        let mut walk = new_whole_history_walk(
            store,
            &query.tips,
            mode,
            &query.shallow,
            cancellation,
            (topology_cache, query.generation),
        )?;
        // A whole history in date order is held as rows already.
        if author.is_none()
            && matches!(mode, HistoryMode::FullReachable | HistoryMode::AllBranches)
            && let super::LogPagedWalk::CachedDateOrder(topology) = &walk.walk
        {
            let tables = topology
                .index_tables(&|| cancellation.is_cancelled())
                .map_err(|error| crate::repo::object_store::gix_error("gix history index", error))
                .and_then(|tables| {
                    cancellation.check_cancelled()?;
                    Ok(tables)
                })?;
            drop(walk);
            let rows = (tables.ids.len() / hash_len) as u64;
            on_progress(HistoryIndexProgress {
                scanned: rows,
                matched: rows,
            });
            let mut decode_buf = Vec::new();
            let mut stashes = Vec::new();
            for row in tables.stash_shaped {
                cancellation.check_cancelled()?;
                let id = &tables.ids[row as usize * hash_len..][..hash_len];
                if is_stash_message(&repo, gix::oid::from_bytes_unchecked(id), &mut decode_buf)? {
                    stashes.push(row);
                }
            }
            return HistoryIndex::from_rows(
                snapshot,
                mode,
                hash_len,
                tables.ids,
                tables.parent_offsets,
                tables.parents,
                stashes,
                cancellation,
            );
        }
        let mut builder = HistoryIndexBuilder::new(snapshot, mode, hash_len)?;
        let mut decode_buf = Vec::new();
        let mut shapes = StashShapes::default();
        let mut scanned = 0u64;
        let mut last_progress = Instant::now();
        for info in &mut walk.walk {
            cancellation.check_cancelled()?;
            let info = info.map_err(|error| {
                crate::repo::object_store::gix_error("gix history index", error)
            })?;
            shapes.visit(info.id, &info.parent_ids);
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
                        crate::repo::object_store::gix_error("gix history index object", error)
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
        // classification reads candidate messages.
        drop(walk);
        drop(decode_buf);

        retain_probable_stashes(&mut builder, &repo, &shapes, author.is_some(), cancellation)?;
        builder.finish(cancellation)
    }

    pub(in super::super) fn read_history_range_impl(
        &self,
        index: &HistoryIndexHandle,
        range: Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<HistoryRange> {
        cancellation.check_cancelled()?;
        let (_, shared) = self.current_history_store()?;
        self.validate_history_store(index, &shared)?;
        self.decode_history_range(index, range, cancellation)
    }

    /// Reads `range` through the rotating range reader, for a caller that has
    /// validated `index` against the current store.
    pub(in super::super) fn decode_history_range(
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
                    crate::repo::object_store::gix_error("gix history range", error)
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

/// Rules stash candidates out by shape as a walk passes their parents. A
/// stash's second parent is its index, one commit on the first parent alone,
/// and a third holds the untracked files and has no parent. A merge is
/// neither, and its message is then never read.
#[derive(Default)]
pub(super) struct StashShapes {
    /// For a commit that candidates name as a parent: each candidate, and
    /// the one parent the commit may have (or none) if that is a stash.
    awaited: FxHashMap<gix::ObjectId, StashParents>,
    ruled_out: FxHashSet<gix::ObjectId>,
}

type StashParents = smallvec::SmallVec<[(gix::ObjectId, Option<gix::ObjectId>); 1]>;

impl StashShapes {
    /// Takes the commits of a walk in its order, children before parents. A
    /// parent the walk never passes rules nothing out.
    pub(super) fn visit(&mut self, id: gix::ObjectId, parents: &[gix::ObjectId]) {
        self.passed(id, Some(parents));
        let (base, index, untracked) = match parents {
            [base, index] => (base, index, None),
            [base, index, untracked] => (base, index, Some(untracked)),
            _ => return,
        };
        self.awaited
            .entry(*index)
            .or_default()
            .push((id, Some(*base)));
        if let Some(untracked) = untracked {
            self.awaited.entry(*untracked).or_default().push((id, None));
        }
    }

    /// Rules on the candidates that name `id` as a parent, from `id`'s own
    /// parents; `None` when it has parents that cannot all be named.
    pub(super) fn passed(&mut self, id: gix::ObjectId, parents: Option<&[gix::ObjectId]>) {
        for (candidate, only_parent) in self.awaited.remove(&id).unwrap_or_default() {
            if parents != Some(only_parent.as_slice()) {
                self.ruled_out.insert(candidate);
            }
        }
    }

    /// The parents candidates name that no visit has passed yet.
    pub(super) fn awaited(&self) -> Vec<gix::ObjectId> {
        self.awaited.keys().copied().collect()
    }

    pub(super) fn rules_out(&self, candidate: &gix::oid) -> bool {
        self.ruled_out.contains(candidate)
    }
}

/// Keeps the stash candidates whose shape and summary are a stash's.
/// `messages_checked` when an author filter has already read the messages.
pub(super) fn retain_probable_stashes(
    builder: &mut HistoryIndexBuilder,
    repo: &gix::Repository,
    shapes: &StashShapes,
    messages_checked: bool,
    cancellation: &CancellationToken,
) -> Result<()> {
    let mut decode_buf = Vec::new();
    builder.retain_probable_stashes(|id, _| {
        cancellation.check_cancelled()?;
        let id = gix::oid::from_bytes_unchecked(id);
        if shapes.rules_out(id) {
            return Ok(false);
        }
        if messages_checked {
            return Ok(true);
        }
        is_stash_message(repo, id, &mut decode_buf)
    })
}

/// Reads a stash candidate's message for the summary Git gives a stash.
fn is_stash_message(repo: &gix::Repository, id: &gix::oid, buffer: &mut Vec<u8>) -> Result<bool> {
    gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::IndexObjectRead);
    let commit = repo.objects.find_commit(id, buffer).map_err(|error| {
        Error::new(ErrorKind::Backend(format!(
            "gix history index object: {error}"
        )))
    })?;
    Ok(has_stash_summary(commit.message))
}

fn has_stash_summary(message: &[u8]) -> bool {
    let summary = message.lines().next().unwrap_or_default();
    (summary.starts_with(b"WIP on ") || summary.starts_with(b"On "))
        && summary.windows(2).any(|pair| pair == b": ")
}
