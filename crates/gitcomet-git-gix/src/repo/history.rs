use super::GixRepo;
use crate::util::{run_git_capture, run_git_with_output, validate_ref_like_arg};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, InteractiveRebaseAction, InteractiveRebaseEntry, ResetMode, Result,
};
use std::fs;
use std::path::PathBuf;

/// Returns the HEAD commit id, or `None` when HEAD is unborn / empty.
pub(super) fn gix_head_id_or_none(repo: &gix::Repository) -> Result<Option<gix::ObjectId>> {
    let mut head = repo
        .head()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head: {e}"))))?;
    head.try_peel_to_id()
        .map(|id| id.map(|id| id.detach()))
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head peel: {e}"))))
}

impl GixRepo {
    pub(super) fn reset_with_output_impl(
        &self,
        target: &str,
        mode: ResetMode,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(target, "reset target")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("reset");
        let mode_flag = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        cmd.arg(mode_flag).arg(target);
        let label = format!("git reset {mode_flag} {target}");
        run_git_with_output(cmd, &label)
    }

    pub(super) fn rebase_with_output_impl(&self, onto: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(onto, "rebase target")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rebase").arg("--").arg(onto);
        run_git_with_output(cmd, &format!("git rebase {onto}"))
    }

    pub(super) fn rebase_continue_with_output_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rebase").arg("--continue");
        run_git_with_output(cmd, "git rebase --continue")
    }

    pub(super) fn rebase_abort_with_output_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rebase").arg("--abort");
        match run_git_with_output(cmd, "git rebase --abort") {
            Ok(output) => Ok(output),
            Err(rebase_error) => {
                // `git am` uses its own sequencer state. Falling back here allows a
                // single "abort in-progress operation" UI action to handle both rebase
                // and patch-apply flows.
                let mut am_cmd = self.git_workdir_cmd();
                am_cmd.arg("am").arg("--abort");
                match run_git_with_output(am_cmd, "git am --abort") {
                    Ok(output) => Ok(output),
                    Err(_) => Err(rebase_error),
                }
            }
        }
    }

    pub(super) fn merge_abort_with_output_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge").arg("--abort");
        run_git_with_output(cmd, "git merge --abort")
    }

    pub(super) fn rebase_in_progress_impl(&self) -> Result<bool> {
        let repo = self._repo.to_thread_local();
        Ok(matches!(
            repo.state(),
            Some(
                gix::state::InProgress::Rebase
                    | gix::state::InProgress::RebaseInteractive
                    | gix::state::InProgress::ApplyMailbox
                    | gix::state::InProgress::ApplyMailboxRebase
            )
        ))
    }

    pub(super) fn list_commits_for_interactive_rebase_impl(
        &self,
        base: &str,
    ) -> Result<Vec<InteractiveRebaseEntry>> {
        validate_ref_like_arg(base, "interactive rebase base")?;

        let range = format!("{base}..HEAD");
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["log", "--format=%H %s", "--reverse", &range]);
        let output = run_git_capture(cmd, &format!("git log {range}"))?;

        let entries = output
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let (sha, summary) = line.split_once(' ').unwrap_or((line, ""));
                InteractiveRebaseEntry {
                    action: InteractiveRebaseAction::Pick,
                    commit_id: sha.to_string(),
                    summary: summary.to_string(),
                    new_message: None,
                }
            })
            .collect();

        Ok(entries)
    }

    pub(super) fn interactive_rebase_with_output_impl(
        &self,
        base: &str,
        entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(base, "interactive rebase base")?;

        let scripts = RebaseScripts::create(entries, self.git_dir_path()).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "failed to create rebase scripts: {e}"
            )))
        })?;

        let mut cmd = self.git_workdir_cmd();
        cmd.env("GIT_SEQUENCE_EDITOR", &scripts.seq_editor_path);
        if let Some(ref msg_editor) = scripts.msg_editor_path {
            cmd.env("GIT_EDITOR", msg_editor);
            cmd.env("GITCOMET_MSGS_DIR", &scripts.msgs_dir);
            let repo = self._repo.to_thread_local();
            cmd.env("GITCOMET_GIT_DIR", repo.path());
        }
        cmd.env("GITCOMET_TODO_FILE", &scripts.todo_path);
        cmd.args(["rebase", "-i", "--", base]);

        let label = format!("git rebase -i {base}");
        run_git_with_output(cmd, &label)
    }

    fn git_dir_path(&self) -> PathBuf {
        self._repo.to_thread_local().path().to_owned()
    }

    pub(super) fn merge_commit_message_impl(&self) -> Result<Option<String>> {
        let repo = self._repo.to_thread_local();
        if repo.state() != Some(gix::state::InProgress::Merge) {
            return Ok(None);
        }

        let merge_msg_path = repo.path().join("MERGE_MSG");
        let contents = match std::fs::read_to_string(&merge_msg_path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::new(ErrorKind::Io(e.kind()))),
        };

        let mut lines: Vec<&str> = Vec::new();
        for line in contents.lines() {
            let line = line.trim_end();
            if line.trim_start().starts_with('#') {
                continue;
            }
            lines.push(line);
        }

        let Some(start) = lines.iter().position(|l| !l.trim().is_empty()) else {
            return Ok(None);
        };
        let end = lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map(|ix| ix + 1)
            .unwrap_or(start + 1);

        let message = lines[start..end].join("\n");
        if message.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(message))
        }
    }
}

