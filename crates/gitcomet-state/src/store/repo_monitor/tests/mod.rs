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
    let _ = fs::create_dir_all(workdir);
    run_git(workdir, &["init"]);
    // Keep tests deterministic and independent from host global excludes.
    let excludes_file = isolated_git_config_env()
        .excludes_file
        .to_string_lossy()
        .into_owned();
    run_git(workdir, &["config", "core.excludesFile", &excludes_file]);
    run_git(workdir, &["config", "core.fileMode", "false"]);
    run_git(workdir, &["config", "user.email", "you@example.com"]);
    run_git(workdir, &["config", "user.name", "You"]);
    run_git(workdir, &["config", "commit.gpgsign", "false"]);
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
        }
    );
    assert_eq!(
        merge_change(RepoExternalChange::GitState, RepoExternalChange::Worktree),
        RepoExternalChange {
            worktree: true,
            index: false,
            git_state: true,
            tags: false,
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
        Some(RepoExternalChange::Worktree)
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
        Some(RepoExternalChange::Worktree),
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
        Some(RepoExternalChange::Worktree)
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
        Some(RepoExternalChange::Worktree)
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

    // packed-refs → tags: true
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
            ..Default::default()
        }),
        "packed-refs should produce tags: true"
    );

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
