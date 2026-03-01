//! Tests ported from git's t6403-merge-file.sh and t6427-diff3-conflict-markers.sh.
//!
//! These verify the core 3-way merge algorithm against git's merge-file
//! behavior as specified in the Reference Test Portability Plan.

use gitgpui_core::merge::{
    ConflictStyle, DiffAlgorithm, MergeError, MergeLabels, MergeOptions, MergeStrategy, merge_file,
    merge_file_bytes,
};

fn default_opts() -> MergeOptions {
    MergeOptions::default()
}

fn opts_with_labels(ours: &str, theirs: &str) -> MergeOptions {
    MergeOptions {
        labels: MergeLabels {
            ours: Some(ours.to_string()),
            base: None,
            theirs: Some(theirs.to_string()),
        },
        ..Default::default()
    }
}

fn opts_strategy(strategy: MergeStrategy) -> MergeOptions {
    MergeOptions {
        strategy,
        ..Default::default()
    }
}

fn opts_style(style: ConflictStyle) -> MergeOptions {
    MergeOptions {
        style,
        ..Default::default()
    }
}

fn opts_zdiff3_with_labels(ours: &str, base: &str, theirs: &str) -> MergeOptions {
    MergeOptions {
        style: ConflictStyle::Zdiff3,
        labels: MergeLabels {
            ours: Some(ours.to_string()),
            base: Some(base.to_string()),
            theirs: Some(theirs.to_string()),
        },
        ..Default::default()
    }
}

fn marker_count(output: &str, marker: &str) -> usize {
    output
        .lines()
        .filter(|line| line.starts_with(marker))
        .count()
}

// ===========================================================================
// Psalm 23 fixtures (from t6403-merge-file.sh)
// ===========================================================================

const PSALM_BASE: &str = "\
Dominus regit me,
et nihil mihi deerit.
In loco pascuae ibi me collocavit,
super aquam refectionis educavit me;
animam meam convertit,
deduxit me super semitas jusitiae,
propter nomen suum.
";

/// new1: base + 3 appended lines.
const PSALM_NEW1: &str = "\
Dominus regit me,
et nihil mihi deerit.
In loco pascuae ibi me collocavit,
super aquam refectionis educavit me;
animam meam convertit,
deduxit me super semitas jusitiae,
propter nomen suum.
Nam et si ambulavero in medio umbrae mortis,
non timebo mala, quoniam tu mecum es:
virga tua et baculus tuus ipsa me consolata sunt.
";

/// new2: first two lines collapsed into one.
const PSALM_NEW2: &str = "\
Dominus regit me, et nihil mihi deerit.
In loco pascuae ibi me collocavit,
super aquam refectionis educavit me;
animam meam convertit,
deduxit me super semitas jusitiae,
propter nomen suum.
";

/// new3: first word uppercased to DOMINUS.
const PSALM_NEW3: &str = "\
DOMINUS regit me,
et nihil mihi deerit.
In loco pascuae ibi me collocavit,
super aquam refectionis educavit me;
animam meam convertit,
deduxit me super semitas jusitiae,
propter nomen suum.
";

/// new4: new2 + 3 appended lines + "tu" -> "TU".
const PSALM_NEW4: &str = "\
Dominus regit me, et nihil mihi deerit.
In loco pascuae ibi me collocavit,
super aquam refectionis educavit me;
animam meam convertit,
deduxit me super semitas jusitiae,
propter nomen suum.
Nam et si ambulavero in medio umbrae mortis,
non timebo mala, quoniam TU mecum es:
virga tua et baculus tuus ipsa me consolata sunt.
";

// ===========================================================================
// Phase 1A: t6403 merge-file algorithm-focused tests
// ===========================================================================

// ── Identity and clean merge ──

#[test]
fn t6403_merge_identity() {
    let result = merge_file(PSALM_BASE, PSALM_BASE, PSALM_BASE, &default_opts());
    assert!(result.is_clean(), "identity merge should be clean");
    assert_eq!(result.output, PSALM_BASE);
}

#[test]
fn t6403_merge_nonoverlapping_clean() {
    // new1 (appended lines) vs new2 (collapsed first line): disjoint changes.
    let result = merge_file(PSALM_BASE, PSALM_NEW1, PSALM_NEW2, &default_opts());
    assert!(
        result.is_clean(),
        "non-overlapping changes should merge cleanly"
    );
    // The merged result should have new2's collapsed first line and new1's appended lines.
    assert!(
        result
            .output
            .contains("Dominus regit me, et nihil mihi deerit.")
    );
    assert!(result.output.contains("Nam et si ambulavero"));
    assert!(result.output.contains("virga tua et baculus tuus"));
}

