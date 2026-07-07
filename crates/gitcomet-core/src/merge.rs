//! Three-way file merge algorithm.
//!
//! Takes base, local (ours), and remote (theirs) file contents and produces
//! merged output, potentially with conflict markers where the two sides
//! changed the same region differently.
//!
//! Compatible with `git merge-file` marker format.

use crate::file_diff::{
    DiffHunk, Edit, edits_to_hunks_with, histogram_edits, myers_edits, reconstruct_side_with,
    split_lines,
};
use std::borrow::Cow;
use std::fmt;

/// Default conflict marker width (matches git's default).
pub const DEFAULT_MARKER_SIZE: usize = 7;

/// How to render the base content in conflict markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ConflictStyle {
    /// Two-section markers: `<<<<<<<` / `=======` / `>>>>>>>`.
    #[default]
    Merge,
    /// Three-section markers showing ancestor: `<<<<<<<` / `|||||||` / `=======` / `>>>>>>>`.
    Diff3,
    /// Like diff3 but strips common prefix/suffix lines from conflict blocks.
    Zdiff3,
}

/// How to automatically resolve conflicts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum MergeStrategy {
    /// Leave conflict markers in output.
    #[default]
    Normal,
    /// Auto-resolve conflicts by picking ours (local).
    Ours,
    /// Auto-resolve conflicts by picking theirs (remote).
    Theirs,
    /// Auto-resolve conflicts by including both sides (ours then theirs).
    Union,
}

/// Which diff algorithm to use for computing edit scripts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum DiffAlgorithm {
    /// Classic Myers O(ND) algorithm. Fast and minimal edit distance.
    #[default]
    Myers,
    /// Patience/histogram algorithm. Anchors on unique lines to produce
    /// semantically cleaner diffs, especially for code with repetitive
    /// structural tokens (braces, returns). Falls back to Myers for
    /// regions with no unique lines.
    Histogram,
}

/// Labels for the three merge sides.
#[derive(Clone, Debug, Default)]
pub struct MergeLabels {
    pub ours: Option<String>,
    pub base: Option<String>,
    pub theirs: Option<String>,
}

/// Options controlling merge behavior.
#[derive(Clone, Debug)]
pub struct MergeOptions {
    pub style: ConflictStyle,
    pub strategy: MergeStrategy,
    pub labels: MergeLabels,
    pub marker_size: usize,
    pub diff_algorithm: DiffAlgorithm,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            style: ConflictStyle::default(),
            strategy: MergeStrategy::default(),
            labels: MergeLabels::default(),
            marker_size: DEFAULT_MARKER_SIZE,
            diff_algorithm: DiffAlgorithm::default(),
        }
    }
}

/// Result of a three-way merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeResult {
    /// The merged output text.
    pub output: String,
    /// Number of conflict regions (0 = clean merge).
    pub conflict_count: usize,
}

impl MergeResult {
    /// Returns `true` if the merge completed without conflicts.
    pub fn is_clean(&self) -> bool {
        self.conflict_count == 0
    }
}

/// Error from a three-way merge operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeError {
    /// One or more inputs contain binary content (null bytes or non-UTF-8).
    BinaryContent,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::BinaryContent => write!(f, "cannot merge binary files"),
        }
    }
}

impl std::error::Error for MergeError {}

/// Perform a three-way merge on raw byte inputs with binary detection.
///
/// Returns `Err(MergeError::BinaryContent)` if any input contains null bytes
/// or is not valid UTF-8. Otherwise delegates to [`merge_file`].
pub fn merge_file_bytes(
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
    options: &MergeOptions,
) -> Result<MergeResult, MergeError> {
    fn check_binary(data: &[u8]) -> Result<&str, MergeError> {
        if data.contains(&0) {
            return Err(MergeError::BinaryContent);
        }
        std::str::from_utf8(data).map_err(|_| MergeError::BinaryContent)
    }

    let base_str = check_binary(base)?;
    let ours_str = check_binary(ours)?;
    let theirs_str = check_binary(theirs)?;

    Ok(merge_file(base_str, ours_str, theirs_str, options))
}

/// Perform a three-way merge of text files.
///
/// Diffs `base` against both `ours` and `theirs`, then walks the two edit
/// scripts to produce a merged result. Where both sides changed the same
/// base region differently, a conflict is emitted (or auto-resolved per
/// the chosen strategy).
pub fn merge_file(base: &str, ours: &str, theirs: &str, options: &MergeOptions) -> MergeResult {
    let base_lines = split_lines(base);
    let ours_lines = split_lines(ours);
    let theirs_lines = split_lines(theirs);

    let diff_fn = match options.diff_algorithm {
        DiffAlgorithm::Myers => myers_edits,
        DiffAlgorithm::Histogram => histogram_edits,
    };
    let edits_ours = diff_fn(&base_lines, &ours_lines);
    let edits_theirs = diff_fn(&base_lines, &theirs_lines);

    let hunks_ours = edits_to_hunks(&edits_ours);
    let hunks_theirs = edits_to_hunks(&edits_theirs);

    let merged_hunks = merge_hunks(&base_lines, &hunks_ours, &hunks_theirs);
    let merged_hunks = coalesce_zealous_conflicts(&base_lines, merged_hunks);
    render_merged(&base_lines, &merged_hunks, base, ours, theirs, options)
}

/// How the sides relate within one aligned run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignedRunKind {
    /// No side changed this region.
    Unchanged,
    /// Only ours changed from base.
    OursChanged,
    /// Only theirs changed from base.
    TheirsChanged,
    /// Both sides changed and agree (identical result).
    BothSame,
    /// Both sides changed differently.
    Conflict,
}

/// One run of a kdiff3-style three-way alignment.
///
/// The three ranges are line ranges into base/ours/theirs respectively; a
/// run renders as `max(len)` visual rows with shorter sides padded, so
/// corresponding lines always sit at the same visual height.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignedRun {
    pub base: std::ops::Range<usize>,
    pub ours: std::ops::Range<usize>,
    pub theirs: std::ops::Range<usize>,
    pub kind: AlignedRunKind,
}

impl AlignedRun {
    /// Number of visual rows this run occupies in the aligned space.
    pub fn visual_rows(&self) -> usize {
        self.base.len().max(self.ours.len()).max(self.theirs.len())
    }
}

