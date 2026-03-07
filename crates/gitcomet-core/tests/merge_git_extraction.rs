//! Phase 3C: Real-world merge extraction harness.
//!
//! Walks merge commits in a git repository, extracts base/contrib1/contrib2
//! file contents for each non-trivial merge, runs the merge algorithm on them,
//! and validates algorithm-independent invariants.
//!
//! Inspired by KDiff3's `generate_testdata_from_git_merges.py`.
//!
//! The default test runs against the gitcomet repository itself. An ignored
//! test demonstrates running against an external repository (e.g. linux kernel).
//!
//! These tests generate fixtures at test time — no file system bloat from
//! pre-committed merge data.
//!
//! Uses the production `merge_extraction` module API for extraction and fixture
//! writing, ensuring that the public API is integration-tested alongside the
//! merge algorithm invariants.

use gitcomet_core::merge::{MergeOptions, merge_file};
use gitcomet_core::merge_extraction::{
    ExtractedMergeCase, MergeExtractionOptions, discover_merge_commits, extract_merge_cases,
    extract_merge_cases_from_repo, write_fixture_files,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Invariant validation
// ---------------------------------------------------------------------------

/// Validate algorithm-independent invariants on merge output.
fn validate_merge_invariants(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case_name: &str,
) {
    // If any input already contains marker-looking lines, marker-structure
    // validation on the merged output is ambiguous (those lines can appear as
    // regular payload). In that case we still validate content integrity.
    let has_ambiguous_input_markers = [base, contrib1, contrib2]
        .iter()
        .any(|text| contains_marker_like_line(text));
    if !has_ambiguous_input_markers {
        validate_marker_wellformedness(output, case_name);
    }
    validate_content_integrity(base, contrib1, contrib2, output, case_name);
}

/// Conflict markers must be well-formed: balanced and properly ordered.
fn validate_marker_wellformedness(output: &str, case_name: &str) {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum State {
        Outside,
        InOurs,
        InBase,
        InTheirs,
    }

    let mut state = State::Outside;
    let mut conflict_count = 0u32;

    for line in output.lines() {
        let trimmed = line.trim_end();

        if state == State::Outside {
            if is_open_marker(trimmed) {
                state = State::InOurs;
                conflict_count += 1;
            }
            continue;
        }

        // Marker-looking payload lines can appear inside conflict bodies
        // (e.g. merged source containing literal "<<<<<<<"). Treat those as
        // content unless they match the expected delimiter for the current
        // conflict parser state.
        match state {
            State::InOurs => {
                if is_base_marker(trimmed) {
                    state = State::InBase;
                } else if is_separator_marker(trimmed) {
                    state = State::InTheirs;
                }
            }
            State::InBase => {
                if is_separator_marker(trimmed) {
                    state = State::InTheirs;
                }
            }
            State::InTheirs => {
                if is_close_marker(trimmed) {
                    state = State::Outside;
                }
            }
            State::Outside => unreachable!("handled above"),
        }
    }

    assert_eq!(
        state,
        State::Outside,
        "[{}] unclosed conflict markers ({} opened)",
        case_name,
        conflict_count
    );
}

/// Every non-marker line in output must trace to at least one input.
fn validate_content_integrity(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case_name: &str,
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
            case_name,
            line_num + 1,
            line
        );
    }
}

// ---------------------------------------------------------------------------
// Marker detection helpers
// ---------------------------------------------------------------------------

fn is_open_marker(line: &str) -> bool {
    line.starts_with("<<<<<<<")
        && line.len() >= 7
        && line[7..]
            .chars()
            .all(|c| c == '<' || c == ' ' || c.is_alphanumeric() || "/.:-_".contains(c))
}

fn is_close_marker(line: &str) -> bool {
    line.starts_with(">>>>>>>")
        && line.len() >= 7
        && line[7..]
            .chars()
            .all(|c| c == '>' || c == ' ' || c.is_alphanumeric() || "/.:-_".contains(c))
}

fn is_separator_marker(line: &str) -> bool {
    line.starts_with("=======") && line[7..].chars().all(|c| c == '=')
}

