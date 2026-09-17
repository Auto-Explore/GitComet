//! Each status worker owns its filter processes. On Windows, starting many LFS
//! pipelines can cost much more than comparing the files themselves.
use gitcomet_core::services::{CancellationToken, Result};

pub(super) fn for_repo(
    repo: &gix::Repository,
    cancellation: &CancellationToken,
) -> Result<Option<usize>> {
    #[cfg(windows)]
    {
        if let Some(limit) = benchmark_override() {
            return Ok(limit);
        }
        if !has_lfs_filter(&repo.config_snapshot()) {
            return Ok(None);
        }
        match repo.index_or_empty() {
            Ok(index) => for_index(repo, &index, cancellation),
            Err(_) => Ok(Some(bounded_limit())),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (repo, cancellation);
        Ok(None)
    }
}

pub(super) fn for_index(
    repo: &gix::Repository,
    index: &gix::index::State,
    cancellation: &CancellationToken,
) -> Result<Option<usize>> {
    #[cfg(windows)]
    {
        if let Some(limit) = benchmark_override() {
            return Ok(limit);
        }
        production_limit(repo, index, cancellation)
    }
    #[cfg(not(windows))]
    {
        let _ = (repo, index, cancellation);
        Ok(None)
    }
}

#[cfg(any(windows, test))]
fn has_lfs_filter(config: &gix::config::File) -> bool {
    ["filter.lfs.process", "filter.lfs.clean"]
        .iter()
        .any(|key| config.string(*key).is_some_and(|value| !value.is_empty()))
}

#[cfg(any(windows, test))]
fn bounded_limit() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8)
}

#[cfg(any(windows, test))]
fn production_limit(
    repo: &gix::Repository,
    index: &gix::index::State,
    cancellation: &CancellationToken,
) -> Result<Option<usize>> {
    use gix::index::entry::Mode;
    if !has_lfs_filter(&repo.config_snapshot()) {
        return Ok(None);
    }
    cancellation.check_cancelled()?;
    let Some(workdir) = repo.workdir() else {
        return Ok(None);
    };
    let regular =
        |entry: &&gix::index::Entry| matches!(entry.mode, Mode::FILE | Mode::FILE_EXECUTABLE);
    let files = index.entries().iter().filter(regular).count();
    let Ok(mut attributes) = repo.attributes_only(
        index,
        gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
    ) else {
        return Ok(Some(bounded_limit()));
    };
    let mut outcome = gix::attrs::search::Outcome::default();
    let mut filtered = 0usize;
    let mut bytes = 0u64;
    for entry in index.entries().iter().filter(regular) {
        cancellation.check_cancelled()?;
        let path = entry.path(index);
        let Ok(platform) = attributes.at_entry(path, Some(entry.mode)) else {
            return Ok(Some(bounded_limit()));
        };
        platform.matching_attributes(&mut outcome);
        let lfs = outcome.iter().any(|matched| {
            matched.assignment.name.as_str() == "filter" && matches!(matched.assignment.state, gix::attrs::StateRef::Value(value) if value.as_bstr() == b"lfs")
        });
        if !lfs {
            continue;
        }
        filtered += 1;
        // Large and mixed worktrees keep enough parallelism for ordinary files.
        if files > 128 {
            return Ok(Some(bounded_limit()));
        }
        let Ok(path) = gix::path::try_from_bstr(path) else {
            return Ok(Some(bounded_limit()));
        };
        let Ok(metadata) = std::fs::symlink_metadata(workdir.join(path)) else {
            return Ok(Some(bounded_limit()));
        };
        bytes = bytes.saturating_add(metadata.len());
        if bytes > 64 * 1024 * 1024 {
            return Ok(Some(bounded_limit()));
        }
    }
    Ok(if filtered == 0 {
        None
    } else if filtered * 2 >= files {
        Some(1)
    } else {
        Some(bounded_limit())
    })
}

