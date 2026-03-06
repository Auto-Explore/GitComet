//! KDiff3-style fixture harness for merge algorithm regression testing.
//!
//! Auto-discovers test fixtures in `tests/fixtures/merge/` following the naming
//! convention:
//!   - `{prefix}_base.{ext}`
//!   - `{prefix}_contrib1.{ext}` (ours / local)
//!   - `{prefix}_contrib2.{ext}` (theirs / remote)
//!   - `{prefix}_expected_result.{ext}` (optional)
//!
//! Expected result files support two formats:
//! 1. **Merged output golden** (plain text): compare directly to `merge_file`.
//! 2. **Alignment triples** (KDiff3-style): one row per visual line with
//!    `base_idx contrib1_idx contrib2_idx` and `-1` for gaps.
//!
//! Expected files may also include optional directive lines prefixed with
//! `#@` to control fixture execution, for example:
//!   - `#@ api=bytes`
//!   - `#@ opts.style=diff3`
//!   - `#@ opts.strategy=ours`
//!   - `#@ opts.marker_size=10`
//!   - `#@ opts.diff_algorithm=histogram`
//!   - `#@ opts.label.ours=HEAD`
//!   - `#@ opts.label.base=BASE`
//!   - `#@ opts.label.theirs=branch`
//!   - `#@ expect.clean=true`
//!   - `#@ expect.conflict_count=0`
//!   - `#@ expect.error=binary_content`
//!
//! For each discovered fixture the runner:
//! 1. Loads all three input files.
//! 2. Runs `merge_file` or `merge_file_bytes` (per directives).
//! 3. Applies algorithm-independent merge-output invariants.
//! 4. If fixture uses alignment triples, builds a three-way line alignment and
//!    validates sequence monotonicity + equality consistency invariants.
//! 5. Compares actual output/alignment against expected result when present.
//! 6. On mismatch, writes `{prefix}_actual_result.{ext}` for manual diff.

