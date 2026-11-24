use super::*;
use crate::process::{
    GitExecutablePreference, current_git_executable_preference, install_git_executable_preference,
    lock_git_runtime_test,
};

// Keep merge-extraction tests isolated from process::tests, which mutate
// the shared git executable preference under this same mutex.
struct SystemGitRuntimeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    original: GitExecutablePreference,
}

impl SystemGitRuntimeGuard {
    fn new() -> Self {
        let lock = lock_git_runtime_test();
        let original = current_git_executable_preference();
        let _ = install_git_executable_preference(GitExecutablePreference::SystemPath);
        Self {
            _lock: lock,
            original,
        }
    }
}

impl Drop for SystemGitRuntimeGuard {
    fn drop(&mut self) {
        let _ = install_git_executable_preference(self.original.clone());
    }
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = crate::test_support::git_fixture::git_command()
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
        bytes_to_text_preserving_utf8(&output.stderr)
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
    std::fs::write(repo.join("img.bin"), b"\x89PNG\r\n\x1a\n\x00\x00base").expect("write img.bin");
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

    let output = crate::test_support::git_fixture::git_command()
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
    std::fs::write(repo.join("moddel.txt"), "branch b modify\n").expect("write moddel branch-b");
    std::fs::write(repo.join("addadd.txt"), "branch b add\n").expect("write addadd branch-b");
    run_git(repo, &["add", "moddel.txt", "addadd.txt"]);
    run_git(repo, &["commit", "-m", "branch-b modify+add"]);

    let output = crate::test_support::git_fixture::git_command()
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
    let _git_runtime = SystemGitRuntimeGuard::new();
    let repo = create_conflicting_merge_repo();
    let merges = discover_merge_commits(repo.path(), 10).expect("discover merges");
    assert_eq!(merges.len(), 1, "expected one merge commit");

    let merge = &merges[0];
    assert!(matches!(merge.merge_sha.len(), 40 | 64));
    assert_eq!(merge.parent1_sha.len(), merge.merge_sha.len());
    assert_eq!(merge.parent2_sha.len(), merge.merge_sha.len());
}

