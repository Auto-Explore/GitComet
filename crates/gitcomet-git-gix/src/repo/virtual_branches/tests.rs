use super::*;

use gitcomet_core::services::{ForcePushLease, GitRepository, PullMode, ResetMode};

fn git(root: &Path, args: &[&str]) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    let output = cmd.output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The executable bit a fixture needs git to record.
///
/// The working copy gets a real `+x` where the platform has one, because
/// with `core.filemode=true` (Linux) `git add` reads the mode from the
/// file. The index is also told explicitly, because with
/// `core.filemode=false` (Windows) the bit cannot be learned from the
/// filesystem at all — `update-index --chmod=+x` is the only way to stage
/// `100755` there.
fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A repository with two virtual branches whose changes touch different
/// files, so applying and unapplying one is visible in the other's absence.
fn repo_with_two_branches() -> (tempfile::TempDir, GixRepo, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("shared.txt"), "a1\na2\na3\n").unwrap();
    std::fs::write(root.join("moved.txt"), "x\n").unwrap();
    std::fs::write(root.join("mine.txt"), "y\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
    // Captured while HEAD is still the target: taking it after the branch
    // checkouts below would name `api` as both the target and a branch,
    // which `write_virtual_branches` refuses.
    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

    // Two siblings off the target, one change each — that is what makes
    // applying and unapplying either one independently observable.
    git(root, &["branch", "api"]);
    git(root, &["branch", "ui"]);
    git(root, &["checkout", "api"]);
    std::fs::write(root.join("shared.txt"), "a1\nAPI\na3\n").unwrap();
    git(root, &["commit", "-am", "api"]);

    git(root, &["checkout", "ui"]);
    std::fs::write(root.join("moved.txt"), "ui-x\n").unwrap();
    git(root, &["commit", "-am", "ui"]);

    git(root, &["checkout", &default_branch]);

    let state = VirtualBranchesState::new(&default_branch)
        .with_branches(vec![VirtualBranch::new("api"), VirtualBranch::new("ui")]);
    let repo = open_repo(root);
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    (temp, repo, default_branch)
}

fn open_repo(workdir: &Path) -> GixRepo {
    let thread_safe = gix::open(workdir).expect("open repo").into_sync();
    GixRepo::new(workdir.to_path_buf(), thread_safe).expect("gix repo")
}

fn read(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join(name)).expect("read file")
}

#[test]
fn entering_the_virtual_branches_puts_every_applied_branch_in_the_files() {
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    let tip = repo
        .enter_virtual_branches()
        .expect("enter virtual branches");

    assert_eq!(read(root, "shared.txt"), "a1\nAPI\na3\n", "api is applied");
    assert_eq!(read(root, "moved.txt"), "ui-x\n", "ui is applied");
    assert_eq!(read(root, "mine.txt"), "y\n");
    assert_eq!(
        repo.current_branch(&gitcomet_core::services::CancellationToken::new())
            .expect("current branch"),
        VIRTUAL_BRANCHES_BRANCH,
        "the working directory is the virtual branches"
    );
    assert_eq!(tip.as_ref(), git(root, &["rev-parse", "HEAD"]).as_str());
}

#[test]
fn unapplying_a_branch_keeps_edits_to_files_it_never_touched() {
    // The property the whole design rests on: a rebuild moves the working
    // directory without taking uncommitted work with it.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    repo.enter_virtual_branches()
        .expect("enter virtual branches");

    std::fs::write(root.join("mine.txt"), "y\nMY UNCOMMITTED WORK\n").unwrap();

    let mut state = repo.read_virtual_branches().expect("read virtual branches");
    state
        .set_applied(
            "ui",
            gitcomet_core::virtual_branches::BranchApplyState::Unapplied,
        )
        .expect("unapply");
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    repo.update_virtual_branches_branch(&["api"])
        .expect("rebuild");

    assert_eq!(
        read(root, "moved.txt"),
        "x\n",
        "the unapplied branch's change is gone"
    );
    assert_eq!(
        read(root, "mine.txt"),
        "y\nMY UNCOMMITTED WORK\n",
        "an unrelated edit survives the rebuild"
    );
    assert_eq!(read(root, "shared.txt"), "a1\nAPI\na3\n");
}

#[test]
fn a_refused_sync_leaves_the_files_and_the_ref_exactly_as_they_were() {
    // GitComet never discards uncommitted changes. A file that is both
    // edited and changed by the switch has no automatic answer, so the
    // switch is refused — and because the sync runs before the ref moves,
    // a refusal cannot leave the two disagreeing.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    repo.enter_virtual_branches()
        .expect("enter virtual branches");
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    // Edit the very file `api` changed, then unapply it.
    std::fs::write(root.join("shared.txt"), "a1\nAPI\nMINE\na3\n").unwrap();
    let mut state = repo.read_virtual_branches().expect("read virtual branches");
    state
        .set_applied(
            "api",
            gitcomet_core::virtual_branches::BranchApplyState::Unapplied,
        )
        .expect("unapply");
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");

    assert!(
        repo.update_virtual_branches_branch(&["ui"]).is_err(),
        "a switch that would overwrite a local edit is refused"
    );
    assert_eq!(
        read(root, "shared.txt"),
        "a1\nAPI\nMINE\na3\n",
        "the file is untouched"
    );
    assert_eq!(
        git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]),
        before,
        "and so is the branch, because the ref is only moved after a sync succeeds"
    );
}

#[test]
fn a_rebuild_does_not_touch_the_files_of_a_virtual_branches_that_is_not_checked_out() {
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    assert!(!repo.virtual_branches_is_checked_out());

    repo.update_virtual_branches_branch(&["api", "ui"])
        .expect("rebuild");

    assert_eq!(
        read(root, "moved.txt"),
        "x\n",
        "the ref moved, but a virtual branches nobody has entered must not move files"
    );
    assert_eq!(read(root, "shared.txt"), "a1\na2\na3\n");
}

#[test]
fn leaving_the_virtual_branches_returns_to_the_branch_it_came_from() {
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    let original = repo
        .current_branch(&gitcomet_core::services::CancellationToken::new())
        .expect("current branch");
    repo.enter_virtual_branches()
        .expect("enter virtual branches");

    repo.leave_virtual_branches(&original)
        .expect("leave virtual branches");
    assert_eq!(
        repo.current_branch(&gitcomet_core::services::CancellationToken::new())
            .expect("current branch"),
        original
    );
    assert_eq!(
        read(root, "moved.txt"),
        "x\n",
        "the applied branch's change is no longer in the files"
    );
}