// ── Conflict detection and marker format ──

#[test]
fn t6403_merge_overlapping_conflict() {
    // new2 (collapsed first line) vs new3 (DOMINUS): overlapping changes at top.
    let result = merge_file(PSALM_BASE, PSALM_NEW2, PSALM_NEW3, &default_opts());
    assert!(!result.is_clean(), "overlapping changes should conflict");
    assert!(result.output.contains("<<<<<<<"));
    assert!(result.output.contains("======="));
    assert!(result.output.contains(">>>>>>>"));
    // Local (new2) section should have the collapsed line.
    assert!(
        result
            .output
            .contains("Dominus regit me, et nihil mihi deerit.")
    );
    // Remote (new3) section should have the uppercased word.
    assert!(result.output.contains("DOMINUS regit me,"));
}

#[test]
fn t6403_merge_conflict_markers_with_labels() {
    let opts = opts_with_labels("new2.txt", "new3.txt");
    let result = merge_file(PSALM_BASE, PSALM_NEW2, PSALM_NEW3, &opts);
    assert!(!result.is_clean());
    assert!(
        result.output.contains("<<<<<<< new2.txt"),
        "ours label should appear"
    );
    assert!(
        result.output.contains(">>>>>>> new3.txt"),
        "theirs label should appear"
    );
}

#[test]
fn t6403_merge_delete_vs_modify_conflict() {
    // new1 has 3 appended lines. local deletes them, remote modifies "tu" → "TU".
    let result = merge_file(PSALM_NEW1, PSALM_BASE, PSALM_NEW4, &default_opts());
    assert!(
        !result.is_clean(),
        "delete vs modify should produce conflict"
    );
}

// ── Conflict resolution strategies ──

#[test]
fn t6403_merge_ours() {
    let result = merge_file(
        PSALM_BASE,
        PSALM_NEW2,
        PSALM_NEW3,
        &opts_strategy(MergeStrategy::Ours),
    );
    assert!(result.is_clean());
    assert!(
        result
            .output
            .contains("Dominus regit me, et nihil mihi deerit.")
    );
    assert!(!result.output.contains("DOMINUS"));
}

#[test]
fn t6403_merge_theirs() {
    let result = merge_file(
        PSALM_BASE,
        PSALM_NEW2,
        PSALM_NEW3,
        &opts_strategy(MergeStrategy::Theirs),
    );
    assert!(result.is_clean());
    assert!(result.output.contains("DOMINUS regit me,"));
    // Theirs picked new3's version: separate lines, not the collapsed form.
    assert!(
        !result.output.contains("Dominus regit me, et nihil"),
        "should not contain ours' collapsed line"
    );
}

#[test]
fn t6403_merge_union() {
    let result = merge_file(
        PSALM_BASE,
        PSALM_NEW2,
        PSALM_NEW3,
        &opts_strategy(MergeStrategy::Union),
    );
    assert!(result.is_clean());
    // Both sides should be present.
    assert!(
        result
            .output
            .contains("Dominus regit me, et nihil mihi deerit.")
    );
    assert!(result.output.contains("DOMINUS regit me,"));
}

// ── Trailing newline / EOF edge cases ──

#[test]
fn t6403_merge_preserves_missing_lf() {
    // When ours lacks trailing LF and theirs changes are far from EOF,
    // output should preserve absence of trailing LF.
    let base = "aaa\nbbb\nccc";
    let ours = "aaa\nbbb\nccc";
    let theirs = "AAA\nbbb\nccc";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert!(!result.output.ends_with('\n'), "should not add trailing LF");
}

#[test]
fn t6403_merge_no_spurious_lf() {
    // Both modified, no trailing newline.
    let base = "a\nb\nc";
    let ours = "a\nb\nc";
    let theirs = "a\nB\nc";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert!(
        !result.output.ends_with('\n'),
        "output should end without newline"
    );
}

// ── CRLF handling ──

