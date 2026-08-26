use gitcomet_core::domain::{CommitId, FileStatusKind, HistoryMode, LogCursor};
use gitcomet_core::error::{ErrorKind, GitFailureId};
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::OnceLock;

fn run_git(repo: &Path, args: &[&str]) {
    run_git_with_env(repo, args, &[]);
}

fn run_git_with_env(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let cmd = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let status = cmd.status().expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .output()
        .expect("git command to run");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_integration_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_remote_tracking_test() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_integration_tests() {
            eprintln!(
                "skipping remote-tracking integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

fn git_remote_url(path: &Path) -> String {
    if cfg!(windows) {
        // Use a file:// URL so drive-letter paths are never treated as
        // scp-style host:path remotes.
        let normalized = path.to_string_lossy().replace('\\', "/");
        format!("file:///{normalized}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn git_force_file_transport_url(path: &Path) -> String {
    if cfg!(windows) {
        let normalized = path.to_string_lossy().replace('\\', "/");
        format!("file:///{normalized}")
    } else {
        // Force Git to use file:// transport so clone flags like `--depth`
        // are honored instead of falling back to the local-clone fast path.
        format!("file://{}", path.to_string_lossy())
    }
}

fn run_git_at(repo: &Path, args: &[&str], unix_seconds: i64) {
    let seconds = unix_seconds.rem_euclid(60);
    let minutes = unix_seconds.div_euclid(60).rem_euclid(60);
    let hours = unix_seconds.div_euclid(3_600).rem_euclid(24);
    let day = 1 + unix_seconds.div_euclid(86_400);
    let date = format!("2000-01-{day:02}T{hours:02}:{minutes:02}:{seconds:02}+0000");
    let envs = [
        ("GIT_AUTHOR_DATE", date.as_str()),
        ("GIT_COMMITTER_DATE", date.as_str()),
    ];
    run_git_with_env(repo, args, &envs);
}

fn fast_import_linear_history(repo: &Path, count: usize) {
    let mut stream = String::new();
    for index in 0..count {
        let message = format!("c{index}");
        stream.push_str("commit refs/heads/master\n");
        stream.push_str(&format!("mark :{}\n", index + 1));
        stream.push_str(&format!(
            "committer You <you@example.com> {} +0000\n",
            1_600_000_000 + index as i64
        ));
        stream.push_str(&format!("data {}\n{message}\n", message.len()));
        if index > 0 {
            stream.push_str(&format!("from :{}\n", index));
        }
        let body = format!("v{index}\n");
        stream.push_str(&format!(
            "M 100644 inline file.txt\ndata {}\n{body}",
            body.len()
        ));
    }
    stream.push_str("done\n");

    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let mut child = cmd
        .arg("-C")
        .arg(repo)
        .args(["fast-import", "--quiet", "--done"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("git fast-import to start");
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), stream.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success(), "git fast-import failed");
    run_git(repo, &["reset", "-q", "--hard", "master"]);
}

fn commit_file_at(repo: &Path, relative_path: &str, contents: &str, message: &str, time: i64) {
    let path = repo.join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, contents).unwrap();
    run_git(repo, &["add", relative_path]);
    run_git_at(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
        time,
    );
}

struct HistoryModeFixture {
    _dir: tempfile::TempDir,
    repo: std::path::PathBuf,
    base_id: String,
    feature_id: String,
    main_id: String,
    merge_id: String,
    side_id: String,
}

impl HistoryModeFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        run_git(&repo, &["init", "-b", "main"]);
        run_git(&repo, &["config", "user.email", "you@example.com"]);
        run_git(&repo, &["config", "user.name", "You"]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);

        commit_file_at(&repo, "base.txt", "base\n", "base", 1);
        let base_id = git_stdout(&repo, &["rev-parse", "HEAD"]);

        run_git(&repo, &["checkout", "-b", "feature"]);
        commit_file_at(&repo, "feature.txt", "feature\n", "feature", 2);
        let feature_id = git_stdout(&repo, &["rev-parse", "HEAD"]);

        run_git(&repo, &["checkout", "main"]);
        commit_file_at(&repo, "main.txt", "main\n", "main", 3);
        let main_id = git_stdout(&repo, &["rev-parse", "HEAD"]);

        run_git_at(
            &repo,
            &["merge", "--no-ff", "feature", "-m", "merge feature"],
            4,
        );
        let merge_id = git_stdout(&repo, &["rev-parse", "HEAD"]);

        run_git(&repo, &["checkout", "-b", "side", base_id.as_str()]);
        commit_file_at(&repo, "side.txt", "side\n", "side", 5);
        let side_id = git_stdout(&repo, &["rev-parse", "HEAD"]);

        run_git(&repo, &["checkout", "main"]);

        Self {
            _dir: dir,
            repo,
            base_id,
            feature_id,
            main_id,
            merge_id,
            side_id,
        }
    }

    fn repo(&self) -> &Path {
        &self.repo
    }
}

#[test]
fn history_modes_return_expected_commits_on_canonical_graph() {
    let fixture = HistoryModeFixture::new();
    let backend = GixBackend;
    let opened = backend.open(fixture.repo()).unwrap();

    let cases = [
        (
            HistoryMode::FullReachable,
            vec![
                fixture.merge_id.as_str(),
                fixture.main_id.as_str(),
                fixture.feature_id.as_str(),
                fixture.base_id.as_str(),
            ],
            vec![fixture.side_id.as_str()],
        ),
        (
            HistoryMode::FirstParent,
            vec![
                fixture.merge_id.as_str(),
                fixture.main_id.as_str(),
                fixture.base_id.as_str(),
            ],
            vec![fixture.feature_id.as_str(), fixture.side_id.as_str()],
        ),
        (
            HistoryMode::NoMerges,
            vec![
                fixture.main_id.as_str(),
                fixture.feature_id.as_str(),
                fixture.base_id.as_str(),
            ],
            vec![fixture.merge_id.as_str(), fixture.side_id.as_str()],
        ),
        (
            HistoryMode::MergesOnly,
            vec![fixture.merge_id.as_str()],
            vec![
                fixture.main_id.as_str(),
                fixture.feature_id.as_str(),
                fixture.base_id.as_str(),
                fixture.side_id.as_str(),
            ],
        ),
        (
            HistoryMode::AllBranches,
            vec![
                fixture.side_id.as_str(),
                fixture.merge_id.as_str(),
                fixture.main_id.as_str(),
                fixture.feature_id.as_str(),
                fixture.base_id.as_str(),
            ],
            Vec::new(),
        ),
    ];

    for (mode, expected_ids, excluded_ids) in cases {
        let page = opened.log_history_mode_page(mode, 20, None).unwrap();
        let ids = page
            .commits
            .iter()
            .map(|commit| commit.id.as_ref())
            .collect::<Vec<_>>();
        for expected_id in expected_ids {
            assert!(
                ids.contains(&expected_id),
                "{mode:?} should include {expected_id}, got {ids:?}"
            );
        }
        for excluded_id in excluded_ids {
            assert!(
                !ids.contains(&excluded_id),
                "{mode:?} should exclude {excluded_id}, got {ids:?}"
            );
        }
    }
}

#[test]
fn history_modes_preserve_expected_order_on_canonical_graph() {
    let fixture = HistoryModeFixture::new();
    let backend = GixBackend;
    let opened = backend.open(fixture.repo()).unwrap();

    let full_reachable = opened
        .log_history_mode_page(HistoryMode::FullReachable, 20, None)
        .unwrap();
    assert_eq!(
        full_reachable
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["merge feature", "main", "feature", "base"]
    );

    let first_parent = opened
        .log_history_mode_page(HistoryMode::FirstParent, 20, None)
        .unwrap();
    assert_eq!(
        first_parent
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["merge feature", "main", "base"]
    );

    let no_merges = opened
        .log_history_mode_page(HistoryMode::NoMerges, 20, None)
        .unwrap();
    assert_eq!(
        no_merges
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["main", "feature", "base"]
    );

    let merges_only = opened
        .log_history_mode_page(HistoryMode::MergesOnly, 20, None)
        .unwrap();
    assert_eq!(
        merges_only
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["merge feature"]
    );
}

#[test]
fn no_merges_history_mode_paginates_without_repeating_filtered_commits() {
    let fixture = HistoryModeFixture::new();
    let backend = GixBackend;
    let opened = backend.open(fixture.repo()).unwrap();

    let first = opened
        .log_history_mode_page(HistoryMode::NoMerges, 2, None)
        .unwrap();
    assert_eq!(
        first
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["main", "feature"]
    );
    let cursor = first.next_cursor.as_ref().expect("next cursor");
    assert!(
        cursor.resume_token.is_some(),
        "filtered history pagination should provide an opaque resume token"
    );

    let second = opened
        .log_history_mode_page(HistoryMode::NoMerges, 2, Some(cursor))
        .unwrap();
    assert_eq!(
        second
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["base"]
    );
    assert!(
        second
            .commits
            .iter()
            .all(|commit| first.commits.iter().all(|first| first.id != commit.id)),
        "filtered pagination should not repeat commits across pages"
    );

    let stale_cursor = LogCursor {
        last_seen: first.commits[1].id.clone(),
        resume_from: None,
        resume_token: Some(Arc::from("stale")),
    };
    let stale_second = opened
        .log_history_mode_page(HistoryMode::NoMerges, 2, Some(&stale_cursor))
        .unwrap();
    assert_eq!(stale_second.commits, second.commits);

    let legacy_cursor = LogCursor {
        last_seen: first.commits[1].id.clone(),
        resume_from: None,
        resume_token: None,
    };
    let legacy_second = opened
        .log_history_mode_page(HistoryMode::NoMerges, 2, Some(&legacy_cursor))
        .unwrap();
    assert_eq!(legacy_second.commits, second.commits);
}

#[test]
fn full_reachable_history_mode_paginates_without_repeating_commits() {
    let fixture = HistoryModeFixture::new();
    let backend = GixBackend;
    let opened = backend.open(fixture.repo()).unwrap();

    let first = opened
        .log_history_mode_page(HistoryMode::FullReachable, 2, None)
        .unwrap();
    assert_eq!(
        first
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["merge feature", "main"]
    );
    let cursor = first.next_cursor.as_ref().expect("next cursor");
    assert!(
        cursor.resume_token.is_some(),
        "full-reachable pagination should provide an opaque resume token"
    );

    let second = opened
        .log_history_mode_page(HistoryMode::FullReachable, 2, Some(cursor))
        .unwrap();
    assert_eq!(
        second
            .commits
            .iter()
            .map(|commit| commit.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["feature", "base"]
    );
    assert!(
        second
            .commits
            .iter()
            .all(|commit| first.commits.iter().all(|first| first.id != commit.id)),
        "full-reachable pagination should not repeat commits across pages"
    );

    let stale_cursor = LogCursor {
        last_seen: first.commits[1].id.clone(),
        resume_from: None,
        resume_token: Some(Arc::from("stale")),
    };
    let stale_second = opened
        .log_history_mode_page(HistoryMode::FullReachable, 2, Some(&stale_cursor))
        .unwrap();
    assert_eq!(stale_second.commits, second.commits);

    let legacy_cursor = LogCursor {
        last_seen: first.commits[1].id.clone(),
        resume_from: None,
        resume_token: None,
    };
    let legacy_second = opened
        .log_history_mode_page(HistoryMode::FullReachable, 2, Some(&legacy_cursor))
        .unwrap();
    assert_eq!(legacy_second.commits, second.commits);
}

#[test]
fn every_history_mode_paginates_through_a_resumable_walk() {
    let fixture = HistoryModeFixture::new();
    let backend = GixBackend;
    let opened = backend.open(fixture.repo()).unwrap();

    for mode in [
        HistoryMode::FullReachable,
        HistoryMode::FirstParent,
        HistoryMode::NoMerges,
        HistoryMode::MergesOnly,
        HistoryMode::AllBranches,
    ] {
        let whole = opened.log_history_mode_page(mode, 20, None).unwrap();
        assert!(
            whole.next_cursor.is_none(),
            "{mode:?}: the fixture is meant to fit in one page"
        );

        let mut paged: Vec<String> = Vec::new();
        let mut cursor = None;
        for _ in 0..whole.commits.len() + 1 {
            let page = opened
                .log_history_mode_page(mode, 1, cursor.as_ref())
                .unwrap();
            paged.extend(page.commits.iter().map(|c| c.id.as_ref().to_string()));
            let Some(next) = page.next_cursor else {
                break;
            };
            assert!(
                next.resume_token.is_some(),
                "{mode:?}: a page with more to give must hand back a resumable walk"
            );
            cursor = Some(next);
        }

        assert_eq!(
            paged,
            whole
                .commits
                .iter()
                .map(|c| c.id.as_ref().to_string())
                .collect::<Vec<_>>(),
            "{mode:?}: paging one at a time must visit the same commits, in the same order"
        );
    }
}

#[test]
fn all_branches_author_filter_resumes_instead_of_re_walking() {
    let fixture = HistoryModeFixture::new();
    let backend = GixBackend;
    let opened = backend.open(fixture.repo()).unwrap();

    let all = opened
        .log_history_mode_page_filtered(HistoryMode::AllBranches, Some("You"), 20, None)
        .unwrap();
    assert!(
        all.commits.len() > 1,
        "the fixture's commits are all by You"
    );

    let first = opened
        .log_history_mode_page_filtered(HistoryMode::AllBranches, Some("You"), 1, None)
        .unwrap();
    let cursor = first.next_cursor.expect("more to page through");
    assert!(
        cursor.resume_token.is_some(),
        "a filtered all-branches page must resume its walk rather than rebuild it"
    );

    let second = opened
        .log_history_mode_page_filtered(HistoryMode::AllBranches, Some("You"), 1, Some(&cursor))
        .unwrap();
    assert_eq!(
        second.commits.first().map(|c| c.id.as_ref()),
        all.commits.get(1).map(|c| c.id.as_ref()),
        "the resumed walk must continue where the first page stopped"
    );
}

#[test]
fn shallow_history_modes_paginate_and_stop_at_the_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let origin_work = dir.path().join("origin-work");
    let origin_bare = dir.path().join("origin.git");
    let shallow = dir.path().join("shallow");
    std::fs::create_dir_all(&origin_work).unwrap();

    run_git(&origin_work, &["init", "-b", "main"]);
    run_git(&origin_work, &["config", "user.email", "you@example.com"]);
    run_git(&origin_work, &["config", "user.name", "You"]);
    run_git(&origin_work, &["config", "commit.gpgsign", "false"]);

    commit_file_at(&origin_work, "base.txt", "base\n", "base", 1);
    let base_id = git_stdout(&origin_work, &["rev-parse", "HEAD"]);

    commit_file_at(&origin_work, "middle.txt", "middle\n", "middle", 2);
    let middle_id = git_stdout(&origin_work, &["rev-parse", "HEAD"]);

    commit_file_at(&origin_work, "tip.txt", "tip\n", "tip", 3);
    let tip_id = git_stdout(&origin_work, &["rev-parse", "HEAD"]);

    let origin_work_str = origin_work.to_string_lossy().to_string();
    let origin_bare_str = origin_bare.to_string_lossy().to_string();
    let shallow_str = shallow.to_string_lossy().to_string();
    run_git(
        dir.path(),
        &[
            "clone",
            "--bare",
            origin_work_str.as_str(),
            origin_bare_str.as_str(),
        ],
    );
    let origin_url = git_force_file_transport_url(&origin_bare);
    run_git(
        dir.path(),
        &[
            "clone",
            "--depth",
            "2",
            origin_url.as_str(),
            shallow_str.as_str(),
        ],
    );
    assert_eq!(
        git_stdout(&shallow, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );

    let backend = GixBackend;
    let opened = backend.open(&shallow).unwrap();

    for mode in [HistoryMode::FullReachable, HistoryMode::NoMerges] {
        let first = opened.log_history_mode_page(mode, 1, None).unwrap();
        assert_eq!(first.commits.len(), 1);
        assert_eq!(first.commits[0].id.as_ref(), tip_id.as_str());
        let cursor = first.next_cursor.as_ref().expect("next cursor");
        assert_eq!(cursor.last_seen.as_ref(), tip_id.as_str());
        assert!(
            cursor.resume_token.is_some(),
            "a shallow repository resumes its walk like any other"
        );

        let second = opened.log_history_mode_page(mode, 1, Some(cursor)).unwrap();
        assert_eq!(second.commits.len(), 1);
        assert_eq!(second.commits[0].id.as_ref(), middle_id.as_str());
        assert!(second.next_cursor.is_none());
        assert_ne!(
            second.commits[0].id.as_ref(),
            base_id.as_str(),
            "depth-2 clone should not expose commits beyond the shallow boundary"
        );
    }
}

#[test]
fn merges_only_history_mode_paginates_without_repeating_filtered_merges() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    commit_file_at(repo, "base.txt", "base\n", "base", 1);

    run_git(repo, &["checkout", "-b", "feature-one"]);
    commit_file_at(repo, "feature-one.txt", "feature one\n", "feature one", 2);

    run_git(repo, &["checkout", "main"]);
    commit_file_at(repo, "main-one.txt", "main one\n", "main one", 3);
    run_git_at(
        repo,
        &["merge", "--no-ff", "feature-one", "-m", "merge feature one"],
        4,
    );
    let merge_one = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["checkout", "-b", "feature-two"]);
    commit_file_at(repo, "feature-two.txt", "feature two\n", "feature two", 5);

    run_git(repo, &["checkout", "main"]);
    commit_file_at(repo, "main-two.txt", "main two\n", "main two", 6);
    run_git_at(
        repo,
        &["merge", "--no-ff", "feature-two", "-m", "merge feature two"],
        7,
    );
    let merge_two = git_stdout(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened
        .log_history_mode_page(HistoryMode::MergesOnly, 1, None)
        .unwrap();
    assert_eq!(first.commits.len(), 1);
    assert_eq!(first.commits[0].id.as_ref(), merge_two.as_str());
    let cursor = first
        .next_cursor
        .as_ref()
        .expect("next cursor for second merge");
    assert!(
        cursor.resume_token.is_some(),
        "filtered history pagination should provide an opaque resume token"
    );

    let second = opened
        .log_history_mode_page(HistoryMode::MergesOnly, 1, Some(cursor))
        .unwrap();
    assert_eq!(second.commits.len(), 1);
    assert_eq!(second.commits[0].id.as_ref(), merge_one.as_str());
    assert!(second.next_cursor.is_none());
    assert_ne!(first.commits[0].id, second.commits[0].id);

    let stale_cursor = LogCursor {
        last_seen: first.commits[0].id.clone(),
        resume_from: None,
        resume_token: Some(Arc::from("stale")),
    };
    let stale_second = opened
        .log_history_mode_page(HistoryMode::MergesOnly, 1, Some(&stale_cursor))
        .unwrap();
    assert_eq!(stale_second.commits, second.commits);

    let legacy_cursor = LogCursor {
        last_seen: first.commits[0].id.clone(),
        resume_from: None,
        resume_token: None,
    };
    let legacy_second = opened
        .log_history_mode_page(HistoryMode::MergesOnly, 1, Some(&legacy_cursor))
        .unwrap();
    assert_eq!(legacy_second.commits, second.commits);
}

#[test]
fn log_all_branches_includes_remote_tracking_branches() {
    if !require_git_shell_for_remote_tracking_test() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");

    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);
    let feature_tip = git_stdout(&repo, &["rev-parse", "HEAD"]);

    run_git(
        dir.path(),
        &["init", "--bare", "-b", "main", origin.to_str().unwrap()],
    );
    let origin_url = git_remote_url(&origin);
    run_git(&repo, &["remote", "add", "origin", origin_url.as_str()]);
    run_git(&repo, &["push", "-u", "origin", "feature"]);

    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["branch", "-D", "feature"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();

    let head = opened.log_head_page(200, None).unwrap();
    assert!(
        !head.commits.iter().any(|c| c.id.as_ref() == feature_tip),
        "head log unexpectedly contains feature commit"
    );

    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        all.commits.iter().any(|c| c.id.as_ref() == feature_tip),
        "all-branches log should include remote-tracking branch commit"
    );
}

#[test]
fn log_all_branches_orders_rows_like_git_date_order_under_committer_date_skew() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "master"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);

    let commit_at = |name: &str, date: &str| {
        std::fs::write(repo.join(format!("{name}.txt")), name).unwrap();
        run_git(repo, &["add", "-A"]);
        run_git_with_env(
            repo,
            &["-c", "commit.gpgsign=false", "commit", "-m", name],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    };

    commit_at("base", "2026-08-20T10:00:00+0000");
    commit_at("master-tip", "2026-08-20T12:00:00+0000");
    run_git(repo, &["checkout", "-q", "-b", "topic-b", "master"]);
    commit_at("topic-b-tip", "2026-08-20T11:50:00+0000");
    run_git(repo, &["checkout", "-q", "-b", "topic-a", "master"]);
    commit_at("topic-a-tip", "2026-08-20T12:10:00+0000");
    run_git(repo, &["checkout", "-q", "master"]);

    let expected: Vec<String> = git_stdout(repo, &["log", "--all", "--date-order", "--format=%H"])
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(expected.len(), 4, "expected a four-commit history");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let page = opened.log_all_branches_page(200, None).unwrap();
    let actual: Vec<String> = page
        .commits
        .iter()
        .map(|commit| commit.id.as_ref().to_owned())
        .collect();

    let summaries: Vec<&str> = page
        .commits
        .iter()
        .map(|commit| commit.summary.as_ref())
        .collect();
    assert_eq!(
        actual, expected,
        "log rows must match `git log --all --date-order`; got {summaries:?}"
    );
    assert_eq!(
        summaries,
        vec!["topic-a-tip", "topic-b-tip", "master-tip", "base"],
        "a parent must not be listed before its own child"
    );
}

