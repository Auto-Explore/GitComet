//! A history index grown from the one before it must be what a rebuild gives.
use super::*;
use gitcomet_core::history_index::HistoryIndexHandle;
use gitcomet_core::history_perf::{Work, capture, count};
use gitcomet_core::services::{CancellationToken, GitRepository};
use std::io::Write as _;

/// A repository written through `git fast-import`, one stream at a time, with
/// marks kept between streams so later commits can name earlier ones.
struct Fixture {
    dir: tempfile::TempDir,
    next_mark: usize,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q", "-b", "main"]);
        Self { dir, next_mark: 1 }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Commits to `branch` with `parents` (marks) at committer time `time`,
    /// and returns the new commit's mark.
    fn commit(&mut self, branch: &str, parents: &[usize], time: i64) -> usize {
        let mark = self.next_mark;
        self.next_mark += 1;
        let mut stream = format!(
            "commit refs/heads/{branch}\nmark :{mark}\ncommitter You <you@example.com> {time} +0000\ndata 5\nnode\n\n"
        );
        for (ix, parent) in parents.iter().enumerate() {
            stream.push_str(&format!(
                "{} :{parent}\n",
                if ix == 0 { "from" } else { "merge" }
            ));
        }
        stream.push('\n');
        let marks = self.path().join(".git/test-marks");
        let mut child = git_cmd(self.path())
            .args(["fast-import", "--quiet"])
            .arg(format!("--import-marks-if-exists={}", marks.display()))
            .arg(format!("--export-marks={}", marks.display()))
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        mark
    }

    /// `(id, parent ids)` of every row `git log --all --date-order` prints.
    fn git_rows(&self) -> Vec<(String, Vec<String>)> {
        git_stdout(
            self.path(),
            &["log", "--all", "--date-order", "--format=%H %P"],
        )
        .lines()
        .map(|line| {
            let mut ids = line.split(' ').filter(|id| !id.is_empty());
            let id = ids.next().unwrap().to_owned();
            (id, ids.map(str::to_owned).collect())
        })
        .collect()
    }
}

fn index_of(repo: &dyn GitRepository) -> HistoryIndexHandle {
    repo.build_history_index(
        &HistoryMode::AllBranches.into(),
        None,
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap()
    .unwrap()
}

fn index_rows(index: &HistoryIndexHandle) -> Vec<(String, Vec<String>)> {
    (0..index.len())
        .map(|row| {
            (
                index.commit_id(row).unwrap().as_ref().to_owned(),
                (0..index.parents(row).len())
                    .map(|parent| {
                        index
                            .parent_commit_id(row, parent)
                            .unwrap()
                            .as_ref()
                            .to_owned()
                    })
                    .collect(),
            )
        })
        .collect()
}

/// The rows of the whole history as pages give them. These fixtures have no
/// commit-graph, so the page walk orders the header topology.
fn page_rows(repo: &dyn GitRepository) -> Vec<(String, Vec<String>)> {
    repo.log_history_mode_page(
        HistoryMode::AllBranches,
        100_000,
        None,
        &CancellationToken::new(),
    )
    .unwrap()
    .commits
    .iter()
    .map(|commit| {
        (
            commit.id.as_ref().to_owned(),
            commit
                .parent_ids
                .iter()
                .map(|id| id.as_ref().to_owned())
                .collect(),
        )
    })
    .collect()
}

/// An index is grown from one that is still held, as a window holds the one
/// it shows; `held` now holds `index` for the next change.
fn keep(held: &mut HistoryIndexHandle, index: HistoryIndexHandle) {
    *held = index;
}

/// What building the index for the changed repository did.
fn rebuilt(repo: &dyn GitRepository) -> (HistoryIndexHandle, u64, u64) {
    let (builds, extends) = (
        count(Work::HistoryIndexBuild),
        count(Work::HistoryIndexExtend),
    );
    let index = index_of(repo);
    (
        index,
        count(Work::HistoryIndexBuild) - builds,
        count(Work::HistoryIndexExtend) - extends,
    )
}

#[test]
fn commits_on_top_grow_the_index_and_anything_else_rebuilds_it() {
    let mut fixture = Fixture::new();
    let a = fixture.commit("main", &[], 100);
    let b = fixture.commit("main", &[a], 110);
    let side = fixture.commit("side", &[a], 120);
    let repo = GixBackend.open(fixture.path()).unwrap();
    let _capture = capture();
    let mut held = index_of(repo.as_ref());
    assert_eq!(index_rows(&held), fixture.git_rows());

    // One commit on top, then a merge of the two branches.
    let c = fixture.commit("main", &[b], 130);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (0, 1), "a commit on top");
    assert_eq!(index_rows(&index), fixture.git_rows());
    keep(&mut held, index);

    let merge = fixture.commit("main", &[c, side], 140);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (0, 1), "a merge on top");
    assert_eq!(index_rows(&index), fixture.git_rows());
    keep(&mut held, index);

    // A branch whose commits main still reaches goes without a rebuild.
    run_git(fixture.path(), &["update-ref", "-d", "refs/heads/side"]);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (0, 1), "a merged branch is deleted");
    assert_eq!(index_rows(&index), fixture.git_rows());
    keep(&mut held, index);

    // A commit dated before the newest row sorts into the old rows.
    fixture.commit("late", &[a], 105);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (1, 0), "an older commit arrives");
    assert_eq!(index_rows(&index), fixture.git_rows());
    keep(&mut held, index);

    // A branch that takes its commits with it changes the old rows.
    run_git(fixture.path(), &["update-ref", "-d", "refs/heads/late"]);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (1, 0), "an unmerged branch is deleted");
    assert_eq!(index_rows(&index), fixture.git_rows());
    keep(&mut held, index);

    // A rewritten tip: main moves back and gets another commit.
    run_git(
        fixture.path(),
        &[
            "update-ref",
            "refs/heads/main",
            &format!("{}^", index_rows(&held)[0].0),
        ],
    );
    fixture.commit("main", &[c], 150);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (1, 0), "the tip is rewritten");
    assert_eq!(index_rows(&index), fixture.git_rows());
    drop((held, merge));
}