/// Compute the three-way alignment of base/ours/theirs (§30 aligned row
/// space). This walks the same base-anchored regions the merge algorithm
/// uses to produce conflict markers, but reports per-side line ranges
/// instead of merged content. The runs partition each input exactly.
pub fn align_three_way(
    base: &str,
    ours: &str,
    theirs: &str,
    algorithm: DiffAlgorithm,
) -> Vec<AlignedRun> {
    let base_lines = split_lines(base);
    let ours_lines = split_lines(ours);
    let theirs_lines = split_lines(theirs);

    let diff_fn = match algorithm {
        DiffAlgorithm::Myers => myers_edits,
        DiffAlgorithm::Histogram => histogram_edits,
    };
    let edits_ours = diff_fn(&base_lines, &ours_lines);
    let edits_theirs = diff_fn(&base_lines, &theirs_lines);
    let hunks_ours = edits_to_hunks(&edits_ours);
    let hunks_theirs = edits_to_hunks(&edits_theirs);

    let mut runs = Vec::new();
    let mut base_pos = 0usize;
    let mut ours_pos = 0usize;
    let mut theirs_pos = 0usize;

    let push_unchanged = |runs: &mut Vec<AlignedRun>,
                          base_pos: &mut usize,
                          ours_pos: &mut usize,
                          theirs_pos: &mut usize,
                          until: usize| {
        let len = until.saturating_sub(*base_pos);
        if len == 0 {
            return;
        }
        runs.push(AlignedRun {
            base: *base_pos..*base_pos + len,
            ours: *ours_pos..*ours_pos + len,
            theirs: *theirs_pos..*theirs_pos + len,
            kind: AlignedRunKind::Unchanged,
        });
        *base_pos += len;
        *ours_pos += len;
        *theirs_pos += len;
    };

    for_each_merge_region(&base_lines, &hunks_ours, &hunks_theirs, |region| {
        push_unchanged(
            &mut runs,
            &mut base_pos,
            &mut ours_pos,
            &mut theirs_pos,
            region.base_start,
        );

        let ours_shape = side_region_shape(
            region.base_start,
            region.base_end,
            region.ours_hunks,
            ours_pos,
        );
        let theirs_shape = side_region_shape(
            region.base_start,
            region.base_end,
            region.theirs_hunks,
            theirs_pos,
        );
        let (ours_end, theirs_end) = emit_region_rows(
            &mut runs,
            &ours_lines,
            &theirs_lines,
            region.base_start,
            region.base_end,
            &ours_shape,
            &theirs_shape,
            ours_pos,
            theirs_pos,
        );
        base_pos = region.base_end;
        ours_pos = ours_end;
        theirs_pos = theirs_end;
    });

    push_unchanged(
        &mut runs,
        &mut base_pos,
        &mut ours_pos,
        &mut theirs_pos,
        base_lines.len(),
    );

    debug_assert_eq!(ours_pos, ours_lines.len());
    debug_assert_eq!(theirs_pos, theirs_lines.len());
    runs
}

/// A base-anchored change region visited by [`for_each_merge_region`].
struct MergeRegion<'h, 'a> {
    base_start: usize,
    base_end: usize,
    ours_hunks: &'h [Hunk<'a>],
    theirs_hunks: &'h [Hunk<'a>],
}

