use super::failure;
use gitcomet_core::services::Result;

fn check_version(version: &str) -> Result<()> {
    let parsed = version.strip_prefix("git version ").and_then(|v| {
        let mut parts = v.split('.');
        Some((
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        ))
    });
    match parsed {
        Some(v) if v >= (2, 50) => Ok(()),
        _ => Err(failure(format!(
            "reftable repositories require Git 2.50 or newer; selected executable reports {version:?}. Select a newer Git executable in Settings."
        ))),
    }
}

pub(super) fn check(repo: &gix::Repository) -> Result<()> {
    let runtime = gitcomet_core::process::current_git_runtime();
    if let Some(version) = runtime.version_output() {
        return check_version(version.trim());
    }
    // Backend callers need the same capability gate even before the GUI's
    // asynchronous runtime probe. This runs once on open, never per ref view.
    let mut cmd = crate::util::git_workdir_cmd_for(repo.workdir().unwrap_or(repo.git_dir()));
    cmd.arg("--version");
    let output = crate::util::run_git_raw_output(cmd, "git --version")?;
    if !output.status.success() {
        return Err(crate::util::git_command_failed_error(
            "git --version",
            output,
        ));
    }
    check_version(String::from_utf8_lossy(&output.stdout).trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_transaction_protocol_has_an_explicit_baseline() {
        for version in [
            "git version 2.50.0",
            "git version 2.50.1.windows.1",
            "git version 3.0.0",
        ] {
            assert!(check_version(version).is_ok());
        }
        for version in [
            "git version 2.44.0",
            "git version 2.45.0",
            "git version 2.49.9",
            "git version unknown",
        ] {
            assert!(
                check_version(version)
                    .unwrap_err()
                    .to_string()
                    .contains("2.50")
            );
        }
    }
}
