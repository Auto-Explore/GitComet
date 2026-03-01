//! Phase 3C: Real-world merge extraction harness.
//!
//! Walks merge commits in a git repository, extracts base/contrib1/contrib2
//! file contents for each non-trivial merge, runs the merge algorithm on them,
//! and validates algorithm-independent invariants.
//!
//! Inspired by KDiff3's `generate_testdata_from_git_merges.py`.
//!
//! The default test runs against the gitgpui repository itself. An ignored
//! test demonstrates running against an external repository (e.g. linux kernel).
//!
//! These tests generate fixtures at test time — no file system bloat from
//! pre-committed merge data.

use gitgpui_core::merge::{MergeOptions, merge_file};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Git helper functions
// ---------------------------------------------------------------------------

/// Run a git command in the given repo directory, returning stdout as a string.
/// Returns `None` if the command fails.
fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// Run a git command that produces raw bytes (for `git show`).
/// Returns `None` if the command fails.
fn git_bytes(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Merge commit extraction
// ---------------------------------------------------------------------------

/// A single extracted merge case: three file versions ready for merge testing.
#[derive(Debug)]
struct ExtractedMerge {
    /// Merge commit hash (abbreviated).
    merge_commit: String,
    /// File path within the repository.
    file_path: String,
    /// File content at the merge-base.
    base: String,
    /// File content in the first parent (contrib1 / local).
    contrib1: String,
    /// File content in the second parent (contrib2 / remote).
    contrib2: String,
}

/// Discover merge commits in a repository, limited to `max_merges`.
///
/// Returns a list of `(merge_sha, parent1_sha, parent2_sha)` tuples.
/// Only considers merges with exactly 2 parents.
fn discover_merge_commits(repo: &Path, max_merges: usize) -> Vec<(String, String, String)> {
    // Use the default branch (HEAD) to find merge commits
    let output = git_output(
        repo,
        &[
            "rev-list",
            "--merges",
            "--parents",
            &format!("--max-count={}", max_merges * 2), // over-fetch to filter octopus merges
            "HEAD",
        ],
    )
    .unwrap_or_default();

    let mut merges = Vec::new();
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() == 3 {
            // Exactly 2 parents — standard merge
            merges.push((
                fields[0].to_string(),
                fields[1].to_string(),
                fields[2].to_string(),
            ));
        }
        // Skip octopus merges (>2 parents)
        if merges.len() >= max_merges {
            break;
        }
    }
    merges
}

/// Extract non-trivial merge cases from a single merge commit.
///
/// For each file changed in both parents relative to the merge-base:
/// 1. Finds the merge-base of the two parents.
/// 2. Gets the list of files changed in both parent branches.
/// 3. Extracts file contents at base/contrib1/contrib2.
/// 4. Skips trivial cases (base == either contrib, or contribs identical).
/// 5. Skips binary files (non-UTF-8 content).
///
/// Returns extracted merge cases, limited to `max_files_per_merge`.
fn extract_merge_cases(
    repo: &Path,
    merge_sha: &str,
    parent1: &str,
    parent2: &str,
    max_files_per_merge: usize,
) -> Vec<ExtractedMerge> {
    // Find the merge-base of the two parents.
    let base_sha = match git_output(repo, &["merge-base", parent1, parent2]) {
        Some(s) => s.trim().to_string(),
        None => return Vec::new(),
    };

    // Find files changed in both parents relative to the merge-base.
    let files1 = git_output(repo, &["diff", "--name-only", &base_sha, parent1])
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();

    let files2 = git_output(repo, &["diff", "--name-only", &base_sha, parent2])
        .unwrap_or_default()
        .lines()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();

    let overlapping: Vec<&String> = files1.intersection(&files2).collect();
    if overlapping.is_empty() {
        return Vec::new();
    }

    let short_merge = &merge_sha[..merge_sha.len().min(8)];
    let mut results = Vec::new();

    for file_path in overlapping {
        if file_path.is_empty() {
            continue;
        }

        // Extract file contents at each revision.
        let base_bytes = match git_bytes(repo, &["show", &format!("{}:{}", base_sha, file_path)]) {
            Some(b) => b,
            None => continue,
        };
        let contrib1_bytes = match git_bytes(repo, &["show", &format!("{}:{}", parent1, file_path)])
        {
            Some(b) => b,
            None => continue,
        };
        let contrib2_bytes = match git_bytes(repo, &["show", &format!("{}:{}", parent2, file_path)])
        {
            Some(b) => b,
            None => continue,
        };

        // Skip trivial merges: base == either contrib, or contribs identical.
        if base_bytes == contrib1_bytes
            || base_bytes == contrib2_bytes
            || contrib1_bytes == contrib2_bytes
        {
            continue;
        }

        // Skip binary files — merge algorithm works on text.
        let base_str = match std::str::from_utf8(&base_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };
        let contrib1_str = match std::str::from_utf8(&contrib1_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };
        let contrib2_str = match std::str::from_utf8(&contrib2_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };

        results.push(ExtractedMerge {
            merge_commit: short_merge.to_string(),
            file_path: file_path.clone(),
            base: base_str,
            contrib1: contrib1_str,
            contrib2: contrib2_str,
        });

        if results.len() >= max_files_per_merge {
            break;
        }
    }

    results
}