#[test]
fn every_history_mode_matches_git_row_order_under_committer_date_skew() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "master"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);

    let commit_at = |name: &str, date: &str| {
        std::fs::write(repo.join(format!("{name}.txt")), name).unwrap();
        run_git(repo, &["add", "-A"]);
        run_git_with_env(
            repo,
            &["-c", "commit.gpgsign=false", "commit", "-m", name],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    };
    let merge_at = |branch: &str, name: &str, date: &str| {
        run_git_with_env(
            repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "merge",
                "--no-ff",
                "-m",
                name,
                branch,
            ],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    };

    commit_at("base", "2026-08-20T10:00:00+0000");
    commit_at("P", "2026-08-20T12:00:00+0000");
    let fork = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["checkout", "-q", "-b", "topic-b", "master"]);
    commit_at("X", "2026-08-20T11:50:00+0000");
    run_git(repo, &["checkout", "-q", "-b", "topic-a", "master"]);
    commit_at("Y", "2026-08-20T12:10:00+0000");
    run_git(repo, &["checkout", "-q", "-b", "topic-c", fork.as_str()]);
    commit_at("Z", "2026-08-20T11:40:00+0000");

    run_git(repo, &["checkout", "-q", "master"]);
    merge_at("topic-b", "M", "2026-08-20T12:20:00+0000");
    merge_at("topic-c", "N", "2026-08-20T11:45:00+0000");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    for (mode, git_args) in [
        (
            HistoryMode::FullReachable,
            vec!["log", "--date-order", "--format=%s"],
        ),
        (
            HistoryMode::FirstParent,
            vec!["log", "--date-order", "--first-parent", "--format=%s"],
        ),
        (
            HistoryMode::NoMerges,
            vec!["log", "--date-order", "--no-merges", "--format=%s"],
        ),
        (
            HistoryMode::MergesOnly,
            vec!["log", "--date-order", "--merges", "--format=%s"],
        ),
        (
            HistoryMode::AllBranches,
            vec!["log", "--all", "--date-order", "--format=%s"],
        ),
    ] {
        let expected: Vec<String> = git_stdout(repo, &git_args)
            .lines()
            .map(str::to_owned)
            .collect();
        let page = opened.log_history_mode_page(mode, 50, None).unwrap();
        let actual: Vec<String> = page
            .commits
            .iter()
            .map(|commit| commit.summary.to_string())
            .collect();
        assert_eq!(
            actual, expected,
            "{mode:?} rows must match `git {git_args:?}`"
        );
    }

    let first_parent = opened
        .log_history_mode_page(HistoryMode::FirstParent, 50, None)
        .unwrap();
    assert!(
        first_parent
            .commits
            .iter()
            .all(|commit| commit.parent_ids.len() <= 1),
        "first-parent rows must report only the parent the walk followed"
    );
    let full = opened
        .log_history_mode_page(HistoryMode::FullReachable, 50, None)
        .unwrap();
    assert!(
        full.commits
            .iter()
            .any(|commit| commit.parent_ids.len() == 2),
        "the merges must still report both parents everywhere else"
    );
}

