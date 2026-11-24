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

/// A first scan has every commit in memory as it decodes it. Reading each
/// one back from the file it was just written to is the same work twice.
#[test]
fn history_find_matches_fresh_commits_without_reading_them_back() {
    let commits = commits(600);
    let index = index(&commits);
    let mut cache = HistoryFindCache::default();
    assert_eq!(search(&mut cache, &index, &commits, &query("fix")), 600);
    assert_eq!(cache.text_reads, 0);
    assert_eq!(cache.comparisons, 600);

    // The text was still written: an unrelated query reads it, not history.
    assert_eq!(search(&mut cache, &index, &commits, &query("Bob")), 0);
    assert_eq!(cache.text_reads, 600);
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
            state =
                state.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(29) ^ 0xbf58_476d_1ce4_e5b9;
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