/// SplitMix64.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Every commit has its own committer time, so date order has one answer and
/// `git log --date-order` is the oracle after each change.
#[test]
fn an_index_kept_across_random_changes_matches_git_after_each() {
    for seed in 1..=6u64 {
        let mut rng = seed;
        let mut fixture = Fixture::new();
        // Even times move forward; odd times are free for commits dated back.
        let mut clock = 1_000i64;
        let tick = |clock: &mut i64| {
            *clock += 2;
            *clock
        };
        let mut branches: Vec<(String, usize)> = Vec::new();
        let mut marks: Vec<usize> = Vec::new();
        let root = fixture.commit("main", &[], tick(&mut clock));
        marks.push(root);
        branches.push(("main".into(), root));
        let repo = GixBackend.open(fixture.path()).unwrap();
        let _capture = capture();
        let mut held = index_of(repo.as_ref());
        let mut extends = 0;
        let mut serial = 0;
        let mut dated_back = rustc_hash::FxHashSet::default();
        for step in 0..40 {
            // Several changes between two indexes, so that added commits
            // depend on each other.
            for _ in 0..1 + next(&mut rng) % 4 {
                serial += 1;
                let pick = next(&mut rng) as usize;
                match next(&mut rng) % 10 {
                    // A commit on top of a branch.
                    0..=4 => {
                        let at = pick % branches.len();
                        let mark =
                            fixture.commit(&branches[at].0, &[branches[at].1], tick(&mut clock));
                        branches[at].1 = mark;
                        marks.push(mark);
                    }
                    // A merge of two branches.
                    5 if branches.len() > 1 => {
                        let (into, from) = (pick % branches.len(), (pick / 7) % branches.len());
                        if into != from {
                            let mark = fixture.commit(
                                &branches[into].0,
                                &[branches[into].1, branches[from].1],
                                tick(&mut clock),
                            );
                            branches[into].1 = mark;
                            marks.push(mark);
                        }
                    }
                    // A new branch on any existing commit.
                    6 => {
                        let name = format!("b{serial}");
                        let from = marks[pick % marks.len()];
                        let mark = fixture.commit(&name, &[from], tick(&mut clock));
                        branches.push((name, mark));
                        marks.push(mark);
                    }
                    // A commit dated back among the existing ones.
                    7 => {
                        let name = format!("old{serial}");
                        let from = marks[pick % marks.len()];
                        // Odd, and taken once, so every commit has its own time.
                        let time = (1_001 + 2 * (pick as i64 % ((clock - 1_000) / 2).max(1)))
                            .min(clock - 1)
                            | 1;
                        if !dated_back.insert(time) {
                            continue;
                        }
                        let mark = fixture.commit(&name, &[from], time);
                        branches.push((name, mark));
                        marks.push(mark);
                    }
                    // A branch is deleted, merged or not.
                    _ if branches.len() > 1 => {
                        let at = 1 + pick % (branches.len() - 1);
                        let (name, _) = branches.remove(at);
                        run_git(
                            fixture.path(),
                            &["update-ref", "-d", &format!("refs/heads/{name}")],
                        );
                    }
                    _ => {}
                }
            }
            let (index, _, extended) = rebuilt(repo.as_ref());
            extends += extended;
            assert_eq!(
                index_rows(&index),
                fixture.git_rows(),
                "seed {seed} step {step}"
            );
            assert_eq!(
                page_rows(repo.as_ref()),
                fixture.git_rows(),
                "seed {seed} step {step}: pages"
            );
            keep(&mut held, index);
        }
        assert!(
            extends >= 5,
            "seed {seed}: only {extends} of 40 steps grew the index"
        );
        drop(held);
    }
}