/// Walk the overlapping-hunk regions of the two edit scripts, exactly as
/// [`merge_hunks`] does, invoking `visit` per region.
fn for_each_merge_region<'a>(
    base_lines: &'a [&'a str],
    ours: &[Hunk<'a>],
    theirs: &[Hunk<'a>],
    mut visit: impl FnMut(MergeRegion<'_, 'a>),
) {
    let _ = base_lines;
    let mut oi = 0;
    let mut ti = 0;

    loop {
        let oh_start = ours.get(oi).map(|h| h.base_start).unwrap_or(usize::MAX);
        let th_start = theirs.get(ti).map(|h| h.base_start).unwrap_or(usize::MAX);
        if oh_start == usize::MAX && th_start == usize::MAX {
            break;
        }

        let change_start = oh_start.min(th_start);
        let mut region_end = change_start;
        let oi_start = oi;
        let ti_start = ti;

        loop {
            let mut extended = false;
            while let Some(oh) = ours.get(oi) {
                if oh.base_start <= region_end {
                    region_end = region_end.max(oh.base_end);
                    oi += 1;
                    extended = true;
                } else {
                    break;
                }
            }
            while let Some(th) = theirs.get(ti) {
                if th.base_start <= region_end {
                    region_end = region_end.max(th.base_end);
                    ti += 1;
                    extended = true;
                } else {
                    break;
                }
            }
            if !extended {
                break;
            }
        }

        visit(MergeRegion {
            base_start: change_start,
            base_end: region_end,
            ours_hunks: &ours[oi_start..oi],
            theirs_hunks: &theirs[ti_start..ti],
        });
    }
}

/// Per-side shape of a change region: which base lines survive on the side
/// (with their side line index), and the side's free (inserted/replacement)
/// line segments keyed by the base boundary they precede.
struct SideRegionShape {
    /// For each region-relative base index: `Some(side_line)` when the base
    /// line survives on this side.
    kept: Vec<Option<usize>>,
    /// For each boundary position `0..=len`: `(side_line_start, count)` of
    /// side lines inserted before that base line (or at the region end).
    free_at: Vec<(usize, usize)>,
}

fn side_region_shape(
    region_start: usize,
    region_end: usize,
    hunks: &[Hunk<'_>],
    side_start: usize,
) -> SideRegionShape {
    let len = region_end - region_start;
    let mut kept = vec![None; len];
    let mut free_at = vec![(0usize, 0usize); len + 1];
    let mut side_line = side_start;
    let mut b = region_start;

    for hunk in hunks {
        let hunk_start = hunk.base_start.clamp(region_start, region_end);
        while b < hunk_start {
            kept[b - region_start] = Some(side_line);
            side_line += 1;
            b += 1;
        }
        let slot = &mut free_at[b - region_start];
        if slot.1 == 0 {
            slot.0 = side_line;
        }
        slot.1 += hunk.new_lines.len();
        side_line += hunk.new_lines.len();
        b = hunk.base_end.clamp(b, region_end);
    }
    while b < region_end {
        kept[b - region_start] = Some(side_line);
        side_line += 1;
        b += 1;
    }

    SideRegionShape { kept, free_at }
}

/// Emit kdiff3-style aligned rows for one change region.
///
/// Base lines anchor to their surviving copies: a side that dropped the base
/// line contributes its next replacement line on that row instead of padding.
/// Free lines from both sides pair up top-aligned; a side's free lines always
/// flush before its own anchored copy so side ranges stay contiguous.
/// Returns the per-side cursor positions after the region.
#[expect(clippy::too_many_arguments)]
fn emit_region_rows(
    runs: &mut Vec<AlignedRun>,
    ours_lines: &[&str],
    theirs_lines: &[&str],
    region_start: usize,
    region_end: usize,
    ours_shape: &SideRegionShape,
    theirs_shape: &SideRegionShape,
    ours_start: usize,
    theirs_start: usize,
) -> (usize, usize) {
    // Pending free lines per side; contiguous by construction (a side's free
    // segments are only ever separated by its own anchored copies, which
    // force a flush first).
    let mut pend_o = (ours_start, 0usize);
    let mut pend_t = (theirs_start, 0usize);

    fn extend(pend: &mut (usize, usize), seg: (usize, usize)) {
        if seg.1 == 0 {
            return;
        }
        if pend.1 == 0 {
            pend.0 = seg.0;
        }
        pend.1 += seg.1;
    }
    fn take(pend: &mut (usize, usize), n: usize) -> std::ops::Range<usize> {
        let n = n.min(pend.1);
        let range = pend.0..pend.0 + n;
        pend.0 += n;
        pend.1 -= n;
        range
    }

    let mut o_cur = ours_start;
    let mut t_cur = theirs_start;
    let push_row = |runs: &mut Vec<AlignedRun>,
                    o_cur: &mut usize,
                    t_cur: &mut usize,
                    base: std::ops::Range<usize>,
                    ours: std::ops::Range<usize>,
                    theirs: std::ops::Range<usize>,
                    kind: AlignedRunKind| {
        debug_assert_eq!(ours.start.max(*o_cur), *o_cur);
        *o_cur = ours.end.max(*o_cur);
        *t_cur = theirs.end.max(*t_cur);
        runs.push(AlignedRun {
            base,
            ours,
            theirs,
            kind,
        });
    };

    let zip_pending = |runs: &mut Vec<AlignedRun>,
                       pend_o: &mut (usize, usize),
                       pend_t: &mut (usize, usize),
                       o_cur: &mut usize,
                       t_cur: &mut usize,
                       base_at: usize| {
        let n = pend_o.1.min(pend_t.1);
        if n == 0 {
            return;
        }
        let ours = take(pend_o, n);
        let theirs = take(pend_t, n);
        let kind = if ours_lines.get(ours.clone()) == theirs_lines.get(theirs.clone())
            && ours_lines.get(ours.clone()).is_some()
        {
            AlignedRunKind::BothSame
        } else {
            AlignedRunKind::Conflict
        };
        push_row(runs, o_cur, t_cur, base_at..base_at, ours, theirs, kind);
    };
    let flush_solo = |runs: &mut Vec<AlignedRun>,
                      pend: &mut (usize, usize),
                      o_cur: &mut usize,
                      t_cur: &mut usize,
                      base_at: usize,
                      is_ours: bool| {
        if pend.1 == 0 {
            return;
        }
        let range = take(pend, pend.1);
        let (ours, theirs, kind) = if is_ours {
            (range, *t_cur..*t_cur, AlignedRunKind::OursChanged)
        } else {
            (*o_cur..*o_cur, range, AlignedRunKind::TheirsChanged)
        };
        push_row(runs, o_cur, t_cur, base_at..base_at, ours, theirs, kind);
    };

    let len = region_end - region_start;
    for p in 0..len {
        extend(&mut pend_o, ours_shape.free_at[p]);
        extend(&mut pend_t, theirs_shape.free_at[p]);
        let b = region_start + p;
        let kept_o = ours_shape.kept[p];
        let kept_t = theirs_shape.kept[p];

        if kept_o.is_none() && kept_t.is_none() {
            // The base line was dropped by both sides: it anchors to the
            // first replacement line of each (kdiff3's modified-line row),
            // with the remaining replacements pairing up below it.
            let ours = take(&mut pend_o, 1);
            let theirs = take(&mut pend_t, 1);
            let kind = if !ours.is_empty()
                && ours_lines.get(ours.clone()) == theirs_lines.get(theirs.clone())
            {
                AlignedRunKind::BothSame
            } else {
                AlignedRunKind::Conflict
            };
            push_row(runs, &mut o_cur, &mut t_cur, b..b + 1, ours, theirs, kind);
            zip_pending(
                runs,
                &mut pend_o,
                &mut pend_t,
                &mut o_cur,
                &mut t_cur,
                b + 1,
            );
            continue;
        }

        zip_pending(runs, &mut pend_o, &mut pend_t, &mut o_cur, &mut t_cur, b);
        // A side's free lines precede its anchored copy in side order, so
        // they must be on rows above the anchor row.
        if kept_o.is_some() {
            flush_solo(runs, &mut pend_o, &mut o_cur, &mut t_cur, b, true);
        }
        if kept_t.is_some() {
            flush_solo(runs, &mut pend_t, &mut o_cur, &mut t_cur, b, false);
        }

        let ours = match kept_o {
            Some(line) => line..line + 1,
            None => take(&mut pend_o, 1),
        };
        let theirs = match kept_t {
            Some(line) => line..line + 1,
            None => take(&mut pend_t, 1),
        };
        let kind = match (kept_o.is_some(), kept_t.is_some()) {
            (true, true) => AlignedRunKind::Unchanged,
            (true, false) => AlignedRunKind::TheirsChanged,
            (false, true) => AlignedRunKind::OursChanged,
            (false, false) => unreachable!("handled above"),
        };
        push_row(runs, &mut o_cur, &mut t_cur, b..b + 1, ours, theirs, kind);
    }

    // Region-end boundary: remaining free lines pair up, overflow flushes.
    extend(&mut pend_o, ours_shape.free_at[len]);
    extend(&mut pend_t, theirs_shape.free_at[len]);
    zip_pending(
        runs,
        &mut pend_o,
        &mut pend_t,
        &mut o_cur,
        &mut t_cur,
        region_end,
    );
    flush_solo(runs, &mut pend_o, &mut o_cur, &mut t_cur, region_end, true);
    flush_solo(runs, &mut pend_t, &mut o_cur, &mut t_cur, region_end, false);

    (o_cur, t_cur)
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A contiguous change from one side's diff against the base.
type Hunk<'a> = DiffHunk<Cow<'a, str>>;

/// A merged hunk — either cleanly resolved or a conflict.
#[derive(Clone, Debug)]
enum MergedHunk<'a> {
    /// Resolved: output these lines.
    Resolved {
        base_start: usize,
        base_end: usize,
        lines: Vec<Cow<'a, str>>,
    },
    /// Conflict: both sides changed the same base region differently.
    Conflict {
        base_start: usize,
        base_end: usize,
        ours_lines: Vec<Cow<'a, str>>,
        theirs_lines: Vec<Cow<'a, str>>,
    },
}

impl MergedHunk<'_> {
    fn base_start(&self) -> usize {
        match self {
            MergedHunk::Resolved { base_start, .. } => *base_start,
            MergedHunk::Conflict { base_start, .. } => *base_start,
        }
    }

    fn base_end(&self) -> usize {
        match self {
            MergedHunk::Resolved { base_end, .. } => *base_end,
            MergedHunk::Conflict { base_end, .. } => *base_end,
        }
    }
}

// ---------------------------------------------------------------------------
// Diff → Hunk conversion
// ---------------------------------------------------------------------------

fn edits_to_hunks<'a>(edits: &[Edit<'a>]) -> Vec<Hunk<'a>> {
    edits_to_hunks_with(edits, Cow::Borrowed)
}

