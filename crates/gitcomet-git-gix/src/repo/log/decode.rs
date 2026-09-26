use super::*;

pub(crate) fn reference_commit_id(
    mut reference: gix::Reference<'_>,
) -> Result<Option<gix::ObjectId>> {
    match reference.peel_to_commit() {
        Ok(commit) => Ok(Some(commit.id().detach())),
        // gix reports peeling to a tree or blob as a validation failure.
        Err(e) if e.is_validation() => Ok(None),
        Err(e) => {
            let ref_name = reference.name().as_bstr().to_str_lossy();
            Err(Error::new(ErrorKind::Backend(format!(
                "gix peel commit ref {ref_name}: {e}"
            ))))
        }
    }
}

/// A normalized author filter.
///
/// Normalizing once, here, is what keeps the needle, the head-page cache key and
/// the paged-walk cache key spelling the same filter the same way: a cache hit
/// that disagreed with the walk cache would hand back a resume token the walk
/// cache then rejects, turning an O(1) resume into a fresh walk of the history.
///
/// Folding is ASCII-only, matching the author picker in the UI, so a name picked
/// from that list is exactly the name the walk looks for. The needle must be
/// folded the same way it is compared — an `str::to_lowercase` needle tested
/// with `eq_ignore_ascii_case` cannot match its own name back, because a
/// Unicode-lowercased 'Á' never equals the 'Á' it came from.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AuthorFilter(String);

impl AuthorFilter {
    /// `None` for "every author" — which an all-whitespace filter also means.
    pub(crate) fn new(author: Option<&str>) -> Option<Self> {
        let author = author?.trim();
        (!author.is_empty()).then(|| Self(author.to_ascii_lowercase()))
    }

    /// Case-insensitive substring match against an author name. Allocation-free,
    /// because this runs once per visited commit.
    pub(crate) fn matches(&self, name: &[u8]) -> bool {
        let needle = self.0.as_bytes();
        name.len() >= needle.len()
            && name
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle))
    }
}

/// Decodes one commit, or returns `None` when `author_filter` rejects it.
///
/// The author is read straight off the decoded object and tested *before*
/// anything is built from it, so a commit the filter rejects costs one object
/// read and no allocation. On a repository the size of Chromium a filtered page
/// visits every one of ~1.8M commits, so what the rejected ones cost is what
/// the whole operation costs.
///
/// Takes the walk's fields rather than its `Info`, so the decoders can be handed
/// a batch to split between them without cloning the parent ids of every commit
/// visited.
pub(crate) fn commit_from_walk_parts<F: gix::objs::Find + Clone>(
    objects: &F,
    id: &gix::oid,
    parent_ids: &[gix::ObjectId],
    commit_time: Option<gix::date::SecondsSinceUnixEpoch>,
    decode_state: &mut CommitDecodeState,
    author_filter: Option<&AuthorFilter>,
) -> Result<Option<Commit>> {
    let mut decode = |objects: &F| {
        objects
            .find_commit(id, &mut decode_state.decode_buf)
            .map(|commit| {
                commit_from_decoded(
                    &commit,
                    id,
                    parent_ids.iter().map(|id| id.as_ref()),
                    commit_time,
                    &mut decode_state.author_cache,
                    &mut decode_state.next_commit_id_cache,
                    author_filter,
                )
            })
    };
    let result = match decode(objects) {
        // Parallel decoders share gix's lazily loaded pack indices. A lookup
        // can miss while another handle loads an index; cloning the handle
        // collects the store's current snapshot. Retry only that missing
        // object, once, so a persistent hole still fails normally.
        Err(e) if e.is_not_found() => decode(&objects.clone()),
        result => result,
    };
    result.map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit object: {e}"))))?
}

pub(crate) fn commit_from_decoded<'a>(
    commit: &gix::objs::CommitRef<'_>,
    id: &gix::oid,
    parent_ids: impl ExactSizeIterator<Item = &'a gix::oid>,
    commit_time: Option<gix::date::SecondsSinceUnixEpoch>,
    author_cache: &mut RepeatedAuthorCache,
    next_commit_id_cache: &mut NextCommitIdCache,
    author_filter: Option<&AuthorFilter>,
) -> Result<Option<Commit>> {
    let author_name = commit.author().map(|author| author.name).ok();
    if let Some(filter) = author_filter
        && !author_name.is_some_and(|name| filter.matches(name.as_ref()))
    {
        return Ok(None);
    }

    let summary_bytes = commit.message.lines().next().unwrap_or_default();
    let summary = bstr_to_arc_str(summary_bytes);

    let author = match author_name {
        Some(name) => author_cache.intern(name.as_ref()),
        None => Arc::from("unknown"),
    };

    let seconds =
        commit_time.unwrap_or_else(|| commit.committer().map(|t| t.seconds()).unwrap_or(0));
    let time = unix_seconds_to_system_time_or_epoch(seconds);

    let commit_id = next_commit_id_cache.reuse_or_new(id, || CommitId(oid_to_arc_str(id)));

    let mut ids = CommitParentIds::new();
    ids.reserve(parent_ids.len());
    if parent_ids.len() == 0 {
        next_commit_id_cache.clear();
    }
    for (index, parent_id) in parent_ids.enumerate() {
        let parent_commit_id = CommitId(oid_to_arc_str(parent_id));
        if index == 0 {
            next_commit_id_cache.remember(parent_id, &parent_commit_id);
        }
        ids.push(parent_commit_id);
    }

    Ok(Some(Commit {
        id: commit_id,
        parent_ids: ids,
        summary,
        author,
        time,
    }))
}