#[test]
fn a_conflict_names_the_applied_branch_that_is_in_the_way() {
    // Two branches that both change the same line cannot both be applied,
    // and the message has to say which pair: the merge itself only knows
    // that the branch it was adding does not fit, because the others are
    // already one merged tree by the time it runs.
    let (temp, repo, _) = repo_with_two_branches();
    git(temp.path(), &["checkout", "api"]);
    std::fs::write(temp.path().join("moved.txt"), "from-api\n").unwrap();
    git(temp.path(), &["commit", "-am", "clash"]);

    let error = repo
        .update_virtual_branches_branch(&["api", "ui"])
        .expect_err("api and ui now change the same line");
    let message = error.to_string();
    assert!(
        message.contains("'ui' conflicts with 'api'"),
        "the message names both branches, got: {message}"
    );
    assert!(
        message.contains("moved.txt"),
        "and the file they clash in, got: {message}"
    );
}

#[test]
fn conflicted_paths_are_read_from_stdout_because_stderr_is_empty() {
    // git writes the conflict to stdout and exits non-zero. Reading stderr
    // — the obvious thing — yields nothing at all.
    let error = Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
        "git merge-tree",
        gitcomet_core::error::GitFailureId::CommandFailed,
        Some(1),
        b"0d69e0ee71279a47cf8802e7400473e50e196110\n\
          100644 aaa 1\tsrc/a.rs\n\
          100644 bbb 2\tsrc/a.rs\n\
          100644 ccc 3\tsrc/a.rs\n\
          100644 ddd 1\tsrc/b.rs\n\
          100644 eee 2\tsrc/b.rs\n\
          100644 fff 3\tsrc/b.rs\n\
          \nAuto-merging src/a.rs\nCONFLICT (content): Merge conflict in src/a.rs\n"
            .to_vec(),
        Vec::new(),
        None,
    )));
    assert_eq!(
        conflicted_paths(&error),
        ["src/a.rs", "src/b.rs"],
        "one entry per path: git writes a line per stage, so a content \
         conflict is three lines naming the same file, and a caller showing \
         these would list one file three times"
    );
}

#[test]
fn only_exit_one_counts_as_a_merge_conflict() {
    // `git merge-tree --write-tree` needs Git 2.38; an older one answers a
    // usage error with exit 129 and a message on stderr. Reporting that as
    // a conflict would send the user to unapply a branch that has nothing
    // to do with what went wrong.
    let failed = |code| {
        Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
            "git merge-tree",
            gitcomet_core::error::GitFailureId::CommandFailed,
            Some(code),
            Vec::new(),
            b"error: unknown option".to_vec(),
            None,
        )))
    };

    assert!(
        merge_tree_conflicted(&failed(1)),
        "1 is git's conflict code"
    );
    assert!(!merge_tree_conflicted(&failed(129)), "129 is a usage error");
    assert!(
        !merge_tree_conflicted(&Error::new(ErrorKind::Backend("nope".to_string()))),
        "an error that never ran git cannot be a conflict in git's output"
    );
}

#[test]
fn a_real_merge_conflict_names_each_path_once() {
    // The same thing against a repository rather than a hand-written
    // fixture, because the shape of `merge-tree`'s output is git's and
    // changes between versions.
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("same.txt"), "base\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

    git(root, &["checkout", "-b", "left"]);
    std::fs::write(root.join("same.txt"), "A\n").unwrap();
    git(root, &["commit", "-am", "left"]);
    git(root, &["checkout", &default_branch]);
    git(root, &["checkout", "-b", "right"]);
    std::fs::write(root.join("same.txt"), "B\n").unwrap();
    git(root, &["commit", "-am", "right"]);

    let repo = open_repo(root);
    let left = repo
        .resolve_revision("left")
        .expect("resolve")
        .expect("left exists");
    let right = repo
        .resolve_revision("right")
        .expect("resolve")
        .expect("right exists");
    // The very command `merge_tree` runs, kept here rather than through a
    // helper so the test cannot pass by agreeing with a bug in one.
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .arg("merge-tree")
        .arg("--write-tree")
        .arg(format!(
            "--merge-base={}",
            merge_base_of(&repo, &left, &right)
        ))
        .arg(left.as_ref())
        .arg(right.as_ref());
    let output = cmd.output().expect("run git");
    assert!(
        !output.status.success(),
        "this conflict has to fail the merge"
    );

    let error = Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
        "git merge-tree",
        gitcomet_core::error::GitFailureId::CommandFailed,
        output.status.code(),
        output.stdout,
        output.stderr,
        None,
    )));
    assert_eq!(conflicted_paths(&error), ["same.txt"]);
}

#[test]
fn a_backend_error_names_no_conflicting_paths_rather_than_guessing() {
    let error = Error::new(ErrorKind::Backend("something else failed".into()));
    assert!(conflicted_paths(&error).is_empty());
    assert_eq!(describe_paths(&[]), "files git did not name");
}

#[test]
fn virtual_branches_branches_are_recognized_by_their_prefix() {
    assert!(is_virtual_branches_branch(VIRTUAL_BRANCHES_BRANCH));
    assert!(is_virtual_branches_branch("gitcomet/anything"));
    assert!(!is_virtual_branches_branch("feature/api"));
    assert!(!is_virtual_branches_branch("main"));
}

#[test]
fn the_state_file_sits_inside_the_virtual_branches_directory() {
    let dir = PathBuf::from("/repo/.git").join(VIRTUAL_BRANCHES_DIR);
    assert!(dir.ends_with(VIRTUAL_BRANCHES_DIR));
    assert_eq!(
        dir.join(VIRTUAL_BRANCHES_STATE_FILE).file_name().unwrap(),
        "virtual branches.json"
    );
}

#[test]
fn only_the_two_executable_modes_are_read_out_of_git() {
    // `git ls-tree` names a gitlink or a symlink too, and staging one of
    // those as `100644` would turn it into a plain file the moment a file
    // was split between branches.
    assert_eq!(parse_mode("100755 blob abc\trun.sh"), Some("100755"));
    assert_eq!(parse_mode("100644 blob abc\tsrc/lib.rs"), Some("100644"));
    assert_eq!(parse_mode("120000 blob abc\tlink"), None);
    assert_eq!(parse_mode("160000 commit abc\tvendor"), None);
    assert_eq!(parse_mode(""), None, "nothing to read");
}

#[test]
fn a_split_file_keeps_the_mode_it_had() {
    // The user's real index wins over the base tree, so a mode they staged
    // survives the split rather than being quietly rewritten.
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
    std::fs::write(root.join("plain.txt"), "text\n").unwrap();
    make_executable(&root.join("run.sh"));
    git(root, &["add", "-A"]);
    git(root, &["update-index", "--chmod=+x", "run.sh"]);
    git(root, &["commit", "-m", "base"]);

    let repo = open_repo(root);
    let base = repo
        .resolve_revision("HEAD")
        .expect("resolve")
        .expect("a base");
    assert_eq!(
        repo.path_mode(&base, Path::new("run.sh")).unwrap(),
        "100755"
    );
    assert_eq!(
        repo.path_mode(&base, Path::new("plain.txt")).unwrap(),
        "100644"
    );
    assert_eq!(
        repo.path_mode(&base, Path::new("new.txt")).unwrap(),
        "100644",
        "a file that is not in the base is a plain file"
    );
}