fn is_base_marker(line: &str) -> bool {
    line.starts_with("|||||||")
        && line.len() >= 7
        && line[7..]
            .chars()
            .all(|c| c == '|' || c == ' ' || c.is_alphanumeric() || "/.:-_".contains(c))
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

// ---------------------------------------------------------------------------
// Pipeline: extract from repo + validate merge invariants
// ---------------------------------------------------------------------------

/// Find the root of the gitcomet repository (the repo we're building from).
fn find_gitcomet_repo() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Walk up to find the .git directory
    let mut dir = Some(manifest_dir);
    while let Some(current) = dir {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    None
}

fn create_repo_with_merge_commit() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);
    std::fs::write(repo.join("base.txt"), "base\n").expect("write base");
    run_git(repo, &["add", "base.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    run_git(repo, &["checkout", "-b", "branch-a"]);
    std::fs::write(repo.join("from_a.txt"), "change from A\n").expect("write from_a");
    run_git(repo, &["add", "from_a.txt"]);
    run_git(repo, &["commit", "-m", "branch-a"]);

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    std::fs::write(repo.join("from_b.txt"), "change from B\n").expect("write from_b");
    run_git(repo, &["add", "from_b.txt"]);
    run_git(repo, &["commit", "-m", "branch-b"]);

    run_git(repo, &["merge", "branch-a", "--no-edit"]);

    tmp
}

/// Run the extraction pipeline on a repository and validate all extracted cases.
///
/// Returns `(total_cases, clean_merges, conflicts)` for reporting.
fn run_extraction_and_validate(
    repo: &Path,
    max_merges: usize,
    max_files_per_merge: usize,
) -> (usize, usize, usize) {
    let options = MergeExtractionOptions {
        max_merges,
        max_files_per_merge,
    };
    let cases = extract_merge_cases_from_repo(repo, options).expect("merge extraction failed");

    let mut clean_count = 0usize;
    let mut conflict_count = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        let case_name = format!(
            "{}:{}",
            case.merge_commit,
            case.file_path.chars().take(40).collect::<String>()
        );

        let merge_opts = MergeOptions::default();
        let result = merge_file(&case.base, &case.contrib1, &case.contrib2, &merge_opts);

        // Catch invariant panics for better error reporting.
        let invariant_result = std::panic::catch_unwind(|| {
            validate_merge_invariants(
                &case.base,
                &case.contrib1,
                &case.contrib2,
                &result.output,
                &case_name,
            );
        });

        match invariant_result {
            Ok(()) => {
                if result.is_clean() {
                    clean_count += 1;
                } else {
                    conflict_count += 1;
                }
            }
            Err(e) => {
                let msg = if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                failures.push(format!("[{}] invariant violation: {}", case_name, msg));
            }
        }
    }

    let total_cases = cases.len();
    eprintln!(
        "\nMerge extraction: {} cases extracted ({} clean, {} conflicts), {} invariant failures",
        total_cases,
        clean_count,
        conflict_count,
        failures.len()
    );

    if !failures.is_empty() {
        panic!(
            "{} invariant failure(s):\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    (total_cases, clean_count, conflict_count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn extraction_discovers_merge_commits() {
    let tmp = create_repo_with_merge_commit();
    let merges = discover_merge_commits(tmp.path(), 10).expect("discover merges");
    assert_eq!(merges.len(), 1, "Expected exactly one merge commit");

    let merge = &merges[0];
    assert!(!merge.merge_sha.is_empty());
    assert!(!merge.parent1_sha.is_empty());
    assert!(!merge.parent2_sha.is_empty());
    // All should be hex SHA strings.
    assert!(
        merge.merge_sha.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid merge SHA: {}",
        merge.merge_sha
    );
    assert!(
        merge.parent1_sha.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid parent1 SHA: {}",
        merge.parent1_sha
    );
    assert!(
        merge.parent2_sha.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid parent2 SHA: {}",
        merge.parent2_sha
    );
}

#[test]
fn extraction_skips_trivial_merges() {
    // Create a temp repo with a trivial merge (fast-forward style).
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    // Init repo with a base commit.
    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);
    std::fs::write(repo.join("file.txt"), "base content\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    // Branch A: modify file.txt
    run_git(repo, &["checkout", "-b", "branch-a"]);
    std::fs::write(repo.join("file.txt"), "modified by A\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "change A"]);

    // Branch B: leave file.txt unchanged (trivial merge — base == contrib2)
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    std::fs::write(repo.join("other.txt"), "unrelated change\n").unwrap();
    run_git(repo, &["add", "other.txt"]);
    run_git(repo, &["commit", "-m", "change B"]);

    // Merge branch-a into branch-b (no conflict — trivial overlap)
    run_git(repo, &["merge", "branch-a", "--no-edit"]);

    let merges = discover_merge_commits(repo, 10).expect("discover merges");
    assert_eq!(merges.len(), 1, "Expected exactly one merge commit");

    let cases = extract_merge_cases(repo, &merges[0], 10).expect("extract cases");
    // file.txt was only changed in one parent, so there's no overlapping file.
    // other.txt was only changed in one parent as well. No non-trivial cases.
    assert!(
        cases.is_empty(),
        "Expected no non-trivial merge cases, got {}",
        cases.len()
    );
}

#[test]
fn extraction_finds_nontrivial_conflict() {
    // Create a temp repo with a real conflict merge.
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);
    std::fs::write(repo.join("file.txt"), "line 1\nline 2\nline 3\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    // Branch A: modify line 2
    run_git(repo, &["checkout", "-b", "branch-a"]);
    std::fs::write(repo.join("file.txt"), "line 1\nmodified by A\nline 3\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "change A"]);

    // Branch B: modify line 2 differently
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    std::fs::write(repo.join("file.txt"), "line 1\nmodified by B\nline 3\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "change B"]);

    // Merge — expect conflict
    let merge_output = Command::new("git")
        .args(["merge", "branch-a", "--no-edit"])
        .current_dir(repo)
        .output()
        .expect("git merge");
    // Merge should fail due to conflict
    assert!(
        !merge_output.status.success(),
        "Expected merge conflict but merge succeeded"
    );

    // Resolve and commit so we have a merge commit
    std::fs::write(repo.join("file.txt"), "line 1\nresolved\nline 3\n").unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "merge with conflict"]);

    // Now extract
    let merges = discover_merge_commits(repo, 10).expect("discover merges");
    assert_eq!(merges.len(), 1);

    let cases = extract_merge_cases(repo, &merges[0], 10).expect("extract cases");
    assert_eq!(cases.len(), 1, "Expected one non-trivial merge case");

    let case = &cases[0];
    assert_eq!(case.file_path, "file.txt");
    assert!(case.base.contains("line 2"));
    assert!(case.contrib1.contains("modified by B")); // parent1 is branch-b (current)
    assert!(case.contrib2.contains("modified by A")); // parent2 is branch-a (merged)

    // Run merge algorithm and validate invariants
    let result = merge_file(
        &case.base,
        &case.contrib1,
        &case.contrib2,
        &MergeOptions::default(),
    );
    assert!(
        result.conflict_count > 0,
        "Expected conflict in merge output"
    );
    validate_merge_invariants(
        &case.base,
        &case.contrib1,
        &case.contrib2,
        &result.output,
        "nontrivial_conflict",
    );
}

#[test]
fn extraction_handles_clean_merge() {
    // Non-overlapping changes: both parents change file.txt but in different regions.
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);
    std::fs::write(
        repo.join("file.txt"),
        "line 1\nline 2\nline 3\nline 4\nline 5\n",
    )
    .unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    // Branch A: change line 1
    run_git(repo, &["checkout", "-b", "branch-a"]);
    std::fs::write(
        repo.join("file.txt"),
        "MODIFIED LINE 1\nline 2\nline 3\nline 4\nline 5\n",
    )
    .unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "change A"]);

    // Branch B: change line 5
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    std::fs::write(
        repo.join("file.txt"),
        "line 1\nline 2\nline 3\nline 4\nMODIFIED LINE 5\n",
    )
    .unwrap();
    run_git(repo, &["add", "file.txt"]);
    run_git(repo, &["commit", "-m", "change B"]);

    // Merge — should succeed (no overlapping changes)
    run_git(repo, &["merge", "branch-a", "--no-edit"]);

    let merges = discover_merge_commits(repo, 10).expect("discover merges");
    assert_eq!(merges.len(), 1);

    let cases = extract_merge_cases(repo, &merges[0], 10).expect("extract cases");
    assert_eq!(cases.len(), 1, "Expected one non-trivial merge case");

    let case = &cases[0];
    let result = merge_file(
        &case.base,
        &case.contrib1,
        &case.contrib2,
        &MergeOptions::default(),
    );
    assert!(
        result.is_clean(),
        "Expected clean merge for non-overlapping changes"
    );
    validate_merge_invariants(
        &case.base,
        &case.contrib1,
        &case.contrib2,
        &result.output,
        "clean_merge",
    );

    // Verify both changes are present in output
    assert!(result.output.contains("MODIFIED LINE 5"));
    assert!(result.output.contains("MODIFIED LINE 1"));
}

