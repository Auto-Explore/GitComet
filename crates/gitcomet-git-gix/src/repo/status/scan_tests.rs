use super::SHARING;
use crate::test_support::{git_stdout, git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::domain::FileStatus;
use std::path::Path;

/// The unstaged half of a status scanned by `scanners` threads, which share
/// a directory holding `files` tracked files or more.
fn unstaged_with(root: &Path, scanners: usize, files: usize) -> Vec<FileStatus> {
    SHARING.set(Some((scanners, files)));
    let status = open_repo(root).status_impl();
    SHARING.set(None);
    std::sync::Arc::unwrap_or_clone(status.unwrap().unstaged)
}

/// A small repeatable source of choices.
struct Choices(u64);

impl Choices {
    fn below(&mut self, limit: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % limit
    }
}

/// A tree of nested directories holding tracked, untracked and ignored
/// files, with ignore rules at several depths that reach into directories
/// below them.
fn scattered_tree(root: &Path, seed: u64) {
    let mut choices = Choices(seed);
    init_test_repo(root);
    write_file(root, ".gitignore", "*.log\n/a/build/\nscratch/\n");
    let names = ["a", "b", "c", "build", "scratch"];
    let mut untracked = Vec::new();
    for file in 0..240 {
        let depth = 1 + choices.below(4);
        let directory: Vec<&str> = (0..depth)
            .map(|_| names[choices.below(names.len() as u64) as usize])
            .collect();
        let directory = directory.join("/");
        let (name, tracked) = match choices.below(8) {
            0..=3 => (format!("tracked{file}.txt"), true),
            4 | 5 => (format!("new{file}.txt"), false),
            6 => (format!("noise{file}.log"), false),
            _ => (format!("kept{file}.log"), false),
        };
        let path = format!("{directory}/{name}");
        write_file(root, &path, "content\n");
        if tracked {
            // Under an ignored directory too: a tracked file stays tracked.
            git_success(root, &["add", "-f", "--", &path]);
        } else {
            untracked.push(path);
        }
        if choices.below(12) == 0 {
            let rules = match choices.below(3) {
                0 => "!kept*.log\n",
                1 => "new*\n",
                _ => "/c/\n",
            };
            write_file(root, &format!("{directory}/.gitignore"), rules);
        }
    }
    git_success(root, &["commit", "-qm", "seed"]);
    // What a scan must leave whole: a repository nobody tracks, an empty
    // directory, and tracked files changed and gone.
    std::fs::create_dir_all(root.join("a/b/nested")).unwrap();
    init_test_repo(&root.join("a/b/nested"));
    write_file(root, "a/b/nested/inside.txt", "inside\n");
    std::fs::create_dir_all(root.join("b/empty/deeper")).unwrap();
    let tracked = git_stdout(root, &["ls-files"]);
    for (number, path) in tracked.lines().enumerate() {
        match number % 9 {
            0 => write_file(root, path, "changed\n"),
            1 => std::fs::remove_file(root.join(path)).unwrap(),
            _ => {}
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("tracked-elsewhere", root.join("a/link")).unwrap();
        std::os::unix::fs::symlink("b", root.join("linked-directory")).unwrap();
    }
}

#[test]
fn directories_scanned_apart_report_what_one_scan_reports() {
    for seed in [1, 2, 3] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        scattered_tree(root, seed);
        let alone = unstaged_with(root, 1, usize::MAX);
        assert!(alone.len() > 60, "seed {seed}: {}", alone.len());
        // Every tracked directory shared, then only the larger ones.
        for (scanners, files) in [(2, 1), (8, 1), (3, 4), (8, 12)] {
            assert_eq!(
                unstaged_with(root, scanners, files),
                alone,
                "seed {seed}, {scanners} scanners sharing from {files} files"
            );
        }

        // Git names the same untracked files, a nested repository as its
        // directory.
        let others = git_stdout(
            root,
            &["ls-files", "--others", "--exclude-standard", "--full-name"],
        );
        let mut expected: Vec<_> = others
            .lines()
            .map(|path| std::path::PathBuf::from(path.trim_end_matches('/')))
            .collect();
        expected.sort();
        let mut untracked: Vec<_> = alone
            .iter()
            .filter(|entry| entry.kind == gitcomet_core::domain::FileStatusKind::Untracked)
            .map(|entry| entry.path.clone())
            .collect();
        untracked.sort();
        assert_eq!(untracked, expected, "seed {seed}");
    }
}

/// A scan that fails ends the others, which wait for it to add directories.
#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_read_fails_the_status_at_any_sharing() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    scattered_tree(root, 4);
    let locked = root.join("a/locked");
    write_file(root, "a/locked/tracked.txt", "content\n");
    git_success(root, &["add", "a/locked/tracked.txt"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Permissions do not bind a privileged user; nothing can be shown then.
    if std::fs::read_dir(&locked).is_err() {
        for (scanners, files) in [(1, usize::MAX), (8, 1)] {
            SHARING.set(Some((scanners, files)));
            let status = open_repo(root).status_impl();
            SHARING.set(None);
            assert!(status.is_err(), "{scanners} scanners");
        }
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
}