#[test]
fn t6403_merge_crlf_conflict_markers() {
    let base = "1\r\n2\r\n3\r\n";
    let ours = "1\r\n2\r\n4\r\n";
    let theirs = "1\r\n2\r\n5\r\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(!result.is_clean());
    assert!(result.output.contains("<<<<<<<\r\n"));
    assert!(result.output.contains("=======\r\n"));
    assert!(result.output.contains(">>>>>>>\r\n"));
}

#[test]
fn t6403_merge_lf_conflict_markers() {
    let base = "1\n2\n3\n";
    let ours = "1\n2\n4\n";
    let theirs = "1\n2\n5\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(!result.is_clean());
    assert!(result.output.contains("<<<<<<<\n"));
    assert!(!result.output.contains("\r\n"));
}

// ── Zealous conflict coalescing ──

#[test]
fn t6403_merge_zealous_coalesces_adjacent_conflict_lines() {
    // Consecutive conflicting edits should render as one conflict hunk.
    let base = "a\nb\nc\n";
    let ours = "A\nB\nc\n";
    let theirs = "X\nY\nc\n";
    let result = merge_file(base, ours, theirs, &default_opts());

    assert!(!result.is_clean());
    assert_eq!(
        marker_count(&result.output, "======="),
        1,
        "adjacent conflicting lines should coalesce into one conflict block:\n{}",
        result.output
    );
}

#[test]
fn t6403_merge_zealous_alnum_coalesces_across_blank_separator() {
    // Two conflict regions separated only by blank context are coalesced.
    let base = "alpha\n\nbeta\ngamma\n";
    let ours = "ALPHA\n\nBETA_OURS\ngamma\n";
    let theirs = "ALPHA_THEIRS\n\nBETA_THEIRS\ngamma\n";
    let result = merge_file(base, ours, theirs, &default_opts());

    assert!(!result.is_clean());
    assert_eq!(
        marker_count(&result.output, "======="),
        1,
        "blank-only context should be absorbed into a single zealous conflict block:\n{}",
        result.output
    );
}

#[test]
fn t6403_merge_zealous_keeps_nonblank_separated_conflicts_split() {
    // Non-blank context between conflicts should keep them as separate hunks.
    let base = "alpha\ncontext\nbeta\ngamma\n";
    let ours = "ALPHA\ncontext\nBETA_OURS\ngamma\n";
    let theirs = "ALPHA_THEIRS\ncontext\nBETA_THEIRS\ngamma\n";
    let result = merge_file(base, ours, theirs, &default_opts());

    assert!(!result.is_clean());
    assert_eq!(
        marker_count(&result.output, "======="),
        2,
        "non-blank context should keep conflict blocks distinct:\n{}",
        result.output
    );
}

// ── Configurable marker width ──

#[test]
fn t6403_merge_marker_size_10() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let opts = MergeOptions {
        marker_size: 10,
        ..Default::default()
    };
    let result = merge_file(base, ours, theirs, &opts);
    assert!(result.output.contains("<<<<<<<<<<\n"));
    assert!(result.output.contains("==========\n"));
    assert!(result.output.contains(">>>>>>>>>>\n"));
}

// ── Diff3 style ──

#[test]
fn t6403_merge_diff3_output() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, ours, theirs, &opts_style(ConflictStyle::Diff3));
    assert!(!result.is_clean());
    assert!(result.output.contains("|||||||"), "should have base marker");
    assert!(
        result.output.contains("bbb"),
        "base content should be shown"
    );
}

// ── Diff algorithm impact: Myers vs Histogram ──

const BASE_C: &str = "\
int f(int x, int y)
{
\tif (x == 0)
\t{
\t\treturn y;
\t}
\treturn x;
}

int g(size_t u)
{
\twhile (u < 30)
\t{
\t\tu++;
\t}
\treturn u;
}
";

const OURS_C: &str = "\
int g(size_t u)
{
\twhile (u < 30)
\t{
\t\tu++;
\t}
\treturn u;
}

int h(int x, int y, int z)
{
\tif (z == 0)
\t{
\t\treturn x;
\t}
\treturn y;
}
";

const THEIRS_C: &str = "\
int f(int x, int y)
{
\tif (x == 0)
\t{
\t\treturn y;
\t}
\treturn x;
}

int g(size_t u)
{
\twhile (u > 34)
\t{
\t\tu--;
\t}
\treturn u;
}
";

