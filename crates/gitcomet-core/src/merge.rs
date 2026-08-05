//! Three-way file merge algorithm.
//!
//! Takes base, local (ours), and remote (theirs) file contents and produces
//! merged output, potentially with conflict markers where the two sides
//! changed the same region differently.
//!
//! Compatible with `git merge-file` marker format.

use crate::file_diff::split_lines;
use std::borrow::Cow;
use std::fmt;

mod minimap;
mod plan;

pub use minimap::{MinimapRowKind, minimap_row_kind, minimap_rows};
pub(crate) use plan::normalized_without_whitespace;
pub use plan::{
    AlignedRow, InteractiveMergePlanBudget, ManualAlignment, ManualAlignmentList, MergeBlock,
    MergeBlockClassification, MergeBlockId, MergePlan, MergeSource, OrderedSelection,
    build_merge_plan, build_merge_plan_with_alignments, build_merge_plan_with_optional_base,
    interactive_merge_plan_is_practical, try_build_interactive_merge_plan_with_alignments,
    try_build_interactive_merge_plan_with_optional_base,
};

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
    /// Run the additional local↔remote alignment pass.
    ///
    /// KDiff3 exposes this pass as an option and disables it in its application
    /// defaults. It can make identical contributor lines share an aligned row
    /// even when neither line matches the base, but is intentionally opt-in
    /// because it must not alter one-sided merge semantics.
    pub align_contributors: bool,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            style: ConflictStyle::default(),
            strategy: MergeStrategy::default(),
            labels: MergeLabels::default(),
            marker_size: DEFAULT_MARKER_SIZE,
            diff_algorithm: DiffAlgorithm::default(),
            align_contributors: false,
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
    merge_file_bytes_with_optional_base(Some(base), ours, theirs, options)
}

/// Perform a text merge on raw byte inputs with an optional base.
///
/// A missing base uses KDiff3's true two-input mode rather than treating an
/// empty file as an ancestor.
pub fn merge_file_bytes_with_optional_base(
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    options: &MergeOptions,
) -> Result<MergeResult, MergeError> {
    let plan = build_merge_plan_bytes_with_optional_base(base, ours, theirs, options)?;
    Ok(render_merge_plan(&plan, options))
}

/// Build a shared merge plan from raw byte inputs after binary detection.
///
/// This exposes the same plan used by [`merge_file_bytes_with_optional_base`]
/// to callers that need its KDiff3-compatible block metadata before rendering.
pub fn build_merge_plan_bytes_with_optional_base(
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    options: &MergeOptions,
) -> Result<MergePlan, MergeError> {
    fn check_binary(data: &[u8]) -> Result<&str, MergeError> {
        if data.contains(&0) {
            return Err(MergeError::BinaryContent);
        }
        std::str::from_utf8(data).map_err(|_| MergeError::BinaryContent)
    }

    let base = base.map(check_binary).transpose()?;
    let ours = check_binary(ours)?;
    let theirs = check_binary(theirs)?;
    Ok(build_merge_plan_with_optional_base(
        base, ours, theirs, options,
    ))
}

/// Alias for [`merge_file_bytes_with_optional_base`].
pub fn merge_file_bytes_optional_base(
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    options: &MergeOptions,
) -> Result<MergeResult, MergeError> {
    merge_file_bytes_with_optional_base(base, ours, theirs, options)
}

/// Perform a three-way merge of text files.
///
/// Diffs `base` against both `ours` and `theirs`, then walks the two edit
/// scripts to produce a merged result. Where both sides changed the same
/// base region differently, a conflict is emitted (or auto-resolved per
/// the chosen strategy).
pub fn merge_file(base: &str, ours: &str, theirs: &str, options: &MergeOptions) -> MergeResult {
    merge_file_with_optional_base(Some(base), ours, theirs, options)
}

