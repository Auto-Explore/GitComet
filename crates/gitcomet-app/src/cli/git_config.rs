use super::*;

/// Read a single git config value from an explicit repository root.
/// Returns `None` if the key is not set or git is not available.
fn read_git_config_at_repo(repo_root: &Path, key: &str) -> Option<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["config", "--get", key])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn git_repo_toplevel_from_probe_dir(probe_dir: &Path) -> Option<PathBuf> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(probe_dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
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
/// command string.
fn apply_git_config_fallback(
    config: &mut MergetoolConfig,
    had_explicit_style: bool,
    had_explicit_algorithm: bool,
    git_config: &dyn Fn(&str) -> Option<String>,
) {
    if !had_explicit_style && let Some(style) = git_config("merge.conflictstyle") {
        match style.as_str() {
            "merge" => config.conflict_style = ConflictStyle::Merge,
            "diff3" => config.conflict_style = ConflictStyle::Diff3,
            "zdiff3" => config.conflict_style = ConflictStyle::Zdiff3,
            _ => {} // ignore unrecognized values, keep default
        }
    }

    if !had_explicit_algorithm && let Some(algo) = git_config("diff.algorithm") {
        match algo.as_str() {
            "histogram" | "patience" => config.diff_algorithm = DiffAlgorithm::Histogram,
            "myers" | "default" | "minimal" => config.diff_algorithm = DiffAlgorithm::Myers,
            _ => {} // ignore unrecognized values, keep default
        }
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
    let repo_scoped_git_config = |key: &str| {
        repo_root
            .as_deref()
            .and_then(|repo| read_git_config_at_repo(repo, key))
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
