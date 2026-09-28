//! Searchable commit text, reused across queries and index replacements.
//!
//! Text lives in a private temporary file: keeping millions of decoded commits
//! in memory would dwarf the index itself. Only row offsets and the last
//! completed result stay in memory. Clearing the find state drops the file.

use super::HistoryFindChunk;
use gitcomet_core::domain::Commit;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::history_index::{
    HISTORY_BLOCK_SIZE, HistoryIndex, HistoryIndexHandle, HistoryRange,
};
use gitcomet_core::services::{CancellationToken, Result};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

const MISSING: u64 = u64::MAX;
const REPORT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(crate) struct HistoryFindCache {
    index: Weak<HistoryIndex>,
    offsets: Vec<u64>,
    text: Option<TextFile>,
    previous: Option<(HistoryFindQuery, Vec<u32>)>,
    #[cfg(test)]
    comparisons: usize,
}

impl HistoryFindCache {
    pub(crate) fn search(
        &mut self,
        index: &HistoryIndexHandle,
        query: &HistoryFindQuery,
        cancellation: &CancellationToken,
        mut read: impl FnMut(Range<usize>) -> Result<HistoryRange>,
        mut report: impl FnMut(HistoryFindChunk),
    ) -> Result<()> {
        self.prepare(index, cancellation)?;
        let narrowed = self
            .previous
            .take()
            .filter(|(old, _)| query.is_refinement_of(old))
            .map(|(_, rows)| rows);
        let rows: Box<dyn Iterator<Item = usize>> = match narrowed {
            Some(rows) => Box::new(rows.into_iter().map(|row| row as usize)),
            None => Box::new(0..index.len()),
        };
        let mut buffer = Vec::new();
        let mut matches = Vec::new();
        let mut pending = Vec::new();
        let mut last_report = Instant::now();
        for (visited, row) in rows.enumerate() {
            if visited.is_multiple_of(HISTORY_BLOCK_SIZE) {
                cancellation.check_cancelled()?;
            }
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
                .read(self.offsets[row], &mut buffer)
                .map_err(io_error)?;
            #[cfg(test)]
            {
                self.comparisons += 1;
            }
            if query.matches_fields(id, summary, author) {
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
        self.previous = Some((query.clone(), matches));
        report(HistoryFindChunk {
            matches: pending,
            done: true,
        });
        Ok(())
    }

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
        let text = self.text.as_mut().unwrap();
        let mut offsets = vec![MISSING; index.len()];
        let mut buffer = Vec::new();
        let mut offset = 0;
        let mut visited = 0usize;
        while offset < text.len {
            if visited.is_multiple_of(HISTORY_BLOCK_SIZE) {
                cancellation.check_cancelled()?;
            }
            let (id, _, _) = text.read(offset, &mut buffer).map_err(io_error)?;
            if let Some(row) = index.position(id) {
                offsets[row] = offset;
            }
            offset = text.position;
            visited += 1;
        }
        self.offsets = offsets;
        self.index = Arc::downgrade(index);
        self.previous = None;
        Ok(())
    }
}

fn io_error(error: io::Error) -> Error {
    Error::new(ErrorKind::Io(error.kind()))
}

#[derive(Debug)]
struct TextFile {
    reader: BufReader<File>,
    position: u64,
    len: u64,
}

impl TextFile {
    fn new() -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, tempfile::tempfile()?),
            position: 0,
            len: 0,
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
        self.reader.seek(SeekFrom::Start(self.len))?;
        self.position = MISSING;
        if let Err(error) = self.reader.get_mut().write_all(&bytes) {
            // A failed append must not leave a partial record for a retry.
            let _ = self.reader.get_mut().set_len(self.len);
            return Err(error);
        }
        self.len += bytes.len() as u64;
        self.position = self.len;
        Ok(offsets)
    }

    fn read<'a>(
        &mut self,
        offset: u64,
        buffer: &'a mut Vec<u8>,
    ) -> io::Result<(&'a str, &'a str, &'a str)> {
        if offset != self.position {
            self.reader.seek(SeekFrom::Start(offset))?;
        }
        // If a read fails partway through, the next attempt must seek again.
        self.position = MISSING;
        let mut header = [0u8; 12];
        self.reader.read_exact(&mut header)?;
        let lengths = [0, 4, 8]
            .map(|start| u32::from_le_bytes(header[start..start + 4].try_into().unwrap()) as usize);
        let total = lengths.iter().sum::<usize>();
        buffer.resize(total, 0);
        self.reader.read_exact(buffer)?;
        self.position = offset + (header.len() + total) as u64;
        let (id, rest) = buffer.split_at(lengths[0]);
        let (summary, author) = rest.split_at(lengths[1]);
        let utf8 = |bytes| {
            std::str::from_utf8(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
        };
        Ok((utf8(id)?, utf8(summary)?, utf8(author)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, LogScope};
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;
    use gitcomet_core::text_search::TextSearchOptions;

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

    #[test]
    fn history_find_failed_reads_retry_without_losing_or_duplicating_cached_rows() {
        let commits = commits(600);
        let index = index(&commits);
        let mut cache = HistoryFindCache::default();
        let result = cache.search(
            &index,
            &query("fix"),
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