#[test]
fn two_sibling_branches_are_merged_from_the_commit_they_share() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("base.txt"), "b\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
    // Before any `checkout -b`: afterwards HEAD is on the new branch and
    // the target would come out as `left`, making the second branch a
    // child of the first rather than a sibling.
    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

    let repo = open_repo(root);
    let base = repo
        .resolve_revision("HEAD")
        .expect("resolve")
        .expect("the base commit exists");

    git(root, &["checkout", "-b", "left"]);
    std::fs::write(root.join("left.txt"), "l\n").unwrap();
    // A new file is untracked: `commit -am` only picks up tracked paths
    // and would fail with "nothing to commit" on stderr's silence.
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "left"]);
    git(root, &["checkout", &default_branch]);
    git(root, &["checkout", "-b", "right"]);
    std::fs::write(root.join("right.txt"), "r\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "right"]);

    let left = repo
        .resolve_revision("left")
        .expect("resolve")
        .expect("left exists");
    let right = repo
        .resolve_revision("right")
        .expect("resolve")
        .expect("right exists");
    assert_ne!(
        left, right,
        "they have to be siblings for this to mean anything"
    );
    assert_eq!(
        merge_base_of(&repo, &left, &right),
        base.as_ref(),
        "a merge from the wrong base would resurrect one branch's commits as \
         conflicts against the other's"
    );
}

