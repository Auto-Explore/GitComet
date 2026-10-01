//! Searchable commit text, reused across queries and index replacements.
//!
//! Text lives in a private, unnamed file: keeping millions of decoded commits
//! in memory would dwarf the index itself. Only row offsets and the last
//! completed result stay in memory. Closing the bar or searching another repo
//! drops the file. Index replacements compact it when enough records retire.

use super::HistoryFindChunk;
use gitcomet_core::domain::{Commit, StashEntry};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::history_find::{HistoryFindQuery, stash_row_summary};
use gitcomet_core::history_index::{
    HISTORY_BLOCK_SIZE, HistoryIndex, HistoryIndexHandle, HistoryRange,
};
use gitcomet_core::services::{CancellationToken, Result};
use rustc_hash::FxHashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

const MISSING: u64 = u64::MAX;
const REPORT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(crate) struct HistoryFindCache {
    index: Weak<HistoryIndex>,
    offsets: Vec<u64>,
    text: Option<TextFile>,
    previous: Option<Previous>,
    #[cfg(test)]
    comparisons: usize,
    #[cfg(test)]
    remap_panic_after: Option<usize>,
}

/// The last completed search, in rows of the cache's current index.
#[derive(Debug)]
struct Previous {
    query: HistoryFindQuery,
    /// The stash list its stash rows were matched with.
    stashes: Arc<Vec<StashEntry>>,
    matches: Vec<u32>,
    /// Rows that search never checked: new commits after an index
    /// replacement. Empty over the index it ran on.
    unchecked: Vec<u32>,
}

impl HistoryFindCache {
    /// Stash rows are matched on what they show, which `stashes` decides.
    pub(crate) fn search(
        &mut self,
        index: &HistoryIndexHandle,
        query: &HistoryFindQuery,
        stashes: &Arc<Vec<StashEntry>>,
        cancellation: &CancellationToken,
        read: impl FnMut(Range<usize>) -> Result<HistoryRange>,
        mut report: impl FnMut(HistoryFindChunk),
    ) -> Result<()> {
        self.prepare(index, cancellation)?;
        let previous = self.previous.take();
        // A commit's text never changes, so the same query keeps its answer
        // and a refinement only needs to recheck it.
        let narrowed = previous.as_ref().filter(|old| {
            old.stashes == *stashes && (old.query == *query || query.is_refinement_of(&old.query))
        });
        let listed = stashes
            .iter()
            .map(|stash| (stash.id.as_ref(), stash.message.as_ref()))
            .collect();
        let scanned = self.scan(
            index,
            query,
            &listed,
            narrowed,
            cancellation,
            read,
            &mut report,
        );
        let (matches, pending) = match scanned {
            Ok(found) => found,
            Err(error) => {
                // An interrupted scan leaves the last complete answer.
                self.previous = previous;
                return Err(error);
            }
        };
        self.previous = Some(Previous {
            query: query.clone(),
            stashes: Arc::clone(stashes),
            matches,
            unchecked: Vec::new(),
        });
        report(HistoryFindChunk {
            matches: pending,
            done: true,
        });
        Ok(())
    }

    /// Checks `narrowed`'s rows, or every row, and returns all matches plus
    /// those not reported yet.
    #[allow(clippy::too_many_arguments)]
    fn scan(
        &mut self,
        index: &HistoryIndexHandle,
        query: &HistoryFindQuery,
        listed: &FxHashMap<&str, &str>,
        narrowed: Option<&Previous>,
        cancellation: &CancellationToken,
        mut read: impl FnMut(Range<usize>) -> Result<HistoryRange>,
        report: &mut impl FnMut(HistoryFindChunk),
    ) -> Result<(Vec<u32>, Vec<u32>)> {
        // Rows to check, and whether each is already known to match.
        let rows: Box<dyn Iterator<Item = (usize, bool)>> = match narrowed {
            Some(old) => {
                let same = old.query == *query;
                Box::new(
                    merge_ascending(&old.matches, &old.unchecked)
                        .map(move |(row, matched)| (row as usize, matched && same)),
                )
            }
            None => Box::new((0..index.len()).map(|row| (row, false))),
        };
        let mut buffer = Vec::new();
        let mut matches = Vec::new();
        let mut pending = Vec::new();
        let mut last_report = Instant::now();
        for (visited, (row, known)) in rows.enumerate() {
            if visited.is_multiple_of(HISTORY_BLOCK_SIZE) {
                cancellation.check_cancelled()?;
            }
            if known || self.matches(index, query, listed, row, &mut buffer, &mut read)? {
                matches.push(row as u32);
                pending.push(row as u32);
            }
            if !pending.is_empty() && last_report.elapsed() >= REPORT_INTERVAL {
                cancellation.check_cancelled()?;
                report(HistoryFindChunk {
                    matches: std::mem::take(&mut pending),
                    done: false,
                });
                last_report = Instant::now();
            }
        }
        cancellation.check_cancelled()?;
        Ok((matches, pending))
    }