/// Merge text with an optional common ancestor.
///
/// `None` selects KDiff3-compatible two-input behavior. Existing callers with
/// a base should continue using [`merge_file`].
pub fn merge_file_with_optional_base(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    options: &MergeOptions,
) -> MergeResult {
    let plan = build_merge_plan_with_optional_base(base, ours, theirs, options);
    render_merge_plan(&plan, options)
}

/// Alias for [`merge_file_with_optional_base`].
pub fn merge_file_optional_base(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    options: &MergeOptions,
) -> MergeResult {
    merge_file_with_optional_base(base, ours, theirs, options)
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

/// Compute the three-way alignment of base/ours/theirs (section 30 aligned row
/// space). This projects the shared KDiff3-compatible plan into per-side line
/// ranges. The runs partition each input exactly.
pub fn align_three_way(
    base: &str,
    ours: &str,
    theirs: &str,
    algorithm: DiffAlgorithm,
) -> Vec<AlignedRun> {
    let options = MergeOptions {
        diff_algorithm: algorithm,
        ..MergeOptions::default()
    };
    let plan = build_merge_plan(base, ours, theirs, &options);
    aligned_plan_to_runs(&plan)
}

/// Compute a direct two-way alignment of ours/theirs (section 30 aligned row space,
/// two-way full mode without a base version — e.g. both-added conflicts).
///
/// Runs carry empty base ranges. A replaced region pairs its lines 1:1
/// top-aligned as one `Conflict` run; lines present on only one side become
/// `OursChanged` (deletion) or `TheirsChanged` (insertion) runs. The runs
/// partition both inputs exactly.
pub fn align_two_way(ours: &str, theirs: &str, algorithm: DiffAlgorithm) -> Vec<AlignedRun> {
    let options = MergeOptions {
        diff_algorithm: algorithm,
        ..MergeOptions::default()
    };
    let plan = build_merge_plan_with_optional_base(None, ours, theirs, &options);
    aligned_plan_to_runs(&plan)
}

/// Project an existing shared merge plan into the legacy aligned-run API.
pub fn align_merge_plan(plan: &MergePlan) -> Vec<AlignedRun> {
    aligned_plan_to_runs(plan)
}

fn aligned_row_kind(row: &AlignedRow, three_way: bool) -> AlignedRunKind {
    if !three_way {
        return match (row.a, row.b) {
            (Some(_), Some(_)) if row.equal_ab => AlignedRunKind::Unchanged,
            (Some(_), Some(_)) => AlignedRunKind::Conflict,
            (Some(_), None) => AlignedRunKind::OursChanged,
            (None, Some(_)) => AlignedRunKind::TheirsChanged,
            (None, None) => AlignedRunKind::Unchanged,
        };
    }

    match (row.a, row.b, row.c) {
        (Some(_), Some(_), Some(_)) if row.equal_ab && row.equal_ac => AlignedRunKind::Unchanged,
        (Some(_), Some(_), Some(_)) if row.equal_ab => AlignedRunKind::TheirsChanged,
        (Some(_), Some(_), Some(_)) if row.equal_ac => AlignedRunKind::OursChanged,
        (Some(_), Some(_), Some(_)) if row.equal_bc => AlignedRunKind::BothSame,
        (Some(_), Some(_), Some(_)) => AlignedRunKind::Conflict,
        (Some(_), Some(_), None) if row.equal_ab => AlignedRunKind::TheirsChanged,
        (Some(_), None, Some(_)) if row.equal_ac => AlignedRunKind::OursChanged,
        (None, Some(_), Some(_)) if row.equal_bc => AlignedRunKind::BothSame,
        (None, Some(_), None) => AlignedRunKind::OursChanged,
        (None, None, Some(_)) => AlignedRunKind::TheirsChanged,
        (Some(_), None, None) => AlignedRunKind::BothSame,
        _ => AlignedRunKind::Conflict,
    }
}

fn aligned_plan_to_runs(plan: &MergePlan) -> Vec<AlignedRun> {
    let three_way = plan.has_base();
    let mut forced_conflict = vec![false; plan.rows.len()];

    // In true two-input mode KDiff3 groups an uneven replacement as one
    // conflict block. The aligned-row projection must keep that block whole:
    // trailing rows on the longer side are padding within the replacement,
    // not independent insertions or deletions.
    if !three_way {
        for block in &plan.blocks {
            let contains_replacement = plan.rows[block.rows.clone()]
                .iter()
                .any(|row| row.a.is_some() && row.b.is_some() && !row.equal_ab);
            if block.original_conflict && contains_replacement {
                forced_conflict[block.rows.clone()].fill(true);
            }
        }
    }

    aligned_rows_to_runs(&plan.rows, three_way, &forced_conflict)
}

fn aligned_rows_to_runs(
    rows: &[AlignedRow],
    three_way: bool,
    forced_conflict: &[bool],
) -> Vec<AlignedRun> {
    let mut runs = Vec::<AlignedRun>::new();
    let mut a = 0usize;
    let mut b = 0usize;
    let mut c = 0usize;

    for (row_index, row) in rows.iter().enumerate() {
        let (base_present, ours_present, theirs_present) = if three_way {
            (row.a.is_some(), row.b.is_some(), row.c.is_some())
        } else {
            (false, row.a.is_some(), row.b.is_some())
        };
        let kind = if forced_conflict.get(row_index).copied().unwrap_or(false) {
            AlignedRunKind::Conflict
        } else {
            aligned_row_kind(row, three_way)
        };
        let next = AlignedRun {
            base: a..a + usize::from(base_present),
            ours: b..b + usize::from(ours_present),
            theirs: c..c + usize::from(theirs_present),
            kind,
        };
        a = next.base.end;
        b = next.ours.end;
        c = next.theirs.end;

        let mut coalesced = false;
        if let Some(previous) = runs.last_mut()
            && previous.kind == kind
        {
            let previous_rows = previous.visual_rows();
            let previous_presence = (
                previous.base.len() == previous_rows,
                previous.ours.len() == previous_rows,
                previous.theirs.len() == previous_rows,
            );
            let next_presence = (base_present, ours_present, theirs_present);
            let combined_rows = next.base.end.saturating_sub(previous.base.start).max(
                next.ours
                    .end
                    .saturating_sub(previous.ours.start)
                    .max(next.theirs.end.saturating_sub(previous.theirs.start)),
            );
            // A run can represent the rows exactly when extending its longest
            // range adds one visual row. Presence changes are only folded for
            // rows explicitly kept in a two-input replacement block.
            if combined_rows == previous_rows + 1
                && (previous_presence == next_presence
                    || forced_conflict.get(row_index).copied().unwrap_or(false))
            {
                previous.base.end = next.base.end;
                previous.ours.end = next.ours.end;
                previous.theirs.end = next.theirs.end;
                coalesced = true;
            }
        }
        if !coalesced {
            runs.push(next);
        }
    }
    runs
}

struct MergePlanLineSlices<'a> {
    a: Vec<&'a str>,
    b: Vec<&'a str>,
    c: Vec<&'a str>,
}