#[test]
fn the_all_branches_page_cache_is_invalidated_by_every_kind_of_ref_move() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-b", "master"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);

    let mut clock = 0i64;
    let mut commit = |name: &str| {
        clock += 60;
        commit_file_at(repo, &format!("{name}.txt"), name, name, clock);
        clock
    };

    commit("one");
    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let summaries = || -> Vec<String> {
        opened
            .log_all_branches_page(50, None)
            .unwrap()
            .commits
            .iter()
            .map(|commit| commit.summary.to_string())
            .collect()
    };

    assert_eq!(summaries(), vec!["one"]);
    assert_eq!(summaries(), vec!["one"]);

    commit("two");
    assert_eq!(summaries(), vec!["two", "one"]);

    run_git(repo, &["checkout", "-q", "-b", "side"]);
    commit("three");
    run_git(repo, &["checkout", "-q", "master"]);
    assert_eq!(summaries(), vec!["three", "two", "one"]);

    std::fs::write(repo.join("one.txt"), "dirty").unwrap();
    run_git(repo, &["stash", "push", "-m", "stashed"]);
    let with_stash = summaries();
    assert_eq!(
        with_stash.len(),
        5,
        "expected the stash commit and its index parent, got {with_stash:?}"
    );
    assert!(
        with_stash.iter().any(|row| row == "On master: stashed"),
        "the stash commit must be a row, got {with_stash:?}"
    );
    assert!(
        with_stash
            .iter()
            .any(|row| row.starts_with("index on master:")),
        "the stash's index commit must be a row, got {with_stash:?}"
    );
    let kept: Vec<&String> = with_stash
        .iter()
        .filter(|row| *row != "On master: stashed" && !row.starts_with("index on master:"))
        .collect();
    assert_eq!(
        kept,
        ["three", "two", "one"],
        "the rows that were already there must be unchanged, got {with_stash:?}"
    );

    run_git(repo, &["stash", "drop"]);
    run_git(repo, &["branch", "-D", "side"]);
    let after_delete = summaries();
    assert_eq!(
        after_delete,
        vec!["two", "one"],
        "deleting a branch must drop the commits only it reached"
    );

    // Tags are not all-branches tips.
    run_git(repo, &["tag", "-a", "v1", "-m", "v1", "master"]);
    assert_eq!(
        summaries(),
        vec!["two", "one"],
        "a tag on a commit that is already reachable changes nothing"
    );
    let unreachable = git_stdout(repo, &["rev-parse", "master"]);
    let _ = commit("four");
    run_git(repo, &["tag", "orphan", unreachable.as_str()]);
    run_git(repo, &["update-ref", "refs/heads/master", &unreachable]);
    run_git(repo, &["reset", "-q", "--hard", "master"]);
    assert_eq!(
        summaries(),
        vec!["two", "one"],
        "a commit reachable only through a tag stays out of the graph"
    );
}

