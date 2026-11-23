use super::GixRepo;
use crate::util::{path_buf_from_git_bytes, run_git_metadata_capture};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::Result;

impl GixRepo {
    pub(super) fn check_automatic_fetch_helpers(&self) -> Result<()> {
        let mut command = self.git_workdir_cmd();
        command.args([
            "--no-pager",
            "config",
            "--null",
            "--show-scope",
            "--show-origin",
            "--name-only",
            "--list",
        ]);
        let listing = run_git_metadata_capture(command, "git config --show-origin")?;
        let repo = self.repo();
        let roots = [
            self.spec.workdir.as_path(),
            repo.git_dir(),
            repo.common_dir(),
        ]
        .map(|path| canonicalize_or_original(path.to_path_buf()));
        let malformed = || {
            Error::new(ErrorKind::Unsupported(
                "Could not check Git configuration before automatic fetch.",
            ))
        };
        let mut fields = listing
            .strip_suffix(b"\0")
            .ok_or_else(malformed)?
            .split(|&byte| byte == 0);
        while let Some(scope) = fields.next() {
            let origin = fields.next().ok_or_else(malformed)?;
            let key = fields.next().ok_or_else(malformed)?;
            let Some(helper) = automatic_fetch_helper(key) else {
                continue;
            };
            let repository_scope = matches!(scope, b"local" | b"worktree");
            let repository_origin = origin.strip_prefix(b"file:").is_some_and(|file| {
                let Ok(path) = path_buf_from_git_bytes(file, "Git configuration origin") else {
                    return true;
                };
                let path = canonicalize_or_original(self.spec.workdir.join(path));
                roots.iter().any(|root| path.starts_with(root))
            });
            if repository_scope || repository_origin {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "Automatic fetch skipped a repository-configured {helper}. Use Fetch all if you trust this checkout."
                ))));
            }
        }
        Ok(())
    }
}

fn automatic_fetch_helper(key: &[u8]) -> Option<&'static str> {
    if key.starts_with(b"credential.") && key.ends_with(b".helper") {
        return Some("credential helper");
    }
    if key.starts_with(b"hook.") {
        return Some("hook setting");
    }
    match key {
        b"core.sshcommand" => Some("SSH command"),
        b"core.gitproxy" => Some("Git proxy command"),
        _ if key.starts_with(b"remote.") && key.ends_with(b".uploadpack") => {
            Some("upload-pack command")
        }
        _ if key.starts_with(b"remote.") && key.ends_with(b".vcs") => Some("remote helper"),
        _ => None,
    }
}
