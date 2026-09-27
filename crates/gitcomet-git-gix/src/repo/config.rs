//! Retain the repository/index across text loads, refreshing its config
//! snapshot only when one of the configuration inputs changes.
use super::GixRepo;
use gitcomet_core::services::Result;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) struct ConfigRepo {
    repo: Arc<gix::ThreadSafeRepository>,
    inputs: Vec<(PathBuf, std::io::Result<Vec<u8>>)>,
}

impl ConfigRepo {
    pub(super) fn new(repo: gix::Repository) -> Self {
        let mut paths = vec![
            repo.common_dir().join("config"),
            repo.git_dir().join("config.worktree"),
            // Conditional onbranch includes can change without a config edit.
            repo.git_dir().join("HEAD"),
            repo.git_dir().join("commondir"),
        ];
        let config = repo.config_snapshot();
        let home = gix::path::env::home_dir();
        if let Some(home) = &home {
            paths.push(home.join(".gitconfig"));
        }
        if let Some(path) = gix::path::env::xdg_config("config", &mut |key| std::env::var_os(key)) {
            paths.push(path);
        }
        for section in config.plumbing().sections() {
            if let Some(path) = &section.meta().path {
                paths.push(path.clone());
            }
            let name = section.header().name();
            if name.eq_ignore_ascii_case(b"include") || name.eq_ignore_ascii_case(b"includeIf") {
                for value in section.values("path") {
                    let value = gix::config::Path::from(value);
                    if let Ok(path) = value.interpolate(gix::config::path::interpolate::Context {
                        home_dir: home.as_deref(),
                        git_install_dir: gix::path::env::installation_config_prefix(),
                        ..Default::default()
                    }) {
                        let parent = section
                            .meta()
                            .path
                            .as_ref()
                            .and_then(|path| path.parent())
                            .unwrap_or(repo.common_dir());
                        paths.push(if path.is_absolute() {
                            path
                        } else {
                            parent.join(path)
                        });
                    }
                }
            }
        }
        paths.sort();
        paths.dedup();
        let inputs = paths
            .into_iter()
            .map(|path| {
                let stamp = std::fs::read(&path);
                (path, stamp)
            })
            .collect();
        drop(config);
        Self {
            repo: Arc::new(repo.into_sync()),
            inputs,
        }
    }

    fn is_current(&self) -> bool {
        self.inputs
            .iter()
            .all(|(path, previous)| match (previous, std::fs::read(path)) {
                (Ok(previous), Ok(current)) => *previous == current,
                (Err(previous), Err(current)) => {
                    previous.kind() == std::io::ErrorKind::NotFound
                        && current.kind() == std::io::ErrorKind::NotFound
                }
                _ => false,
            })
    }
}

impl GixRepo {
    pub(super) fn repo_with_current_config(&self) -> Result<gix::Repository> {
        let mut cached = self
            .config_repo
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !cached.is_current() {
            *cached = ConfigRepo::new(self.reopen_repo()?);
        }
        Ok(cached.repo.to_thread_local())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_loads_reuse_repo_until_configuration_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init(dir.path()).unwrap();
        let repo = GixRepo::new(dir.path().to_path_buf(), repo.into_sync());
        let cached = Arc::clone(&repo.config_repo.lock().unwrap().repo);
        for _ in 0..3 {
            repo.text_attributes_impl(std::path::Path::new("a.txt"))
                .unwrap();
        }
        assert!(Arc::ptr_eq(&cached, &repo.config_repo.lock().unwrap().repo));
        let config_path = dir.path().join(".git/config");
        let mut text = std::fs::read_to_string(&config_path).unwrap();
        text.push_str("\n[core]\nwhitespace = tabwidth=8\n");
        std::fs::write(config_path, text).unwrap();
        assert_eq!(
            repo.text_attributes_impl(std::path::Path::new("a.txt"))
                .unwrap()
                .tab_width
                .unwrap()
                .columns,
            8
        );
        assert!(!Arc::ptr_eq(
            &cached,
            &repo.config_repo.lock().unwrap().repo
        ));
    }
}
