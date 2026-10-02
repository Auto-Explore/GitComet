mod support;
use super::ignore_rules::*;
use support::*;
mod selective;
use super::*;
use notify::EventKind;
use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind};
use std::fs;
use std::process::Command;
use std::sync::{OnceLock, atomic::AtomicBool, mpsc};

struct IsolatedGitConfigEnv {
    _root: tempfile::TempDir,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    global_config: PathBuf,
    excludes_file: PathBuf,
}

fn isolated_git_config_env() -> &'static IsolatedGitConfigEnv {
    static ENV: OnceLock<IsolatedGitConfigEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("create isolated git config tempdir");
        let home_dir = root.path().join("home");
        let xdg_config_home = root.path().join("xdg");
        let global_config = root.path().join("global.gitconfig");
        let excludes_file = root.path().join("global-excludes");

        fs::create_dir_all(&home_dir).expect("create isolated HOME directory");
        fs::create_dir_all(&xdg_config_home).expect("create isolated XDG_CONFIG_HOME directory");
        fs::write(&global_config, "").expect("create isolated global git config file");
        fs::write(&excludes_file, "").expect("create isolated excludes file");

        IsolatedGitConfigEnv {
            _root: root,
            home_dir,
            xdg_config_home,
            global_config,
            excludes_file,
        }
    })
}

fn run_git(repo: &Path, args: &[&str]) {
    let _timer = gitcomet_core::test_support::git_fixture::FixtureTimer::new(
        "subprocess",
        args.first().copied().unwrap_or("git"),
    );
    let env = isolated_git_config_env();
    let output = Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &env.global_config)
        .env("HOME", &env.home_dir)
        .env("XDG_CONFIG_HOME", &env.xdg_config_home)
        .env_remove("GIT_CONFIG_SYSTEM")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8(output.stdout).unwrap_or_else(|_| "<non-utf8 stdout>".to_string()),
        String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string())
    );
}

