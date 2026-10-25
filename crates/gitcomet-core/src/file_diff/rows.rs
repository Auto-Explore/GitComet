//! Row projection: rows, anchors, masks, and line-to-row maps from a plan.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffRow {
    pub kind: FileDiffRowKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub old: Option<FileDiffLineText>,
    pub new: Option<FileDiffLineText>,
    pub eof_newline: Option<FileDiffEofNewline>,
}

/// Stable anchor metadata for a rendered side-by-side diff row.
///
/// `region_id`/`ordinal_in_region` are only populated for non-context rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileDiffRowAnchor {
    pub row_index: usize,
    pub region_id: Option<u32>,
    pub ordinal_in_region: Option<u32>,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

/// Stable anchor metadata for one contiguous changed region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileDiffRegionAnchor {
    pub region_id: u32,
    pub row_start: usize,
    pub row_end_exclusive: usize,
    pub old_start_line: Option<u32>,
    pub old_end_line: Option<u32>,
    pub new_start_line: Option<u32>,
    pub new_end_line: Option<u32>,
}

/// Anchors for all rows and change regions in a side-by-side diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffAnchors {
    pub row_anchors: Vec<FileDiffRowAnchor>,
    pub region_anchors: Vec<FileDiffRegionAnchor>,
}

/// Side-by-side diff rows along with stable row/region anchors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffRowsWithAnchors {
    pub rows: Vec<FileDiffRow>,
    pub anchors: FileDiffAnchors,
}

pub fn side_by_side_rows(old: &str, new: &str) -> Vec<FileDiffRow> {
    let old_text: Arc<str> = Arc::from(old);
    let new_text: Arc<str> = Arc::from(new);
    let old_lines = split_lines(old_text.as_ref());
    let new_lines = split_lines(new_text.as_ref());
    let plan = build_side_by_side_plan_with_pair_cost(
        old_text.as_ref(),
        new_text.as_ref(),
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );
    materialize_rows_from_plan(
        &plan,
        &old_text,
        old_lines.as_slice(),
        &new_text,
        new_lines.as_slice(),
    )
}

pub fn append_side_by_side_rows_with_offsets(
    rows: &mut Vec<FileDiffRow>,
    old: &str,
    new: &str,
    old_line_offset: u32,
    new_line_offset: u32,
) {
    let old_text: Arc<str> = Arc::from(old);
    let new_text: Arc<str> = Arc::from(new);
    let old_lines = split_lines(old_text.as_ref());
    let new_lines = split_lines(new_text.as_ref());
    let plan = build_side_by_side_plan_with_pair_cost(
        old_text.as_ref(),
        new_text.as_ref(),
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );
    materialize_rows_from_plan_into(
        rows,
        &plan,
        &old_text,
        old_lines.as_slice(),
        &new_text,
        new_lines.as_slice(),
        old_line_offset,
        new_line_offset,
    )
}

pub fn side_by_side_rows_with_anchors(old: &str, new: &str) -> FileDiffRowsWithAnchors {
    let rows = side_by_side_rows(old, new);
    let anchors = compute_row_region_anchors(&rows);
    FileDiffRowsWithAnchors { rows, anchors }
}

pub fn plan_row_region_anchors(plan: &FileDiffPlan) -> FileDiffAnchors {
    let mut builder = FileDiffAnchorBuilder::with_capacity(plan.row_count);
    for_each_plan_row_meta(plan, |row_index, row| builder.push(row_index, row));
    builder.finish(plan.row_count)
}

pub fn plan_emitted_line_prefix_counts(plan: &FileDiffPlan) -> (Vec<usize>, Vec<usize>) {
    let mut old_prefix = Vec::with_capacity(plan.row_count.saturating_add(1));
    let mut new_prefix = Vec::with_capacity(plan.row_count.saturating_add(1));
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    old_prefix.push(0);
    new_prefix.push(0);

    for_each_plan_row_meta(plan, |_row_index, row| {
        if row.old_line.is_some() {
            old_count = old_count.saturating_add(1);
        }
        if row.new_line.is_some() {
            new_count = new_count.saturating_add(1);
        }
        old_prefix.push(old_count);
        new_prefix.push(new_count);
    });

    (old_prefix, new_prefix)
}

