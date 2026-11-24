use super::*;

#[test]
fn random_ids_and_colliding_prefixes_cross_fanout_threshold() {
    for hash_len in [20, 32] {
        for count in [31, 65_535, 65_536, 100_000] {
            let mut builder = HistoryIndexBuilder::new(
                HistorySnapshot("random".into()),
                HistoryMode::FullReachable,
                hash_len,
            )
            .unwrap();
            let mut random = 0x9e3779b97f4a7c15u64;
            let mut ids = Vec::with_capacity(count);
            for row in 0..count {
                let mut id = vec![0; hash_len];
                for chunk in id.chunks_mut(8) {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    chunk.copy_from_slice(&random.to_be_bytes()[..chunk.len()]);
                }
                if row % 4 == 0 {
                    id[..8].fill(0xab);
                }
                id[hash_len - 4..].copy_from_slice(&(row as u32).to_be_bytes());
                builder
                    .push(
                        &id,
                        ids.last().map(|id: &Vec<u8>| id.as_slice()),
                        row % 9001 == 0,
                    )
                    .unwrap();
                ids.push(id);
            }
            let peak = builder.estimated_peak_bytes();
            let index = builder.finish(&CancellationToken::new()).unwrap();
            assert!(peak >= index.estimated_bytes());
            assert_eq!(index.fanout.is_empty(), count < 65_536);
            assert_eq!(index.probable_stash_rows().len(), count.div_ceil(9001));
            for (row, id) in ids.iter().enumerate() {
                assert_eq!(index.position_bytes(id), Some(row));
                let hex = index.commit_id(row).unwrap();
                assert!(index.row_matches_hex_id(row, hex.as_ref()));
                assert!(index.row_matches_hex_id(row, &hex.as_ref().to_uppercase()));
                assert_eq!(
                    index.parent_id_bytes(row, 0),
                    row.checked_sub(1).map(|row| ids[row].as_slice())
                );
            }
            assert_eq!(index.resolve_reference("abababababababab"), None);
            assert_eq!(index.position_bytes(&[0; 19]), None);
            assert!(!index.row_matches_hex_id(usize::MAX, &"0".repeat(hash_len * 2)));
            assert!(!index.row_matches_hex_id(0, &"é".repeat(hash_len)));
            assert_eq!(index.parent_id_bytes(usize::MAX, usize::MAX), None);
        }
    }
}

#[test]
fn large_index_resolves_external_and_colliding_parents_through_the_prefix_cache() {
    let count = 70_000usize;
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("external".into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    let mut random = 0x2545f4914f6cdd1du64;
    let mut next = move || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random
    };
    let mut id_for = |row: usize, external: bool| {
        let mut id = [0u8; 20];
        for chunk in id.chunks_mut(8) {
            chunk.copy_from_slice(&next().to_be_bytes()[..chunk.len()]);
        }
        // Every fourth ID shares one prefix; externals borrow a prefix that is
        // in the index so a miss must survive the equal-prefix run search.
        if row.is_multiple_of(4) || (external && row.is_multiple_of(2)) {
            id[..8].fill(0xab);
        }
        id[16..].copy_from_slice(&(row as u32).to_be_bytes());
        if external {
            id[15] ^= 0x80;
        }
        id
    };
    let ids: Vec<[u8; 20]> = (0..count).map(|row| id_for(row, false)).collect();
    let externals: Vec<Option<[u8; 20]>> = (0..count)
        .map(|row| (row % 7 == 3).then(|| id_for(row, true)))
        .collect();
    for row in 0..count {
        let mut parents: Vec<&[u8]> = Vec::new();
        if row > 0 {
            parents.push(&ids[row - 1]);
        }
        if let Some(external) = &externals[row] {
            parents.push(external);
        }
        builder.push(&ids[row], parents, false).unwrap();
    }
    let index = builder.finish(&CancellationToken::new()).unwrap();
    assert!(
        !index.fanout.is_empty(),
        "must exercise the prefix cache path"
    );
    for row in 0..count {
        assert_eq!(index.position_bytes(&ids[row]), Some(row));
        let parents = index.parents(row);
        if row > 0 {
            assert_eq!(parents[0], (row - 1) as u32, "row {row}");
            assert_eq!(index.parent_id_bytes(row, 0), Some(ids[row - 1].as_slice()));
        }
        if let Some(external) = &externals[row] {
            let slot = usize::from(row > 0);
            assert_eq!(parents[slot], MISSING_PARENT, "row {row}");
            assert_eq!(index.parent_id_bytes(row, slot), Some(external.as_slice()));
            assert_eq!(index.position_bytes(external), None);
        }
    }
}

