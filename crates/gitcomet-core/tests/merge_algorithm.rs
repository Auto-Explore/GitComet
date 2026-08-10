//! Tests ported from git's t6403-merge-file.sh and t6427-diff3-conflict-markers.sh.
//!
//! These verify the core 3-way merge algorithm against git's merge-file
//! behavior as specified in the Reference Test Portability Plan.

use gitcomet_core::merge::{
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
    assert_eq!(
        result.conflict_count, 1,
        "expected one delete-vs-modify conflict block:\n{}",
        result.output
    );
    // Local side deletes the appended tail, so the local section is empty.
    assert!(
        result.output.contains("<<<<<<<\n=======\n"),
        "expected empty local section in delete-vs-modify conflict:\n{}",
        result.output
    );
    // Remote section should contain the modified uppercase line, not the base line.
    assert!(
        result
            .output
            .contains("non timebo mala, quoniam TU mecum es:"),
        "expected modified remote content in conflict:\n{}",
        result.output
    );
    assert!(
        !result
            .output
            .contains("non timebo mala, quoniam tu mecum es:"),
        "did not expect unmodified base line in conflict output:\n{}",
        result.output
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
fn t6403_merge_missing_lf_at_eof() {
    // Git t6403: test_expect_failure "merge without conflict (missing LF at EOF)"
    //
    // remote (theirs) lacks trailing LF while the head-of-file change is
    // non-overlapping with ours' tail-of-file change.  Git's merge-file
    // currently fails on this case; our implementation does better.
    //
    // base: full psalm (trailing LF)
    // ours: psalm + 3 appended lines at end (trailing LF)
    // theirs: collapsed first line, same body, NO trailing LF
    let theirs_no_lf = PSALM_NEW2.trim_end_matches('\n');
    let result = merge_file(PSALM_BASE, PSALM_NEW1, theirs_no_lf, &default_opts());

    // Non-overlapping changes: ours adds lines at end, theirs changes first
    // line. The merge should succeed (improvement over git's expected-failure).
    assert!(
        result.is_clean(),
        "missing-LF-at-EOF merge should succeed (git expected-failure, we do better).\nOutput:\n{}",
        result.output
    );

    // Merged output should contain both sides' changes.
    assert!(
        result
            .output
            .contains("Dominus regit me, et nihil mihi deerit."),
        "should contain theirs' collapsed first line"
    );
    assert!(
        result.output.contains("Nam et si ambulavero"),
        "should contain ours' appended lines"
    );

    // The missing trailing LF from theirs should be preserved if the merge
    // algorithm respects the theirs-side EOF behavior. However, since ours
    // appends lines WITH trailing LF, the merged output will end with LF
    // (ours' appended lines end with newline).
    assert!(
        result.output.ends_with('\n'),
        "merged output should end with LF from ours' appended lines"
    );
}

#[test]
fn t6403_merge_missing_lf_at_eof_away_from_change() {
    // Git t6403: "merge without conflict (missing LF at EOF, away from change)"
    //
    // ours lacks trailing LF, theirs changes first word (at head, far from EOF).
    // Merged output should preserve the missing trailing LF.
    //
    // base: collapsed first line (PSALM_NEW2) with trailing LF
    // ours: same as base but WITHOUT trailing LF (PSALM_NEW4-like, no appended lines)
    // theirs: DOMINUS uppercased (PSALM_NEW3) with trailing LF
    let base_collapsed = PSALM_NEW2;
    let ours_no_lf = PSALM_NEW2.trim_end_matches('\n');

    // theirs changes first word relative to base_collapsed:
    // base: "Dominus regit me, et nihil mihi deerit."
    // theirs needs first word uppercased. Since base is collapsed form,
    // create theirs manually.
    let theirs = base_collapsed.replacen("Dominus", "DOMINUS", 1);

    let result = merge_file(base_collapsed, ours_no_lf, &theirs, &default_opts());

    assert!(
        result.is_clean(),
        "missing-LF away from change should merge cleanly.\nOutput:\n{}",
        result.output
    );
    assert!(
        result.output.contains("DOMINUS"),
        "should contain theirs' uppercased word"
    );
    assert!(
        !result.output.ends_with('\n'),
        "should preserve ours' missing trailing LF"
    );
}

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
fn kdiff3_grouping_keeps_blank_separated_conflicts_split() {
    // KDiff3 groups adjacent rows of the same merge kind. Even blank,
    // unchanged context therefore remains outside the surrounding conflicts.
    let base = "alpha\n\nbeta\ngamma\n";
    let ours = "ALPHA\n\nBETA_OURS\ngamma\n";
    let theirs = "ALPHA_THEIRS\n\nBETA_THEIRS\ngamma\n";
    let result = merge_file(base, ours, theirs, &default_opts());

    assert!(!result.is_clean());
    assert_eq!(
        marker_count(&result.output, "======="),
        2,
        "unchanged blank context should separate KDiff3 merge blocks:\n{}",
        result.output
    );
    let first_close = result.output.find(">>>>>>>").expect("first close marker");
    let second_open = result.output[first_close..]
        .find("<<<<<<<")
        .map(|offset| first_close + offset)
        .expect("second open marker");
    assert!(
        result.output[first_close..second_open].contains("\n\n"),
        "the unchanged blank separator should remain between conflicts"
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
    let mut opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    opts.align_contributors = true;
    let result = merge_file(base, ours, theirs, &opts);

    assert!(!result.is_clean());
    assert_eq!(
        result.conflict_count, 2,
        "the shared contributor line C should split the two KDiff3 conflicts"
    );

    // Common A/C/E anchors should be outside the two conflict blocks.
    let first_marker_start = result
        .output
        .find("<<<<<<< HEAD")
        .expect("should have first ours marker");
    let first_marker_end = result
        .output
        .find(">>>>>>> right")
        .expect("should have first theirs marker");
    let second_marker_start = result.output[first_marker_end..]
        .find("<<<<<<< HEAD")
        .map(|offset| first_marker_end + offset)
        .expect("should have second ours marker");
    let second_marker_end = result.output[second_marker_start..]
        .find(">>>>>>> right")
        .map(|offset| second_marker_start + offset)
        .expect("should have second theirs marker");

    // "A\n" should appear before the opening marker.
    let before_markers = &result.output[..first_marker_start];
    assert!(
        before_markers.ends_with("A\n"),
        "common prefix 'A' should be extracted before conflict markers.\nBefore markers: {:?}",
        before_markers
    );

    let first_marker_line_end =
        result.output[first_marker_end..].find('\n').unwrap() + first_marker_end + 1;
    assert!(
        result.output[first_marker_line_end..second_marker_start].contains("C\n"),
        "shared contributor anchor C should remain between the conflicts"
    );

    // "E\n" should appear after the final closing marker.
    let after_marker_line_end =
        result.output[second_marker_end..].find('\n').unwrap() + second_marker_end + 1;
    let after_markers = &result.output[after_marker_line_end..];
    assert!(
        after_markers.starts_with("E\n"),
        "common suffix 'E' should be extracted after conflict markers.\nAfter markers: {:?}",
        after_markers
    );

    // The two conflict regions should contain their respective differing rows.
    let first_conflict = &result.output[first_marker_start..first_marker_line_end];
    assert!(
        first_conflict.contains("B\n") && first_conflict.contains("X\n"),
        "first conflict should contain B/X"
    );
    let second_conflict = &result.output[second_marker_start..after_marker_line_end];
    assert!(
        second_conflict.contains("D\n") && second_conflict.contains("Y\n"),
        "second conflict should contain D/Y"
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
    // Both contributors add the same surrounding lines, while only ours
    // replaces the ancestor's 5/6 with D/E/F. KDiff3 recognizes this as a
    // one-sided change and resolves it without a conflict.
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\nF\nG\nH\nI\nJ\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nA\nB\nC\n5\n6\nG\nH\nI\nJ\n7\n8\n9\n";
    let mut opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    opts.align_contributors = true;
    let result = merge_file(base, ours, theirs, &opts);

    assert!(result.is_clean());
    assert_eq!(result.output, ours);
}

#[test]
fn t6427_zdiff3_evil() {
    // Contributor alignment exposes the full common A/B/C run after the X/Y
    // conflict; the second B/C pair is a remote-only addition.
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let ours = "1\n2\n3\n4\nX\nA\nB\nC\n7\n8\n9\n";
    let theirs = "1\n2\n3\n4\nY\nA\nB\nC\nB\nC\n7\n8\n9\n";
    let mut opts = opts_zdiff3_with_labels("HEAD", "base", "right");
    opts.align_contributors = true;
    let result = merge_file(base, ours, theirs, &opts);

    assert!(!result.is_clean());

    // The shared A/B/C anchor and remote-only B/C addition should both appear
    // after the conflict rather than being swallowed by its marker range.
    let marker_end_line = result.output.find(">>>>>>> right").unwrap();
    let line_end = result.output[marker_end_line..].find('\n').unwrap() + marker_end_line + 1;
    let after = &result.output[line_end..];
    assert!(
        after.starts_with("A\nB\nC\nB\nC\n"),
        "aligned common and remote-only suffixes should follow the marker.\nActual after: {:?}",
        &after[..after.len().min(40)]
    );
}

/// Contributor alignment is not a nicety, it is what keeps the planner from
/// emitting a line twice, so it has to hold under the *default* options.
///
/// Base `aaa` matches ours' `    aaa` whitespace-insensitively and theirs'
/// `aaa` exactly, so the two base-relative diffs claim different contributor
/// lines for the same base line. With nothing to reconcile them, theirs'
/// leftover `    aaa` becomes a one-sided add and lands in the output on top of
/// ours' copy — a duplicated line in a merge reported as clean.
#[test]
fn merge_contributor_alignment_does_not_duplicate_shared_lines() {
    let result = merge_file("aaa\n", "bbb\n    aaa\n", "aaa\n    aaa\n", &default_opts());

    assert!(result.is_clean(), "output: {:?}", result.output);
    assert_eq!(result.output, "bbb\n    aaa\n");
    assert_eq!(
        result.output.matches("    aaa").count(),
        1,
        "the shared indented line must be emitted once: {:?}",
        result.output
    );
}

/// The same hazard with a whole added block: both sides add `same in b and c`
/// and `again same in b and c`, and only ours adds `only in b` between them.
#[test]
fn merge_contributor_alignment_pairs_shared_added_lines() {
    let result = merge_file(
        "same everywhere\n",
        "same in b and c\nonly in b\nagain same in b and c\nsame everywhere\n",
        "same in b and c\nagain same in b and c\nsame everywhere\n",
        &default_opts(),
    );

    assert!(result.is_clean(), "output: {:?}", result.output);
    assert_eq!(
        result.output,
        "same in b and c\nonly in b\nagain same in b and c\nsame everywhere\n"
    );
}

/// A one-sided change must merge identically however the flag is set: the pass
/// is skipped whenever any two inputs are equal, so it cannot reach these.
#[test]
fn merge_contributor_alignment_leaves_one_sided_merges_alone() {
    let base = "1\n2\n3\n4\n";
    let cases = [
        // Only ours changed.
        (base, "1\nX\n3\n4\n", base),
        // Only theirs changed.
        (base, base, "1\n2\nY\n4\n"),
        // Both sides made the identical change.
        (base, "1\nZ\n3\n4\n", "1\nZ\n3\n4\n"),
    ];

    for (base, ours, theirs) in cases {
        let mut aligned = default_opts();
        aligned.align_contributors = true;
        let mut unaligned = default_opts();
        unaligned.align_contributors = false;

        let with_pass = merge_file(base, ours, theirs, &aligned);
        let without_pass = merge_file(base, ours, theirs, &unaligned);
        assert_eq!(
            with_pass.output, without_pass.output,
            "one-sided merge of {ours:?} / {theirs:?} must not depend on the pass"
        );
        assert!(with_pass.is_clean(), "output: {:?}", with_pass.output);
    }
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
    // Git t6403 parity: conflict at EOF without trailing LF resolved by --ours
    // should preserve no-LF output exactly.
    let base = "line1\nline2\nline3";
    let ours = "line1\nline2\nline3x";
    let theirs = "line1\nline2\nline3y";
    let result = merge_file(base, ours, theirs, &opts_strategy(MergeStrategy::Ours));
    assert!(result.is_clean());
    assert_eq!(result.output, "line1\nline2\nline3x");
    assert!(!result.output.ends_with('\n'));
}

#[test]
fn merge_theirs_strategy_at_eof() {
    // Git t6403 parity: conflict at EOF without trailing LF resolved by --theirs
    // should preserve no-LF output exactly.
    let base = "line1\nline2\nline3";
    let ours = "line1\nline2\nline3x";
    let theirs = "line1\nline2\nline3y";
    let result = merge_file(base, ours, theirs, &opts_strategy(MergeStrategy::Theirs));
    assert!(result.is_clean());
    assert_eq!(result.output, "line1\nline2\nline3y");
    assert!(!result.output.ends_with('\n'));
}

#[test]
fn merge_union_strategy_at_eof() {
    // Git t6403 parity: --union keeps both sides with exactly one newline
    // separator and still no trailing LF at EOF.
    let base = "line1\nline2\nline3";
    let ours = "line1\nline2\nline3x";
    let theirs = "line1\nline2\nline3y";
    let result = merge_file(base, ours, theirs, &opts_strategy(MergeStrategy::Union));
    assert!(result.is_clean());
    assert_eq!(result.output, "line1\nline2\nline3x\nline3y");
    assert!(!result.output.ends_with('\n'));
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