#[test]
fn deepening_a_shallow_clone_invalidates_the_all_branches_page_cache() {
    let dir = tempfile::tempdir().unwrap();
    let origin_work = dir.path().join("origin-work");
    let origin_bare = dir.path().join("origin.git");
    let shallow = dir.path().join("shallow");
    std::fs::create_dir_all(&origin_work).unwrap();

    run_git(&origin_work, &["init", "-b", "main"]);
    run_git(&origin_work, &["config", "user.email", "you@example.com"]);
    run_git(&origin_work, &["config", "user.name", "You"]);
    run_git(&origin_work, &["config", "commit.gpgsign", "false"]);
    for (index, name) in ["base", "one", "two", "three", "tip"].iter().enumerate() {
        commit_file_at(
            &origin_work,
            &format!("{name}.txt"),
            &format!("{name}\n"),
            name,
            index as i64 + 1,
        );
    }

    let origin_work_str = origin_work.to_string_lossy().to_string();
    let origin_bare_str = origin_bare.to_string_lossy().to_string();
    let shallow_str = shallow.to_string_lossy().to_string();
    run_git(
        dir.path(),
        &[
            "clone",
            "--bare",
            origin_work_str.as_str(),
            origin_bare_str.as_str(),
        ],
    );
    let origin_url = git_force_file_transport_url(&origin_bare);
    run_git(
        dir.path(),
        &[
            "clone",
            "--depth",
            "2",
            origin_url.as_str(),
            shallow_str.as_str(),
        ],
    );

    let opened = GixBackend.open(&shallow).unwrap();
    let tips_before = git_stdout(
        &shallow,
        &["for-each-ref", "--format=%(objectname) %(refname)"],
    );
    let truncated = opened.log_all_branches_page(50, None).unwrap();
    assert_eq!(
        truncated.commits.len(),
        2,
        "a depth-2 clone starts with two rows"
    );

    run_git(&shallow, &["fetch", "--unshallow"]);
    assert_eq!(
        git_stdout(&shallow, &["rev-parse", "--is-shallow-repository"]),
        "false"
    );
    assert_eq!(
        git_stdout(
            &shallow,
            &["for-each-ref", "--format=%(objectname) %(refname)"]
        ),
        tips_before,
        "the deepen must not move a ref, or the test proves nothing"
    );

    let deepened = opened.log_all_branches_page(50, None).unwrap();
    assert_eq!(
        deepened.commits.len(),
        5,
        "the deepened history must not be served from the page cached before the deepen"
    );
}