fn init_repo_for_ignore_tests(workdir: &Path) {
    let _timer =
        gitcomet_core::test_support::git_fixture::FixtureTimer::new("setup", "watcher-repository");
    let _ = fs::create_dir_all(workdir);
    run_git(workdir, &["init"]);
    // Keep tests deterministic and independent from host global excludes.
    let excludes_file = isolated_git_config_env()
        .excludes_file
        .to_string_lossy()
        .into_owned();
    gitcomet_core::test_support::git_fixture::append_config(
        workdir,
        &[
            ("core.excludesFile", &excludes_file),
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
    // Init already writes core.fileMode. Replace it through Git rather than
    // appending a duplicate that would break later reinitialization tests.
    run_git(workdir, &["config", "core.fileMode", "false"]);
    // Most fixtures need a HEAD and an index. The unborn-repository
    // regression uses plain `git init` to exercise the missing-index case.
    run_git(workdir, &["commit", "--allow-empty", "-m", "init"]);
}

fn load_gitignore_rules(workdir: &Path) -> TestRules {
    TestRules::load(workdir, Arc::new(gitcomet_git_gix::GixBackend))
}

#[test]
fn lfs_reads_cannot_schedule_another_refresh() {
    let dir = unique_temp_dir("gitcomet-lfs-policy");
    // Like the monitor, classify canonical paths: Git directories are reported
    // canonically, while raw macOS temp paths still begin at the /var link.
    let workdir = &normalized(&dir.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(workdir);
    let git_dir = workdir.join(".git");
    let mut rules = load_gitignore_rules(workdir);
    for cache in ["lfs/tmp/clean", "objects/ab/object"] {
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Remove(RemoveKind::File),
        ] {
            let event = notify::Event::new(kind).add_path(git_dir.join(cache));
            assert_eq!(
                classify_change(workdir, Some(&git_dir), &mut rules, &event),
                None,
                "{cache}"
            );
        }
    }
}

#[test]
fn nested_submodule_lfs_cache_is_excluded() {
    let dir = unique_temp_dir("gitcomet-submodule-policy");
    let workdir = &normalized(&dir.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(workdir);
    let git_dir = workdir.join(".git");
    let submodule = git_dir.join("modules/Assets/Standard Assets/CharacterBuilder");
    fs::create_dir_all(&submodule).unwrap();
    run_git(&submodule, &["init", "--bare"]);
    let mut rules = load_gitignore_rules(workdir);
    let event = notify::Event::new(EventKind::Any).add_path(submodule.join("lfs/tmp/clean"));
    assert_eq!(
        classify_change(workdir, Some(&git_dir), &mut rules, &event),
        None
    );
    let event = notify::Event::new(EventKind::Any).add_path(git_dir.join("refs/heads/lfs/topic"));
    assert!(
        classify_change(workdir, Some(&git_dir), &mut rules, &event)
            .unwrap()
            .git_state
    );
}

#[test]
fn ignored_gitignore_does_not_reload_policy() {
    let dir = unique_temp_dir("gitcomet-ignored-config");
    let workdir = dir.path();
    init_repo_for_ignore_tests(workdir);
    fs::write(workdir.join(".gitignore"), "node_modules/\n").unwrap();
    fs::create_dir_all(workdir.join("node_modules/package")).unwrap();
    let mut rules = load_gitignore_rules(workdir);
    let event = notify::Event::new(EventKind::Any)
        .add_path(workdir.join("node_modules/package/.gitignore"));
    let classified = summarize_event(workdir, Some(&workdir.join(".git")), &mut rules, &event);
    assert_eq!(classified, EventEffect::default());
}

#[test]
fn ignore_edit_does_not_hide_index_change_in_same_event() {
    let dir = unique_temp_dir("gitcomet-mixed-config");
    let workdir = &normalized(&dir.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(workdir);
    let mut rules = load_gitignore_rules(workdir);
    let event = notify::Event::new(EventKind::Any)
        .add_path(workdir.join(".gitignore"))
        .add_path(workdir.join(".git/index"));
    let classified = summarize_event(workdir, Some(&workdir.join(".git")), &mut rules, &event);
    let change = classified.change.unwrap();
    assert!(classified.policy_dirty && change.worktree && change.index);
}

fn unique_temp_dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("create unique tempdir")
}

/// Discard everything a freshly established watch still owes us, returning once it has been
/// silent for `quiet` (or `budget` runs out).
///
/// Linux may run a file's final `fput` — which is what emits `IN_CLOSE_WRITE` — after `close()`
/// has already returned to userspace, so a write performed *before* `watch()` can still be
/// delivered a few hundred microseconds *after* the watch is live. Tests that assert on what a
/// watch delivers must drain that setup residue first, or they flake under load.
#[cfg(target_os = "linux")]
fn drain_until_quiet(
    rx: &mpsc::Receiver<notify::Event>,
    quiet: Duration,
    budget: Duration,
) -> Vec<notify::Event> {
    let deadline = Instant::now() + budget;
    let mut drained = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(quiet) {
            Ok(event) => drained.push(event),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }
    drained
}

fn cache_key(rel: impl Into<PathBuf>, is_dir_hint: Option<bool>) -> IgnoreCacheKey {
    IgnoreCacheKey {
        rel: rel.into(),
        is_dir_hint,
    }
}

/// Test helper: classify an event and return just the coalesced change (dropping the
/// `gitignore_changed` signal), matching the pre-`ClassifiedEvent` return shape these tests use.
/// A worktree change the watcher attributes to exactly `paths`.
fn worktree_change(paths: &[&str]) -> RepoExternalChange {
    RepoExternalChange {
        paths: crate::msg::ChangedPaths::known(paths.iter().map(PathBuf::from).collect()),
        ..RepoExternalChange::worktree()
    }
}

fn classify_change(
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut TestRules,
    event: &notify::Event,
) -> Option<RepoExternalChange> {
    summarize_event(workdir, git_dir, gitignore, event).change
}

#[test]
fn resolve_git_dir_handles_dot_git_directory() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let _ = fs::create_dir_all(workdir.join(".git"));

    assert_eq!(resolve_git_dir(&workdir), Some(workdir.join(".git")));
}

#[test]
fn watcher_marks_attribute_inputs_without_marking_ordinary_file_edits() {
    let dir = unique_temp_dir("gitcomet-attribute-watch");
    let workdir = dir.path();
    fs::create_dir_all(workdir.join(".git/info")).unwrap();
    for (path, expected) in [
        ("a.txt", false),
        (".git/index", false),
        (".git/HEAD", false),
        (".gitattributes", true),
        ("nested/.gitattributes", true),
        (".git/info/attributes", true),
    ] {
        let event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(path)],
            attrs: Default::default(),
        };
        let change = classify_change(
            workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event,
        )
        .unwrap();
        assert_eq!(change.text_attributes, expected, "{path}: {change:?}");
        assert_eq!(
            merge_change(change, RepoExternalChange::Worktree).text_attributes,
            expected
        );
    }
}

#[test]
fn resolve_git_dir_parses_dot_git_file() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let gitdir = dir.path().join("actual-git-dir");
    let _ = fs::create_dir_all(&workdir);
    let _ = fs::create_dir_all(&gitdir);

    fs::write(
        workdir.join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )
    .expect("write .git file");

    assert_eq!(resolve_git_dir(&workdir), Some(gitdir));
}

#[test]
fn merge_change_coalesces_to_both() {
    assert_eq!(
        merge_change(RepoExternalChange::Worktree, RepoExternalChange::GitState),
        RepoExternalChange {
            worktree: true,
            index: false,
            git_state: true,
            tags: false,
            verification_context: false,
            large_file_support: false,
            text_attributes: false,
            paths: crate::msg::ChangedPaths::Unknown,
        }
    );
    assert_eq!(
        merge_change(RepoExternalChange::GitState, RepoExternalChange::Worktree),
        RepoExternalChange {
            worktree: true,
            index: false,
            git_state: true,
            tags: false,
            verification_context: false,
            large_file_support: false,
            text_attributes: false,
            paths: crate::msg::ChangedPaths::Unknown,
        }
    );
    assert_eq!(
        merge_change(RepoExternalChange::Both, RepoExternalChange::Worktree),
        RepoExternalChange::Both
    );
    assert_eq!(
        merge_change(RepoExternalChange::GitState, RepoExternalChange::GitState),
        RepoExternalChange::GitState
    );
}

#[test]
fn classify_repo_change_distinguishes_gitdir_from_worktree() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let _ = fs::create_dir_all(workdir.join(".git"));

    let event = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join(".git").join("index")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event
        ),
        Some(RepoExternalChange::Index)
    );

    let event = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join("file.txt")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event
        ),
        Some(worktree_change(&["file.txt"]))
    );

    let event = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join(".git").join("HEAD"), workdir.join("file.txt")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event
        ),
        Some(RepoExternalChange {
            worktree: true,
            index: false,
            git_state: true,
            tags: false,
            verification_context: false,
            large_file_support: false,
            text_attributes: false,
            paths: crate::msg::ChangedPaths::known(vec!["file.txt".into()]),
        })
    );
}