#[test]
fn t6403_merge_myers_c_code_has_spurious_conflicts() {
    // With Myers diff, this produces spurious conflicts because Myers
    // greedily matches common structural tokens (braces, returns) across
    // different functions. The merge detects the g() body change as a
    // conflict but also drags in unrelated hunks.
    let result = merge_file(BASE_C, OURS_C, THEIRS_C, &default_opts());
    assert!(
        !result.is_clean(),
        "Myers diff should produce conflicts on this C code"
    );
    // The g() body modifications should be in the conflict.
    assert!(result.output.contains("u < 30") || result.output.contains("u > 34"));
}

// ── Binary detection ──

#[test]
fn t6403_merge_binary_rejected() {
    // merge_file_bytes rejects inputs containing null bytes.
    let png_header: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let text = b"text content\n";

    // Binary base.
    assert_eq!(
        merge_file_bytes(png_header, text, text, &default_opts()),
        Err(MergeError::BinaryContent),
        "binary base should be rejected"
    );

    // Binary ours.
    assert_eq!(
        merge_file_bytes(text, png_header, text, &default_opts()),
        Err(MergeError::BinaryContent),
        "binary ours should be rejected"
    );

    // Binary theirs.
    assert_eq!(
        merge_file_bytes(text, text, png_header, &default_opts()),
        Err(MergeError::BinaryContent),
        "binary theirs should be rejected"
    );

    // All text should succeed.
    let result = merge_file_bytes(text, text, text, &default_opts());
    assert!(result.is_ok(), "all-text inputs should succeed");
    assert!(result.unwrap().is_clean());
}

#[test]
fn t6403_merge_binary_null_byte_in_utf8() {
    // Even valid UTF-8 strings with null bytes should be rejected by
    // merge_file_bytes, matching git's binary detection heuristic.
    let with_null = b"text\x00more\n";
    let clean = b"clean text\n";

    assert_eq!(
        merge_file_bytes(with_null, clean, clean, &default_opts()),
        Err(MergeError::BinaryContent),
    );
}

#[test]
fn t6403_merge_binary_content_text_api_no_panic() {
    // The text-based merge_file API doesn't reject null bytes (they're
    // valid UTF-8) but should not panic.
    let base = "text\0binary\n";
    let ours = "text\0binary\n";
    let theirs = "text\0CHANGED\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean() || !result.is_clean());
}

// ── Identical changes across both sides ──

#[test]
fn t6403_merge_both_sides_identical_change() {
    let base = "aaa\nbbb\nccc\n";
    let changed = "aaa\nXXX\nccc\n";
    let result = merge_file(base, changed, changed, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, changed);
}

// ── Only one side changes ──

#[test]
fn t6403_merge_only_ours_changed() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let result = merge_file(base, ours, base, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, ours);
}

#[test]
fn t6403_merge_only_theirs_changed() {
    let base = "aaa\nbbb\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, base, theirs, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, theirs);
}

// ===========================================================================
// Phase 1B: t6427 zdiff3 test cases
// ===========================================================================

#[test]
fn t6427_zdiff3_basic() {
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
    let opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    let result = merge_file(base, ours, theirs, &opts);

    assert!(!result.is_clean());
    assert_eq!(result.conflict_count, 1);

    // Common prefix "A" and suffix "E" should be OUTSIDE the conflict markers.
    let marker_start = result
        .output
        .find("<<<<<<< HEAD")
        .expect("should have ours marker");
    let marker_end = result
        .output
        .find(">>>>>>> right")
        .expect("should have theirs marker");

    // "A\n" should appear before the opening marker.
    let before_markers = &result.output[..marker_start];
    assert!(
        before_markers.ends_with("A\n"),
        "common prefix 'A' should be extracted before conflict markers.\nBefore markers: {:?}",
        before_markers
    );

    // "E\n" should appear after the closing marker.
    let after_marker_line_end = result.output[marker_end..].find('\n').unwrap() + marker_end + 1;
    let after_markers = &result.output[after_marker_line_end..];
    assert!(
        after_markers.starts_with("E\n"),
        "common suffix 'E' should be extracted after conflict markers.\nAfter markers: {:?}",
        after_markers
    );

    // The conflict region should contain the differing middle parts.
    let conflict_region = &result.output[marker_start..after_marker_line_end];
    assert!(
        conflict_region.contains("B\nC\nD"),
        "ours conflict should contain B/C/D"
    );
    assert!(
        conflict_region.contains("X\nC\nY"),
        "theirs conflict should contain X/C/Y"
    );
}

