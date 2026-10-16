//! Each status worker owns its filter processes. On Windows, starting many LFS
//! pipelines can cost much more than comparing the files themselves.
use gitcomet_core::services::{CancellationToken, Result};

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
    if !super::super::large_files::lfs_filter_configured(&repo.config_snapshot()) {
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
            matched.assignment.name.as_str() == "filter" && matches!(matched.assignment.state, gix::attrs::StateRef::Value(value) if value.as_bstr() == super::super::large_files::LFS_FILTER_DRIVER)
        });
        if !lfs {
            continue;
        }
        filtered += 1;
        // Large and mixed worktrees keep enough parallelism for ordinary files.
        if files > 128 {
            return Ok(Some(bounded_limit()));
        }
        let Ok(path) = gix::path::from_bstr(path) else {
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
    use crate::test_support::{git_success, open_gix};

    fn fixture(attributes: &str) -> (tempfile::TempDir, gix::Repository) {
        let dir = tempfile::tempdir().unwrap();
        git_success(dir.path(), &["init"]);
        std::fs::write(dir.path().join("a.bin"), b"asset").unwrap();
        std::fs::write(dir.path().join(".gitattributes"), attributes).unwrap();
        git_success(
            dir.path(),
            &[
                "-c",
                "filter.lfs.required=false",
                "-c",
                "filter.lfs.process=",
                "-c",
                "filter.lfs.clean=",
                "add",
                ".",
            ],
        );
        git_success(
            dir.path(),
            &["config", "filter.lfs.process", "git-lfs filter-process"],
        );
        let repo = open_gix(dir.path());
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
        git_success(
            dir.path(),
            &["config", "core.attributesFile", global.to_str().unwrap()],
        );
        let repo = open_gix(dir.path());
        assert_eq!(policy(&repo), Some(1));
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/b.bin"), "asset").unwrap();
        git_success(
            dir.path(),
            &[
                "-c",
                "filter.lfs.required=false",
                "-c",
                "filter.lfs.process=",
                "-c",
                "filter.lfs.clean=",
                "add",
                "nested/b.bin",
            ],
        );
        let repo = open_gix(dir.path());
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
