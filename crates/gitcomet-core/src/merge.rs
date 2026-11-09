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

/// Where a conflict-style or diff-algorithm value comes from, controlling
/// which spellings are accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigValueSource {
    /// CLI flags: only the canonical values are accepted.
    Cli,
    /// `git config`: historical and alias values are accepted too.
    GitConfig,
}

/// Parses a `merge.conflictstyle` value into a [`ConflictStyle`].
///
/// Both sources accept the same spellings, so the policy parameter exists for
/// symmetry with [`parse_diff_algorithm`]; `None` means the value was not
/// recognized.
pub fn parse_conflict_style(source: ConfigValueSource, value: &str) -> Option<ConflictStyle> {
    let _ = source;
    match value {
        "merge" => Some(ConflictStyle::Merge),
        "diff3" => Some(ConflictStyle::Diff3),
        "zdiff3" => Some(ConflictStyle::Zdiff3),
        _ => None,
    }
}

/// Parses a `diff.algorithm` value into a [`DiffAlgorithm`].
///
/// Git config additionally accepts `patience`, `default`, and `minimal`
/// aliases; the CLI only accepts the canonical spellings.
pub fn parse_diff_algorithm(source: ConfigValueSource, value: &str) -> Option<DiffAlgorithm> {
    match value {
        "myers" => Some(DiffAlgorithm::Myers),
        "histogram" => Some(DiffAlgorithm::Histogram),
        "patience" if source == ConfigValueSource::GitConfig => Some(DiffAlgorithm::Histogram),
        "default" | "minimal" if source == ConfigValueSource::GitConfig => {
            Some(DiffAlgorithm::Myers)
        }
        _ => None,
    }
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
    /// On by default, and the planner is only correct that way. Each
    /// contributor is first diffed against the base independently, so a base
    /// line can be matched to one contributor's line and to a *different*
    /// line of the other; without this pass nothing reconciles the two, and
    /// the leftover contributor line is emitted a second time as a one-sided
    /// add. That shows up as a duplicated line in a merge reported as clean
    /// (see `merge_contributor_alignment_does_not_duplicate_shared_lines`).
    ///
    /// The pass cannot disturb a one-sided merge: it is skipped outright
    /// unless all three inputs differ, and its result is discarded whenever it
    /// would drop an exact base↔contributor anchor the base-only alignment had
    /// established, or would break a manual alignment pin.
    ///
    /// This deliberately diverges from KDiff3, whose `m_bDiff3AlignBC` defaults
    /// to false. KDiff3 can afford that: it is a GUI, the duplicate is visible
    /// in the three columns, and a human resolves it. The same planner here
    /// also backs headless `merge_file`, where the duplicate is written to a
    /// file with `conflict_count == 0` and nobody ever sees it.
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
            align_contributors: true,
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

/// Compute the three-way alignment of base/ours/theirs (aligned row
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

/// Compute a direct two-way alignment of ours/theirs (aligned row space,
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
    // Only two-input plans force rows; `aligned_rows_to_runs` treats an
    // out-of-range index as not forced, so three-way plans pass no vector.
    let mut forced_conflict = if three_way {
        Vec::new()
    } else {
        vec![false; plan.rows.len()]
    };

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

    fn block_source_lines<'s>(
        &'s self,
        plan: &'s MergePlan,
        block: &MergeBlock,
        source: MergeSource,
    ) -> impl Iterator<Item = &'a str> + 's {
        let source_lines = self.source(source);
        plan.rows[block.rows.clone()].iter().filter_map(move |row| {
            row.line(source)
                .and_then(|index| source_lines.get(index).copied())
        })
    }

    fn block_ancestor_lines(&self, plan: &MergePlan, block: &MergeBlock) -> &[&'a str] {
        if !plan.has_base() {
            return &[];
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
        &self.a[start.min(end)..end]
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
        .map(Cow::Borrowed)
        .collect();
    let theirs: Vec<Cow<'_, str>> = lines
        .block_source_lines(plan, block, plan.remote_source())
        .map(Cow::Borrowed)
        .collect();
    let base = lines.block_ancestor_lines(plan, block);

    if plan.has_base() {
        emit_conflict_markers(output, &ours, &theirs, base, options, plan.line_ending);
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
    // A merge output is at least as long as its larger contributor except
    // when whole blocks are dropped; growing from empty re-copied the buffer
    // ~20 times for a multi-megabyte file.
    let mut output = String::with_capacity(plan.local.len().max(plan.remote.len()));
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
            let (prefix_len, suffix_len) =
                crate::text_utils::common_affix_len(ours_lines, theirs_lines);

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
mod tests;
