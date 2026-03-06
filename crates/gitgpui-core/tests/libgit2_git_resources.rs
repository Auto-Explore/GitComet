use gitgpui_core::merge::{MergeOptions, merge_file};
use gitgpui_core::merge_extraction::{
    MergeExtractionOptions, discover_merge_commits, extract_merge_cases_from_repo,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn resource_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("libgit2")
        .join("resources")
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

struct RehydratedRepo {
    _tempdir: tempfile::TempDir,
    worktree_path: PathBuf,
}

fn rehydrate_worktree_repo(name: &str) -> RehydratedRepo {
    let src = resource_root().join(name);
    assert!(src.is_dir(), "missing resource repo: {}", src.display());
    assert!(
        src.join(".gitted").is_dir(),
        "expected .gitted dir in {}",
        src.display()
    );

    let tempdir = tempfile::tempdir().expect("create temp dir");
    let worktree_path = tempdir.path().join(name);
    copy_dir_recursive(&src, &worktree_path).expect("copy fixture repo");

    let gitted = worktree_path.join(".gitted");
    let git_dir = worktree_path.join(".git");
    fs::rename(&gitted, &git_dir).expect("rehydrate .gitted -> .git");

    RehydratedRepo {
        _tempdir: tempdir,
        worktree_path,
    }
}

fn run_git_in_worktree(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {e}", args));
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_git_with_git_dir(git_dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", git_dir.display()))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {e}", args));
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn libgit2_merge_resource_layout_matches_upstream_style() {
    let root = resource_root();

    for name in [
        "merge-recursive",
        "merge-resolve",
        "merge-whitespace",
        "mergedrepo",
    ] {
        let repo = root.join(name);
        assert!(repo.is_dir(), "missing {}", repo.display());
        assert!(
            repo.join(".gitted").is_dir(),
            "missing .gitted in {}",
            repo.display()
        );
        assert!(
            repo.join(".gitted").join("HEAD").is_file(),
            "missing .gitted/HEAD in {}",
            repo.display()
        );
    }

    for name in ["redundant.git", "twowaymerge.git"] {
        let repo = root.join(name);
        assert!(repo.is_dir(), "missing {}", repo.display());
        assert!(
            repo.join("HEAD").is_file(),
            "missing HEAD in {}",
            repo.display()
        );
        let is_bare = run_git_with_git_dir(&repo, &["rev-parse", "--is-bare-repository"]);
        assert_eq!(is_bare.trim(), "true", "{} should be bare", repo.display());
    }
}

#[test]
fn merge_recursive_rehydration_extracts_real_text_cases() {
    let repo = rehydrate_worktree_repo("merge-recursive");
    let git_dir = run_git_in_worktree(&repo.worktree_path, &["rev-parse", "--git-dir"]);
    assert_eq!(git_dir.trim(), ".git");

    let merges = discover_merge_commits(&repo.worktree_path, 20).expect("discover merges");
    assert!(
        !merges.is_empty(),
        "expected merge commits in merge-recursive"
    );

    let cases = extract_merge_cases_from_repo(
        &repo.worktree_path,
        MergeExtractionOptions {
            max_merges: 20,
            max_files_per_merge: 20,
        },
    )
    .expect("extract merge cases");

    assert!(!cases.is_empty(), "expected extracted merge cases");
    assert!(
        cases.iter().all(|case| {
            !case.base.contains("anonymous blob ")
                && !case.contrib1.contains("anonymous blob ")
                && !case.contrib2.contains("anonymous blob ")
        }),
        "merge-recursive should yield real text payloads"
    );
    assert!(
        cases.iter().any(|case| {
            case.base.contains("VEAL SOUP.")
                || case.contrib1.contains("VEAL SOUP.")
                || case.contrib2.contains("VEAL SOUP.")
        }),
        "expected known merge-recursive corpus text in extracted cases"
    );

    for case in cases.iter().take(12) {
        let result = merge_file(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &MergeOptions::default(),
        );
        assert!(
            !result.output.is_empty(),
            "merge output should not be empty for case {}:{}",
            case.merge_commit,
            case.file_path
        );
    }
}

#[test]
fn rehydrated_worktree_fixtures_match_libgit2_payloads() {
    let merge_resolve = rehydrate_worktree_repo("merge-resolve");
    assert_eq!(
        run_git_in_worktree(&merge_resolve.worktree_path, &["rev-parse", "--git-dir"]).trim(),
        ".git"
    );
    let conflicting =
        fs::read_to_string(merge_resolve.worktree_path.join("conflicting.txt")).expect("read");
    assert!(conflicting.contains("changed in master and branch"));

    let merge_whitespace = rehydrate_worktree_repo("merge-whitespace");
    let whitespace_text =
        fs::read_to_string(merge_whitespace.worktree_path.join("test.txt")).expect("read");
    assert!(
        whitespace_text
            .lines()
            .next()
            .is_some_and(|line| line == "0")
    );

    let mergedrepo = rehydrate_worktree_repo("mergedrepo");
    let conflicts_one =
        fs::read_to_string(mergedrepo.worktree_path.join("conflicts-one.txt")).expect("read");
    assert!(conflicts_one.contains("<<<<<<< HEAD"));
    assert!(conflicts_one.contains(">>>>>>>"));
}
