use gitcomet_core::domain::{CommitId, FileEntry, FileEntryKind};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::GixRepo;

impl GixRepo {
    pub(super) fn list_worktree_files_impl(&self) -> Result<Vec<FileEntry>> {
        let repo = self._repo.to_thread_local();
        let index = repo
            .index_or_empty()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
        let options = repo
            .dirwalk_options()
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix file browser dirwalk options: {e}"
                )))
            })?
            // Tracked entries are hidden by default.
            .emit_tracked(true)
            // `CollapseDirectory` would fold a wholly-untracked folder into one row
            // and hide its contents.
            .emit_untracked(gix::dir::walk::EmissionMode::Matching)
            .emit_ignored(None)
            .emit_pruned(false)
            .emit_empty_directories(true)
            .recurse_repositories(false);

        let mut delegate = CollectWorktreePaths::default();
        repo.dirwalk(
            &index,
            Vec::<gix::bstr::BString>::new(),
            &AtomicBool::default(),
            options,
            &mut delegate,
        )
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix dirwalk: {e}"))))?;

        Ok(flatten_worktree_paths(delegate.paths))
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

/// `can_recurse` is deliberately left at its default: it already declines to
/// descend into ignored directories and into nested repositories, which is the
/// pruning this listing depends on.
#[derive(Default)]
struct CollectWorktreePaths {
    paths: Vec<(String, bool)>,
}

impl gix::dir::walk::Delegate for CollectWorktreePaths {
    fn emit(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        _collapsed_directory_status: Option<gix::dir::entry::Status>,
    ) -> gix::dir::walk::Action {
        use gix::dir::entry::Kind;

        let is_directory = match entry.disk_kind {
            // `Repository` is a submodule: a folder we never descend into.
            Some(Kind::Directory | Kind::Repository) => true,
            Some(Kind::File | Kind::Symlink) => false,
            // FIFOs, sockets and devices cannot be tracked or opened.
            Some(Kind::Untrackable) | None => return gix::dir::walk::Action::Continue(()),
        };

        let path = entry.rela_path.to_string();
        if !path.is_empty() {
            self.paths.push((path, is_directory));
        }
        gix::dir::walk::Action::Continue(())
    }
}

#[derive(Default)]
struct WorktreeDir {
    dirs: BTreeMap<String, WorktreeDir>,
    files: BTreeSet<String>,
}

/// The walk emits leaves only, in readdir order, so intermediate directories are
/// synthesized from path components and the result re-sorted to match
/// [`collect_tree_entries`]. Both sources must agree on ordering and path form,
/// or expansion state and reveal-by-path stop matching when switching between the
/// live tree and a commit's tree.
fn flatten_worktree_paths(paths: Vec<(String, bool)>) -> Vec<FileEntry> {
    let mut root = WorktreeDir::default();

    for (path, is_directory) in paths {
        let mut node = &mut root;
        let mut components = path.split('/').filter(|c| !c.is_empty()).peekable();
        while let Some(component) = components.next() {
            if components.peek().is_some() || is_directory {
                node = node.dirs.entry(component.to_string()).or_default();
            } else {
                node.files.insert(component.to_string());
            }
        }
    }

    let mut out = Vec::new();
    flatten_worktree_dir(&root, String::new(), 0, &mut out);
    out
}