    /// Whether `row`'s text matches, reading its commits first if needed.
    fn matches(
        &mut self,
        index: &HistoryIndexHandle,
        query: &HistoryFindQuery,
        listed: &FxHashMap<&str, &str>,
        row: usize,
        buffer: &mut Vec<u8>,
        read: &mut impl FnMut(Range<usize>) -> Result<HistoryRange>,
    ) -> Result<bool> {
        if self.offsets[row] == MISSING {
            // Only read missing spans. A refreshed index normally adds
            // a few commits while reusing all the existing search text.
            let end = (row..(row + HISTORY_BLOCK_SIZE).min(index.len()))
                .take_while(|&row| self.offsets[row] == MISSING)
                .last()
                .unwrap()
                + 1;
            let range = read(row..end)?;
            if range.start != row
                || range.commits.len() != end - row
                || range.snapshot != index.snapshot
            {
                return Err(Error::new(ErrorKind::Backend(
                    "Incomplete history search range".into(),
                )));
            }
            let offsets = self
                .text
                .as_mut()
                .unwrap()
                .append(&range.commits)
                .map_err(io_error)?;
            self.offsets[row..end].copy_from_slice(&offsets);
        }
        let (id, summary, author) = self
            .text
            .as_mut()
            .unwrap()
            .read(self.offsets[row], buffer)
            .map_err(io_error)?;
        #[cfg(test)]
        {
            self.comparisons += 1;
        }
        let message = listed.get(id).copied();
        let summary = if message.is_some() || index.is_probable_stash(row) {
            stash_row_summary(message, summary)
        } else {
            summary
        };
        Ok(query.matches_fields(id, summary, author))
    }

    /// Maps the cached text onto `index`'s rows, carrying the last completed
    /// search over when the index is new.
    fn prepare(
        &mut self,
        index: &HistoryIndexHandle,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        cancellation.check_cancelled()?;
        if self.index.ptr_eq(&Arc::downgrade(index)) {
            return Ok(());
        }
        if self.text.is_none() {
            self.text = Some(TextFile::new().map_err(io_error)?);
        }
        self.remap(index, cancellation)
    }