use gitgpui_core::merge::{
    ConflictStyle, DiffAlgorithm, MergeError, MergeOptions, MergeStrategy, merge_file,
    merge_file_bytes,
};
use std::collections::{BTreeMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

/// A single discovered merge fixture.
#[derive(Debug)]
struct MergeFixture {
    name: String,
    base_path: PathBuf,
    contrib1_path: PathBuf,
    contrib2_path: PathBuf,
    expected_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AlignmentRow {
    base: Option<usize>,
    contrib1: Option<usize>,
    contrib2: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExpectedFixture {
    Empty,
    MergeOutput(String),
    Alignment(Vec<AlignmentRow>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MergeApi {
    #[default]
    Text,
    Bytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpectedError {
    BinaryContent,
}

#[derive(Clone, Debug)]
struct FixtureDirectives {
    api: MergeApi,
    merge_options: MergeOptions,
    expected_clean: Option<bool>,
    expected_conflict_count: Option<usize>,
    expected_error: Option<ExpectedError>,
}

impl Default for FixtureDirectives {
    fn default() -> Self {
        Self {
            api: MergeApi::Text,
            merge_options: MergeOptions::default(),
            expected_clean: None,
            expected_conflict_count: None,
            expected_error: None,
        }
    }
}

#[derive(Clone, Debug)]
struct ParsedExpectedFixture {
    expected: ExpectedFixture,
    directives: FixtureDirectives,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffOp {
    Equal,
    Delete,
    Insert,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SideProjection {
    base_to_side: Vec<Option<usize>>,
    inserts_before: Vec<Vec<usize>>,
}

/// Discover all merge fixtures in the given directory.
///
/// Scans for files matching `*_base.*` and derives companion file paths.
/// Returns fixtures when `base/contrib1/contrib2` exist; expected result is optional.
fn discover_fixtures(dir: &Path) -> Vec<MergeFixture> {
    let mut fixtures_by_name: BTreeMap<String, MergeFixture> = BTreeMap::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("Failed to read fixtures directory {}: {}", dir.display(), e),
    };

    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Look for files matching *_base.*.
        if let Some((prefix, extension_suffix)) = parse_base_fixture_filename(&file_name) {
            let contrib1_path = dir.join(format!("{}_contrib1{}", prefix, extension_suffix));
            let contrib2_path = dir.join(format!("{}_contrib2{}", prefix, extension_suffix));
            let expected_path = dir.join(format!("{}_expected_result{}", prefix, extension_suffix));

            if contrib1_path.exists() && contrib2_path.exists() {
                fixtures_by_name.insert(
                    prefix.to_string(),
                    MergeFixture {
                        name: prefix.to_string(),
                        base_path: path.clone(),
                        contrib1_path,
                        contrib2_path,
                        expected_path: expected_path.exists().then_some(expected_path),
                    },
                );
            }
        }
    }

    fixtures_by_name.into_values().collect()
}

fn parse_base_fixture_filename(file_name: &str) -> Option<(&str, &str)> {
    let marker = "_base.";
    let marker_start = file_name.rfind(marker)?;
    let prefix = &file_name[..marker_start];
    let extension_suffix = &file_name[marker_start + "_base".len()..];

    if prefix.is_empty() || extension_suffix.len() <= 1 {
        return None;
    }

    Some((prefix, extension_suffix))
}

fn actual_result_path(fixture: &MergeFixture) -> PathBuf {
    let extension_suffix = fixture
        .base_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|file_name| {
            parse_base_fixture_filename(file_name)
                .filter(|(prefix, _)| *prefix == fixture.name)
                .map(|(_, ext)| ext.to_string())
        })
        .or_else(|| {
            fixture
                .base_path
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
        })
        .unwrap_or_default();

    let file_name = format!("{}_actual_result{}", fixture.name, extension_suffix);

    if let Some(expected_path) = &fixture.expected_path {
        expected_path.with_file_name(file_name)
    } else {
        fixture.base_path.with_file_name(file_name)
    }
}

fn parse_expected_fixture(raw: &str) -> ExpectedFixture {
    let has_data_line = raw
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with('#'));

    if !has_data_line {
        return ExpectedFixture::Empty;
    }

    if let Some(rows) = parse_alignment_rows(raw) {
        return ExpectedFixture::Alignment(rows);
    }

    ExpectedFixture::MergeOutput(raw.to_string())
}

fn parse_expected_fixture_with_directives(raw: &str) -> Result<ParsedExpectedFixture, String> {
    let mut directives = FixtureDirectives::default();
    let mut filtered = String::new();

    for segment in raw.split_inclusive('\n') {
        let line_without_lf = segment.strip_suffix('\n').unwrap_or(segment);
        let directive_line = line_without_lf.trim_end_matches('\r');

        if let Some(rest) = directive_line.strip_prefix("#@") {
            parse_directive(rest.trim(), &mut directives)?;
            continue;
        }

        filtered.push_str(segment);
    }

    // Handle final line when the file does not end with '\n'.
    if !raw.ends_with('\n')
        && !raw.is_empty()
        && filtered.is_empty()
        && !raw.trim_end_matches('\r').starts_with("#@")
    {
        filtered.push_str(raw);
    }

    Ok(ParsedExpectedFixture {
        expected: parse_expected_fixture(&filtered),
        directives,
    })
}

fn parse_directive(raw: &str, directives: &mut FixtureDirectives) -> Result<(), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| format!("directive must be key=value, got `{raw}`"))?;
    let key = key.trim();
    let value = value.trim();

    match key {
        "api" => {
            directives.api = parse_merge_api(value)?;
            Ok(())
        }
        "opts.style" => {
            directives.merge_options.style = parse_conflict_style(value)?;
            Ok(())
        }
        "opts.strategy" => {
            directives.merge_options.strategy = parse_merge_strategy(value)?;
            Ok(())
        }
        "opts.marker_size" => {
            directives.merge_options.marker_size = value
                .parse::<usize>()
                .map_err(|_| format!("invalid marker size `{value}`"))?;
            Ok(())
        }
        "opts.diff_algorithm" => {
            directives.merge_options.diff_algorithm = parse_diff_algorithm(value)?;
            Ok(())
        }
        "opts.label.ours" => {
            directives.merge_options.labels.ours = Some(value.to_string());
            Ok(())
        }
        "opts.label.base" => {
            directives.merge_options.labels.base = Some(value.to_string());
            Ok(())
        }
        "opts.label.theirs" => {
            directives.merge_options.labels.theirs = Some(value.to_string());
            Ok(())
        }
        "expect.clean" => {
            directives.expected_clean = Some(parse_bool(value)?);
            Ok(())
        }
        "expect.conflict_count" => {
            directives.expected_conflict_count = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid conflict_count `{value}`"))?,
            );
            Ok(())
        }
        "expect.error" => {
            directives.expected_error = Some(parse_expected_error(value)?);
            Ok(())
        }
        _ => Err(format!("unknown directive key `{key}`")),
    }
}

fn parse_merge_api(value: &str) -> Result<MergeApi, String> {
    match value.to_ascii_lowercase().as_str() {
        "text" => Ok(MergeApi::Text),
        "bytes" => Ok(MergeApi::Bytes),
        _ => Err(format!("unknown api `{value}`")),
    }
}

fn parse_conflict_style(value: &str) -> Result<ConflictStyle, String> {
    match value.to_ascii_lowercase().as_str() {
        "merge" => Ok(ConflictStyle::Merge),
        "diff3" => Ok(ConflictStyle::Diff3),
        "zdiff3" => Ok(ConflictStyle::Zdiff3),
        _ => Err(format!("unknown opts.style `{value}`")),
    }
}

fn parse_merge_strategy(value: &str) -> Result<MergeStrategy, String> {
    match value.to_ascii_lowercase().as_str() {
        "normal" => Ok(MergeStrategy::Normal),
        "ours" => Ok(MergeStrategy::Ours),
        "theirs" => Ok(MergeStrategy::Theirs),
        "union" => Ok(MergeStrategy::Union),
        _ => Err(format!("unknown opts.strategy `{value}`")),
    }
}

fn parse_diff_algorithm(value: &str) -> Result<DiffAlgorithm, String> {
    match value.to_ascii_lowercase().as_str() {
        "myers" => Ok(DiffAlgorithm::Myers),
        "histogram" => Ok(DiffAlgorithm::Histogram),
        _ => Err(format!("unknown opts.diff_algorithm `{value}`")),
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(format!("invalid boolean `{value}`")),
    }
}

fn parse_expected_error(value: &str) -> Result<ExpectedError, String> {
    match value.to_ascii_lowercase().as_str() {
        "binary" | "binary_content" => Ok(ExpectedError::BinaryContent),
        _ => Err(format!("unknown expect.error `{value}`")),
    }
}

fn parse_alignment_rows(raw: &str) -> Option<Vec<AlignmentRow>> {
    fn parse_cell(token: &str) -> Option<Option<usize>> {
        let value: i64 = token.parse().ok()?;
        if value == -1 {
            Some(None)
        } else if value >= 0 {
            Some(Some(value as usize))
        } else {
            None
        }
    }

    let mut rows = Vec::new();
    let mut saw_row = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let p0 = parts.next()?;
        let p1 = parts.next()?;
        let p2 = parts.next()?;
        if parts.next().is_some() {
            return None;
        }

        rows.push(AlignmentRow {
            base: parse_cell(p0)?,
            contrib1: parse_cell(p1)?,
            contrib2: parse_cell(p2)?,
        });
        saw_row = true;
    }

    if saw_row { Some(rows) } else { None }
}

