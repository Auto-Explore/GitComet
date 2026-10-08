//! Generic building blocks for clients preparing several related revisions.
use super::GixRepo;
use crate::util::{
    git_workdir_cmd_for, run_git_capture, run_git_with_output, validate_hex_commit_id,
    validate_ref_like_arg,
};
use gitcomet_core::{
    domain::CommitId,
    error::{Error, ErrorKind},
    services::{
        BranchUpdate, CommandOutput, GitBackend, RebaseRange, RefUpdate, RemoteRefUpdate, Result,
    },
};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

fn failed(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Backend(message.into()))
}
fn valid_updates(updates: &[RefUpdate]) -> Result<String> {
    if updates.is_empty() {
        return Err(failed("No refs were selected"));
    }
    let mut refs = BTreeSet::new();
    let mut input = String::new();
    for u in updates {
        validate_ref_like_arg(&u.reference, "reference")?;
        if !u.reference.starts_with("refs/")
            || !refs.insert(&u.reference)
            || u.reference.chars().any(char::is_whitespace)
        {
            return Err(failed("Invalid or duplicate transaction ref"));
        }
        validate_hex_commit_id(&u.new)?;
        if let Some(old) = &u.expected {
            validate_hex_commit_id(old)?;
        }
        let zero = "0".repeat(u.new.as_ref().len());
        input.push_str(&format!(
            "update {} {} {}\n",
            u.reference,
            u.new,
            u.expected.as_ref().map_or(zero.as_str(), AsRef::as_ref)
        ));
    }
    Ok(input)
}
/// Owns the prepared ref locks. Closing stdin aborts Git's transaction;
/// every early return also reaps the process before releasing this guard.
struct RefTransaction {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    // Git acknowledges commit/abort on stdout. Keep the reader alive until
    // the child exits so its acknowledgement cannot hit a closed pipe.
    stdout: Option<BufReader<ChildStdout>>,
}

impl RefTransaction {
    fn prepare(mut command: Command, input: &str) -> Result<Self> {
        command
            .args(["update-ref", "--no-deref", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command.spawn().map_err(|e| failed(e.to_string()))?;
        let mut transaction = Self {
            child: Some(child),
            stdin: None,
            stdout: None,
        };
        let child = transaction
            .child
            .as_mut()
            .expect("transaction owns its child");
        transaction.stdin = child.stdin.take();
        transaction.stdout = Some(BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| failed("Missing transaction output"))?,
        ));
        let stdin = transaction
            .stdin
            .as_mut()
            .ok_or_else(|| failed("Missing transaction input"))?;
        stdin
            .write_all(format!("start\n{input}prepare\n").as_bytes())
            .map_err(|e| failed(e.to_string()))?;
        stdin.flush().map_err(|e| failed(e.to_string()))?;
        for expected in ["start: ok", "prepare: ok"] {
            let mut line = String::new();
            transaction
                .stdout
                .as_mut()
                .expect("transaction owns stdout")
                .read_line(&mut line)
                .map_err(|e| failed(e.to_string()))?;
            if line.trim() != expected {
                transaction.stdin.take();
                let output = transaction
                    .child
                    .take()
                    .expect("transaction owns its child")
                    .wait_with_output()
                    .map_err(|e| failed(e.to_string()))?;
                return Err(failed(String::from_utf8_lossy(&output.stderr)));
            }
        }
        Ok(transaction)
    }

