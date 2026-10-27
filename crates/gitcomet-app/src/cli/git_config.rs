use super::*;
use gitcomet_core::process::git_command as process_git_command;

fn git_command() -> std::process::Command {
    process_git_command()
}

fn trim_git_stdout_bytes(bytes: &[u8]) -> &[u8] {
    bytes.trim_ascii_end()
}

fn decode_git_text_stdout(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8(trim_git_stdout_bytes(bytes).to_vec()).ok()?;
    if text.is_empty() { None } else { Some(text) }
}

fn decode_git_path_stdout(bytes: &[u8]) -> Option<PathBuf> {
    let raw = trim_git_stdout_bytes(bytes);
    if raw.is_empty() {
        return None;
    }

    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        Some(PathBuf::from(OsString::from_vec(raw.to_vec())))
    }
    #[cfg(windows)]
    {
        let path_text = std::str::from_utf8(raw).ok()?;
        if path_text.is_empty() {
            None
        } else {
            Some(PathBuf::from(path_text))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        decode_git_text_stdout(raw).map(PathBuf::from)
    }
}

/// Read several git config keys from an explicit repository root with one
/// `git config --get-regexp` spawn. Later lines win, matching `--get`. Empty
/// when none of the keys is set or git is not available.
fn read_git_config_values_at_repo(repo_root: &Path, keys: &[&str]) -> Vec<(String, String)> {
    let pattern = format!(
        "^({})$",
        keys.iter()
            .map(|key| key.replace('.', "\\."))
            .collect::<Vec<_>>()
            .join("|")
    );
    let mut command = git_command();
    command
        .arg("-C")
        .arg(repo_root)
        .args(["config", "--get-regexp", &pattern]);
    gitcomet_core::git_execution_log::command_output(command, "Read repository configuration")
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| decode_git_text_stdout(&o.stdout))
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let (key, value) = line.split_once(' ')?;
                    Some((key.to_string(), value.trim().to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn git_repo_toplevel_from_probe_dir(probe_dir: &Path) -> Option<PathBuf> {
    let mut command = git_command();
    command
        .arg("-C")
        .arg(probe_dir)
        .args(["rev-parse", "--show-toplevel"]);
    let absolute_probe =
        gitcomet_core::path_utils::canonicalize_or_original(probe_dir.to_path_buf());
    let owner = crate::git_root::find_git_root(&absolute_probe).unwrap_or(absolute_probe);
    let key = gitcomet_core::git_execution_log::RepoLogKey::new(&owner);
    gitcomet_core::git_execution_log::command_output_in(&key, command, "Discover repository root")
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| decode_git_path_stdout(&o.stdout))
}

#[cfg(test)]
mod command_log_tests {
    use super::*;

    #[test]
    fn relative_discovery_from_a_subdirectory_uses_the_repository_history() {
        use gitcomet_core::git_execution_log::{GitExecutionLog, RepoLogKey};
        const ROOT_ENV: &str = "GITCOMET_TEST_COMMAND_LOG_ROOT";
        if let Some(root) = std::env::var_os(ROOT_ENV) {
            let discovered = git_repo_toplevel_from_probe_dir(Path::new(".")).unwrap();
            let key = RepoLogKey::new(Path::new(&root));
            assert_eq!(RepoLogKey::new(&discovered), key);
            let entries = GitExecutionLog::global().snapshot(&key);
            assert_eq!(entries.len(), 1);
            assert!(entries[0].command.contains("\".\""));
            assert!(entries[0].command.contains("--show-toplevel"));
            assert!(
                GitExecutionLog::global()
                    .snapshot(&RepoLogKey::new(Path::new(".")))
                    .is_empty()
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        assert!(
            gitcomet_core::test_support::git_fixture::git_command()
                .current_dir(root.path())
                .args(["init", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        let child = root.path().join("src/nested");
        std::fs::create_dir_all(&child).unwrap();
        let result = gitcomet_core::process::background_command(std::env::current_exe().unwrap())
            .arg("relative_discovery_from_a_subdirectory_uses_the_repository_history")
            .args(["--test-threads=1", "--nocapture"])
            .env(ROOT_ENV, root.path())
            .current_dir(child)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn discovery_from_a_subdirectory_uses_the_repository_history() {
        use gitcomet_core::git_execution_log::{GitExecutionLog, RepoLogKey};
        let root = tempfile::tempdir().unwrap();
        assert!(
            gitcomet_core::test_support::git_fixture::git_command()
                .current_dir(root.path())
                .args(["init", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        let child = root.path().join("src/nested");
        std::fs::create_dir_all(&child).unwrap();
        let discovered = git_repo_toplevel_from_probe_dir(&child).unwrap();
        assert_eq!(RepoLogKey::new(&discovered), RepoLogKey::new(root.path()));
        let entries = GitExecutionLog::global().snapshot(&RepoLogKey::new(root.path()));
        assert_eq!(entries.len(), 1);
        assert!(entries[0].command.contains("--show-toplevel"));
        assert!(
            GitExecutionLog::global()
                .snapshot(&RepoLogKey::new(&child))
                .is_empty()
        );
    }
}

fn resolve_git_repo_root_from_path(path: &Path) -> Option<PathBuf> {
    let mut probe_dirs = Vec::with_capacity(2);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        probe_dirs.push(parent.to_path_buf());
    }

    let path_buf = path.to_path_buf();
    if !path_buf.as_os_str().is_empty() && !probe_dirs.iter().any(|p| p == &path_buf) {
        probe_dirs.push(path_buf);
    }

    probe_dirs
        .into_iter()
        .find_map(|probe| git_repo_toplevel_from_probe_dir(&probe))
}

fn resolve_mergetool_repo_root(config: &MergetoolConfig) -> Option<PathBuf> {
    let mut candidates = vec![
        config.merged.as_path(),
        config.local.as_path(),
        config.remote.as_path(),
    ];
    if let Some(base) = config.base.as_deref() {
        candidates.push(base);
    }

    candidates
        .into_iter()
        .find_map(resolve_git_repo_root_from_path)
        .or_else(|| git_repo_toplevel_from_probe_dir(Path::new(".")))
}

/// Apply git config fallback for `merge.conflictstyle` and `diff.algorithm`
/// when the user did not provide explicit CLI flags.
///
/// This mirrors `git merge-file` behavior: the tool respects the user's
/// configured preferences without requiring them to modify the mergetool
/// command string. Git config accepts the alias spellings (`patience`,
/// `default`, `minimal`) that the CLI rejects.
fn apply_git_config_fallback(
    config: &mut MergetoolConfig,
    had_explicit_style: bool,
    had_explicit_algorithm: bool,
    git_config: &dyn Fn(&str) -> Option<String>,
) {
    if !had_explicit_style
        && let Some(style) = git_config("merge.conflictstyle")
        && let Some(style) =
            gitcomet_core::merge::parse_conflict_style(ConfigValueSource::GitConfig, &style)
    {
        config.conflict_style = style;
    }

    if !had_explicit_algorithm
        && let Some(algo) = git_config("diff.algorithm")
        && let Some(algo) =
            gitcomet_core::merge::parse_diff_algorithm(ConfigValueSource::GitConfig, &algo)
    {
        config.diff_algorithm = algo;
    }
}

/// Internal: resolve mergetool args with both env and git config fallback.
pub(super) fn resolve_mergetool_with_config(
    args: MergetoolArgs,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<MergetoolConfig, String> {
    let had_explicit_style = args.conflict_style.is_some();
    let had_explicit_algorithm = args.diff_algorithm.is_some();

    let mut config = resolve_mergetool_with_env(args, env)?;
    let repo_root = resolve_mergetool_repo_root(&config);
    let repo_values = repo_root
        .as_deref()
        .map(|repo| {
            read_git_config_values_at_repo(repo, &["merge.conflictstyle", "diff.algorithm"])
        })
        .unwrap_or_default();
    let repo_scoped_git_config = |key: &str| {
        repo_values
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.clone())
            .or_else(|| git_config(key))
    };
    apply_git_config_fallback(
        &mut config,
        had_explicit_style,
        had_explicit_algorithm,
        &repo_scoped_git_config,
    );
    Ok(config)
}