#[test]
fn branches_with_no_shared_history_merge_against_ours() {
    // `git merge-tree` needs an explicit base and there is none here.
    // Falling back to `ours` gives a two-way merge, which surfaces the
    // clash rather than silently picking a side.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    git(root, &["checkout", "--orphan", "lonely"]);
    git(root, &["rm", "-rfq", "."]);
    std::fs::write(root.join("other.txt"), "z\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "lonely"]);

    let api = repo
        .resolve_revision("api")
        .expect("resolve")
        .expect("api exists");
    let lonely = repo
        .resolve_revision("lonely")
        .expect("resolve")
        .expect("lonely exists");
    assert_eq!(merge_base_of(&repo, &api, &lonely), api.as_ref());
}

#[test]
fn one_conflicting_path_is_named_and_several_are_counted() {
    assert_eq!(describe_paths(&["src/lib.rs".to_string()]), "src/lib.rs");
    assert_eq!(
        describe_paths(&["src/lib.rs".to_string(), "src/main.rs".to_string()]),
        "src/lib.rs (and 1 more)",
    );
    assert_eq!(
        describe_paths(&["a".to_string(), "b".to_string(), "c".to_string()]),
        "a (and 2 more)"
    );
}

#[test]
fn a_conflict_names_the_other_branch_or_says_that_it_could_not() {
    let against = conflict_message("feature/ui", Some("feature/api"), &["src/lib.rs".into()]);
    assert!(
        against.contains("'feature/ui'") && against.contains("'feature/api'"),
        "the two branches in the way have to be named: {against}"
    );

    let nameless = conflict_message("feature/ui", None, &[]);
    assert!(
        nameless.contains("feature/ui") && !nameless.contains("conflicts with '"),
        "with no branch to blame the message must not invent one: {nameless}"
    );
}

// ── Committing assigned paths ─────────────────────────────────────

/// Ten lines, edited at the first and the last.
///
/// The gap between the two edits is eight unchanged lines, past the six
/// that `git diff` folds into one `@@`, so this is two hunks to the user
/// and two spans here — which is what makes it splittable at all.
const SPLIT_BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
const SPLIT_WORKING: &str = "API\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nL10\n";

/// A repository with two sibling virtual branches and one file that can be
/// split between them, plus an unrelated file that must never be dragged in.
fn repo_with_a_splittable_file() -> (tempfile::TempDir, GixRepo, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("shared.txt"), SPLIT_BASE).unwrap();
    std::fs::write(root.join("other.txt"), "keep\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);

    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
    git(root, &["branch", "api"]);
    git(root, &["branch", "ui"]);

    let state = VirtualBranchesState::new(&default_branch)
        .with_branches(vec![VirtualBranch::new("api"), VirtualBranch::new("ui")]);
    let repo = open_repo(root);
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    (temp, repo, default_branch)
}

/// A file's contents at a revision, exactly as git has them.
fn blob(root: &Path, revision: &str, path: &str) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .arg("show")
        .arg(format!("{revision}:{path}"));
    let output = cmd.output().expect("run git");
    assert!(output.status.success(), "git show {revision}:{path} failed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Split `shared.txt` between the two branches, one hunk each.
fn assign_the_two_hunks(repo: &GixRepo) -> usize {
    let spans = hunk_spans(SPLIT_BASE, SPLIT_WORKING);
    let mut index = AssignmentIndex::new();
    for (span, branch) in spans.iter().zip(["api", "ui"]) {
        index.set_hunk(
            Path::new("shared.txt"),
            span.fingerprint,
            Some(branch.into()),
        );
    }
    repo.write_virtual_branches_assignments(&index)
        .expect("write assignments");
    spans.len()
}

#[test]
fn two_edits_far_apart_are_two_hunks_to_split_between_branches() {
    // The premise the rest of these tests stand on: if this ever said one,
    // "split a file between two branches" would be impossible and the
    // assignments would silently do nothing.
    assert_eq!(hunk_spans(SPLIT_BASE, SPLIT_WORKING).len(), 2);
    // Edits at line 1 and line 8: six unchanged lines between them, which
    // is exactly `2 * HUNK_CONTEXT_LINES`, so the contexts touch and git
    // prints a single `@@`.
    assert_eq!(
        hunk_spans(SPLIT_BASE, "API\nl2\nl3\nl4\nl5\nl6\nl7\nL8\nl9\nl10\n").len(),
        1,
        "edits within git's context are one hunk, not two"
    );
}

#[test]
fn a_commit_to_one_branch_takes_only_the_hunks_assigned_to_it() {
    let (temp, repo, default_branch) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
    assert_eq!(assign_the_two_hunks(&repo), 2);

    let outcome = repo
        .commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
        .expect("commit to api");

    assert_eq!(
        blob(root, "api", "shared.txt"),
        "API\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n",
        "the commit holds this branch's hunk and not the other branch's"
    );
    assert_eq!(
        git(root, &["rev-parse", "api~1"]),
        git(root, &["rev-parse", &default_branch]),
        "the commit is built on the branch's own base, never on the virtual branches"
    );
    assert_eq!(outcome.local_branch.as_deref(), Some("api"));
    assert_eq!(
        read(root, "shared.txt"),
        SPLIT_WORKING,
        "committing to a branch must not change what the user is looking at"
    );
}

#[test]
fn both_branches_commit_their_own_half_and_the_virtual_branches_gathers_both() {
    // The point of the whole feature: two unrelated changes in one working
    // directory, each landing on its own branch, and the virtual branches showing
    // both.
    let (temp, repo, _) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
    assign_the_two_hunks(&repo);

    repo.commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
        .expect("commit to api");
    repo.commit_paths_to_virtual_branch("ui", "the ui change", &[Path::new("shared.txt")])
        .expect("commit to ui");

    assert_eq!(
        blob(root, "ui", "shared.txt"),
        "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nL10\n",
        "the second commit must not pick up the first branch's hunk"
    );
    assert_eq!(
        blob(root, VIRTUAL_BRANCHES_BRANCH, "shared.txt"),
        SPLIT_WORKING,
        "the virtual branches is the two branches together, which is what makes the \
         working directory show both changes"
    );
}

#[test]
fn a_stacked_branch_commits_on_top_of_its_parents_newest_commit() {
    // Otherwise a pull request for the upper branch would contain only its
    // own change, with the lower branch's work missing from its history.
    let (temp, repo, _) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
    let mut state = repo.read_virtual_branches().expect("read virtual branches");
    state
        .set_parent("ui", Some("api"))
        .expect("stacked branch ui on api");
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    assign_the_two_hunks(&repo);

    repo.commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
        .expect("commit to api");
    let api_tip = git(root, &["rev-parse", "api"]);
    repo.commit_paths_to_virtual_branch("ui", "the ui change", &[Path::new("shared.txt")])
        .expect("commit to ui");

    assert_eq!(
        git(root, &["rev-parse", "ui~1"]),
        api_tip,
        "the stacked branch has to be built on where its parent ended up"
    );
}

#[test]
fn only_the_paths_named_are_committed() {
    let (temp, repo, _) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::write(root.join("mine.txt"), "y\n").unwrap();
    let mut index = AssignmentIndex::new();
    index.set(Path::new("mine.txt"), Some("api".into()));
    repo.write_virtual_branches_assignments(&index)
        .expect("write assignments");

    repo.commit_paths_to_virtual_branch("api", "just the new file", &[Path::new("mine.txt")])
        .expect("commit");

    assert_eq!(blob(root, "api", "mine.txt"), "y\n");
    assert_eq!(
        git(root, &["ls-tree", "--name-only", "api"]),
        "mine.txt\nother.txt\nshared.txt",
        "a file nobody assigned comes from the base, not from the working tree"
    );
}

#[test]
fn a_file_deleted_in_the_working_tree_is_deleted_on_its_branch() {
    // Deleting a file is a change to it, and a branch assigned it.
    let (temp, repo, _) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::remove_file(root.join("shared.txt")).unwrap();

    repo.commit_paths_to_virtual_branch("api", "drop the shared file", &[Path::new("shared.txt")])
        .expect("commit");

    assert_eq!(
        git(root, &["ls-tree", "--name-only", "api"]),
        "other.txt",
        "the deletion has to land on the branch too"
    );
    assert!(
        !root
            .join(".git")
            .join("gitcomet")
            .join("commit-index")
            .exists(),
        "the temporary index has to be cleaned up"
    );
}

#[test]
fn committing_leaves_the_users_index_and_staging_exactly_as_they_were() {
    // The commit is built through a temporary index. If it were built
    // through the real one, this would stage the commit away and swallow
    // whatever the user had staged for their next commit.
    let (temp, repo, _) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::write(root.join("shared.txt"), SPLIT_WORKING).unwrap();
    std::fs::write(root.join("other.txt"), "staged edit\n").unwrap();
    git(root, &["add", "-A"]);
    let staged_before = git(root, &["diff", "--cached", "--name-only"]);

    repo.commit_paths_to_virtual_branch("api", "the api change", &[Path::new("shared.txt")])
        .expect("commit");

    assert_eq!(
        git(root, &["diff", "--cached", "--name-only"]),
        staged_before,
        "the user's staging has to survive a virtual branches commit untouched"
    );
    assert_eq!(read(root, "shared.txt"), SPLIT_WORKING);
    assert_eq!(read(root, "other.txt"), "staged edit\n");
}

#[test]
fn an_executable_file_stays_executable_after_a_commit_to_a_branch() {
    // Splitting or committing goes through an index entry this code writes
    // by hand, and a mode guessed wrong turns a script into a text file.
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
    make_executable(&root.join("run.sh"));
    git(root, &["add", "-A"]);
    git(root, &["update-index", "--chmod=+x", "run.sh"]);
    git(root, &["commit", "-m", "base"]);

    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
    git(root, &["branch", "api"]);
    let state =
        VirtualBranchesState::new(&default_branch).with_branches(vec![VirtualBranch::new("api")]);
    let repo = open_repo(root);
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");

    std::fs::write(root.join("run.sh"), "#!/bin/sh\necho bye\n").unwrap();
    let mut index = AssignmentIndex::new();
    index.set(Path::new("run.sh"), Some("api".into()));
    repo.write_virtual_branches_assignments(&index)
        .expect("write assignments");

    repo.commit_paths_to_virtual_branch("api", "shout", &[Path::new("run.sh")])
        .expect("commit");

    let entry = git(root, &["ls-tree", "api", "--", "run.sh"]);
    assert!(
        entry.starts_with("100755 "),
        "the mode has to come from the base, not be defaulted to a plain \
         file: {entry}"
    );
}

#[test]
fn committing_to_a_branch_that_is_not_in_the_virtual_branches_is_refused() {
    // Otherwise a typo would create a commit on a ref the virtual branches has no
    // idea about, invisible in every stacked branch in the UI.
    let (temp, repo, _) = repo_with_a_splittable_file();
    let root = temp.path();
    std::fs::write(root.join("mine.txt"), "y\n").unwrap();

    let result = repo.commit_paths_to_virtual_branch("nowhere", "m", &[Path::new("mine.txt")]);
    assert!(
        result.is_err(),
        "a branch the virtual branches does not have is an error"
    );
    assert!(
        !root
            .join(".git")
            .join("refs")
            .join("heads")
            .join("nowhere")
            .exists(),
        "a refused commit must not create the ref either"
    );
}

// ── Rebuilding, restacking, and what is on disk ──────────────

/// Whether a git command succeeds, for the assertions where failure *is* the
/// expected outcome.
fn git_succeeds(root: &Path, args: &[&str]) -> bool {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// The subjects on the virtual-branches branch, newest first.
fn virtual_branches_log(root: &Path) -> String {
    git(root, &["log", "--format=%s", VIRTUAL_BRANCHES_BRANCH])
}

/// Two branches that changed the same line, so applying both cannot work.
fn repo_with_conflicting_branches() -> (tempfile::TempDir, GixRepo, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("same.txt"), "base\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

    git(root, &["checkout", "-b", "left"]);
    std::fs::write(root.join("same.txt"), "A\n").unwrap();
    git(root, &["commit", "-am", "left"]);
    git(root, &["checkout", &default_branch]);
    git(root, &["checkout", "-b", "right"]);
    std::fs::write(root.join("same.txt"), "B\n").unwrap();
    git(root, &["commit", "-am", "right"]);
    git(root, &["checkout", &default_branch]);

    let state = VirtualBranchesState::new(&default_branch).with_branches(vec![
        VirtualBranch::new("left"),
        VirtualBranch::new("right"),
    ]);
    let repo = open_repo(root);
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    (temp, repo, default_branch)
}

/// An independent branch with a stacked one on it, plus a second target
/// that also moved.
fn repo_with_a_stacked_pair() -> (tempfile::TempDir, GixRepo, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("base.txt"), "base\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);

    git(root, &["checkout", "-b", "api"]);
    std::fs::write(root.join("api.txt"), "api\n").unwrap();
    // New files are untracked; `commit -am` alone stages nothing and fails.
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "api"]);
    git(root, &["checkout", "-b", "ui"]);
    std::fs::write(root.join("ui.txt"), "ui\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "ui"]);
    git(root, &["checkout", &default_branch]);

    // A second target that moved too, so pointing the virtual branches at it is a
    // real change rather than a rename.
    git(root, &["checkout", "-b", "next"]);
    std::fs::write(root.join("next.txt"), "next\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "next"]);
    git(root, &["checkout", &default_branch]);

    let state = VirtualBranchesState::new(&default_branch).with_branches(vec![
        VirtualBranch::new("api"),
        VirtualBranch::new("ui").with_parent("api"),
    ]);
    let repo = open_repo(root);
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    (temp, repo, default_branch)
}

#[test]
fn an_empty_applied_set_points_the_virtual_branches_at_the_target() {
    // Unapplying everything is a legitimate state, and then the virtual branches
    // branch *is* the target rather than a branch whose tree nobody can
    // explain.
    let (temp, repo, default_branch) = repo_with_two_branches();
    let root = temp.path();

    let tip = repo
        .update_virtual_branches_branch(&[])
        .expect("rebuild with nothing applied");

    assert_eq!(
        tip,
        repo.resolve_revision(&default_branch)
            .expect("resolve")
            .expect("the target exists"),
        "with nothing applied the virtual-branches branch is the target"
    );
    assert_eq!(
        git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]),
        git(root, &["rev-parse", &default_branch])
    );
    assert!(
        !virtual_branches_log(root).contains("Apply "),
        "a merge that did not happen must not leave a commit: {}",
        virtual_branches_log(root)
    );
}

#[test]
fn rebuilding_the_virtual_branches_branch_is_a_pure_function_of_the_applied_set() {
    // Rebuilt from the target every time, never patched. If merges piled up,
    // unapplying a branch would leave its merge commit in the history for
    // good and the branch could no longer be explained by the virtual
    // branches alone.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();

    repo.update_virtual_branches_branch(&["api", "ui"])
        .expect("apply both");
    let both = virtual_branches_log(root);
    assert!(
        both.contains("Apply api") && both.contains("Apply ui"),
        "{both}"
    );

    repo.update_virtual_branches_branch(&["api"])
        .expect("unapply ui");
    let one = virtual_branches_log(root);
    assert!(one.contains("Apply api"), "{one}");
    assert!(
        !one.contains("Apply ui"),
        "the merge that is no longer wanted is still in the history: {one}"
    );
}

#[test]
fn rebuilding_an_unchanged_applied_set_appends_nothing() {
    // Every reload of the Virtual branches tab rebuilds. A rebuild that invented a
    // commit would lengthen the branch every time the user looked at it.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();

    repo.update_virtual_branches_branch(&["api", "ui"])
        .expect("first");
    let tree = format!("{VIRTUAL_BRANCHES_BRANCH}^{{tree}}");
    let tree_after_first = git(root, &["rev-parse", &tree]);
    let commits_after_first = virtual_branches_log(root).lines().count();

    repo.update_virtual_branches_branch(&["api", "ui"])
        .expect("second");

    assert_eq!(
        virtual_branches_log(root).lines().count(),
        commits_after_first,
        "an identical rebuild grew the branch"
    );
    assert_eq!(
        git(root, &["rev-parse", &tree]),
        tree_after_first,
        "and it changed the tree it points at"
    );
    assert_eq!(
        commits_after_first, 3,
        "the base plus one commit per applied branch, and nothing else"
    );
}

#[test]
fn restacking_a_branch_onto_where_it_already_is_changes_nothing() {
    let (temp, repo, default_branch) = repo_with_a_splittable_file();
    let root = temp.path();
    let before = git(root, &["rev-parse", "api"]);

    repo.rebase_virtual_branch("api", &default_branch)
        .expect("restack onto the base it is already on");

    assert_eq!(
        git(root, &["rev-parse", "api"]),
        before,
        "a branch already on its base must not gain a commit"
    );
}

#[test]
fn applying_two_branches_that_conflict_names_both_and_writes_nothing() {
    // The failure the user has to act on: which two branches are in the way,
    // and over which file. A bare "merge failed" is not actionable.
    let (temp, repo, default_branch) = repo_with_conflicting_branches();
    let root = temp.path();

    let Err(error) = repo.update_virtual_branches_branch(&["left", "right"]) else {
        panic!("these two branches edit the same line; the rebuild must be refused");
    };
    let message = format!("{error}");
    assert!(
        message.contains("left") && message.contains("right"),
        "the two branches in the way have to be named: {message}"
    );
    assert!(
        message.contains("same.txt"),
        "and so does the file they clash on: {message}"
    );
    assert!(
        !git_succeeds(
            root,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{VIRTUAL_BRANCHES_BRANCH}")
            ]
        ),
        "a refused rebuild must not leave a half-built branch behind"
    );
    assert_eq!(
        git(root, &["symbolic-ref", "--short", "HEAD"]),
        default_branch,
        "and must not have moved the working directory either"
    );
}