    fn remap(
        &mut self,
        index: &HistoryIndexHandle,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        // Build the replacement without taking any old state. Cancellation,
        // I/O errors and unwinds cannot leave an index paired with other rows.
        let previous = self.previous.as_ref();
        let mut offsets = vec![MISSING; index.len()];
        let mut checked = vec![false; if previous.is_some() { index.len() } else { 0 }];
        let mut matches = Vec::new();
        if let Some(old) = self.index.upgrade() {
            // The old index already holds every id. Mapping through it avoids
            // reading the text file on the usual refresh path.
            for (old_row, &offset) in self.offsets.iter().enumerate() {
                #[cfg(test)]
                assert_ne!(
                    self.remap_panic_after,
                    Some(old_row),
                    "injected remap panic"
                );
                if old_row.is_multiple_of(HISTORY_BLOCK_SIZE) {
                    cancellation.check_cancelled()?;
                }
                let Some(row) = old
                    .id_bytes(old_row)
                    .and_then(|id| index.position_bytes(id))
                else {
                    continue;
                };
                offsets[row] = offset;
                if let Some(previous) = previous {
                    checked[row] = previous.unchecked.binary_search(&(old_row as u32)).is_err();
                    if previous.matches.binary_search(&(old_row as u32)).is_ok() {
                        matches.push(row as u32);
                    }
                }
            }
        } else {
            // A weak identity does not keep obsolete indexes alive. Fall back
            // to the file's ids if the old index was released before this scan.
            let mut matched = Vec::new();
            let mut checked_offsets = Vec::new();
            if let Some(previous) = previous {
                matched = previous
                    .matches
                    .iter()
                    .map(|&row| self.offsets[row as usize])
                    .collect();
                matched.sort_unstable();
                checked_offsets = self
                    .offsets
                    .iter()
                    .enumerate()
                    .filter(|(row, offset)| {
                        **offset != MISSING
                            && previous.unchecked.binary_search(&(*row as u32)).is_err()
                    })
                    .map(|(_, &offset)| offset)
                    .collect();
                checked_offsets.sort_unstable();
            }
            let text = self.text.as_mut().unwrap();
            let (mut next_checked, mut next_matched) = (0, 0);
            let mut buffer = Vec::new();
            let mut offset = 0;
            let mut visited = 0usize;
            while offset < text.len {
                #[cfg(test)]
                assert_ne!(
                    self.remap_panic_after,
                    Some(visited),
                    "injected remap panic"
                );
                if visited.is_multiple_of(HISTORY_BLOCK_SIZE) {
                    cancellation.check_cancelled()?;
                }
                let id = text.read_id(offset, &mut buffer).map_err(io_error)?;
                if let Some(row) = index.position(id) {
                    offsets[row] = offset;
                    if previous.is_some() && advance_to(&checked_offsets, &mut next_checked, offset)
                    {
                        checked[row] = true;
                        if advance_to(&matched, &mut next_matched, offset) {
                            matches.push(row as u32);
                        }
                    }
                }
                offset = text.position;
                visited += 1;
            }
        }
        matches.sort_unstable();
        let previous = previous.map(|previous| Previous {
            query: previous.query.clone(),
            stashes: Arc::clone(&previous.stashes),
            matches,
            unchecked: (0..index.len() as u32)
                .filter(|&row| !checked[row as usize])
                .collect(),
        });
        let live = offsets.iter().filter(|&&offset| offset != MISSING).count();
        let text = self.text.as_mut().unwrap();
        // Reclaim removed history once it makes up a quarter of the records.
        // A replacement file also makes compaction transactional.
        let compacted = if live < text.records && text.records - live >= text.records.div_ceil(4) {
            Some(text.compact(&mut offsets, cancellation)?)
        } else {
            None
        };
        cancellation.check_cancelled()?;
        if let Some(text) = compacted {
            self.text = Some(text);
        }
        self.offsets = offsets;
        self.index = Arc::downgrade(index);
        self.previous = previous;
        Ok(())
    }
}

/// Whether `offset` is in `sorted`, for ascending `offset`s across calls.
fn advance_to(sorted: &[u64], next: &mut usize, offset: u64) -> bool {
    while sorted.get(*next).is_some_and(|&known| known < offset) {
        *next += 1;
    }
    sorted.get(*next) == Some(&offset)
}

/// Two ascending, disjoint row lists as one ascending sequence, each row
/// paired with whether it came from `a`.
fn merge_ascending<'a>(a: &'a [u32], b: &'a [u32]) -> impl Iterator<Item = (u32, bool)> + 'a {
    let (mut a, mut b) = (a.iter().copied().peekable(), b.iter().copied().peekable());
    std::iter::from_fn(move || match (a.peek(), b.peek()) {
        (Some(x), Some(y)) if y < x => b.next().map(|row| (row, false)),
        _ => a
            .next()
            .map(|row| (row, true))
            .or_else(|| b.next().map(|row| (row, false))),
    })
}

fn io_error(error: io::Error) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "History search text could not be stored: {error}"
    )))
}