fn serialize_alignment_rows(rows: &[AlignmentRow]) -> String {
    fn cell_text(cell: Option<usize>) -> String {
        cell.map(|v| v.to_string())
            .unwrap_or_else(|| "-1".to_string())
    }

    let mut out = String::new();
    for row in rows {
        out.push_str(&cell_text(row.base));
        out.push(' ');
        out.push_str(&cell_text(row.contrib1));
        out.push(' ');
        out.push_str(&cell_text(row.contrib2));
        out.push('\n');
    }
    out
}

fn split_visual_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}

fn lcs_diff_ops(a: &[&str], b: &[&str]) -> Vec<DiffOp> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];

    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(n + m);
    let mut i = 0usize;
    let mut j = 0usize;
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(DiffOp::Equal);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(DiffOp::Delete);
            i += 1;
        } else {
            ops.push(DiffOp::Insert);
            j += 1;
        }
    }
    while i < n {
        ops.push(DiffOp::Delete);
        i += 1;
    }
    while j < m {
        ops.push(DiffOp::Insert);
        j += 1;
    }
    ops
}

fn project_side(base_lines: &[&str], side_lines: &[&str]) -> SideProjection {
    let ops = lcs_diff_ops(base_lines, side_lines);
    let mut base_to_side = vec![None; base_lines.len()];
    let mut inserts_before = vec![Vec::new(); base_lines.len() + 1];
    let mut base_idx = 0usize;
    let mut side_idx = 0usize;

    for op in ops {
        match op {
            DiffOp::Equal => {
                base_to_side[base_idx] = Some(side_idx);
                base_idx += 1;
                side_idx += 1;
            }
            DiffOp::Delete => {
                base_to_side[base_idx] = None;
                base_idx += 1;
            }
            DiffOp::Insert => {
                inserts_before[base_idx].push(side_idx);
                side_idx += 1;
            }
        }
    }

    assert_eq!(base_idx, base_lines.len());
    assert_eq!(side_idx, side_lines.len());

    SideProjection {
        base_to_side,
        inserts_before,
    }
}

fn align_insertions(
    contrib1_indices: &[usize],
    contrib2_indices: &[usize],
    contrib1_lines: &[&str],
    contrib2_lines: &[&str],
) -> Vec<(Option<usize>, Option<usize>)> {
    let seq1: Vec<&str> = contrib1_indices
        .iter()
        .map(|&idx| contrib1_lines[idx])
        .collect();
    let seq2: Vec<&str> = contrib2_indices
        .iter()
        .map(|&idx| contrib2_lines[idx])
        .collect();
    let ops = lcs_diff_ops(&seq1, &seq2);

    let mut out = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;

    for op in ops {
        match op {
            DiffOp::Equal => {
                out.push((Some(contrib1_indices[i]), Some(contrib2_indices[j])));
                i += 1;
                j += 1;
            }
            DiffOp::Delete => {
                out.push((Some(contrib1_indices[i]), None));
                i += 1;
            }
            DiffOp::Insert => {
                out.push((None, Some(contrib2_indices[j])));
                j += 1;
            }
        }
    }

    out
}

fn build_three_way_alignment(base: &str, contrib1: &str, contrib2: &str) -> Vec<AlignmentRow> {
    let base_lines = split_visual_lines(base);
    let contrib1_lines = split_visual_lines(contrib1);
    let contrib2_lines = split_visual_lines(contrib2);

    let p1 = project_side(&base_lines, &contrib1_lines);
    let p2 = project_side(&base_lines, &contrib2_lines);

    let mut rows = Vec::new();

    for slot in 0..=base_lines.len() {
        let inserted_rows = align_insertions(
            &p1.inserts_before[slot],
            &p2.inserts_before[slot],
            &contrib1_lines,
            &contrib2_lines,
        );
        for (c1, c2) in inserted_rows {
            rows.push(AlignmentRow {
                base: None,
                contrib1: c1,
                contrib2: c2,
            });
        }

        if slot < base_lines.len() {
            rows.push(AlignmentRow {
                base: Some(slot),
                contrib1: p1.base_to_side[slot],
                contrib2: p2.base_to_side[slot],
            });
        }
    }

    rows
}