    fn commit(mut self) -> Result<()> {
        self.stdin
            .as_mut()
            .expect("prepared transaction has stdin")
            .write_all(b"commit\n")
            .map_err(|e| failed(e.to_string()))?;
        self.stdin.take();
        let output = self
            .child
            .take()
            .expect("transaction owns its child")
            .wait_with_output()
            .map_err(|e| failed(e.to_string()))?;
        if !output.status.success() {
            return Err(failed(format!(
                "Ref transaction needs recovery: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(())
    }
}

impl Drop for RefTransaction {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

impl GixRepo {
    pub(super) fn rebase_range_impl(&self, range: &RebaseRange) -> Result<CommandOutput> {
        for id in [&range.onto, &range.upstream, &range.expected_head] {
            validate_hex_commit_id(id)?;
        }
        if self.head_commit_id_impl()?.as_ref() != Some(&range.expected_head) {
            return Err(failed("The prepared branch head changed"));
        }
        if self.operation_state_on_disk()? {
            return Err(failed("Finish the current Git operation first"));
        }
        let mut status = self.git_workdir_cmd();
        status.args(["status", "--porcelain", "--untracked-files=all"]);
        if !run_git_capture(status, "git status")?.is_empty() {
            return Err(failed("The rebase worktree must be clean"));
        }
        let mut ancestry = self.git_workdir_cmd();
        ancestry.args([
            "merge-base",
            "--is-ancestor",
            range.upstream.as_ref(),
            range.expected_head.as_ref(),
        ]);
        if !ancestry
            .status()
            .map_err(|e| failed(e.to_string()))?
            .success()
        {
            return Err(failed(
                "The replay boundary is not an ancestor of the prepared head",
            ));
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.args([
            "-c",
            "rebase.updateRefs=false",
            "-c",
            "rebase.autoStash=false",
            "rebase",
            "--no-autostash",
            "--no-update-refs",
        ]);
        if range.preserve_merges {
            cmd.arg("--rebase-merges=no-rebase-cousins");
        }
        cmd.arg("--onto")
            .arg(range.onto.as_ref())
            .arg(range.upstream.as_ref());
        run_git_with_output(cmd, "git rebase --onto (captured revisions)")
    }
    pub(super) fn update_refs_impl(&self, updates: &[RefUpdate]) -> Result<CommandOutput> {
        let input = valid_updates(updates)?;
        RefTransaction::prepare(self.git_workdir_cmd(), &input)?.commit()?;
        Ok(CommandOutput::empty_success("git update-ref transaction"))
    }
    pub(super) fn apply_branch_updates_impl(
        &self,
        updates: &[BranchUpdate],
        recover: bool,
    ) -> Result<CommandOutput> {
        let mut refs: Vec<_> = updates.iter().map(|u| u.update.clone()).collect();
        valid_updates(&refs)?;
        let common = self
            .common_dir_impl()
            .canonicalize()
            .map_err(|e| failed(e.to_string()))?;
        let worktrees = self.list_worktrees_impl()?;
        let mut paths = BTreeSet::new();
        let mut syncs = Vec::new();
        for (index, u) in updates.iter().enumerate() {
            if !u.update.reference.starts_with("refs/heads/") {
                return Err(failed("Only branches can synchronize worktrees"));
            }
            let declared: BTreeSet<_> = u
                .worktrees
                .iter()
                .map(|p| p.canonicalize().map_err(|e| failed(e.to_string())))
                .collect::<Result<_>>()?;
            let actual: BTreeSet<_> = worktrees
                .iter()
                .filter(|w| w.branch.as_deref() == u.update.reference.strip_prefix("refs/heads/"))
                .map(|w| w.path.canonicalize().map_err(|e| failed(e.to_string())))
                .collect::<Result<_>>()?;
            if declared != actual {
                return Err(failed(
                    "Checked-out worktrees changed since preparation. Refresh the transaction before applying.",
                ));
            }
            if recover {
                let mut current = self.git_workdir_cmd();
                current.args(["rev-parse", "--verify", &u.update.reference]);
                if let Ok(head) = run_git_capture(current, "git captured branch tip")
                    && head.trim() == u.update.new.as_ref()
                {
                    refs[index].expected = Some(u.update.new.clone());
                }
            }
            for path in &u.worktrees {
                if !paths.insert(path.canonicalize().map_err(|e| failed(e.to_string()))?) {
                    return Err(failed("A worktree occurs more than once"));
                }
                let repo = crate::GixBackend.open(path)?;
                let mut common_cmd = git_workdir_cmd_for(path);
                common_cmd.args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
                let dir = run_git_capture(common_cmd, "git common directory")?;
                if Path::new(dir.trim())
                    .canonicalize()
                    .map_err(|e| failed(e.to_string()))?
                    != common
                {
                    return Err(failed("The worktree belongs to another repository"));
                }
                let mut branch = git_workdir_cmd_for(path);
                branch.args(["symbolic-ref", "HEAD"]);
                if run_git_capture(branch, "git symbolic-ref HEAD")?.trim() != u.update.reference
                    || repo.head_commit_id()? != refs[index].expected
                {
                    return Err(failed("A checked-out branch changed"));
                }
                let mut marker_cmd = git_workdir_cmd_for(path);
                marker_cmd.args([
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    "MERGE_HEAD",
                ]);
                let marker = run_git_capture(marker_cmd, "git merge marker")?;
                if repo.sequencer_state()? != gitcomet_core::services::SequencerState::None
                    || Path::new(marker.trim()).exists()
                {
                    return Err(failed("Finish the current Git operation before applying"));
                }
                let old = u
                    .update
                    .expected
                    .as_ref()
                    .ok_or_else(|| failed("A checked-out branch has no old tip"))?;
                let from = if recover {
                    let mut index_cmd = git_workdir_cmd_for(path);
                    index_cmd.arg("write-tree");
                    let index_tree = run_git_capture(index_cmd, "git recovery index tree")?;
                    let tree = |revision: &CommitId| -> Result<String> {
                        let mut cmd = git_workdir_cmd_for(path);
                        cmd.args(["rev-parse", &format!("{}^{{tree}}", revision)]);
                        run_git_capture(cmd, "git recovery commit tree")
                    };
                    let from = if index_tree == tree(&u.update.new)? {
                        u.update.new.clone()
                    } else if index_tree == tree(old)? {
                        old.clone()
                    } else {
                        return Err(failed(
                            "Recovery found unexpected staged edits; files were preserved",
                        ));
                    };
                    let mut tracked = git_workdir_cmd_for(path);
                    // Compare contents against the recorded index, not HEAD:
                    // interrupted recovery may have already prepared the index.
                    // Porcelain diff also refreshes stale stat data for restored
                    // files, which diff-files can report as unsaved edits.
                    tracked.args([
                        "-c",
                        "diff.autoRefreshIndex=true",
                        "diff",
                        "--quiet",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--ignore-submodules=none",
                    ]);
                    let mut untracked = git_workdir_cmd_for(path);
                    untracked.args(["ls-files", "--others", "--exclude-standard"]);
                    if !tracked
                        .status()
                        .map_err(|e| failed(e.to_string()))?
                        .success()
                        || !run_git_capture(untracked, "git recovery untracked files")?.is_empty()
                    {
                        return Err(failed("Recovery found unsaved files; files were preserved"));
                    }
                    from
                } else {
                    let mut status = git_workdir_cmd_for(path);
                    status.args(["status", "--porcelain", "--untracked-files=all"]);
                    if !run_git_capture(status, "git status")?.is_empty() {
                        return Err(failed(format!(
                            "Finish or save changes in {} before applying",
                            path.display()
                        )));
                    }
                    old.clone()
                };
                sync_tree(path, &from, &u.update.new, true)?;
                syncs.push((path.as_path(), from, u.update.new.clone()));
            }
        }
        let input = valid_updates(&refs)?;
        let transaction = RefTransaction::prepare(self.git_workdir_cmd(), &input)?;
        let mut synced: Vec<(&Path, &CommitId, &CommitId)> = Vec::new();
        for (path, old, new) in &syncs {
            if let Err(error) = sync_tree(path, old, new, false) {
                let mut recovery = Vec::new();
                for (path, old, new) in synced.iter().rev() {
                    if let Err(e) = sync_tree(path, new, old, false) {
                        recovery.push(e.to_string());
                    }
                }
                drop(transaction);
                return Err(failed(format!(
                    "{error}; worktree recovery: {}",
                    recovery.join("; ")
                )));
            }
            synced.push((path, old, new));
        }
        transaction.commit()?;
        Ok(CommandOutput::empty_success(
            "apply prepared branches and worktrees",
        ))
    }
    pub(super) fn push_refs_impl(
        &self,
        remote: &str,
        updates: &[RemoteRefUpdate],
        atomic: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote")?;
        if updates.is_empty() {
            return Err(failed("No branches were selected for publication"));
        }
        let mut branches = BTreeSet::new();
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["push", "--porcelain"]);
        if atomic {
            cmd.arg("--atomic");
        }
        for u in updates {
            validate_ref_like_arg(&u.branch, "branch")?;
            if !branches.insert(&u.branch) {
                return Err(failed("Duplicate publication branch"));
            }
            validate_hex_commit_id(&u.expected)?;
            validate_hex_commit_id(&u.new)?;
            cmd.arg(format!(
                "--force-with-lease=refs/heads/{}:{}",
                u.branch, u.expected
            ));
        }
        cmd.arg("--").arg(remote);
        for u in updates {
            cmd.arg(format!("{}:refs/heads/{}", u.new, u.branch));
        }
        run_git_with_output(cmd, "git push captured revisions with exact leases")
    }
}
fn sync_tree(path: &Path, old: &CommitId, new: &CommitId, dry: bool) -> Result<CommandOutput> {
    let mut cmd = git_workdir_cmd_for(path);
    cmd.args(["read-tree", "-u", "-m"]);
    if dry {
        cmd.arg("--dry-run");
    }
    cmd.arg(old.as_ref()).arg(new.as_ref());
    run_git_with_output(cmd, "git read-tree guarded worktree synchronization")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    fn git(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("file"), "base\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "base"]);
        dir
    }
    fn id(s: String) -> CommitId {
        CommitId(s.into())
    }
    #[test]
    fn dropping_a_prepared_transaction_aborts_and_releases_ref_locks() {
        let d = fixture();
        let old = id(git(d.path(), &["rev-parse", "HEAD"]));
        std::fs::write(d.path().join("file"), "next\n").unwrap();
        git(d.path(), &["commit", "-am", "next"]);
        let new = id(git(d.path(), &["rev-parse", "HEAD"]));
        git(d.path(), &["update-ref", "refs/heads/main", old.as_ref()]);
        let input = valid_updates(&[RefUpdate {
            reference: "refs/heads/main".into(),
            expected: Some(old.clone()),
            new: new.clone(),
        }])
        .unwrap();
        let transaction = RefTransaction::prepare(git_workdir_cmd_for(d.path()), &input).unwrap();
        assert_eq!(
            git(d.path(), &["rev-parse", "refs/heads/main"]),
            old.as_ref()
        );
        drop(transaction);
        git(
            d.path(),
            &["update-ref", "refs/heads/main", new.as_ref(), old.as_ref()],
        );
        assert_eq!(
            git(d.path(), &["rev-parse", "refs/heads/main"]),
            new.as_ref()
        );
    }

    #[test]
    fn explicit_range_does_not_move_other_refs_when_update_refs_is_configured() {
        let d = fixture();
        let old = id(git(d.path(), &["rev-parse", "HEAD"]));
        git(d.path(), &["checkout", "-b", "parent"]);
        std::fs::write(d.path().join("parent"), "parent").unwrap();
        git(d.path(), &["add", "."]);
        git(d.path(), &["commit", "-m", "parent"]);
        let cut = id(git(d.path(), &["rev-parse", "HEAD"]));
        git(d.path(), &["checkout", "-b", "child"]);
        std::fs::write(d.path().join("child"), "child").unwrap();
        git(d.path(), &["add", "."]);
        git(d.path(), &["commit", "-m", "child"]);
        let head = id(git(d.path(), &["rev-parse", "HEAD"]));
        git(d.path(), &["branch", "other"]);
        git(d.path(), &["checkout", "main"]);
        std::fs::write(d.path().join("trunk"), "trunk").unwrap();
        git(d.path(), &["add", "."]);
        git(d.path(), &["commit", "-m", "trunk"]);
        let onto = id(git(d.path(), &["rev-parse", "HEAD"]));
        let scratch = d.path().join("scratch");
        git(
            d.path(),
            &[
                "worktree",
                "add",
                "--detach",
                scratch.to_str().unwrap(),
                head.as_ref(),
            ],
        );
        git(d.path(), &["config", "rebase.updateRefs", "true"]);
        git(d.path(), &["config", "rebase.autoStash", "true"]);
        let repo = crate::GixBackend.open(&scratch).unwrap();
        repo.rebase_range_with_output(&RebaseRange {
            onto: onto.clone(),
            upstream: cut,
            expected_head: head.clone(),
            preserve_merges: true,
        })
        .unwrap();
        assert_eq!(git(d.path(), &["rev-parse", "child"]), head.as_ref());
        assert_eq!(git(d.path(), &["rev-parse", "other"]), head.as_ref());
        assert!(!scratch.join("parent").exists());
        assert!(scratch.join("child").exists());
        assert_ne!(old, onto);
    }
    #[test]
    fn stale_ref_aborts_all_updates() {
        let d = fixture();
        let repo = crate::GixBackend.open(d.path()).unwrap();
        let head = id(git(d.path(), &["rev-parse", "HEAD"]));
        let updates = vec![
            RefUpdate {
                reference: "refs/heads/created".into(),
                expected: None,
                new: head.clone(),
            },
            RefUpdate {
                reference: "refs/heads/main".into(),
                expected: Some(id("a".repeat(40))),
                new: head,
            },
        ];
        assert!(repo.update_refs_with_output(&updates).is_err());
        assert!(
            !Command::new("git")
                .current_dir(d.path())
                .args(["show-ref", "--verify", "refs/heads/created"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    #[test]
    fn checked_out_branch_is_synchronized_and_dirty_work_is_preserved() {
        let d = fixture();
        let old = id(git(d.path(), &["rev-parse", "HEAD"]));
        git(d.path(), &["checkout", "-b", "next"]);
        std::fs::write(d.path().join("file"), "new\n").unwrap();
        git(d.path(), &["commit", "-am", "new"]);
        let new = id(git(d.path(), &["rev-parse", "HEAD"]));
        git(d.path(), &["checkout", "main"]);
        let repo = crate::GixBackend.open(d.path()).unwrap();
        let update = BranchUpdate {
            update: RefUpdate {
                reference: "refs/heads/main".into(),
                expected: Some(old),
                new: new.clone(),
            },
            worktrees: vec![d.path().to_path_buf()],
        };
        std::fs::write(d.path().join("file"), "unsaved\n").unwrap();
        assert!(
            repo.apply_branch_updates_with_output(std::slice::from_ref(&update))
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(d.path().join("file")).unwrap(),
            "unsaved\n"
        );
        git(d.path(), &["restore", "file"]);
        repo.apply_branch_updates_with_output(&[update]).unwrap();
        assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), new.as_ref());
        assert!(git(d.path(), &["status", "--porcelain"]).is_empty());
    }
    #[test]
    fn atomic_publication_rejects_every_ref_if_one_lease_changed() {
        let d = fixture();
        let remote = tempfile::tempdir().unwrap();
        git(remote.path(), &["init", "--bare"]);
        git(
            d.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(d.path(), &["push", "origin", "main:one", "main:two"]);
        let old = id(git(d.path(), &["rev-parse", "HEAD"]));
        std::fs::write(d.path().join("file"), "new\n").unwrap();
        git(d.path(), &["commit", "-am", "new"]);
        let new = id(git(d.path(), &["rev-parse", "HEAD"]));
        let repo = crate::GixBackend.open(d.path()).unwrap();
        let updates = vec![
            RemoteRefUpdate {
                branch: "one".into(),
                expected: old.clone(),
                new: new.clone(),
            },
            RemoteRefUpdate {
                branch: "two".into(),
                expected: id("a".repeat(40)),
                new,
            },
        ];
        assert!(
            repo.push_refs_with_lease_with_output("origin", &updates, true)
                .is_err()
        );
        assert_eq!(git(remote.path(), &["rev-parse", "one"]), old.as_ref());
        assert_eq!(git(remote.path(), &["rev-parse", "two"]), old.as_ref());
    }
    #[test]
    fn recovery_accepts_only_recorded_trees_and_preserves_later_edits() {
        for (autocrlf, refs_committed) in [
            ("false", false),
            ("false", true),
            ("true", false),
            ("true", true),
        ] {
            let d = fixture();
            git(d.path(), &["config", "core.autocrlf", autocrlf]);
            git(d.path(), &["config", "diff.autoRefreshIndex", "false"]);
            let old = id(git(d.path(), &["rev-parse", "HEAD"]));
            git(d.path(), &["checkout", "-b", "prepared"]);
            std::fs::write(d.path().join("file"), "prepared\n").unwrap();
            git(d.path(), &["commit", "-am", "prepared"]);
            let new = id(git(d.path(), &["rev-parse", "HEAD"]));
            git(d.path(), &["checkout", "main"]);
            let update = BranchUpdate {
                update: RefUpdate {
                    reference: "refs/heads/main".into(),
                    expected: Some(old.clone()),
                    new: new.clone(),
                },
                worktrees: vec![d.path().into()],
            };
            if refs_committed {
                git(
                    d.path(),
                    &["update-ref", "refs/heads/main", new.as_ref(), old.as_ref()],
                );
            } else {
                git(
                    d.path(),
                    &["read-tree", "-u", "-m", old.as_ref(), new.as_ref()],
                );
            }
            let recorded_tree = git(d.path(), &["write-tree"]);
            let original_bytes = std::fs::read(d.path().join("file")).unwrap();
            let repo = crate::GixBackend.open(d.path()).unwrap();
            std::fs::write(d.path().join("file"), "later unsaved edit\n").unwrap();
            assert!(
                repo.recover_branch_updates_with_output(std::slice::from_ref(&update))
                    .is_err()
            );
            assert_eq!(
                std::fs::read_to_string(d.path().join("file")).unwrap(),
                "later unsaved edit\n"
            );
            assert_eq!(git(d.path(), &["write-tree"]), recorded_tree);
            assert_eq!(
                git(d.path(), &["rev-parse", "HEAD"]),
                if refs_committed {
                    new.as_ref()
                } else {
                    old.as_ref()
                }
            );
            // Restore the checkout's bytes, including CRLF when configured,
            // while keeping its cached stat information deterministically stale.
            std::fs::write(d.path().join("file"), original_bytes).unwrap();
            filetime::set_file_mtime(
                d.path().join("file"),
                filetime::FileTime::from_unix_time(
                    filetime::FileTime::now().unix_seconds() - 120,
                    0,
                ),
            )
            .unwrap();
            repo.recover_branch_updates_with_output(std::slice::from_ref(&update))
                .unwrap();
            assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), new.as_ref());
            assert!(git(d.path(), &["status", "--porcelain"]).is_empty());
            repo.recover_branch_updates_with_output(&[update]).unwrap();
        }
    }
    #[test]
    fn omitted_checked_out_worktree_rejects_the_whole_transaction() {
        let d = fixture();
        let repo = crate::GixBackend.open(d.path()).unwrap();
        let head = id(git(d.path(), &["rev-parse", "HEAD"]));
        let updates = [
            BranchUpdate {
                update: RefUpdate {
                    reference: "refs/heads/main".into(),
                    expected: Some(head.clone()),
                    new: head.clone(),
                },
                worktrees: Vec::new(),
            },
            BranchUpdate {
                update: RefUpdate {
                    reference: "refs/heads/created".into(),
                    expected: None,
                    new: head,
                },
                worktrees: Vec::new(),
            },
        ];
        assert!(repo.apply_branch_updates_with_output(&updates).is_err());
        assert!(
            !std::process::Command::new("git")
                .current_dir(d.path())
                .args(["show-ref", "--verify", "refs/heads/created"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    #[test]
    fn clean_pending_merge_blocks_rebase_and_branch_application() {
        let d = fixture();
        let head = id(git(d.path(), &["rev-parse", "HEAD"]));
        std::fs::write(d.path().join(".git/MERGE_HEAD"), format!("{head}\n")).unwrap();
        let repo = crate::GixBackend.open(d.path()).unwrap();
        assert!(
            repo.rebase_range_with_output(&RebaseRange {
                onto: head.clone(),
                upstream: head.clone(),
                expected_head: head.clone(),
                preserve_merges: false
            })
            .is_err()
        );
        assert!(
            repo.apply_branch_updates_with_output(&[BranchUpdate {
                update: RefUpdate {
                    reference: "refs/heads/main".into(),
                    expected: Some(head.clone()),
                    new: head
                },
                worktrees: vec![d.path().into()]
            }])
            .is_err()
        );
        assert!(d.path().join(".git/MERGE_HEAD").exists());
    }
    #[test]
    fn unsupported_atomic_push_reports_no_updates_and_allows_explicit_fallback() {
        let d = fixture();
        let remote = tempfile::tempdir().unwrap();
        git(remote.path(), &["init", "--bare"]);
        git(
            remote.path(),
            &["config", "receive.advertiseAtomic", "false"],
        );
        git(
            d.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(d.path(), &["push", "origin", "main:one"]);
        let old = id(git(d.path(), &["rev-parse", "HEAD"]));
        std::fs::write(d.path().join("file"), "new\n").unwrap();
        git(d.path(), &["commit", "-am", "new"]);
        let new = id(git(d.path(), &["rev-parse", "HEAD"]));
        let repo = crate::GixBackend.open(d.path()).unwrap();
        let updates = [RemoteRefUpdate {
            branch: "one".into(),
            expected: old.clone(),
            new: new.clone(),
        }];
        assert!(
            repo.push_refs_with_lease_with_output("origin", &updates, true)
                .unwrap_err()
                .to_string()
                .contains("does not support --atomic")
        );
        assert_eq!(git(remote.path(), &["rev-parse", "one"]), old.as_ref());
        repo.push_refs_with_lease_with_output("origin", &updates, false)
            .unwrap();
        assert_eq!(git(remote.path(), &["rev-parse", "one"]), new.as_ref());
    }
}