/// With equal committer times date order has several answers, so the grown
/// index is checked for being one of them: every row comes after all its
/// children and is as new as any other row that could have come instead.
fn assert_date_order(fixture: &Fixture, rows: &[(String, Vec<String>)], context: &str) {
    let times: rustc_hash::FxHashMap<String, i64> =
        git_stdout(fixture.path(), &["log", "--all", "--format=%H %ct"])
            .lines()
            .map(|line| {
                let (id, time) = line.split_once(' ').unwrap();
                (id.to_owned(), time.parse().unwrap())
            })
            .collect();
    assert_eq!(rows.len(), times.len(), "{context}: every commit, once");
    let mut waiting: rustc_hash::FxHashMap<&str, usize> =
        rows.iter().map(|(id, _)| (id.as_str(), 0)).collect();
    for parent in rows.iter().flat_map(|(_, parents)| parents) {
        *waiting.get_mut(parent.as_str()).expect("a parent is a row") += 1;
    }
    let mut shown = rustc_hash::FxHashSet::default();
    for (id, parents) in rows {
        assert_eq!(waiting[id.as_str()], 0, "{context}: {id} before a child");
        let newest_ready = waiting
            .iter()
            .filter(|(other, children)| **children == 0 && !shown.contains(*other))
            .map(|(other, _)| times[*other])
            .max()
            .unwrap();
        assert_eq!(times[id], newest_ready, "{context}: {id} is not the newest");
        shown.insert(id.as_str());
        for parent in parents {
            *waiting.get_mut(parent.as_str()).unwrap() -= 1;
        }
    }
}

#[test]
fn an_index_grown_among_equal_times_is_still_in_date_order() {
    for seed in 1..=4u64 {
        let mut rng = seed;
        let mut fixture = Fixture::new();
        // Time moves forward on one change in three, so most commits tie.
        let mut clock = 500i64;
        let root = fixture.commit("main", &[], clock);
        let mut branches: Vec<(String, usize)> = vec![("main".into(), root)];
        let repo = GixBackend.open(fixture.path()).unwrap();
        let _capture = capture();
        let mut held = index_of(repo.as_ref());
        let mut extends = 0;
        let mut serial = 0;
        for step in 0..30 {
            for _ in 0..1 + next(&mut rng) % 4 {
                serial += 1;
                clock += i64::from(next(&mut rng).is_multiple_of(3));
                let at = next(&mut rng) as usize % branches.len();
                match next(&mut rng) % 4 {
                    0 => {
                        let name = format!("b{serial}");
                        let mark = fixture.commit(&name, &[branches[at].1], clock);
                        branches.push((name, mark));
                    }
                    1 if branches.len() > 1 => {
                        let from = (at + 1) % branches.len();
                        branches[at].1 = fixture.commit(
                            &branches[at].0,
                            &[branches[at].1, branches[from].1],
                            clock,
                        );
                    }
                    _ => {
                        branches[at].1 = fixture.commit(&branches[at].0, &[branches[at].1], clock);
                    }
                }
            }
            let (index, _, extended) = rebuilt(repo.as_ref());
            extends += extended;
            let context = format!("seed {seed} step {step}");
            assert_date_order(&fixture, &index_rows(&index), &context);
            assert_date_order(
                &fixture,
                &page_rows(repo.as_ref()),
                &format!("{context}: pages"),
            );
            keep(&mut held, index);
        }
        assert!(
            extends >= 20,
            "seed {seed}: only {extends} of 30 changes grew the index"
        );
        drop(held);
    }
}