#[test]
fn extraction_skips_binary_files() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);
    // Write a binary file
    std::fs::write(repo.join("image.bin"), b"\x89PNG\r\n\x1a\n\x00\x00base").unwrap();
    run_git(repo, &["add", "image.bin"]);
    run_git(repo, &["commit", "-m", "base"]);

    // Branch A: modify binary
    run_git(repo, &["checkout", "-b", "branch-a"]);
    std::fs::write(repo.join("image.bin"), b"\x89PNG\r\n\x1a\n\x00\x00contribA").unwrap();
    run_git(repo, &["add", "image.bin"]);
    run_git(repo, &["commit", "-m", "change A"]);

    // Branch B: modify binary differently
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    std::fs::write(repo.join("image.bin"), b"\x89PNG\r\n\x1a\n\x00\x00contribB").unwrap();
    run_git(repo, &["add", "image.bin"]);
    run_git(repo, &["commit", "-m", "change B"]);

    // Force merge with conflict resolution
    let _ = Command::new("git")
        .args(["merge", "branch-a", "--no-edit"])
        .current_dir(repo)
        .output();
    std::fs::write(repo.join("image.bin"), b"\x89PNG\r\n\x1a\n\x00\x00resolved").unwrap();
    run_git(repo, &["add", "image.bin"]);
    run_git(repo, &["commit", "-m", "merge"]);

    let merges = discover_merge_commits(repo, 10).expect("discover merges");
    assert_eq!(merges.len(), 1);

    let cases = extract_merge_cases(repo, &merges[0], 10).expect("extract cases");
    assert!(
        cases.is_empty(),
        "Binary files should be skipped, got {} cases",
        cases.len()
    );
}

