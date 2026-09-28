//! Retain the repository/index across text loads, refreshing its config
//! snapshot only when one of the configuration inputs changes.
use super::GixRepo;
use gitcomet_core::services::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) struct ConfigRepo {
    repo: Arc<gix::ThreadSafeRepository>,
    inputs: Vec<(PathBuf, std::io::Result<ConfigStamp>)>,
    branch: Option<(PathBuf, Option<Vec<u8>>)>,
}

/// Config freshness needs metadata, not another read of every input. Include
/// file identity and change time on Unix to catch replacements/backdated edits.
#[derive(Debug, PartialEq, Eq)]
struct ConfigStamp {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl ConfigStamp {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

fn symbolic_head(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    bytes
        .strip_prefix(b"ref: ")
        .map(|name| name.trim_ascii().to_vec())
}

impl ConfigRepo {
    pub(super) fn new(repo: gix::Repository) -> Self {
        let mut paths = vec![
            repo.common_dir().join("config"),
            repo.git_dir().join("config.worktree"),
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
        let mut depends_on_branch = false;
        for section in config.plumbing().sections() {
            if let Some(path) = &section.meta().path {
                paths.push(path.clone());
            }
            let name = section.header().name();
            depends_on_branch |= name.eq_ignore_ascii_case(b"includeIf")
                && section
                    .header()
                    .subsection_name()
                    .is_some_and(|condition| condition.starts_with(b"onbranch:"));
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
                let stamp = ConfigStamp::read(&path);
                (path, stamp)
            })
            .collect();
        let branch = depends_on_branch.then(|| {
            let path = repo.git_dir().join("HEAD");
            let name = symbolic_head(&path);
            (path, name)
        });
        Self {
            repo: Arc::new(repo.into_sync()),
            inputs,
            branch,
        }
    }

    fn is_current(&self) -> bool {
        self.branch
            .as_ref()
            .is_none_or(|(path, previous)| *previous == symbolic_head(path))
            && self.inputs.iter().all(|(path, previous)| {
                match (previous, ConfigStamp::read(path)) {
                    (Ok(previous), Ok(current)) => *previous == current,
                    (Err(previous), Err(current)) => {
                        previous.kind() == std::io::ErrorKind::NotFound
                            && current.kind() == std::io::ErrorKind::NotFound
                    }
                    _ => false,
                }
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
    fn review_detached_head_commits_do_not_reopen_config() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init(dir.path()).unwrap();
        let head = dir.path().join(".git/HEAD");
        std::fs::write(&head, format!("{}\n", "1".repeat(40))).unwrap();
        let cached = ConfigRepo::new(repo);
        std::fs::write(&head, format!("{}\n", "2".repeat(40))).unwrap();
        assert!(
            cached.is_current(),
            "detached commit IDs cannot change onbranch includes"
        );
    }

    #[test]
    fn onbranch_inputs_track_branch_names_and_missing_includes() {
        let dir = tempfile::tempdir().unwrap();
        gix::init(dir.path()).unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::write(
            git_dir.join("config"),
            "[includeIf \"onbranch:main\"]\npath = branch-config\n",
        )
        .unwrap();
        let head = git_dir.join("HEAD");
        std::fs::write(&head, format!("{}\n", "1".repeat(40))).unwrap();
        let cached = ConfigRepo::new(gix::open(dir.path()).unwrap());
        std::fs::write(&head, format!("{}\n", "2".repeat(40))).unwrap();
        assert!(cached.is_current());
        std::fs::write(&head, "ref: refs/heads/main\n").unwrap();
        assert!(!cached.is_current());
        let cached = ConfigRepo::new(gix::open(dir.path()).unwrap());
        std::fs::write(
            git_dir.join("branch-config"),
            "[gui]\nencoding = windows-1250\n",
        )
        .unwrap();
        assert!(
            !cached.is_current(),
            "creating a previously missing include invalidates the snapshot"
        );
    }

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