#[test]
fn sparse_projection_maps_thousands_of_hidden_rows_both_ways() {
    let count = 200_000usize;
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("sparse".into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    for row in 0..count {
        let mut id = [0u8; 20];
        id[..8].copy_from_slice(&(row as u64 + 1).to_be_bytes());
        builder.push(&id, std::iter::empty(), false).unwrap();
    }
    let index = builder.finish(&CancellationToken::new()).unwrap();
    let mut hidden: Vec<u32> = (0..3_000u32)
        .map(|ix| (ix * 66_601 + 7) % count as u32)
        .collect();
    hidden.push(0);
    hidden.push(count as u32 - 1);
    let projection = HistoryProjection::new(index, hidden.clone());
    hidden.sort_unstable();
    hidden.dedup();
    assert_eq!(projection.len(), count - hidden.len());
    let mut hidden_before = vec![0usize; count + 1];
    for raw in 0..count {
        hidden_before[raw + 1] =
            hidden_before[raw] + usize::from(hidden.binary_search(&(raw as u32)).is_ok());
    }
    let mut visible_rows = Vec::with_capacity(projection.len());
    for (raw, &hidden_count) in hidden_before.iter().enumerate().take(count) {
        let is_hidden = hidden.binary_search(&(raw as u32)).is_ok();
        let expected_visible = (!is_hidden).then_some(raw - hidden_count);
        assert_eq!(
            projection.visible_position(raw),
            expected_visible,
            "raw {raw}"
        );
        assert_eq!(
            projection.visible_position_at_or_after(raw),
            raw - hidden_count,
            "raw {raw}"
        );
        if !is_hidden {
            visible_rows.push(raw);
        }
    }
    for (visible, &raw) in visible_rows.iter().enumerate() {
        assert_eq!(
            projection.raw_position(visible),
            Some(raw),
            "visible {visible}"
        );
    }
    assert_eq!(projection.raw_position(projection.len()), None);
    assert_eq!(
        projection.visible_position_at_or_after(count),
        projection.len()
    );
    assert_eq!(
        projection.visible_position_at_or_after(usize::MAX),
        projection.len()
    );
}

#[test]
#[ignore = "index construction phase timing at chromium scale"]
fn history_index_finish_phase_timing() {
    use std::time::Instant;
    let count = 1_922_916usize;
    let id = |row: usize| {
        let mut id = [0u8; 20];
        let mut seed = row as u64;
        for chunk in id.chunks_mut(8) {
            seed = seed.wrapping_add(0x9e3779b97f4a7c15);
            let mut value = seed;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
            chunk.copy_from_slice(&(value ^ (value >> 31)).to_be_bytes()[..chunk.len()]);
        }
        id
    };
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("timing".into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    let started = Instant::now();
    for row in 0..count {
        let mut parents = Vec::new();
        if row + 1 < count {
            parents.push(id(row + 1));
        }
        if row % 40 == 0 && row + 97 < count {
            parents.push(id(row + 97));
        }
        builder
            .push(&id(row), parents.iter().map(|id| id.as_slice()), false)
            .unwrap();
    }
    let pushed = started.elapsed();
    let started = Instant::now();
    let index = builder.finish(&CancellationToken::new()).unwrap();
    eprintln!(
        "history_index rows={count} push_s={:.3} finish_s={:.3} estimated_mib={:.1}",
        pushed.as_secs_f64(),
        started.elapsed().as_secs_f64(),
        index.estimated_bytes() as f64 / 1048576.0
    );
}

/// A history of `rows` commits with up to three parents each, every
/// parent a later row, as the tables `from_rows` takes.
fn scattered_rows(rows: u32, seed: u64) -> (Vec<u8>, Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut state = seed;
    let mut below = |limit: u32| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % u64::from(limit)) as u32
    };
    let (mut ids, mut offsets, mut parents, mut stashes) =
        (Vec::new(), vec![0], Vec::new(), Vec::new());
    for row in 0..rows {
        // IDs in no order, each its own.
        let mut id = [0u8; 20];
        id[..4].copy_from_slice(&row.wrapping_mul(2_654_435_761).to_be_bytes());
        id[4..8].copy_from_slice(&row.to_be_bytes());
        ids.extend_from_slice(&id);
        let later = rows - row - 1;
        for _ in 0..below(4).min(later) {
            parents.push(row + 1 + below(later));
        }
        offsets.push(parents.len() as u32);
        if below(50) == 0 {
            stashes.push(row);
        }
    }
    (ids, offsets, parents, stashes)
}