#[derive(Default)]
pub(crate) struct CommitDecodeState {
    decode_buf: Vec<u8>,
    pub(crate) author_cache: RepeatedAuthorCache,
    pub(crate) next_commit_id_cache: NextCommitIdCache,
}

#[derive(Default)]
pub(crate) struct RepeatedAuthorCache {
    raw_name: Vec<u8>,
    value: Option<Arc<str>>,
}

impl RepeatedAuthorCache {
    pub(crate) fn intern(&mut self, name: &[u8]) -> Arc<str> {
        if let Some(value) = self.value.as_ref()
            && self.raw_name.as_slice() == name
        {
            return Arc::clone(value);
        }

        self.raw_name.clear();
        self.raw_name.extend_from_slice(name);
        let value = bstr_to_arc_str(name);
        self.value = Some(Arc::clone(&value));
        value
    }
}

#[derive(Default)]
pub(crate) struct NextCommitIdCache {
    raw_id: Vec<u8>,
    value: Option<CommitId>,
}

impl NextCommitIdCache {
    pub(crate) fn reuse_or_new(&self, oid: &gix::oid, make: impl FnOnce() -> CommitId) -> CommitId {
        if let Some(value) = self.value.as_ref()
            && self.raw_id.as_slice() == oid.as_bytes()
        {
            return value.clone();
        }
        make()
    }

    pub(crate) fn remember(&mut self, oid: &gix::oid, value: &CommitId) {
        self.raw_id.clear();
        self.raw_id.extend_from_slice(oid.as_bytes());
        self.value = Some(value.clone());
    }

    pub(crate) fn clear(&mut self) {
        self.raw_id.clear();
        self.value = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const COMMIT: &[u8] = b"tree 0000000000000000000000000000000000000000\nauthor Alice <alice@example.com> 1600000000 +0000\ncommitter Alice <alice@example.com> 1600000000 +0000\n\npacked commit\n";

    struct SnapshotFind<'a> {
        visible: bool,
        data: Option<&'a [u8]>,
        kind: gix::objs::Kind,
        reads: &'a Cell<usize>,
        snapshots: &'a Cell<usize>,
    }

    impl Clone for SnapshotFind<'_> {
        fn clone(&self) -> Self {
            self.snapshots.set(self.snapshots.get() + 1);
            Self {
                visible: true,
                ..*self
            }
        }
    }

    impl gix::objs::Find for SnapshotFind<'_> {
        fn try_find<'a>(
            &self,
            _id: &gix::oid,
            buffer: &'a mut Vec<u8>,
        ) -> gix::ExnResult<Option<gix::objs::Data<'a>>> {
            self.reads.set(self.reads.get() + 1);
            let Some(data) = self.data.filter(|_| self.visible) else {
                return Ok(None);
            };
            buffer.clear();
            buffer.extend_from_slice(data);
            Ok(Some(gix::objs::Data {
                kind: self.kind,
                object_hash: gix::hash::Kind::Sha1,
                data: buffer,
            }))
        }
    }

    #[test]
    fn commit_decode_refreshes_a_stale_snapshot_once() {
        let reads = Cell::new(0);
        let snapshots = Cell::new(0);
        let objects = SnapshotFind {
            visible: false,
            data: Some(COMMIT),
            kind: gix::objs::Kind::Commit,
            reads: &reads,
            snapshots: &snapshots,
        };
        let id = gix::ObjectId::from_hex(b"1bdfb604b6e50f9515fd638275b11423341540a5").unwrap();
        let parent = gix::ObjectId::null(gix::hash::Kind::Sha1);
        let commit = commit_from_walk_parts(
            &objects,
            &id,
            &[parent],
            Some(1600000001),
            &mut CommitDecodeState::default(),
            AuthorFilter::new(Some("alice")).as_ref(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(commit.id.as_ref(), id.to_string());
        assert_eq!(commit.summary.as_ref(), "packed commit");
        assert_eq!(commit.author.as_ref(), "Alice");
        assert_eq!(commit.parent_ids[0].as_ref(), parent.to_string());
        assert_eq!(
            commit.time,
            unix_seconds_to_system_time_or_epoch(1600000001)
        );
        assert_eq!(reads.get(), 2);
        assert_eq!(snapshots.get(), 1);
    }

    #[test]
    fn commit_decode_preserves_missing_and_corrupt_object_errors() {
        for (data, kind, expected_reads, expected_snapshots, error_text) in [
            (None, gix::objs::Kind::Commit, 2, 1, "could not be found"),
            (
                Some(b"invalid commit".as_slice()),
                gix::objs::Kind::Commit,
                1,
                0,
                "Could not decode object",
            ),
            (
                Some(COMMIT),
                gix::objs::Kind::Blob,
                1,
                0,
                "Expected object of kind",
            ),
        ] {
            let reads = Cell::new(0);
            let snapshots = Cell::new(0);
            let objects = SnapshotFind {
                visible: true,
                data,
                kind,
                reads: &reads,
                snapshots: &snapshots,
            };
            let error = commit_from_walk_parts(
                &objects,
                &gix::ObjectId::null(gix::hash::Kind::Sha1),
                &[],
                None,
                &mut CommitDecodeState::default(),
                None,
            )
            .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Backend(message) if message.contains(error_text))
            );
            assert_eq!(reads.get(), expected_reads);
            assert_eq!(snapshots.get(), expected_snapshots);
        }
    }
}
