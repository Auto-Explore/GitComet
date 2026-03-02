//! Utilities for extracting non-trivial 3-way merge cases from git history.
//!
//! This module ports the core workflow described in
//! `docs/REFERENCE_TEST_PORTABILITY.md` Phase 3C (real-world merge extraction)
//! into production code so it can be reused outside ad-hoc test harnesses.

use crate::merge::{MergeOptions, merge_file};
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A merge commit with exactly two parents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeCommit {
    pub merge_sha: String,
    pub parent1_sha: String,
    pub parent2_sha: String,
}

/// A non-trivial extracted 3-way merge case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedMergeCase {
    /// Abbreviated merge commit SHA (up to 8 chars).
    pub merge_commit: String,
    /// Path of the file in the repository at merge time.
    pub file_path: String,
    /// Content at merge-base.
    pub base: String,
    /// Content in parent1.
    pub contrib1: String,
    /// Content in parent2.
    pub contrib2: String,
}

/// Extraction limits for scanning a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeExtractionOptions {
    /// Maximum number of merge commits to scan.
    pub max_merges: usize,
    /// Maximum number of files extracted from each merge commit.
    pub max_files_per_merge: usize,
}

impl Default for MergeExtractionOptions {
    fn default() -> Self {
        Self {
            max_merges: 20,
            max_files_per_merge: 5,
        }
    }
}

