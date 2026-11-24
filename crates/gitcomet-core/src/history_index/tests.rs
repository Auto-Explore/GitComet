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
fn abbreviated_ids_preserve_odd_nibble_boundaries_and_ambiguity() {
    for hash_len in [20, 32] {
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("prefixes".into()),
            HistoryMode::FullReachable,
            hash_len,
        )
        .unwrap();
        let mut first = vec![0xab; hash_len];
        first[3] = 0xc0;
        let mut second = first.clone();
        second[3] = 0xcf;
        let mut third = first.clone();
        third[3] = 0xd0;
        for id in [&first, &second, &third] {
            builder
                .push(id, std::iter::empty::<&[u8]>(), false)
                .unwrap();
        }
        let index = builder.finish(&CancellationToken::new()).unwrap();
        assert_eq!(
            index.resolve_reference("abababc"),
            None,
            "both low-nibble extremes match"
        );
        assert_eq!(index.resolve_reference("abababd"), Some(2));
        assert_eq!(index.resolve_reference("ABABABCF"), Some(1));
        for bad in [
            "ababab",
            "abababe",
            "abababg",
            "abababé",
            "abababcé",
            "abababc ",
        ] {
            assert_eq!(index.resolve_reference(bad), None, "{bad:?}");
        }
        let full = crate::hex::encode(&third);
        for len in 7..=full.len() {
            assert_eq!(index.resolve_reference(&full[..len]), Some(2));
        }
    }
}

#[test]
fn deferred_stash_filter_preserves_parent_order_and_external_ids() {
    for hash_len in [20, 32] {
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("stashes".into()),
            HistoryMode::AllBranches,
            hash_len,
        )
        .unwrap();
        let ids: Vec<_> = (1..=6).map(|byte| vec![byte; hash_len]).collect();
        for row in 0..4 {
            builder
                .push(&ids[row], [ids[4].as_slice(), ids[5].as_slice()], row != 1)
                .unwrap();
        }
        let mut visited = Vec::new();
        builder
            .retain_probable_stashes(|id, parents| {
                visited.push(id[0]);
                assert_eq!(
                    parents.collect::<Vec<_>>(),
                    [ids[4].as_slice(), ids[5].as_slice()]
                );
                Ok(id[0] % 2 == 1)
            })
            .unwrap();
        assert_eq!(visited, [1, 3, 4]);
        let index = builder.finish(&CancellationToken::new()).unwrap();
        assert_eq!(index.probable_stash_rows(), [0, 2]);
        for row in 0..4 {
            assert_eq!(index.parent_id_bytes(row, 0), Some(ids[4].as_slice()));
            assert_eq!(index.parent_id_bytes(row, 1), Some(ids[5].as_slice()));
        }
    }
}

#[test]
fn deferred_stash_filter_stops_on_cancellation() {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("cancel".into()),
        HistoryMode::AllBranches,
        20,
    )
    .unwrap();
    for byte in 1..=4 {
        builder.push(&[byte; 20], std::iter::empty(), true).unwrap();
    }
    let cancellation = CancellationToken::new();
    let mut visited = 0;
    let error = builder
        .retain_probable_stashes(|_, _| {
            visited += 1;
            if visited == 2 {
                cancellation.cancel();
            }
            cancellation.check_cancelled()?;
            Ok(true)
        })
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert_eq!(visited, 2);
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
fn indexed_first_parent_position_matches_decoded_topology() {
    use crate::squash::{FirstParentPosition, first_parent_position_indexed};
    // e -> m(c, x); c -> b; x -> b; b -> a; a is the root.
    let topology: [(u8, &[u8]); 6] = [
        (6, &[5]),
        (5, &[4, 3]),
        (4, &[2]),
        (3, &[2]),
        (2, &[1]),
        (1, &[]),
    ];
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("merge".into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    for (id, parents) in topology {
        let parents: Vec<Vec<u8>> = parents.iter().map(|parent| vec![*parent; 20]).collect();
        builder
            .push(&[id; 20], parents.iter().map(Vec::as_slice), false)
            .unwrap();
    }
    let index = builder.finish(&CancellationToken::new()).unwrap();
    let commits: Vec<_> = (0..index.len())
        .map(|row| Commit {
            id: index.commit_id(row).unwrap(),
            parent_ids: (0..index.parents(row).len())
                .filter_map(|parent| index.parent_commit_id(row, parent))
                .collect(),
            author: "a".into(),
            summary: "s".into(),
            time: std::time::UNIX_EPOCH,
        })
        .collect();
    let head = &commits[0].id;
    for target in &commits {
        assert_eq!(
            first_parent_position_indexed(&index, head, &target.id),
            crate::squash::first_parent_position(&commits, head, &target.id),
            "target {}",
            target.id.as_ref()
        );
    }
    let id = |row: usize| commits[row].id.clone();
    assert_eq!(
        first_parent_position_indexed(&index, head, &id(1)),
        FirstParentPosition::Linear { distance: 1 }
    );
    assert_eq!(
        first_parent_position_indexed(&index, head, &id(2)),
        FirstParentPosition::BelowMerge
    );
    assert_eq!(
        first_parent_position_indexed(&index, head, &id(3)),
        FirstParentPosition::NotFound
    );
    assert_eq!(
        first_parent_position_indexed(&index, head, &CommitId("missing".into())),
        FirstParentPosition::NotFound
    );
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