#[test]
fn classify_repo_change_ignores_git_index_lock_churn() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let _ = fs::create_dir_all(workdir.join(".git"));

    let mut rules = TestRules::default();
    let create_lock = notify::Event {
        kind: EventKind::Create(CreateKind::File),
        paths: vec![workdir.join(".git").join("index.lock")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut rules,
            &create_lock
        ),
        None,
        "index.lock creation should not trigger external refresh"
    );

    let mut rules = TestRules::default();
    let remove_lock = notify::Event {
        kind: EventKind::Remove(RemoveKind::File),
        paths: vec![workdir.join(".git").join("index.lock")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut rules,
            &remove_lock
        ),
        None,
        "index.lock deletion should not trigger external refresh"
    );
}

#[test]
fn classify_repo_change_ignoring_index_lock_does_not_drop_real_worktree_events() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let _ = fs::create_dir_all(workdir.join(".git"));

    let mut rules = TestRules::default();
    let event = notify::Event {
        kind: EventKind::Create(CreateKind::Any),
        paths: vec![
            workdir.join(".git").join("index.lock"),
            workdir.join("file.txt"),
        ],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, Some(&workdir.join(".git")), &mut rules, &event),
        Some(worktree_change(&["file.txt"])),
        "ignoring index.lock should still classify real worktree changes"
    );
}

#[test]
fn debouncer_flushes_on_debounce_or_max_delay() {
    let base = Instant::now();
    let mut d = DebouncedChange::new(Duration::from_millis(100), Duration::from_millis(250));

    assert_eq!(d.push(RepoExternalChange::Worktree, base), None);
    assert!(d.is_pending());

    // Another event resets debounce window.
    assert_eq!(
        d.push(
            RepoExternalChange::Worktree,
            base + Duration::from_millis(50)
        ),
        None
    );
    assert!(d.next_timeout(base + Duration::from_millis(50)).is_some());

    // Not yet due at 149ms from base.
    assert_eq!(d.take_if_due(base + Duration::from_millis(149)), None);

    // Due by debounce at 150ms from base (last at 50ms + 100ms).
    assert_eq!(
        d.take_if_due(base + Duration::from_millis(150)),
        Some(RepoExternalChange::Worktree)
    );
    assert!(!d.is_pending());

    // Continuous events should flush by max_delay.
    assert_eq!(d.push(RepoExternalChange::GitState, base), None);
    assert_eq!(
        d.push(
            RepoExternalChange::GitState,
            base + Duration::from_millis(300)
        ),
        Some(RepoExternalChange::GitState)
    );
    assert!(!d.is_pending());
}

#[test]
fn access_events_do_not_trigger_refresh_loops() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let _ = fs::create_dir_all(workdir.join(".git"));

    let event = notify::Event {
        kind: EventKind::Access(AccessKind::Open(AccessMode::Read)),
        paths: vec![workdir.join(".git").join("index")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event
        ),
        None
    );

    let event = notify::Event {
        kind: EventKind::Access(AccessKind::Close(AccessMode::Read)),
        paths: vec![workdir.join("file.txt")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event
        ),
        None
    );

    let event = notify::Event {
        kind: EventKind::Access(AccessKind::Close(AccessMode::Write)),
        paths: vec![workdir.join("file.txt")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            Some(&workdir.join(".git")),
            &mut TestRules::default(),
            &event
        ),
        Some(worktree_change(&["file.txt"]))
    );
}

#[test]
fn gitignore_rules_match_git_semantics_for_nested_negation_and_anchoring() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    init_repo_for_ignore_tests(&workdir);
    let git_dir = resolve_git_dir(&workdir);

    fs::write(
        workdir.join(".gitignore"),
        "target/\n*.gitcomet-log\n!keep.gitcomet-log\n/build/output\nlogs/*.tmp\n",
    )
    .expect("write .gitignore");
    fs::create_dir_all(workdir.join("logs")).expect("create logs directory");
    fs::write(workdir.join("logs/.gitignore"), "!keep.tmp\n").expect("write nested .gitignore");
    fs::write(
        git_dir
            .as_ref()
            .expect("git dir")
            .join("info")
            .join("exclude"),
        "info-excluded.gitcomet\n",
    )
    .expect("write .git/info/exclude");
    fs::create_dir_all(workdir.join("target/debug")).expect("create target/debug directory");
    // The gix excludes stack traverses directories on disk when processing
    // path components; intermediate dirs must exist (in production, filesystem
    // events always reference existing paths).
    fs::create_dir_all(workdir.join("build")).expect("create build directory");

    let mut rules = load_gitignore_rules(&workdir);
    assert!(rules.is_ignored_rel(Path::new("target/debug/app"), Some(false)));
    assert!(rules.is_ignored_rel(Path::new("foo.gitcomet-log"), Some(false)));
    assert!(!rules.is_ignored_rel(Path::new("keep.gitcomet-log"), Some(false)));
    assert!(rules.is_ignored_rel(Path::new("build/output"), Some(false)));
    assert!(!rules.is_ignored_rel(Path::new("nested/build/output"), Some(false)));
    assert!(rules.is_ignored_rel(Path::new("logs/drop.tmp"), Some(false)));
    assert!(!rules.is_ignored_rel(Path::new("logs/keep.tmp"), Some(false)));
    assert!(rules.is_ignored_rel(Path::new("info-excluded.gitcomet"), Some(false)));
    assert!(rules.is_ignored_rel(Path::new("target"), Some(true)));

    // Ensure folder create events for ignored directories are treated as ignorable worktree
    // changes.
    let event = notify::Event {
        kind: EventKind::Create(CreateKind::Folder),
        paths: vec![workdir.join("target")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, git_dir.as_deref(), &mut rules, &event),
        None
    );
}

