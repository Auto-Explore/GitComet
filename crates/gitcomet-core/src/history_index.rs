//! Compact, immutable history topology. Text and full commit objects belong in
//! the bounded range cache, not in this table.

use crate::domain::{Commit, CommitId, HistoryMode};
use crate::error::{Error, ErrorKind};
use crate::services::{CancellationToken, HistorySnapshot, Result};
use std::ops::Range;
use std::sync::Arc;

pub const HISTORY_BLOCK_SIZE: usize = 256;
pub const HISTORY_ROW_CACHE_LIMIT: usize = 8_192;
pub const MISSING_PARENT: u32 = u32::MAX;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryIndexProgress {
    pub scanned: u64,
    pub matched: u64,
}

#[derive(Eq, PartialEq)]
pub struct HistoryIndex {
    pub snapshot: HistorySnapshot,
    pub mode: HistoryMode,
    hash_len: usize,
    ids: Vec<u8>,
    sorted_rows: Vec<u32>,
    parent_offsets: Vec<u32>,
    parents: Vec<u32>,
    external_edges: Vec<u32>,
    external_ids: Vec<u8>,
    times: Vec<i64>,
    probable_stashes: Vec<bool>,
}

impl HistoryIndex {
    pub fn len(&self) -> usize {
        self.times.len()
    }
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
    pub fn id_bytes(&self, row: usize) -> Option<&[u8]> {
        let start = row.checked_mul(self.hash_len)?;
        self.ids.get(start..start.checked_add(self.hash_len)?)
    }
    pub fn commit_id(&self, row: usize) -> Option<CommitId> {
        let bytes = self.id_bytes(row)?;
        Some(CommitId(crate::hex::encode(bytes).into()))
    }
    pub fn position(&self, id: &str) -> Option<usize> {
        if id.len() != self.hash_len * 2 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (slot, pair) in bytes.iter_mut().zip(id.as_bytes().as_chunks::<2>().0) {
            let digit = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            *slot = digit(pair[0])? * 16 + digit(pair[1])?;
        }
        self.position_bytes(&bytes[..self.hash_len])
    }
    /// Resolve a full ID or a unique hexadecimal prefix (at least seven digits).
    pub fn resolve_reference(&self, reference: &str) -> Option<usize> {
        if reference.len() == self.hash_len * 2 {
            return self.position(reference);
        }
        if !(7..self.hash_len * 2).contains(&reference.len()) {
            return None;
        }
        let mut lower = [0u8; 32];
        let mut upper = [255u8; 32];
        for (ix, byte) in reference.bytes().enumerate() {
            let digit = (byte as char).to_digit(16)? as u8;
            if ix.is_multiple_of(2) {
                lower[ix / 2] = digit << 4;
                upper[ix / 2] = (digit << 4) | 15;
            } else {
                lower[ix / 2] |= digit;
                upper[ix / 2] = lower[ix / 2];
            }
        }
        let start = self
            .sorted_rows
            .partition_point(|row| self.id_bytes(*row as usize).unwrap() < &lower[..self.hash_len]);
        let row = *self.sorted_rows.get(start)? as usize;
        if self.id_bytes(row)? > &upper[..self.hash_len] {
            return None;
        }
        if self
            .sorted_rows
            .get(start + 1)
            .is_some_and(|row| self.id_bytes(*row as usize).unwrap() <= &upper[..self.hash_len])
        {
            return None;
        }
        Some(row)
    }
    fn position_bytes(&self, id: &[u8]) -> Option<usize> {
        self.sorted_rows
            .binary_search_by(|row| self.id_bytes(*row as usize).expect("indexed row").cmp(id))
            .ok()
            .map(|ix| self.sorted_rows[ix] as usize)
    }
    pub fn parents(&self, row: usize) -> &[u32] {
        match (
            self.parent_offsets.get(row),
            self.parent_offsets.get(row + 1),
        ) {
            (Some(&start), Some(&end)) => &self.parents[start as usize..end as usize],
            _ => &[],
        }
    }
    pub fn parent_commit_id(&self, row: usize, parent: usize) -> Option<CommitId> {
        let rank = *self.parents(row).get(parent)?;
        if rank != MISSING_PARENT {
            return self.commit_id(rank as usize);
        }
        let edge = *self.parent_offsets.get(row)? as usize + parent;
        let ix = self.external_edges.binary_search(&(edge as u32)).ok()? * self.hash_len;
        Some(CommitId(
            crate::hex::encode(&self.external_ids[ix..ix + self.hash_len]).into(),
        ))
    }
    pub fn timestamp(&self, row: usize) -> Option<i64> {
        self.times.get(row).copied()
    }
    pub fn is_probable_stash(&self, row: usize) -> bool {
        self.probable_stashes.get(row).copied().unwrap_or(false)
    }
    pub fn estimated_bytes(&self) -> usize {
        self.ids.capacity()
            + self.sorted_rows.capacity() * 4
            + self.parent_offsets.capacity() * 4
            + self.parents.capacity() * 4
            + self.times.capacity() * 8
            + self.probable_stashes.capacity()
            + self.external_edges.capacity() * 4
            + self.external_ids.capacity()
    }
}

