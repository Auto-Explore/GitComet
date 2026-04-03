use super::GixRepo;
use crate::util::{path_buf_from_git_bytes, run_git_capture_bytes, run_git_with_output};
use gitcomet_core::domain::{CommitId, Worktree};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CommandOutput, Result};
use std::path::Path;

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
        cmd.arg("worktree").arg("add").arg(path);
        let label = if let Some(reference) = reference {
            cmd.arg(reference);
            format!("git worktree add {} {}", path.display(), reference)
        } else {
            format!("git worktree add {}", path.display())
        };
        run_git_with_output(cmd, &label)
    }

    pub(super) fn remove_worktree_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("worktree").arg("remove").arg(path);
        run_git_with_output(cmd, &format!("git worktree remove {}", path.display()))
    }

    pub(super) fn force_remove_worktree_with_output_impl(
        &self,
        path: &Path,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("worktree").arg("remove").arg("--force").arg(path);
        run_git_with_output(
            cmd,
            &format!("git worktree remove --force {}", path.display()),
        )
    }
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
    use super::parse_git_worktree_list_porcelain_z;
    use std::path::PathBuf;

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
        assert_eq!(parsed[0].path, std::fs::canonicalize(&nested).unwrap());
    }
}