#[test]
fn extraction_writes_fixture_files() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let dest = tmp.path().join("fixtures");

    let cases = vec![ExtractedMergeCase {
        merge_commit: "abc12345".to_string(),
        file_path: "src/main.rs".to_string(),
        base: "fn main() {}\n".to_string(),
        contrib1: "fn main() { println!(\"A\"); }\n".to_string(),
        contrib2: "fn main() { println!(\"B\"); }\n".to_string(),
    }];

    write_fixture_files(&cases, &dest).expect("write fixtures");

    // Module sanitizes "src/main.rs" -> "src_main_rs"
    assert!(dest.join("abc12345_src_main_rs_base.txt").exists());
    assert!(dest.join("abc12345_src_main_rs_contrib1.txt").exists());
    assert!(dest.join("abc12345_src_main_rs_contrib2.txt").exists());
    assert!(
        dest.join("abc12345_src_main_rs_expected_result.txt")
            .exists()
    );

    // Verify content
    let base = std::fs::read_to_string(dest.join("abc12345_src_main_rs_base.txt")).unwrap();
    assert_eq!(base, "fn main() {}\n");

    // Expected result should contain merge engine output (generated golden)
    let expected =
        std::fs::read_to_string(dest.join("abc12345_src_main_rs_expected_result.txt")).unwrap();
    let golden = merge_file(
        "fn main() {}\n",
        "fn main() { println!(\"A\"); }\n",
        "fn main() { println!(\"B\"); }\n",
        &MergeOptions::default(),
    )
    .output;
    assert_eq!(expected, golden);
    assert!(!expected.is_empty());
}

#[test]
fn extraction_handles_multifile_merge() {
    // Merge with multiple conflicting files.
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);
    std::fs::write(repo.join("a.txt"), "alpha base\n").unwrap();
    std::fs::write(repo.join("b.txt"), "beta base\n").unwrap();
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    // Branch A: change both files
    run_git(repo, &["checkout", "-b", "branch-a"]);
    std::fs::write(repo.join("a.txt"), "alpha by A\n").unwrap();
    std::fs::write(repo.join("b.txt"), "beta by A\n").unwrap();
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(repo, &["commit", "-m", "change A"]);

    // Branch B: change both files differently
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    std::fs::write(repo.join("a.txt"), "alpha by B\n").unwrap();
    std::fs::write(repo.join("b.txt"), "beta by B\n").unwrap();
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(repo, &["commit", "-m", "change B"]);

    // Merge
    let _ = Command::new("git")
        .args(["merge", "branch-a", "--no-edit"])
        .current_dir(repo)
        .output();
    std::fs::write(repo.join("a.txt"), "alpha resolved\n").unwrap();
    std::fs::write(repo.join("b.txt"), "beta resolved\n").unwrap();
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(repo, &["commit", "-m", "merge"]);

    let merges = discover_merge_commits(repo, 10).expect("discover merges");
    assert_eq!(merges.len(), 1);

    let cases = extract_merge_cases(repo, &merges[0], 10).expect("extract cases");
    assert_eq!(
        cases.len(),
        2,
        "Expected two non-trivial merge cases (a.txt and b.txt)"
    );

    // Both should produce valid merge output
    for case in &cases {
        let result = merge_file(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &MergeOptions::default(),
        );
        validate_merge_invariants(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &result.output,
            &format!("multifile:{}", case.file_path),
        );
    }
}