/// A file for the text, preferably in the user's cache directory: `/tmp` is
/// often RAM-backed (tmpfs) on Linux, and may have a small per-user quota.
fn text_file() -> io::Result<File> {
    let dir = text_dir(
        std::env::var_os("XDG_CACHE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    );
    if let Some(dir) = dir
        && std::fs::create_dir_all(&dir).is_ok()
        && let Ok(file) = tempfile::tempfile_in(&dir)
    {
        return Ok(file);
    }
    tempfile::tempfile()
}

/// The XDG cache directory on systems whose temp dir may live in memory;
/// macOS and Windows keep theirs on disk.
fn text_dir(xdg_cache_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    if cfg!(any(target_os = "macos", not(unix))) {
        return None;
    }
    let non_empty =
        |path: Option<&OsStr>| path.map(PathBuf::from).filter(|path| path.is_absolute());
    let cache = non_empty(xdg_cache_home).or_else(|| Some(non_empty(home)?.join(".cache")))?;
    Some(cache.join("gitcomet").join("history-find"))
}

#[derive(Debug)]
struct TextFile {
    reader: BufReader<File>,
    position: u64,
    len: u64,
    records: usize,
}

impl TextFile {
    fn new() -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, text_file()?),
            position: 0,
            len: 0,
            records: 0,
        })
    }

    fn append(&mut self, commits: &[Commit]) -> io::Result<Vec<u64>> {
        let mut bytes = Vec::new();
        let mut offsets = Vec::with_capacity(commits.len());
        for commit in commits {
            offsets.push(self.len + bytes.len() as u64);
            let fields = [
                commit.id.as_ref(),
                commit.summary.as_ref(),
                commit.author.as_ref(),
            ];
            for field in fields {
                let len = u32::try_from(field.len()).map_err(|_| io::ErrorKind::InvalidData)?;
                bytes.extend_from_slice(&len.to_le_bytes());
            }
            for field in fields {
                bytes.extend_from_slice(field.as_bytes());
            }
        }
        self.append_bytes(&bytes)?;
        self.records += commits.len();
        Ok(offsets)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.reader.seek(SeekFrom::Start(self.len))?;
        self.position = MISSING;
        if let Err(error) = self.reader.get_mut().write_all(bytes) {
            let _ = self.reader.get_mut().set_len(self.len);
            return Err(error);
        }
        self.len += bytes.len() as u64;
        self.position = self.len;
        Ok(())
    }

    fn compact(&mut self, offsets: &mut [u64], cancellation: &CancellationToken) -> Result<Self> {
        let mut live: Vec<_> = offsets
            .iter()
            .enumerate()
            .filter(|(_, offset)| **offset != MISSING)
            .map(|(row, &offset)| (offset, row))
            .collect();
        live.sort_unstable();
        let mut next = Self::new().map_err(io_error)?;
        let mut bytes = Vec::with_capacity(64 * 1024);
        for (visited, (offset, row)) in live.iter().copied().enumerate() {
            if visited.is_multiple_of(HISTORY_BLOCK_SIZE) {
                cancellation.check_cancelled()?;
            }
            let lengths = self.read_header(offset).map_err(io_error)?;
            offsets[row] = next.len + bytes.len() as u64;
            for len in lengths {
                bytes.extend_from_slice(&(len as u32).to_le_bytes());
            }
            let start = bytes.len();
            bytes.resize(start + lengths.iter().sum::<usize>(), 0);
            self.reader
                .read_exact(&mut bytes[start..])
                .map_err(io_error)?;
            self.position = offset + (RECORD_HEADER + lengths.iter().sum::<usize>()) as u64;
            if bytes.len() >= 64 * 1024 {
                next.append_bytes(&bytes).map_err(io_error)?;
                bytes.clear();
            }
        }
        next.append_bytes(&bytes).map_err(io_error)?;
        next.records = live.len();
        Ok(next)
    }

    fn read<'a>(
        &mut self,
        offset: u64,
        buffer: &'a mut Vec<u8>,
    ) -> io::Result<(&'a str, &'a str, &'a str)> {
        let lengths = self.read_header(offset)?;
        let total = lengths.iter().sum::<usize>();
        buffer.resize(total, 0);
        self.reader.read_exact(buffer)?;
        self.position = offset + (RECORD_HEADER + total) as u64;
        let (id, rest) = buffer.split_at(lengths[0]);
        let (summary, author) = rest.split_at(lengths[1]);
        Ok((utf8(id)?, utf8(summary)?, utf8(author)?))
    }

    /// Only the id, for mapping records onto a new index's rows.
    fn read_id<'a>(&mut self, offset: u64, buffer: &'a mut Vec<u8>) -> io::Result<&'a str> {
        let lengths = self.read_header(offset)?;
        buffer.resize(lengths[0], 0);
        self.reader.read_exact(buffer)?;
        let rest = lengths[1] + lengths[2];
        self.reader.seek_relative(rest as i64)?;
        self.position = offset + (RECORD_HEADER + lengths[0] + rest) as u64;
        utf8(buffer)
    }

    /// Moves to the record at `offset` and returns its field lengths.
    fn read_header(&mut self, offset: u64) -> io::Result<[usize; 3]> {
        if self.position == MISSING {
            self.reader.seek(SeekFrom::Start(offset))?;
        } else if offset != self.position {
            // Keeps the buffer when a refinement skips a few records.
            self.reader
                .seek_relative(offset as i64 - self.position as i64)?;
        }
        // If a read fails partway through, the next attempt must seek again.
        self.position = MISSING;
        let mut header = [0u8; RECORD_HEADER];
        self.reader.read_exact(&mut header)?;
        Ok([0, 4, 8]
            .map(|start| u32::from_le_bytes(header[start..start + 4].try_into().unwrap()) as usize))
    }
}

