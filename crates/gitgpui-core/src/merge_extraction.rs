//! Utilities for extracting non-trivial 3-way merge cases from git history.
//!
//! This module ports the core workflow described in
//! `docs/REFERENCE_TEST_PORTABILITY.md` Phase 3C (real-world merge extraction)
//! into production code so it can be reused outside ad-hoc test harnesses.

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

        let base_bytes = match run_git_bytes_optional(repo, &["show", &base_ref])? {
            Some(bytes) => bytes,
            None => continue,
        };
        let contrib1_bytes = match run_git_bytes_optional(repo, &["show", &parent1_ref])? {
            Some(bytes) => bytes,
            None => continue,
        };
        let contrib2_bytes = match run_git_bytes_optional(repo, &["show", &parent2_ref])? {
            Some(bytes) => bytes,
            None => continue,
        };

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

    let mut used_prefixes: HashSet<String> = HashSet::with_capacity(cases.len());

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
            write_text_file(&expected_path, "")?;
        }
    }

    Ok(())
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
        Ok(None)
    }
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

        assert!(dest
            .join(format!("{first_prefix}_expected_result.txt"))
            .exists());
        assert!(dest
            .join(format!("{second_prefix}_expected_result.txt"))
            .exists());
    }
}