#[test]
fn moving_the_target_moves_the_independent_branches_and_leaves_the_stacked_ones() {
    let (temp, repo, _) = repo_with_a_stacked_pair();
    let root = temp.path();
    let ui_parent_before = git(root, &["rev-parse", "ui^"]);
    let next = git(root, &["rev-parse", "next"]);

    repo.update_virtual_branches_target("next")
        .expect("move the target");

    assert_eq!(repo.read_virtual_branches().expect("read").target, "next");
    assert_eq!(
        git(root, &["rev-parse", "api^"]),
        next,
        "an independent branch is based on the target, so it has to follow it"
    );
    assert_eq!(
        git(root, &["rev-parse", "ui^"]),
        ui_parent_before,
        "a stacked branch moves with its base, not with the target: replaying \
         it here too would rewrite it twice"
    );
}

#[test]
fn a_corrupt_virtual_branches_file_reads_as_an_empty_virtual_branches() {
    // A file written by a newer version, or half-written by a crash, must
    // not make the whole repository unusable: the user rebuilds from the UI.
    let (_temp, repo, _) = repo_with_two_branches();
    std::fs::write(repo.virtual_branches_state_path(), b"{ this is not json").expect("corrupt it");

    let state = repo
        .read_virtual_branches()
        .expect("a corrupt file is not an error");
    assert!(
        state.branches.is_empty(),
        "the branches are gone from the file, so they are gone here too"
    );
}