#[test]
fn rows_given_with_their_parents_as_rows_finish_as_the_builder_does() {
    let cancellation = CancellationToken::new();
    // Below and above the size at which IDs are sorted by a cached prefix.
    for rows in [1, 400, 70_000] {
        let (ids, offsets, parents, stashes) = scattered_rows(rows, u64::from(rows));
        let mut builder =
            HistoryIndexBuilder::new(HistorySnapshot("rows".into()), HistoryMode::AllBranches, 20)
                .unwrap();
        for row in 0..rows as usize {
            let parents = parents[offsets[row] as usize..offsets[row + 1] as usize]
                .iter()
                .map(|&parent| &ids[parent as usize * 20..][..20]);
            let stash = stashes.binary_search(&(row as u32)).is_ok();
            builder
                .push(&ids[row * 20..][..20], parents, stash)
                .unwrap();
        }
        let built = builder.finish(&cancellation).unwrap();
        let given = HistoryIndex::from_rows(
            HistorySnapshot("rows".into()),
            HistoryMode::AllBranches,
            20,
            ids,
            offsets,
            parents,
            stashes,
            &cancellation,
        )
        .unwrap();
        assert!(*given == *built, "{rows} rows");
        let by_id: Vec<_> = given
            .sorted_rows
            .iter()
            .map(|&row| given.id_bytes(row as usize).unwrap())
            .collect();
        assert!(by_id.is_sorted_by(|a, b| a < b), "{rows} rows");
        for row in (0..rows as usize).step_by(97) {
            let id = given.id_bytes(row).unwrap();
            assert_eq!(given.position_bytes(id), Some(row));
        }
    }
}

#[test]
fn rows_that_do_not_fit_together_are_refused() {
    let from_rows = |ids: Vec<u8>, offsets: Vec<u32>, parents: Vec<u32>, stashes: Vec<u32>| {
        HistoryIndex::from_rows(
            HistorySnapshot("rows".into()),
            HistoryMode::AllBranches,
            20,
            ids,
            offsets,
            parents,
            stashes,
            &CancellationToken::new(),
        )
        .is_ok()
    };
    let ids = || [[1u8; 20], [2u8; 20]].concat();
    assert!(from_rows(ids(), vec![0, 1, 1], vec![1], vec![0]));
    // A parent outside the index, offsets that end early, run backwards
    // or miss a row, a stash that is no row, and half an ID.
    assert!(!from_rows(ids(), vec![0, 1, 1], vec![2], vec![]));
    assert!(!from_rows(ids(), vec![0, 0, 0], vec![1], vec![]));
    assert!(!from_rows(ids(), vec![0, 1, 0], vec![1], vec![]));
    assert!(!from_rows(ids(), vec![0, 1], vec![1], vec![]));
    assert!(!from_rows(ids(), vec![0, 1, 1], vec![1], vec![2]));
    assert!(!from_rows(ids(), vec![0, 1, 1], vec![1], vec![1, 0]));
    assert!(!from_rows(
        ids()[..30].to_vec(),
        vec![0, 1],
        vec![0],
        vec![]
    ));
}