#[test]
fn a_missing_ancestor_object_still_renders_the_rows_above_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-b", "master"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    fast_import_linear_history(repo, 600);

    let root = git_stdout(repo, &["rev-list", "--max-parents=0", "HEAD"]);
    // fast-import packs; unpack so a single commit object can be removed.
    let pack_dir = repo.join(".git/objects/pack");
    let packs: Vec<std::path::PathBuf> = std::fs::read_dir(&pack_dir)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension().is_some_and(|ext| ext == "pack")).then_some(path)
        })
        .collect();
    for pack in &packs {
        // The pack has to leave the object store first: `unpack-objects` skips
        // anything the repository can already find.
        let bytes = std::fs::read(pack).unwrap();
        std::fs::remove_file(pack).unwrap();
        std::fs::remove_file(pack.with_extension("idx")).ok();
        let mut cmd = Command::new("git");
        test_git_env::apply(&mut cmd);
        let mut child = cmd
            .arg("-C")
            .arg(repo)
            .args(["unpack-objects", "-q"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), &bytes).unwrap();
        assert!(child.wait().unwrap().success(), "git unpack-objects failed");
    }

    let loose = repo.join(".git/objects").join(&root[..2]).join(&root[2..]);
    assert!(loose.exists(), "expected a loose root commit at {loose:?}");
    std::fs::remove_file(&loose).unwrap();

    let opened = GixBackend.open(repo).unwrap();
    let page = opened
        .log_history_mode_page(HistoryMode::FullReachable, 2, None)
        .expect("a page that stops above the hole must not fail");
    let summaries: Vec<&str> = page
        .commits
        .iter()
        .map(|commit| commit.summary.as_ref())
        .collect();
    assert_eq!(
        summaries,
        vec!["c599", "c598"],
        "the rows above the hole must still render"
    );
}

#[test]
fn log_all_branches_includes_nonstandard_ref_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);
    let feature_tip = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["branch", "-D", "feature"]);
    run_git(
        repo,
        &[
            "update-ref",
            "refs/branch-heads/feature",
            feature_tip.as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        all.commits.iter().any(|c| c.id.as_ref() == feature_tip),
        "all-branches log should include commits reachable from refs outside refs/heads and refs/remotes"
    );
}

#[test]
fn log_all_branches_does_not_include_tag_only_tips() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(repo, &["checkout", "-b", "tag-only"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);
    let tag_only_tip = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(
        repo,
        &[
            "-c",
            "tag.gpgSign=false",
            "tag",
            "-a",
            "-m",
            "tag",
            "v0.0",
            tag_only_tip.as_str(),
        ],
    );
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["branch", "-D", "tag-only"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        !all.commits.iter().any(|c| c.id.as_ref() == tag_only_tip),
        "all-branches log should not be expanded by tag-only tips"
    );
}

