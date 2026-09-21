//! Frontier transitions touch only lanes participating in a row. Dense paint
//! arrays are an optional output used by visible windows and the legacy view.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
struct LiveLane {
    id: u32,
    target: u32,
    born: usize,
    color: LaneColorIx,
}

/// A column with no lane in a checkpoint. Live targets are row indices, which
/// the index keeps below this value.
const NO_LANE: u32 = u32::MAX;

/// A frontier stored one target and one colour per column: five bytes a
/// column, where a list of live lanes spelled out the column and a lane
/// identity for twelve. The identity only ever distinguishes the main lane, so
/// its column is kept instead; every other restored lane gets identity zero,
/// which no walk assigns (identities start at one).
#[derive(Clone, Debug)]
pub(in crate::view) struct GraphCheckpoint {
    targets: Box<[u32]>,
    colors: Box<[LaneColorIx]>,
    main_col: Option<u16>,
    next_id: u32,
    next_color: usize,
    main: Option<u32>,
    main_target: Option<usize>,
    pending: bool,
}

impl GraphCheckpoint {
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn estimated_bytes(&self) -> usize {
        self.targets.len() * std::mem::size_of::<u32>()
            + self.colors.len() * std::mem::size_of::<LaneColorIx>()
    }
    #[cfg(test)]
    pub fn lane_capacity_slack(&self) -> usize {
        0
    }
    #[cfg(test)]
    fn width(&self) -> usize {
        self.targets.len()
    }
    pub fn restore(&self) -> GraphWalk {
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::CheckpointRestore);
        // Sized up front: a wide frontier restores thousands of lanes on every
        // window miss, and growing the map from empty rehashes it a dozen times.
        let live = self
            .targets
            .iter()
            .filter(|&&target| target != NO_LANE)
            .count();
        let mut walk = GraphWalk {
            next_id: self.next_id,
            next_color: self.next_color,
            main: self.main,
            main_target: self.main_target,
            pending: self.pending,
            lanes: Vec::with_capacity(self.targets.len()),
            targets: FxHashMap::with_capacity_and_hasher(live, Default::default()),
            free: BTreeSet::new(),
            colors: [0; LANE_COLOR_PALETTE_SIZE],
        };
        for (col, (&target, &color)) in self.targets.iter().zip(&*self.colors).enumerate() {
            if target == NO_LANE {
                walk.free.insert(col);
                walk.lanes.push(None);
                continue;
            }
            let id = match (self.main_col, self.main) {
                (Some(main_col), Some(main)) if usize::from(main_col) == col => main,
                _ => 0,
            };
            walk.place(
                col,
                LiveLane {
                    id,
                    target,
                    color,
                    born: usize::MAX,
                },
            );
        }
        walk
    }
}

pub(in crate::view) struct GraphTransition {
    pub paint: GraphRow,
    /// Overrides to the preceding row's outgoing lanes, including whiskers.
    pub now: SmallVec<[(usize, LanePaint); 4]>,
    /// Only changed outgoing columns; holes and births update labels and spans.
    pub next: SmallVec<[(usize, LanePaint); 4]>,
    pub next_len: usize,
}

#[derive(Debug)]
pub(in crate::view) struct GraphWalk {
    lanes: Vec<Option<LiveLane>>,
    targets: FxHashMap<u32, SmallVec<[usize; 2]>>,
    free: BTreeSet<usize>,
    colors: [u32; LANE_COLOR_PALETTE_SIZE],
    next_id: u32,
    next_color: usize,
    main: Option<u32>,
    main_target: Option<usize>,
    pending: bool,
}