/// Write extracted merge cases to disk as fixture files compatible with the
/// existing fixture harness.
///
/// Files are written to `dest_dir` following the naming convention:
///   `{merge_sha}_{simplified_path}_{base,contrib1,contrib2,expected_result}.txt`
fn write_fixtures(cases: &[ExtractedMerge], dest_dir: &Path) {
    std::fs::create_dir_all(dest_dir).expect("Failed to create fixture output directory");

    for case in cases {
        let simplified = case
            .file_path
            .replace('/', "_")
            .replace('.', "_")
            .replace(' ', "_");
        let prefix = format!("{}_{}", case.merge_commit, simplified);

        let base_path = dest_dir.join(format!("{}_base.txt", prefix));
        let contrib1_path = dest_dir.join(format!("{}_contrib1.txt", prefix));
        let contrib2_path = dest_dir.join(format!("{}_contrib2.txt", prefix));
        let expected_path = dest_dir.join(format!("{}_expected_result.txt", prefix));

        std::fs::write(&base_path, &case.base).expect("Failed to write base fixture");
        std::fs::write(&contrib1_path, &case.contrib1).expect("Failed to write contrib1 fixture");
        std::fs::write(&contrib2_path, &case.contrib2).expect("Failed to write contrib2 fixture");
        // Write empty expected_result — invariant checks still run; expected results
        // can be populated later from a reference tool run.
        if !expected_path.exists() {
            std::fs::write(&expected_path, "").expect("Failed to write expected_result fixture");
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant validation (shared with fixture harness)
// ---------------------------------------------------------------------------

/// Validate algorithm-independent invariants on merge output.
fn validate_merge_invariants(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case_name: &str,
) {
    validate_marker_wellformedness(output, case_name);
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

    for (line_num, line) in output.lines().enumerate() {
        let trimmed = line.trim_end();
        let line_num = line_num + 1;

        if is_open_marker(trimmed) {
            assert_eq!(
                state,
                State::Outside,
                "[{}] line {}: nested <<<<<<< marker",
                case_name,
                line_num
            );
            state = State::InOurs;
            conflict_count += 1;
        } else if is_base_marker(trimmed) {
            assert_eq!(
                state,
                State::InOurs,
                "[{}] line {}: unexpected ||||||| marker",
                case_name,
                line_num
            );
            state = State::InBase;
        } else if is_separator_marker(trimmed) {
            assert!(
                state == State::InOurs || state == State::InBase,
                "[{}] line {}: unexpected ======= marker",
                case_name,
                line_num
            );
            state = State::InTheirs;
        } else if is_close_marker(trimmed) {
            assert_eq!(
                state,
                State::InTheirs,
                "[{}] line {}: unexpected >>>>>>> marker",
                case_name,
                line_num
            );
            state = State::Outside;
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

// ---------------------------------------------------------------------------
// Test: extract merges from gitgpui's own repository
// ---------------------------------------------------------------------------

/// Find the root of the gitgpui repository (the repo we're building from).
fn find_gitgpui_repo() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Walk up to find the .git directory
    let mut dir = manifest_dir;
    loop {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        dir = match dir.parent() {
            Some(p) => p,
            None => panic!(
                "Could not find git repository root from {}",
                manifest_dir.display()
            ),
        };
    }
}

/// Check if the repo has enough merge commits for meaningful testing.
fn repo_has_merges(repo: &Path, minimum: usize) -> bool {
    let merges = discover_merge_commits(repo, minimum);
    merges.len() >= minimum
}

/// Run the extraction pipeline on a repository and validate all extracted cases.
///
/// Returns `(total_cases, clean_merges, conflicts)` for reporting.
fn run_extraction_and_validate(
    repo: &Path,
    max_merges: usize,
    max_files_per_merge: usize,
) -> (usize, usize, usize) {
    let merges = discover_merge_commits(repo, max_merges);

    let mut total_cases = 0usize;
    let mut clean_count = 0usize;
    let mut conflict_count = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (merge_sha, parent1, parent2) in &merges {
        let cases = extract_merge_cases(repo, merge_sha, parent1, parent2, max_files_per_merge);

        for case in &cases {
            total_cases += 1;
            let case_name = format!(
                "{}:{}",
                case.merge_commit,
                case.file_path.chars().take(40).collect::<String>()
            );

            let options = MergeOptions::default();
            let result = merge_file(&case.base, &case.contrib1, &case.contrib2, &options);

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
    }

    eprintln!(
        "\nMerge extraction: {} merges scanned, {} cases extracted ({} clean, {} conflicts), {} invariant failures",
        merges.len(),
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
    let repo = find_gitgpui_repo();
    let merges = discover_merge_commits(&repo, 10);
    // The gitgpui repo may or may not have merges depending on branch strategy.
    // This test verifies the discovery mechanism works without panicking.
    eprintln!(
        "Discovered {} merge commits in {}",
        merges.len(),
        repo.display()
    );
    for (merge, p1, p2) in &merges {
        assert!(!merge.is_empty());
        assert!(!p1.is_empty());
        assert!(!p2.is_empty());
        // All should be hex SHA strings
        assert!(
            merge.chars().all(|c| c.is_ascii_hexdigit()),
            "Invalid merge SHA: {}",
            merge
        );
    }
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

    let merges = discover_merge_commits(repo, 10);
    assert_eq!(merges.len(), 1, "Expected exactly one merge commit");

    let (merge, p1, p2) = &merges[0];
    let cases = extract_merge_cases(repo, merge, p1, p2, 10);
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
    let merges = discover_merge_commits(repo, 10);
    assert_eq!(merges.len(), 1);

    let (merge, p1, p2) = &merges[0];
    let cases = extract_merge_cases(repo, merge, p1, p2, 10);
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

    let merges = discover_merge_commits(repo, 10);
    assert_eq!(merges.len(), 1);

    let (merge, p1, p2) = &merges[0];
    let cases = extract_merge_cases(repo, merge, p1, p2, 10);
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

    let merges = discover_merge_commits(repo, 10);
    assert_eq!(merges.len(), 1);

    let (merge, p1, p2) = &merges[0];
    let cases = extract_merge_cases(repo, merge, p1, p2, 10);
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

    let cases = vec![ExtractedMerge {
        merge_commit: "abc12345".to_string(),
        file_path: "src/main.rs".to_string(),
        base: "fn main() {}\n".to_string(),
        contrib1: "fn main() { println!(\"A\"); }\n".to_string(),
        contrib2: "fn main() { println!(\"B\"); }\n".to_string(),
    }];

    write_fixtures(&cases, &dest);

    // Verify all four files exist
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

    // Expected result should be empty (to be filled later)
    let expected =
        std::fs::read_to_string(dest.join("abc12345_src_main_rs_expected_result.txt")).unwrap();
    assert!(expected.is_empty());
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

    let merges = discover_merge_commits(repo, 10);
    assert_eq!(merges.len(), 1);

    let (merge, p1, p2) = &merges[0];
    let cases = extract_merge_cases(repo, merge, p1, p2, 10);
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

/// Run extraction against gitgpui's own repository for real-world regression testing.
///
/// This test scans the most recent merge commits in the gitgpui repo, extracts
/// non-trivial file merge cases, and validates merge algorithm invariants on
/// each case.
///
/// If the repo has no merge commits (linear history), the test passes trivially.
#[test]
fn extraction_regression_on_gitgpui_repo() {
    let repo = find_gitgpui_repo();

    if !repo_has_merges(&repo, 1) {
        eprintln!(
            "Skipping gitgpui extraction: no merge commits in {}",
            repo.display()
        );
        return;
    }

    let (total, clean, conflicts) = run_extraction_and_validate(&repo, 20, 5);
    eprintln!(
        "gitgpui extraction: {} total cases, {} clean, {} conflicts",
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
///   GITGPUI_MERGE_EXTRACTION_DEST=crates/gitgpui-core/tests/fixtures/merge_extracted \
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

    let merges = discover_merge_commits(repo, 50);
    let mut all_cases = Vec::new();

    for (merge_sha, p1, p2) in &merges {
        let cases = extract_merge_cases(repo, merge_sha, p1, p2, 10);
        all_cases.extend(cases);
    }

    eprintln!(
        "Extracted {} non-trivial merge cases from {} merges",
        all_cases.len(),
        merges.len()
    );

    write_fixtures(&all_cases, dest);
    eprintln!("Fixtures written to {}", dest.display());
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
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