impl<'a> MergePlanLineSlices<'a> {
    fn new(plan: &'a MergePlan) -> Self {
        if let Some(base) = plan.base.as_deref() {
            Self {
                a: split_lines(base),
                b: split_lines(&plan.local),
                c: split_lines(&plan.remote),
            }
        } else {
            Self {
                a: split_lines(&plan.local),
                b: split_lines(&plan.remote),
                c: Vec::new(),
            }
        }
    }

    fn source(&self, source: MergeSource) -> &[&'a str] {
        match source {
            MergeSource::A => &self.a,
            MergeSource::B => &self.b,
            MergeSource::C => &self.c,
        }
    }

    fn block_source_lines(
        &self,
        plan: &MergePlan,
        block: &MergeBlock,
        source: MergeSource,
    ) -> Vec<&'a str> {
        let source_lines = self.source(source);
        plan.rows[block.rows.clone()]
            .iter()
            .filter_map(|row| {
                row.line(source)
                    .and_then(|index| source_lines.get(index).copied())
            })
            .collect()
    }

    fn block_ancestor_lines(&self, plan: &MergePlan, block: &MergeBlock) -> Vec<&'a str> {
        if !plan.has_base() {
            return Vec::new();
        }
        let start = plan.rows[..block.rows.start]
            .iter()
            .rev()
            .find(|row| row.equal_ab && row.equal_ac)
            .and_then(|row| row.a)
            .map_or(0, |line| line + 1);
        let end = plan.rows[block.rows.end..]
            .iter()
            .find(|row| row.equal_ab && row.equal_ac)
            .and_then(|row| row.a)
            .unwrap_or(self.a.len());
        self.a[start.min(end)..end].to_vec()
    }
}

