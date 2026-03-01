//! KDiff3-style fixture harness for merge algorithm regression testing.
//!
//! Auto-discovers test fixtures in `tests/fixtures/merge/` following the naming
//! convention:
//!   - `{prefix}_base.{ext}`
//!   - `{prefix}_contrib1.{ext}` (ours / local)
//!   - `{prefix}_contrib2.{ext}` (theirs / remote)
//!   - `{prefix}_expected_result.{ext}` (expected merged output)
//!
//! For each discovered fixture the runner:
//! 1. Loads all three input files.
//! 2. Runs `merge_file(base, contrib1, contrib2, &default_options)`.
//! 3. Applies algorithm-independent invariant checks.
//! 4. Compares actual output against expected result (when non-empty).
//! 5. On mismatch, writes `{prefix}_actual_result.{ext}` for manual diff.

use gitgpui_core::merge::{MergeOptions, merge_file};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A single discovered merge fixture.
#[derive(Debug)]
struct MergeFixture {
    name: String,
    base_path: PathBuf,
    contrib1_path: PathBuf,
    contrib2_path: PathBuf,
    expected_path: PathBuf,
}

/// Discover all merge fixtures in the given directory.
///
/// Scans for files matching `*_base.*` and derives the companion file paths.
/// Only returns fixtures where all four files exist.
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

        // Look for files matching *_base.*
        if let Some(prefix_end) = file_name.find("_base.") {
            let prefix = &file_name[..prefix_end];
            let ext = &file_name[prefix_end + "_base".len()..]; // includes the dot + extension

            let contrib1_path = dir.join(format!("{}_contrib1{}", prefix, ext));
            let contrib2_path = dir.join(format!("{}_contrib2{}", prefix, ext));
            let expected_path = dir.join(format!("{}_expected_result{}", prefix, ext));

            if contrib1_path.exists() && contrib2_path.exists() && expected_path.exists() {
                fixtures_by_name.insert(
                    prefix.to_string(),
                    MergeFixture {
                        name: prefix.to_string(),
                        base_path: path.clone(),
                        contrib1_path,
                        contrib2_path,
                        expected_path,
                    },
                );
            }
        }
    }

    fixtures_by_name.into_values().collect()
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
fn validate_invariants(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    fixture_name: &str,
) {
    validate_marker_wellformedness(output, fixture_name);
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
    let base_lines: std::collections::HashSet<&str> = base.lines().collect();
    let contrib1_lines: std::collections::HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: std::collections::HashSet<&str> = contrib2.lines().collect();

    for (line_num, line) in output.lines().enumerate() {
        let trimmed = line.trim_end();
        // Skip conflict markers
        if is_open_marker(trimmed)
            || is_close_marker(trimmed)
            || is_separator_marker(trimmed)
            || is_base_marker(trimmed)
        {
            continue;
        }

        let line_content = line;
        assert!(
            base_lines.contains(line_content)
                || contrib1_lines.contains(line_content)
                || contrib2_lines.contains(line_content),
            "[{}] line {}: output line {:?} not found in any input",
            fixture_name,
            line_num + 1,
            line_content
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
    let contrib1_lines: std::collections::HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: std::collections::HashSet<&str> = contrib2.lines().collect();
    let output_lines: std::collections::HashSet<&str> = output.lines().collect();

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
        let base = std::fs::read_to_string(&fixture.base_path)
            .unwrap_or_else(|e| panic!("[{}] failed to read base: {}", fixture.name, e));
        let contrib1 = std::fs::read_to_string(&fixture.contrib1_path)
            .unwrap_or_else(|e| panic!("[{}] failed to read contrib1: {}", fixture.name, e));
        let contrib2 = std::fs::read_to_string(&fixture.contrib2_path)
            .unwrap_or_else(|e| panic!("[{}] failed to read contrib2: {}", fixture.name, e));
        let expected = std::fs::read_to_string(&fixture.expected_path)
            .unwrap_or_else(|e| panic!("[{}] failed to read expected_result: {}", fixture.name, e));

        let options = MergeOptions::default();
        let result = merge_file(&base, &contrib1, &contrib2, &options);

        // Run invariant checks (these panic on failure).
        validate_invariants(&base, &contrib1, &contrib2, &result.output, &fixture.name);

        // Compare against expected result if the expected file is non-empty.
        if !expected.is_empty() {
            if result.output == expected {
                pass_count += 1;
            } else {
                fail_count += 1;

                // Write actual result for manual comparison.
                let actual_path = fixture.expected_path.with_file_name(format!(
                    "{}_actual_result{}",
                    fixture.name,
                    fixture
                        .base_path
                        .extension()
                        .map(|e| format!(".{}", e.to_string_lossy()))
                        .unwrap_or_default()
                ));
                let _ = std::fs::write(&actual_path, &result.output);

                failures.push(format!(
                    "[{}] output mismatch (actual written to {})\n  expected:\n{}\n  actual:\n{}",
                    fixture.name,
                    actual_path.display(),
                    indent_text(&expected),
                    indent_text(&result.output),
                ));
            }
        } else {
            // Empty expected file — only invariant checks were run.
            pass_count += 1;
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

fn run_single_fixture(name: &str) {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge");
    let fixtures = discover_fixtures(&fixtures_dir);
    let fixture = fixtures
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("Fixture {:?} not found", name));

    let base = std::fs::read_to_string(&fixture.base_path).unwrap();
    let contrib1 = std::fs::read_to_string(&fixture.contrib1_path).unwrap();
    let contrib2 = std::fs::read_to_string(&fixture.contrib2_path).unwrap();
    let expected = std::fs::read_to_string(&fixture.expected_path).unwrap();

    let result = merge_file(&base, &contrib1, &contrib2, &MergeOptions::default());

    validate_invariants(&base, &contrib1, &contrib2, &result.output, name);

    if !expected.is_empty() {
        assert_eq!(
            result.output, expected,
            "[{}] merge output does not match expected result",
            name
        );
    }
}

fn indent_text(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}