#[test]
fn tracked_paths_are_not_treated_as_ignored() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    init_repo_for_ignore_tests(&workdir);
    let git_dir = resolve_git_dir(&workdir);

    fs::write(
        workdir.join(".gitignore"),
        "*.tracked-ignore\n*.untracked-ignore\n",
    )
    .expect("write .gitignore");
    fs::write(workdir.join("tracked.tracked-ignore"), "tracked\n").expect("write tracked file");
    fs::write(workdir.join("new.untracked-ignore"), "untracked\n").expect("write ignored file");

    run_git(&workdir, &["add", "-f", "tracked.tracked-ignore"]);

    let mut rules = load_gitignore_rules(&workdir);
    assert!(
        !rules.is_ignored_rel(Path::new("tracked.tracked-ignore"), Some(false)),
        "tracked paths must not be treated as ignored"
    );
    assert!(rules.is_ignored_rel(Path::new("new.untracked-ignore"), Some(false)));

    let tracked_event = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join("tracked.tracked-ignore")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, git_dir.as_deref(), &mut rules, &tracked_event),
        Some(worktree_change(&["tracked.tracked-ignore"]))
    );

    let ignored_event = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join("new.untracked-ignore")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, git_dir.as_deref(), &mut rules, &ignored_event),
        None
    );
}

#[test]
fn watched_event_kinds_exclude_read_access_but_keep_close_write() {
    assert!(!WATCHED_EVENT_KINDS.intersects(EventKindMask::ACCESS_OPEN));
    assert!(!WATCHED_EVENT_KINDS.intersects(EventKindMask::ACCESS_CLOSE_NOWRITE));
    // `should_ignore_event_kind` keeps close-after-write, so it must still be requested.
    assert!(WATCHED_EVENT_KINDS.contains(EventKindMask::ACCESS_CLOSE));
    // Every event kind that repository summarization acts on.
    assert!(WATCHED_EVENT_KINDS.contains(EventKindMask::CORE));
}