fn append_plan_source(
    output: &mut String,
    plan: &MergePlan,
    block: &MergeBlock,
    source: MergeSource,
    lines: &MergePlanLineSlices<'_>,
) {
    for line in lines.block_source_lines(plan, block, source) {
        output.push_str(line);
        output.push_str(plan.line_ending);
    }
}

fn render_plan_conflict(
    output: &mut String,
    plan: &MergePlan,
    block: &MergeBlock,
    options: &MergeOptions,
    lines: &MergePlanLineSlices<'_>,
) {
    let ours: Vec<Cow<'_, str>> = lines
        .block_source_lines(plan, block, plan.local_source())
        .into_iter()
        .map(Cow::Borrowed)
        .collect();
    let theirs: Vec<Cow<'_, str>> = lines
        .block_source_lines(plan, block, plan.remote_source())
        .into_iter()
        .map(Cow::Borrowed)
        .collect();
    let base = lines.block_ancestor_lines(plan, block);

    if plan.has_base() {
        emit_conflict_markers(output, &ours, &theirs, &base, options, plan.line_ending);
    } else {
        // Diff3/zdiff3 have no ancestor section in true two-input mode.
        let mut two_input_options = options.clone();
        two_input_options.style = ConflictStyle::Merge;
        emit_conflict_markers(
            output,
            &ours,
            &theirs,
            &[],
            &two_input_options,
            plan.line_ending,
        );
    }
}

