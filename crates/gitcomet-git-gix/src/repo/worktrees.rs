use super::GixRepo;
use crate::util::{
    path_buf_from_git_bytes, run_git_capture_bytes, run_git_with_output, validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, Worktree};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CommandOutput, Result};
use std::path::Path;
use std::process::Command;

impl GixRepo {
    pub(super) fn list_worktrees_impl(&self) -> Result<Vec<Worktree>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("worktree").arg("list").arg("--porcelain").arg("-z");
        let output = run_git_capture_bytes(cmd, "git worktree list --porcelain -z")?;
        parse_git_worktree_list_porcelain_z(&output)
    }

    pub(super) fn add_worktree_with_output_impl(
        &self,
        path: &Path,
        reference: Option<&str>,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = push_worktree_args(&mut cmd, &["add"], path, reference)?;
        run_git_with_output(cmd, &label)
    }

    pub(super) fn remove_worktree_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = push_worktree_args(&mut cmd, &["remove"], path, None)?;
        run_git_with_output(cmd, &label)
    }

    pub(super) fn force_remove_worktree_with_output_impl(
        &self,
        path: &Path,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = push_worktree_args(&mut cmd, &["remove", "--force"], path, None)?;
        run_git_with_output(cmd, &label)
    }
}

/// Append `git worktree <args> -- <path> [<reference>]` and return the display
/// label.
///
/// `path` and `reference` are typed by the user, so `--` keeps a value that
/// starts with `-` out of the option parser, and the reference is checked the
/// way every other ref argument in this crate is.
fn push_worktree_args(
    cmd: &mut Command,
    args: &[&str],
    path: &Path,
    reference: Option<&str>,
) -> Result<String> {
    if let Some(reference) = reference {
        validate_ref_like_arg(reference, "worktree reference")?;
    }
    // The label is a human-readable summary that the UI also inspects to
    // recover the worktree path, so `--` stays an argv concern only.
    cmd.arg("worktree").args(args).arg("--").arg(path);
    let mut label = format!("git worktree {} {}", args.join(" "), path.display());
    if let Some(reference) = reference {
        cmd.arg(reference);
        label.push(' ');
        label.push_str(reference);
    }
    Ok(label)
}

fn parse_git_worktree_list_porcelain_z(output: &[u8]) -> Result<Vec<Worktree>> {
    let mut out = Vec::new();
    let mut current: Option<Worktree> = None;

    for field in output.split(|b| *b == b'\0') {
        if field.is_empty() {
            if let Some(mut wt) = current.take() {
                canonicalize_worktree_path(&mut wt);
                out.push(wt);
            }
            continue;
        }

        if let Some(rest) = field.strip_prefix(b"worktree ") {
            if let Some(mut wt) = current.take() {
                canonicalize_worktree_path(&mut wt);
                out.push(wt);
            }
            current = Some(Worktree {
                path: path_buf_from_git_bytes(rest, "git worktree list path")?,
                head: None,
                branch: None,
                detached: false,
            });
            continue;
        }

        let Some(wt) = current.as_mut() else {
            continue;
        };

        if let Some(rest) = field.strip_prefix(b"HEAD ") {
            if !rest.is_empty() {
                wt.head = Some(CommitId(String::from_utf8_lossy(rest).into_owned().into()));
            }
        } else if let Some(rest) = field.strip_prefix(b"branch ") {
            let branch = String::from_utf8_lossy(rest);
            if let Some(stripped) = branch.strip_prefix("refs/heads/") {
                wt.branch = Some(stripped.to_string());
            } else if !branch.is_empty() {
                wt.branch = Some(branch.into_owned());
            }
        } else if field == b"detached" {
            wt.detached = true;
            wt.branch = None;
        }
    }

    if let Some(mut wt) = current.take() {
        canonicalize_worktree_path(&mut wt);
        out.push(wt);
    }

    Ok(out)
}

fn canonicalize_worktree_path(worktree: &mut Worktree) {
    worktree.path = canonicalize_or_original(worktree.path.clone());
}

#[cfg(test)]
mod tests {
    use super::{parse_git_worktree_list_porcelain_z, push_worktree_args};
    use gitcomet_core::path_utils::canonicalize_or_original;
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    fn worktree_add_places_separator_before_path_and_reference() {
        let mut cmd = Command::new("git");
        let label = push_worktree_args(&mut cmd, &["add"], Path::new("-linked"), Some("main"))
            .expect("valid worktree arguments");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("--"),
                OsStr::new("-linked"),
                OsStr::new("main"),
            ]
        );
        assert_eq!(label, "git worktree add -linked main");
    }

    #[test]
    fn worktree_add_rejects_option_like_reference() {
        let mut cmd = Command::new("git");
        let err = push_worktree_args(&mut cmd, &["add"], Path::new("linked"), Some("--detach"))
            .expect_err("an option-looking reference must be refused");
        assert!(err.to_string().contains("worktree reference"), "{err}");
        assert_eq!(
            cmd.get_args().count(),
            0,
            "nothing should be appended on refusal"
        );
    }

    #[test]
    fn worktree_remove_keeps_flags_before_separator() {
        let mut cmd = Command::new("git");
        let label = push_worktree_args(&mut cmd, &["remove", "--force"], Path::new("linked"), None)
            .expect("valid worktree arguments");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--force"),
                OsStr::new("--"),
                OsStr::new("linked"),
            ]
        );
        assert_eq!(label, "git worktree remove --force linked");
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_parses_regular_and_detached_entries() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0worktree /repo-linked\0HEAD 2222222222222222222222222222222222222222\0detached\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);

        assert_eq!(parsed[0].path, PathBuf::from("/repo"));
        assert_eq!(
            parsed[0].head.as_ref().map(|id| id.as_ref()),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert!(!parsed[0].detached);

        assert_eq!(parsed[1].path, PathBuf::from("/repo-linked"));
        assert_eq!(
            parsed[1].head.as_ref().map(|id| id.as_ref()),
            Some("2222222222222222222222222222222222222222")
        );
        assert!(parsed[1].branch.is_none());
        assert!(parsed[1].detached);
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_ignores_noise_before_first_worktree() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"HEAD deadbeef\0branch refs/heads/ignored\0\0worktree /repo\0branch feature/topic\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/repo"));
        assert_eq!(parsed[0].branch.as_deref(), Some("feature/topic"));
        assert!(parsed[0].head.is_none());
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_skips_empty_head_values() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\0HEAD \0branch refs/heads/main\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].head.is_none());
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_preserves_newlines_in_paths() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\nlinked\0HEAD 1111111111111111111111111111111111111111\0detached\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/repo\nlinked"));
        assert!(parsed[0].detached);
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_canonicalizes_existing_worktree_paths() {
        let root = std::env::temp_dir().join(format!(
            "gitcomet-worktree-parse-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let nested = root.join("repo");
        std::fs::create_dir_all(&nested).unwrap();

        let input = format!(
            "worktree {}\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0",
            nested.join("..").join("repo").display()
        );
        let parsed = parse_git_worktree_list_porcelain_z(input.as_bytes()).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, canonicalize_or_original(nested.clone()));
    }
}