#[test]
fn a_virtual_branches_file_that_cannot_be_read_is_an_error_not_an_empty_virtual_branches() {
    // Corrupt is recoverable — the user rebuilds from the UI. Unreadable is
    // not: reported as "empty", the next edit writes over a file the user
    // still cannot read, and that data is gone.
    let (_temp, repo, _) = repo_with_two_branches();
    std::fs::remove_file(repo.virtual_branches_state_path()).expect("remove the file");
    std::fs::create_dir(repo.virtual_branches_state_path())
        .expect("put something unreadable there");

    let error = repo
        .read_virtual_branches()
        .expect_err("an unreadable file is not an empty virtual branches");
    assert!(
        format!("{error}").contains("reading"),
        "the message has to say which file: {error}"
    );

    // The assignments file follows the same rule.
    std::fs::create_dir(repo.virtual_branches_assignments_path()).expect("same for assignments");
    assert!(repo.read_virtual_branches_assignments().is_err());
}

#[test]
fn an_invalid_virtual_branches_is_refused_and_the_file_on_disk_is_untouched() {
    let (_temp, repo, _) = repo_with_two_branches();
    let before = repo.read_virtual_branches().expect("read");

    // A loop between two branches: the state renders, but no ordering of it
    // can be applied, so it must never reach the disk.
    let cyclic = VirtualBranchesState::new("main").with_branches(vec![
        VirtualBranch::new("a").with_parent("b"),
        VirtualBranch::new("b").with_parent("a"),
    ]);
    assert!(
        repo.write_virtual_branches(&cyclic).is_err(),
        "a state that cannot be ordered has to be refused"
    );

    assert_eq!(
        repo.read_virtual_branches().expect("read"),
        before,
        "the file on disk still has to describe what Git actually has"
    );
}

#[test]
fn assignments_survive_a_write_and_a_read() {
    // The whole reason the file exists: a drag made today has to still be
    // there after a restart, on the branch the user picked.
    let (_temp, repo, _) = repo_with_two_branches();
    let mut index = AssignmentIndex::new();
    index.set(Path::new("src/a.rs"), Some("api".to_string()));
    index.set(Path::new("src/b.rs"), None);
    index.set_hunk(
        Path::new("src/c.rs"),
        gitcomet_core::virtual_branches::HunkFingerprint::of(&["NEW\n"], 1),
        Some("ui".to_string()),
    );

    repo.write_virtual_branches_assignments(&index)
        .expect("write");

    assert_eq!(
        repo.read_virtual_branches_assignments().expect("read"),
        index
    );
}

#[test]
fn an_unreadable_assignments_file_reads_as_empty() {
    // Same reasoning as the virtual branches file: a corrupt file must not make
    // every changed file in the repository unusable.
    let (_temp, repo, _) = repo_with_two_branches();
    std::fs::write(repo.virtual_branches_assignments_path(), b"not json at all")
        .expect("corrupt it");
    assert!(
        repo.read_virtual_branches_assignments()
            .expect("read")
            .is_empty()
    );
}

#[test]
fn a_virtual_branch_tracks_nothing_until_it_is_pushed() {
    // A branch that claims an upstream it was never pushed to shows as
    // permanently behind one, and the first push would try to reconcile
    // against a ref that does not exist.
    let (_temp, repo, _) = repo_with_two_branches();
    let root = repo.spec().workdir.clone();
    let target = repo.read_virtual_branches().expect("read").target;

    repo.create_virtual_branch("feature", &target)
        .expect("create");

    assert!(git_succeeds(
        &root,
        &["rev-parse", "--verify", "refs/heads/feature"]
    ));
    assert!(
        !git_succeeds(&root, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]),
        "a branch nobody pushed has to track nothing"
    );
}

#[test]
fn creating_a_virtual_branch_on_a_base_that_does_not_exist_is_refused() {
    // A typo in a stacked branch's base would otherwise create a branch that GitComet
    // believes in and that nothing can be built on.
    let (_temp, repo, _) = repo_with_two_branches();
    let root = repo.spec().workdir.clone();

    assert!(
        repo.create_virtual_branch("feature", "no-such-branch")
            .is_err(),
        "a base that does not name a commit cannot be built on"
    );
    assert!(
        !git_succeeds(&root, &["rev-parse", "--verify", "refs/heads/feature"]),
        "a refused creation must not leave the ref behind"
    );
}

// ── Pushing ─────────────────────────────────────────────────

/// A repository with a bare repository standing in for a remote.
fn repo_with_a_remote() -> (tempfile::TempDir, tempfile::TempDir, GixRepo) {
    let remote = tempfile::tempdir().expect("tempdir");
    git(remote.path(), &["init", "--bare"]);

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    git(root, &["init"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Every assertion here is about exact bytes: a system-wide
    // `core.autocrlf` would rewrite the checkout to CRLF.
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("a.txt"), "x\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);

    let default_branch = git(root, &["symbolic-ref", "--short", "HEAD"]);
    git(root, &["branch", "api"]);
    let remote_path = remote.path().to_string_lossy().into_owned();
    git(root, &["remote", "add", "origin", &remote_path]);

    let state =
        VirtualBranchesState::new(&default_branch).with_branches(vec![VirtualBranch::new("api")]);
    let repo = open_repo(root);
    repo.write_virtual_branches(&state)
        .expect("write virtual branches");
    (temp, remote, repo)
}

#[test]
fn pushing_a_virtual_branch_publishes_that_branch_and_not_head() {
    // The user is usually sitting on the target while they work in a
    // virtual branches, so pushing `HEAD` would publish the target and leave the
    // branch they clicked unpushed.
    let (temp, remote, repo) = repo_with_a_remote();
    let root = temp.path();

    repo.push_virtual_branch("api").expect("push");

    assert_eq!(
        git(remote.path(), &["rev-parse", "refs/heads/api"]),
        git(root, &["rev-parse", "refs/heads/api"]),
        "the remote has to have the branch that was pushed"
    );
    let head = git(root, &["symbolic-ref", "--short", "HEAD"]);
    assert!(
        !git_succeeds(
            remote.path(),
            &["rev-parse", "--verify", &format!("refs/heads/{head}")]
        ),
        "and not whichever branch the working directory happens to be on"
    );
}

#[test]
fn the_first_push_sets_the_upstream_and_later_ones_do_not_fail() {
    let (temp, _remote, repo) = repo_with_a_remote();
    let root = temp.path();

    repo.push_virtual_branch("api").expect("first push");
    assert_eq!(
        git(root, &["rev-parse", "--abbrev-ref", "api@{upstream}"]),
        "origin/api",
        "a branch that was never pushed has no upstream to compare against"
    );
    repo.push_virtual_branch("api").expect("second push");
}

#[test]
fn pushing_with_no_remote_says_so_rather_than_reporting_a_git_failure() {
    let (_temp, repo, _) = repo_with_two_branches();
    let error = repo.push_virtual_branch("api").expect_err("no remote");
    assert!(
        format!("{error}").contains("no remote"),
        "the message has to name the reason: {error}"
    );
}

#[test]
fn a_pull_request_targets_the_branch_the_virtual_branch_is_stacked_on() {
    // Not the remote default: a branch stacked on another virtual branch
    // belongs in a pull request against *that* branch, or the reviewer sees
    // the lower branch's work as part of this one.
    let (_temp, repo, _) = repo_with_two_branches();
    let mut state = repo.read_virtual_branches().expect("read");
    let target = state.target.clone();
    state
        .set_parent("ui", Some("api"))
        .expect("stacked branch ui on api");

    assert_eq!(
        repo.virtual_branch_push_target("api", &state).as_deref(),
        Some(target.as_str()),
        "an independent branch is reviewed against the virtual branches target"
    );
    assert_eq!(
        repo.virtual_branch_push_target("ui", &state).as_deref(),
        Some("api"),
        "a stacked branch is reviewed against the branch under it"
    );
    assert_eq!(
        repo.virtual_branch_push_target("nowhere", &state),
        None,
        "a branch the virtual branches does not have has no target"
    );
}

// ── The working directory ────────────────────────────────────

#[test]
fn entering_a_virtual_branches_that_already_exists_does_not_rebuild_it() {
    // The tab is opened and closed constantly. Rebuilding on every entry
    // would rewrite the branch each time and make the working directory
    // jump for no reason.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    repo.enter_virtual_branches().expect("first enter");
    let tip = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    repo.enter_virtual_branches().expect("second enter");

    assert_eq!(
        git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]),
        tip,
        "entering again must not move the virtual-branches branch"
    );
}