#[test]
fn discovers_merge_commits_from_repo_subdirectory() {
    let _git_runtime = SystemGitRuntimeGuard::new();
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
    let _git_runtime = SystemGitRuntimeGuard::new();
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
fn discover_merge_commits_reports_not_git_repository_stderr() {
    let _git_runtime = SystemGitRuntimeGuard::new();
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();
    let error = discover_merge_commits(repo, 1).expect_err("expected not-a-git-repository error");

    match error {
        MergeExtractionError::NotGitRepository { path, stderr } => {
            assert_eq!(path, repo);
            assert!(
                !stderr.is_empty(),
                "expected stderr details for repository validation failure"
            );
        }
        other => panic!("expected NotGitRepository, got {other:?}"),
    }
}

#[test]
fn discovers_merge_commits_after_recent_octopus_merges() {
    let _git_runtime = SystemGitRuntimeGuard::new();
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
    let _git_runtime = SystemGitRuntimeGuard::new();
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
#[cfg(not(windows))]
fn changed_files_handles_paths_with_newlines() {
    let _git_runtime = SystemGitRuntimeGuard::new();
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);

    let tricky_path = "dir/line\nbreak.txt";
    std::fs::create_dir_all(repo.join("dir")).expect("create dir");
    std::fs::write(repo.join(tricky_path), "base\n").expect("write base file");
    run_git(repo, &["add", "--", tricky_path]);
    run_git(repo, &["commit", "-m", "base"]);

    std::fs::write(repo.join(tricky_path), "changed\n").expect("write changed file");
    run_git(repo, &["add", "--", tricky_path]);
    run_git(repo, &["commit", "-m", "changed"]);

    let from = run_git_text(repo, &["rev-parse", "HEAD~1"]).expect("rev-parse HEAD~1");
    let to = run_git_text(repo, &["rev-parse", "HEAD"]).expect("rev-parse HEAD");
    let files = changed_files(repo, from.trim(), to.trim()).expect("changed files");

    assert_eq!(files.len(), 1, "expected exactly one changed path");
    assert!(
        files.contains(tricky_path),
        "expected changed path set to include {:?}, got {:?}",
        tricky_path,
        files
    );
}

#[test]
fn extracts_cases_with_missing_base_or_parent_as_empty_text() {
    let _git_runtime = SystemGitRuntimeGuard::new();
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
    let _git_runtime = SystemGitRuntimeGuard::new();
    let repo = create_conflicting_merge_repo();
    let missing = read_blob_bytes_optional(repo.path(), "HEAD", "this-file-does-not-exist")
        .expect("missing path should not error");
    assert!(
        missing.is_none(),
        "missing path should be interpreted as absent side content"
    );
}

#[test]
fn git_show_non_missing_errors_are_propagated() {
    let _git_runtime = SystemGitRuntimeGuard::new();
    let repo = create_conflicting_merge_repo();
    let err = read_blob_bytes_optional(repo.path(), "definitely-not-a-ref", "a.txt")
        .expect_err("invalid ref should surface as git command failure");

    match err {
        MergeExtractionError::GitCommandFailed { command, stderr } => {
            assert_eq!(
                command, "git ls-tree -z --full-tree definitely-not-a-ref -- a.txt",
                "unexpected git command context"
            );
            assert!(
                !stderr.is_empty(),
                "expected stderr details for invalid ref failure"
            );
        }
        other => panic!("expected GitCommandFailed, got {other:?}"),
    }
}

#[test]
fn read_blob_bytes_optional_reads_existing_blob_content() {
    let _git_runtime = SystemGitRuntimeGuard::new();
    let repo = create_conflicting_merge_repo();
    let content =
        read_blob_bytes_optional(repo.path(), "HEAD", "a.txt").expect("read existing path");
    let text = String::from_utf8(content.expect("expected blob content"))
        .expect("existing blob content should be UTF-8");
    assert_eq!(text, "resolved a\n");
}

#[test]
fn extracts_merge_cases_from_repo_subdirectory() {
    let _git_runtime = SystemGitRuntimeGuard::new();
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

    let base = std::fs::read_to_string(dest.join(format!("{prefix}_base.txt"))).expect("read base");
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
    let expected = std::fs::read_to_string(&expected_path).expect("read generated expected result");
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
fn write_fixture_files_backfills_whitespace_only_expected_placeholder() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let dest = tmp.path().join("fixtures");
    std::fs::create_dir_all(&dest).expect("create fixture dir");

    let case = ExtractedMergeCase {
        merge_commit: "1234abcd".to_string(),
        file_path: "src/placeholder.rs".to_string(),
        base: "base\n".to_string(),
        contrib1: "left\n".to_string(),
        contrib2: "right\n".to_string(),
    };
    let prefix = "1234abcd_src_placeholder_rs";
    let expected_path = dest.join(format!("{prefix}_expected_result.txt"));
    std::fs::write(&expected_path, " \n\t\n").expect("write placeholder expected result");

    write_fixture_files(std::slice::from_ref(&case), &dest).expect("write fixtures");

    let expected = std::fs::read_to_string(&expected_path).expect("read generated expected result");
    let merge_expected = merge_file(
        &case.base,
        &case.contrib1,
        &case.contrib2,
        &MergeOptions::default(),
    )
    .output;
    assert_eq!(
        expected, merge_expected,
        "whitespace-only placeholder expected result should be backfilled"
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

    let first_expected =
        std::fs::read_to_string(dest.join(format!("{first_prefix}_expected_result.txt")))
            .expect("read first expected result");
    let second_expected =
        std::fs::read_to_string(dest.join(format!("{second_prefix}_expected_result.txt")))
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
    let existing_base = std::fs::read_to_string(dest.join(format!("{existing_prefix}_base.txt")))
        .expect("read existing base");
    assert_eq!(existing_base, "existing base\n");
    let existing_expected =
        std::fs::read_to_string(dest.join(format!("{existing_prefix}_expected_result.txt")))
            .expect("read existing expected");
    assert_eq!(existing_expected, "existing expected\n");

    // New extraction output is appended under the next available suffix.
    let appended_prefix = "abc12345_src_main_rs_1";
    let appended_base = std::fs::read_to_string(dest.join(format!("{appended_prefix}_base.txt")))
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
    assert!(
        dest.join(format!("{appended_prefix}_expected_result.txt"))
            .exists()
    );
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