/// Run extraction against gitcomet's own repository for real-world regression testing.
///
/// This test scans the most recent merge commits in the gitcomet repo, extracts
/// non-trivial file merge cases, and validates merge algorithm invariants on
/// each case.
///
/// If the repo has no merge commits (linear history), the test passes trivially.
#[test]
fn extraction_regression_on_gitcomet_repo() {
    let Some(repo) = find_gitcomet_repo() else {
        eprintln!(
            "Skipping gitcomet extraction: could not find a host repository root from {}",
            env!("CARGO_MANIFEST_DIR")
        );
        return;
    };

    let host_merges = match discover_merge_commits(&repo, 1) {
        Ok(merges) => merges,
        Err(error) => {
            eprintln!(
                "Skipping gitcomet extraction: repository validation failed for {}: {}",
                repo.display(),
                error
            );
            return;
        }
    };

    if host_merges.is_empty() {
        eprintln!(
            "Skipping gitcomet extraction: no merge commits in {}",
            repo.display()
        );
        return;
    }

    let (total, clean, conflicts) = run_extraction_and_validate(&repo, 20, 5);
    eprintln!(
        "gitcomet extraction: {} total cases, {} clean, {} conflicts",
        total, clean, conflicts
    );
}

/// Run extraction against a large external repository for comprehensive
/// regression testing. Set the `GITGPUI_MERGE_EXTRACTION_REPO` environment
/// variable to the path of the repository to test against.
///
/// Example:
///   GITGPUI_MERGE_EXTRACTION_REPO=/home/user/git/linux cargo test \
///     --test merge_git_extraction extraction_regression_on_external_repo \
///     -- --ignored
#[test]
#[ignore]
fn extraction_regression_on_external_repo() {
    let repo_path = std::env::var("GITGPUI_MERGE_EXTRACTION_REPO").unwrap_or_else(|_| {
        panic!(
            "Set GITGPUI_MERGE_EXTRACTION_REPO to a git repo path to run this test.\n\
             Example: GITGPUI_MERGE_EXTRACTION_REPO=/home/user/git/linux"
        )
    });
    let repo = Path::new(&repo_path);
    assert!(
        repo.join(".git").exists(),
        "{} is not a git repository",
        repo.display()
    );

    let (total, clean, conflicts) = run_extraction_and_validate(repo, 50, 10);
    eprintln!(
        "External repo extraction: {} total cases, {} clean, {} conflicts",
        total, clean, conflicts
    );
}

/// Generate fixture files from real merge commits to disk.
///
/// Set `GITGPUI_MERGE_EXTRACTION_REPO` and `GITGPUI_MERGE_EXTRACTION_DEST`
/// to run. Generated fixtures are compatible with the existing fixture harness.
///
/// Example:
///   GITGPUI_MERGE_EXTRACTION_REPO=/home/user/git/linux \
///   GITGPUI_MERGE_EXTRACTION_DEST=crates/gitcomet-core/tests/fixtures/merge_extracted \
///   cargo test --test merge_git_extraction generate_fixtures_from_repo -- --ignored
#[test]
#[ignore]
fn generate_fixtures_from_repo() {
    let repo_path = std::env::var("GITGPUI_MERGE_EXTRACTION_REPO")
        .unwrap_or_else(|_| panic!("Set GITGPUI_MERGE_EXTRACTION_REPO to a git repo path"));
    let dest_path = std::env::var("GITGPUI_MERGE_EXTRACTION_DEST")
        .unwrap_or_else(|_| panic!("Set GITGPUI_MERGE_EXTRACTION_DEST to an output directory"));

    let repo = Path::new(&repo_path);
    let dest = Path::new(&dest_path);

    assert!(
        repo.join(".git").exists(),
        "{} is not a git repository",
        repo.display()
    );

    let options = MergeExtractionOptions {
        max_merges: 50,
        max_files_per_merge: 10,
    };
    let cases = extract_merge_cases_from_repo(repo, options).expect("merge extraction failed");

    eprintln!("Extracted {} non-trivial merge cases", cases.len());

    write_fixture_files(&cases, dest).expect("write fixture files");
    eprintln!("Fixtures written to {}", dest.display());
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        // Keep fixture-driven tests portable even when the host has
        // `commit.gpgsign=true` globally.
        .arg("-c")
        .arg("commit.gpgsign=false")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run git {:?}: {}", args, e));
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_git_user(repo: &Path) {
    run_git(repo, &["config", "user.email", "test@example.com"]);
    run_git(repo, &["config", "user.name", "Test User"]);
}