#[test]
fn t6427_zdiff3_middle_common() {
    // Two disjoint change regions with common "4\n5\n" between them.
    let base = "1\n2\n3\nAA\n4\n5\nBB\n6\n7\n8\n";
    let ours = "1\n2\n3\nCC\n4\n5\nDD\n6\n7\n8\n";
    let theirs = "1\n2\n3\nEE\n4\n5\nFF\n6\n7\n8\n";
    let opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    let result = merge_file(base, ours, theirs, &opts);

    assert!(!result.is_clean());
    assert_eq!(
        result.conflict_count, 2,
        "should be two separate conflict hunks"
    );

    // Both CC/EE and DD/FF should be in separate conflicts.
    assert!(result.output.contains("CC"));
    assert!(result.output.contains("EE"));
    assert!(result.output.contains("DD"));
    assert!(result.output.contains("FF"));

    // The common "4\n5\n" should be preserved between conflicts as resolved context.
    let first_close = result.output.find(">>>>>>> right").unwrap();
    let second_open = result.output[first_close..].find("<<<<<<< HEAD").unwrap() + first_close;
    let between = &result.output[first_close..second_open];
    assert!(
        between.contains("4\n5\n"),
        "common material '4\\n5\\n' should be preserved between conflicts"
    );
}

#[test]
fn t6427_zdiff3_interesting() {
    // Left adds D/E/F then G/H/I/J; right adds 5/6 then G/H/I/J.
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\nF\nG\nH\nI\nJ\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nA\nB\nC\n5\n6\nG\nH\nI\nJ\n7\n8\n9\n";
    let opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    let result = merge_file(base, ours, theirs, &opts);

    assert!(!result.is_clean());

    // Common prefix "A\nB\nC\n" should be extracted.
    let marker_start = result.output.find("<<<<<<< HEAD").unwrap();
    let before = &result.output[..marker_start];
    assert!(
        before.contains("A\nB\nC\n"),
        "common prefix A/B/C should be before markers"
    );

    // Common suffix "G\nH\nI\nJ\n" should be extracted.
    let marker_end_line = result.output.find(">>>>>>> right").unwrap();
    let line_end = result.output[marker_end_line..].find('\n').unwrap() + marker_end_line + 1;
    let after = &result.output[line_end..];
    assert!(
        after.starts_with("G\nH\nI\nJ\n"),
        "common suffix G/H/I/J should be after markers.\nActual after: {:?}",
        &after[..after.len().min(40)]
    );
}

#[test]
fn t6427_zdiff3_evil() {
    // Tricky case with common trailing "B\nC\n".
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nX\nA\nB\nC\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nY\nA\nB\nC\nB\nC\n7\n8\n9\n";
    let opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    let result = merge_file(base, ours, theirs, &opts);

    assert!(!result.is_clean());

    // "B\nC\n" should appear after the conflict markers as common suffix.
    let marker_end_line = result.output.find(">>>>>>> right").unwrap();
    let line_end = result.output[marker_end_line..].find('\n').unwrap() + marker_end_line + 1;
    let after = &result.output[line_end..];
    assert!(
        after.starts_with("B\nC\n"),
        "common suffix B/C should be extracted after markers.\nActual after: {:?}",
        &after[..after.len().min(40)]
    );
}

// ===========================================================================
// Additional edge cases from design doc
// ===========================================================================

#[test]
fn merge_empty_base_both_add_same() {
    let result = merge_file("", "new content\n", "new content\n", &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "new content\n");
}

#[test]
fn merge_empty_base_both_add_different() {
    let result = merge_file("", "ours\n", "theirs\n", &default_opts());
    assert!(!result.is_clean());
}

#[test]
fn merge_multiple_nonoverlapping_changes() {
    let base = "a\nb\nc\nd\ne\nf\ng\n";
    let ours = "A\nb\nc\nd\ne\nf\nG\n";
    let theirs = "a\nb\nC\nd\nE\nf\ng\n";
    let result = merge_file(base, ours, theirs, &default_opts());
    assert!(result.is_clean());
    assert_eq!(result.output, "A\nb\nC\nd\nE\nf\nG\n");
}

