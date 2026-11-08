use super::super::repository_registry::RepositoryTarget;
use gitcomet_core::services::{GitBackend, GitRepository};
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Target = RepositoryTarget<Arc<dyn GitRepository>>;

fn target_index(targets: &[Target], path: &Path) -> Option<usize> {
    targets
        .iter()
        .enumerate()
        .filter(|(_, target)| path.starts_with(&target.root))
        // The innermost root still wins, including a bare nested repository.
        // Other windows may publish the same root; reuse their live handle.
        .max_by_key(|(_, target)| {
            (
                target.root.components().count(),
                target.repository.is_some(),
                target.parked,
            )
        })
        .map(|(index, _)| index)
}

pub(super) fn target<'a>(targets: &'a [Target], path: &Path) -> Option<&'a Target> {
    target_index(targets, path).map(|index| &targets[index])
}

pub(super) fn resolve(
    targets: &mut [Target],
    paths: &[PathBuf],
    backend: &dyn GitBackend,
) -> std::io::Result<()> {
    for path in paths {
        let Some(index) = target_index(targets, path) else {
            continue;
        };
        let target = &mut targets[index];
        if target.repository.is_none() && target.parked {
            target.repository = Some(
                backend
                    .open(&target.root)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Backend(AtomicUsize);

    impl GitBackend for Backend {
        fn open(&self, root: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(
                gitcomet_core::test_support::UnconfiguredRepository::new(root.to_path_buf()),
            ))
        }
    }

    #[test]
    fn duplicate_roots_reuse_live_handles_or_resolve_the_parked_copy_once() {
        let root = PathBuf::from("/nested");
        for reversed in [false, true] {
            for live in [false, true] {
                let backend = Backend::default();
                let repository = live.then(|| {
                    Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                        root.clone(),
                    )) as Arc<dyn GitRepository>
                });
                let mut targets = vec![
                    Target {
                        root: root.clone(),
                        repository: None,
                        parked: true,
                    },
                    Target {
                        root: root.clone(),
                        repository: repository.clone(),
                        parked: false,
                    },
                ];
                if reversed {
                    targets.reverse();
                }
                targets.push(Target {
                    root: PathBuf::from("/unrelated"),
                    repository: None,
                    parked: true,
                });
                let paths = [root.join("a.txt"), root.join("b.txt")];
                resolve(&mut targets, &paths, &backend).unwrap();
                assert_eq!(backend.0.load(Ordering::SeqCst), usize::from(!live));
                let resolved = target(&targets, &paths[0])
                    .unwrap()
                    .repository
                    .as_ref()
                    .unwrap();
                if let Some(repository) = repository {
                    assert!(Arc::ptr_eq(resolved, &repository));
                }
                assert!(targets[2].repository.is_none(), "unaffected tabs stay cold");
            }
        }
    }

    #[test]
    fn an_initial_open_keeps_its_existing_wait_policy() {
        let backend = Backend::default();
        let mut targets = [Target {
            root: PathBuf::from("/opening"),
            repository: None,
            parked: false,
        }];
        resolve(&mut targets, &[PathBuf::from("/opening/file")], &backend).unwrap();
        assert_eq!(backend.0.load(Ordering::SeqCst), 0);
        assert!(targets[0].repository.is_none());
    }
}
