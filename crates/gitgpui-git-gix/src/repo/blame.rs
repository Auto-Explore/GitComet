use super::GixRepo;
use crate::util::{run_git_capture, run_git_with_output};
use gitgpui_core::error::{Error, ErrorKind};
use gitgpui_core::services::{BlameLine, CommandOutput, ConflictSide, Result};
use rustc_hash::FxHashMap as HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

impl GixRepo {
    pub(super) fn blame_file_impl(&self, path: &Path, rev: Option<&str>) -> Result<Vec<BlameLine>> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.spec.workdir)
            .arg("blame")
            .arg("--line-porcelain");
        if let Some(rev) = rev {
            cmd.arg(rev);
        }
        cmd.arg("--").arg(path);

        let output = run_git_capture(cmd, "git blame --line-porcelain")?;
        Ok(parse_git_blame_porcelain(&output))
    }

    pub(super) fn checkout_conflict_side_impl(
        &self,
        path: &Path,
        side: ConflictSide,
    ) -> Result<CommandOutput> {
        let desired_stage = match side {
            ConflictSide::Ours => 2,
            ConflictSide::Theirs => 3,
        };

        let stage_exists = unmerged_stage_exists(&self.spec.workdir, path, desired_stage)?;

        if !stage_exists {
            let mut rm = Command::new("git");
            rm.arg("-C")
                .arg(&self.spec.workdir)
                .arg("rm")
                .arg("--")
                .arg(path);
            return run_git_with_output(rm, "git rm --");
        }

        let mut checkout = Command::new("git");
        checkout.arg("-C").arg(&self.spec.workdir).arg("checkout");
        match side {
            ConflictSide::Ours => {
                checkout.arg("--ours");
            }
            ConflictSide::Theirs => {
                checkout.arg("--theirs");
            }
        }
        checkout.arg("--").arg(path);
        let checkout_out = run_git_with_output(checkout, "git checkout --ours/--theirs")?;

        let mut add = Command::new("git");
        add.arg("-C")
            .arg(&self.spec.workdir)
            .arg("add")
            .arg("--")
            .arg(path);
        let add_out = run_git_with_output(add, "git add --")?;

        Ok(CommandOutput {
            command: checkout_out.command,
            stdout: [checkout_out.stdout, add_out.stdout]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            stderr: [checkout_out.stderr, add_out.stderr]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            exit_code: add_out.exit_code.or(checkout_out.exit_code),
        })
    }

    pub(super) fn accept_conflict_deletion_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut rm = Command::new("git");
        rm.arg("-C")
            .arg(&self.spec.workdir)
            .arg("rm")
            .arg("--")
            .arg(path);
        run_git_with_output(rm, "git rm --")
    }

    pub(super) fn checkout_conflict_base_impl(&self, path: &Path) -> Result<CommandOutput> {
        if !unmerged_stage_exists(&self.spec.workdir, path, 1)? {
            return Err(Error::new(ErrorKind::Backend(format!(
                "base conflict stage is not available for {}",
                path.display()
            ))));
        }

        let base_bytes = read_unmerged_stage_bytes(&self.spec.workdir, path, 1)?;
        let abs_path = self.spec.workdir.join(path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        }
        fs::write(&abs_path, base_bytes).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        let mut add = Command::new("git");
        add.arg("-C")
            .arg(&self.spec.workdir)
            .arg("add")
            .arg("--")
            .arg(path);
        let add_out = run_git_with_output(add, "git add --")?;

        Ok(CommandOutput {
            command: format!("git show :1:{} + git add --", path.display()),
            stdout: add_out.stdout,
            stderr: add_out.stderr,
            exit_code: add_out.exit_code,
        })
    }
}

fn unmerged_stage_exists(workdir: &Path, path: &Path, desired_stage: u8) -> Result<bool> {
    let mut ls = Command::new("git");
    ls.arg("-C")
        .arg(workdir)
        .arg("ls-files")
        .arg("-u")
        .arg("--")
        .arg(path);
    let output = ls
        .output()
        .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::new(ErrorKind::Backend(format!(
            "git ls-files -u failed: {}",
            stderr.trim()
        ))));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| {
        let mut parts = line.split_whitespace();
        let _mode = parts.next();
        let _sha = parts.next();
        let stage = parts.next().and_then(|s| s.parse::<u8>().ok());
        stage == Some(desired_stage)
    }))
}

fn read_unmerged_stage_bytes(workdir: &Path, path: &Path, stage: u8) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(workdir)
        .arg("-c")
        .arg("color.ui=false")
        .arg("--no-pager")
        .arg("show")
        .arg("--no-ext-diff")
        .arg("--pretty=format:")
        .arg(format!(":{stage}:{}", path.to_string_lossy()));

    let output = cmd
        .output()
        .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(Error::new(ErrorKind::Backend(format!(
        "git show :{stage}:{} failed: {}",
        path.display(),
        stderr.trim()
    ))))
}

fn parse_git_blame_porcelain(output: &str) -> Vec<BlameLine> {
    let mut out = Vec::new();
    let mut cached_by_commit: HashMap<String, (String, Option<i64>, String)> = HashMap::default();

    let mut current_commit: Option<String> = None;
    let mut author: Option<String> = None;
    let mut author_time: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in output.lines() {
        if line.starts_with('\t') {
            let commit = current_commit
                .clone()
                .unwrap_or_else(|| "0000000".to_string());
            let line_text = line.strip_prefix('\t').unwrap_or("").to_string();

            let cached = if author.is_none() && author_time.is_none() && summary.is_none() {
                cached_by_commit.get(&commit).cloned()
            } else {
                None
            };

            let (author_filled, author_time_filled, summary_filled) = cached.unwrap_or_else(|| {
                (
                    author.clone().unwrap_or_default(),
                    author_time,
                    summary.clone().unwrap_or_default(),
                )
            });

            cached_by_commit.insert(
                commit.clone(),
                (
                    author_filled.clone(),
                    author_time_filled,
                    summary_filled.clone(),
                ),
            );

            out.push(BlameLine {
                commit_id: commit,
                author: author_filled,
                author_time_unix: author_time_filled,
                summary: summary_filled,
                line: line_text,
            });

            author = None;
            author_time = None;
            summary = None;
            continue;
        }

        let mut parts = line.split_whitespace();
        if let Some(first) = parts.next() {
            let is_header = first.len() >= 8 && first.chars().all(|c| c.is_ascii_hexdigit());
            if is_header && parts.next().is_some() && parts.next().is_some() {
                current_commit = Some(first.to_string());
                continue;
            }
        }

        if let Some(rest) = line.strip_prefix("author ") {
            author = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            author_time = rest.trim().parse::<i64>().ok();
        } else if let Some(rest) = line.strip_prefix("summary ") {
            summary = Some(rest.to_string());
        }
    }

    out
}