pub fn plan_changed_line_masks(
    plan: &FileDiffPlan,
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<bool>, Vec<bool>) {
    let mut old_mask = vec![false; old_line_count];
    let mut new_mask = vec![false; new_line_count];

    for_each_plan_row_meta(plan, |_row_index, row| match row.kind {
        FileDiffRowKind::Context => {}
        FileDiffRowKind::Remove => mark_changed_line(old_mask.as_mut_slice(), row.old_line),
        FileDiffRowKind::Add => mark_changed_line(new_mask.as_mut_slice(), row.new_line),
        FileDiffRowKind::Modify => {
            mark_changed_line(old_mask.as_mut_slice(), row.old_line);
            mark_changed_line(new_mask.as_mut_slice(), row.new_line);
        }
    });

    (old_mask, new_mask)
}

pub fn plan_line_to_row_maps(
    plan: &FileDiffPlan,
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut old_line_to_row = vec![None; old_line_count];
    let mut new_line_to_row = vec![None; new_line_count];

    for_each_plan_row_meta(plan, |row_index, row| {
        assign_line_to_row(old_line_to_row.as_mut_slice(), row.old_line, row_index);
        assign_line_to_row(new_line_to_row.as_mut_slice(), row.new_line, row_index);
    });

    (old_line_to_row, new_line_to_row)
}

/// A borrowed view of a single side-by-side diff row.
///
/// This is the zero-allocation equivalent of iterating `side_by_side_rows()`:
/// text references point directly into the source line slices instead of being
/// cloned into owned `String`s.
#[derive(Clone, Copy, Debug)]
pub enum PlanRowView<'a> {
    Context {
        old_line: u32,
        new_line: u32,
        text: &'a str,
    },
    Remove {
        old_line: u32,
        text: &'a str,
    },
    Add {
        new_line: u32,
        text: &'a str,
    },
    Modify {
        old_line: u32,
        new_line: u32,
        old_text: &'a str,
        new_text: &'a str,
    },
}

impl PlanRowView<'_> {
    pub fn kind(&self) -> FileDiffRowKind {
        match self {
            Self::Context { .. } => FileDiffRowKind::Context,
            Self::Remove { .. } => FileDiffRowKind::Remove,
            Self::Add { .. } => FileDiffRowKind::Add,
            Self::Modify { .. } => FileDiffRowKind::Modify,
        }
    }
}

/// Iterate over side-by-side diff rows with borrowed text, avoiding the
/// `Vec<FileDiffRow>` materialization that `side_by_side_rows()` performs.
///
/// Internally computes the diff plan and walks it, yielding `PlanRowView`
/// references into the source texts.
pub fn for_each_side_by_side_row<'a>(
    old: &'a str,
    new: &'a str,
    mut f: impl FnMut(PlanRowView<'a>),
) {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let plan = build_side_by_side_plan_with_pair_cost(
        old,
        new,
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );

    for run in &plan.runs {
        match *run {
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    if let (Some(ol), Some(nl)) =
                        (one_based_line_number(old_ix), one_based_line_number(new_ix))
                    {
                        let text = old_lines.get(old_ix).copied().unwrap_or_default();
                        f(PlanRowView::Context {
                            old_line: ol,
                            new_line: nl,
                            text,
                        });
                    }
                }
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    if let Some(ol) = one_based_line_number(old_ix) {
                        let text = old_lines.get(old_ix).copied().unwrap_or_default();
                        f(PlanRowView::Remove { old_line: ol, text });
                    }
                }
            }
            FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..len {
                    let new_ix = new_start.saturating_add(offset);
                    if let Some(nl) = one_based_line_number(new_ix) {
                        let text = new_lines.get(new_ix).copied().unwrap_or_default();
                        f(PlanRowView::Add { new_line: nl, text });
                    }
                }
            }
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    if let (Some(ol), Some(nl)) =
                        (one_based_line_number(old_ix), one_based_line_number(new_ix))
                    {
                        let old_text = old_lines.get(old_ix).copied().unwrap_or_default();
                        let new_text = new_lines.get(new_ix).copied().unwrap_or_default();
                        f(PlanRowView::Modify {
                            old_line: ol,
                            new_line: nl,
                            old_text,
                            new_text,
                        });
                    }
                }
            }
        }
    }
}