struct RebaseScripts {
    _dir: tempfile::TempDir,
    seq_editor_path: PathBuf,
    todo_path: PathBuf,
    msg_editor_path: Option<PathBuf>,
    msgs_dir: PathBuf,
}

impl RebaseScripts {
    fn create(entries: &[InteractiveRebaseEntry], git_dir: PathBuf) -> std::io::Result<Self> {
        let dir = tempfile::tempdir()?;

        let todo_content = build_todo_content(entries);
        let todo_path = dir.path().join("git-rebase-todo");
        fs::write(&todo_path, &todo_content)?;

        #[cfg(windows)]
        let seq_editor_name = "gitcomet-seq-editor.cmd";
        #[cfg(not(windows))]
        let seq_editor_name = "gitcomet-seq-editor.sh";
        let seq_editor_path = dir.path().join(seq_editor_name);
        fs::write(&seq_editor_path, seq_editor_script_contents())?;
        #[cfg(unix)]
        make_executable(&seq_editor_path)?;

        let msgs_dir = dir.path().join("msgs");
        let mut msg_editor_path = None;
        let has_reword = entries
            .iter()
            .any(|e| e.action == InteractiveRebaseAction::Reword);
        if has_reword {
            fs::create_dir_all(&msgs_dir)?;
            for entry in entries {
                if entry.action == InteractiveRebaseAction::Reword {
                    if let Some(ref msg) = entry.new_message {
                        let msg_file = msgs_dir.join(&entry.commit_id);
                        fs::write(msg_file, msg.as_bytes())?;
                    }
                }
            }

            #[cfg(windows)]
            let msg_editor_name = "gitcomet-msg-editor.cmd";
            #[cfg(not(windows))]
            let msg_editor_name = "gitcomet-msg-editor.sh";
            let path = dir.path().join(msg_editor_name);
            let script = msg_editor_script_contents(&git_dir);
            fs::write(&path, script.as_bytes())?;
            #[cfg(unix)]
            make_executable(&path)?;
            msg_editor_path = Some(path);
        }

        Ok(Self {
            _dir: dir,
            seq_editor_path,
            todo_path,
            msg_editor_path,
            msgs_dir,
        })
    }
}

fn build_todo_content(entries: &[InteractiveRebaseEntry]) -> String {
    entries
        .iter()
        .map(|e| {
            let safe_summary = e.summary.replace('\n', " ");
            format!(
                "{} {} {}\n",
                e.action.to_todo_str(),
                e.commit_id,
                safe_summary
            )
        })
        .collect()
}

#[cfg(unix)]
fn make_executable(path: &PathBuf) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
}

#[cfg(windows)]
fn seq_editor_script_contents() -> &'static [u8] {
    b"@echo off\ncopy /Y \"%GITCOMET_TODO_FILE%\" \"%~1\" >nul\n"
}

#[cfg(not(windows))]
fn seq_editor_script_contents() -> &'static [u8] {
    b"#!/bin/sh\ncp \"$GITCOMET_TODO_FILE\" \"$1\"\n"
}

#[cfg(windows)]
fn msg_editor_script_contents(_git_dir: &PathBuf) -> String {
    r#"@echo off
set "REBASE_HEAD_FILE=%GITCOMET_GIT_DIR%\REBASE_HEAD"
if not exist "%REBASE_HEAD_FILE%" exit /b 0
set /p sha=<"%REBASE_HEAD_FILE%"
set "msg_file=%GITCOMET_MSGS_DIR%\%sha%"
if not exist "%msg_file%" exit /b 0
copy /Y "%msg_file%" "%~1" >nul
"#
    .to_string()
}

#[cfg(not(windows))]
fn msg_editor_script_contents(_git_dir: &PathBuf) -> String {
    r#"#!/bin/sh
if [ -f "$GITCOMET_GIT_DIR/REBASE_HEAD" ]; then
  sha=$(cat "$GITCOMET_GIT_DIR/REBASE_HEAD")
  msg_file="$GITCOMET_MSGS_DIR/$sha"
  if [ -f "$msg_file" ]; then
    cp "$msg_file" "$1"
  fi
fi
"#
    .to_string()
}