#[test]
fn leaving_onto_a_branch_that_no_longer_exists_is_refused() {
    // The branch the user was on was deleted or renamed while they were in
    // the virtual branches. Falling back to a guess would move them onto a branch
    // they never had checked out.
    let (temp, repo, default_branch) = repo_with_two_branches();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    git(root, &["branch", "-D", &default_branch]);

    let result = repo.leave_virtual_branches(&default_branch);
    assert!(
        result.is_err(),
        "there is nowhere to go back to, and guessing is worse than failing"
    );
    assert_eq!(
        git(root, &["symbolic-ref", "--short", "HEAD"]),
        VIRTUAL_BRANCHES_BRANCH,
        "and the working directory has to stay where it is"
    );
}

#[test]
fn only_the_virtual_branches_branch_counts_as_being_in_the_virtual_branches() {
    let (temp, repo, default_branch) = repo_with_two_branches();
    let _root = temp.path();

    assert!(
        !repo.virtual_branches_is_checked_out(),
        "the working directory starts on the target"
    );
    repo.enter_virtual_branches().expect("enter");
    assert!(
        repo.virtual_branches_is_checked_out(),
        "and is on the virtual-branches branch afterwards"
    );
    repo.leave_virtual_branches(&default_branch).expect("leave");
    assert!(!repo.virtual_branches_is_checked_out());
}

#[test]
fn the_virtual_branches_branch_never_appears_as_a_user_branch() {
    // Criterion 14 is carried by the name: `gitcomet/*` is GitComet's own
    // namespace and the branch list skips it, so no picker can offer to
    // check it out, rename it, or delete it. The virtual branches are
    // ordinary Git refs and *do* appear — hiding them would be wrong, they
    // are the user's work.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    repo.update_virtual_branches_branch(&["api", "ui"])
        .expect("build");
    assert!(git_succeeds(
        root,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{VIRTUAL_BRANCHES_BRANCH}")
        ]
    ));

    let branches: Vec<String> = repo
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .expect("list branches")
        .into_iter()
        .map(|branch| branch.name)
        .collect();
    assert!(
        !branches.iter().any(|name| is_virtual_branches_branch(name)),
        "the virtual-branches branch is bookkeeping, not a branch the user works on: {branches:?}"
    );
    assert!(
        branches.iter().any(|name| name == "api"),
        "the virtual branches are ordinary refs and belong in the list: {branches:?}"
    );
}

#[test]
fn committing_while_the_working_directory_is_in_the_virtual_branches_is_refused() {
    // The other half of criterion 14, and the half a hidden branch does not
    // cover: a plain commit goes to HEAD, and entering the virtual branches is
    // exactly what puts HEAD there. A commit here would be thrown away at
    // the next apply, after the log had shown it to the user as theirs.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    std::fs::write(root.join("shared.txt"), "edited\na2\na3\n").expect("edit");
    git(root, &["add", "-A"]);

    let error = repo
        .commit("a message")
        .expect_err("the virtual-branches branch is not commit-able");
    assert!(
        format!("{error}").contains("Virtual branches tab"),
        "the message has to say where to commit instead: {error}"
    );

    assert_eq!(
        git(root, &["symbolic-ref", "--short", "HEAD"]),
        VIRTUAL_BRANCHES_BRANCH,
        "and nothing may have been committed or moved"
    );
    assert_eq!(
        git(root, &["diff", "--cached", "--name-only"]),
        "shared.txt",
        "the change is still staged: a refused commit must leave the \
         working tree exactly as it was"
    );
    assert_eq!(read(root, "shared.txt"), "edited\na2\na3\n");
}

#[test]
fn amending_a_commit_on_the_virtual_branches_branch_is_refused_too() {
    let (temp, repo, _) = repo_with_two_branches();
    let _root = temp.path();
    repo.enter_virtual_branches().expect("enter");

    let error = repo
        .commit_amend("a message")
        .expect_err("an amend here rewrites a commit the next rebuild discards");
    assert!(
        format!("{error}").contains("gitcomet/virtual"),
        "the refusal has to name the branch it is about: {error}"
    );
}

#[test]
fn pushing_from_inside_the_virtual_branches_would_publish_it_and_is_refused() {
    // The ordinary push publishes whatever HEAD is, and entering the
    // virtual branches is what puts HEAD there. Left alone it would create
    // `gitcomet/virtual` on the user's remote — visible in their fork and
    // in every pull request's branch picker — and set it as an upstream, so
    // it would follow them to every later branch switch.
    let (temp, remote, repo) = repo_with_a_remote();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");

    let error = repo
        .push()
        .expect_err("the virtual-branches branch is not the user's to publish");
    let message = format!("{error}");
    assert!(
        message.contains("gitcomet/virtual") && message.contains("Virtual branches tab"),
        "the message has to name the branch and say what to do instead: {message}"
    );
    assert!(
        !git_succeeds(
            remote.path(),
            &["rev-parse", "--verify", "refs/heads/gitcomet/virtual"]
        ),
        "nothing may have reached the remote"
    );
    assert!(
        !git_succeeds(root, &["config", "--get", "branch.gitcomet/virtual.remote"]),
        "and no upstream may have been configured"
    );
}