#[cfg(target_os = "linux")]
#[test]
fn watcher_does_not_deliver_events_for_reads_of_watched_files() {
    // Regression: the default notify config asks inotify for IN_OPEN and
    // IN_CLOSE_NOWRITE, so every file this app reads while producing a status,
    // diff or blame bounced straight back as an event the monitor then threw
    // away. On an active repo that was >99% of all delivered events — tens of
    // thousands per minute of pure thread-wakeup and allocation churn.
    let dir = unique_temp_dir("gitcomet-monitor-read-noise");
    let workdir = dir.path().join("repo");
    fs::create_dir_all(&workdir).expect("create workdir");
    let file = workdir.join("tracked.txt");
    fs::write(&file, b"before").expect("seed file");

    let (tx, rx) = mpsc::channel::<notify::Event>();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        NotifyConfig::default().with_event_kinds(WATCHED_EVENT_KINDS),
    )
    .expect("create watcher");
    watcher
        .watch(&workdir, RecursiveMode::NonRecursive)
        .expect("watch workdir");
    // The seed write closed its file before this watch existed, but its `IN_CLOSE_WRITE` can
    // still land here (see `drain_until_quiet`). Start the read phase from a quiet watch so the
    // assertion below only ever sees events the reads themselves caused.
    let setup_residue = drain_until_quiet(&rx, Duration::from_millis(300), Duration::from_secs(5));
    assert!(
        setup_residue
            .iter()
            .all(|event| event.paths == vec![file.clone()]
                && event.kind == notify::EventKind::Access(AccessKind::Close(AccessMode::Write))),
        "only the seed write may show up before the read phase, got {setup_residue:?}"
    );

    for _ in 0..50 {
        assert_eq!(fs::read(&file).expect("read file"), b"before");
    }
    std::thread::sleep(Duration::from_secs(1));
    let read_events: Vec<_> = rx.try_iter().collect();
    assert!(
        read_events.is_empty(),
        "reading watched files must not deliver any event, got {read_events:?}"
    );

    // The watch is still live: a real write must still arrive.
    fs::write(&file, b"after").expect("modify file");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_write = false;
    while !saw_write && Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => saw_write = event.paths.iter().any(|p| p == &file),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(
        saw_write,
        "a write to a watched file must still be delivered"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn gitignore_change_reinit_unwatches_newly_ignored_dir() {
    // Regression test: when a directory becomes gitignored at runtime, the monitor must
    // re-initiate the worktree watches so the now-ignored tree is no longer watched. Before the
    // fix, the re-watch was add-only and left the stale watches in place, so churn under the
    // freshly-ignored dir kept flooding the event queue (the failure mode behind large worktrees
    // dropping real edits). `start_watcher` rebuilds the minimal watch set from the
    // current rules, and dropping the previous watcher releases its inotify watches.
    let dir = unique_temp_dir("gitcomet-monitor-reinit");
    let workdir = dir.path().join("repo");
    init_repo_for_ignore_tests(&workdir);
    fs::create_dir_all(workdir.join("vendor").join("pkg")).expect("create vendor/pkg");
    fs::create_dir_all(workdir.join("src")).expect("create src");
    let mut gitignore = load_gitignore_rules(&workdir);

    // vendor/ is not yet ignored, so the initial setup watches it.
    let (initial, _, _rx) = gitignore.start_watcher(&workdir);
    assert!(
        gitignore
            .state
            .plan
            .worktree_dirs
            .contains(&workdir.join("vendor")),
        "vendor/ must be watched before it is ignored"
    );

    // vendor/ becomes gitignored; re-initiate the worktree watches exactly like the monitor
    // loop does: drop the old watches, reload the rules and rebuild coverage.
    fs::write(workdir.join(".gitignore"), "vendor/\n").expect("write .gitignore");
    drop(initial);
    gitignore = load_gitignore_rules(&workdir);
    let (_watcher, _, monitor_rx) = gitignore.start_watcher(&workdir);
    assert!(
        !gitignore
            .state
            .plan
            .worktree_dirs
            .contains(&workdir.join("vendor")),
        "rebuilding after the ignore edit must drop vendor/ from the watched set"
    );

    std::thread::sleep(Duration::from_millis(300));
    while monitor_rx.try_recv().is_ok() {}

    // Churn under the now-ignored vendor/ and make one real edit under the tracked src/.
    for i in 0..20 {
        fs::write(
            workdir.join("vendor").join("pkg").join(format!("f{i}.bin")),
            b"x",
        )
        .expect("write vendor churn");
    }
    fs::write(workdir.join("src").join("main.rs"), b"fn main() {}").expect("write src file");

    std::thread::sleep(Duration::from_secs(1));
    let mut vendor_events = 0usize;
    let mut saw_src = false;
    while let Ok(msg) = monitor_rx.try_recv() {
        if let MonitorMsg::Event(Ok(event)) = msg {
            for path in &event.paths {
                if path.starts_with(workdir.join("vendor")) {
                    vendor_events += 1;
                }
                saw_src |= path.starts_with(workdir.join("src"));
            }
        }
    }

    assert!(
        saw_src,
        "a real edit under the tracked src/ must still be delivered"
    );
    assert_eq!(
        vendor_events, 0,
        "vendor/ is now gitignored; re-initiating the watches must unwatch it so its churn \
         produces no events (got {vendor_events})"
    );
}

#[test]
fn watch_degraded_transition_fires_once_per_degraded_episode() {
    let mut degraded = false;
    // Entering the skipped state warns, carrying the folder count.
    assert_eq!(
        watch_degraded_transition(
            &mut degraded,
            WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count: 9000 }
        ),
        Some(RepoWatchDegradedReason::TooManyFolders { dir_count: 9000 })
    );
    // Staying degraded (e.g. a .gitignore rebuild that is still over budget) does not re-warn.
    assert_eq!(
        watch_degraded_transition(
            &mut degraded,
            WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count: 9001 }
        ),
        None
    );
    // Recovering to full watching clears the flag without warning.
    assert_eq!(
        watch_degraded_transition(
            &mut degraded,
            WatchSetupOutcome::Watching { failed_dirs: 0 }
        ),
        None
    );
    assert!(!degraded);
    // A partial watch failure is also a degraded transition and warns with the unwatched count.
    assert_eq!(
        watch_degraded_transition(
            &mut degraded,
            WatchSetupOutcome::Watching { failed_dirs: 7 }
        ),
        Some(RepoWatchDegradedReason::WatchLimitReached { unwatched_dirs: 7 })
    );
    // Still partially failing on a rebuild does not re-warn.
    assert_eq!(
        watch_degraded_transition(
            &mut degraded,
            WatchSetupOutcome::Watching { failed_dirs: 3 }
        ),
        None
    );
}

#[test]
fn degraded_watch_recheck_is_throttled() {
    // Reproduces the "reload ignore rules every idle tick (30s) while degraded" cost: the
    // recovery re-check must be rate-limited, not run on every tick. With no prior attempt it is
    // due; immediately after an attempt it is not; only after the interval elapses is it due again.
    let interval = Duration::from_secs(120);
    let base = Instant::now();
    assert!(
        recovery_recheck_due(None, base, interval),
        "first re-check (no prior attempt) must be due"
    );
    assert!(
        !recovery_recheck_due(Some(base), base + Duration::from_secs(30), interval),
        "a re-check 30s after the last attempt must be throttled (not due)"
    );
    assert!(
        recovery_recheck_due(Some(base), base + interval, interval),
        "a re-check after the full interval must be due again"
    );
}

#[test]
fn gitignore_lookup_stats_track_cache_hits_misses_and_matcher_failures() {
    let before = repo_monitor_ignore_lookup_stats();

    let mut rules = TestRules::default();
    rules.workdir = Some(PathBuf::from("/tmp/nonexistent"));
    // No matcher — lookups default to not-ignored and count as matcher failures.

    assert!(!rules.is_ignored_rel(Path::new("sample.ignored"), Some(false)));
    assert!(!rules.is_ignored_rel(Path::new("sample.ignored"), Some(false)));

    let after = repo_monitor_ignore_lookup_stats();
    assert!(
        after.request_count >= before.request_count.saturating_add(2),
        "one miss and one hit should each count as ignore lookup requests"
    );
    assert!(
        after.cache_misses >= before.cache_misses.saturating_add(1),
        "the first lookup should miss the cache"
    );
    assert!(
        after.cache_hits >= before.cache_hits.saturating_add(1),
        "the second lookup should hit the cache"
    );
    assert!(
        after.fallback_count >= before.fallback_count.saturating_add(1),
        "disabling the matcher should count as matcher failure"
    );
}