impl GraphWalk {
    pub fn new(main_target: Option<usize>) -> Self {
        let mut walk = Self {
            lanes: Vec::new(),
            targets: FxHashMap::default(),
            free: BTreeSet::new(),
            colors: [0; LANE_COLOR_PALETTE_SIZE],
            next_id: 1,
            next_color: 0,
            main: None,
            main_target,
            pending: main_target.is_some(),
        };
        if let Some(target) = main_target {
            walk.place(
                0,
                LiveLane {
                    id: 1,
                    target: target as u32,
                    color: 0,
                    born: usize::MAX,
                },
            );
            walk.main = Some(1);
            walk.next_id = 2;
            walk.next_color = 1;
        }
        walk
    }
    pub fn checkpoint(&self) -> GraphCheckpoint {
        // Trailing holes are trimmed after every step, so the dense width is
        // the frontier's; boxed slices carry no growth slack.
        let targets = self
            .lanes
            .iter()
            .map(|lane| lane.map_or(NO_LANE, |lane| lane.target))
            .collect();
        let colors = self
            .lanes
            .iter()
            .map(|lane| lane.map_or(0, |lane| lane.color))
            .collect();
        let main_col = self.main.and_then(|main| {
            self.lanes
                .iter()
                .position(|lane| lane.is_some_and(|lane| lane.id == main))
                .map(lane_col)
        });
        GraphCheckpoint {
            targets,
            colors,
            main_col,
            next_id: self.next_id,
            next_color: self.next_color,
            main: self.main,
            main_target: self.main_target,
            pending: self.pending,
        }
    }
    fn place(&mut self, col: usize, lane: LiveLane) {
        if col == self.lanes.len() {
            self.lanes.push(Some(lane));
        } else {
            debug_assert!(self.lanes[col].is_none());
            self.lanes[col] = Some(lane);
        }
        self.free.remove(&col);
        self.targets.entry(lane.target).or_default().push(col);
        self.colors[usize::from(lane.color)] += 1;
    }
    fn take(&mut self, col: usize) -> Option<LiveLane> {
        let lane = self.lanes.get_mut(col)?.take()?;
        let targets = self.targets.get_mut(&lane.target).unwrap();
        targets.retain(|target| *target != col);
        if targets.is_empty() {
            self.targets.remove(&lane.target);
        }
        self.free.insert(col);
        self.colors[usize::from(lane.color)] -= 1;
        Some(lane)
    }
    fn alloc(&self, prefer: usize) -> usize {
        self.free
            .range(prefer..)
            .next()
            .or_else(|| self.free.first())
            .copied()
            .unwrap_or(self.lanes.len())
    }
    fn color(&mut self, avoid: &[LaneColorIx]) -> LaneColorIx {
        let start = self.next_color;
        for offset in 0..LANE_COLOR_PALETTE_SIZE {
            let color = ((start + offset) % LANE_COLOR_PALETTE_SIZE) as LaneColorIx;
            if self.colors[usize::from(color)] == 0 && !avoid.contains(&color) {
                self.next_color = start + offset + 1;
                return color;
            }
        }
        self.next_color = start + 1;
        (start % LANE_COLOR_PALETTE_SIZE) as LaneColorIx
    }
    fn birth(&mut self, col: usize, target: usize, row: usize, color: LaneColorIx) {
        let id = self.next_id;
        self.next_id += 1;
        self.place(
            col,
            LiveLane {
                id,
                target: target as u32,
                born: row,
                color,
            },
        );
    }
    pub fn step(&mut self, row: usize, parents: &[usize], merge: bool, head: bool) -> GraphRow {
        self.transition(row, parents, merge, head, true).paint
    }
    pub fn transition(
        &mut self,
        row: usize,
        parents: &[usize],
        merge: bool,
        head: bool,
        materialize: bool,
    ) -> GraphTransition {
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::GraphTransition);
        if materialize {
            gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintRow);
        }
        let mut hits = self.targets.get(&(row as u32)).cloned().unwrap_or_default();
        hits.sort_unstable();
        let had_hits = !hits.is_empty();
        let mut now = SmallVec::new();
        if hits.is_empty() {
            let color = self.color(&[]);
            let col = self.alloc(0);
            self.birth(col, row, row, color);
            hits.push(col);
            now.push((col, LanePaint::lane(color, false, false)));
        }
        let only_main = hits.len() == 1 && self.main == self.lanes[hits[0]].map(|lane| lane.id);
        let force = head
            && had_hits
            && hits.len() == 1
            && parents.len() <= 1
            && !(self.main_target == Some(row) && only_main);
        let node = hits
            .iter()
            .copied()
            .find(|&col| self.main == self.lanes[col].map(|lane| lane.id))
            .unwrap_or(hits[0]);
        let fork_color = force.then(|| self.color(&[]));
        let fork = fork_color.and_then(|color| {
            self.lanes
                .get(node + 1)
                .copied()
                .flatten()
                .is_none()
                .then_some((node + 1, color))
        });
        if let Some((col, color)) = fork {
            now.push((col, LanePaint::lane(color, false, false)));
        }
        let adopt = force && !only_main;
        let node_color = if adopt {
            fork_color.unwrap()
        } else {
            self.lanes[node].unwrap().color
        };
        let mut lanes_now = LanePaints::new();
        if materialize {
            let len = self.lanes.len().max(fork.map_or(0, |(col, _)| col + 1));
            lanes_now.reserve(len);
            for col in 0..len {
                lanes_now.push(self.lanes.get(col).copied().flatten().map_or(
                    LanePaint::HOLE,
                    |lane| {
                        LanePaint::lane(
                            lane.color,
                            lane.born != row
                                && !(self.pending
                                    && self.main_target == Some(row)
                                    && self.main == Some(lane.id)),
                            false,
                        )
                    },
                ));
            }
            if let Some((col, color)) = fork {
                lanes_now[col] = LanePaint::lane(color, false, false);
            }
        }
        let pos = hits.iter().position(|&col| col == node).unwrap();
        hits.swap(0, pos);
        let mut joins = GraphEdges::new();
        for &col in hits.iter().skip(1) {
            joins.push(GraphEdge {
                from_col: lane_col(col),
                to_col: lane_col(node),
                color_ix: self.lanes[col].unwrap().color,
            });
        }
        if let Some((col, color)) = fork {
            joins.push(GraphEdge {
                from_col: lane_col(col),
                to_col: lane_col(node),
                color_ix: color,
            });
        }
        let mut changed: SmallVec<[usize; 4]> = hits.iter().copied().collect();
        let mut ended: SmallVec<[LaneColorIx; 4]> = SmallVec::new();
        for (ix, &col) in hits.iter().enumerate() {
            if let Some(&parent) = parents.get(ix) {
                let lane = self.lanes[col].as_mut().unwrap();
                let old_target = lane.target;
                lane.target = parent as u32;
                let targets = self.targets.get_mut(&old_target).unwrap();
                targets.retain(|target| *target != col);
                if targets.is_empty() {
                    self.targets.remove(&old_target);
                }
                self.targets.entry(parent as u32).or_default().push(col);
            } else {
                ended.push(self.take(col).unwrap().color);
            }
        }
        if adopt && let Some(lane) = self.lanes.get_mut(node).and_then(Option::as_mut) {
            ended.push(lane.color);
            self.colors[usize::from(lane.color)] -= 1;
            lane.color = fork_color.unwrap();
            self.colors[usize::from(lane.color)] += 1;
            lane.id = self.next_id;
            self.next_id += 1;
            lane.born = row;
        }
        for &parent in parents.iter().skip(parents.len().min(hits.len())) {
            if self.targets.contains_key(&(parent as u32)) {
                continue;
            }
            let color = self.color(&ended);
            let col = self.alloc(node + 1);
            self.birth(col, parent, row, color);
            changed.push(col);
        }
        while self.lanes.last().is_some_and(Option::is_none) {
            self.free.remove(&(self.lanes.len() - 1));
            self.lanes.pop();
        }
        if let Some((col, _)) = fork {
            changed.push(col);
        }
        changed.sort_unstable();
        changed.dedup();
        let outgoing = |slot: Option<LiveLane>| {
            slot.map_or(LanePaint::HOLE, |lane| {
                LanePaint::lane(lane.color, false, lane.born == row)
            })
        };
        let next: SmallVec<[(usize, LanePaint); 4]> = changed
            .into_iter()
            .map(|col| (col, outgoing(self.lanes.get(col).copied().flatten())))
            .collect();
        let (lanes_next, from_node_cols) = if materialize {
            // Every lane born this row is a changed column, so the summary
            // needs no second pass over the frontier.
            (
                self.lanes.iter().map(|lane| outgoing(*lane)).collect(),
                next.iter()
                    .filter(|(_, lane)| lane.is_active() && lane.starts_at_node())
                    .map(|(col, _)| lane_col(*col))
                    .collect(),
            )
        } else {
            (LanePaints::new(), FromNodeCols::new())
        };
        let mut edges = GraphEdges::new();
        for &parent in parents.iter().skip(1) {
            if let Some(col) = self.targets.get(&(parent as u32)).and_then(|cols| {
                cols.iter()
                    .copied()
                    .filter(|&col| self.lanes[col].unwrap().born != row)
                    .min()
            }) {
                edges.push(GraphEdge {
                    from_col: lane_col(node),
                    to_col: lane_col(col),
                    color_ix: self.lanes[col].unwrap().color,
                });
            }
        }
        self.pending = false;
        GraphTransition {
            paint: GraphRow {
                lanes_now,
                lanes_next,
                joins_in: joins,
                edges_out: edges,
                node_col: lane_col(node),
                node_color_ix: node_color,
                is_merge: merge,
                from_node_cols,
            },
            now,
            next,
            next_len: self.lanes.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_cost_five_bytes_per_column_and_restore_presized() {
        for width in [1usize, 64, 512, 5261] {
            let mut walk = GraphWalk::new(None);
            for row in 0..width {
                walk.step(row, &[row + width], false, false);
            }
            let checkpoint = walk.checkpoint();
            assert_eq!(checkpoint.width(), width);
            assert_eq!(checkpoint.lane_capacity_slack(), 0, "width={width}");
            assert_eq!(checkpoint.estimated_bytes(), width * 5);
            let restored = checkpoint.restore();
            assert_eq!(restored.lanes.len(), width);
            assert_eq!(
                restored.lanes.capacity(),
                width,
                "restore is sized up front"
            );
            assert!(restored.targets.capacity() >= width);
        }
    }

    /// Holes and the main lane survive the dense layout: a restored walk must
    /// keep producing the original frontier's rows, hole reuse included.
    #[test]
    fn dense_checkpoints_keep_holes_and_the_main_lane() {
        // Six parallel lanes; the lane through column 2 ends at rows 8 and 20
        // and is only re-born six rows later, so the checkpoints at rows 12 and
        // 24 are taken while the frontier has an interior hole. Every eleventh
        // row is a branch head, so head handling crosses restores too.
        let mut walk = GraphWalk::new(Some(3));
        let mut original = super::super::oracle::OracleGraphWalk::new(Some(3));
        let parents = |row: usize| -> SmallVec<[usize; 4]> {
            let mut parents = SmallVec::new();
            if row != 8 && row != 20 {
                parents.push(row + 6);
            }
            if row % 15 == 0 {
                parents.push(row + 7);
            }
            parents
        };
        for row in 0..300 {
            let parents = parents(row);
            if row.is_multiple_of(12) && row > 0 {
                let checkpoint = walk.checkpoint();
                if row == 12 || row == 24 {
                    assert!(
                        checkpoint.targets.contains(&NO_LANE),
                        "row {row}: the lane ended six rows earlier must still be a hole"
                    );
                }
                assert!(
                    checkpoint.main_col.is_some(),
                    "row {row}: the seeded main lane follows first parents throughout"
                );
                walk = checkpoint.restore();
            }
            let expected = original.step(row, &parents, parents.len() > 1, row % 11 == 0);
            let actual = walk.step(row, &parents, parents.len() > 1, row % 11 == 0);
            assert_eq!(actual, expected, "row={row}");
        }
    }

    #[test]
    fn transitions_and_restored_checkpoints_match_original_frontier() {
        for width in [1, 8, 64, 512, 5261] {
            let count = width * 3 + 2200;
            let mut walk = GraphWalk::new(Some(width / 2));
            let mut original = super::super::oracle::OracleGraphWalk::new(Some(width / 2));
            let mut no_paint = GraphWalk::new(Some(width / 2));
            let mut random = 0x92e2a972u64;
            for row in 0..count {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let mut parents: SmallVec<[usize; 4]> = SmallVec::new();
                if row + width < count {
                    parents.push(row + width);
                }
                if row % 7 == 0 && row + 1 < count {
                    parents.push(row + 1);
                }
                if row % 13 == 0 && row + 1 < count {
                    parents.push(row + 1 + random as usize % (count - row - 1));
                }
                let head = row % 31 == 0;
                if row % 1024 == 0 {
                    walk = walk.checkpoint().restore();
                    no_paint = no_paint.checkpoint().restore();
                }
                let expected = original.step(row, &parents, parents.len() > 1, head);
                let actual = walk.step(row, &parents, parents.len() > 1, head);
                assert_eq!(actual, expected, "width={width} row={row}");
                let transition = no_paint.transition(row, &parents, parents.len() > 1, head, false);
                assert!(transition.paint.lanes_now.is_empty());
                assert!(transition.paint.lanes_next.is_empty());
                assert_eq!(transition.paint.joins_in, expected.joins_in);
                assert_eq!(transition.paint.edges_out, expected.edges_out);
                assert_eq!(transition.paint.node_col, expected.node_col);
                assert_eq!(transition.paint.node_color_ix, expected.node_color_ix);
            }
        }
    }
}