/// Render a shared merge plan using the requested marker and strategy options.
pub fn render_merge_plan(plan: &MergePlan, options: &MergeOptions) -> MergeResult {
    let mut output = String::new();
    let mut conflict_count = 0usize;
    let lines = MergePlanLineSlices::new(plan);
    let final_block_is_manual = plan
        .blocks
        .last()
        .is_some_and(|block| block.manual_content.is_some());

    for block in &plan.blocks {
        if let Some(manual) = block.manual_content.as_deref() {
            output.push_str(manual);
            continue;
        }

        let strategy_selection = if block.selection.is_empty() {
            match options.strategy {
                MergeStrategy::Normal => None,
                MergeStrategy::Ours => Some(OrderedSelection::from(plan.local_source())),
                MergeStrategy::Theirs => Some(OrderedSelection::from(plan.remote_source())),
                MergeStrategy::Union => Some(OrderedSelection::from_sources([
                    plan.local_source(),
                    plan.remote_source(),
                ])),
            }
        } else {
            Some(block.selection.clone())
        };

        if let Some(selection) = strategy_selection {
            for source in selection.iter() {
                append_plan_source(&mut output, plan, block, source, &lines);
            }
        } else {
            render_plan_conflict(&mut output, plan, block, options, &lines);
            conflict_count += 1;
        }
    }

    if !final_block_is_manual {
        let base_text = plan.base.as_deref().unwrap_or_default();
        let base_lines = if plan.has_base() {
            lines.source(MergeSource::A)
        } else {
            &[]
        };
        apply_trailing_newline_decision(
            &mut output,
            base_text,
            base_lines,
            &plan.local,
            lines.source(plan.local_source()),
            &plan.remote,
            lines.source(plan.remote_source()),
        );
    }

    MergeResult {
        output,
        conflict_count,
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
    ours_lines: &[&str],
    theirs_text: &str,
    theirs_lines: &[&str],
) {
    let ours_has_trailing = ours_text.is_empty() || ours_text.ends_with('\n');
    let theirs_has_trailing = theirs_text.is_empty() || theirs_text.ends_with('\n');
    let base_has_trailing = base_text.is_empty() || base_text.ends_with('\n');

    let output_last = output
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .rsplit('\n')
        .next()
        .unwrap_or("");

    let ours_last_matches = ours_lines.last().is_some_and(|l| *l == output_last);
    let theirs_last_matches = theirs_lines.last().is_some_and(|l| *l == output_last);
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
            // The base section shows the ancestor of the *conflicting* region.
            // Trim it only by the prefix/suffix lines the two sides share *with
            // the base* — not by the sides' full common prefix/suffix length,
            // which can include lines both sides merely added (absent from base).
            // Trimming by the raw length drops real base content (git zdiff3
            // keeps it). The hoisted prefix/suffix are common to ours and theirs,
            // so comparing against `ours_lines` is equivalent to `theirs_lines`.
            let base_prefix = base_common_prefix_len(base_lines, &ours_lines[..prefix_len]);
            let base_suffix =
                base_common_suffix_len(base_lines, &ours_lines[ours_lines.len() - suffix_len..]);
            let base_start = base_prefix.min(base_lines.len());
            let base_end = base_lines.len().saturating_sub(base_suffix).max(base_start);
            let base_conflict = &base_lines[base_start..base_end];
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

/// Count leading lines of `base` that match the corresponding hoisted context
/// lines in `side`. Used to trim the zdiff3 base section by shared-with-base
/// context only, so lines both sides merely added (absent from base) never
/// cause real base content to be dropped.
fn base_common_prefix_len(base: &[&str], side: &[Cow<'_, str>]) -> usize {
    let mut n = 0;
    while n < base.len() && n < side.len() && base[n] == side[n].as_ref() {
        n += 1;
    }
    n
}

/// Count trailing lines of `base` that match the corresponding hoisted context
/// lines in `side`. Companion to [`base_common_prefix_len`].
fn base_common_suffix_len(base: &[&str], side: &[Cow<'_, str>]) -> usize {
    let mut n = 0;
    while n < base.len()
        && n < side.len()
        && base[base.len() - 1 - n] == side[side.len() - 1 - n].as_ref()
    {
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn render_merge_plan_uncached_for_test(
        plan: &MergePlan,
        options: &MergeOptions,
    ) -> MergeResult {
        let mut output = String::new();
        let mut conflict_count = 0usize;
        let final_block_is_manual = plan
            .blocks
            .last()
            .is_some_and(|block| block.manual_content.is_some());
        for block in &plan.blocks {
            if let Some(manual) = block.manual_content.as_deref() {
                output.push_str(manual);
                continue;
            }
            if !block.selection.is_empty() {
                for source in block.selection.iter() {
                    for line in plan.block_source_lines(block, source) {
                        output.push_str(line);
                        output.push_str(plan.line_ending);
                    }
                }
                continue;
            }

            let ours: Vec<Cow<'_, str>> = plan
                .block_source_lines(block, plan.local_source())
                .into_iter()
                .map(Cow::Borrowed)
                .collect();
            let theirs: Vec<Cow<'_, str>> = plan
                .block_source_lines(block, plan.remote_source())
                .into_iter()
                .map(Cow::Borrowed)
                .collect();
            let base = plan.block_ancestor_lines(block);
            emit_conflict_markers(
                &mut output,
                &ours,
                &theirs,
                &base,
                options,
                plan.line_ending,
            );
            conflict_count += 1;
        }
        if !final_block_is_manual {
            let base_text = plan.base.as_deref().unwrap_or_default();
            let base_lines = split_lines(base_text);
            let local_lines = split_lines(&plan.local);
            let remote_lines = split_lines(&plan.remote);
            apply_trailing_newline_decision(
                &mut output,
                base_text,
                &base_lines,
                &plan.local,
                &local_lines,
                &plan.remote,
                &remote_lines,
            );
        }
        MergeResult {
            output,
            conflict_count,
        }
    }

    #[test]
    fn final_manual_content_controls_its_trailing_newline() {
        let options = MergeOptions::default();
        let mut plan = build_merge_plan("base", "local", "remote", &options);
        let conflict = plan.original_conflict_block_indices()[0];

        assert!(plan.set_manual_content(conflict, "local\n".to_owned()));
        assert_eq!(render_merge_plan(&plan, &options).output, "local\n");

        assert!(plan.set_manual_content(conflict, "local".to_owned()));
        assert_eq!(render_merge_plan(&plan, &options).output, "local");

        assert!(plan.set_manual_content(conflict, "local\r\n".to_owned()));
        assert_eq!(render_merge_plan(&plan, &options).output, "local\r\n");
    }

    #[test]
    fn cached_line_renderer_matches_uncached_many_small_blocks() {
        let mut base = String::new();
        let mut local = String::new();
        let mut remote = String::new();
        for index in 0..256 {
            base.push_str(&format!("context-{index}\nbase-{index}\n"));
            local.push_str(&format!("context-{index}\nlocal-{index}\n"));
            remote.push_str(&format!("context-{index}\nremote-{index}\n"));
        }
        let options = MergeOptions {
            style: ConflictStyle::Diff3,
            ..MergeOptions::default()
        };
        let plan = build_merge_plan(&base, &local, &remote, &options);
        assert!(
            plan.original_conflict_block_indices().len() > 200,
            "fixture should exercise many independently rendered blocks",
        );
        assert_eq!(
            render_merge_plan(&plan, &options),
            render_merge_plan_uncached_for_test(&plan, &options),
        );
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

    #[test]
    fn zdiff3_preserves_base_when_sides_share_added_prefix() {
        // Both sides prepend "A" (a line absent from base) before diverging.
        // The zdiff3 base section must still show the real ancestor line "X",
        // not be trimmed away by the sides' common-prefix length.
        let base = "pre\nX\npost\n";
        let ours = "pre\nA\nO\npost\n";
        let theirs = "pre\nA\nT\npost\n";
        let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
        assert!(!result.is_clean());
        let base_marker = result.output.find("|||||||").expect("base marker");
        let sep = result.output[base_marker..]
            .find("=======")
            .expect("separator")
            + base_marker;
        let base_section = &result.output[base_marker..sep];
        assert!(
            base_section.contains("\nX\n"),
            "zdiff3 base section should preserve ancestor line X, got: {base_section:?}"
        );
    }

    #[test]
    fn zdiff3_shows_base_when_prefix_suffix_span_whole_base() {
        // Regression: conflict base region is exactly prefix_len + suffix_len
        // long. The old `len > prefix + suffix` guard emitted an empty base
        // section here; the ancestor lines must be shown.
        let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
        let theirs = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
        let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
        assert!(!result.is_clean());
        let base_marker = result.output.find("|||||||").expect("base marker");
        let sep = result.output[base_marker..]
            .find("=======")
            .expect("separator")
            + base_marker;
        let base_section = &result.output[base_marker..sep];
        assert!(
            base_section.contains("\n5\n6\n"),
            "zdiff3 base section should show ancestor lines 5,6, got: {base_section:?}"
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
    // Three-way alignment (section 30 aligned row space)
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

    /// Third kdiff3-parity case from manual testing: a block where every
    /// line was modified on both sides aligns 1:1 — seven clean rows, no
    /// padding and no stranded base lines.
    #[test]
    fn align_pairs_fully_rewritten_blocks_line_by_line() {
        let make = |suffix: &str| -> String {
            (0..7).map(|i| format!("const_{i} = {suffix}\n")).collect()
        };
        let base = format!("ctx\n{}rest\n", make("base"));
        let ours = format!("ctx\n{}rest\n", make("ours"));
        let theirs = format!("ctx\n{}rest\n", make("theirs"));
        let runs = align_three_way(&base, &ours, &theirs, DiffAlgorithm::Myers);
        assert_partitions(&runs, &base, &ours, &theirs);

        let total_rows: usize = runs.iter().map(AlignedRun::visual_rows).sum();
        assert_eq!(total_rows, 9, "ctx + 7 paired rows + rest, no padding");
        for run in &runs {
            assert_eq!(
                (run.base.len(), run.ours.len(), run.theirs.len()),
                (run.visual_rows(), run.visual_rows(), run.visual_rows()),
                "every row pairs one line from each side",
            );
        }
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

    // -----------------------------------------------------------------------
    // Two-way alignment (section 30 aligned row space, no-base fallback)
    // -----------------------------------------------------------------------

    fn assert_two_way_partitions(runs: &[AlignedRun], ours: &str, theirs: &str) {
        let (b, o, t) = run_lens(runs);
        assert_eq!(b, 0, "two-way runs carry no base lines");
        assert_eq!(o, split_lines(ours).len(), "ours lines partitioned");
        assert_eq!(t, split_lines(theirs).len(), "theirs lines partitioned");
        let mut op = 0;
        let mut tp = 0;
        for run in runs {
            assert_eq!(run.ours.start, op, "ours ranges contiguous");
            assert_eq!(run.theirs.start, tp, "theirs ranges contiguous");
            op = run.ours.end;
            tp = run.theirs.end;
        }
    }

    #[test]
    fn align_two_way_identity_is_one_unchanged_run() {
        let text = "a\nb\nc\n";
        let runs = align_two_way(text, text, DiffAlgorithm::Myers);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, AlignedRunKind::Unchanged);
        assert_eq!(runs[0].visual_rows(), 3);
        assert_two_way_partitions(&runs, text, text);
    }

    #[test]
    fn align_two_way_classifies_inserts_deletes_and_replacements() {
        let ours = "a\nours-only\nb\nreplaced-o\nc\n";
        let theirs = "a\nb\nreplaced-t\nc\ntheirs-only\n";
        let runs = align_two_way(ours, theirs, DiffAlgorithm::Myers);
        assert_two_way_partitions(&runs, ours, theirs);

        let kinds: Vec<_> = runs.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AlignedRunKind::Unchanged,
                AlignedRunKind::OursChanged,
                AlignedRunKind::Unchanged,
                AlignedRunKind::Conflict,
                AlignedRunKind::Unchanged,
                AlignedRunKind::TheirsChanged,
            ],
        );
        // The replaced line pairs 1:1 on a single visual row.
        assert_eq!(runs[3].visual_rows(), 1);
        // Deletion pads theirs; insertion pads ours.
        assert_eq!(runs[1].theirs.len(), 0);
        assert_eq!(runs[5].ours.len(), 0);
    }

    #[test]
    fn align_two_way_pairs_uneven_replacement_top_aligned() {
        let ours = "ctx\no1\no2\nrest\n";
        let theirs = "ctx\nt1\nt2\nt3\nt4\nrest\n";
        let runs = align_two_way(ours, theirs, DiffAlgorithm::Myers);
        assert_two_way_partitions(&runs, ours, theirs);

        let conflict = runs
            .iter()
            .find(|r| r.kind == AlignedRunKind::Conflict)
            .expect("replacement region");
        assert_eq!(conflict.ours.len(), 2);
        assert_eq!(conflict.theirs.len(), 4);
        assert_eq!(conflict.visual_rows(), 4, "shorter side padded below");
    }

    #[test]
    fn align_two_way_handles_empty_sides() {
        assert!(align_two_way("", "", DiffAlgorithm::Myers).is_empty());

        let runs = align_two_way("", "added\n", DiffAlgorithm::Myers);
        assert_two_way_partitions(&runs, "", "added\n");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, AlignedRunKind::TheirsChanged);

        let runs = align_two_way("gone\n", "", DiffAlgorithm::Myers);
        assert_two_way_partitions(&runs, "gone\n", "");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, AlignedRunKind::OursChanged);
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