#[test]
fn log_all_branches_ignores_non_commit_refs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);
    let head = git_stdout(repo, &["rev-parse", "HEAD"]);

    let blob = git_stdout(repo, &["hash-object", "-w", "a.txt"]);
    run_git(repo, &["update-ref", "refs/blob-test", blob.as_str()]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();

    assert_eq!(all.commits.len(), 1);
    assert_eq!(all.commits[0].id.as_ref(), head);
    assert_eq!(&*all.commits[0].summary, "A");
}

#[test]
fn empty_repo_log_and_head_branch_do_not_error() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert_eq!(opened.current_branch().unwrap(), "main");
    assert!(opened.log_head_page(200, None).unwrap().commits.is_empty());
    assert!(
        opened
            .log_all_branches_page(200, None)
            .unwrap()
            .commits
            .is_empty()
    );
}

#[test]
fn detached_head_reports_head_as_current_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);
    run_git(repo, &["checkout", "--detach", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    assert_eq!(opened.current_branch().unwrap(), "HEAD");
}

#[test]
fn log_head_page_limit_sets_next_cursor_and_supports_pagination() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened.log_head_page(1, None).unwrap();
    assert_eq!(first.commits.len(), 1);
    let first_id = first.commits[0].id.as_ref().to_string();
    let cursor = first.next_cursor.as_ref().expect("next cursor");
    let expected_resume = first.commits[0]
        .parent_ids
        .first()
        .cloned()
        .expect("resume hint should point at next first-parent commit");
    assert_eq!(cursor.resume_from.as_ref(), Some(&expected_resume));

    let second = opened.log_head_page(10, Some(cursor)).unwrap();
    assert!(!second.commits.is_empty());
    assert!(
        second.commits.iter().all(|c| c.id.as_ref() != first_id),
        "paginated page should skip last-seen commit"
    );

    let legacy_cursor = LogCursor {
        last_seen: first.commits[0].id.clone(),
        resume_from: None,
        resume_token: None,
    };
    let legacy_second = opened.log_head_page(10, Some(&legacy_cursor)).unwrap();
    assert_eq!(legacy_second.commits, second.commits);
}