fn validate_alignment_invariants(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    rows: &[AlignmentRow],
    fixture_name: &str,
) {
    let base_lines = split_visual_lines(base);
    let contrib1_lines = split_visual_lines(contrib1);
    let contrib2_lines = split_visual_lines(contrib2);

    validate_alignment_monotonicity(rows, fixture_name);

    for (row_ix, row) in rows.iter().enumerate() {
        if let Some(ix) = row.base {
            assert!(
                ix < base_lines.len(),
                "[{}] alignment row {}: base index {} out of bounds ({})",
                fixture_name,
                row_ix + 1,
                ix,
                base_lines.len()
            );
        }
        if let Some(ix) = row.contrib1 {
            assert!(
                ix < contrib1_lines.len(),
                "[{}] alignment row {}: contrib1 index {} out of bounds ({})",
                fixture_name,
                row_ix + 1,
                ix,
                contrib1_lines.len()
            );
        }
        if let Some(ix) = row.contrib2 {
            assert!(
                ix < contrib2_lines.len(),
                "[{}] alignment row {}: contrib2 index {} out of bounds ({})",
                fixture_name,
                row_ix + 1,
                ix,
                contrib2_lines.len()
            );
        }

        if let (Some(b), Some(c1)) = (row.base, row.contrib1) {
            assert_eq!(
                base_lines[b],
                contrib1_lines[c1],
                "[{}] alignment row {}: base/contrib1 content mismatch",
                fixture_name,
                row_ix + 1
            );
        }
        if let (Some(b), Some(c2)) = (row.base, row.contrib2) {
            assert_eq!(
                base_lines[b],
                contrib2_lines[c2],
                "[{}] alignment row {}: base/contrib2 content mismatch",
                fixture_name,
                row_ix + 1
            );
        }
        if let (Some(c1), Some(c2)) = (row.contrib1, row.contrib2) {
            assert_eq!(
                contrib1_lines[c1],
                contrib2_lines[c2],
                "[{}] alignment row {}: contrib1/contrib2 content mismatch",
                fixture_name,
                row_ix + 1
            );
        }
    }
}

fn validate_alignment_monotonicity(rows: &[AlignmentRow], fixture_name: &str) {
    fn check_column(
        rows: &[AlignmentRow],
        fixture_name: &str,
        column_name: &str,
        value: impl Fn(&AlignmentRow) -> Option<usize>,
    ) {
        let mut prev: Option<usize> = None;
        for (row_ix, row) in rows.iter().enumerate() {
            let Some(curr) = value(row) else {
                continue;
            };
            if let Some(prev_ix) = prev {
                assert!(
                    curr > prev_ix,
                    "[{}] alignment row {}: {} index {} is not strictly increasing after {}",
                    fixture_name,
                    row_ix + 1,
                    column_name,
                    curr,
                    prev_ix
                );
            }
            prev = Some(curr);
        }
    }

    check_column(rows, fixture_name, "base", |row| row.base);
    check_column(rows, fixture_name, "contrib1", |row| row.contrib1);
    check_column(rows, fixture_name, "contrib2", |row| row.contrib2);
}

/// Validate algorithm-independent invariants on the merge output.
///
/// These checks apply regardless of the specific merge algorithm:
///
/// 1. **Conflict marker well-formedness**: Every `<<<<<<<` has a matching
///    `=======` and `>>>>>>>`, in order, with no nesting.
///
/// 2. **Content integrity**: Every non-marker line in the output can be traced
///    back to at least one of the three input files (base, contrib1, contrib2).
///
/// 3. **Context preservation**: Lines that are identical in base, contrib1, and
///    contrib2 all appear in the output.
fn validate_merge_output_invariants(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    fixture_name: &str,
) {
    // If any input already contains marker-like lines, marker structure checks
    // become ambiguous because those lines may be payload.
    let has_ambiguous_input_markers = [base, contrib1, contrib2]
        .iter()
        .any(|text| contains_marker_like_line(text));
    if !has_ambiguous_input_markers {
        validate_marker_wellformedness(output, fixture_name);
    }
    validate_content_integrity(base, contrib1, contrib2, output, fixture_name);
    validate_context_preservation(base, contrib1, contrib2, output, fixture_name);
}

/// Check that conflict markers are well-formed: balanced and properly ordered.
fn validate_marker_wellformedness(output: &str, fixture_name: &str) {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum State {
        Outside,
        InOurs,   // after <<<<<<< before =======
        InBase,   // after ||||||| before ======= (diff3/zdiff3)
        InTheirs, // after ======= before >>>>>>>
    }

    let mut state = State::Outside;
    let mut conflict_count = 0u32;

    for (line_num, line) in output.lines().enumerate() {
        let trimmed = line.trim_end();
        let line_num = line_num + 1; // 1-indexed for error messages

        if is_open_marker(trimmed) {
            assert_eq!(
                state,
                State::Outside,
                "[{}] line {}: unexpected <<<<<<< (already inside conflict)",
                fixture_name,
                line_num
            );
            state = State::InOurs;
            conflict_count += 1;
        } else if is_base_marker(trimmed) {
            assert_eq!(
                state,
                State::InOurs,
                "[{}] line {}: unexpected ||||||| (expected inside ours section)",
                fixture_name,
                line_num
            );
            state = State::InBase;
        } else if is_separator_marker(trimmed) {
            assert!(
                state == State::InOurs || state == State::InBase,
                "[{}] line {}: unexpected ======= (expected after <<<<<<< or |||||||)",
                fixture_name,
                line_num
            );
            state = State::InTheirs;
        } else if is_close_marker(trimmed) {
            assert_eq!(
                state,
                State::InTheirs,
                "[{}] line {}: unexpected >>>>>>> (expected after =======)",
                fixture_name,
                line_num
            );
            state = State::Outside;
        }
    }

    assert_eq!(
        state,
        State::Outside,
        "[{}] unclosed conflict markers ({} conflicts opened)",
        fixture_name,
        conflict_count
    );
}