#[test]
fn panic_payload_to_string_handles_string_and_unknown_payloads() {
    assert_eq!(
        panic_payload_to_string(&"panic message".to_string()),
        "panic message"
    );
    assert_eq!(panic_payload_to_string(&123usize), "unknown panic payload");
}

#[test]
fn debouncer_covers_no_pending_due_check_and_max_delay_selection() {
    let base = Instant::now();
    let mut d = DebouncedChange::new(Duration::from_millis(500), Duration::from_millis(100));

    assert_eq!(d.take_if_due(base), None);
    assert_eq!(d.push(RepoExternalChange::Worktree, base), None);

    let timeout = d
        .next_timeout(base + Duration::from_millis(90))
        .expect("pending timeout");
    assert!(
        timeout <= Duration::from_millis(10),
        "max-delay path should schedule the earliest timeout; got {timeout:?}"
    );
}

#[test]
fn resolve_git_dir_parses_relative_dot_git_file() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    fs::create_dir_all(&workdir).expect("create workdir");
    fs::write(workdir.join(".git"), "gitdir: .actual-git\n").expect("write .git file");

    assert_eq!(resolve_git_dir(&workdir), Some(workdir.join(".actual-git")));
}

#[test]
fn empty_paths_git_state_and_policy_edits_have_distinct_effects() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    fs::create_dir_all(workdir.join(".git")).expect("create .git dir");
    let git_dir = Some(workdir.join(".git"));

    let mut rules = TestRules::default();
    let empty_paths = notify::Event {
        kind: EventKind::Any,
        paths: vec![],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, git_dir.as_deref(), &mut rules, &empty_paths),
        Some(RepoExternalChange::Both)
    );

    let git_head = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join(".git").join("HEAD")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, git_dir.as_deref(), &mut rules, &git_head),
        Some(RepoExternalChange::GitState)
    );

    let gitignore_changed = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join(".gitignore")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(&workdir, git_dir.as_deref(), &mut rules, &gitignore_changed),
        Some(RepoExternalChange::Worktree)
    );

    let nested_gitignore_changed = notify::Event {
        kind: EventKind::Any,
        paths: vec![workdir.join("nested").join(".gitignore")],
        attrs: Default::default(),
    };
    assert_eq!(
        classify_change(
            &workdir,
            git_dir.as_deref(),
            &mut rules,
            &nested_gitignore_changed
        ),
        Some(RepoExternalChange::Worktree)
    );
}

#[test]
fn gitignore_cache_enforces_max_size() {
    let mut rules = TestRules::default();
    let now = Instant::now();
    let total = GITIGNORE_CACHE_MAX_ENTRIES + 8;
    for idx in 0..total {
        rules.cache_insert(
            cache_key(format!("path-{idx}.tmp"), Some(false)),
            idx % 2 == 0,
            now + Duration::from_millis(idx as u64),
        );
    }

    assert_eq!(rules.cache.len(), GITIGNORE_CACHE_MAX_ENTRIES);
    assert!(
        !rules
            .cache
            .contains_key(&cache_key("path-0.tmp", Some(false)).rel),
        "oldest entries should be evicted first"
    );
    assert!(
        rules
            .cache
            .contains_key(&cache_key(format!("path-{}.tmp", total - 1), Some(false)).rel),
        "newest entry should remain in cache"
    );
}

#[test]
fn gitignore_cache_expires_entries_by_ttl() {
    let mut rules = TestRules::default();
    let now = Instant::now();
    let key = cache_key("stale.txt", Some(false));
    rules.cache_insert(key.clone(), true, now);

    assert_eq!(
        rules.cache_get(&key, now + Duration::from_secs(1)),
        Some(true),
        "fresh cache entry should be returned"
    );
    assert_eq!(
        rules.cache_get(&key, now + GITIGNORE_CACHE_TTL + Duration::from_secs(1)),
        None,
        "expired cache entry should miss"
    );
    assert!(
        !rules.cache.contains_key(&key.rel),
        "expired cache entry should be removed"
    );
}

#[test]
fn watcher_callback_send_is_skipped_when_shutdown_gate_is_closed() {
    let (tx, rx) = mpsc::channel::<MonitorMsg>();
    drop(rx);
    let monitor_enabled = AtomicBool::new(false);
    let did_send = send_watcher_event_or_log(
        RepoId(1),
        &tx,
        Ok(notify::Event {
            kind: EventKind::Any,
            paths: vec![],
            attrs: Default::default(),
        }),
        &monitor_enabled,
    );
    assert!(!did_send, "callback gate should suppress watcher sends");
}