// ---------------------------------------------------------------------------
// Hunk merging
// ---------------------------------------------------------------------------

/// Merge two hunk lists into a sequence of resolved/conflict hunks.
fn merge_hunks<'a>(
    base_lines: &'a [&'a str],
    ours: &[Hunk<'a>],
    theirs: &[Hunk<'a>],
) -> Vec<MergedHunk<'a>> {
    let mut result = Vec::new();
    let mut oi = 0;
    let mut ti = 0;

    loop {
        let oh_start = ours.get(oi).map(|h| h.base_start).unwrap_or(usize::MAX);
        let th_start = theirs.get(ti).map(|h| h.base_start).unwrap_or(usize::MAX);

        if oh_start == usize::MAX && th_start == usize::MAX {
            break;
        }

        // Determine the start of the next change region.
        let change_start = oh_start.min(th_start);

        // Expand the region to include all overlapping hunks from both sides.
        let mut region_end = change_start;
        let oi_start = oi;
        let ti_start = ti;

        // Consume initial hunks at change_start.
        while let Some(oh) = ours.get(oi) {
            if oh.base_start <= region_end {
                region_end = region_end.max(oh.base_end);
                oi += 1;
            } else {
                break;
            }
        }
        while let Some(th) = theirs.get(ti) {
            if th.base_start <= region_end {
                region_end = region_end.max(th.base_end);
                ti += 1;
            } else {
                break;
            }
        }

        // Keep expanding while hunks overlap.
        loop {
            let mut extended = false;
            while let Some(oh) = ours.get(oi) {
                if oh.base_start <= region_end {
                    region_end = region_end.max(oh.base_end);
                    oi += 1;
                    extended = true;
                } else {
                    break;
                }
            }
            while let Some(th) = theirs.get(ti) {
                if th.base_start <= region_end {
                    region_end = region_end.max(th.base_end);
                    ti += 1;
                    extended = true;
                } else {
                    break;
                }
            }
            if !extended {
                break;
            }
        }

        let ours_involved = oi > oi_start;
        let theirs_involved = ti > ti_start;

        if ours_involved && theirs_involved {
            // Both sides changed the same region.
            let ours_hunks = &ours[oi_start..oi];
            let theirs_hunks = &theirs[ti_start..ti];

            if ours_hunks == theirs_hunks {
                // Identical hunk structure — skip reconstructing theirs entirely.
                let ours_content =
                    reconstruct_side(base_lines, change_start, region_end, ours_hunks);
                result.push(MergedHunk::Resolved {
                    base_start: change_start,
                    base_end: region_end,
                    lines: ours_content,
                });
            } else {
                let ours_content =
                    reconstruct_side(base_lines, change_start, region_end, ours_hunks);
                let theirs_content =
                    reconstruct_side(base_lines, change_start, region_end, theirs_hunks);

                if ours_content == theirs_content {
                    // Different hunks but same result — resolved.
                    result.push(MergedHunk::Resolved {
                        base_start: change_start,
                        base_end: region_end,
                        lines: ours_content,
                    });
                } else {
                    result.push(MergedHunk::Conflict {
                        base_start: change_start,
                        base_end: region_end,
                        ours_lines: ours_content,
                        theirs_lines: theirs_content,
                    });
                }
            }
        } else if ours_involved {
            let content =
                reconstruct_side(base_lines, change_start, region_end, &ours[oi_start..oi]);
            result.push(MergedHunk::Resolved {
                base_start: change_start,
                base_end: region_end,
                lines: content,
            });
        } else if theirs_involved {
            let content =
                reconstruct_side(base_lines, change_start, region_end, &theirs[ti_start..ti]);
            result.push(MergedHunk::Resolved {
                base_start: change_start,
                base_end: region_end,
                lines: content,
            });
        }
    }

    result
}

/// Coalesce consecutive conflict hunks when the unchanged base context between
/// them is adjacent or blank-only. This mirrors git's "zealous" behavior for
/// reducing noisy back-to-back conflict markers.
fn coalesce_zealous_conflicts<'a>(
    base_lines: &'a [&'a str],
    hunks: Vec<MergedHunk<'a>>,
) -> Vec<MergedHunk<'a>> {
    let mut out = Vec::with_capacity(hunks.len());

    for hunk in hunks {
        let mut merged_into_previous = false;

        if let Some(last) = out.last_mut()
            && let (
                MergedHunk::Conflict {
                    base_end: last_base_end,
                    ours_lines: last_ours,
                    theirs_lines: last_theirs,
                    ..
                },
                MergedHunk::Conflict {
                    base_start: next_base_start,
                    base_end: next_base_end,
                    ours_lines: next_ours,
                    theirs_lines: next_theirs,
                    ..
                },
            ) = (last, &hunk)
            && blank_only_or_adjacent_separator(base_lines, *last_base_end, *next_base_start)
        {
            let start = (*last_base_end).min(base_lines.len());
            let end = (*next_base_start).min(base_lines.len());
            for &line in &base_lines[start..end] {
                last_ours.push(Cow::Borrowed(line));
                last_theirs.push(Cow::Borrowed(line));
            }
            last_ours.extend(next_ours.iter().cloned());
            last_theirs.extend(next_theirs.iter().cloned());
            *last_base_end = *next_base_end;
            merged_into_previous = true;
        }

        if !merged_into_previous {
            out.push(hunk);
        }
    }

    out
}

fn blank_only_or_adjacent_separator(base_lines: &[&str], from: usize, to: usize) -> bool {
    if to < from {
        return false;
    }

    let start = from.min(base_lines.len());
    let end = to.min(base_lines.len());
    base_lines[start..end]
        .iter()
        .all(|line| line.trim().is_empty())
}