fn flatten_worktree_dir(
    dir: &WorktreeDir,
    parent_path: String,
    depth: usize,
    out: &mut Vec<FileEntry>,
) {
    let join = |name: &str| {
        if parent_path.is_empty() {
            name.to_string()
        } else {
            format!("{parent_path}/{name}")
        }
    };

    for (name, child) in &dir.dirs {
        let path = join(name);
        out.push(FileEntry {
            name: name.clone(),
            path: Arc::new(PathBuf::from(&path)),
            kind: FileEntryKind::Directory,
            depth,
        });
        flatten_worktree_dir(child, path, depth + 1, out);
    }

    for name in &dir.files {
        out.push(FileEntry {
            name: name.clone(),
            path: Arc::new(PathBuf::from(join(name))),
            kind: FileEntryKind::File,
            depth,
        });
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
    use gitcomet_core::domain::{CommitId, FileEntry, FileEntryKind};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn paths_of(entries: &[FileEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect()
    }

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
    fn list_worktree_files_returns_flat_files_correctly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "README.md", "# Project", "first");
        commit_file(workdir, "main.rs", "fn main() {}", "second");

        let repo = open_repo(workdir);
        let entries = repo.list_worktree_files_impl().expect("list tree files");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "README.md");
        assert_eq!(entries[0].kind, FileEntryKind::File);
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[1].name, "main.rs");
        assert_eq!(entries[1].kind, FileEntryKind::File);
        assert_eq!(entries[1].depth, 0);
    }

    #[test]
    fn list_worktree_files_sorts_directories_before_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "z_file.txt", "z", "first");
        commit_file(workdir, "a_dir/nested.txt", "nested", "second");

        let repo = open_repo(workdir);
        let entries = repo.list_worktree_files_impl().expect("list tree files");

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
    fn list_worktree_files_nested_directories_have_correct_depth() {
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
        let entries = repo.list_worktree_files_impl().expect("list tree files");

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

    /// Expansion state and reveal-by-path are keyed by path across both sources,
    /// so a divergence here collapses the tree when browsing a commit and back.
    #[test]
    fn worktree_and_commit_listings_agree_on_a_clean_tree() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "alpha.txt", "alpha", "first commit");
        commit_file(workdir, "beta.txt", "beta", "second commit");
        commit_file(workdir, "src/nested/deep.txt", "deep", "third commit");

        let repo = open_repo(workdir);

        let head_entries = repo.list_worktree_files_impl().expect("head tree");

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

    /// The reported bug: the listing came from `HEAD`'s tree, so a new folder of
    /// untracked files was invisible.
    #[test]
    fn list_worktree_files_includes_untracked_files_in_a_new_folder() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "committed.txt", "committed", "first");
        write_file(workdir, "newfolder/a.txt", "a");
        write_file(workdir, "newfolder/b.txt", "b");
        write_file(workdir, "newfolder/c.txt", "c");

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(
            paths_of(&entries),
            vec![
                "newfolder",
                "newfolder/a.txt",
                "newfolder/b.txt",
                "newfolder/c.txt",
                "committed.txt",
            ]
        );
        assert_eq!(entries[0].kind, FileEntryKind::Directory);
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[1].kind, FileEntryKind::File);
        assert_eq!(entries[1].depth, 1);
    }

    #[test]
    fn list_worktree_files_includes_staged_but_uncommitted_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "committed.txt", "committed", "first");
        write_file(workdir, "staged/new.txt", "staged");
        git_success(workdir, &["add", "staged/new.txt"]);

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(
            paths_of(&entries),
            vec!["staged", "staged/new.txt", "committed.txt"]
        );
    }

    #[test]
    fn list_worktree_files_omits_files_deleted_from_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "kept.txt", "kept", "first");
        commit_file(workdir, "removed.txt", "removed", "second");
        fs::remove_file(workdir.join("removed.txt")).expect("remove file");

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(paths_of(&entries), vec!["kept.txt"]);
    }

    #[test]
    fn list_worktree_files_omits_ignored_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, ".gitignore", "target/\n*.log\n", "add ignores");
        write_file(workdir, "target/debug/huge.bin", "binary");
        write_file(workdir, "noisy.log", "log");
        write_file(workdir, "kept.txt", "kept");

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(paths_of(&entries), vec![".gitignore", "kept.txt"]);
    }

    #[test]
    fn list_worktree_files_includes_empty_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        commit_file(workdir, "file.txt", "file", "first");
        fs::create_dir_all(workdir.join("empty_dir")).expect("create dir");

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(paths_of(&entries), vec!["empty_dir", "file.txt"]);
        assert_eq!(entries[0].kind, FileEntryKind::Directory);
    }

    /// An unborn HEAD used to fail `head_commit()`, so every freshly-initialized
    /// repository showed "Error loading files."
    #[test]
    fn list_worktree_files_works_without_any_commit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "src/main.rs", "fn main() {}");

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(paths_of(&entries), vec!["src", "src/main.rs"]);
    }

    #[test]
    fn list_worktree_files_sorts_directories_first_then_alphabetically() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "z_file.txt", "z");
        write_file(workdir, "a_file.txt", "a");
        write_file(workdir, "z_dir/inner.txt", "inner");
        write_file(workdir, "a_dir/inner.txt", "inner");

        let repo = open_repo(workdir);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        assert_eq!(
            paths_of(&entries),
            vec![
                "a_dir",
                "a_dir/inner.txt",
                "z_dir",
                "z_dir/inner.txt",
                "a_file.txt",
                "z_file.txt",
            ]
        );
    }

    /// The tree walk drops gitlinks outright (mode `160000` is neither tree, blob,
    /// nor link), so submodules used to be missing entirely.
    #[test]
    fn list_worktree_files_lists_a_submodule_as_a_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let outer = tmp.path().join("outer");
        let inner = tmp.path().join("inner");
        fs::create_dir_all(&outer).expect("create outer");
        fs::create_dir_all(&inner).expect("create inner");

        init_test_repo(&inner);
        commit_file(&inner, "inner.txt", "inner", "inner commit");

        init_test_repo(&outer);
        commit_file(&outer, "outer.txt", "outer", "outer commit");
        git_success(
            &outer,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                inner.to_str().expect("utf8 path"),
                "sub",
            ],
        );

        let repo = open_repo(&outer);
        let entries = repo
            .list_worktree_files_impl()
            .expect("list worktree files");

        let sub = entries
            .iter()
            .find(|e| e.name == "sub")
            .expect("submodule listed");
        assert_eq!(sub.kind, FileEntryKind::Directory);
        // `Path::starts_with` compares whole components, so the `sub` row itself
        // matches `sub/` and has to be excluded by depth.
        assert!(
            !entries
                .iter()
                .any(|e| e.depth > 0 && e.path.starts_with("sub")),
            "submodule contents leaked into the parent listing: {:?}",
            paths_of(&entries)
        );
    }
}