#[test]
fn the_named_branch_push_still_works_from_inside_the_virtual_branches() {
    // The refusal above must not take the Virtual branches tab's own push with it:
    // that one names the ref, so it is exactly the action the user wants
    // while they are in here.
    let (temp, remote, repo) = repo_with_a_remote();
    let _root = temp.path();
    repo.enter_virtual_branches().expect("enter");

    repo.push_virtual_branch("api").expect("push the branch");

    assert!(git_succeeds(
        remote.path(),
        &["rev-parse", "--verify", "refs/heads/api"]
    ));
    assert!(
        !git_succeeds(
            remote.path(),
            &["rev-parse", "--verify", "refs/heads/gitcomet/virtual"]
        ),
        "pushing a branch must not drag HEAD along with it"
    );
}

/// A repository with the working directory inside the virtual branches, and a commit
/// on `api` to point history operations at.
fn repo_inside_the_virtual_branches() -> (tempfile::TempDir, GixRepo, CommitId) {
    let (temp, repo, _) = repo_with_two_branches();
    let _root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    let api_tip = repo
        .resolve_revision("api")
        .expect("resolve")
        .expect("api exists");
    (temp, repo, api_tip)
}

/// Assert that an operation was refused for the right reason.
///
/// Every one of these is the same bug: an action that applies to **HEAD**,
/// run while the virtual branches has put HEAD on `gitcomet/virtual`. Each would
/// create or rewrite a commit on GitComet's bookkeeping branch, where the
/// next rebuild discards it. Listing them one by one is the point — a guard
/// that covers six operations and not the seventh is indistinguishable from
/// one that covers none.
fn assert_refused(result: Result<CommandOutput>, action: &str) {
    let error = result.expect_err("the virtual-branches branch is not the user's to rewrite");
    let message = format!("{error}");
    assert!(
        message.contains("gitcomet/virtual"),
        "{action} has to say which branch it is about: {message}"
    );
    assert!(
        message.contains("Virtual branches tab"),
        "{action} has to say what to do instead: {message}"
    );
}

#[test]
fn a_cherry_pick_inside_the_virtual_branches_is_refused() {
    let (temp, repo, tip) = repo_inside_the_virtual_branches();
    let root = temp.path();
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(
        repo.cherry_pick_with_output(&tip, true, None),
        "a cherry-pick",
    );

    assert_eq!(
        git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]),
        before,
        "a refused pick must not move the branch"
    );
}

#[test]
fn a_revert_inside_the_virtual_branches_is_refused() {
    let (temp, repo, tip) = repo_inside_the_virtual_branches();
    let root = temp.path();
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(repo.revert_with_output(&tip, true, None), "a revert");

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn a_reset_inside_the_virtual_branches_is_refused() {
    // A reset here would throw away the merge history the branch exists to hold,
    // and the next rebuild would put it straight back.
    let (temp, repo, tip) = repo_inside_the_virtual_branches();
    let root = temp.path();
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(
        repo.reset_with_output(tip.as_ref(), ResetMode::Hard),
        "a reset",
    );

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn a_squash_inside_the_virtual_branches_is_refused() {
    let (temp, repo, tip) = repo_inside_the_virtual_branches();
    let root = temp.path();
    let head = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(
        repo.squash_commits_with_output(&tip, &tip, "squashed"),
        "a squash",
    );

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), head);
}

#[test]
fn a_rebase_inside_the_virtual_branches_is_refused() {
    // Rebasing the virtual-branches branch would rewrite the merge commits that say
    // which branch contributed what — the one thing the branch's history is for.
    let (temp, repo, _) = repo_inside_the_virtual_branches();
    let root = temp.path();
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(repo.rebase_with_output("api"), "a rebase");

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn a_merge_inside_the_virtual_branches_is_refused() {
    let (temp, repo, _) = repo_inside_the_virtual_branches();
    let root = temp.path();
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(repo.merge_ref_with_output("api"), "a merge");

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn a_pull_inside_the_virtual_branches_is_refused() {
    // A pull merges into HEAD, so here it would leave a merge commit that the
    // next rebuild discards. Moving the base forward is SetTarget, which is the
    // operation that means it here.
    let (temp, _remote, repo) = repo_with_a_remote();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(repo.pull(PullMode::Merge), "a pull");

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn pulling_a_named_branch_inside_the_virtual_branches_is_refused() {
    // Pulling a named remote branch is a second entry point that also merges
    // into HEAD, so the plain-pull guard does not cover it.
    let (temp, _remote, repo) = repo_with_a_remote();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(
        repo.pull_branch_with_output("origin", "main"),
        "a named pull",
    );

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn a_force_push_inside_the_virtual_branches_is_refused() {
    // A force push publishes HEAD just as an ordinary push does, and with
    // `--force-with-lease` it would overwrite what the remote already has
    // rather than add to it — so the ordinary-push guard is not enough.
    let (temp, _remote, repo) = repo_with_a_remote();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);

    assert_refused(repo.push_force_with_output(), "a force push");

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn a_force_push_with_a_lease_inside_the_virtual_branches_is_refused() {
    // The lease form is a separate entry point with its own command, so
    // covering only the plain force push would leave this path open.
    let (temp, _remote, repo) = repo_with_a_remote();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    let before = git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]);
    let head = repo
        .resolve_revision(VIRTUAL_BRANCHES_BRANCH)
        .expect("resolve")
        .expect("the virtual-branches branch exists");

    assert_refused(
        repo.push_force_with_lease_with_output(&ForcePushLease {
            remote: "origin".into(),
            branch: "api".into(),
            expected: head.clone(),
            local_branch: VIRTUAL_BRANCHES_BRANCH.into(),
            local_head: head,
        }),
        "a force push with a lease",
    );

    assert_eq!(git(root, &["rev-parse", VIRTUAL_BRANCHES_BRANCH]), before);
}

#[test]
fn committing_on_an_ordinary_branch_still_works() {
    // The guard is about the virtual-branches branch, not about commits: refusing
    // here would break the workflow the Virtual branches view sits next to.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    // An edit that actually changes the file: the fixture already contains
    // `mine.txt`, and rewriting identical bytes leaves git with nothing to
    // commit.
    std::fs::write(root.join("mine.txt"), "y\nordinary work\n").expect("edit");
    git(root, &["add", "-A"]);

    repo.commit("ordinary work").expect("commit");

    assert_eq!(
        git(root, &["log", "-1", "--format=%s"]),
        "ordinary work",
        "the user must still be able to commit normally"
    );
}

#[test]
fn a_rebuild_that_changes_nothing_leaves_the_files_alone() {
    // Syncing the working directory to the tree it already has would still
    // be a `read-tree -u`, which can refuse on a file the user has edited.
    let (temp, repo, _) = repo_with_two_branches();
    let root = temp.path();
    repo.enter_virtual_branches().expect("enter");
    let tip = repo
        .resolve_revision(VIRTUAL_BRANCHES_BRANCH)
        .expect("resolve")
        .expect("tip");

    std::fs::write(root.join("shared.txt"), "mine\na2\na3\n").expect("edit");
    repo.sync_virtual_branches_workdir_impl(&tip, &tip)
        .expect("sync onto itself");

    assert_eq!(
        read(root, "shared.txt"),
        "mine\na2\na3\n",
        "an edit to a file nothing changed has to survive"
    );
}