/// Error type for merge-case extraction and fixture writing.
#[derive(Debug)]
pub enum MergeExtractionError {
    InvalidArgument(&'static str),
    NotGitRepository(PathBuf),
    GitCommandFailed {
        command: String,
        stderr: String,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for MergeExtractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message) => write!(f, "{message}"),
            Self::NotGitRepository(path) => {
                write!(f, "{} is not a git repository", path.display())
            }
            Self::GitCommandFailed { command, stderr } => {
                if stderr.is_empty() {
                    write!(f, "{command} failed")
                } else {
                    write!(f, "{command} failed: {stderr}")
                }
            }
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "Failed to {action} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for MergeExtractionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Discover merge commits (exactly two parents) in `repo`, newest first.
pub fn discover_merge_commits(
    repo: &Path,
    max_merges: usize,
) -> Result<Vec<MergeCommit>, MergeExtractionError> {
    ensure_git_repository(repo)?;
    if max_merges == 0 {
        return Err(MergeExtractionError::InvalidArgument(
            "max_merges must be greater than zero",
        ));
    }

    let mut merges = Vec::new();
    let mut skip = 0usize;
    let page_size = max_merges.saturating_mul(4).max(32);

    loop {
        let rev_list = run_git_text(
            repo,
            &[
                "rev-list",
                "--merges",
                "--parents",
                &format!("--max-count={page_size}"),
                &format!("--skip={skip}"),
                "HEAD",
            ],
        )?;

        if rev_list.trim().is_empty() {
            break;
        }

        let mut lines_seen = 0usize;
        for line in rev_list.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            lines_seen += 1;

            let mut parts = trimmed.split_whitespace();
            let Some(merge_sha) = parts.next() else {
                continue;
            };
            let Some(parent1_sha) = parts.next() else {
                continue;
            };
            let Some(parent2_sha) = parts.next() else {
                continue;
            };

            // Keep only exactly two-parent merges.
            if parts.next().is_none() {
                merges.push(MergeCommit {
                    merge_sha: merge_sha.to_string(),
                    parent1_sha: parent1_sha.to_string(),
                    parent2_sha: parent2_sha.to_string(),
                });
            }

            if merges.len() >= max_merges {
                return Ok(merges);
            }
        }

        if lines_seen < page_size {
            break;
        }

        skip = skip.saturating_add(lines_seen);
    }

    Ok(merges)
}

/// Extract non-trivial text merge cases from a single merge commit.
///
/// The extractor keeps only files changed in both parents relative to merge-base,
/// skipping trivial cases and binary/non-UTF8 contents.
///
/// Missing blobs (path absent in base or one parent) are materialized as empty
/// content so add/add and modify/delete style conflicts are retained in the
/// extracted corpus.
pub fn extract_merge_cases(
    repo: &Path,
    merge: &MergeCommit,
    max_files_per_merge: usize,
) -> Result<Vec<ExtractedMergeCase>, MergeExtractionError> {
    ensure_git_repository(repo)?;
    if max_files_per_merge == 0 {
        return Err(MergeExtractionError::InvalidArgument(
            "max_files_per_merge must be greater than zero",
        ));
    }

    let base_sha = run_git_text(
        repo,
        &["merge-base", &merge.parent1_sha, &merge.parent2_sha],
    )?
    .trim()
    .to_string();
    if base_sha.is_empty() {
        return Ok(Vec::new());
    }

    let files1 = changed_files(repo, &base_sha, &merge.parent1_sha)?;
    let files2 = changed_files(repo, &base_sha, &merge.parent2_sha)?;
    if files1.is_empty() || files2.is_empty() {
        return Ok(Vec::new());
    }

    let short_merge = shorten_sha(&merge.merge_sha);
    let mut cases = Vec::new();

    for file_path in files1.intersection(&files2) {
        if file_path.is_empty() {
            continue;
        }

        let base_ref = format!("{base_sha}:{file_path}");
        let parent1_ref = format!("{}:{file_path}", merge.parent1_sha);
        let parent2_ref = format!("{}:{file_path}", merge.parent2_sha);

        let base_bytes = run_git_bytes_optional(repo, &["show", &base_ref])?.unwrap_or_default();
        let contrib1_bytes =
            run_git_bytes_optional(repo, &["show", &parent1_ref])?.unwrap_or_default();
        let contrib2_bytes =
            run_git_bytes_optional(repo, &["show", &parent2_ref])?.unwrap_or_default();

        // Trivial merge cases are not useful as regression samples.
        if base_bytes == contrib1_bytes
            || base_bytes == contrib2_bytes
            || contrib1_bytes == contrib2_bytes
        {
            continue;
        }

        let base = match String::from_utf8(base_bytes) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let contrib1 = match String::from_utf8(contrib1_bytes) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let contrib2 = match String::from_utf8(contrib2_bytes) {
            Ok(text) => text,
            Err(_) => continue,
        };

        cases.push(ExtractedMergeCase {
            merge_commit: short_merge.clone(),
            file_path: file_path.clone(),
            base,
            contrib1,
            contrib2,
        });

        if cases.len() >= max_files_per_merge {
            break;
        }
    }

    Ok(cases)
}

/// Extract merge cases from the latest merge commits in a repository.
pub fn extract_merge_cases_from_repo(
    repo: &Path,
    options: MergeExtractionOptions,
) -> Result<Vec<ExtractedMergeCase>, MergeExtractionError> {
    let merges = discover_merge_commits(repo, options.max_merges)?;
    let mut all_cases = Vec::new();

    for merge in &merges {
        let mut cases = extract_merge_cases(repo, merge, options.max_files_per_merge)?;
        all_cases.append(&mut cases);
    }

    Ok(all_cases)
}

/// Write extracted merge cases as fixture files compatible with the Phase 2 harness.
pub fn write_fixture_files(
    cases: &[ExtractedMergeCase],
    dest_dir: &Path,
) -> Result<(), MergeExtractionError> {
    std::fs::create_dir_all(dest_dir).map_err(|source| MergeExtractionError::Io {
        action: "create fixture directory",
        path: dest_dir.to_path_buf(),
        source,
    })?;

    // Reserve prefixes already present on disk so repeated extraction runs
    // append new fixture sets instead of clobbering existing ones.
    let existing_prefixes = discover_existing_fixture_prefixes(dest_dir)?;
    let mut used_prefixes: HashSet<String> =
        HashSet::with_capacity(existing_prefixes.len() + cases.len());
    used_prefixes.extend(existing_prefixes);

    for case in cases {
        let simplified = sanitize_fixture_component(&case.file_path);
        let base_prefix = format!("{}_{}", case.merge_commit, simplified);
        let prefix = allocate_unique_prefix(&base_prefix, &mut used_prefixes);

        let base_path = dest_dir.join(format!("{prefix}_base.txt"));
        let contrib1_path = dest_dir.join(format!("{prefix}_contrib1.txt"));
        let contrib2_path = dest_dir.join(format!("{prefix}_contrib2.txt"));
        let expected_path = dest_dir.join(format!("{prefix}_expected_result.txt"));

        write_text_file(&base_path, &case.base)?;
        write_text_file(&contrib1_path, &case.contrib1)?;
        write_text_file(&contrib2_path, &case.contrib2)?;

        if !expected_path.exists() {
            // Generate a golden expected result from the current merge engine
            // so newly extracted fixtures are immediately runnable in the
            // Phase 2 fixture harness without manual bootstrapping.
            let expected = merge_file(
                &case.base,
                &case.contrib1,
                &case.contrib2,
                &MergeOptions::default(),
            )
            .output;
            write_text_file(&expected_path, &expected)?;
        }
    }

    Ok(())
}

fn discover_existing_fixture_prefixes(
    dest_dir: &Path,
) -> Result<HashSet<String>, MergeExtractionError> {
    let mut prefixes = HashSet::new();
    let entries = std::fs::read_dir(dest_dir).map_err(|source| MergeExtractionError::Io {
        action: "read fixture directory",
        path: dest_dir.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| MergeExtractionError::Io {
            action: "read fixture directory entry in",
            path: dest_dir.to_path_buf(),
            source,
        })?;
        let Some(file_name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };

        // Reserve only base/contrib fixture prefixes so an expected-only file
        // can still be completed by a later extraction run.
        for suffix in ["_base.txt", "_contrib1.txt", "_contrib2.txt"] {
            if let Some(prefix) = file_name.strip_suffix(suffix) {
                if !prefix.is_empty() {
                    prefixes.insert(prefix.to_string());
                }
                break;
            }
        }
    }

    Ok(prefixes)
}

fn allocate_unique_prefix(base_prefix: &str, used_prefixes: &mut HashSet<String>) -> String {
    let mut suffix = 0usize;
    loop {
        let candidate = if suffix == 0 {
            base_prefix.to_string()
        } else {
            format!("{base_prefix}_{suffix}")
        };

        if used_prefixes.insert(candidate.clone()) {
            return candidate;
        }

        suffix += 1;
    }
}

fn ensure_git_repository(repo: &Path) -> Result<(), MergeExtractionError> {
    let output = run_git(repo, &["rev-parse", "--git-dir"])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(MergeExtractionError::NotGitRepository(repo.to_path_buf()))
    }
}