#[test]
fn a_stash_on_top_grows_the_index_and_is_marked_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-q", "-b", "main"]);
    run_git(path, &["config", "user.email", "you@example.com"]);
    run_git(path, &["config", "user.name", "You"]);
    std::fs::write(path.join("file.txt"), "one\n").unwrap();
    run_git(path, &["add", "file.txt"]);
    run_git(path, &["commit", "-qm", "first"]);
    let repo = GixBackend.open(path).unwrap();
    let _capture = capture();
    let held = index_of(repo.as_ref());
    assert_eq!(held.len(), 1);

    std::fs::write(path.join("file.txt"), "two\n").unwrap();
    run_git(path, &["stash", "-q"]);
    let (index, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (0, 1));
    let stash = git_stdout(path, &["rev-parse", "refs/stash"]);
    let stash_row = index.position(stash.trim()).expect("the stash is a row");
    assert_eq!(index.len(), 3, "the stash, its index commit and the base");
    for row in 0..index.len() {
        assert_eq!(index.is_probable_stash(row), row == stash_row, "row {row}");
    }
    drop(held);
}

/// A merge on top whose summary reads like a stash's is one only if its
/// other parents are shaped like a stash's; those are older rows here.
#[test]
fn a_merge_on_top_named_like_a_stash_is_none_in_a_grown_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-q", "-b", "main"]);
    run_git(path, &["config", "user.email", "you@example.com"]);
    run_git(path, &["config", "user.name", "You"]);
    std::fs::write(path.join("file.txt"), "one\n").unwrap();
    run_git(path, &["add", "file.txt"]);
    run_git(path, &["commit", "-qm", "first"]);
    run_git(path, &["switch", "-qc", "hotfix"]);
    std::fs::write(path.join("fix.txt"), "fix\n").unwrap();
    run_git(path, &["add", "fix.txt"]);
    run_git(path, &["commit", "-qm", "fix"]);
    run_git(path, &["switch", "-q", "main"]);
    std::fs::write(path.join("file.txt"), "two\n").unwrap();
    run_git(path, &["commit", "-qam", "second"]);
    let repo = GixBackend.open(path).unwrap();
    let _capture = capture();
    let held = index_of(repo.as_ref());
    assert_eq!(held.len(), 3);

    run_git(
        path,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "On release: merge hotfix",
            "hotfix",
        ],
    );
    let (grown, builds, extends) = rebuilt(repo.as_ref());
    assert_eq!((builds, extends), (0, 1));
    let rebuilt = index_of(GixBackend.open(path).unwrap().as_ref());
    assert_eq!(index_rows(&grown), index_rows(&rebuilt));
    for row in 0..grown.len() {
        assert!(!rebuilt.is_probable_stash(row), "row {row}");
        assert!(!grown.is_probable_stash(row), "row {row}");
    }
    drop(held);
}

#[test]
fn pages_after_commits_on_top_read_only_the_added_headers() {
    let mut fixture = Fixture::new();
    let mut tip = fixture.commit("main", &[], 100);
    for time in 101..140 {
        tip = fixture.commit("main", &[tip], time);
    }
    let side = fixture.commit("side", &[tip], 140);
    let repo = GixBackend.open(fixture.path()).unwrap();
    let _capture = capture();
    let work = || {
        (
            count(Work::LogTopologyBuild),
            count(Work::LogTopologyExtend),
            count(Work::LogWalkObjectRead),
        )
    };
    assert_eq!(page_rows(repo.as_ref()), fixture.git_rows());
    assert_eq!(work(), (1, 0, 41));

    // Two commits and a merge on top: three headers, no scan of the rest.
    let next = fixture.commit("main", &[tip], 141);
    let merged = fixture.commit("main", &[next, side], 142);
    fixture.commit("main", &[merged], 143);
    assert_eq!(page_rows(repo.as_ref()), fixture.git_rows());
    assert_eq!(work(), (1, 1, 44));

    // A commit dated among the older rows is sorted by a full scan, after
    // its own header showed that it is.
    fixture.commit("late", &[tip], 120);
    assert_eq!(page_rows(repo.as_ref()), fixture.git_rows());
    assert_eq!(work(), (2, 1, 44 + 1 + 45));
}
