//! Output invariants shared by the merge integration suites.
#![allow(dead_code)] // each suite uses a subset

use std::collections::HashSet;

/// Markers must be balanced and ordered: `<<<<<<<` [`|||||||`] `=======` `>>>>>>>`.
pub fn validate_marker_wellformedness(output: &str, case: &str) {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum State {
        Outside,
        InOurs,
        InBase,
        InTheirs,
    }

    let mut state = State::Outside;
    let mut conflict_count = 0u32;
    for (line_ix, line) in output.lines().enumerate() {
        let line = line.trim_end();
        let line_no = line_ix + 1;
        if is_open_marker(line) {
            assert_eq!(
                state,
                State::Outside,
                "[{case}] line {line_no}: unexpected <<<<<<< (already inside conflict)"
            );
            state = State::InOurs;
            conflict_count += 1;
        } else if is_base_marker(line) {
            assert_eq!(
                state,
                State::InOurs,
                "[{case}] line {line_no}: unexpected ||||||| (expected inside ours section)"
            );
            state = State::InBase;
        } else if is_separator_marker(line) {
            assert!(
                state == State::InOurs || state == State::InBase,
                "[{case}] line {line_no}: unexpected ======= (expected after <<<<<<< or |||||||)"
            );
            state = State::InTheirs;
        } else if is_close_marker(line) {
            assert_eq!(
                state,
                State::InTheirs,
                "[{case}] line {line_no}: unexpected >>>>>>> (expected after =======)"
            );
            state = State::Outside;
        }
    }

    assert_eq!(
        state,
        State::Outside,
        "[{case}] unclosed conflict markers ({conflict_count} conflicts opened)"
    );
}

/// Every non-marker output line comes from at least one input.
pub fn validate_content_integrity(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case: &str,
) {
    let base_lines: HashSet<&str> = base.lines().collect();
    let contrib1_lines: HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: HashSet<&str> = contrib2.lines().collect();

    for (line_ix, line) in output.lines().enumerate() {
        if is_marker(line.trim_end()) {
            continue;
        }
        assert!(
            base_lines.contains(line)
                || contrib1_lines.contains(line)
                || contrib2_lines.contains(line),
            "[{case}] line {}: output line {line:?} not found in any input",
            line_ix + 1
        );
    }
}

/// Lines common to all three inputs appear in the output.
pub fn validate_context_preservation(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case: &str,
) {
    let contrib1_lines: HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: HashSet<&str> = contrib2.lines().collect();
    let output_lines: HashSet<&str> = output.lines().collect();

    for line in base.lines() {
        if contrib1_lines.contains(line) && contrib2_lines.contains(line) {
            assert!(
                output_lines.contains(line),
                "[{case}] line {line:?} is common to all three inputs but missing from output"
            );
        }
    }
}

/// Marker-looking input lines make the output's marker structure ambiguous.
pub fn contains_marker_like_line(text: &str) -> bool {
    text.lines().any(|line| is_marker(line.trim_end()))
}

pub fn count_open_markers(output: &str) -> usize {
    output
        .lines()
        .filter(|line| is_open_marker(line.trim_end()))
        .count()
}

fn is_marker(line: &str) -> bool {
    is_open_marker(line)
        || is_base_marker(line)
        || is_separator_marker(line)
        || is_close_marker(line)
}

fn is_open_marker(line: &str) -> bool {
    is_labelled_marker(line, "<<<<<<<", '<')
}

fn is_base_marker(line: &str) -> bool {
    is_labelled_marker(line, "|||||||", '|')
}

fn is_close_marker(line: &str) -> bool {
    is_labelled_marker(line, ">>>>>>>", '>')
}

fn is_separator_marker(line: &str) -> bool {
    line.starts_with("=======") && line[7..].chars().all(|c| c == '=')
}

/// The marker, then more `fill` characters or a label of path-like characters.
fn is_labelled_marker(line: &str, marker: &str, fill: char) -> bool {
    let Some(label) = line.strip_prefix(marker) else {
        return false;
    };
    label
        .chars()
        .all(|c| c == fill || c == ' ' || c.is_alphanumeric() || "/.:-_".contains(c))
}