#[test]
fn watcher_callback_send_records_failure_when_gate_is_open() {
    let before = super::super::send_diagnostics::send_failure_count(
        super::super::send_diagnostics::SendFailureKind::RepoMonitorMessage,
    );

    let (tx, rx) = mpsc::channel::<MonitorMsg>();
    drop(rx);
    let monitor_enabled = AtomicBool::new(true);

    let did_send = send_watcher_event_or_log(
        RepoId(1),
        &tx,
        Ok(notify::Event {
            kind: EventKind::Any,
            paths: vec![],
            attrs: Default::default(),
        }),
        &monitor_enabled,
    );
    assert!(
        did_send,
        "callback should attempt sends while monitor is active"
    );

    let after = super::super::send_diagnostics::send_failure_count(
        super::super::send_diagnostics::SendFailureKind::RepoMonitorMessage,
    );
    assert!(after > before);
}

#[test]
fn tag_file_changes_refresh_tags() {
    let dir = unique_temp_dir("gitcomet-monitor-test");
    let workdir = dir.path().join("repo");
    let git_dir = workdir.join(".git");
    let _ = fs::create_dir_all(git_dir.join("refs").join("tags"));

    let mut rules = TestRules::default();

    // Loose tag file → tags: true
    let tag_event = notify::Event {
        kind: EventKind::Create(CreateKind::File),
        paths: vec![git_dir.join("refs").join("tags").join("v1.0.0")],
        attrs: Default::default(),
    };
    let change = classify_change(&workdir, Some(&git_dir), &mut rules, &tag_event);
    assert_eq!(
        change,
        Some(RepoExternalChange {
            git_state: true,
            tags: true,
            ..Default::default()
        }),
        "tag ref file should produce tags: true"
    );

    // packed-refs may also change whether a git-annex bookkeeping ref exists.
    let packed_event = notify::Event {
        kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
        paths: vec![git_dir.join("packed-refs")],
        attrs: Default::default(),
    };
    let change = classify_change(&workdir, Some(&git_dir), &mut rules, &packed_event);
    assert_eq!(
        change,
        Some(RepoExternalChange {
            git_state: true,
            tags: true,
            large_file_support: true,
            ..Default::default()
        }),
        "packed-refs should produce tags: true"
    );

    for file in ["reftable/tables.list", "reftable/0x0001-0x0002-example.ref"] {
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![git_dir.join(file)],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, Some(&git_dir), &mut rules, &event),
            Some(RepoExternalChange {
                git_state: true,
                tags: true,
                ..Default::default()
            })
        );
    }

    // Branch ref file → tags: false
    let branch_event = notify::Event {
        kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
        paths: vec![git_dir.join("refs").join("heads").join("main")],
        attrs: Default::default(),
    };
    let change = classify_change(&workdir, Some(&git_dir), &mut rules, &branch_event);
    assert_eq!(
        change,
        Some(RepoExternalChange {
            git_state: true,
            tags: false,
            ..Default::default()
        }),
        "branch ref file should produce tags: false"
    );
}

/// git-annex writes its keys database while GitComet reads status-related
/// data, and every command touches locks, temp files and per-key location
/// logs (paths seen under strace). Treating those writes as Git state changes
/// re-ran the same reads forever.
#[test]
fn annex_bookkeeping_writes_cannot_schedule_another_refresh() {
    let dir = unique_temp_dir("gitcomet-annex-policy");
    let workdir = &normalized(&dir.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(workdir);
    let git_dir = workdir.join(".git");
    let mut rules = load_gitignore_rules(workdir);
    for path in [
        "annex/keysdb/db-wal",
        "annex/keysdb.lck",
        "annex/keysdb.tmp/db",
        "annex/index",
        "annex/index.lck",
        "annex/index.lck2569681-4.tmp",
        "annex/objects/Xk/Wq/KEY/KEY",
        "annex/journal.lck",
        "annex/journal/3cb_894_SHA256E-s1000--db02.bin.log",
        "annex/journal-private/3cb_894_SHA256E-s1000--db02.bin.log",
        "annex/journal-private.lck",
        "annex/mergedrefs",
        "annex/mergedrefs2569681-0.tmp",
        "annex/ignoredrefs",
        "annex/gitqueue.lck",
        "annex/othertmp.lck",
        "annex/misctmp/x",
        "annex/reposize/db/db-wal",
        "annex/unused",
        "annex/badunused",
        "annex/tmpunused",
        "annex/daemon.log",
        "annex/daemon.status",
        "annex/smudge.log",
        "annex/ssh/socket",
    ] {
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Remove(RemoveKind::File),
        ] {
            let event = notify::Event::new(kind).add_path(git_dir.join(path));
            assert_eq!(
                classify_change(workdir, Some(&git_dir), &mut rules, &event),
                None,
                "{path}"
            );
        }
    }
}

#[test]
fn annex_support_metadata_changes_schedule_a_refresh() {
    let dir = unique_temp_dir("gitcomet-annex-metadata-policy");
    let workdir = &normalized(&dir.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(workdir);
    let git_dir = workdir.join(".git");
    let mut rules = load_gitignore_rules(workdir);
    for path in [
        "annex/restage.log",
        "annex/journal/uuid.log",
        "annex/journal/numcopies.log",
        "annex/journal/trust.log",
        "annex/journal/remote.log",
        "annex/journal-private/uuid.log",
        "annex/daemon.pid",
    ] {
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Remove(RemoveKind::File),
        ] {
            let event = notify::Event::new(kind).add_path(git_dir.join(path));
            let change = classify_change(workdir, Some(&git_dir), &mut rules, &event).expect(path);
            assert!(change.large_file_support, "{path}");
            assert!(
                !change.git_state && !change.index && !change.worktree,
                "metadata must not reload file contents: {path}"
            );
        }
    }
}

