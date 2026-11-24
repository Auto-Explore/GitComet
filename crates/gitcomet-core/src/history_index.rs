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
    /// Start offsets for each two-byte ID prefix, plus the final end offset.
    fanout: Vec<u32>,
    probable_stashes: Vec<u32>,
}

impl HistoryIndex {
    pub fn len(&self) -> usize {
        self.parent_offsets.len() - 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn id_bytes(&self, row: usize) -> Option<&[u8]> {
        let start = row.checked_mul(self.hash_len)?;
        self.ids.get(start..start.checked_add(self.hash_len)?)
    }
    pub fn commit_id(&self, row: usize) -> Option<CommitId> {
        let bytes = self.id_bytes(row)?;
        Some(CommitId(crate::hex::encode_object_id(bytes)))
    }
    pub fn position(&self, id: &str) -> Option<usize> {
        if id.len() != self.hash_len * 2 {
            return None;
        }
        let mut bytes = [0u8; 32];
        faster_hex::hex_decode(id.as_bytes(), &mut bytes[..self.hash_len]).ok()?;
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
        // Pad an odd-length prefix with a zero nibble for its lower bound.
        // The upper bound includes every possible low nibble of that byte.
        let mut padded = [b'0'; 64];
        padded[..reference.len()].copy_from_slice(reference.as_bytes());
        let byte_len = reference.len().div_ceil(2);
        faster_hex::hex_decode(&padded[..byte_len * 2], &mut lower[..byte_len]).ok()?;
        upper[..byte_len].copy_from_slice(&lower[..byte_len]);
        if !reference.len().is_multiple_of(2) {
            upper[byte_len - 1] |= 0x0f;
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
    pub fn position_bytes(&self, id: &[u8]) -> Option<usize> {
        if id.len() != self.hash_len {
            return None;
        }
        let rows = if self.fanout.is_empty() {
            &self.sorted_rows[..]
        } else {
            let prefix = u16::from_be_bytes([id[0], id[1]]) as usize;
            &self.sorted_rows[self.fanout[prefix] as usize..self.fanout[prefix + 1] as usize]
        };
        rows.binary_search_by(|row| self.id_bytes(*row as usize).expect("indexed row").cmp(id))
            .ok()
            .map(|ix| rows[ix] as usize)
    }
    /// `position_bytes` over the builder's sorted `(prefix, row)` cache, which is
    /// parallel to `sorted_rows`. Equal prefixes form a run sorted by full ID.
    fn position_in_keyed(&self, keyed: &[(u64, u32)], id: &[u8]) -> Option<usize> {
        let prefix = u64::from_be_bytes(id[..8].try_into().ok()?);
        let bucket = (prefix >> 48) as usize;
        let bucket = &keyed[self.fanout[bucket] as usize..self.fanout[bucket + 1] as usize];
        let first = bucket.partition_point(|entry| entry.0 < prefix);
        let run = &bucket[first..];
        let run = &run[..run.partition_point(|entry| entry.0 == prefix)];
        run.binary_search_by(|entry| {
            self.id_bytes(entry.1 as usize)
                .expect("indexed row")
                .cmp(id)
        })
        .ok()
        .map(|ix| run[ix].1 as usize)
    }
    pub fn parents(&self, row: usize) -> &[u32] {
        match (
            self.parent_offsets.get(row),
            row.checked_add(1)
                .and_then(|row| self.parent_offsets.get(row)),
        ) {
            (Some(&start), Some(&end)) => &self.parents[start as usize..end as usize],
            _ => &[],
        }
    }
    /// The parent IDs and their order are those of the walk, including external parents.
    pub fn parent_id_bytes(&self, row: usize, parent: usize) -> Option<&[u8]> {
        let rank = *self.parents(row).get(parent)?;
        if rank != MISSING_PARENT {
            return self.id_bytes(rank as usize);
        }
        let edge = *self.parent_offsets.get(row)? as usize + parent;
        let ix = self.external_edges.binary_search(&(edge as u32)).ok()? * self.hash_len;
        self.external_ids.get(ix..ix + self.hash_len)
    }
    pub fn parent_commit_id(&self, row: usize, parent: usize) -> Option<CommitId> {
        Some(CommitId(crate::hex::encode_object_id(
            self.parent_id_bytes(row, parent)?,
        )))
    }
    /// Check a range response without searching the index or allocating an ID.
    pub fn row_matches_hex_id(&self, row: usize, id: &str) -> bool {
        self.id_bytes(row)
            .is_some_and(|bytes| crate::hex::matches(bytes, id))
    }
    /// Fills the rows-by-ID table and its fanout, which every lookup by ID
    /// uses. Returns the sorted ID prefixes of a large index for the caller
    /// that has many lookups to make right away.
    fn sort_rows(&mut self) -> Option<Vec<(u64, u32)>> {
        let hash_len = self.hash_len;
        let ids = &self.ids;
        if self.len() >= 65_536 {
            // Deal the rows into their two-byte prefixes, then sort within
            // each: a prefix holds a few dozen rows, where one sort of the
            // whole table moves every row a dozen times. Prefixes are cached
            // contiguously, so comparisons usually avoid random reads into
            // the much larger ID table; full IDs resolve prefix collisions.
            let mut fanout = vec![0u32; 65_537];
            for id in ids.chunks_exact(hash_len) {
                fanout[u16::from_be_bytes([id[0], id[1]]) as usize + 1] += 1;
            }
            for prefix in 1..fanout.len() {
                fanout[prefix] += fanout[prefix - 1];
            }
            let mut next = fanout[..65_536].to_vec();
            let mut keyed = vec![(0u64, 0u32); self.len()];
            for (row, id) in ids.chunks_exact(hash_len).enumerate() {
                let prefix = u64::from_be_bytes(id[..8].try_into().unwrap());
                let at = &mut next[(prefix >> 48) as usize];
                keyed[*at as usize] = (prefix, row as u32);
                *at += 1;
            }
            for prefix in fanout.windows(2) {
                keyed[prefix[0] as usize..prefix[1] as usize].sort_unstable_by(|a, b| {
                    a.0.cmp(&b.0).then_with(|| {
                        let a = a.1 as usize * hash_len;
                        let b = b.1 as usize * hash_len;
                        ids[a..a + hash_len].cmp(&ids[b..b + hash_len])
                    })
                });
            }
            self.sorted_rows = keyed.iter().map(|&(_, row)| row).collect();
            self.fanout = fanout;
            Some(keyed)
        } else {
            self.sorted_rows = (0..self.len() as u32).collect();
            self.sorted_rows.sort_unstable_by(|&a, &b| {
                let a = a as usize * hash_len;
                let b = b as usize * hash_len;
                ids[a..a + hash_len].cmp(&ids[b..b + hash_len])
            });
            None
        }
    }

    /// The index of rows whose parents are already known as rows, which is
    /// what a builder finishes when given the same rows with their parents'
    /// IDs, without its lookup of every parent. `ids` holds one ID per row,
    /// `parents[parent_offsets[row]..parent_offsets[row + 1]]` a row's
    /// parents, each a row of this index, and `probable_stashes` rises.
    pub fn from_rows(
        snapshot: HistorySnapshot,
        mode: HistoryMode,
        hash_len: usize,
        ids: Vec<u8>,
        parent_offsets: Vec<u32>,
        parents: Vec<u32>,
        probable_stashes: Vec<u32>,
        cancellation: &CancellationToken,
    ) -> Result<HistoryIndexHandle> {
        cancellation.check_cancelled()?;
        let rows = ids.len() / hash_len.max(1);
        let well_formed = matches!(hash_len, 20 | 32)
            && ids.len() == rows * hash_len
            && rows < MISSING_PARENT as usize
            && parent_offsets.len() == rows + 1
            && parent_offsets.first() == Some(&0)
            && parent_offsets.last().copied() == u32::try_from(parents.len()).ok()
            && parent_offsets.is_sorted()
            && parents.iter().all(|&parent| (parent as usize) < rows)
            && probable_stashes.is_sorted_by(|a, b| a < b)
            && probable_stashes
                .last()
                .is_none_or(|&row| (row as usize) < rows);
        if !well_formed {
            return Err(invalid_index("inconsistent history rows"));
        }
        let mut index = Self {
            snapshot,
            mode,
            hash_len,
            ids,
            sorted_rows: Vec::new(),
            parent_offsets,
            parents,
            external_edges: Vec::new(),
            external_ids: Vec::new(),
            fanout: Vec::new(),
            probable_stashes,
        };
        cancellation.check_cancelled()?;
        index.sort_rows();
        cancellation.check_cancelled()?;
        Ok(Arc::new(index))
    }

    pub fn is_probable_stash(&self, row: usize) -> bool {
        u32::try_from(row)
            .ok()
            .is_some_and(|row| self.probable_stashes.binary_search(&row).is_ok())
    }
    pub fn probable_stash_rows(&self) -> &[u32] {
        &self.probable_stashes
    }
    pub fn estimated_bytes(&self) -> usize {
        self.ids.capacity()
            + self.sorted_rows.capacity() * 4
            + self.parent_offsets.capacity() * 4
            + self.parents.capacity() * 4
            + self.fanout.capacity() * 4
            + self.probable_stashes.capacity() * 4
            + self.external_edges.capacity() * 4
            + self.external_ids.capacity()
    }

    /// Reorders the leading rows of an unpublished index. Row references and
    /// external parents follow the permutation; the ID lookup stays sorted.
    pub fn reorder_prefix(&mut self, order: &[u32]) -> Result<()> {
        if order.len() > self.len() {
            return Err(invalid_index("row permutation exceeds index length"));
        }
        let mut place = vec![MISSING_PARENT; order.len()];
        for (at, &row) in order.iter().enumerate() {
            let Some(previous) = place.get_mut(row as usize) else {
                return Err(invalid_index("row permutation leaves its prefix"));
            };
            if *previous != MISSING_PARENT {
                return Err(invalid_index("duplicate row in permutation"));
            }
            *previous = at as u32;
        }
        let edge_count = self.parent_offsets[order.len()] as usize;
        let mut ids = Vec::with_capacity(order.len() * self.hash_len);
        let mut offsets = vec![0u32];
        let mut parents = Vec::with_capacity(edge_count);
        let mut missing_edges = Vec::new();
        let mut missing_ids = Vec::new();
        for &row in order {
            let row = row as usize;
            ids.extend_from_slice(self.id_bytes(row).expect("validated row"));
            for (slot, &parent) in self.parents(row).iter().enumerate() {
                if parent == MISSING_PARENT {
                    missing_edges.push(parents.len() as u32);
                    missing_ids.extend_from_slice(
                        self.parent_id_bytes(row, slot).expect("external parent"),
                    );
                }
                parents.push(parent);
            }
            offsets.push(parents.len() as u32);
        }
        self.ids[..ids.len()].copy_from_slice(&ids);
        self.parent_offsets[..offsets.len()].copy_from_slice(&offsets);
        self.parents[..edge_count].copy_from_slice(&parents);
        self.external_edges[..missing_edges.len()].copy_from_slice(&missing_edges);
        self.external_ids[..missing_ids.len()].copy_from_slice(&missing_ids);
        for row in self
            .sorted_rows
            .iter_mut()
            .chain(&mut self.parents)
            .chain(&mut self.probable_stashes)
        {
            if let Some(&new) = place.get(*row as usize) {
                *row = new;
            }
        }
        self.probable_stashes.sort_unstable();
        Ok(())
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
        // `hidden[k] - k` is the visible index the k-th hidden row displaced, and it
        // never decreases, so the hidden rows at or before the answer are one
        // partition point rather than a binary search over partition points.
        let (mut lo, mut hi) = (0, self.hidden.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.hidden[mid] as usize - mid <= visible {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some(visible + lo)
    }
    /// The visible index of the first shown row at or after `raw`, or the
    /// visible length when none remains. Lets block scans step by block instead
    /// of by row.
    pub fn visible_position_at_or_after(&self, raw: usize) -> usize {
        if raw >= self.index.len() {
            return self.len();
        }
        raw - self
            .hidden
            .partition_point(|&hidden| (hidden as usize) < raw)
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
                fanout: Vec::new(),
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
        probable_stash: bool,
    ) -> Result<()> {
        if id.len() != self.index.hash_len || self.len() >= MISSING_PARENT as usize {
            return Err(invalid_index("invalid or oversized history"));
        }
        let original_parents = self.parent_ids.len();
        for parent in parents {
            if parent.len() != self.index.hash_len {
                self.parent_ids.truncate(original_parents);
                return Err(invalid_index("inconsistent parent ID"));
            }
            self.parent_ids.extend_from_slice(parent);
        }
        let end = u32::try_from(self.parent_ids.len() / self.index.hash_len)
            .map_err(|_| invalid_index("too many history edges"))?;
        self.index.ids.extend_from_slice(id);
        self.index.parent_offsets.push(end);
        if probable_stash {
            self.index.probable_stashes.push((self.len() - 1) as u32);
        }
        Ok(())
    }

    /// Refine the stash candidates after traversal, when the walk's temporary
    /// storage can be released. Parent IDs retain their original order and
    /// include parents outside this index. An error aborts construction.
    pub fn retain_probable_stashes(
        &mut self,
        mut predicate: impl FnMut(&[u8], std::slice::ChunksExact<'_, u8>) -> Result<bool>,
    ) -> Result<()> {
        let hash_len = self.index.hash_len;
        let mut retained = 0;
        for candidate in 0..self.index.probable_stashes.len() {
            let row = self.index.probable_stashes[candidate] as usize;
            let start = self.index.parent_offsets[row] as usize * hash_len;
            let end = self.index.parent_offsets[row + 1] as usize * hash_len;
            match predicate(
                self.index
                    .id_bytes(row)
                    .expect("candidate is an indexed row"),
                self.parent_ids[start..end].chunks_exact(hash_len),
            ) {
                Ok(true) => {
                    self.index.probable_stashes[retained] = row as u32;
                    retained += 1;
                }
                Ok(false) => {}
                Err(error) => {
                    self.index.probable_stashes.truncate(retained);
                    return Err(error);
                }
            }
        }
        self.index.probable_stashes.truncate(retained);
        Ok(())
    }

    /// Estimated peak construction storage, including the cached-prefix sort
    /// (retained through parent resolution), unresolved parents, and capacity
    /// growth if every parent is external. Retained index bytes are reported
    /// separately.
    pub fn estimated_peak_bytes(&self) -> usize {
        let rows = self.len();
        let (sort, fanout) = if rows >= 65_536 {
            (rows * std::mem::size_of::<(u64, u32)>(), 65_537 * 4)
        } else {
            (0, 0)
        };
        let edges = self.parent_ids.len() / self.index.hash_len;
        let resolved = edges * 4 + 2 * edges * (self.index.hash_len + 4) + 32;
        self.index.estimated_bytes()
            + self.parent_ids.capacity()
            + rows * 4
            + fanout
            + sort
            + resolved
    }

    pub fn finish(mut self, cancellation: &CancellationToken) -> Result<HistoryIndexHandle> {
        cancellation.check_cancelled()?;
        let hash_len = self.index.hash_len;
        let keyed = self.index.sort_rows();
        cancellation.check_cancelled()?;
        self.index.parents.reserve(self.parent_ids.len() / hash_len);
        for (ix, parent) in self.parent_ids.chunks_exact(hash_len).enumerate() {
            if ix.is_multiple_of(1024) {
                cancellation.check_cancelled()?;
            }
            // The cached prefixes stay alive for this pass: a bucket is a few
            // contiguous cache lines, and only the final full-ID check touches
            // the ID table, instead of one random read per probe.
            let rank = match &keyed {
                Some(keyed) => self.index.position_in_keyed(keyed, parent),
                None => self.index.position_bytes(parent),
            }
            .map_or(MISSING_PARENT, |row| row as u32);
            if rank == MISSING_PARENT {
                self.index.external_edges.push(ix as u32);
                self.index.external_ids.extend_from_slice(parent);
            }
            self.index.parents.push(rank);
        }
        cancellation.check_cancelled()?;
        drop(keyed);
        drop(self.parent_ids);
        // The tables were grown a commit at a time; the doubling slack is a few
        // MiB on a two-million-commit history and is retained for its lifetime.
        self.index.ids.shrink_to_fit();
        self.index.parent_offsets.shrink_to_fit();
        self.index.external_edges.shrink_to_fit();
        self.index.external_ids.shrink_to_fit();
        self.index.probable_stashes.shrink_to_fit();
        Ok(Arc::new(self.index))
    }

    /// The index of the rows pushed so far followed by every row of `older`:
    /// what one builder given all of them would finish. `older`'s tables are
    /// copied and shifted rather than sorted and resolved again, so this costs
    /// a pass over them instead of a rebuild.
    pub fn finish_before(
        mut self,
        older: &HistoryIndex,
        cancellation: &CancellationToken,
    ) -> Result<HistoryIndexHandle> {
        cancellation.check_cancelled()?;
        let hash_len = self.index.hash_len;
        if older.hash_len != hash_len {
            return Err(invalid_index("mismatched object ID length"));
        }
        let added = self.len();
        let added_edges = self.parent_ids.len() / hash_len;
        let (Some(row_shift), Some(edge_shift)) = (
            u32::try_from(added).ok().filter(|added| {
                added
                    .checked_add(older.len() as u32)
                    .is_some_and(|rows| rows < MISSING_PARENT)
            }),
            u32::try_from(added_edges)
                .ok()
                .filter(|edges| edges.checked_add(older.parents.len() as u32).is_some()),
        ) else {
            return Err(invalid_index("invalid or oversized history"));
        };

        let added_ids = std::mem::take(&mut self.index.ids);
        let added_id = |row: u32| &added_ids[row as usize * hash_len..][..hash_len];
        let mut added_sorted: Vec<u32> = (0..row_shift).collect();
        added_sorted.sort_unstable_by(|&a, &b| added_id(a).cmp(added_id(b)));
        let find_added = |id: &[u8]| {
            added_sorted
                .binary_search_by(|&row| added_id(row).cmp(id))
                .ok()
                .map(|at| added_sorted[at])
        };

        // By ID: the added rows merged into the older order.
        let mut sorted_rows = Vec::with_capacity(added + older.len());
        let mut pending = added_sorted.iter().copied().peekable();
        for &row in &older.sorted_rows {
            let id = older.id_bytes(row as usize).expect("indexed row");
            while let Some(&next) = pending.peek() {
                match added_id(next).cmp(id) {
                    std::cmp::Ordering::Less => sorted_rows.push(pending.next().expect("peeked")),
                    std::cmp::Ordering::Equal => {
                        return Err(invalid_index("a row is added twice"));
                    }
                    std::cmp::Ordering::Greater => break,
                }
            }
            sorted_rows.push(row + row_shift);
        }
        sorted_rows.extend(pending);
        cancellation.check_cancelled()?;

        let mut parents = Vec::with_capacity(added_edges + older.parents.len());
        let mut external_edges = Vec::new();
        let mut external_ids = Vec::new();
        for (edge, parent) in self.parent_ids.chunks_exact(hash_len).enumerate() {
            let row = find_added(parent).or_else(|| {
                older
                    .position_bytes(parent)
                    .map(|row| row as u32 + row_shift)
            });
            if row.is_none() {
                external_edges.push(edge as u32);
                external_ids.extend_from_slice(parent);
            }
            parents.push(row.unwrap_or(MISSING_PARENT));
        }
        // An older row's parent outside its index may be one of the added rows.
        let mut older_external = older.external_ids.chunks_exact(hash_len);
        for (edge, &parent) in older.parents.iter().enumerate() {
            if parent != MISSING_PARENT {
                parents.push(parent + row_shift);
                continue;
            }
            let id = older_external.next().expect("an external edge has its ID");
            let row = find_added(id);
            if row.is_none() {
                external_edges.push(edge as u32 + edge_shift);
                external_ids.extend_from_slice(id);
            }
            parents.push(row.unwrap_or(MISSING_PARENT));
        }
        cancellation.check_cancelled()?;

        let mut ids = added_ids;
        ids.reserve_exact(older.ids.len());
        ids.extend_from_slice(&older.ids);
        let mut parent_offsets = std::mem::take(&mut self.index.parent_offsets);
        parent_offsets.reserve_exact(older.len());
        parent_offsets.extend(older.parent_offsets[1..].iter().map(|end| end + edge_shift));
        let mut probable_stashes = std::mem::take(&mut self.index.probable_stashes);
        probable_stashes.reserve_exact(older.probable_stashes.len());
        probable_stashes.extend(older.probable_stashes.iter().map(|row| row + row_shift));
        let fanout = if sorted_rows.len() >= 65_536 {
            let mut fanout = vec![0u32; 65_537];
            for id in ids.chunks_exact(hash_len) {
                fanout[u16::from_be_bytes([id[0], id[1]]) as usize + 1] += 1;
            }
            for prefix in 1..fanout.len() {
                fanout[prefix] += fanout[prefix - 1];
            }
            fanout
        } else {
            Vec::new()
        };
        external_edges.shrink_to_fit();
        external_ids.shrink_to_fit();
        Ok(Arc::new(HistoryIndex {
            snapshot: self.index.snapshot,
            mode: self.index.mode,
            hash_len,
            ids,
            sorted_rows,
            parent_offsets,
            parents,
            external_edges,
            external_ids,
            fanout,
            probable_stashes,
        }))
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
mod tests;

#[cfg(test)]
mod lookup_regressions;
