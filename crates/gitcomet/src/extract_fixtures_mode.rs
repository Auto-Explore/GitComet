use crate::cli::{ExtractMergeFixturesConfig, exit_code};
use gitcomet_core::merge_extraction::{
    MergeExtractionOptions, extract_merge_cases_from_repo, write_fixture_files,
};

/// Result of running merge-fixture extraction mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractMergeFixturesRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub extracted_case_count: usize,
}

/// Extract non-trivial merge cases from a repository and write fixture files.
pub fn run_extract_merge_fixtures(
    config: &ExtractMergeFixturesConfig,
) -> Result<ExtractMergeFixturesRunResult, String> {
    if config.max_merges == 0 {
        return Err("Invalid --max-merges value '0': expected a positive integer.".to_string());
    }
    if config.max_files_per_merge == 0 {
        return Err(
            "Invalid --max-files-per-merge value '0': expected a positive integer.".to_string(),
        );
    }

    let options = MergeExtractionOptions {
        max_merges: config.max_merges,
        max_files_per_merge: config.max_files_per_merge,
    };

    let cases = extract_merge_cases_from_repo(&config.repo, options).map_err(|err| {
        format!(
            "Failed to extract merge cases from {}: {err}",
            config.repo.display()
        )
    })?;

    write_fixture_files(&cases, &config.output_dir).map_err(|err| {
        format!(
            "Failed to write fixture files to {}: {err}",
            config.output_dir.display()
        )
    })?;

    let stdout = if cases.is_empty() {
        format!(
            "No non-trivial merge cases found in {}.\nDestination: {}\n",
            config.repo.display(),
            config.output_dir.display(),
        )
    } else {
        format!(
            "Extracted {} merge case(s) from {}.\nWrote fixtures to {}.\n",
            cases.len(),
            config.repo.display(),
            config.output_dir.display(),
        )
    };

    Ok(ExtractMergeFixturesRunResult {
        stdout,
        stderr: String::new(),
        exit_code: exit_code::SUCCESS,
        extracted_case_count: cases.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command as ProcessCommand;

    fn run_git(repo: &Path, args: &[&str]) {
        let output = ProcessCommand::new("git")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {e}", args));
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string())
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

        fs::write(repo.join("a.txt"), "base\n").expect("write base");
        run_git(repo, &["add", "a.txt"]);
        run_git(repo, &["commit", "-m", "base"]);

        run_git(repo, &["checkout", "-b", "branch-a"]);
        fs::write(repo.join("a.txt"), "branch a\n").expect("write branch-a");
        run_git(repo, &["add", "a.txt"]);
        run_git(repo, &["commit", "-m", "branch-a change"]);

        run_git(repo, &["checkout", "main"]);
        run_git(repo, &["checkout", "-b", "branch-b"]);
        fs::write(repo.join("a.txt"), "branch b\n").expect("write branch-b");
        run_git(repo, &["add", "a.txt"]);
        run_git(repo, &["commit", "-m", "branch-b change"]);

        let output = ProcessCommand::new("git")
            .args(["merge", "branch-a", "--no-edit"])
            .current_dir(repo)
            .output()
            .expect("run merge");
        assert!(
            !output.status.success(),
            "expected merge conflict while creating fixture repo"
        );

        fs::write(repo.join("a.txt"), "resolved\n").expect("write resolution");
        run_git(repo, &["add", "a.txt"]);
        run_git(repo, &["commit", "-m", "merge commit"]);

        tmp
    }

    fn count_suffix(dir: &Path, suffix: &str) -> usize {
        fs::read_dir(dir)
            .expect("read output directory")
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
            .filter(|name| name.ends_with(suffix))
            .count()
    }

    #[test]
    fn run_extract_merge_fixtures_writes_fixture_sets() {
        let repo = create_conflicting_merge_repo();
        let out = tempfile::tempdir().expect("create output dir");
        let config = ExtractMergeFixturesConfig {
            repo: repo.path().to_path_buf(),
            output_dir: out.path().to_path_buf(),
            max_merges: 10,
            max_files_per_merge: 5,
        };

        let result = run_extract_merge_fixtures(&config).expect("run extraction mode");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.extracted_case_count >= 1,
            "expected at least one extracted merge case"
        );

        let base_count = count_suffix(out.path(), "_base.txt");
        let contrib1_count = count_suffix(out.path(), "_contrib1.txt");
        let contrib2_count = count_suffix(out.path(), "_contrib2.txt");
        let expected_count = count_suffix(out.path(), "_expected_result.txt");

        assert_eq!(base_count, result.extracted_case_count);
        assert_eq!(contrib1_count, result.extracted_case_count);
        assert_eq!(contrib2_count, result.extracted_case_count);
        assert_eq!(expected_count, result.extracted_case_count);
    }

    #[test]
    fn run_extract_merge_fixtures_errors_for_non_repo() {
        let non_repo = tempfile::tempdir().expect("create temp dir");
        let out = tempfile::tempdir().expect("create output dir");
        let config = ExtractMergeFixturesConfig {
            repo: non_repo.path().to_path_buf(),
            output_dir: out.path().to_path_buf(),
            max_merges: 5,
            max_files_per_merge: 2,
        };

        let err = run_extract_merge_fixtures(&config).expect_err("expected non-repo error");
        assert!(
            err.contains("not a git repository"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn run_extract_merge_fixtures_rejects_zero_max_merges() {
        let repo = create_conflicting_merge_repo();
        let out = tempfile::tempdir().expect("create output dir");
        let config = ExtractMergeFixturesConfig {
            repo: repo.path().to_path_buf(),
            output_dir: out.path().to_path_buf(),
            max_merges: 0,
            max_files_per_merge: 1,
        };

        let err = run_extract_merge_fixtures(&config).expect_err("expected validation error");
        assert_eq!(
            err,
            "Invalid --max-merges value '0': expected a positive integer."
        );
    }

    #[test]
    fn run_extract_merge_fixtures_rejects_zero_max_files_per_merge() {
        let repo = create_conflicting_merge_repo();
        let out = tempfile::tempdir().expect("create output dir");
        let config = ExtractMergeFixturesConfig {
            repo: repo.path().to_path_buf(),
            output_dir: out.path().to_path_buf(),
            max_merges: 1,
            max_files_per_merge: 0,
        };

        let err = run_extract_merge_fixtures(&config).expect_err("expected validation error");
        assert_eq!(
            err,
            "Invalid --max-files-per-merge value '0': expected a positive integer."
        );
    }
}
