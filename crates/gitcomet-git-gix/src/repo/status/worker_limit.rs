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
    // Only the effective filter controls this policy. Let gix stop matching
    // once it resolves that attribute, including through macros and overrides.
    let mut outcome = attributes.selected_attribute_matches(["filter"]);
    let mut filtered = 0usize;
    let mut bytes = 0u64;
    for entry in index.entries().iter().filter(regular) {
        cancellation.check_cancelled()?;
        let path = entry.path(index);
        let Ok(platform) = attributes.at_entry(path, Some(entry.mode)) else {
            return Ok(Some(bounded_limit()));
        };
        platform.matching_attributes(&mut outcome);
        let lfs = outcome.iter_selected().any(|matched| {
            matches!(matched.assignment.state, gix::attrs::StateRef::Value(value) if value.as_bstr() == super::super::large_files::LFS_FILTER_DRIVER)
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

    /// Stages `path` with the configured LFS filter disabled, so git-lfs need
    /// not be installed. Index refreshes re-filter racily clean entries too.
    fn add_without_lfs(workdir: &std::path::Path, path: &str) {
        git_success(
            workdir,
            &[
                "-c",
                "filter.lfs.required=false",
                "-c",
                "filter.lfs.process=",
                "-c",
                "filter.lfs.clean=",
                "add",
                path,
            ],
        );
    }

    fn fixture(attributes: &str) -> (tempfile::TempDir, gix::Repository) {
        let dir = tempfile::tempdir().unwrap();
        git_success(dir.path(), &["init"]);
        std::fs::write(dir.path().join("a.bin"), b"asset").unwrap();
        std::fs::write(dir.path().join(".gitattributes"), attributes).unwrap();
        add_without_lfs(dir.path(), ".");
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
        let (dir, repo) =
            fixture("[attr]asset filter=lfs -diff -text\n*.bin asset\n*.bin eol=lf\n");
        assert_eq!(policy(&repo), Some(1));
        for assignment in ["-filter", "!filter", "filter", "filter=other"] {
            std::fs::write(
                dir.path().join(".git/info/attributes"),
                format!("a.bin {assignment} text eol=lf\n"),
            )
            .unwrap();
            assert_eq!(policy(&repo), None, "override: {assignment}");
        }
        std::fs::remove_file(dir.path().join(".git/info/attributes")).unwrap();
        std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(policy(&repo), Some(1));
    }

    #[test]
    fn selection_discovers_nested_filter_and_preserves_worktree_over_index_precedence() {
        let (dir, _) = fixture("* text=auto\n");
        let root = dir.path();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/b.bin"), "asset").unwrap();
        git_success(root, &["add", "nested/b.bin"]);
        let repo = open_gix(root);
        assert_eq!(policy(&repo), None);

        // The selection is created before at_entry discovers this first
        // definition of filter. Unrelated later rules must not hide its value.
        let nested_attributes = root.join("nested/.gitattributes");
        std::fs::write(
            &nested_attributes,
            "*.bin filter=lfs\n*.bin text eol=lf -diff\n",
        )
        .unwrap();
        assert_eq!(policy(&repo), Some(bounded_limit()));

        add_without_lfs(root, "nested/.gitattributes");
        let repo = open_gix(root);
        std::fs::remove_file(&nested_attributes).unwrap();
        assert_eq!(policy(&repo), Some(bounded_limit()));
        std::fs::write(&nested_attributes, "*.bin !filter\n").unwrap();
        assert_eq!(policy(&repo), None);
        std::fs::remove_file(&nested_attributes).unwrap();
        assert_eq!(policy(&repo), Some(bounded_limit()));
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
        add_without_lfs(dir.path(), "nested/b.bin");
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

    /// `GITCOMET_PROBE_REPO=<worktree> cargo test -p gitcomet-git-gix --lib
    /// timing_worker_limit -- --ignored --nocapture`: the Windows status
    /// policy's cost per refresh where `filter.lfs` is configured globally.
    #[test]
    #[ignore = "timing probe"]
    fn timing_worker_limit_policy() {
        let root = std::path::PathBuf::from(
            std::env::var_os("GITCOMET_PROBE_REPO").expect("set GITCOMET_PROBE_REPO"),
        );
        let repo = open_gix(&root);
        let index = repo.index_or_empty().unwrap();
        let mut samples = Vec::new();
        let mut limit = None;
        for _ in 0..5 {
            let started = std::time::Instant::now();
            limit = production_limit(&repo, &index, &CancellationToken::new()).unwrap();
            samples.push(started.elapsed());
        }
        samples.sort();
        println!(
            "timing worker_limit entries {} limit {limit:?} p50 {:?}",
            index.entries().len(),
            samples[2]
        );
    }
}