pub(crate) fn compute_row_region_anchors(rows: &[FileDiffRow]) -> FileDiffAnchors {
    let mut builder = FileDiffAnchorBuilder::with_capacity(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        builder.push(row_index, row.into());
    }
    builder.finish(rows.len())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiffRowMeta {
    pub(super) kind: FileDiffRowKind,
    pub(super) old_line: Option<u32>,
    pub(super) new_line: Option<u32>,
}

impl From<&FileDiffRow> for DiffRowMeta {
    fn from(row: &FileDiffRow) -> Self {
        Self {
            kind: row.kind,
            old_line: row.old_line,
            new_line: row.new_line,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ActiveRegion {
    pub(super) region_id: u32,
    pub(super) row_start: usize,
    pub(super) old_start_line: Option<u32>,
    pub(super) old_end_line: Option<u32>,
    pub(super) new_start_line: Option<u32>,
    pub(super) new_end_line: Option<u32>,
    pub(super) next_ordinal: u32,
}

impl ActiveRegion {
    pub(super) fn new(region_id: u32, row_start: usize) -> Self {
        Self {
            region_id,
            row_start,
            old_start_line: None,
            old_end_line: None,
            new_start_line: None,
            new_end_line: None,
            next_ordinal: 0,
        }
    }

    pub(super) fn update_lines(&mut self, row: DiffRowMeta) {
        if let Some(old_line) = row.old_line {
            self.old_start_line = Some(
                self.old_start_line
                    .map_or(old_line, |line| line.min(old_line)),
            );
            self.old_end_line = Some(
                self.old_end_line
                    .map_or(old_line, |line| line.max(old_line)),
            );
        }
        if let Some(new_line) = row.new_line {
            self.new_start_line = Some(
                self.new_start_line
                    .map_or(new_line, |line| line.min(new_line)),
            );
            self.new_end_line = Some(
                self.new_end_line
                    .map_or(new_line, |line| line.max(new_line)),
            );
        }
    }

    pub(super) fn as_region_anchor(self, row_end_exclusive: usize) -> FileDiffRegionAnchor {
        FileDiffRegionAnchor {
            region_id: self.region_id,
            row_start: self.row_start,
            row_end_exclusive,
            old_start_line: self.old_start_line,
            old_end_line: self.old_end_line,
            new_start_line: self.new_start_line,
            new_end_line: self.new_end_line,
        }
    }
}

pub(super) struct FileDiffAnchorBuilder {
    pub(super) row_anchors: Vec<FileDiffRowAnchor>,
    pub(super) region_anchors: Vec<FileDiffRegionAnchor>,
    pub(super) active_region: Option<ActiveRegion>,
}

impl FileDiffAnchorBuilder {
    pub(super) fn with_capacity(row_count_hint: usize) -> Self {
        Self {
            row_anchors: Vec::with_capacity(row_count_hint),
            region_anchors: Vec::new(),
            active_region: None,
        }
    }

    pub(super) fn push(&mut self, row_index: usize, row: DiffRowMeta) {
        if row.kind == FileDiffRowKind::Context {
            if let Some(region) = self.active_region.take() {
                self.region_anchors.push(region.as_region_anchor(row_index));
            }
            self.row_anchors.push(FileDiffRowAnchor {
                row_index,
                region_id: None,
                ordinal_in_region: None,
                old_line: row.old_line,
                new_line: row.new_line,
            });
            return;
        }

        let region = self.active_region.get_or_insert_with(|| {
            let region_id = self.region_anchors.len() as u32;
            ActiveRegion::new(region_id, row_index)
        });
        region.update_lines(row);
        let ordinal_in_region = region.next_ordinal;
        region.next_ordinal = region.next_ordinal.saturating_add(1);

        self.row_anchors.push(FileDiffRowAnchor {
            row_index,
            region_id: Some(region.region_id),
            ordinal_in_region: Some(ordinal_in_region),
            old_line: row.old_line,
            new_line: row.new_line,
        });
    }

    pub(super) fn finish(mut self, row_count: usize) -> FileDiffAnchors {
        if let Some(region) = self.active_region.take() {
            self.region_anchors.push(region.as_region_anchor(row_count));
        }
        FileDiffAnchors {
            row_anchors: self.row_anchors,
            region_anchors: self.region_anchors,
        }
    }
}

pub(super) fn one_based_line_number(line_ix: usize) -> Option<u32> {
    line_ix
        .checked_add(1)
        .and_then(|line| u32::try_from(line).ok())
}

pub(super) fn mark_changed_line(mask: &mut [bool], line: Option<u32>) {
    let Some(line) = line else {
        return;
    };
    let line_ix = line.saturating_sub(1) as usize;
    if let Some(slot) = mask.get_mut(line_ix) {
        *slot = true;
    }
}

pub(super) fn assign_line_to_row(
    line_to_row: &mut [Option<usize>],
    line: Option<u32>,
    row_index: usize,
) {
    let Some(line) = line else {
        return;
    };
    let line_ix = line.saturating_sub(1) as usize;
    if let Some(slot) = line_to_row.get_mut(line_ix) {
        *slot = Some(row_index);
    }
}

pub(super) fn for_each_plan_row_meta(plan: &FileDiffPlan, mut f: impl FnMut(usize, DiffRowMeta)) {
    let mut row_index = 0usize;

    for run in &plan.runs {
        match *run {
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Context,
                            old_line: one_based_line_number(old_ix),
                            new_line: one_based_line_number(new_ix),
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Remove,
                            old_line: one_based_line_number(old_ix),
                            new_line: None,
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
            FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..len {
                    let new_ix = new_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Add,
                            old_line: None,
                            new_line: one_based_line_number(new_ix),
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Modify,
                            old_line: one_based_line_number(old_ix),
                            new_line: one_based_line_number(new_ix),
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
        }
    }

    debug_assert_eq!(row_index, plan.row_count);
}

pub(super) fn materialize_rows_from_plan(
    plan: &FileDiffPlan,
    old_text: &Arc<str>,
    old_lines: &[&str],
    new_text: &Arc<str>,
    new_lines: &[&str],
) -> Vec<FileDiffRow> {
    let mut rows = Vec::with_capacity(plan.row_count);
    materialize_rows_from_plan_into(
        &mut rows, plan, old_text, old_lines, new_text, new_lines, 0, 0,
    );
    rows
}

pub(super) fn shared_line_text(text: &Arc<str>, line: &str) -> FileDiffLineText {
    let base_ptr = text.as_ptr() as usize;
    let line_ptr = line.as_ptr() as usize;
    let start = line_ptr.saturating_sub(base_ptr);
    let end = start.saturating_add(line.len());
    debug_assert_eq!(text.get(start..end), Some(line));
    FileDiffLineText::shared_slice(Arc::clone(text), start..end)
}

pub(super) fn materialize_rows_from_plan_into(
    rows: &mut Vec<FileDiffRow>,
    plan: &FileDiffPlan,
    old_text: &Arc<str>,
    old_lines: &[&str],
    new_text: &Arc<str>,
    new_lines: &[&str],
    old_line_offset: u32,
    new_line_offset: u32,
) {
    let old_line_delta = old_line_offset.saturating_sub(1);
    let new_line_delta = new_line_offset.saturating_sub(1);
    rows.reserve(plan.row_count);
    let row_start = rows.len();

    for run in &plan.runs {
        match run {
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..*len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    let text = shared_line_text(
                        old_text,
                        old_lines.get(old_ix).copied().unwrap_or_default(),
                    );
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Context,
                        old_line: one_based_line_number(old_ix)
                            .map(|line| line.saturating_add(old_line_delta)),
                        new_line: one_based_line_number(new_ix)
                            .map(|line| line.saturating_add(new_line_delta)),
                        old: Some(text.clone()),
                        new: Some(text),
                        eof_newline: None,
                    });
                }
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..*len {
                    let old_ix = old_start.saturating_add(offset);
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Remove,
                        old_line: one_based_line_number(old_ix)
                            .map(|line| line.saturating_add(old_line_delta)),
                        new_line: None,
                        old: Some(shared_line_text(
                            old_text,
                            old_lines.get(old_ix).copied().unwrap_or_default(),
                        )),
                        new: None,
                        eof_newline: None,
                    });
                }
            }
            FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..*len {
                    let new_ix = new_start.saturating_add(offset);
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Add,
                        old_line: None,
                        new_line: one_based_line_number(new_ix)
                            .map(|line| line.saturating_add(new_line_delta)),
                        old: None,
                        new: Some(shared_line_text(
                            new_text,
                            new_lines.get(new_ix).copied().unwrap_or_default(),
                        )),
                        eof_newline: None,
                    });
                }
            }
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..*len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Modify,
                        old_line: one_based_line_number(old_ix)
                            .map(|line| line.saturating_add(old_line_delta)),
                        new_line: one_based_line_number(new_ix)
                            .map(|line| line.saturating_add(new_line_delta)),
                        old: Some(shared_line_text(
                            old_text,
                            old_lines.get(old_ix).copied().unwrap_or_default(),
                        )),
                        new: Some(shared_line_text(
                            new_text,
                            new_lines.get(new_ix).copied().unwrap_or_default(),
                        )),
                        eof_newline: None,
                    });
                }
            }
        }
    }

    if let Some(marker) = plan.eof_newline {
        if let Some(last) = rows[row_start..].last_mut() {
            last.eof_newline = Some(marker);
        } else {
            rows.push(FileDiffRow {
                kind: FileDiffRowKind::Modify,
                old_line: None,
                new_line: None,
                old: None,
                new: None,
                eof_newline: Some(marker),
            });
        }
    }
}
