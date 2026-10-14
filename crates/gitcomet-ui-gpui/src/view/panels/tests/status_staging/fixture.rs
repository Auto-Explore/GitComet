use super::{commit, git};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

type RepositoryTemplate = Vec<(PathBuf, Vec<u8>)>;

// Keep bytes in the shared template; each scenario restores its own writable
// index. The construction TempDir stays alive until the snapshot is complete.
pub(super) fn clean_repository_template() -> &'static RepositoryTemplate {
    static TEMPLATE: OnceLock<RepositoryTemplate> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        build_repository(dir.path());
        snapshot_repository(dir.path())
    })
}

fn build_repository(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main", "--template="]);
    // Detached maintenance can remove its lock after commit returns, while
    // the byte snapshot is reading it. This fixture has no maintenance work.
    git(dir, &["config", "maintenance.auto", "false"]);
    git(dir, &["config", "gc.auto", "0"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    std::fs::create_dir(dir.join("nested")).unwrap();
    for path in ["a.txt", "b.txt", "nested/c.txt", "nested/d.txt"] {
        std::fs::write(dir.join(path), "base\n").unwrap();
    }
    let moved: String = (0..20).map(|line| format!("line {line}\n")).collect();
    std::fs::write(dir.join("moved.txt"), moved).unwrap();
    git(dir, &["add", "."]);
    commit(dir, "base");
}

fn is_runtime_git_path(relative: &Path) -> bool {
    relative.starts_with(".git")
        && (relative == Path::new(".git/gc.pid")
            || relative
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().ends_with(b".lock")))
}

fn snapshot_repository(root: &Path) -> RepositoryTemplate {
    fn collect(root: &Path, dir: &Path, files: &mut RepositoryTemplate) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("cannot snapshot directory {}: {error}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!("cannot snapshot entry in {}: {error}", dir.display())
            });
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap();
            // A live Git process can remove these between directory enumeration
            // and metadata/read. Even stable orphan locks must not be restored.
            if is_runtime_git_path(relative) {
                continue;
            }
            let file_type = entry.file_type().unwrap_or_else(|error| {
                panic!("cannot snapshot metadata for {}: {error}", path.display())
            });
            if file_type.is_dir() {
                collect(root, &path, files);
            } else {
                let bytes = std::fs::read(&path).unwrap_or_else(|error| {
                    panic!("cannot snapshot file {}: {error}", path.display())
                });
                files.push((relative.to_path_buf(), bytes));
            }
        }
    }

    let mut files = Vec::new();
    collect(root, root, &mut files);
    files
}

pub(super) fn restore_repository(dir: &Path, template: &[(PathBuf, Vec<u8>)]) {
    for (relative, bytes) in template {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap_or_else(|error| {
            panic!("cannot restore parent for {}: {error}", path.display())
        });
        std::fs::write(&path, bytes)
            .unwrap_or_else(|error| panic!("cannot restore file {}: {error}", path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::super::git_output;
    use super::*;

    #[test]
    fn repository_snapshot_omits_runtime_files_and_restores_independent_indexes() {
        let source = tempfile::tempdir().unwrap();
        build_repository(source.path());
        for path in [".git/index.lock", ".git/config.lock", ".git/gc.pid"] {
            std::fs::write(source.path().join(path), "transient Git state\n").unwrap();
        }
        let maintenance_lock = source.path().join(".git/objects/maintenance.lock");
        #[cfg(unix)]
        std::os::unix::fs::symlink("removed-maintenance-state", &maintenance_lock).unwrap();
        #[cfg(not(unix))]
        std::fs::write(&maintenance_lock, "transient Git state\n").unwrap();

        // The dangling maintenance lock deterministically fails if the
        // snapshot reads transient state, just like a lock removed mid-read.
        let template = snapshot_repository(source.path());
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for restored in [first.path(), second.path()] {
            restore_repository(restored, &template);
            for path in [
                ".git/index.lock",
                ".git/config.lock",
                ".git/gc.pid",
                ".git/objects/maintenance.lock",
            ] {
                assert_eq!(
                    std::fs::symlink_metadata(restored.join(path))
                        .unwrap_err()
                        .kind(),
                    std::io::ErrorKind::NotFound,
                    "restored repository retained {path}"
                );
            }
            assert!(git_output(restored, &["status", "--porcelain=v1"]).is_empty());
        }

        let second_head = git_output(second.path(), &["rev-parse", "HEAD"]);
        let second_index = std::fs::read(second.path().join(".git/index")).unwrap();
        std::fs::write(first.path().join("a.txt"), "base\nchanged independently\n").unwrap();
        git(first.path(), &["add", "a.txt"]);
        commit(first.path(), "independent index");
        assert!(git_output(first.path(), &["status", "--porcelain=v1"]).is_empty());
        assert_ne!(
            git_output(first.path(), &["rev-parse", "HEAD"]),
            second_head
        );
        assert_eq!(
            git_output(second.path(), &["rev-parse", "HEAD"]),
            second_head
        );
        assert_eq!(
            std::fs::read(second.path().join(".git/index")).unwrap(),
            second_index
        );
        assert_eq!(
            std::fs::read(second.path().join("a.txt")).unwrap(),
            b"base\n"
        );
        assert!(git_output(second.path(), &["status", "--porcelain=v1"]).is_empty());
    }
}