/// Check that every non-marker line in output comes from at least one input.
fn validate_content_integrity(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    fixture_name: &str,
) {
    let base_lines: HashSet<&str> = base.lines().collect();
    let contrib1_lines: HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: HashSet<&str> = contrib2.lines().collect();

    for (line_num, line) in output.lines().enumerate() {
        let trimmed = line.trim_end();
        if is_open_marker(trimmed)
            || is_close_marker(trimmed)
            || is_separator_marker(trimmed)
            || is_base_marker(trimmed)
        {
            continue;
        }

        assert!(
            base_lines.contains(line)
                || contrib1_lines.contains(line)
                || contrib2_lines.contains(line),
            "[{}] line {}: output line {:?} not found in any input",
            fixture_name,
            line_num + 1,
            line
        );
    }
}

/// Check that lines common to all three inputs appear in the output.
fn validate_context_preservation(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    fixture_name: &str,
) {
    let contrib1_lines: HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: HashSet<&str> = contrib2.lines().collect();
    let output_lines: HashSet<&str> = output.lines().collect();

    for line in base.lines() {
        if contrib1_lines.contains(line) && contrib2_lines.contains(line) {
            assert!(
                output_lines.contains(line),
                "[{}] line {:?} is common to all three inputs but missing from output",
                fixture_name,
                line
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Marker detection helpers
// ---------------------------------------------------------------------------

fn is_open_marker(line: &str) -> bool {
    line.starts_with("<<<<<<<")
        && line[7..].chars().all(|c| {
            c == '<'
                || c == ' '
                || c.is_alphanumeric()
                || c == '/'
                || c == '.'
                || c == ':'
                || c == '-'
                || c == '_'
        })
}

fn is_close_marker(line: &str) -> bool {
    line.starts_with(">>>>>>>")
        && line[7..].chars().all(|c| {
            c == '>'
                || c == ' '
                || c.is_alphanumeric()
                || c == '/'
                || c == '.'
                || c == ':'
                || c == '-'
                || c == '_'
        })
}

fn is_separator_marker(line: &str) -> bool {
    line.starts_with("=======") && line[7..].chars().all(|c| c == '=')
}

fn is_base_marker(line: &str) -> bool {
    line.starts_with("|||||||")
        && line[7..].chars().all(|c| {
            c == '|'
                || c == ' '
                || c.is_alphanumeric()
                || c == '/'
                || c == '.'
                || c == ':'
                || c == '-'
                || c == '_'
        })
}

fn contains_marker_like_line(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_end();
        is_open_marker(trimmed)
            || is_base_marker(trimmed)
            || is_separator_marker(trimmed)
            || is_close_marker(trimmed)
    })
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn run_validation_with_artifact<F>(
    fixture_name: &str,
    validation_name: &str,
    actual_path: &Path,
    actual_content: &str,
    validate: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    match panic::catch_unwind(AssertUnwindSafe(validate)) {
        Ok(()) => Ok(()),
        Err(payload) => {
            let panic_message = panic_payload_message(payload);
            let artifact_note = match std::fs::write(actual_path, actual_content) {
                Ok(()) => format!("actual written to {}", actual_path.display()),
                Err(e) => format!(
                    "failed to write actual result to {}: {}",
                    actual_path.display(),
                    e
                ),
            };
            Err(format!(
                "[{}] {} failed ({artifact_note}): {}",
                fixture_name, validation_name, panic_message
            ))
        }
    }
}

fn merge_error_name(err: &MergeError) -> &'static str {
    match err {
        MergeError::BinaryContent => "binary_content",
    }
}

fn run_fixture(fixture: &MergeFixture) -> Result<(), String> {
    let base_bytes = std::fs::read(&fixture.base_path)
        .map_err(|e| format!("[{}] failed to read base: {}", fixture.name, e))?;
    let contrib1_bytes = std::fs::read(&fixture.contrib1_path)
        .map_err(|e| format!("[{}] failed to read contrib1: {}", fixture.name, e))?;
    let contrib2_bytes = std::fs::read(&fixture.contrib2_path)
        .map_err(|e| format!("[{}] failed to read contrib2: {}", fixture.name, e))?;

    let ParsedExpectedFixture {
        expected,
        directives,
    } = match &fixture.expected_path {
        Some(expected_path) => {
            let expected_raw = std::fs::read_to_string(expected_path)
                .map_err(|e| format!("[{}] failed to read expected_result: {}", fixture.name, e))?;
            parse_expected_fixture_with_directives(&expected_raw)
                .map_err(|e| format!("[{}] invalid expected directives: {}", fixture.name, e))?
        }
        None => ParsedExpectedFixture {
            expected: ExpectedFixture::Empty,
            directives: FixtureDirectives::default(),
        },
    };

    if directives.expected_error.is_some() && !matches!(expected, ExpectedFixture::Empty) {
        return Err(format!(
            "[{}] expected error fixtures must not include non-directive expected output",
            fixture.name
        ));
    }

    let (base_text_opt, contrib1_text_opt, contrib2_text_opt, merge_result) = match directives.api {
        MergeApi::Text => {
            let base = String::from_utf8(base_bytes.clone())
                .map_err(|e| format!("[{}] base is not valid UTF-8: {}", fixture.name, e))?;
            let contrib1 = String::from_utf8(contrib1_bytes.clone())
                .map_err(|e| format!("[{}] contrib1 is not valid UTF-8: {}", fixture.name, e))?;
            let contrib2 = String::from_utf8(contrib2_bytes.clone())
                .map_err(|e| format!("[{}] contrib2 is not valid UTF-8: {}", fixture.name, e))?;
            let result = merge_file(&base, &contrib1, &contrib2, &directives.merge_options);
            (Some(base), Some(contrib1), Some(contrib2), Ok(result))
        }
        MergeApi::Bytes => (
            None,
            None,
            None,
            merge_file_bytes(
                &base_bytes,
                &contrib1_bytes,
                &contrib2_bytes,
                &directives.merge_options,
            ),
        ),
    };

    let result = match merge_result {
        Ok(result) => {
            if let Some(expected_error) = directives.expected_error {
                return Err(format!(
                    "[{}] expected error {:?}, but merge succeeded with {} conflict(s)",
                    fixture.name, expected_error, result.conflict_count
                ));
            }
            result
        }
        Err(err) => {
            return if let Some(expected_error) = directives.expected_error {
                if expected_error == ExpectedError::BinaryContent
                    && matches!(err, MergeError::BinaryContent)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "[{}] expected error {:?}, got {}",
                        fixture.name,
                        expected_error,
                        merge_error_name(&err)
                    ))
                }
            } else {
                Err(format!(
                    "[{}] merge failed unexpectedly: {}",
                    fixture.name,
                    merge_error_name(&err)
                ))
            };
        }
    };

    if let Some(expected_clean) = directives.expected_clean {
        let actual_clean = result.is_clean();
        if actual_clean != expected_clean {
            return Err(format!(
                "[{}] expected clean={}, got clean={}",
                fixture.name, expected_clean, actual_clean
            ));
        }
    }

    if let Some(expected_conflict_count) = directives.expected_conflict_count {
        if result.conflict_count != expected_conflict_count {
            return Err(format!(
                "[{}] expected conflict_count={}, got {}",
                fixture.name, expected_conflict_count, result.conflict_count
            ));
        }
    }

    let base = match base_text_opt {
        Some(base) => base,
        None => String::from_utf8(base_bytes)
            .map_err(|e| format!("[{}] base is not valid UTF-8: {}", fixture.name, e))?,
    };
    let contrib1 = match contrib1_text_opt {
        Some(contrib1) => contrib1,
        None => String::from_utf8(contrib1_bytes)
            .map_err(|e| format!("[{}] contrib1 is not valid UTF-8: {}", fixture.name, e))?,
    };
    let contrib2 = match contrib2_text_opt {
        Some(contrib2) => contrib2,
        None => String::from_utf8(contrib2_bytes)
            .map_err(|e| format!("[{}] contrib2 is not valid UTF-8: {}", fixture.name, e))?,
    };

    let merge_actual_path = actual_result_path(fixture);
    run_validation_with_artifact(
        &fixture.name,
        "merge output invariants",
        &merge_actual_path,
        &result.output,
        || {
            validate_merge_output_invariants(
                &base,
                &contrib1,
                &contrib2,
                &result.output,
                &fixture.name,
            )
        },
    )?;

    match expected {
        ExpectedFixture::Empty => Ok(()),
        ExpectedFixture::MergeOutput(expected_output) => {
            if result.output == expected_output {
                Ok(())
            } else {
                let actual_path = &merge_actual_path;
                let _ = std::fs::write(actual_path, &result.output);
                Err(format!(
                    "[{}] merge output mismatch (actual written to {})\n  expected:\n{}\n  actual:\n{}",
                    fixture.name,
                    actual_path.display(),
                    indent_text(&expected_output),
                    indent_text(&result.output),
                ))
            }
        }
        ExpectedFixture::Alignment(expected_rows) => {
            let actual_rows = build_three_way_alignment(&base, &contrib1, &contrib2);
            let actual_path = actual_result_path(fixture);
            let actual_text = serialize_alignment_rows(&actual_rows);
            run_validation_with_artifact(
                &fixture.name,
                "alignment invariants",
                &actual_path,
                &actual_text,
                || {
                    validate_alignment_invariants(
                        &base,
                        &contrib1,
                        &contrib2,
                        &actual_rows,
                        &fixture.name,
                    )
                },
            )?;

            if actual_rows == expected_rows {
                Ok(())
            } else {
                let expected_text = serialize_alignment_rows(&expected_rows);
                let _ = std::fs::write(&actual_path, &actual_text);
                Err(format!(
                    "[{}] alignment mismatch (actual written to {})\n  expected:\n{}\n  actual:\n{}",
                    fixture.name,
                    actual_path.display(),
                    indent_text(&expected_text),
                    indent_text(&actual_text),
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main fixture test
// ---------------------------------------------------------------------------

#[test]
fn fixture_harness_discovers_and_runs_all_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge");
    let fixtures = discover_fixtures(&fixtures_dir);

    assert!(
        !fixtures.is_empty(),
        "No fixtures discovered in {}",
        fixtures_dir.display()
    );

    let mut pass_count = 0u32;
    let mut fail_count = 0u32;
    let mut failures: Vec<String> = Vec::new();

    for fixture in &fixtures {
        match run_fixture(fixture) {
            Ok(()) => pass_count += 1,
            Err(err) => {
                fail_count += 1;
                failures.push(err);
            }
        }
    }

    eprintln!(
        "\nFixture harness: {} fixtures, {} passed, {} failed",
        fixtures.len(),
        pass_count,
        fail_count
    );

    if !failures.is_empty() {
        panic!(
            "{} fixture(s) failed:\n\n{}",
            fail_count,
            failures.join("\n\n")
        );
    }
}

/// Individually test each fixture so failures are reported per-fixture.
#[test]
fn fixture_1_simpletest() {
    run_single_fixture("1_simpletest");
}

#[test]
fn fixture_2_prefer_identical() {
    run_single_fixture("2_prefer_identical");
}

#[test]
fn fixture_3_nonoverlapping_changes() {
    run_single_fixture("3_nonoverlapping_changes");
}

#[test]
fn fixture_4_overlapping_conflict() {
    run_single_fixture("4_overlapping_conflict");
}

#[test]
fn fixture_5_identical_changes() {
    run_single_fixture("5_identical_changes");
}

#[test]
fn fixture_6_delete_vs_modify() {
    run_single_fixture("6_delete_vs_modify");
}

#[test]
fn fixture_7_add_add_conflict() {
    run_single_fixture("7_add_add_conflict");
}

#[test]
fn fixture_8_kdiff3_simple_alignment() {
    run_single_fixture("8_kdiff3_simple_alignment");
}

#[test]
fn fixture_9_kdiff3_prefer_identical_alignment() {
    run_single_fixture("9_kdiff3_prefer_identical_alignment");
}

#[test]
fn parses_alignment_expected_rows() {
    let parsed = parse_expected_fixture(
        "# alignment format\n\
         0 0 0\n\
         -1 1 -1\n\
         1 2 1\n",
    );
    assert!(matches!(parsed, ExpectedFixture::Alignment(_)));
}

#[test]
fn parses_directives_and_merge_output_body() {
    let parsed = parse_expected_fixture_with_directives(
        "#@ api=bytes\n\
         #@ opts.style=diff3\n\
         #@ opts.strategy=union\n\
         #@ opts.marker_size=10\n\
         #@ opts.diff_algorithm=histogram\n\
         #@ opts.label.ours=HEAD\n\
         #@ opts.label.base=BASE\n\
         #@ opts.label.theirs=theirs\n\
         #@ expect.clean=false\n\
         #@ expect.conflict_count=1\n\
         body line\n",
    )
    .expect("directives should parse");

    assert_eq!(parsed.directives.api, MergeApi::Bytes);
    assert_eq!(parsed.directives.merge_options.style, ConflictStyle::Diff3);
    assert_eq!(
        parsed.directives.merge_options.strategy,
        MergeStrategy::Union
    );
    assert_eq!(parsed.directives.merge_options.marker_size, 10);
    assert_eq!(
        parsed.directives.merge_options.diff_algorithm,
        DiffAlgorithm::Histogram
    );
    assert_eq!(
        parsed.directives.merge_options.labels.ours.as_deref(),
        Some("HEAD")
    );
    assert_eq!(
        parsed.directives.merge_options.labels.base.as_deref(),
        Some("BASE")
    );
    assert_eq!(
        parsed.directives.merge_options.labels.theirs.as_deref(),
        Some("theirs")
    );
    assert_eq!(parsed.directives.expected_clean, Some(false));
    assert_eq!(parsed.directives.expected_conflict_count, Some(1));
    assert!(matches!(
        parsed.expected,
        ExpectedFixture::MergeOutput(ref s) if s == "body line\n"
    ));
}

#[test]
fn parses_directive_expected_error_binary() {
    let parsed = parse_expected_fixture_with_directives(
        "#@ api=bytes\n\
         #@ expect.error=binary_content\n",
    )
    .expect("directives should parse");

    assert_eq!(parsed.directives.api, MergeApi::Bytes);
    assert_eq!(
        parsed.directives.expected_error,
        Some(ExpectedError::BinaryContent)
    );
    assert!(matches!(parsed.expected, ExpectedFixture::Empty));
}

#[test]
fn discover_fixtures_includes_cases_without_expected_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixtures_dir = dir.path();

    std::fs::write(fixtures_dir.join("10_optional_base.txt"), "line\n").expect("write base");
    std::fs::write(fixtures_dir.join("10_optional_contrib1.txt"), "line\n").expect("write c1");
    std::fs::write(fixtures_dir.join("10_optional_contrib2.txt"), "line\n").expect("write c2");

    let fixtures = discover_fixtures(fixtures_dir);
    let fixture = fixtures
        .iter()
        .find(|f| f.name == "10_optional")
        .expect("fixture should be discovered");

    assert!(
        fixture.expected_path.is_none(),
        "expected_path should be optional when expected fixture file is absent"
    );
}

#[test]
fn discover_fixtures_uses_last_base_marker_in_filename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixtures_dir = dir.path();

    std::fs::write(
        fixtures_dir.join("13_nested_base_token_base.fixture.txt"),
        "base\n",
    )
    .expect("write base");
    std::fs::write(
        fixtures_dir.join("13_nested_base_token_contrib1.fixture.txt"),
        "base\n",
    )
    .expect("write c1");
    std::fs::write(
        fixtures_dir.join("13_nested_base_token_contrib2.fixture.txt"),
        "base\n",
    )
    .expect("write c2");

    let fixtures = discover_fixtures(fixtures_dir);
    let fixture = fixtures
        .iter()
        .find(|f| f.name == "13_nested_base_token")
        .expect("fixture should be discovered");

    assert_eq!(
        fixture.base_path.file_name().and_then(|n| n.to_str()),
        Some("13_nested_base_token_base.fixture.txt")
    );
    assert_eq!(
        fixture.contrib1_path.file_name().and_then(|n| n.to_str()),
        Some("13_nested_base_token_contrib1.fixture.txt")
    );
    assert_eq!(
        fixture.contrib2_path.file_name().and_then(|n| n.to_str()),
        Some("13_nested_base_token_contrib2.fixture.txt")
    );
}

#[test]
fn run_fixture_without_expected_result_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixtures_dir = dir.path();

    std::fs::write(fixtures_dir.join("11_no_expected_base.txt"), "base\n").expect("write base");
    std::fs::write(fixtures_dir.join("11_no_expected_contrib1.txt"), "base\n").expect("write c1");
    std::fs::write(fixtures_dir.join("11_no_expected_contrib2.txt"), "base\n").expect("write c2");

    let fixtures = discover_fixtures(fixtures_dir);
    let fixture = fixtures
        .iter()
        .find(|f| f.name == "11_no_expected")
        .expect("fixture should be discovered");

    run_fixture(fixture).expect("fixture should pass without expected result file");
}

#[test]
fn run_fixture_bytes_api_expected_binary_error_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixtures_dir = dir.path();

    std::fs::write(
        fixtures_dir.join("15_binary_error_base.txt"),
        b"text\x00binary\n",
    )
    .expect("write base");
    std::fs::write(
        fixtures_dir.join("15_binary_error_contrib1.txt"),
        b"text\x00binary\n",
    )
    .expect("write c1");
    std::fs::write(fixtures_dir.join("15_binary_error_contrib2.txt"), b"text\n").expect("write c2");
    std::fs::write(
        fixtures_dir.join("15_binary_error_expected_result.txt"),
        "#@ api=bytes\n#@ expect.error=binary_content\n",
    )
    .expect("write expected");

    let fixtures = discover_fixtures(fixtures_dir);
    let fixture = fixtures
        .iter()
        .find(|f| f.name == "15_binary_error")
        .expect("fixture should be discovered");

    run_fixture(fixture).expect("fixture should pass with expected binary error");
}