#[test]
fn log_head_page_resume_hint_follows_first_parent_after_merge_commit() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("feature.txt"), "feature\n").unwrap();
    run_git(repo, &["add", "feature.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    let feature_tip = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["checkout", "main"]);
    std::fs::write(repo.join("main.txt"), "main\n").unwrap();
    run_git(repo, &["add", "main.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );
    let first_parent_tip = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(
        repo,
        &["merge", "--no-ff", "feature", "-m", "merge feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened.log_head_page(1, None).unwrap();
    assert_eq!(first.commits.len(), 1);
    assert_eq!(&*first.commits[0].summary, "merge feature");
    assert_eq!(
        first.commits[0].parent_ids.first().map(CommitId::as_ref),
        Some(first_parent_tip.as_str())
    );

    let cursor = first.next_cursor.as_ref().expect("next cursor");
    assert_eq!(
        cursor.resume_from,
        Some(CommitId(first_parent_tip.clone().into()))
    );

    let second = opened.log_head_page(10, Some(cursor)).unwrap();
    let second_summaries: Vec<&str> = second.commits.iter().map(|c| &*c.summary).collect();
    assert_eq!(second_summaries, vec!["main", "base"]);
    assert!(
        second.commits.iter().all(|c| c.id.as_ref() != feature_tip),
        "first-parent pagination should not revisit merged side-branch commits"
    );

    let legacy_cursor = LogCursor {
        last_seen: first.commits[0].id.clone(),
        resume_from: None,
        resume_token: None,
    };
    let legacy_second = opened.log_head_page(10, Some(&legacy_cursor)).unwrap();
    assert_eq!(legacy_second.commits, second.commits);
}

#[test]
fn log_head_page_exact_limit_has_no_next_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let page = opened.log_head_page(2, None).unwrap();
    assert_eq!(page.commits.len(), 2);
    assert!(page.next_cursor.is_none());
}

#[test]
fn repeated_log_head_page_reuses_cached_commit_arcs_and_invalidates_on_head_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened.log_head_page(2, None).unwrap();
    let second = opened.log_head_page(2, None).unwrap();

    assert_eq!(first.commits, second.commits);
    assert!(Arc::ptr_eq(&first.commits[0].id.0, &second.commits[0].id.0));
    assert!(Arc::ptr_eq(
        &first.commits[0].summary,
        &second.commits[0].summary
    ));
    assert!(Arc::ptr_eq(
        &first.commits[0].parent_ids[0].0,
        &second.commits[0].parent_ids[0].0
    ));

    std::fs::write(repo.join("a.txt"), "three\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);

    let refreshed = opened.log_head_page(2, None).unwrap();
    let summaries: Vec<&str> = refreshed
        .commits
        .iter()
        .map(|commit| &*commit.summary)
        .collect();
    assert_eq!(summaries, vec!["C", "B"]);
}

#[test]
fn zero_limit_log_pages_return_empty_without_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let head = opened.log_head_page(0, None).unwrap();
    assert!(head.commits.is_empty());
    assert!(head.next_cursor.is_none());

    let all = opened.log_all_branches_page(0, None).unwrap();
    assert!(all.commits.is_empty());
    assert!(all.next_cursor.is_none());

    let file = opened.log_file_page(Path::new("a.txt"), 0, None).unwrap();
    assert!(file.commits.is_empty());
    assert!(file.next_cursor.is_none());
}

#[test]
fn log_file_page_follows_renames() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/old name.txt"), "line 1\n").unwrap();
    run_git(repo, &["add", "docs/old name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add history file",
        ],
    );

    run_git(repo, &["mv", "docs/old name.txt", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "rename history file",
        ],
    );

    std::fs::write(repo.join("docs/new name.txt"), "line 1\nline 2\n").unwrap();
    run_git(repo, &["add", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update history file",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let page = opened
        .log_file_page(Path::new("docs/new name.txt"), 10, None)
        .unwrap();
    let summaries: Vec<&str> = page.commits.iter().map(|c| &*c.summary).collect();

    assert_eq!(
        summaries,
        vec![
            "update history file",
            "rename history file",
            "add history file"
        ]
    );
}

#[test]
fn log_file_page_cursor_paginates_rename_follow_history() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/old name.txt"), "line 1\n").unwrap();
    run_git(repo, &["add", "docs/old name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add history file",
        ],
    );

    run_git(repo, &["mv", "docs/old name.txt", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "rename history file",
        ],
    );

    std::fs::write(repo.join("docs/new name.txt"), "line 1\nline 2\n").unwrap();
    run_git(repo, &["add", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update history file once",
        ],
    );

    std::fs::write(repo.join("docs/new name.txt"), "line 1\nline 2\nline 3\n").unwrap();
    run_git(repo, &["add", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update history file twice",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened
        .log_file_page(Path::new("docs/new name.txt"), 2, None)
        .unwrap();
    let first_summaries: Vec<&str> = first.commits.iter().map(|c| &*c.summary).collect();
    assert_eq!(
        first_summaries,
        vec!["update history file twice", "update history file once"]
    );

    let cursor = first.next_cursor.as_ref().expect("next cursor");
    let second = opened
        .log_file_page(Path::new("docs/new name.txt"), 2, Some(cursor))
        .unwrap();
    let second_summaries: Vec<&str> = second.commits.iter().map(|c| &*c.summary).collect();
    assert_eq!(
        second_summaries,
        vec!["rename history file", "add history file"]
    );
    assert!(second.next_cursor.is_none());
}

#[test]
fn log_file_page_exact_limit_has_no_next_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let page = opened.log_file_page(Path::new("a.txt"), 2, None).unwrap();
    assert_eq!(page.commits.len(), 2);
    assert!(page.next_cursor.is_none());
}

#[test]
fn commit_details_reports_merge_parents_and_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("base.txt"), "base\n").unwrap();
    run_git(repo, &["add", "base.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("feature.txt"), "feature\n").unwrap();
    run_git(repo, &["add", "feature.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "main"]);
    std::fs::write(repo.join("main.txt"), "main\n").unwrap();
    run_git(repo, &["add", "main.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );
    run_git(
        repo,
        &["merge", "--no-ff", "feature", "-m", "merge feature branch"],
    );

    let merge_id = git_stdout(repo, &["rev-parse", "HEAD"]);
    let feature_id = git_stdout(repo, &["rev-parse", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let merge_details = opened
        .commit_details(&CommitId(merge_id.clone().into()))
        .expect("commit details");
    let feature_details = opened
        .commit_details(&CommitId(feature_id.into()))
        .expect("feature commit details");

    assert_eq!(merge_details.id, CommitId(merge_id.into()));
    assert_eq!(merge_details.message, "merge feature branch");
    assert!(
        !merge_details.committed_at.is_empty(),
        "expected committed_at to be set"
    );
    assert_eq!(merge_details.parent_ids.len(), 2);
    assert!(
        merge_details.files.is_empty(),
        "merge commit details should match `git show` and omit file rows without `-m`"
    );
    assert!(
        feature_details.files.iter().any(|f| {
            f.path.as_path() == Path::new("feature.txt") && f.kind == FileStatusKind::Added
        }),
        "expected feature commit details to include feature file"
    );
}

#[test]
fn commit_details_reports_root_and_rename_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "diff.renames", "true"]);

    std::fs::write(repo.join("old name.txt"), "hello\n").unwrap();
    run_git(repo, &["add", "old name.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "root commit"],
    );
    let root_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["mv", "old name.txt", "new name.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "rename file"],
    );
    let rename_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let root_details = opened
        .commit_details(&CommitId(root_id.clone().into()))
        .expect("root commit details");
    let rename_details = opened
        .commit_details(&CommitId(rename_id.clone().into()))
        .expect("rename commit details");

    assert_eq!(root_details.id, CommitId(root_id.into()));
    assert_eq!(root_details.message, "root commit");
    assert_eq!(root_details.parent_ids, Vec::<CommitId>::new());
    assert_eq!(
        root_details.files,
        vec![gitcomet_core::domain::CommitFileChange {
            path: Path::new("old name.txt").to_path_buf(),
            kind: FileStatusKind::Added,
            is_submodule: false,
            additions: Some(1),
            deletions: Some(0),
        }]
    );

    assert_eq!(rename_details.id, CommitId(rename_id.into()));
    assert_eq!(rename_details.message, "rename file");
    assert_eq!(rename_details.parent_ids.len(), 1);
    assert_eq!(
        rename_details.files,
        vec![gitcomet_core::domain::CommitFileChange {
            path: Path::new("new name.txt").to_path_buf(),
            kind: FileStatusKind::Renamed,
            is_submodule: false,
            additions: Some(0),
            deletions: Some(0),
        }]
    );
}

#[test]
fn reflog_head_returns_recent_entries_with_indices() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let reflog = opened.reflog_head(2).unwrap();

    assert_eq!(reflog.len(), 2);
    assert_eq!(&*reflog[0].selector, "HEAD@{0}");
    assert_eq!(&*reflog[1].selector, "HEAD@{1}");
    assert_eq!(reflog[0].index, 0);
    assert_eq!(reflog[1].index, 1);
    assert!(reflog.iter().all(|entry| !entry.new_id.0.is_empty()));
    assert!(reflog.iter().all(|entry| entry.time.is_some()));
}

#[test]
fn reflog_head_returns_error_for_unborn_head() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .reflog_head(5)
        .expect_err("unborn HEAD should not have a reflog");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git reflog");
            assert_eq!(failure.id(), GitFailureId::CommandFailed);
            assert_eq!(failure.exit_code(), Some(128));
            assert_eq!(
                failure.detail(),
                Some("fatal: your current branch 'main' does not have any commits yet")
            );
            assert_eq!(failure.stdout(), b"");
            assert_eq!(
                failure.stderr(),
                b"fatal: your current branch 'main' does not have any commits yet\n"
            );
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn log_all_branches_includes_older_stash_reflog_entries() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    std::fs::write(repo.join("stash.txt"), "first\n").unwrap();
    run_git(repo, &["add", "stash.txt"]);
    run_git(repo, &["stash", "push", "-m", "stash-one"]);

    std::fs::write(repo.join("stash.txt"), "second\n").unwrap();
    run_git(repo, &["add", "stash.txt"]);
    run_git(repo, &["stash", "push", "-m", "stash-two"]);

    let stash_ids = git_stdout(
        repo,
        &["reflog", "show", "-n2", "--format=%H", "refs/stash"],
    );
    let stash_ids: Vec<&str> = stash_ids.lines().collect();
    assert_eq!(stash_ids.len(), 2, "expected two stash reflog entries");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();

    assert!(
        all.commits.iter().any(|c| c.id.as_ref() == stash_ids[0]),
        "expected all-branches log to include stash tip"
    );
    assert!(
        all.commits.iter().any(|c| c.id.as_ref() == stash_ids[1]),
        "expected all-branches log to include older stash reflog commit"
    );
}

// --- Author filtering ---

/// Commits alternating between two authors, newest first: `even` owns the
/// even-numbered ones, `rare` owns commit 3 alone.
fn author_filter_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "--initial-branch=main"]);
    run_git(&repo, &["config", "user.email", "dev@example.com"]);
    run_git(&repo, &["config", "user.name", "Common Dev"]);

    for index in 0..8 {
        let author = if index == 3 {
            "Rare Person <rare@example.com>"
        } else {
            "Common Dev <dev@example.com>"
        };
        std::fs::write(repo.join("file.txt"), format!("v{index}")).unwrap();
        run_git(&repo, &["add", "file.txt"]);
        run_git_at(
            &repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--author",
                author,
                "-m",
                &format!("c{index}"),
            ],
            1_000 + index,
        );
    }
    (dir, repo)
}

#[test]
fn author_filter_keeps_only_matching_commits() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();

    let page = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("rare"), 10, None)
        .unwrap();

    assert_eq!(
        page.commits
            .iter()
            .map(|c| c.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["c3"],
        "only the rare author's commit matches"
    );
    assert!(
        page.commits
            .iter()
            .all(|c| c.author.as_ref() == "Rare Person"),
        "the filter must match on the author, not the committer"
    );
    // Every match is on the page, so there is nothing to page to. Advertising
    // more here would send the next page walking the rest of history for
    // nothing.
    assert!(page.next_cursor.is_none());
}

#[test]
fn author_filter_matches_case_insensitively_on_a_substring() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();

    for query in ["RARE", "rare per", "Person"] {
        let page = opened
            .log_history_mode_page_filtered(HistoryMode::FullReachable, Some(query), 10, None)
            .unwrap();
        assert_eq!(
            page.commits.len(),
            1,
            "`{query}` should match the rare author"
        );
    }
}

