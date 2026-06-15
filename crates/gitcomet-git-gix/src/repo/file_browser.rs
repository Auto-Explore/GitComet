use gitcomet_core::domain::{CommitId, FileEntry, FileEntryKind};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use std::path::PathBuf;
use std::sync::Arc;

use super::GixRepo;

impl GixRepo {
    pub(super) fn list_tree_files_impl(&self) -> Result<Vec<FileEntry>> {
        let repo = self._repo.to_thread_local();
        let head = repo
            .head_commit()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head_commit: {e}"))))?;
        let tree_id = head
            .tree_id()
            .map(|id| id.detach())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree_id: {e}"))))?;
        list_tree_files_at_oid(&repo, tree_id)
    }

    pub(super) fn list_tree_files_at_commit_impl(
        &self,
        commit_id: &CommitId,
    ) -> Result<Vec<FileEntry>> {
        let repo = self._repo.to_thread_local();
        let oid = gix::ObjectId::from_hex(commit_id.0.as_bytes())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("invalid commit id: {e}"))))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find_commit: {e}"))))?;
        let tree_id = commit
            .tree_id()
            .map(|id| id.detach())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree_id: {e}"))))?;
        list_tree_files_at_oid(&repo, tree_id)
    }
}

fn list_tree_files_at_oid(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
) -> Result<Vec<FileEntry>> {
    let object = repo
        .find_object(tree_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find_object: {e}"))))?;
    let tree = object
        .peel_to_tree()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel_to_tree: {e}"))))?;

    let mut entries = Vec::new();
    collect_tree_entries(repo, &tree, String::new(), &mut entries, 0)?;
    Ok(entries)
}

fn collect_tree_entries(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    parent_path: String,
    out: &mut Vec<FileEntry>,
    depth: usize,
) -> Result<()> {
    let mut child_entries: Vec<(String, gix::objs::tree::EntryMode, gix::ObjectId)> = Vec::new();

    for entry in tree.iter() {
        let entry =
            entry.map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree entry: {e}"))))?;
        let name = entry.filename().to_string();
        let mode = entry.mode();
        let oid = entry.oid().to_owned();
        child_entries.push((name, mode, oid));
    }

    child_entries.sort_by(|(a_name, a_mode, _), (b_name, b_mode, _)| {
        let a_is_dir = a_mode.is_tree();
        let b_is_dir = b_mode.is_tree();
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a_name.cmp(b_name),
        }
    });

    for (name, mode, oid) in child_entries {
        let path = if parent_path.is_empty() {
            name.clone()
        } else {
            format!("{parent_path}/{name}")
        };

        if mode.is_tree() {
            out.push(FileEntry {
                name,
                path: Arc::new(PathBuf::from(&path)),
                kind: FileEntryKind::Directory,
                depth,
            });

            let child_object = repo
                .find_object(oid)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find_object: {e}"))))?;
            let child_tree = child_object
                .peel_to_tree()
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel_to_tree: {e}"))))?;
            collect_tree_entries(repo, &child_tree, path, out, depth + 1)?;
        } else if mode.is_blob() || mode.is_link() {
            out.push(FileEntry {
                name,
                path: Arc::new(PathBuf::from(&path)),
                kind: FileEntryKind::File,
                depth,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::GixRepo;
    use gitcomet_core::domain::{CommitId, FileEntryKind};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn git_success(workdir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(args)
            .output()
            .expect("git command to run");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_test_repo(workdir: &Path) {
        git_success(workdir, &["init"]);
        for args in [
            ["config", "user.name", "Test User"].as_slice(),
            ["config", "user.email", "test@example.com"].as_slice(),
        ] {
            git_success(workdir, args);
        }
    }

    fn write_file(workdir: &Path, relative: &str, contents: &str) {
        let path = workdir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    fn commit_file(workdir: &Path, path: &str, contents: &str, message: &str) {
        write_file(workdir, path, contents);
        git_success(workdir, &["add", path]);
        git_success(workdir, &["commit", "-m", message]);
    }

    fn open_repo(workdir: &Path) -> GixRepo {
        let thread_safe_repo = gix::open(workdir).expect("open repo").into_sync();
        GixRepo::new(workdir.to_path_buf(), thread_safe_repo)
    }

    #[test]
    fn list_tree_files_returns_flat_files_correctly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "README.md", "# Project", "first");
        commit_file(workdir, "main.rs", "fn main() {}", "second");

        let repo = open_repo(workdir);
        let entries = repo.list_tree_files_impl().expect("list tree files");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "README.md");
        assert_eq!(entries[0].kind, FileEntryKind::File);
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[1].name, "main.rs");
        assert_eq!(entries[1].kind, FileEntryKind::File);
        assert_eq!(entries[1].depth, 0);
    }

    #[test]
    fn list_tree_files_sorts_directories_before_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "z_file.txt", "z", "first");
        commit_file(workdir, "a_dir/nested.txt", "nested", "second");

        let repo = open_repo(workdir);
        let entries = repo.list_tree_files_impl().expect("list tree files");

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "a_dir");
        assert_eq!(entries[0].kind, FileEntryKind::Directory);
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[1].name, "nested.txt");
        assert_eq!(entries[1].kind, FileEntryKind::File);
        assert_eq!(entries[1].depth, 1);
        assert_eq!(entries[2].name, "z_file.txt");
        assert_eq!(entries[2].kind, FileEntryKind::File);
        assert_eq!(entries[2].depth, 0);
    }

    #[test]
    fn list_tree_files_nested_directories_have_correct_depth() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "src/main.rs", "fn main() {}", "add src/main.rs");
        commit_file(
            workdir,
            "src/utils/helpers.rs",
            "pub fn help() {}",
            "add helpers",
        );

        let repo = open_repo(workdir);
        let entries = repo.list_tree_files_impl().expect("list tree files");

        assert_eq!(entries.len(), 4);

        assert_eq!(entries[0].name, "src");
        assert_eq!(entries[0].kind, FileEntryKind::Directory);
        assert_eq!(entries[0].depth, 0);

        assert_eq!(entries[1].name, "utils");
        assert_eq!(entries[1].kind, FileEntryKind::Directory);
        assert_eq!(entries[1].depth, 1);

        assert_eq!(entries[2].name, "helpers.rs");
        assert_eq!(entries[2].kind, FileEntryKind::File);
        assert_eq!(entries[2].depth, 2);

        assert_eq!(entries[3].name, "main.rs");
        assert_eq!(entries[3].kind, FileEntryKind::File);
        assert_eq!(entries[3].depth, 1);
    }

    #[test]
    fn list_tree_files_at_commit_returns_same_tree_as_head() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "alpha.txt", "alpha", "first commit");
        commit_file(workdir, "beta.txt", "beta", "second commit");

        let repo = open_repo(workdir);

        let head_entries = repo.list_tree_files_impl().expect("head tree");

        let output = Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("rev-parse");
        let head_sha = String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string();
        let commit_id = CommitId(head_sha.into());

        let commit_entries = repo
            .list_tree_files_at_commit_impl(&commit_id)
            .expect("commit tree");

        assert_eq!(head_entries.len(), commit_entries.len());
        for (a, b) in head_entries.iter().zip(commit_entries.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.path, b.path);
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.depth, b.depth);
        }
    }

    #[test]
    fn list_tree_files_at_commit_respects_historical_tree() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "only_in_first.txt", "first", "first commit");

        let output = Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("rev-parse");
        let first_sha = String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string();
        let first_commit_id = CommitId(first_sha.into());

        commit_file(workdir, "added_later.txt", "later", "second commit");

        let repo = open_repo(workdir);

        let first_entries = repo
            .list_tree_files_at_commit_impl(&first_commit_id)
            .expect("first commit tree");

        assert_eq!(first_entries.len(), 1);
        assert_eq!(first_entries[0].name, "only_in_first.txt");
    }
}