/// A handle pins its immutable topology across refreshes and range reads.
pub type HistoryIndexHandle = Arc<HistoryIndex>;

/// A sparse visibility projection; hiding a handful of stash helpers does not
/// allocate an additional array with one entry for every commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryProjection {
    pub index: HistoryIndexHandle,
    hidden: Arc<[u32]>,
}

impl HistoryProjection {
    pub fn new(index: HistoryIndexHandle, mut hidden: Vec<u32>) -> Self {
        hidden.retain(|&row| (row as usize) < index.len());
        hidden.sort_unstable();
        hidden.dedup();
        Self {
            index,
            hidden: hidden.into(),
        }
    }
    pub fn len(&self) -> usize {
        self.index.len() - self.hidden.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn visible_position(&self, raw: usize) -> Option<usize> {
        if raw >= self.index.len() {
            return None;
        }
        let before = self
            .hidden
            .partition_point(|&hidden| (hidden as usize) < raw);
        if self
            .hidden
            .get(before)
            .is_some_and(|&hidden| hidden as usize == raw)
        {
            None
        } else {
            Some(raw - before)
        }
    }
    pub fn raw_position(&self, visible: usize) -> Option<usize> {
        if visible >= self.len() {
            return None;
        }
        let (mut lo, mut hi) = (visible, visible + self.hidden.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let hidden = self.hidden.partition_point(|&row| row as usize <= mid);
            if mid + 1 - hidden <= visible {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some(lo)
    }
    pub fn position(&self, id: &str) -> Option<usize> {
        self.visible_position(self.index.position(id)?)
    }
    pub fn commit_id(&self, visible: usize) -> Option<CommitId> {
        self.index.commit_id(self.raw_position(visible)?)
    }
    /// Map each old visible row to the closest surviving row in a new
    /// projection. Built once in the background so refresh anchoring is O(1),
    /// even when a rebase removes millions of commits around the viewport.
    pub fn nearest_survivors(
        &self,
        next: &Self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u32>> {
        let mut mapped = vec![MISSING_PARENT; self.index.len()];
        let mut right = 0;
        for (ix, &left_row) in self.index.sorted_rows.iter().enumerate() {
            if ix.is_multiple_of(1024) {
                cancellation.check_cancelled()?;
            }
            let id = self.index.id_bytes(left_row as usize).unwrap();
            while right < next.index.sorted_rows.len()
                && next
                    .index
                    .id_bytes(next.index.sorted_rows[right] as usize)
                    .unwrap()
                    < id
            {
                right += 1;
            }
            if let Some(&right_row) = next.index.sorted_rows.get(right)
                && next.index.id_bytes(right_row as usize).unwrap() == id
            {
                mapped[left_row as usize] = next
                    .visible_position(right_row as usize)
                    .map_or(MISSING_PARENT, |row| row as u32);
            }
        }
        let mut raw = 0;
        mapped.retain(|_| {
            let visible = self.visible_position(raw).is_some();
            raw += 1;
            visible
        });
        let mut start = 0;
        while start < mapped.len() {
            if start.is_multiple_of(1024) {
                cancellation.check_cancelled()?;
            }
            if mapped[start] != MISSING_PARENT {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < mapped.len() && mapped[end] == MISSING_PARENT {
                end += 1;
            }
            let left = start.checked_sub(1).map(|ix| (ix, mapped[ix]));
            let right = mapped.get(end).copied().map(|row| (end, row));
            for (ix, value) in mapped.iter_mut().enumerate().take(end).skip(start) {
                if ix.is_multiple_of(1024) {
                    cancellation.check_cancelled()?;
                }
                *value = match (left, right) {
                    (Some((a, row)), Some((b, _))) if ix - a < b - ix => row,
                    (_, Some((_, row))) | (Some((_, row)), None) => row,
                    (None, None) => MISSING_PARENT,
                };
            }
            start = end;
        }
        cancellation.check_cancelled()?;
        Ok(mapped)
    }
    pub fn parents(&self, visible: usize) -> impl Iterator<Item = usize> + '_ {
        self.raw_position(visible)
            .into_iter()
            .flat_map(|row| self.index.parents(row))
            .filter_map(|&row| self.visible_position(row as usize))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryRange {
    pub snapshot: HistorySnapshot,
    pub start: usize,
    pub commits: Vec<Commit>,
}

impl HistoryRange {
    pub fn range(&self) -> Range<usize> {
        self.start..self.start + self.commits.len()
    }
}

pub struct HistoryIndexBuilder {
    index: HistoryIndex,
    parent_ids: Vec<u8>,
}

impl HistoryIndexBuilder {
    pub fn new(snapshot: HistorySnapshot, mode: HistoryMode, hash_len: usize) -> Result<Self> {
        if !matches!(hash_len, 20 | 32) {
            return Err(invalid_index("unsupported object ID length"));
        }
        Ok(Self {
            index: HistoryIndex {
                snapshot,
                mode,
                hash_len,
                ids: Vec::new(),
                sorted_rows: Vec::new(),
                parent_offsets: vec![0],
                parents: Vec::new(),
                external_edges: Vec::new(),
                external_ids: Vec::new(),
                times: Vec::new(),
                probable_stashes: Vec::new(),
            },
            parent_ids: Vec::new(),
        })
    }
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
    pub fn push<'a>(
        &mut self,
        id: &[u8],
        parents: impl IntoIterator<Item = &'a [u8]>,
        time: i64,
        probable_stash: bool,
    ) -> Result<()> {
        if id.len() != self.index.hash_len || self.len() >= MISSING_PARENT as usize {
            return Err(invalid_index("invalid or oversized history"));
        }
        self.index.ids.extend_from_slice(id);
        for parent in parents {
            if parent.len() != self.index.hash_len {
                return Err(invalid_index("inconsistent parent ID"));
            }
            self.parent_ids.extend_from_slice(parent);
        }
        let end = u32::try_from(self.parent_ids.len() / self.index.hash_len)
            .map_err(|_| invalid_index("too many history edges"))?;
        self.index.parent_offsets.push(end);
        self.index.times.push(time);
        self.index.probable_stashes.push(probable_stash);
        Ok(())
    }
    pub fn finish(mut self, cancellation: &CancellationToken) -> Result<HistoryIndexHandle> {
        cancellation.check_cancelled()?;
        self.index.sorted_rows = (0..self.index.len() as u32).collect();
        let hash_len = self.index.hash_len;
        let ids = &self.index.ids;
        self.index.sorted_rows.sort_unstable_by(|&a, &b| {
            let a = a as usize * hash_len;
            let b = b as usize * hash_len;
            ids[a..a + hash_len].cmp(&ids[b..b + hash_len])
        });
        cancellation.check_cancelled()?;
        self.index.parents.reserve(self.parent_ids.len() / hash_len);
        for (ix, parent) in self.parent_ids.chunks_exact(hash_len).enumerate() {
            if ix.is_multiple_of(1024) {
                cancellation.check_cancelled()?;
            }
            let rank = self
                .index
                .position_bytes(parent)
                .map_or(MISSING_PARENT, |row| row as u32);
            if rank == MISSING_PARENT {
                self.index.external_edges.push(ix as u32);
                self.index.external_ids.extend_from_slice(parent);
            }
            self.index.parents.push(rank);
        }
        cancellation.check_cancelled()?;
        Ok(Arc::new(self.index))
    }
}

fn invalid_index(message: &str) -> Error {
    Error::new(ErrorKind::Backend(format!("history index: {message}")))
}

impl std::fmt::Debug for HistoryIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryIndex")
            .field("mode", &self.mode)
            .field("rows", &self.len())
            .field("bytes", &self.estimated_bytes())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(hash_len: usize) -> HistoryIndexHandle {
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("fixture".into()),
            HistoryMode::FullReachable,
            hash_len,
        )
        .unwrap();
        for id in (1..=12u8).rev() {
            builder
                .push(
                    &vec![id; hash_len],
                    [vec![id - 1; hash_len].as_slice()],
                    id as i64,
                    false,
                )
                .unwrap();
        }
        builder.finish(&CancellationToken::new()).unwrap()
    }

    #[test]
    fn binary_ids_round_trip_and_external_parents_survive() {
        for hash_len in [20, 32] {
            let index = index(hash_len);
            for row in 0..index.len() {
                let id = index.commit_id(row).unwrap();
                assert_eq!(index.position(id.as_ref()), Some(row));
                assert_eq!(index.resolve_reference(&id.as_ref()[..7]), Some(row));
                assert_eq!(index.position(&id.as_ref().to_ascii_uppercase()), Some(row));
                assert_eq!(index.timestamp(row), Some((12 - row) as i64));
                if row + 1 < index.len() {
                    assert_eq!(index.parent_commit_id(row, 0), index.commit_id(row + 1));
                }
            }
            assert_eq!(index.parents(11), &[MISSING_PARENT]);
            assert_eq!(
                index.parent_commit_id(11, 0).unwrap().as_ref(),
                "00".repeat(hash_len)
            );
            assert_eq!(index.position("invalid"), None);
            assert_eq!(index.commit_id(usize::MAX), None);
        }
    }

    #[test]
    fn every_sparse_projection_maps_both_directions() {
        let index = index(20);
        for mask in 0..4096 {
            let hidden: Vec<u32> = (0..12).filter(|row| mask & (1 << row) != 0).collect();
            let visible: Vec<usize> = (0..12).filter(|row| mask & (1 << row) == 0).collect();
            let projection = HistoryProjection::new(index.clone(), hidden);
            assert_eq!(projection.len(), visible.len());
            for (rank, raw) in visible.into_iter().enumerate() {
                assert_eq!(projection.raw_position(rank), Some(raw));
                assert_eq!(projection.visible_position(raw), Some(rank));
                assert_eq!(
                    projection.position(projection.commit_id(rank).unwrap().as_ref()),
                    Some(rank)
                );
            }
            assert_eq!(projection.raw_position(projection.len()), None);
        }
    }

    #[test]
    fn refresh_maps_removed_rows_to_the_nearest_survivor() {
        let index = index(20);
        let old = HistoryProjection::new(index.clone(), Vec::new());
        for mask in 0..4096 {
            let hidden: Vec<u32> = (0..12).filter(|row| mask & (1 << row) != 0).collect();
            let next = HistoryProjection::new(index.clone(), hidden);
            let mapped = old
                .nearest_survivors(&next, &CancellationToken::new())
                .unwrap();
            for (row, &actual) in mapped.iter().enumerate() {
                let closest = (0..12)
                    .filter(|&raw| next.visible_position(raw).is_some())
                    .min_by_key(|&raw| (raw.abs_diff(row), std::cmp::Reverse(raw)));
                let expected = closest
                    .and_then(|raw| next.visible_position(raw))
                    .map_or(MISSING_PARENT, |row| row as u32);
                assert_eq!(actual, expected, "mask={mask} row={row}");
            }
        }
    }

    #[test]
    fn indexed_squash_eligibility_matches_decoded_topology() {
        let index = index(20);
        let commits: Vec<_> = (0..index.len())
            .map(|row| Commit {
                id: index.commit_id(row).unwrap(),
                parent_ids: index.parent_commit_id(row, 0).into_iter().collect(),
                author: "a".into(),
                summary: "s".into(),
                time: std::time::UNIX_EPOCH,
            })
            .collect();
        for mask in 0..4096 {
            let selected: Vec<_> = (0..12)
                .filter(|row| mask & (1 << row) != 0)
                .map(|row| index.commit_id(row).unwrap())
                .collect();
            assert_eq!(
                crate::squash::squash_eligibility_indexed(&index, &selected, &commits[0].id),
                crate::squash::squash_eligibility(&commits, &selected, &commits[0].id)
            );
        }
    }

    #[test]
    fn cancellation_is_inherited_without_cancelling_siblings() {
        let parent = CancellationToken::new();
        let child = CancellationToken::new().with_parent(parent.clone());
        child.cancel();
        assert!(!parent.is_cancelled());
        let child = CancellationToken::new().with_parent(parent.clone());
        parent.cancel();
        assert!(child.is_cancelled());
        let builder =
            HistoryIndexBuilder::new(HistorySnapshot("x".into()), HistoryMode::FullReachable, 20)
                .unwrap();
        assert!(matches!(
            builder.finish(&child).unwrap_err().kind(),
            ErrorKind::Cancelled
        ));
    }
}