fn changed_files(
    repo: &Path,
    from: &str,
    to: &str,
) -> Result<BTreeSet<String>, MergeExtractionError> {
    let text = run_git_text(repo, &["diff", "--name-only", from, to])?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn run_git_text(repo: &Path, args: &[&str]) -> Result<String, MergeExtractionError> {
    let output = run_git(repo, args)?;
    if !output.status.success() {
        return Err(MergeExtractionError::GitCommandFailed {
            command: git_command_string(args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_bytes_optional(
    repo: &Path,
    args: &[&str],
) -> Result<Option<Vec<u8>>, MergeExtractionError> {
    let output = run_git(repo, args)?;
    if output.status.success() {
        Ok(Some(output.stdout))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if is_missing_path_error(&stderr) {
            Ok(None)
        } else {
            Err(MergeExtractionError::GitCommandFailed {
                command: git_command_string(args),
                stderr,
            })
        }
    }
}

fn is_missing_path_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    (lower.contains("path '") && lower.contains("does not exist in"))
        || (lower.contains("path '") && lower.contains("exists on disk, but not in"))
}

fn run_git(repo: &Path, args: &[&str]) -> Result<Output, MergeExtractionError> {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|source| MergeExtractionError::Io {
            action: "run git command in",
            path: repo.to_path_buf(),
            source,
        })
}

fn write_text_file(path: &Path, contents: &str) -> Result<(), MergeExtractionError> {
    std::fs::write(path, contents).map_err(|source| MergeExtractionError::Io {
        action: "write file",
        path: path.to_path_buf(),
        source,
    })
}

fn git_command_string(args: &[&str]) -> String {
    let mut command = String::from("git");
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

fn shorten_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

fn sanitize_fixture_component(path: &str) -> String {
    let mut out = String::new();
    let mut previous_underscore = false;

    for ch in path.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            previous_underscore = false;
        } else if !previous_underscore {
            out.push('_');
            previous_underscore = true;
        }
    }

    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap_or_else(|e| panic!("Failed to run git {:?}: {e}", args));
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

    fn create_conflicting_merge_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let repo = tmp.path();

        run_git(repo, &["init"]);
        run_git(repo, &["checkout", "-b", "main"]);
        configure_git_user(repo);
        std::fs::write(repo.join("a.txt"), "base a\n").expect("write a.txt");
        std::fs::write(repo.join("z.txt"), "base z\n").expect("write z.txt");
        std::fs::write(repo.join("img.bin"), b"\x89PNG\r\n\x1a\n\x00\x00base")
            .expect("write img.bin");
        run_git(repo, &["add", "a.txt", "z.txt", "img.bin"]);
        run_git(repo, &["commit", "-m", "base"]);

        run_git(repo, &["checkout", "-b", "branch-a"]);
        std::fs::write(repo.join("a.txt"), "branch a change A\n").expect("write a.txt branch-a");
        std::fs::write(repo.join("z.txt"), "branch a change Z\n").expect("write z.txt branch-a");
        std::fs::write(repo.join("img.bin"), b"\x89PNG\r\n\x1a\n\x00\x00A")
            .expect("write img.bin branch-a");
        run_git(repo, &["add", "a.txt", "z.txt", "img.bin"]);
        run_git(repo, &["commit", "-m", "branch-a changes"]);

        run_git(repo, &["checkout", "main"]);
        run_git(repo, &["checkout", "-b", "branch-b"]);
        std::fs::write(repo.join("a.txt"), "branch b change A\n").expect("write a.txt branch-b");
        std::fs::write(repo.join("z.txt"), "branch b change Z\n").expect("write z.txt branch-b");
        std::fs::write(repo.join("img.bin"), b"\x89PNG\r\n\x1a\n\x00\x00B")
            .expect("write img.bin branch-b");
        run_git(repo, &["add", "a.txt", "z.txt", "img.bin"]);
        run_git(repo, &["commit", "-m", "branch-b changes"]);

        let output = Command::new("git")
            .args(["merge", "branch-a", "--no-edit"])
            .current_dir(repo)
            .output()
            .expect("git merge");
        assert!(
            !output.status.success(),
            "expected merge conflict while building test fixture"
        );

        std::fs::write(repo.join("a.txt"), "resolved a\n").expect("resolve a.txt");
        std::fs::write(repo.join("z.txt"), "resolved z\n").expect("resolve z.txt");
        std::fs::write(repo.join("img.bin"), b"\x89PNG\r\n\x1a\n\x00\x00resolved")
            .expect("resolve img.bin");
        run_git(repo, &["add", "a.txt", "z.txt", "img.bin"]);
        run_git(repo, &["commit", "-m", "merge commit"]);

        tmp
    }

    fn create_missing_side_conflict_merge_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let repo = tmp.path();

        run_git(repo, &["init"]);
        run_git(repo, &["checkout", "-b", "main"]);
        configure_git_user(repo);

        std::fs::write(repo.join("moddel.txt"), "base moddel\n").expect("write moddel base");
        run_git(repo, &["add", "moddel.txt"]);
        run_git(repo, &["commit", "-m", "base"]);

        run_git(repo, &["checkout", "-b", "branch-a"]);
        std::fs::remove_file(repo.join("moddel.txt")).expect("delete moddel in branch-a");
        std::fs::write(repo.join("addadd.txt"), "branch a add\n").expect("write addadd branch-a");
        run_git(repo, &["add", "-A"]);
        run_git(repo, &["commit", "-m", "branch-a delete+add"]);

        run_git(repo, &["checkout", "main"]);
        run_git(repo, &["checkout", "-b", "branch-b"]);
        std::fs::write(repo.join("moddel.txt"), "branch b modify\n")
            .expect("write moddel branch-b");
        std::fs::write(repo.join("addadd.txt"), "branch b add\n").expect("write addadd branch-b");
        run_git(repo, &["add", "moddel.txt", "addadd.txt"]);
        run_git(repo, &["commit", "-m", "branch-b modify+add"]);

        let output = Command::new("git")
            .args(["merge", "branch-a", "--no-edit"])
            .current_dir(repo)
            .output()
            .expect("git merge");
        assert!(
            !output.status.success(),
            "expected merge conflict while building missing-side fixture"
        );

        std::fs::write(repo.join("moddel.txt"), "resolved moddel\n").expect("resolve moddel");
        std::fs::write(repo.join("addadd.txt"), "resolved addadd\n").expect("resolve addadd");
        run_git(repo, &["add", "moddel.txt", "addadd.txt"]);
        run_git(repo, &["commit", "-m", "merge commit"]);

        tmp
    }

    #[test]
    fn discovers_merge_commits_with_two_parents() {
        let repo = create_conflicting_merge_repo();
        let merges = discover_merge_commits(repo.path(), 10).expect("discover merges");
        assert_eq!(merges.len(), 1, "expected one merge commit");

        let merge = &merges[0];
        assert_eq!(merge.merge_sha.len(), 40);
        assert_eq!(merge.parent1_sha.len(), 40);
        assert_eq!(merge.parent2_sha.len(), 40);
    }

    #[test]
    fn discovers_merge_commits_from_repo_subdirectory() {
        let repo = create_conflicting_merge_repo();
        let subdir = repo.path().join("nested").join("dir");
        std::fs::create_dir_all(&subdir).expect("create nested subdirectory");

        let merges = discover_merge_commits(&subdir, 10).expect("discover merges from subdir");
        assert_eq!(
            merges.len(),
            1,
            "expected merge discovery to work from nested subdirectory"
        );
    }

    #[test]
    fn discover_merge_commits_zero_max_merges_errors() {
        let repo = create_conflicting_merge_repo();
        let error =
            discover_merge_commits(repo.path(), 0).expect_err("expected invalid argument error");

        assert!(
            matches!(
                error,
                MergeExtractionError::InvalidArgument("max_merges must be greater than zero")
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn discovers_merge_commits_after_recent_octopus_merges() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let repo = tmp.path();

        run_git(repo, &["init"]);
        run_git(repo, &["checkout", "-b", "main"]);
        configure_git_user(repo);
        std::fs::write(repo.join("base.txt"), "base\n").expect("write base");
        run_git(repo, &["add", "base.txt"]);
        run_git(repo, &["commit", "-m", "base"]);

        // Oldest merge: normal two-parent merge.
        run_git(repo, &["checkout", "-b", "two-parent"]);
        std::fs::write(repo.join("two_parent.txt"), "two-parent branch\n")
            .expect("write two-parent branch");
        run_git(repo, &["add", "two_parent.txt"]);
        run_git(repo, &["commit", "-m", "two-parent branch"]);
        run_git(repo, &["checkout", "main"]);
        run_git(repo, &["merge", "--no-edit", "--no-ff", "two-parent"]);

        // Newer merge #1: octopus merge (more than two parents).
        run_git(repo, &["checkout", "-b", "oct1-a"]);
        std::fs::write(repo.join("oct1_a.txt"), "octopus one a\n").expect("write oct1-a");
        run_git(repo, &["add", "oct1_a.txt"]);
        run_git(repo, &["commit", "-m", "oct1 a"]);
        run_git(repo, &["checkout", "main"]);

        run_git(repo, &["checkout", "-b", "oct1-b"]);
        std::fs::write(repo.join("oct1_b.txt"), "octopus one b\n").expect("write oct1-b");
        run_git(repo, &["add", "oct1_b.txt"]);
        run_git(repo, &["commit", "-m", "oct1 b"]);
        run_git(repo, &["checkout", "main"]);

        run_git(repo, &["checkout", "-b", "oct1-c"]);
        std::fs::write(repo.join("oct1_c.txt"), "octopus one c\n").expect("write oct1-c");
        run_git(repo, &["add", "oct1_c.txt"]);
        run_git(repo, &["commit", "-m", "oct1 c"]);
        run_git(repo, &["checkout", "main"]);

        run_git(repo, &["merge", "--no-edit", "oct1-a", "oct1-b", "oct1-c"]);

        // Newest merge #2: another octopus merge.
        run_git(repo, &["checkout", "-b", "oct2-a"]);
        std::fs::write(repo.join("oct2_a.txt"), "octopus two a\n").expect("write oct2-a");
        run_git(repo, &["add", "oct2_a.txt"]);
        run_git(repo, &["commit", "-m", "oct2 a"]);
        run_git(repo, &["checkout", "main"]);

        run_git(repo, &["checkout", "-b", "oct2-b"]);
        std::fs::write(repo.join("oct2_b.txt"), "octopus two b\n").expect("write oct2-b");
        run_git(repo, &["add", "oct2_b.txt"]);
        run_git(repo, &["commit", "-m", "oct2 b"]);
        run_git(repo, &["checkout", "main"]);

        run_git(repo, &["checkout", "-b", "oct2-c"]);
        std::fs::write(repo.join("oct2_c.txt"), "octopus two c\n").expect("write oct2-c");
        run_git(repo, &["add", "oct2_c.txt"]);
        run_git(repo, &["commit", "-m", "oct2 c"]);
        run_git(repo, &["checkout", "main"]);

        run_git(repo, &["merge", "--no-edit", "oct2-a", "oct2-b", "oct2-c"]);

        // max_merges=1 should still find the older two-parent merge even
        // though the two newest merges are octopus merges.
        let merges = discover_merge_commits(repo, 1).expect("discover merges");
        assert_eq!(merges.len(), 1, "expected one two-parent merge");

        let parent_line = run_git_text(repo, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .expect("read head parent line");
        assert!(
            parent_line.split_whitespace().count() > 3,
            "HEAD should be an octopus merge in this fixture"
        );

        let merge = &merges[0];
        let discovered_parent_line = run_git_text(
            repo,
            &["rev-list", "--parents", "-n", "1", &merge.merge_sha],
        )
        .expect("read discovered merge parent line");
        assert_eq!(
            discovered_parent_line.split_whitespace().count(),
            3,
            "discovered merge should have exactly two parents"
        );
    }

    #[test]
    fn extracts_sorted_text_cases_and_skips_binary() {
        let repo = create_conflicting_merge_repo();
        let merges = discover_merge_commits(repo.path(), 10).expect("discover merges");
        let cases = extract_merge_cases(repo.path(), &merges[0], 10).expect("extract cases");

        let paths: Vec<&str> = cases.iter().map(|case| case.file_path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["a.txt", "z.txt"],
            "expected deterministic sorted text-only extraction"
        );

        for case in &cases {
            assert_eq!(case.merge_commit.len(), 8);
            assert!(case.base.is_ascii());
            assert!(case.contrib1.is_ascii());
            assert!(case.contrib2.is_ascii());
        }
    }

    #[test]
    fn extracts_cases_with_missing_base_or_parent_as_empty_text() {
        let repo = create_missing_side_conflict_merge_repo();
        let merges = discover_merge_commits(repo.path(), 10).expect("discover merges");
        assert_eq!(merges.len(), 1, "expected one merge commit");

        let cases = extract_merge_cases(repo.path(), &merges[0], 10).expect("extract cases");
        let paths: Vec<&str> = cases.iter().map(|case| case.file_path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["addadd.txt", "moddel.txt"],
            "expected deterministic extraction including add/add and modify/delete paths"
        );

        let addadd = cases
            .iter()
            .find(|case| case.file_path == "addadd.txt")
            .expect("find add/add case");
        assert!(
            addadd.base.is_empty(),
            "add/add base should be materialized as empty text"
        );
        assert!(
            !addadd.contrib1.is_empty() && !addadd.contrib2.is_empty(),
            "both add/add sides should contain added content"
        );
        assert_ne!(
            addadd.contrib1, addadd.contrib2,
            "add/add conflicting sides should remain distinct"
        );
        assert!(
            addadd.contrib1 == "branch a add\n" || addadd.contrib1 == "branch b add\n",
            "unexpected contrib1 add/add content: {:?}",
            addadd.contrib1
        );
        assert!(
            addadd.contrib2 == "branch a add\n" || addadd.contrib2 == "branch b add\n",
            "unexpected contrib2 add/add content: {:?}",
            addadd.contrib2
        );

        let moddel = cases
            .iter()
            .find(|case| case.file_path == "moddel.txt")
            .expect("find modify/delete case");
        assert_eq!(moddel.base, "base moddel\n");
        assert!(
            moddel.contrib1.is_empty() ^ moddel.contrib2.is_empty(),
            "modify/delete should have exactly one empty side"
        );
        assert!(
            moddel.contrib1 == "branch b modify\n" || moddel.contrib2 == "branch b modify\n",
            "modify/delete should retain modified content on one side; got {:?} / {:?}",
            moddel.contrib1,
            moddel.contrib2
        );
    }

    #[test]
    fn git_show_missing_path_is_treated_as_absent_side() {
        let repo = create_conflicting_merge_repo();
        let missing = run_git_bytes_optional(repo.path(), &["show", "HEAD:this-file-does-not-exist"])
            .expect("missing path should not error");
        assert!(
            missing.is_none(),
            "missing path should be interpreted as absent side content"
        );
    }

    #[test]
    fn git_show_non_missing_errors_are_propagated() {
        let repo = create_conflicting_merge_repo();
        let err = run_git_bytes_optional(repo.path(), &["show", "definitely-not-a-ref:a.txt"])
            .expect_err("invalid ref should surface as git command failure");

        match err {
            MergeExtractionError::GitCommandFailed { command, stderr } => {
                assert_eq!(
                    command, "git show definitely-not-a-ref:a.txt",
                    "unexpected git command context"
                );
                assert!(
                    !stderr.is_empty(),
                    "expected stderr details for invalid ref failure"
                );
                assert!(
                    !is_missing_path_error(&stderr),
                    "invalid ref error must not be misclassified as missing path: {stderr}"
                );
            }
            other => panic!("expected GitCommandFailed, got {other:?}"),
        }
    }

    #[test]
    fn missing_path_error_detection_matches_git_patterns() {
        assert!(is_missing_path_error(
            "fatal: path 'docs/file.txt' does not exist in 'HEAD'"
        ));
        assert!(is_missing_path_error(
            "fatal: path 'docs/file.txt' exists on disk, but not in 'HEAD'"
        ));
        assert!(!is_missing_path_error(
            "fatal: invalid object name 'definitely-not-a-ref'"
        ));
    }

    #[test]
    fn extracts_merge_cases_from_repo_subdirectory() {
        let repo = create_conflicting_merge_repo();
        let subdir = repo.path().join("nested").join("dir");
        std::fs::create_dir_all(&subdir).expect("create nested subdirectory");

        let cases = extract_merge_cases_from_repo(
            &subdir,
            MergeExtractionOptions {
                max_merges: 10,
                max_files_per_merge: 10,
            },
        )
        .expect("extract cases from subdir");

        let paths: Vec<&str> = cases.iter().map(|case| case.file_path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["a.txt", "z.txt"],
            "expected deterministic sorted text-only extraction from subdir"
        );
    }

    #[test]
    fn writes_fixture_files_and_preserves_existing_expected() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dest = tmp.path().join("fixtures");

        let case = ExtractedMergeCase {
            merge_commit: "abc12345".to_string(),
            file_path: "src/main.rs".to_string(),
            base: "base\n".to_string(),
            contrib1: "one\n".to_string(),
            contrib2: "two\n".to_string(),
        };
        let prefix = "abc12345_src_main_rs";
        let expected_path = dest.join(format!("{prefix}_expected_result.txt"));

        std::fs::create_dir_all(&dest).expect("create fixture dir");
        std::fs::write(&expected_path, "existing expected\n").expect("write expected");

        write_fixture_files(&[case], &dest).expect("write fixtures");

        let expected = std::fs::read_to_string(&expected_path).expect("read expected");
        assert_eq!(
            expected, "existing expected\n",
            "expected fixture writer to keep existing expected result"
        );

        let base =
            std::fs::read_to_string(dest.join(format!("{prefix}_base.txt"))).expect("read base");
        assert_eq!(base, "base\n");
    }

    #[test]
    fn write_fixture_files_generates_expected_result_from_merge_output() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dest = tmp.path().join("fixtures");

        let case = ExtractedMergeCase {
            merge_commit: "def67890".to_string(),
            file_path: "src/lib.rs".to_string(),
            base: "line one\nline two\n".to_string(),
            contrib1: "line one changed\nline two\n".to_string(),
            contrib2: "line one\nline two changed\n".to_string(),
        };
        let prefix = "def67890_src_lib_rs";

        write_fixture_files(std::slice::from_ref(&case), &dest).expect("write fixtures");

        let expected_path = dest.join(format!("{prefix}_expected_result.txt"));
        let expected =
            std::fs::read_to_string(&expected_path).expect("read generated expected result");
        let merge_expected = merge_file(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &MergeOptions::default(),
        )
        .output;
        assert_eq!(
            expected, merge_expected,
            "expected_result should match merge engine output for extracted fixture"
        );
    }

    #[test]
    fn write_fixture_files_disambiguates_colliding_sanitized_paths() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dest = tmp.path().join("fixtures");

        let cases = vec![
            ExtractedMergeCase {
                merge_commit: "abc12345".to_string(),
                file_path: "src/a-b.txt".to_string(),
                base: "base one\n".to_string(),
                contrib1: "one\n".to_string(),
                contrib2: "one-two\n".to_string(),
            },
            ExtractedMergeCase {
                merge_commit: "abc12345".to_string(),
                file_path: "src/a/b.txt".to_string(),
                base: "base two\n".to_string(),
                contrib1: "two\n".to_string(),
                contrib2: "two-two\n".to_string(),
            },
        ];

        write_fixture_files(&cases, &dest).expect("write fixtures");

        let first_prefix = "abc12345_src_a_b_txt";
        let second_prefix = "abc12345_src_a_b_txt_1";

        let first_base = std::fs::read_to_string(dest.join(format!("{first_prefix}_base.txt")))
            .expect("read first base");
        let second_base = std::fs::read_to_string(dest.join(format!("{second_prefix}_base.txt")))
            .expect("read second base");
        assert_eq!(first_base, "base one\n");
        assert_eq!(second_base, "base two\n");

        let first_expected = std::fs::read_to_string(dest.join(format!(
            "{first_prefix}_expected_result.txt"
        )))
        .expect("read first expected result");
        let second_expected = std::fs::read_to_string(dest.join(format!(
            "{second_prefix}_expected_result.txt"
        )))
        .expect("read second expected result");
        assert!(
            !first_expected.is_empty(),
            "generated expected result should not be empty"
        );
        assert!(
            !second_expected.is_empty(),
            "generated expected result should not be empty"
        );
    }

    #[test]
    fn write_fixture_files_avoids_overwriting_existing_fixture_sets() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dest = tmp.path().join("fixtures");
        std::fs::create_dir_all(&dest).expect("create fixture dir");

        let existing_prefix = "abc12345_src_main_rs";
        std::fs::write(
            dest.join(format!("{existing_prefix}_base.txt")),
            "existing base\n",
        )
        .expect("write existing base");
        std::fs::write(
            dest.join(format!("{existing_prefix}_contrib1.txt")),
            "existing contrib1\n",
        )
        .expect("write existing contrib1");
        std::fs::write(
            dest.join(format!("{existing_prefix}_contrib2.txt")),
            "existing contrib2\n",
        )
        .expect("write existing contrib2");
        std::fs::write(
            dest.join(format!("{existing_prefix}_expected_result.txt")),
            "existing expected\n",
        )
        .expect("write existing expected");

        let case = ExtractedMergeCase {
            merge_commit: "abc12345".to_string(),
            file_path: "src/main.rs".to_string(),
            base: "new base\n".to_string(),
            contrib1: "new contrib1\n".to_string(),
            contrib2: "new contrib2\n".to_string(),
        };

        write_fixture_files(&[case], &dest).expect("write fixtures");

        // Existing fixture files remain unchanged.
        let existing_base =
            std::fs::read_to_string(dest.join(format!("{existing_prefix}_base.txt")))
                .expect("read existing base");
        assert_eq!(existing_base, "existing base\n");
        let existing_expected =
            std::fs::read_to_string(dest.join(format!("{existing_prefix}_expected_result.txt")))
                .expect("read existing expected");
        assert_eq!(existing_expected, "existing expected\n");

        // New extraction output is appended under the next available suffix.
        let appended_prefix = "abc12345_src_main_rs_1";
        let appended_base =
            std::fs::read_to_string(dest.join(format!("{appended_prefix}_base.txt")))
                .expect("read appended base");
        let appended_contrib1 =
            std::fs::read_to_string(dest.join(format!("{appended_prefix}_contrib1.txt")))
                .expect("read appended contrib1");
        let appended_contrib2 =
            std::fs::read_to_string(dest.join(format!("{appended_prefix}_contrib2.txt")))
                .expect("read appended contrib2");
        assert_eq!(appended_base, "new base\n");
        assert_eq!(appended_contrib1, "new contrib1\n");
        assert_eq!(appended_contrib2, "new contrib2\n");
        assert!(dest
            .join(format!("{appended_prefix}_expected_result.txt"))
            .exists());
    }

    #[test]
    fn discover_existing_fixture_prefixes_ignores_expected_only_files() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let dest = tmp.path().join("fixtures");
        std::fs::create_dir_all(&dest).expect("create fixture dir");

        std::fs::write(dest.join("abc_expected_result.txt"), "expected only\n")
            .expect("write expected-only fixture");

        let prefixes = discover_existing_fixture_prefixes(&dest).expect("discover prefixes");
        assert!(
            !prefixes.contains("abc"),
            "expected-only fixtures should not reserve prefixes"
        );
    }
}