/// Reconstruct the content of one side for a base line range, applying hunks.
fn reconstruct_side<'a>(
    base_lines: &'a [&'a str],
    range_start: usize,
    range_end: usize,
    hunks: &[Hunk<'a>],
) -> Vec<Cow<'a, str>> {
    let mut lines = Vec::new();
    reconstruct_side_with(
        base_lines,
        range_start..range_end,
        hunks,
        &mut lines,
        Cow::Borrowed,
    );
    lines
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render merged hunks into final output text.
fn render_merged(
    base_lines: &[&str],
    merged_hunks: &[MergedHunk<'_>],
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    options: &MergeOptions,
) -> MergeResult {
    let line_ending = detect_line_ending(ours_text, theirs_text, base_text);
    let mut output = String::new();
    let mut conflict_count = 0;
    let mut base_pos = 0;

    for hunk in merged_hunks {
        // Emit unchanged base lines before this hunk.
        let ctx_end = hunk.base_start().min(base_lines.len());
        emit_context_lines(&mut output, base_lines, base_pos, ctx_end, line_ending);
        base_pos = hunk.base_end();

        match hunk {
            MergedHunk::Resolved { lines, .. } => {
                for line in lines {
                    output.push_str(line.as_ref());
                    output.push_str(line_ending);
                }
            }
            MergedHunk::Conflict {
                base_start,
                base_end,
                ours_lines,
                theirs_lines,
            } => {
                let base_conflict_lines =
                    &base_lines[*base_start..(*base_end).min(base_lines.len())];

                match options.strategy {
                    MergeStrategy::Ours => {
                        for line in ours_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                    }
                    MergeStrategy::Theirs => {
                        for line in theirs_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                    }
                    MergeStrategy::Union => {
                        for line in ours_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                        for line in theirs_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                    }
                    MergeStrategy::Normal => {
                        emit_conflict_markers(
                            &mut output,
                            ours_lines,
                            theirs_lines,
                            base_conflict_lines,
                            options,
                            line_ending,
                        );
                        conflict_count += 1;
                    }
                }
            }
        }
    }

    // Remaining base lines after all hunks.
    emit_context_lines(
        &mut output,
        base_lines,
        base_pos,
        base_lines.len(),
        line_ending,
    );

    apply_trailing_newline_decision(&mut output, base_text, base_lines, ours_text, theirs_text);

    MergeResult {
        output,
        conflict_count,
    }
}

fn emit_context_lines(
    output: &mut String,
    base_lines: &[&str],
    from: usize,
    to: usize,
    line_ending: &str,
) {
    for &line in &base_lines[from..to] {
        output.push_str(line);
        output.push_str(line_ending);
    }
}

/// 3-way merge decision for whether the output should end with a trailing
/// newline. Checks which input(s) contributed the output's last line, then
/// applies merge logic to the trailing-LF "bit".
fn apply_trailing_newline_decision(
    output: &mut String,
    base_text: &str,
    base_lines: &[&str],
    ours_text: &str,
    theirs_text: &str,
) {
    let ours_has_trailing = ours_text.is_empty() || ours_text.ends_with('\n');
    let theirs_has_trailing = theirs_text.is_empty() || theirs_text.ends_with('\n');
    let base_has_trailing = base_text.is_empty() || base_text.ends_with('\n');

    let ours_lines_all = split_lines(ours_text);
    let theirs_lines_all = split_lines(theirs_text);

    let output_last = output
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .rsplit('\n')
        .next()
        .unwrap_or("");

    let ours_last_matches = ours_lines_all.last().is_some_and(|l| *l == output_last);
    let theirs_last_matches = theirs_lines_all.last().is_some_and(|l| *l == output_last);
    let base_last_matches = base_lines.last().is_some_and(|l| *l == output_last);

    // Each branch has distinct semantics even when the result expression
    // happens to be the same (`ours_has_trailing`):
    //   - agree    → both match, pick either
    //   - ours-only→ only ours diverged from base, pick ours
    //   - conflict → both diverged, prefer ours
    #[allow(clippy::if_same_then_else)]
    let want_trailing = if ours_last_matches && theirs_last_matches {
        if ours_has_trailing == theirs_has_trailing {
            ours_has_trailing
        } else if base_last_matches && base_has_trailing == theirs_has_trailing {
            ours_has_trailing // only ours changed
        } else if base_last_matches && base_has_trailing == ours_has_trailing {
            theirs_has_trailing // only theirs changed
        } else {
            ours_has_trailing // both changed; prefer ours
        }
    } else if ours_last_matches {
        ours_has_trailing
    } else if theirs_last_matches {
        theirs_has_trailing
    } else if base_last_matches {
        base_has_trailing
    } else {
        true // conflict marker or union content — keep trailing LF
    };

    if !want_trailing {
        if output.ends_with("\r\n") {
            output.truncate(output.len() - 2);
        } else if output.ends_with('\n') {
            output.truncate(output.len() - 1);
        }
    }
}

fn emit_conflict_markers(
    output: &mut String,
    ours_lines: &[Cow<'_, str>],
    theirs_lines: &[Cow<'_, str>],
    base_lines: &[&str],
    options: &MergeOptions,
    line_ending: &str,
) {
    let ms = options.marker_size;

    match options.style {
        ConflictStyle::Zdiff3 => {
            // Strip common prefix and suffix lines from the conflict.
            let (prefix_len, suffix_len) = common_prefix_suffix_lines(ours_lines, theirs_lines);

            // Emit common prefix as resolved.
            for line in &ours_lines[..prefix_len] {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }

            let ours_end = ours_lines.len().saturating_sub(suffix_len).max(prefix_len);
            let theirs_end = theirs_lines
                .len()
                .saturating_sub(suffix_len)
                .max(prefix_len);
            let ours_conflict = &ours_lines[prefix_len..ours_end];
            let theirs_conflict = &theirs_lines[prefix_len..theirs_end];

            // Emit conflict markers for the remaining inner region.
            emit_marker(output, '<', ms, options.labels.ours.as_deref(), line_ending);
            for line in ours_conflict {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(output, '|', ms, options.labels.base.as_deref(), line_ending);
            // In zdiff3, the base section shows the trimmed base content.
            let base_conflict = if base_lines.len() > prefix_len + suffix_len {
                &base_lines[prefix_len..base_lines.len() - suffix_len]
            } else {
                &[] as &[&str]
            };
            for line in base_conflict {
                output.push_str(line);
                output.push_str(line_ending);
            }
            emit_marker(output, '=', ms, None, line_ending);
            for line in theirs_conflict {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(
                output,
                '>',
                ms,
                options.labels.theirs.as_deref(),
                line_ending,
            );

            // Emit common suffix as resolved.
            for line in &ours_lines[ours_lines.len() - suffix_len..] {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
        }
        ConflictStyle::Diff3 => {
            emit_marker(output, '<', ms, options.labels.ours.as_deref(), line_ending);
            for line in ours_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(output, '|', ms, options.labels.base.as_deref(), line_ending);
            for line in base_lines {
                output.push_str(line);
                output.push_str(line_ending);
            }
            emit_marker(output, '=', ms, None, line_ending);
            for line in theirs_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(
                output,
                '>',
                ms,
                options.labels.theirs.as_deref(),
                line_ending,
            );
        }
        ConflictStyle::Merge => {
            emit_marker(output, '<', ms, options.labels.ours.as_deref(), line_ending);
            for line in ours_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(output, '=', ms, None, line_ending);
            for line in theirs_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(
                output,
                '>',
                ms,
                options.labels.theirs.as_deref(),
                line_ending,
            );
        }
    }
}

fn emit_marker(output: &mut String, ch: char, size: usize, label: Option<&str>, le: &str) {
    for _ in 0..size {
        output.push(ch);
    }
    if let Some(lbl) = label {
        output.push(' ');
        output.push_str(lbl);
    }
    output.push_str(le);
}

/// Find common prefix and suffix lines between two line sequences.
fn common_prefix_suffix_lines<T: PartialEq>(a: &[T], b: &[T]) -> (usize, usize) {
    let max = a.len().min(b.len());
    let mut prefix = 0;
    while prefix < max && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let remaining = max - prefix;
    let mut suffix = 0;
    while suffix < remaining && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }
    (prefix, suffix)
}

/// Detect the dominant line ending in the full-file merge inputs.
///
/// This remains a local counting heuristic so merge-file output keeps its
/// historical full-text behavior even as other modules share
/// `text_utils::detect_line_ending_from_texts` with context-specific modes.
fn detect_line_ending(ours: &str, theirs: &str, base: &str) -> &'static str {
    let crlf_count = ours.matches("\r\n").count()
        + theirs.matches("\r\n").count()
        + base.matches("\r\n").count();
    let lf_only_count =
        ours.matches('\n').count() + theirs.matches('\n').count() + base.matches('\n').count()
            - crlf_count;

    if crlf_count > lf_only_count {
        "\r\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_diff::EditKind;
    use std::borrow::Cow;

    fn default_opts() -> MergeOptions {
        MergeOptions::default()
    }

    fn opts_with_labels(ours: &str, base: &str, theirs: &str) -> MergeOptions {
        MergeOptions {
            labels: MergeLabels {
                ours: Some(ours.to_string()),
                base: Some(base.to_string()),
                theirs: Some(theirs.to_string()),
            },
            ..Default::default()
        }
    }

    fn opts_with_strategy(strategy: MergeStrategy) -> MergeOptions {
        MergeOptions {
            strategy,
            ..Default::default()
        }
    }

    fn opts_with_style(style: ConflictStyle) -> MergeOptions {
        MergeOptions {
            style,
            ..Default::default()
        }
    }

    #[test]
    fn edits_to_hunks_inserts_use_borrowed_cow() {
        let inserted_line = String::from("inserted");
        let edits = vec![Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(inserted_line.as_str()),
        }];

        let hunks = edits_to_hunks(&edits);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].new_lines.len(), 1);
        assert!(matches!(
            &hunks[0].new_lines[0],
            Cow::Borrowed(line) if *line == "inserted"
        ));
    }

    #[test]
    fn reconstruct_side_uses_borrowed_base_and_insert_lines() {
        let base_lines = split_lines("base-1\nbase-2\n");
        let inserted_lines = split_lines("inserted\n");
        let hunks = vec![Hunk {
            base_start: 1,
            base_end: 1,
            new_lines: vec![Cow::Borrowed(inserted_lines[0])],
        }];

        let lines = reconstruct_side(&base_lines, 0, 2, &hunks);
        assert_eq!(lines.len(), 3);
        assert!(matches!(&lines[0], Cow::Borrowed(line) if *line == "base-1"));
        assert!(matches!(&lines[1], Cow::Borrowed(line) if *line == "inserted"));
        assert!(matches!(&lines[2], Cow::Borrowed(line) if *line == "base-2"));
    }

    #[test]
    fn coalesce_zealous_conflicts_reuses_borrowed_separator_lines() {
        let base_lines = split_lines("top\n\nbottom\n");
        let hunks = vec![
            MergedHunk::Conflict {
                base_start: 0,
                base_end: 1,
                ours_lines: vec![Cow::Borrowed("ours-1")],
                theirs_lines: vec![Cow::Borrowed("theirs-1")],
            },
            MergedHunk::Conflict {
                base_start: 2,
                base_end: 3,
                ours_lines: vec![Cow::Borrowed("ours-2")],
                theirs_lines: vec![Cow::Borrowed("theirs-2")],
            },
        ];

        let coalesced = coalesce_zealous_conflicts(&base_lines, hunks);
        assert_eq!(coalesced.len(), 1);

        let MergedHunk::Conflict {
            ours_lines,
            theirs_lines,
            ..
        } = &coalesced[0]
        else {
            panic!("expected coalesced conflict hunk");
        };

        assert_eq!(ours_lines.len(), 3);
        assert_eq!(theirs_lines.len(), 3);
        assert!(matches!(&ours_lines[1], Cow::Borrowed(line) if line.is_empty()));
        assert!(matches!(&theirs_lines[1], Cow::Borrowed(line) if line.is_empty()));
    }

    // -----------------------------------------------------------------------
    // Identity and clean merge
    // -----------------------------------------------------------------------

    #[test]
    fn merge_identity() {
        let text = "line1\nline2\nline3\n";
        let result = merge_file(text, text, text, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, text);
    }

    #[test]
    fn merge_nonoverlapping_clean() {
        let base = "line1\nline2\nline3\n";
        let ours = "LINE1\nline2\nline3\n";
        let theirs = "line1\nline2\nLINE3\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "LINE1\nline2\nLINE3\n");
    }

    #[test]
    fn merge_nonoverlapping_additions() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nbbb\nccc\nours_added\n";
        let theirs = "theirs_added\naaa\nbbb\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "theirs_added\naaa\nbbb\nccc\nours_added\n");
    }

    // -----------------------------------------------------------------------
    // Conflict detection and marker format
    // -----------------------------------------------------------------------

    #[test]
    fn merge_overlapping_conflict() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
        assert_eq!(result.conflict_count, 1);
        assert!(result.output.contains("<<<<<<<"));
        assert!(result.output.contains("======="));
        assert!(result.output.contains(">>>>>>>"));
        assert!(result.output.contains("OURS"));
        assert!(result.output.contains("THEIRS"));
    }

    #[test]
    fn merge_conflict_markers_with_labels() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let opts = opts_with_labels("local", "ancestor", "remote");
        let result = merge_file(base, ours, theirs, &opts);
        assert!(!result.is_clean());
        assert!(result.output.contains("<<<<<<< local"));
        assert!(result.output.contains(">>>>>>> remote"));
    }

    #[test]
    fn merge_delete_vs_modify_conflict() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\n";
        let theirs = "aaa\nBBB\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
    }

    // -----------------------------------------------------------------------
    // Conflict resolution strategies
    // -----------------------------------------------------------------------

    #[test]
    fn merge_ours_strategy() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, ours, theirs, &opts_with_strategy(MergeStrategy::Ours));
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nOURS\nccc\n");
    }

    #[test]
    fn merge_theirs_strategy() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(
            base,
            ours,
            theirs,
            &opts_with_strategy(MergeStrategy::Theirs),
        );
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nTHEIRS\nccc\n");
    }

    #[test]
    fn merge_union_strategy() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(
            base,
            ours,
            theirs,
            &opts_with_strategy(MergeStrategy::Union),
        );
        assert!(result.is_clean());
        assert!(result.output.contains("OURS"));
        assert!(result.output.contains("THEIRS"));
        // Union: ours comes before theirs.
        let ours_pos = result.output.find("OURS").unwrap();
        let theirs_pos = result.output.find("THEIRS").unwrap();
        assert!(ours_pos < theirs_pos);
    }

    // -----------------------------------------------------------------------
    // Diff3 and zdiff3 conflict styles
    // -----------------------------------------------------------------------

    #[test]
    fn merge_diff3_output() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Diff3));
        assert!(!result.is_clean());
        assert!(result.output.contains("|||||||"));
        assert!(result.output.contains("bbb"));
    }

    #[test]
    fn zdiff3_extracts_common_prefix_suffix() {
        // Both sides share prefix "A" and suffix "E" around the conflict.
        let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
        let theirs = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
        let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
        assert!(!result.is_clean());
        // "A" should appear before the conflict marker, not inside.
        let marker_start = result.output.find("<<<<<<<").unwrap();
        let a_positions: Vec<_> = result
            .output
            .match_indices("\nA\n")
            .map(|(pos, _)| pos)
            .collect();
        // At least one "A" occurrence should be before the conflict.
        assert!(
            a_positions.iter().any(|&pos| pos < marker_start),
            "Common prefix 'A' should be before conflict markers"
        );
    }

    // -----------------------------------------------------------------------
    // Marker size
    // -----------------------------------------------------------------------

    #[test]
    fn merge_marker_size_10() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let opts = MergeOptions {
            marker_size: 10,
            ..Default::default()
        };
        let result = merge_file(base, ours, theirs, &opts);
        assert!(result.output.contains("<<<<<<<<<<"));
        assert!(result.output.contains("=========="));
        assert!(result.output.contains(">>>>>>>>>>"));
    }

    // -----------------------------------------------------------------------
    // Trailing newline / EOF edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn merge_preserves_trailing_newline() {
        let base = "aaa\nbbb\n";
        let ours = "aaa\nbbb\n";
        let theirs = "aaa\nBBB\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert!(result.output.ends_with('\n'));
    }

    #[test]
    fn merge_no_trailing_newline_when_inputs_lack_it() {
        let base = "aaa";
        let ours = "aaa";
        let theirs = "aaa";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert!(!result.output.ends_with('\n'));
    }

    // -----------------------------------------------------------------------
    // CRLF handling
    // -----------------------------------------------------------------------

    #[test]
    fn merge_crlf_conflict_markers() {
        let base = "1\r\n2\r\n3\r\n";
        let ours = "1\r\n2\r\n4\r\n";
        let theirs = "1\r\n2\r\n5\r\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
        // Conflict markers should use CRLF too.
        assert!(result.output.contains("<<<<<<<\r\n"));
        assert!(result.output.contains("=======\r\n"));
        assert!(result.output.contains(">>>>>>>\r\n"));
    }

    #[test]
    fn merge_lf_conflict_markers() {
        let base = "1\n2\n3\n";
        let ours = "1\n2\n4\n";
        let theirs = "1\n2\n5\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
        assert!(result.output.contains("<<<<<<<\n"));
        assert!(result.output.contains("=======\n"));
        assert!(result.output.contains(">>>>>>>\n"));
        // Ensure no CRLF.
        assert!(!result.output.contains("\r\n"));
    }

    // -----------------------------------------------------------------------
    // Multiple conflicts
    // -----------------------------------------------------------------------

    #[test]
    fn merge_multiple_conflicts() {
        let base = "a\nb\nc\nd\ne\n";
        let ours = "A\nb\nC\nd\ne\n";
        let theirs = "X\nb\nY\nd\ne\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert_eq!(result.conflict_count, 2);
    }

    // -----------------------------------------------------------------------
    // Identical changes
    // -----------------------------------------------------------------------

    #[test]
    fn merge_identical_changes_are_clean() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nXXX\nccc\n";
        let theirs = "aaa\nXXX\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nXXX\nccc\n");
    }

    // -----------------------------------------------------------------------
    // Empty inputs
    // -----------------------------------------------------------------------

    #[test]
    fn merge_all_empty() {
        let result = merge_file("", "", "", &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "");
    }

    #[test]
    fn merge_base_empty_both_add_same() {
        let result = merge_file("", "added\n", "added\n", &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "added\n");
    }

    #[test]
    fn merge_base_empty_both_add_different() {
        let result = merge_file("", "ours\n", "theirs\n", &default_opts());
        assert!(!result.is_clean());
    }

    // -----------------------------------------------------------------------
    // Only one side changes
    // -----------------------------------------------------------------------

    #[test]
    fn merge_only_ours_changes() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let result = merge_file(base, ours, base, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nOURS\nccc\n");
    }

    #[test]
    fn merge_only_theirs_changes() {
        let base = "aaa\nbbb\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, base, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nTHEIRS\nccc\n");
    }

    #[test]
    fn merge_identical_changes_both_sides_resolves_cleanly() {
        // When ours and theirs make the exact same change, the hunk-level
        // short-circuit avoids reconstructing the theirs side entirely.
        let base = "first\nsecond\nthird\n";
        let both = "first\nreplaced\nthird\n";
        let result = merge_file(base, both, both, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "first\nreplaced\nthird\n");
    }

    // -----------------------------------------------------------------------
    // Three-way alignment (§30 aligned row space)
    // -----------------------------------------------------------------------

    fn run_lens(runs: &[AlignedRun]) -> (usize, usize, usize) {
        runs.iter().fold((0, 0, 0), |acc, run| {
            (
                acc.0 + run.base.len(),
                acc.1 + run.ours.len(),
                acc.2 + run.theirs.len(),
            )
        })
    }

    fn assert_partitions(runs: &[AlignedRun], base: &str, ours: &str, theirs: &str) {
        let (b, o, t) = run_lens(runs);
        assert_eq!(b, split_lines(base).len(), "base lines partitioned");
        assert_eq!(o, split_lines(ours).len(), "ours lines partitioned");
        assert_eq!(t, split_lines(theirs).len(), "theirs lines partitioned");
        let mut bp = 0;
        let mut op = 0;
        let mut tp = 0;
        for run in runs {
            assert_eq!(run.base.start, bp, "base ranges contiguous");
            assert_eq!(run.ours.start, op, "ours ranges contiguous");
            assert_eq!(run.theirs.start, tp, "theirs ranges contiguous");
            bp = run.base.end;
            op = run.ours.end;
            tp = run.theirs.end;
        }
    }

    #[test]
    fn align_identity_is_one_unchanged_run() {
        let text = "a\nb\nc\n";
        let runs = align_three_way(text, text, text, DiffAlgorithm::Myers);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, AlignedRunKind::Unchanged);
        assert_eq!(runs[0].visual_rows(), 3);
        assert_partitions(&runs, text, text, text);
    }

    #[test]
    fn align_classifies_side_changes_and_conflicts() {
        let base = "a\nb\nc\nd\ne\n";
        let ours = "a\nB1\nB2\nc\nd\nE-ours\n";
        let theirs = "a\nb\nc\nd\nE-theirs\n";
        let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
        assert_partitions(&runs, base, ours, theirs);

        // Fine-grained rows: a | b anchored to its replacement + theirs copy |
        // ours' second replacement line alone | c d | e as one modified-line
        // row holding both replacements.
        let kinds: Vec<_> = runs.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AlignedRunKind::Unchanged,
                AlignedRunKind::OursChanged,
                AlignedRunKind::OursChanged,
                AlignedRunKind::Unchanged,
                AlignedRunKind::Conflict,
            ],
        );
        // Row 1: base `b` rides with ours' first replacement and theirs' copy.
        assert_eq!(runs[1].base, 1..2);
        assert_eq!(runs[1].ours, 1..2);
        assert_eq!(runs[1].theirs, 1..2);
        // Row 2: ours' second replacement line, base/theirs padded.
        assert_eq!(runs[2].base, 2..2);
        assert_eq!(runs[2].ours, 2..3);
        assert_eq!(runs[2].theirs.len(), 0);
        // Row 4: `e` modified by both — one row with both replacements.
        assert_eq!(runs[4].base.len(), 1);
        assert_eq!(runs[4].ours.len(), 1);
        assert_eq!(runs[4].theirs.len(), 1);
        assert_eq!(runs[4].visual_rows(), 1);
    }

    /// First kdiff3-parity case from manual testing: ours inserts two lines
    /// and keeps the base line; theirs replaces it with four lines. The kept
    /// base line must anchor to its identical ours copy on one row, with
    /// theirs' replacement lines flowing 1:1 through all rows.
    #[test]
    fn align_anchors_kept_line_below_paired_insertions() {
        let base = "ctx\nreturn SLA\n";
        let ours = "ctx\nif critical:\n    return 2x\nreturn SLA\n";
        let theirs = "ctx\nthreshold = SLA\nif blocked:\n    half\nreturn threshold\n";
        let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
        assert_partitions(&runs, base, ours, theirs);

        // ctx | [o1|t1] [o2|t2] | [base ~ ours copy ~ t3] | [t4]
        let rows: Vec<(usize, usize, usize)> = runs
            .iter()
            .map(|r| (r.base.len(), r.ours.len(), r.theirs.len()))
            .collect();
        assert_eq!(
            rows,
            vec![(1, 1, 1), (0, 2, 2), (1, 1, 1), (0, 0, 1)],
            "kept base line must share a row with its ours copy and theirs' third line",
        );
        // The anchored row pairs base line 1 with ours line 3 (its copy).
        assert_eq!(runs[2].base, 1..2);
        assert_eq!(runs[2].ours, 3..4);
        assert_eq!(runs[2].theirs, 3..4);
    }

    /// Second kdiff3-parity case: ours replaces two base lines with two new
    /// ones; theirs keeps one base line mid-region among three new lines.
    /// The theirs-kept base line anchors to its copy, and ours' second
    /// replacement rides on that anchor row.
    #[test]
    fn align_anchors_theirs_kept_line_with_ours_replacement() {
        let base = "ctx\ndone = len\nreturn round(done)\n";
        let ours = "ctx\nfinished = len\nreturn round(finished)\n";
        let theirs = "ctx\nactive = [t]\ndone = len\ndenom = len or 1\nreturn round(denom)\n";
        let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
        assert_partitions(&runs, base, ours, theirs);

        // ctx | [o1|t1] | [base done ~ o2 ~ theirs copy] | [base return ~ t3]
        // | [t4]
        let rows: Vec<(usize, usize, usize)> = runs
            .iter()
            .map(|r| (r.base.len(), r.ours.len(), r.theirs.len()))
            .collect();
        assert_eq!(
            rows,
            vec![(1, 1, 1), (0, 1, 1), (1, 1, 1), (1, 0, 1), (0, 0, 1)],
        );
        // Anchor row: base `done = len` with theirs' identical copy.
        assert_eq!(runs[2].base, 1..2);
        assert_eq!(runs[2].theirs, 2..3);
    }

    #[test]
    fn align_marks_identical_changes_as_both_same() {
        let base = "a\nb\nc\n";
        let both = "a\nX\nc\n";
        let runs = align_three_way(base, both, both, DiffAlgorithm::Myers);
        assert_partitions(&runs, base, both, both);
        assert!(runs.iter().any(|r| r.kind == AlignedRunKind::BothSame));
        assert!(!runs.iter().any(|r| r.kind == AlignedRunKind::Conflict));
    }

    #[test]
    fn align_handles_empty_and_added_files() {
        let runs = align_three_way("", "", "", DiffAlgorithm::Myers);
        assert!(runs.is_empty());

        let runs = align_three_way("", "added\n", "other\n", DiffAlgorithm::Myers);
        assert_partitions(&runs, "", "added\n", "other\n");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, AlignedRunKind::Conflict);
        assert_eq!(runs[0].visual_rows(), 1);
    }

    #[test]
    fn align_matches_merge_conflict_detection() {
        // Alignment and merge_file must agree on what is a conflict.
        let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let ours = "1\nO2\n3\n4\n5\nO6\n7\n8\n9\n";
        let theirs = "1\nT2\n3\n4\n5\n6\n7\nT8\n9\n";
        let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
        assert_partitions(&runs, base, ours, theirs);
        let conflict_runs = runs
            .iter()
            .filter(|r| r.kind == AlignedRunKind::Conflict)
            .count();
        let merged = merge_file(base, ours, theirs, &MergeOptions::default());
        assert_eq!(conflict_runs, merged.conflict_count);
    }

    #[test]
    fn merge_identical_multi_hunk_changes_resolves_cleanly() {
        let base = "a\nb\nc\nd\ne\n";
        let both = "a\nX\nc\nY\ne\n";
        let result = merge_file(base, both, both, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "a\nX\nc\nY\ne\n");
    }
}
