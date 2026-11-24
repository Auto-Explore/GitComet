//! Utilities for extracting non-trivial 3-way merge cases from git history.

use crate::merge::{MergeOptions, merge_file};
use crate::process::{bytes_to_text_preserving_utf8, git_command};
use rustc_hash::FxHashSet;
use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;

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
    NotGitRepository {
        path: PathBuf,
        stderr: String,
    },
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
            Self::NotGitRepository { path, stderr } => {
                if stderr.is_empty() {
                    write!(f, "{} is not a git repository", path.display())
                } else {
                    write!(f, "{} is not a git repository: {}", path.display(), stderr)
                }
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

        let base_bytes = read_blob_bytes_optional(repo, &base_sha, file_path)?.unwrap_or_default();
        let contrib1_bytes =
            read_blob_bytes_optional(repo, &merge.parent1_sha, file_path)?.unwrap_or_default();
        let contrib2_bytes =
            read_blob_bytes_optional(repo, &merge.parent2_sha, file_path)?.unwrap_or_default();

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

/// Write extracted merge cases as fixture files the merge fixture harness reads.
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
    let mut used_prefixes: FxHashSet<String> = FxHashSet::with_capacity_and_hasher(
        existing_prefixes.len() + cases.len(),
        Default::default(),
    );
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

        if should_generate_expected_result(&expected_path)? {
            // Generate a golden expected result from the current merge engine
            // so newly extracted fixtures are immediately runnable in the
            // fixture harness without manual bootstrapping.
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
) -> Result<FxHashSet<String>, MergeExtractionError> {
    let mut prefixes = FxHashSet::default();
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

fn allocate_unique_prefix(base_prefix: &str, used_prefixes: &mut FxHashSet<String>) -> String {
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
        Err(MergeExtractionError::NotGitRepository {
            path: repo.to_path_buf(),
            stderr: bytes_to_text_preserving_utf8(&output.stderr)
                .trim()
                .to_string(),
        })
    }
}

fn changed_files(
    repo: &Path,
    from: &str,
    to: &str,
) -> Result<BTreeSet<String>, MergeExtractionError> {
    let output = run_git(repo, &["diff", "--name-only", "-z", from, to])?;
    if !output.status.success() {
        return Err(MergeExtractionError::GitCommandFailed {
            command: git_command_string(&["diff", "--name-only", "-z", from, to]),
            stderr: bytes_to_text_preserving_utf8(&output.stderr)
                .trim()
                .to_string(),
        });
    }

    let mut files = BTreeSet::new();
    for raw_path in output.stdout.split(|byte| *byte == 0) {
        if raw_path.is_empty() {
            continue;
        }

        // Extraction fixtures currently store paths as UTF-8 strings.
        // Skip non-UTF8 paths instead of lossy conversion to avoid creating
        // invalid blob refs in later `git show <sha>:<path>` calls.
        if let Ok(path) = std::str::from_utf8(raw_path) {
            files.insert(path.to_string());
        }
    }

    Ok(files)
}

fn run_git_text(repo: &Path, args: &[&str]) -> Result<String, MergeExtractionError> {
    let output = run_git(repo, args)?;
    if !output.status.success() {
        return Err(MergeExtractionError::GitCommandFailed {
            command: git_command_string(args),
            stderr: bytes_to_text_preserving_utf8(&output.stderr)
                .trim()
                .to_string(),
        });
    }

    Ok(bytes_to_text_preserving_utf8(&output.stdout))
}

fn read_blob_bytes_optional(
    repo: &Path,
    commit_sha: &str,
    path: &str,
) -> Result<Option<Vec<u8>>, MergeExtractionError> {
    // Use `ls-tree` for locale-independent existence checks instead of parsing
    // `git show` stderr strings, which vary with localization.
    let existence_args = ["ls-tree", "-z", "--full-tree", commit_sha, "--", path];
    let existence = run_git(repo, &existence_args)?;
    if !existence.status.success() {
        return Err(MergeExtractionError::GitCommandFailed {
            command: git_command_string(&existence_args),
            stderr: bytes_to_text_preserving_utf8(&existence.stderr)
                .trim()
                .to_string(),
        });
    }
    if existence.stdout.is_empty() {
        return Ok(None);
    }

    // Blob exists at this path in this commit, so `git show <sha>:<path>`
    // should succeed and provides stable textual formatting for gitlinks.
    let object_ref = format!("{commit_sha}:{path}");
    let show_args = ["show", object_ref.as_str()];
    let show = run_git(repo, &show_args)?;
    if show.status.success() {
        Ok(Some(show.stdout))
    } else {
        Err(MergeExtractionError::GitCommandFailed {
            command: git_command_string(&show_args),
            stderr: bytes_to_text_preserving_utf8(&show.stderr)
                .trim()
                .to_string(),
        })
    }
}

fn run_git(repo: &Path, args: &[&str]) -> Result<Output, MergeExtractionError> {
    let mut command = git_command();
    command
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

fn should_generate_expected_result(path: &Path) -> Result<bool, MergeExtractionError> {
    match std::fs::read(path) {
        Ok(existing) => Ok(existing.iter().all(|byte| byte.is_ascii_whitespace())),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(source) => Err(MergeExtractionError::Io {
            action: "read file",
            path: path.to_path_buf(),
            source,
        }),
    }
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
mod tests;
