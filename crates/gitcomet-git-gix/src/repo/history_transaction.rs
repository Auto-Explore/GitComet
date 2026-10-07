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
    process::Stdio,
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
impl GixRepo {
    pub(super) fn rebase_range_impl(&self, range: &RebaseRange) -> Result<CommandOutput> {
        for id in [&range.onto, &range.upstream, &range.expected_head] {
            validate_hex_commit_id(id)?;
        }
        if self.head_commit_id_impl()?.as_ref() != Some(&range.expected_head) {
            return Err(failed("The prepared branch head changed"));
        }
        if self.rebase_in_progress_impl()? {
            return Err(failed("Finish the current rebase first"));
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
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["update-ref", "--no-deref", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| failed(e.to_string()))?;
        child
            .stdin
            .take()
            .ok_or_else(|| failed("Missing ref transaction input"))?
            .write_all(input.as_bytes())
            .map_err(|e| failed(e.to_string()))?;
        let output = child
            .wait_with_output()
            .map_err(|e| failed(e.to_string()))?;
        if !output.status.success() {
            return Err(failed(String::from_utf8_lossy(&output.stderr)));
        }
        Ok(CommandOutput::empty_success("git update-ref transaction"))
    }
    pub(super) fn apply_branch_updates_impl(
        &self,
        updates: &[BranchUpdate],
    ) -> Result<CommandOutput> {
        let refs: Vec<_> = updates.iter().map(|u| u.update.clone()).collect();
        let input = valid_updates(&refs)?;
        let mut paths = BTreeSet::new();
        for u in updates {
            if !u.update.reference.starts_with("refs/heads/") {
                return Err(failed("Only branches can synchronize worktrees"));
            }
            for path in &u.worktrees {
                if !paths.insert(path) {
                    return Err(failed("A worktree occurs more than once"));
                }
                let repo = crate::GixBackend.open(path)?;
                let mut common = git_workdir_cmd_for(path);
                common.args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
                let dir = run_git_capture(common, "git common directory")?;
                if Path::new(dir.trim()).canonicalize().ok()
                    != self.common_dir_impl().canonicalize().ok()
                {
                    return Err(failed("The worktree belongs to another repository"));
                }
                let mut branch = git_workdir_cmd_for(path);
                branch.args(["symbolic-ref", "HEAD"]);
                if run_git_capture(branch, "git symbolic-ref HEAD")?.trim() != u.update.reference
                    || repo.head_commit_id()? != u.update.expected
                {
                    return Err(failed("A checked-out branch changed"));
                }
                let mut status = git_workdir_cmd_for(path);
                status.args(["status", "--porcelain", "--untracked-files=all"]);
                if !run_git_capture(status, "git status")?.is_empty()
                    || repo.rebase_in_progress()?
                {
                    return Err(failed(format!(
                        "Finish or save changes in {} before applying",
                        path.display()
                    )));
                }
                let old = u
                    .update
                    .expected
                    .as_ref()
                    .ok_or_else(|| failed("A checked-out branch has no old tip"))?;
                sync_tree(path, old, &u.update.new, true)?;
            }
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["update-ref", "--no-deref", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| failed(e.to_string()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| failed("Missing transaction input"))?;
        let mut stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| failed("Missing transaction output"))?,
        );
        // EOF aborts a prepared transaction. No ref moves before all worktrees succeed.
        stdin
            .write_all(format!("start\n{input}prepare\n").as_bytes())
            .map_err(|e| failed(e.to_string()))?;
        stdin.flush().map_err(|e| failed(e.to_string()))?;
        for expected in ["start: ok", "prepare: ok"] {
            let mut line = String::new();
            stdout
                .read_line(&mut line)
                .map_err(|e| failed(e.to_string()))?;
            if line.trim() != expected {
                drop(stdin);
                let output = child
                    .wait_with_output()
                    .map_err(|e| failed(e.to_string()))?;
                return Err(failed(String::from_utf8_lossy(&output.stderr)));
            }
        }
        let mut synced: Vec<(&Path, &CommitId, &CommitId)> = Vec::new();
        for u in updates {
            for path in &u.worktrees {
                let old = u
                    .update
                    .expected
                    .as_ref()
                    .expect("validated checked-out tip");
                if let Err(error) = sync_tree(path, old, &u.update.new, false) {
                    let mut recovery = Vec::new();
                    for (path, old, new) in synced.iter().rev() {
                        if let Err(e) = sync_tree(path, new, old, false) {
                            recovery.push(e.to_string());
                        }
                    }
                    let _ = stdin.write_all(b"abort\n");
                    drop(stdin);
                    let _ = child.wait();
                    return Err(failed(format!(
                        "{error}; worktree recovery: {}",
                        recovery.join("; ")
                    )));
                }
                synced.push((path.as_path(), old, &u.update.new));
            }
        }
        stdin
            .write_all(b"commit\n")
            .map_err(|e| failed(e.to_string()))?;
        drop(stdin);
        let output = child
            .wait_with_output()
            .map_err(|e| failed(e.to_string()))?;
        if !output.status.success() {
            return Err(failed(format!(
                "Branch application needs recovery: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
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
}
