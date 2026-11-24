use super::{ADDED_COMMIT_LIMIT, header};
use gitcomet_core::history_index::HistoryIndex;
use gitcomet_core::history_perf::Work;
use gitcomet_core::services::{CancellationToken, Result};

/// Replay the full date-order queue until its only eligible commit is the next
/// original row. At that point the remaining graph and queue match the old walk,
/// even when insertion order decided earlier timestamp ties.
pub(super) fn prefix(
    objects: &gix::OdbHandleArc,
    graph: Option<&gix::commitgraph::Graph>,
    index: &HistoryIndex,
    added_times: &[i64],
    tips: &[gix::ObjectId],
    cancellation: &CancellationToken,
) -> Result<Option<Vec<u32>>> {
    let added = added_times.len();
    let mut degrees = vec![0u32; index.len()];
    for row in 0..index.len() {
        if row.is_multiple_of(65_536) {
            cancellation.check_cancelled()?;
        }
        for &parent in index.parents(row) {
            let Some(count) = degrees.get_mut(parent as usize) else {
                return Ok(None);
            };
            *count += 1;
        }
    }
    let mut ready = gix::revwalk::PriorityQueue::new();
    let mut buffer = Vec::new();
    let mut remaining_old = ADDED_COMMIT_LIMIT;
    let mut enqueue =
        |row: u32, ready: &mut gix::revwalk::PriorityQueue<i64, u32>| -> Result<bool> {
            let time = match added_times.get(row as usize) {
                Some(&time) => time,
                None => {
                    if remaining_old == 0 {
                        return Ok(false);
                    }
                    remaining_old -= 1;
                    cancellation.check_cancelled()?;
                    let id = gix::oid::from_bytes_unchecked(
                        index.id_bytes(row as usize).expect("indexed row"),
                    );
                    match graph.and_then(|graph| graph.lookup(id).map(|at| graph.commit_at(at))) {
                        Some(commit) => commit.committer_timestamp() as i64,
                        None => header(objects, id, &mut buffer, Work::IndexObjectRead)?.1,
                    }
                }
            };
            ready.insert(time, row);
            Ok(true)
        };
    let mut seen_tips = rustc_hash::FxHashSet::default();
    for tip in tips {
        let Some(row) = index.position_bytes(tip.as_bytes()) else {
            return Ok(None);
        };
        if degrees[row] == 0 && seen_tips.insert(row) && !enqueue(row as u32, &mut ready)? {
            return Ok(None);
        }
    }

    let mut order = Vec::new();
    let mut seen_added = 0;
    let mut seen_old = 0;
    let mut old_end = 0;
    while let Some(row) = ready.pop_value() {
        if seen_added == added
            && old_end == seen_old
            && ready.is_empty()
            && row as usize == added + seen_old
        {
            return Ok(Some(order));
        }
        cancellation.check_cancelled()?;
        order.push(row);
        if (row as usize) < added {
            seen_added += 1;
        } else {
            seen_old += 1;
            old_end = old_end.max(row as usize + 1 - added);
        }
        for &parent in index.parents(row as usize) {
            degrees[parent as usize] -= 1;
            if degrees[parent as usize] == 0 && !enqueue(parent, &mut ready)? {
                return Ok(None);
            }
        }
    }
    Ok((order.len() == index.len()).then_some(order))
}