#[test]
fn finished_index_retains_no_growth_slack() {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("slack".into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    let mut ids = Vec::new();
    for row in 0..70_000u32 {
        let mut id = [0u8; 20];
        id[..4].copy_from_slice(&row.to_be_bytes());
        let parent = ids.last().copied();
        builder
            .push(
                &id,
                parent.iter().map(|parent: &[u8; 20]| parent.as_slice()),
                row % 5_000 == 0,
            )
            .unwrap();
        ids.push(id);
    }
    let index = builder.finish(&CancellationToken::new()).unwrap();
    assert_eq!(index.ids.capacity(), index.ids.len());
    assert_eq!(index.parent_offsets.capacity(), index.parent_offsets.len());
    assert_eq!(index.parents.capacity(), index.parents.len());
    assert_eq!(
        index.probable_stashes.capacity(),
        index.probable_stashes.len()
    );
    assert_eq!(
        index.estimated_bytes(),
        index.len() * 20 + (index.len() * 2 + 1) * 4 + (index.len() - 1) * 4 + 65_537 * 4 + 14 * 4
    );
}

#[test]
fn invalid_parent_does_not_corrupt_builder() {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("bad".into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    assert!(
        builder
            .push(&[1; 20], [[2; 20].as_slice(), [3; 19].as_slice()], false)
            .is_err()
    );
    assert!(builder.is_empty());
    builder.push(&[4; 20], std::iter::empty(), false).unwrap();
    let index = builder.finish(&CancellationToken::new()).unwrap();
    assert_eq!(index.id_bytes(0), Some([4; 20].as_slice()));
    assert!(index.parents(0).is_empty());
}

/// A row's ID, its parents' IDs and whether it is a stash candidate.
type Row = (Vec<u8>, Vec<Vec<u8>>, bool);

/// Rows for a random history: each row's parents are later rows, a row
/// outside the table, or (rarely) an earlier row.
fn random_rows(rows: usize, hash_len: usize, seed: u64) -> Vec<Row> {
    let mut state = seed;
    let mut next = move || {
        // SplitMix64.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let id = |row: usize| -> Vec<u8> {
        // Spread, but with many shared two-byte prefixes.
        let mixed = (row as u64 ^ seed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut id = vec![0u8; hash_len];
        id[..8].copy_from_slice(&mixed.to_be_bytes());
        id[1] &= 0x03;
        id[8..16].copy_from_slice(&(row as u64).to_be_bytes());
        id
    };
    (0..rows)
        .map(|row| {
            let parents = (0..next() % 3)
                .map(|_| match next() % 16 {
                    0 => vec![0xEE; hash_len],
                    1 if row > 0 => id(next() as usize % row),
                    _ if row + 1 < rows => id(row + 1 + next() as usize % (rows - row - 1)),
                    _ => vec![0xDD; hash_len],
                })
                .collect();
            (id(row), parents, next() % 9 == 0)
        })
        .collect()
}

fn built(rows: &[Row], hash_len: usize) -> HistoryIndexBuilder {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("fixture".into()),
        HistoryMode::FullReachable,
        hash_len,
    )
    .unwrap();
    for (id, parents, stash) in rows {
        builder
            .push(id, parents.iter().map(Vec::as_slice), *stash)
            .unwrap();
    }
    builder
}

#[test]
fn rows_added_before_an_index_equal_one_build_of_them_all() {
    let cancel = CancellationToken::new();
    // Below, across and above the fanout threshold, and both ID lengths.
    for (rows, added, hash_len) in [
        (40, 0, 20),
        (40, 1, 20),
        (40, 40, 32),
        (900, 37, 20),
        (65_600, 100, 20),
        (66_000, 600, 32),
        (70_000, 65_990, 20),
    ] {
        let all = random_rows(rows, hash_len, rows as u64 * 31 + added as u64);
        let whole = built(&all, hash_len).finish(&cancel).unwrap();
        let older = built(&all[added..], hash_len).finish(&cancel).unwrap();
        let grown = built(&all[..added], hash_len)
            .finish_before(&older, &cancel)
            .unwrap();
        assert!(
            grown == whole,
            "rows={rows} added={added} hash_len={hash_len}"
        );
    }
}

#[test]
fn reordered_prefix_matches_a_full_build_with_external_edges_and_stashes() {
    let cancel = CancellationToken::new();
    for (count, hash_len) in [(193usize, 20), (65_700, 32)] {
        let rows = random_rows(count, hash_len, 0x70b2);
        let mut index = built(&rows, hash_len).finish(&cancel).unwrap();
        let prefix = 113usize;
        let order: Vec<u32> = (0..prefix as u32)
            .map(|row| row * 17 % prefix as u32)
            .collect();
        let reordered: Vec<_> = order
            .iter()
            .map(|&row| rows[row as usize].clone())
            .chain(rows[prefix..].iter().cloned())
            .collect();
        let index_mut = Arc::get_mut(&mut index).unwrap();
        assert!(index_mut.reorder_prefix(&[1, 1]).is_err());
        assert!(index_mut.reorder_prefix(&[1, 2]).is_err());
        index_mut.reorder_prefix(&order).unwrap();
        assert!(index == built(&reordered, hash_len).finish(&cancel).unwrap());
    }
}

#[test]
fn rows_added_before_an_index_must_be_new_and_of_its_id_length() {
    let cancel = CancellationToken::new();
    let all = random_rows(8, 20, 7);
    let older = built(&all[2..], 20).finish(&cancel).unwrap();
    assert!(
        built(&all[2..3], 20)
            .finish_before(&older, &cancel)
            .is_err()
    );
    assert!(built(&[], 32).finish_before(&older, &cancel).is_err());
}