#[test]
fn actual_result_path_without_expected_uses_base_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixture = MergeFixture {
        name: "12_path_fallback".to_string(),
        base_path: dir.path().join("12_path_fallback_base.txt"),
        contrib1_path: dir.path().join("12_path_fallback_contrib1.txt"),
        contrib2_path: dir.path().join("12_path_fallback_contrib2.txt"),
        expected_path: None,
    };

    assert_eq!(
        actual_result_path(&fixture),
        dir.path().join("12_path_fallback_actual_result.txt")
    );
}

#[test]
fn actual_result_path_preserves_multi_dot_extension_suffix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixture = MergeFixture {
        name: "14_multi_ext".to_string(),
        base_path: dir.path().join("14_multi_ext_base.merge.fixture.txt"),
        contrib1_path: dir.path().join("14_multi_ext_contrib1.merge.fixture.txt"),
        contrib2_path: dir.path().join("14_multi_ext_contrib2.merge.fixture.txt"),
        expected_path: Some(
            dir.path()
                .join("14_multi_ext_expected_result.merge.fixture.txt"),
        ),
    };

    assert_eq!(
        actual_result_path(&fixture),
        dir.path()
            .join("14_multi_ext_actual_result.merge.fixture.txt")
    );
}

#[test]
fn run_validation_with_artifact_writes_actual_result_on_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let actual_path = dir.path().join("panic_actual.txt");

    let err = run_validation_with_artifact(
        "fixture_case",
        "synthetic validation",
        &actual_path,
        "actual-output\n",
        || panic!("synthetic panic"),
    )
    .expect_err("validation should report panic as an error");

    assert!(
        err.contains("synthetic validation failed"),
        "unexpected error text: {err}"
    );
    assert!(
        err.contains("synthetic panic"),
        "panic message should be preserved: {err}"
    );

    let written = std::fs::read_to_string(&actual_path).expect("read actual output artifact");
    assert_eq!(written, "actual-output\n");
}

#[test]
fn run_validation_with_artifact_success_does_not_write_actual_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let actual_path = dir.path().join("success_actual.txt");

    run_validation_with_artifact(
        "fixture_case",
        "synthetic validation",
        &actual_path,
        "unused\n",
        || {},
    )
    .expect("successful validation should pass");

    assert!(
        !actual_path.exists(),
        "success path should not write an actual_result artifact"
    );
}

fn run_single_fixture(name: &str) {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge");
    let fixtures = discover_fixtures(&fixtures_dir);
    let fixture = fixtures
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("Fixture {:?} not found", name));

    if let Err(err) = run_fixture(fixture) {
        panic!("{err}");
    }
}

fn indent_text(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}