/// `Some(None)` explicitly requests the uncapped baseline. Normal builds never
/// consult the environment; absent/"production" selects the production policy.
#[cfg(windows)]
fn benchmark_override() -> Option<Option<usize>> {
    #[cfg(feature = "benchmarks")]
    if let Ok(value) = std::env::var("GITCOMET_BENCH_STATUS_WORKERS") {
        return match value.as_str() {
            "auto" => Some(None),
            "1" => Some(Some(1)),
            "4" | "8" => {
                Some(Some(value.parse::<usize>().unwrap().min(
                    std::thread::available_parallelism().map_or(1, usize::from),
                )))
            }
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn fixture(attributes: &str) -> (tempfile::TempDir, gix::Repository) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init"]);
        std::fs::write(dir.path().join("a.bin"), b"asset").unwrap();
        std::fs::write(dir.path().join(".gitattributes"), attributes).unwrap();
        git(&[
            "-c",
            "filter.lfs.required=false",
            "-c",
            "filter.lfs.process=",
            "-c",
            "filter.lfs.clean=",
            "add",
            ".",
        ]);
        git(&["config", "filter.lfs.process", "git-lfs filter-process"]);
        let repo = gix::open(dir.path()).unwrap();
        (dir, repo)
    }

    fn policy(repo: &gix::Repository) -> Option<usize> {
        production_limit(
            repo,
            &repo.index_or_empty().unwrap(),
            &CancellationToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn configured_filter_without_matching_paths_keeps_parallelism() {
        let (_dir, repo) = fixture("*.unused filter=lfs\n");
        assert_eq!(policy(&repo), None);
    }

    #[test]
    fn resolves_effective_attributes_and_index_fallback() {
        let (dir, repo) = fixture("[attr]asset filter=lfs\n*.bin asset\n");
        assert_eq!(policy(&repo), Some(1));
        std::fs::write(dir.path().join(".git/info/attributes"), b"a.bin -filter\n").unwrap();
        assert_eq!(policy(&repo), None);
        std::fs::remove_file(dir.path().join(".git/info/attributes")).unwrap();
        std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(policy(&repo), Some(1));
    }

    #[test]
    fn global_attributes_and_nested_overrides_control_the_worker_policy() {
        let (dir, _) = fixture("");
        let global = dir.path().join(".git/global-attributes");
        std::fs::write(&global, "*.bin filter=lfs\n").unwrap();
        let git = |args: &[&std::ffi::OsStr]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(dir.path())
                    .args(args)
                    .env("GIT_CONFIG_GLOBAL", "")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&[
            "config".as_ref(),
            "core.attributesFile".as_ref(),
            global.as_os_str(),
        ]);
        let repo = gix::open(dir.path()).unwrap();
        assert_eq!(policy(&repo), Some(1));
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/b.bin"), "asset").unwrap();
        git(&[
            "-c".as_ref(),
            "filter.lfs.required=false".as_ref(),
            "-c".as_ref(),
            "filter.lfs.process=".as_ref(),
            "-c".as_ref(),
            "filter.lfs.clean=".as_ref(),
            "add".as_ref(),
            "nested/b.bin".as_ref(),
        ]);
        let repo = gix::open(dir.path()).unwrap();
        assert_eq!(policy(&repo), Some(1));
        std::fs::write(dir.path().join("nested/.gitattributes"), "*.bin -filter\n").unwrap();
        assert_eq!(policy(&repo), Some(bounded_limit()));
        std::fs::write(dir.path().join(".git/info/attributes"), "*.bin -filter\n").unwrap();
        assert_eq!(policy(&repo), None);
    }

    #[test]
    fn large_assets_keep_bounded_parallelism_and_classification_is_cancellable() {
        let (dir, repo) = fixture("*.bin filter=lfs\n");
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join("a.bin"))
            .unwrap()
            .set_len(65 * 1024 * 1024)
            .unwrap();
        assert_eq!(policy(&repo), Some(bounded_limit()));
        let token = CancellationToken::new();
        token.cancel();
        assert!(production_limit(&repo, &repo.index_or_empty().unwrap(), &token).is_err());
    }
}