/// Non-recursive backends see only registered directories: the journals must
/// be walked, git-annex's bulky and churning directories must not.
#[test]
fn watch_plan_walks_annex_journals_but_not_its_bookkeeping() {
    let dir = unique_temp_dir("gitcomet-annex-plan");
    let root = normalized(&dir.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(&root);
    let private = [
        "objects/Xk/Wq",
        "keysdb",
        "keysdb.tmp",
        "transfer/upload",
        "tmp",
        "othertmp",
        "misctmp",
        "reposize/db",
        "ssh",
    ];
    for path in private.iter().chain(&["journal", "journal-private"]) {
        fs::create_dir_all(root.join(".git/annex").join(path)).unwrap();
    }
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    for path in ["annex", "annex/journal", "annex/journal-private"] {
        assert!(plan.dirs.contains(&root.join(".git").join(path)), "{path}");
    }
    for path in private {
        let first = path.split('/').next().unwrap();
        assert!(
            plan.dirs
                .iter()
                .all(|dir| !dir.starts_with(root.join(".git/annex").join(first))),
            "{path}"
        );
    }
}

#[test]
fn attribute_events_request_support_without_scanning_on_ordinary_edits() {
    let dir = unique_temp_dir("gitcomet-monitor-attributes");
    let root = dir.path();
    fs::create_dir(root.join(".git")).unwrap();
    for path in [".gitattributes", "sub/.gitattributes", "file.txt"] {
        for kind in [
            EventKind::Any,
            EventKind::Remove(notify::event::RemoveKind::File),
        ] {
            let event = notify::Event {
                kind,
                paths: vec![root.join(path)],
                attrs: Default::default(),
            };
            let change = classify_change(
                root,
                Some(&root.join(".git")),
                &mut TestRules::default(),
                &event,
            )
            .unwrap();
            assert!(change.worktree);
            assert_eq!(change.large_file_support, path.ends_with(".gitattributes"));
            assert_eq!(
                merge_change(change.clone(), RepoExternalChange::Worktree).large_file_support,
                change.large_file_support
            );
        }
    }
    let event = notify::Event {
        kind: EventKind::Any,
        paths: vec![root.join(".git/info/attributes")],
        attrs: Default::default(),
    };
    assert!(
        classify_change(
            root,
            Some(&root.join(".git")),
            &mut TestRules::default(),
            &event
        )
        .unwrap()
        .git_state
    );
}

#[test]
fn ignored_attributes_file_still_refreshes_support() {
    let dir = unique_temp_dir("gitcomet-ignored-attributes");
    let workdir = dir.path();
    init_repo_for_ignore_tests(workdir);
    fs::write(workdir.join(".gitignore"), ".gitattributes\n").unwrap();
    let mut rules = load_gitignore_rules(workdir);
    let event = notify::Event::new(EventKind::Any).add_path(workdir.join(".gitattributes"));
    let change = summarize_event(workdir, Some(&workdir.join(".git")), &mut rules, &event)
        .change
        .unwrap();
    assert!(change.large_file_support);
}

#[test]
fn annex_ref_and_local_attributes_changes_refresh_support_selectively() {
    let dir = unique_temp_dir("gitcomet-annex-support-events");
    let root = dir.path();
    fs::create_dir(root.join(".git")).unwrap();
    for (path, expected) in [
        (".git/index", false),
        (".git/HEAD", false),
        (".git/refs/heads/main", false),
        (".git/refs/heads/git-annex", true),
        (".git/refs/remotes/origin/git-annex", true),
        (".git/info/attributes", true),
    ] {
        let event = notify::Event::new(EventKind::Any).add_path(root.join(path));
        let change = classify_change(
            root,
            Some(&root.join(".git")),
            &mut TestRules::default(),
            &event,
        )
        .unwrap();
        assert_eq!(change.large_file_support, expected, "{path}");
    }
}
/// Cost of the monitor's index-only reload, which every index write (stage,
/// unstage, commit, an external `git add`) triggers, on real repositories:
/// `GITCOMET_PROBE_REPOS=/a:/b`.
#[test]
#[ignore = "timing probe"]
fn timing_monitor_index_reload_real_repos() {
    let Ok(repos) = std::env::var("GITCOMET_PROBE_REPOS") else {
        eprintln!("GITCOMET_PROBE_REPOS not set");
        return;
    };
    let backend = gitcomet_git_gix::GixBackend;
    let best = |mut run: Box<dyn FnMut() + '_>| {
        run();
        (0..5)
            .map(|_| {
                let start = Instant::now();
                run();
                start.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::MAX, f64::min)
    };
    for path in repos.split(':') {
        let workdir = Path::new(path);
        let name = workdir.file_name().unwrap().to_string_lossy();
        let mut state = MonitorState::default();
        assert!(state.reload(workdir, &backend, false));
        let reload = best(Box::new(|| {
            state.reload(workdir, &backend, true);
        }));
        let inputs = best(Box::new(|| {
            WatchInputs::load(workdir, &backend).unwrap();
        }));
        let info = WatchInputs::load(workdir, &backend).unwrap().info;
        let mut rules = IgnoreRules::default();
        let rules_ms = best(Box::new(|| {
            rules.reload(workdir, &backend, &info);
        }));
        println!(
            "timing monitor_index_reload {name} reload={reload:.2}ms watch_inputs={inputs:.2}ms ignore_rules={rules_ms:.2}ms"
        );
    }
}