/// Three little-endian `u32` field lengths.
const RECORD_HEADER: usize = 12;

fn utf8(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, LogScope};
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;
    use gitcomet_core::text_search::TextSearchOptions;

    #[test]
    #[ignore = "manual cache remap measurement"]
    fn history_find_remap_measurement() {
        let mut commits = commits(200_000);
        for commit in &mut commits {
            commit.summary = format!("fix {}", "x".repeat(256)).into();
        }
        for alive in [true, false] {
            let old = index(&commits);
            let next = index(&commits[..50_000]);
            let mut cache = HistoryFindCache::default();
            search(&mut cache, &old, &commits, &query("fix"));
            let before = cache.text.as_ref().unwrap().len;
            let old = alive.then_some(old);
            let start = Instant::now();
            cache.prepare(&next, &CancellationToken::new()).unwrap();
            eprintln!(
                "remap old_alive={alive}: {:?}, bytes {before} -> {}, offsets={}",
                start.elapsed(),
                cache.text.as_ref().unwrap().len,
                cache.offsets.len()
            );
            drop(old);
        }
    }

    #[test]
    fn history_find_remap_panic_leaves_the_old_index_and_offsets_together() {
        for alive in [true, false] {
            let commits = commits(600);
            let old = index(&commits);
            let next = index(&commits[100..]);
            let mut cache = HistoryFindCache::default();
            search(&mut cache, &old, &commits, &query("fix"));
            let old_identity = cache.index.clone();
            let offsets = cache.offsets.clone();
            let old = alive.then_some(old);
            cache.remap_panic_after = Some(10);
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    cache.prepare(&next, &CancellationToken::new()).unwrap();
                }))
                .is_err()
            );
            assert!(cache.index.ptr_eq(&old_identity));
            assert_eq!(cache.offsets, offsets);
            assert!(cache.previous.is_some());
            cache.remap_panic_after = None;
            search(&mut cache, &next, &commits[100..], &query("fix"));
            drop(old);
        }
    }

    #[test]
    fn history_find_compacts_records_removed_from_history() {
        for alive in [true, false] {
            let commits = commits(600);
            let old = index(&commits);
            let next = index(&commits[500..]);
            let mut cache = HistoryFindCache::default();
            search(&mut cache, &old, &commits, &query("fix"));
            let old_len = cache.text.as_ref().unwrap().len;
            let old = alive.then_some(old);
            assert_eq!(search(&mut cache, &next, &commits[500..], &query("fix")), 0);
            assert_eq!(cache.text.as_ref().unwrap().records, 100);
            assert!(cache.text.as_ref().unwrap().len < old_len / 2);
            // Compacted offsets must also serve a different query correctly.
            assert_eq!(search(&mut cache, &next, &commits[500..], &query("Bob")), 0);
            drop(old);
        }
    }

    #[test]
    fn history_find_ignores_relative_cache_directories() {
        for relative in ["cache", "./cache", "../cache"] {
            assert_eq!(
                text_dir(Some(OsStr::new(relative)), Some(OsStr::new("/home/u"))),
                text_dir(None, Some(OsStr::new("/home/u"))),
            );
            assert_eq!(text_dir(Some(OsStr::new(relative)), None), None);
        }
    }

    fn commits(count: usize) -> Vec<Commit> {
        (0..count)
            .map(|row| Commit {
                id: CommitId(format!("{:040x}", row + 1).into()),
                summary: if row.is_multiple_of(5) {
                    "fix typo"
                } else {
                    "feature"
                }
                .into(),
                author: if row.is_multiple_of(7) {
                    "Bob"
                } else {
                    "Élodie"
                }
                .into(),
                parent_ids: Default::default(),
                time: std::time::SystemTime::UNIX_EPOCH,
            })
            .collect()
    }

    fn index(commits: &[Commit]) -> HistoryIndexHandle {
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("find-cache".into()),
            LogScope::AllBranches,
            20,
        )
        .unwrap();
        for commit in commits {
            builder
                .push(
                    &gitcomet_core::hex::decode(commit.id.as_ref()).unwrap(),
                    [],
                    false,
                )
                .unwrap();
        }
        builder.finish(&CancellationToken::new()).unwrap()
    }

    fn query(text: &str) -> HistoryFindQuery {
        HistoryFindQuery::new(text, TextSearchOptions::default()).unwrap()
    }

    fn search(
        cache: &mut HistoryFindCache,
        index: &HistoryIndexHandle,
        commits: &[Commit],
        query: &HistoryFindQuery,
    ) -> usize {
        let mut reads = 0;
        let mut matches = Vec::new();
        let mut done = false;
        cache
            .search(
                index,
                query,
                &Arc::default(),
                &CancellationToken::new(),
                |range| {
                    reads += range.len();
                    Ok(HistoryRange {
                        snapshot: index.snapshot.clone(),
                        start: range.start,
                        commits: commits[range].to_vec(),
                    })
                },
                |chunk| {
                    assert!(!done);
                    done = chunk.done;
                    matches.extend(chunk.matches);
                },
            )
            .unwrap();
        assert!(done);
        let expected: Vec<u32> = commits
            .iter()
            .enumerate()
            .filter(|(_, commit)| query.matches(commit))
            .map(|(row, _)| row as u32)
            .collect();
        assert_eq!(matches, expected);
        reads
    }

    #[test]
    fn history_find_reuses_text_and_only_checks_previous_matches_for_a_refinement() {
        let commits = commits(600);
        let index = index(&commits);
        let mut cache = HistoryFindCache::default();
        assert_eq!(search(&mut cache, &index, &commits, &query("fix")), 600);
        let before = cache.comparisons;
        assert_eq!(search(&mut cache, &index, &commits, &query("fix typo")), 0);
        assert_eq!(cache.comparisons - before, 120);
        for text in ["Bob", "Élodie", "nothing", "fix"] {
            assert_eq!(search(&mut cache, &index, &commits, &query(text)), 0);
        }
        for options in [
            TextSearchOptions {
                match_case: true,
                ..Default::default()
            },
            TextSearchOptions {
                whole_word: true,
                ..Default::default()
            },
            TextSearchOptions {
                regex: true,
                ..Default::default()
            },
        ] {
            let query = HistoryFindQuery::new("fix", options).unwrap();
            assert_eq!(search(&mut cache, &index, &commits, &query), 0);
        }
    }

    #[test]
    fn history_find_reuses_text_after_the_old_index_is_dropped_and_rows_move() {
        let mut commits = commits(600);
        let old = index(&commits);
        let weak = Arc::downgrade(&old);
        let mut cache = HistoryFindCache::default();
        search(&mut cache, &old, &commits, &query("fix"));
        drop(old);
        assert!(weak.upgrade().is_none());
        commits.reverse();
        commits.truncate(400);
        commits.insert(
            0,
            Commit {
                id: CommitId(format!("{:040x}", 900).into()),
                ..commits[0].clone()
            },
        );
        let replacement = index(&commits);
        assert_eq!(
            search(&mut cache, &replacement, &commits, &query("fix typo")),
            1
        );
        assert_eq!(search(&mut cache, &replacement, &commits, &query("Bob")), 0);
        assert_eq!(cache.offsets.len(), commits.len());
    }

    /// A stash row shows its listed message, or its summary after the
    /// "WIP on main:" prefix. Find matches that text, like the row does.
    #[test]
    fn history_find_matches_stash_rows_on_what_they_show() {
        let mut commits = commits(3);
        commits[0].summary = "WIP on main: 1234567 wip".into();
        commits[1].summary = "On main: listed stash".into();
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("find-cache-stashes".into()),
            LogScope::AllBranches,
            20,
        )
        .unwrap();
        for (row, commit) in commits.iter().enumerate() {
            let id = gitcomet_core::hex::decode(commit.id.as_ref()).unwrap();
            builder.push(&id, [], row == 0).unwrap();
        }
        let index = builder.finish(&CancellationToken::new()).unwrap();
        let stashes = Arc::new(vec![StashEntry {
            index: 0,
            id: commits[1].id.clone(),
            message: "shown message".into(),
            created_at: None,
        }]);
        let mut cache = HistoryFindCache::default();
        for (text, expected) in [
            ("main", vec![]),
            ("1234567", vec![0]),
            ("shown", vec![1]),
            ("listed", vec![]),
            ("feature", vec![2]),
        ] {
            let mut found = Vec::new();
            cache
                .search(
                    &index,
                    &query(text),
                    &stashes,
                    &CancellationToken::new(),
                    |range| {
                        Ok(HistoryRange {
                            snapshot: index.snapshot.clone(),
                            start: range.start,
                            commits: commits[range].to_vec(),
                        })
                    },
                    |chunk| found.extend(chunk.matches),
                )
                .unwrap();
            assert_eq!(found, expected, "{text}");
        }
    }

    fn with_new_commit_on_top(commits: &[Commit], id: usize, summary: &str) -> Vec<Commit> {
        let mut next = vec![Commit {
            id: CommitId(format!("{id:040x}").into()),
            summary: summary.into(),
            ..commits[0].clone()
        }];
        next.extend_from_slice(commits);
        next
    }

    /// A fetch or a commit replaces the index with a few new rows on top. The
    /// same query only has to check those; every old row keeps its answer.
    #[test]
    fn history_find_same_query_over_a_replacement_index_checks_only_new_rows() {
        let commits = commits(600);
        let old = index(&commits);
        let mut cache = HistoryFindCache::default();
        for (id, summary) in [(900, "fix on top"), (901, "feature on top")] {
            search(&mut cache, &old, &commits, &query("fix"));
            let commits = with_new_commit_on_top(&commits, id, summary);
            let replacement = index(&commits);
            let before = cache.comparisons;
            assert_eq!(search(&mut cache, &replacement, &commits, &query("fix")), 1);
            assert_eq!(cache.comparisons - before, 1, "{summary}");
            // A refinement narrows the carried-over matches too.
            let before = cache.comparisons;
            search(&mut cache, &replacement, &commits, &query("fix t"));
            assert!(cache.comparisons - before <= 121, "{summary}");
        }

        // Equal regex and whole-word queries are not refinements, but their
        // old answers still hold.
        let regex = HistoryFindQuery::new(
            "typo$",
            TextSearchOptions {
                regex: true,
                ..Default::default()
            },
        )
        .unwrap();
        search(&mut cache, &old, &commits, &regex);
        let commits = with_new_commit_on_top(&commits, 902, "fix typo");
        let replacement = index(&commits);
        let before = cache.comparisons;
        search(&mut cache, &replacement, &commits, &regex);
        assert_eq!(cache.comparisons - before, 1);
    }

    /// Narrowing compacts removed text; widening must reload and check it.
    #[test]
    fn history_find_replacement_index_checks_cached_rows_the_old_search_never_saw() {
        let commits = commits(600);
        let full = index(&commits);
        let narrow = index(&commits[..300]);
        let mut cache = HistoryFindCache::default();
        search(&mut cache, &full, &commits, &query("Bob"));
        search(&mut cache, &narrow, &commits[..300], &query("fix"));
        let before = cache.comparisons;
        assert_eq!(search(&mut cache, &full, &commits, &query("fix")), 300);
        assert_eq!(cache.comparisons - before, 300);
    }

    /// Toggling Match Case on and back off cancels a scan; the plain query
    /// must still narrow its last complete answer instead of starting over.
    #[test]
    fn history_find_interrupted_scans_keep_the_last_completed_result() {
        for cancel in [true, false] {
            let commits = commits(600);
            let old = index(&commits);
            let mut cache = HistoryFindCache::default();
            search(&mut cache, &old, &commits, &query("fix"));
            let commits = with_new_commit_on_top(&commits, 900, "fix on top");
            let replacement = index(&commits);
            let match_case = HistoryFindQuery::new(
                "Fix",
                TextSearchOptions {
                    match_case: true,
                    ..Default::default()
                },
            )
            .unwrap();
            let cancellation = CancellationToken::new();
            let result = cache.search(
                &replacement,
                &match_case,
                &Arc::default(),
                &cancellation,
                |_| {
                    if cancel {
                        cancellation.cancel();
                        Err(Error::new(ErrorKind::Cancelled))
                    } else {
                        Err(Error::new(ErrorKind::Backend("transient".into())))
                    }
                },
                |chunk| assert!(!chunk.done),
            );
            assert!(result.is_err());
            let before = cache.comparisons;
            search(&mut cache, &replacement, &commits, &query("fix t"));
            assert_eq!(cache.comparisons - before, 121, "cancelled: {cancel}");
        }
    }

    #[test]
    fn history_find_text_lives_in_the_user_cache_where_tmp_may_be_memory() {
        let dir = text_dir(Some(OsStr::new("/x/cache")), Some(OsStr::new("/home/u")));
        let home = text_dir(Some(OsStr::new("")), Some(OsStr::new("/home/u")));
        if cfg!(any(target_os = "macos", not(unix))) {
            assert_eq!((dir, home), (None, None));
        } else {
            assert_eq!(dir, Some("/x/cache/gitcomet/history-find".into()));
            assert_eq!(home, Some("/home/u/.cache/gitcomet/history-find".into()));
            assert_eq!(text_dir(None, None), None);
        }
    }

    /// Timing probe: the same query after a fetch adds one commit on top.
    #[test]
    #[ignore]
    fn history_find_replacement_index_timing() {
        // Real ids spread over the index's fanout; the fixture's do not.
        let mut commits = commits(300_000);
        for (row, commit) in commits.iter_mut().enumerate() {
            let mut state = row as u64 + 1;
            let mut id = String::with_capacity(40);
            for _ in 0..3 {
                state = state.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(29)
                    ^ 0xbf58_476d_1ce4_e5b9;
                id.push_str(&format!("{state:016x}"));
            }
            id.truncate(40);
            commit.id = CommitId(id.into());
        }
        let old = index(&commits);
        let mut cache = HistoryFindCache::default();
        search(&mut cache, &old, &commits, &query("fix"));
        let mut next = vec![Commit {
            id: CommitId(format!("{:040x}", 900_000).into()),
            ..commits[0].clone()
        }];
        next.extend_from_slice(&commits);
        let replacement = index(&next);
        let started = Instant::now();
        search(&mut cache, &replacement, &next, &query("fix"));
        eprintln!(
            "same query over a replacement index: {:?}",
            started.elapsed()
        );
    }

    /// Timing probe: a refinement reads scattered earlier matches.
    #[test]
    #[ignore]
    fn history_find_sparse_refinement_timing() {
        let mut commits = commits(300_000);
        for (row, commit) in commits.iter_mut().enumerate() {
            commit.summary = if row.is_multiple_of(20) {
                "fix typo"
            } else {
                "feature"
            }
            .into();
        }
        let index = index(&commits);
        let mut cache = HistoryFindCache::default();
        search(&mut cache, &index, &commits, &query("fix"));
        let started = Instant::now();
        search(&mut cache, &index, &commits, &query("fix t"));
        eprintln!("sparse refinement: {:?}", started.elapsed());
    }

    #[test]
    fn history_find_failed_reads_retry_without_losing_or_duplicating_cached_rows() {
        let commits = commits(600);
        let index = index(&commits);
        let mut cache = HistoryFindCache::default();
        let result = cache.search(
            &index,
            &query("fix"),
            &Arc::default(),
            &CancellationToken::new(),
            |range| {
                if range.start > 0 {
                    return Err(Error::new(ErrorKind::Backend("transient".into())));
                }
                Ok(HistoryRange {
                    snapshot: index.snapshot.clone(),
                    start: range.start,
                    commits: commits[range].to_vec(),
                })
            },
            |chunk| assert!(!chunk.done),
        );
        assert!(result.is_err());
        assert!(cache.previous.is_none());
        assert_eq!(
            search(&mut cache, &index, &commits, &query("fix typo")),
            600 - HISTORY_BLOCK_SIZE
        );
    }

    #[test]
    fn history_find_cancelled_scans_do_not_cache_incomplete_results() {
        let commits = commits(600);
        let index = index(&commits);
        let mut cache = HistoryFindCache::default();
        let cancellation = CancellationToken::new();
        let result = cache.search(
            &index,
            &query("fix"),
            &Arc::default(),
            &cancellation,
            |range| {
                cancellation.cancel();
                Ok(HistoryRange {
                    snapshot: index.snapshot.clone(),
                    start: range.start,
                    commits: commits[range].to_vec(),
                })
            },
            |chunk| assert!(!chunk.done),
        );
        assert!(result.is_err());
        assert!(cache.previous.is_none());
        assert_eq!(
            search(&mut cache, &index, &commits, &query("fix typo")),
            600 - HISTORY_BLOCK_SIZE
        );
    }

    #[test]
    fn history_find_does_not_narrow_word_regex_or_new_sha_matches() {
        let mut commits = commits(1);
        commits[0].id = CommitId("abcd000000000000000000000000000000000000".into());
        commits[0].summary = "fixes feature".into();
        let index = index(&commits);
        let mut cache = HistoryFindCache::default();
        for (first, second, options) in [
            ("abc", "abcd", TextSearchOptions::default()),
            (
                "fix",
                "fixes",
                TextSearchOptions {
                    whole_word: true,
                    ..Default::default()
                },
            ),
            (
                "fix$",
                "fix$|feature",
                TextSearchOptions {
                    regex: true,
                    ..Default::default()
                },
            ),
        ] {
            search(
                &mut cache,
                &index,
                &commits,
                &HistoryFindQuery::new(first, options).unwrap(),
            );
            search(
                &mut cache,
                &index,
                &commits,
                &HistoryFindQuery::new(second, options).unwrap(),
            );
        }
    }
}