#[test]
fn merge_diff3_marker_size_10() {
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let opts = MergeOptions {
        style: ConflictStyle::Diff3,
        marker_size: 10,
        ..Default::default()
    };
    let result = merge_file(base, ours, theirs, &opts);
    assert!(result.output.contains("<<<<<<<<<<\n"));
    assert!(result.output.contains("||||||||||\n"));
    assert!(result.output.contains("==========\n"));
    assert!(result.output.contains(">>>>>>>>>>\n"));
}

#[test]
fn merge_ours_strategy_at_eof() {
    // Conflict at EOF without trailing LF.
    let base = "aaa\nbbb";
    let ours = "aaa\nOURS";
    let theirs = "aaa\nTHEIRS";
    let result = merge_file(base, ours, theirs, &opts_strategy(MergeStrategy::Ours));
    assert!(result.is_clean());
    assert!(result.output.contains("OURS"));
    assert!(!result.output.contains("THEIRS"));
}

#[test]
fn merge_theirs_strategy_at_eof() {
    let base = "aaa\nbbb";
    let ours = "aaa\nOURS";
    let theirs = "aaa\nTHEIRS";
    let result = merge_file(base, ours, theirs, &opts_strategy(MergeStrategy::Theirs));
    assert!(result.is_clean());
    assert!(result.output.contains("THEIRS"));
    assert!(!result.output.contains("OURS"));
}

#[test]
fn merge_union_strategy_at_eof() {
    let base = "aaa\nbbb";
    let ours = "aaa\nOURS";
    let theirs = "aaa\nTHEIRS";
    let result = merge_file(base, ours, theirs, &opts_strategy(MergeStrategy::Union));
    assert!(result.is_clean());
    assert!(result.output.contains("OURS"));
    assert!(result.output.contains("THEIRS"));
}

// ===========================================================================
// Diff algorithm impact: Myers vs Histogram
// ===========================================================================

fn opts_histogram() -> MergeOptions {
    MergeOptions {
        diff_algorithm: DiffAlgorithm::Histogram,
        ..Default::default()
    }
}

#[test]
fn t6403_merge_histogram_clean() {
    // The histogram/patience algorithm anchors on unique function signatures
    // rather than common structural tokens (braces, returns). This produces
    // a clean merge for the C code test case where Myers creates spurious
    // conflicts.
    //
    // base: f() then g()
    // ours: deletes f(), keeps g(), adds h()
    // theirs: keeps f(), modifies g() body
    //
    // With histogram: f() deletion and h() addition don't overlap with
    // the g() body modification, so merge is clean.
    let result = merge_file(BASE_C, OURS_C, THEIRS_C, &opts_histogram());
    assert!(
        result.is_clean(),
        "histogram diff should produce a clean merge for the C code test case.\n\
         Output:\n{}",
        result.output
    );
    // The merged result should contain the modified g() body from theirs.
    assert!(
        result.output.contains("u > 34"),
        "merged output should contain theirs' g() body change"
    );
    assert!(
        result.output.contains("u--"),
        "merged output should contain theirs' g() body decrement"
    );
    // The merged result should contain h() from ours.
    assert!(
        result.output.contains("int h(int x, int y, int z)"),
        "merged output should contain ours' h() function"
    );
    // f() should be deleted (not present in ours).
    assert!(
        !result.output.contains("int f(int x, int y)"),
        "f() should be deleted in merged output"
    );
}

#[test]
fn t6403_merge_histogram_identity() {
    // Histogram algorithm should still handle identity merges.
    let text = "line1\nline2\nline3\n";
    let result = merge_file(text, text, text, &opts_histogram());
    assert!(result.is_clean());
    assert_eq!(result.output, text);
}

#[test]
fn t6403_merge_histogram_nonoverlapping() {
    // Histogram should handle non-overlapping changes cleanly.
    let base = "aaa\nbbb\nccc\n";
    let ours = "AAA\nbbb\nccc\n";
    let theirs = "aaa\nbbb\nCCC\n";
    let result = merge_file(base, ours, theirs, &opts_histogram());
    assert!(result.is_clean());
    assert_eq!(result.output, "AAA\nbbb\nCCC\n");
}

#[test]
fn t6403_merge_histogram_conflict() {
    // Histogram should still detect true conflicts.
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nOURS\nccc\n";
    let theirs = "aaa\nTHEIRS\nccc\n";
    let result = merge_file(base, ours, theirs, &opts_histogram());
    assert!(!result.is_clean());
    assert_eq!(result.conflict_count, 1);
}