/// The needle has to be folded the same way it is compared. Folding it with
/// `str::to_lowercase` while comparing with `eq_ignore_ascii_case` left any name
/// holding an uppercase non-ASCII letter unmatchable — including by picking that
/// exact name out of the author dropdown, which then walked the whole history to
/// return nothing.
#[test]
fn author_filter_matches_a_name_with_non_ascii_uppercase() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "--initial-branch=main"]);
    run_git(&repo, &["config", "user.email", "dev@example.com"]);
    run_git(&repo, &["config", "user.name", "Common Dev"]);
    std::fs::write(repo.join("file.txt"), "v0").unwrap();
    run_git(&repo, &["add", "file.txt"]);
    run_git_at(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--author",
            "José Ángel <ja@example.com>",
            "-m",
            "c0",
        ],
        1_000,
    );

    let opened = GixBackend.open(&repo).unwrap();
    // Exact, ASCII-folded, and a substring starting on the non-ASCII letter.
    for query in ["José Ángel", "josé Ángel", "Ángel"] {
        let page = opened
            .log_history_mode_page_filtered(HistoryMode::FullReachable, Some(query), 10, None)
            .unwrap();
        assert_eq!(
            page.commits
                .iter()
                .map(|c| c.author.as_ref())
                .collect::<Vec<_>>(),
            vec!["José Ángel"],
            "`{query}` should match the author it was taken from"
        );
    }
}

#[test]
fn author_filter_pages_without_repeating_or_skipping() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();

    let first = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("common"), 3, None)
        .unwrap();
    assert_eq!(
        first
            .commits
            .iter()
            .map(|c| c.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["c7", "c6", "c5"]
    );
    let cursor = first.next_cursor.as_ref().expect("more matches remain");

    let second = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("common"), 3, Some(cursor))
        .unwrap();
    assert_eq!(
        second
            .commits
            .iter()
            .map(|c| c.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["c4", "c2", "c1"],
        "paging must resume after the last match, skipping the rare author's c3"
    );
    assert!(
        second
            .commits
            .iter()
            .all(|c| first.commits.iter().all(|f| f.id != c.id)),
        "pages must not repeat commits"
    );
}

/// A walk resumed under a different filter would silently skip everything the
/// first pass had already consumed, so the token must not be reusable.
#[test]
fn author_filter_resume_token_is_not_reused_across_filters() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();

    let filtered = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("common"), 3, None)
        .unwrap();
    let cursor = filtered.next_cursor.as_ref().expect("more matches remain");

    // Same cursor, different filter: the walk is rebuilt from the cursor's
    // `last_seen` rather than resumed mid-history.
    let other = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("rare"), 3, Some(cursor))
        .unwrap();
    assert_eq!(
        other
            .commits
            .iter()
            .map(|c| c.summary.as_ref())
            .collect::<Vec<_>>(),
        vec!["c3"],
        "the rare author's commit is still found after the cursor"
    );
}

#[test]
fn streamed_chunks_are_prefixes_of_the_finished_page() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();
    let cancellation = gitcomet_core::services::CancellationToken::new();

    let mut chunks: Vec<Vec<CommitId>> = Vec::new();
    let mut on_chunk = |chunk: gitcomet_core::services::LogChunk| {
        chunks.push(chunk.commits.iter().map(|c| c.id.clone()).collect());
    };
    let page = opened
        .log_history_mode_page_streaming(
            HistoryMode::FullReachable,
            Some("common"),
            10,
            None,
            &cancellation,
            &mut on_chunk,
        )
        .unwrap();

    let final_ids: Vec<CommitId> = page.commits.iter().map(|c| c.id.clone()).collect();
    for chunk in &chunks {
        assert!(
            final_ids.starts_with(chunk),
            "every chunk must be a prefix of the finished page, got {chunk:?} for {final_ids:?}"
        );
    }
}

#[test]
fn cancelled_author_filter_walk_reports_cancellation() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();
    let cancellation = gitcomet_core::services::CancellationToken::new();
    cancellation.cancel();

    let error = opened
        .log_history_mode_page_streaming(
            HistoryMode::FullReachable,
            Some("common"),
            10,
            None,
            &cancellation,
            &mut |_| {},
        )
        .expect_err("a cancelled walk must not return a page");
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
}

/// "There is more" must mean another *matching* commit exists. Deciding it from
/// the next commit of any author hands back a cursor whose page walks the rest
/// of history to return nothing — on a large repository, seconds of work for an
/// empty result.
#[test]
fn author_filter_full_page_does_not_advertise_more_without_another_match() {
    let (_dir, repo) = author_filter_fixture();
    let opened = GixBackend.open(&repo).unwrap();

    // The rare author owns exactly one commit, and a page of one is full.
    let page = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("rare"), 1, None)
        .unwrap();

    assert_eq!(page.commits.len(), 1);
    assert!(
        page.next_cursor.is_none(),
        "a full page with no further match must not offer another page"
    );

    // Whereas a filter that does have more to give still pages.
    let more = opened
        .log_history_mode_page_filtered(HistoryMode::FullReachable, Some("common"), 1, None)
        .unwrap();
    assert_eq!(more.commits.len(), 1);
    assert!(more.next_cursor.is_some());
}
