use gitgpui_core::conflict_session::{ConflictPayload, ConflictResolverStrategy};
use gitgpui_core::domain::{DiffArea, DiffTarget, FileConflictKind, FileStatusKind};
use gitgpui_core::services::ConflictSide;
use gitgpui_core::services::GitBackend;
use gitgpui_git_gix::GixBackend;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn run_git_expect_failure(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(!status.success(), "expected git {:?} to fail", args);
}

fn write(repo: &Path, rel: &str, contents: &str) -> PathBuf {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

fn write_bytes(repo: &Path, rel: &str, contents: &[u8]) -> PathBuf {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

fn hash_blob(repo: &Path, contents: &[u8]) -> String {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git hash-object to run");

    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(contents)
        .expect("write blob contents");

    let output = child.wait_with_output().expect("wait for hash-object");
    assert!(
        output.status.success(),
        "git hash-object failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("hash-object stdout utf8")
        .trim()
        .to_owned()
}

fn set_unmerged_stages(
    repo: &Path,
    path: &str,
    base_blob: Option<&str>,
    ours_blob: Option<&str>,
    theirs_blob: Option<&str>,
) {
    run_git(repo, &["update-index", "--force-remove", "--", path]);
    let _ = fs::remove_file(repo.join(path));

    let mut index_info = String::new();
    if let Some(blob) = base_blob {
        index_info.push_str(&format!("100644 {blob} 1\t{path}\n"));
    }
    if let Some(blob) = ours_blob {
        index_info.push_str(&format!("100644 {blob} 2\t{path}\n"));
    }
    if let Some(blob) = theirs_blob {
        index_info.push_str(&format!("100644 {blob} 3\t{path}\n"));
    }

    if index_info.is_empty() {
        return;
    }

    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git update-index --index-info to run");

    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(index_info.as_bytes())
        .expect("write index-info");

    let output = child.wait_with_output().expect("wait for update-index");
    assert!(
        output.status.success(),
        "git update-index --index-info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn png_1x1_rgba(r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
    fn push_be_u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320u32 & mask);
            }
        }
        !crc
    }

    fn adler32(bytes: &[u8]) -> u32 {
        const MOD: u32 = 65521;
        let mut a = 1u32;
        let mut b = 0u32;
        for &byte in bytes {
            a = (a + byte as u32) % MOD;
            b = (b + a) % MOD;
        }
        (b << 16) | a
    }

    let raw = [0u8, r, g, b, a];
    let len = raw.len() as u16;
    let nlen = !len;

    let mut zlib = Vec::new();
    zlib.push(0x78);
    zlib.push(0x01);
    zlib.push(0x01);
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&nlen.to_le_bytes());
    zlib.extend_from_slice(&raw);
    push_be_u32(&mut zlib, adler32(&raw));

    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = Vec::new();
    push_be_u32(&mut ihdr, 1);
    push_be_u32(&mut ihdr, 1);
    ihdr.push(8);
    ihdr.push(6);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);
    push_be_u32(&mut out, ihdr.len() as u32);
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&ihdr);
    push_be_u32(&mut out, crc32(&[b"IHDR".as_slice(), &ihdr].concat()));

    push_be_u32(&mut out, zlib.len() as u32);
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&zlib);
    push_be_u32(&mut out, crc32(&[b"IDAT".as_slice(), &zlib].concat()));

    push_be_u32(&mut out, 0);
    out.extend_from_slice(b"IEND");
    push_be_u32(&mut out, crc32(b"IEND"));

    out
}

#[derive(Clone, Copy)]
struct ConflictStageFixture {
    path: &'static str,
    kind: FileConflictKind,
    has_base: bool,
    has_ours: bool,
    has_theirs: bool,
}

#[test]
fn status_separates_staged_and_unstaged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);
    write(repo, "b.txt", "untracked\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.staged[0].kind, FileStatusKind::Modified);

    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("b.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Untracked);
}

#[test]
fn status_lists_untracked_files_in_directories() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);

    write(repo, "dir/a.txt", "one\n");
    write(repo, "dir/b.txt", "two\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    assert_eq!(status.unstaged.len(), 2);
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("dir/a.txt") && e.kind == FileStatusKind::Untracked)
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("dir/b.txt") && e.kind == FileStatusKind::Untracked)
    );
}

#[test]
fn diff_unified_works_for_staged_and_unstaged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert!(unstaged.contains("@@"));

    run_git(repo, &["add", "a.txt"]);

    let staged = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert!(staged.contains("@@"));
}

#[test]
fn diff_file_text_reports_old_and_new_for_working_tree_and_commits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("file diff for unstaged changes");
    assert_eq!(unstaged.path, PathBuf::from("a.txt"));
    assert_eq!(unstaged.old.as_deref(), Some("one\n"));
    assert_eq!(unstaged.new.as_deref(), Some("one\ntwo\n"));

    run_git(repo, &["add", "a.txt"]);

    let staged = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .expect("file diff for staged changes");
    assert_eq!(staged.old.as_deref(), Some("one\n"));
    assert_eq!(staged.new.as_deref(), Some("one\ntwo\n"));

    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse to run");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();

    let commit = opened
        .diff_file_text(&DiffTarget::Commit {
            commit_id: gitgpui_core::domain::CommitId(head),
            path: Some(PathBuf::from("a.txt")),
        })
        .unwrap()
        .expect("file diff for commit");
    assert_eq!(commit.old.as_deref(), Some("one\n"));
    assert_eq!(commit.new.as_deref(), Some("one\ntwo\n"));
}

#[test]
fn diff_file_text_staged_add_and_delete_report_missing_sides() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    // Stage a new file (missing on HEAD) and delete the initial file (missing on disk + index).
    write(repo, "b.txt", "new\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["rm", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let added = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("b.txt"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .expect("file diff for staged added file");
    assert_eq!(added.path, PathBuf::from("b.txt"));
    assert_eq!(added.old.as_deref(), None);
    assert_eq!(added.new.as_deref(), Some("new\n"));

    let deleted = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .expect("file diff for staged deleted file");
    assert_eq!(deleted.path, PathBuf::from("a.txt"));
    assert_eq!(deleted.old.as_deref(), Some("one\n"));
    assert_eq!(deleted.new.as_deref(), None);
}

#[test]
fn diff_file_text_returns_none_for_directories() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    write(repo, "dir/a.txt", "one\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let result = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("dir"),
            area: DiffArea::Unstaged,
        })
        .unwrap();

    assert!(result.is_none());
}

#[test]
fn diff_file_image_reports_old_and_new_for_working_tree_and_commits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let old_png = png_1x1_rgba(0, 0, 0, 255);
    let new_png = png_1x1_rgba(255, 0, 0, 255);

    write_bytes(repo, "img.png", &old_png);
    run_git(repo, &["add", "img.png"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write_bytes(repo, "img.png", &new_png);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: PathBuf::from("img.png"),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("image diff for unstaged changes");
    assert_eq!(unstaged.path, PathBuf::from("img.png"));
    assert_eq!(unstaged.old.as_deref(), Some(old_png.as_slice()));
    assert_eq!(unstaged.new.as_deref(), Some(new_png.as_slice()));

    run_git(repo, &["add", "img.png"]);

    let staged = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: PathBuf::from("img.png"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .expect("image diff for staged changes");
    assert_eq!(staged.old.as_deref(), Some(old_png.as_slice()));
    assert_eq!(staged.new.as_deref(), Some(new_png.as_slice()));

    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse to run");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();

    let commit = opened
        .diff_file_image(&DiffTarget::Commit {
            commit_id: gitgpui_core::domain::CommitId(head),
            path: Some(PathBuf::from("img.png")),
        })
        .unwrap()
        .expect("image diff for commit");
    assert_eq!(commit.old.as_deref(), Some(old_png.as_slice()));
    assert_eq!(commit.new.as_deref(), Some(new_png.as_slice()));
}

#[test]
fn diff_file_image_returns_none_for_directories() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    write(repo, "dir/a.png", "not really a png\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let result = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: PathBuf::from("dir"),
            area: DiffArea::Unstaged,
        })
        .unwrap();

    assert!(result.is_none());
}

#[test]
fn diff_file_text_uses_ours_and_theirs_for_conflicted_paths() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "ours\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Conflicted);
    assert_eq!(
        status.unstaged[0].conflict,
        Some(FileConflictKind::BothModified)
    );

    let diff = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("file diff for conflicted changes");
    assert_eq!(diff.old.as_deref(), Some("ours\n"));
    assert_eq!(diff.new.as_deref(), Some("theirs\n"));

    let session = opened
        .conflict_session(Path::new("a.txt"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.conflict_kind, FileConflictKind::BothModified);
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);
    assert_eq!(session.regions[0].ours, "ours\n");
    assert_eq!(session.regions[0].theirs, "theirs\n");
}

#[test]
fn status_and_conflict_stages_cover_all_conflict_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let base_blob = hash_blob(repo, b"base\n");
    let ours_blob = hash_blob(repo, b"ours\n");
    let theirs_blob = hash_blob(repo, b"theirs\n");

    let fixtures = [
        ConflictStageFixture {
            path: "dd.txt",
            kind: FileConflictKind::BothDeleted,
            has_base: true,
            has_ours: false,
            has_theirs: false,
        },
        ConflictStageFixture {
            path: "au.txt",
            kind: FileConflictKind::AddedByUs,
            has_base: false,
            has_ours: true,
            has_theirs: false,
        },
        ConflictStageFixture {
            path: "ud.txt",
            kind: FileConflictKind::DeletedByThem,
            has_base: true,
            has_ours: true,
            has_theirs: false,
        },
        ConflictStageFixture {
            path: "ua.txt",
            kind: FileConflictKind::AddedByThem,
            has_base: false,
            has_ours: false,
            has_theirs: true,
        },
        ConflictStageFixture {
            path: "du.txt",
            kind: FileConflictKind::DeletedByUs,
            has_base: true,
            has_ours: false,
            has_theirs: true,
        },
        ConflictStageFixture {
            path: "aa.txt",
            kind: FileConflictKind::BothAdded,
            has_base: false,
            has_ours: true,
            has_theirs: true,
        },
        ConflictStageFixture {
            path: "uu.txt",
            kind: FileConflictKind::BothModified,
            has_base: true,
            has_ours: true,
            has_theirs: true,
        },
    ];

    for fixture in &fixtures {
        set_unmerged_stages(
            repo,
            fixture.path,
            fixture.has_base.then_some(base_blob.as_str()),
            fixture.has_ours.then_some(ours_blob.as_str()),
            fixture.has_theirs.then_some(theirs_blob.as_str()),
        );
    }

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    for fixture in &fixtures {
        let path = Path::new(fixture.path);
        let status_entry = status
            .unstaged
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("missing status entry for {}", fixture.path));
        assert_eq!(
            status_entry.kind,
            FileStatusKind::Conflicted,
            "expected conflicted kind for {}",
            fixture.path
        );
        assert_eq!(
            status_entry.conflict,
            Some(fixture.kind),
            "wrong conflict kind for {}",
            fixture.path
        );

        assert!(
            !status.staged.iter().any(|e| e.path == path),
            "conflicted path {} should not appear in staged status",
            fixture.path
        );

        let stages = opened
            .conflict_file_stages(path)
            .unwrap()
            .expect("conflict stages");
        assert_eq!(
            stages.base.is_some(),
            fixture.has_base,
            "base stage mismatch for {}",
            fixture.path
        );
        assert_eq!(
            stages.ours.is_some(),
            fixture.has_ours,
            "ours stage mismatch for {}",
            fixture.path
        );
        assert_eq!(
            stages.theirs.is_some(),
            fixture.has_theirs,
            "theirs stage mismatch for {}",
            fixture.path
        );

        let session = opened
            .conflict_session(path)
            .unwrap()
            .expect("conflict session");
        assert_eq!(session.path, PathBuf::from(fixture.path));
        assert_eq!(session.conflict_kind, fixture.kind);
        assert_eq!(
            session.strategy,
            ConflictResolverStrategy::for_conflict(fixture.kind, false)
        );
        assert_eq!(session.base.is_absent(), !fixture.has_base);
        assert_eq!(session.ours.is_absent(), !fixture.has_ours);
        assert_eq!(session.theirs.is_absent(), !fixture.has_theirs);
    }
}

#[test]
fn checkout_conflict_side_resolves_all_conflict_stage_shapes() {
    #[derive(Clone, Copy)]
    struct ConflictCheckoutFixture {
        kind: FileConflictKind,
        has_base: bool,
        has_ours: bool,
        has_theirs: bool,
    }

    let fixtures = [
        ConflictCheckoutFixture {
            kind: FileConflictKind::BothDeleted,
            has_base: true,
            has_ours: false,
            has_theirs: false,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::AddedByUs,
            has_base: false,
            has_ours: true,
            has_theirs: false,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::DeletedByThem,
            has_base: true,
            has_ours: true,
            has_theirs: false,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::AddedByThem,
            has_base: false,
            has_ours: false,
            has_theirs: true,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::DeletedByUs,
            has_base: true,
            has_ours: false,
            has_theirs: true,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::BothAdded,
            has_base: false,
            has_ours: true,
            has_theirs: true,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::BothModified,
            has_base: true,
            has_ours: true,
            has_theirs: true,
        },
    ];

    for fixture in fixtures {
        for side in [ConflictSide::Ours, ConflictSide::Theirs] {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path();

            run_git(repo, &["init"]);
            run_git(repo, &["config", "user.email", "you@example.com"]);
            run_git(repo, &["config", "user.name", "You"]);
            run_git(repo, &["config", "commit.gpgsign", "false"]);

            write(repo, "seed.txt", "seed\n");
            run_git(repo, &["add", "seed.txt"]);
            run_git(
                repo,
                &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
            );

            let base_blob = hash_blob(repo, b"base\n");
            let ours_blob = hash_blob(repo, b"ours\n");
            let theirs_blob = hash_blob(repo, b"theirs\n");

            set_unmerged_stages(
                repo,
                "a.txt",
                fixture.has_base.then_some(base_blob.as_str()),
                fixture.has_ours.then_some(ours_blob.as_str()),
                fixture.has_theirs.then_some(theirs_blob.as_str()),
            );

            let backend = GixBackend;
            let opened = backend.open(repo).unwrap();

            let before = opened.status().unwrap();
            let conflict_entry = before
                .unstaged
                .iter()
                .find(|e| e.path == Path::new("a.txt"))
                .expect("expected staged-shape fixture to appear as conflict");
            assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
            assert_eq!(conflict_entry.conflict, Some(fixture.kind));

            opened
                .checkout_conflict_side(Path::new("a.txt"), side)
                .unwrap();

            let after = opened.status().unwrap();
            let selected_stage_exists = match side {
                ConflictSide::Ours => fixture.has_ours,
                ConflictSide::Theirs => fixture.has_theirs,
            };

            if selected_stage_exists {
                let expected_bytes: &[u8] = match side {
                    ConflictSide::Ours => b"ours\n",
                    ConflictSide::Theirs => b"theirs\n",
                };
                assert_eq!(fs::read(repo.join("a.txt")).unwrap(), expected_bytes);
                assert!(
                    after
                        .staged
                        .iter()
                        .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Added),
                    "expected selected side to stage added file for {:?} with {:?}; status={after:?}",
                    fixture.kind,
                    side
                );
                assert!(
                    after.unstaged.iter().all(|e| e.path != Path::new("a.txt")),
                    "expected conflict path to disappear from unstaged after resolving {:?} with {:?}; status={after:?}",
                    fixture.kind,
                    side
                );
            } else {
                assert!(
                    !repo.join("a.txt").exists(),
                    "expected path to be removed when chosen stage is missing for {:?} with {:?}",
                    fixture.kind,
                    side
                );
                assert!(
                    after
                        .staged
                        .iter()
                        .chain(after.unstaged.iter())
                        .all(|e| e.path != Path::new("a.txt")),
                    "expected no status entry for removed path after resolving {:?} with {:?}; status={after:?}",
                    fixture.kind,
                    side
                );
            }
        }
    }
}

#[test]
fn status_reports_single_conflict_for_modify_delete() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    let entries = status
        .unstaged
        .iter()
        .filter(|e| e.path == Path::new("a.txt"))
        .collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one status entry for a.txt, got {:#?}",
        status.unstaged
    );
    assert_eq!(entries[0].kind, FileStatusKind::Conflicted);
    assert_eq!(entries[0].conflict, Some(FileConflictKind::DeletedByUs));
}

#[test]
fn status_reports_conflict_kind_for_add_add() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "base.txt", "base\n");
    run_git(repo, &["add", "base.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs_add"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "ours\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_add"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Conflicted);
    assert_eq!(
        status.unstaged[0].conflict,
        Some(FileConflictKind::BothAdded)
    );
}

#[test]
fn conflict_file_stages_preserve_non_utf8_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let base_bytes = b"\x00base\xff\n".to_vec();
    let ours_bytes = b"\x00ours\xff\n".to_vec();
    let theirs_bytes = b"\x00theirs\xff\n".to_vec();

    write_bytes(repo, "bin.dat", &base_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write_bytes(repo, "bin.dat", &theirs_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write_bytes(repo, "bin.dat", &ours_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let stages = opened
        .conflict_file_stages(Path::new("bin.dat"))
        .unwrap()
        .expect("conflict stage data");

    assert_eq!(stages.path, PathBuf::from("bin.dat"));
    assert_eq!(stages.base_bytes.as_deref(), Some(base_bytes.as_slice()));
    assert_eq!(stages.ours_bytes.as_deref(), Some(ours_bytes.as_slice()));
    assert_eq!(
        stages.theirs_bytes.as_deref(),
        Some(theirs_bytes.as_slice())
    );
    assert_eq!(stages.base, None);
    assert_eq!(stages.ours, None);
    assert_eq!(stages.theirs, None);

    let session = opened
        .conflict_session(Path::new("bin.dat"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.path, PathBuf::from("bin.dat"));
    assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);
    assert_eq!(session.total_regions(), 0);
    assert!(matches!(session.base, ConflictPayload::Binary(_)));
    assert!(matches!(session.ours, ConflictPayload::Binary(_)));
    assert!(matches!(session.theirs, ConflictPayload::Binary(_)));
}

#[test]
fn diff_file_text_handles_modify_delete_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let diff = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("file diff for conflicted changes");
    assert_eq!(diff.old, None);
    assert_eq!(diff.new.as_deref(), Some("theirs\n"));
}

#[test]
fn checkout_conflict_side_resolves_modify_delete_using_ours() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Ours)
        .unwrap();

    assert!(
        !repo.join("a.txt").exists(),
        "expected ours resolution to remove file from worktree"
    );
    let status = opened.status().unwrap();
    assert!(
        !status
            .staged
            .iter()
            .chain(status.unstaged.iter())
            .any(|e| e.path == Path::new("a.txt")),
        "expected ours resolution to clear status entries for a.txt, got {status:?}"
    );
}

#[test]
fn checkout_conflict_side_resolves_modify_delete_using_theirs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Theirs)
        .unwrap();

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "theirs\n",
        "expected theirs resolution to restore file contents"
    );
    let status = opened.status().unwrap();
    assert_eq!(
        status.unstaged,
        Vec::new(),
        "expected theirs resolution to clear unstaged entries"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Added),
        "expected theirs resolution to stage file as added, got {status:?}"
    );
}

#[test]
fn checkout_conflict_side_stages_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "ours\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Theirs)
        .unwrap();

    let status = opened.status().unwrap();
    assert!(status.unstaged.iter().all(|s| s.path != Path::new("a.txt")));
    assert!(
        status
            .staged
            .iter()
            .any(|s| s.path == Path::new("a.txt") && s.kind == FileStatusKind::Modified)
    );

    let on_disk = fs::read_to_string(repo.join("a.txt")).unwrap();
    assert_eq!(on_disk, "theirs\n");
}

#[test]
fn stage_and_unstage_paths_update_status() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    write(repo, "b.txt", "untracked\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.stage(&[Path::new("a.txt")]).unwrap();
    let status = opened.status().unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.staged[0].kind, FileStatusKind::Modified);
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("b.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Untracked);

    opened.unstage(&[Path::new("a.txt")]).unwrap();
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert_eq!(status.unstaged.len(), 2);
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Modified)
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("b.txt") && e.kind == FileStatusKind::Untracked)
    );
}

#[test]
fn commit_creates_new_commit_and_cleans_status() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.commit("second").unwrap();

    let msg = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .expect("git log to run");
    assert!(msg.status.success());
    assert_eq!(String::from_utf8(msg.stdout).unwrap().trim(), "second");

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn reset_soft_moves_head_and_leaves_changes_staged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);
    let c1 = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c1");
    assert!(c1.status.success());
    let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .reset_with_output("HEAD~1", gitgpui_core::services::ResetMode::Soft)
        .unwrap();

    let head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), c1);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");

    let status = opened.status().unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.staged[0].kind, FileStatusKind::Modified);
    assert!(status.unstaged.is_empty());
}

#[test]
fn reset_mixed_moves_head_and_leaves_changes_unstaged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);
    let c1 = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c1");
    assert!(c1.status.success());
    let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .reset_with_output("HEAD~1", gitgpui_core::services::ResetMode::Mixed)
        .unwrap();

    let head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), c1);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Modified);
}

#[test]
fn reset_hard_moves_head_and_discards_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);
    let c1 = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c1");
    assert!(c1.status.success());
    let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    write(repo, "a.txt", "two-modified\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .reset_with_output("HEAD~1", gitgpui_core::services::ResetMode::Hard)
        .unwrap();

    let head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), c1);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn revert_commit_creates_new_commit_and_reverts_content() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    let c2 = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c2");
    assert!(c2.status.success());
    let c2 = String::from_utf8(c2.stdout).unwrap().trim().to_string();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .revert(&gitgpui_core::domain::CommitId(c2.clone()))
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());

    let head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();
    assert_ne!(head, c2, "expected revert to create a new commit");
}

#[test]
fn amend_rewrites_head_commit_message_and_content() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let head_before = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head_before.status.success());
    let head_before = String::from_utf8(head_before.stdout)
        .unwrap()
        .trim()
        .to_string();

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.commit_amend("amended").unwrap();

    let head_after = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head_after.status.success());
    let head_after = String::from_utf8(head_after.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_ne!(head_after, head_before);

    let count = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("rev-list --count");
    assert!(count.status.success());
    assert_eq!(String::from_utf8(count.stdout).unwrap().trim(), "1");

    let msg = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .expect("git log to run");
    assert!(msg.status.success());
    assert_eq!(String::from_utf8(msg.stdout).unwrap().trim(), "amended");
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn merge_creates_merge_commit_when_branches_diverged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "c.txt", "main\n");
    run_git(repo, &["add", "c.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.merge_ref_with_output("feature").unwrap();

    let parents = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 2, "expected merge commit");

    assert!(repo.join("b.txt").exists());
    assert!(repo.join("c.txt").exists());
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    assert_eq!(fs::read_to_string(repo.join("c.txt")).unwrap(), "main\n");
}

#[test]
fn merge_fast_forwards_when_possible_even_if_merge_ff_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["config", "merge.ff", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.merge_ref_with_output("feature").unwrap();

    let parents = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 1, "expected fast-forward");

    let msg = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .expect("git log to run");
    assert!(msg.status.success());
    assert_eq!(String::from_utf8(msg.stdout).unwrap().trim(), "feature");
}

#[test]
fn merge_commit_message_is_available_during_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.merge_ref_with_output("feature").is_err());

    let msg = opened
        .merge_commit_message()
        .unwrap()
        .expect("merge commit message");
    assert_eq!(
        msg.lines().next().unwrap_or_default(),
        "Merge branch 'feature'"
    );
    assert!(
        !msg.contains('#'),
        "expected message to be cleaned, got: {msg}"
    );

    run_git(repo, &["merge", "--abort"]);
    assert!(opened.merge_commit_message().unwrap().is_none());
}

#[test]
fn rebase_replays_commits_onto_target_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "c.txt", "main\n");
    run_git(repo, &["add", "c.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );
    let master_head = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse master");
    assert!(master_head.status.success());
    let master_head = String::from_utf8(master_head.stdout)
        .unwrap()
        .trim()
        .to_string();

    run_git(repo, &["checkout", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.rebase_with_output("master").unwrap();

    let parent = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD^"])
        .output()
        .expect("rev-parse parent");
    assert!(parent.status.success());
    assert_eq!(
        String::from_utf8(parent.stdout).unwrap().trim(),
        master_head
    );

    assert!(repo.join("b.txt").exists());
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn create_and_delete_local_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .create_branch(
            "feature",
            &gitgpui_core::domain::CommitId("HEAD".to_string()),
        )
        .unwrap();
    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );

    opened.delete_branch("feature").unwrap();
    let deleted = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected branch to be deleted");
}

#[test]
fn create_and_delete_local_tag() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.create_tag_with_output("v1.0.0", "HEAD").unwrap();
    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );

    opened.delete_tag_with_output("v1.0.0").unwrap();
    let deleted = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected tag to be deleted");
}

#[test]
fn list_remote_branches_includes_fetched_remote_tracking_refs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    fs::create_dir_all(&origin).unwrap();
    run_git(&origin, &["init", "--bare"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "master"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    write(&repo, "b.txt", "feature\n");
    run_git(&repo, &["add", "b.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&repo, &["push", "-u", "origin", "feature"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let branches = opened.list_remote_branches().unwrap();

    assert!(
        branches
            .iter()
            .any(|b| b.remote == "origin" && b.name == "master")
    );
    assert!(
        branches
            .iter()
            .any(|b| b.remote == "origin" && b.name == "feature")
    );
    assert!(!branches.iter().any(|b| b.name == "HEAD"));
}

#[test]
fn push_with_output_updates_remote_head() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "master"]);

    write(&repo, "a.txt", "one\ntwo\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let head_local = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(head_local.status.success());
    let head_local = String::from_utf8(head_local.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.push_with_output().unwrap();

    let head_remote = Command::new("git")
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/master"])
        .output()
        .expect("rev-parse origin/master");
    assert!(head_remote.status.success());
    let head_remote = String::from_utf8(head_remote.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(head_remote, head_local);
}

#[test]
fn force_push_with_output_updates_remote_head_after_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "master"]);

    write(&repo, "a.txt", "one\ntwo\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo, &["push"]);
    run_git(&repo, &["fetch", "origin"]);

    // Rewrite local history so it diverges from the remote.
    run_git(&repo, &["reset", "--hard", "HEAD~1"]);
    write(&repo, "a.txt", "one\ntwo (rewritten)\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "second rewritten",
        ],
    );
    let head_local = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(head_local.status.success());
    let head_local = String::from_utf8(head_local.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.push_force_with_output().unwrap();

    let head_remote = Command::new("git")
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/master"])
        .output()
        .expect("rev-parse refs/heads/master");
    assert!(head_remote.status.success());
    let head_remote = String::from_utf8(head_remote.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(head_remote, head_local);
}

#[test]
fn pull_with_output_fast_forwards_from_remote() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&repo_a).unwrap();

    run_git(&origin, &["init", "--bare"]);

    run_git(&repo_a, &["init"]);
    run_git(&repo_a, &["config", "user.email", "you@example.com"]);
    run_git(&repo_a, &["config", "user.name", "You"]);
    run_git(&repo_a, &["config", "commit.gpgsign", "false"]);
    write(&repo_a, "a.txt", "one\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &repo_a,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo_a, &["push", "-u", "origin", "master"]);

    run_git(
        dir.path(),
        &[
            "clone",
            origin.to_string_lossy().as_ref(),
            repo_b.to_string_lossy().as_ref(),
        ],
    );

    write(&repo_a, "a.txt", "one\ntwo\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo_a, &["push"]);

    let head_origin = Command::new("git")
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/master"])
        .output()
        .expect("rev-parse origin");
    assert!(head_origin.status.success());
    let head_origin = String::from_utf8(head_origin.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened_b = backend.open(&repo_b).unwrap();
    opened_b
        .pull_with_output(gitgpui_core::services::PullMode::FastForwardOnly)
        .unwrap();

    let head_b = Command::new("git")
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse b");
    assert!(head_b.status.success());
    let head_b = String::from_utf8(head_b.stdout).unwrap().trim().to_string();
    assert_eq!(head_b, head_origin);
}

#[test]
fn pull_with_output_fast_forwards_when_possible_even_if_pull_ff_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&repo_a).unwrap();

    run_git(&origin, &["init", "--bare"]);

    run_git(&repo_a, &["init"]);
    run_git(&repo_a, &["config", "user.email", "you@example.com"]);
    run_git(&repo_a, &["config", "user.name", "You"]);
    run_git(&repo_a, &["config", "commit.gpgsign", "false"]);
    write(&repo_a, "a.txt", "one\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &repo_a,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo_a, &["push", "-u", "origin", "master"]);

    run_git(
        dir.path(),
        &[
            "clone",
            origin.to_string_lossy().as_ref(),
            repo_b.to_string_lossy().as_ref(),
        ],
    );

    run_git(&repo_b, &["config", "user.email", "you@example.com"]);
    run_git(&repo_b, &["config", "user.name", "You"]);
    run_git(&repo_b, &["config", "commit.gpgsign", "false"]);
    run_git(&repo_b, &["config", "pull.ff", "false"]);

    write(&repo_a, "a.txt", "one\ntwo\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo_a, &["push"]);

    let head_origin = Command::new("git")
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/master"])
        .output()
        .expect("rev-parse origin");
    assert!(head_origin.status.success());
    let head_origin = String::from_utf8(head_origin.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened_b = backend.open(&repo_b).unwrap();
    opened_b
        .pull_with_output(gitgpui_core::services::PullMode::Merge)
        .unwrap();

    let head_b = Command::new("git")
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse b");
    assert!(head_b.status.success());
    let head_b = String::from_utf8(head_b.stdout).unwrap().trim().to_string();
    assert_eq!(head_b, head_origin);

    let parents = Command::new("git")
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 1, "expected fast-forward");
}

#[test]
fn stash_create_list_apply_and_drop_work() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.stash_create("wip", false).unwrap();
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");

    let stashes = opened.stash_list().unwrap();
    assert!(!stashes.is_empty());
    assert!(stashes[0].message.contains("wip"));

    opened.stash_apply(0).unwrap();
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    opened.stash_drop(0).unwrap();
    let stashes = opened.stash_list().unwrap();
    assert!(stashes.is_empty());
}

#[test]
fn checkout_commit_detaches_head_at_target() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let sha = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(sha.status.success());
    let sha = String::from_utf8(sha.stdout).unwrap().trim().to_string();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_commit(&gitgpui_core::domain::CommitId(sha.clone()))
        .unwrap();

    let head_name = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("rev-parse --abbrev-ref");
    assert!(head_name.status.success());
    assert_eq!(String::from_utf8(head_name.stdout).unwrap().trim(), "HEAD");

    let head_sha = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head sha");
    assert!(head_sha.status.success());
    assert_eq!(String::from_utf8(head_sha.stdout).unwrap().trim(), sha);
}

#[test]
fn discard_worktree_changes_reverts_to_index_version() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);
    write(repo, "a.txt", "one\ntwo\nthree\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("a.txt")])
        .unwrap();

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Modified)
    );
    assert!(!status.unstaged.iter().any(|e| e.path == Path::new("a.txt")));
}

#[test]
fn discard_worktree_changes_reverts_modified_file_to_head() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("a.txt")])
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn discard_worktree_changes_removes_staged_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "new.txt", "new\n");
    run_git(repo, &["add", "new.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("new.txt")])
        .unwrap();

    assert!(!repo.join("new.txt").exists());
    let status = opened.status().unwrap();
    assert!(!status.staged.iter().any(|e| e.path == Path::new("new.txt")));
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("new.txt"))
    );
}

#[test]
fn discard_worktree_changes_removes_untracked_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "untracked.txt", "new\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("untracked.txt")])
        .unwrap();

    assert!(!repo.join("untracked.txt").exists());
    let status = opened.status().unwrap();
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("untracked.txt"))
    );
}

#[test]
fn discard_worktree_changes_supports_mixed_selection() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "two\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one!\n");
    fs::remove_file(repo.join("b.txt")).unwrap();
    write(repo, "c.txt", "three\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("a.txt"), Path::new("b.txt"), Path::new("c.txt")])
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "two\n");
    assert!(!repo.join("c.txt").exists());
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn stage_hunk_applies_only_part_of_a_file_to_index() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let mut base = String::new();
    for i in 1..=30 {
        base.push_str(&format!("L{i:02}\n"));
    }
    write(repo, "a.txt", &base);
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let modified = base
        .replace("L02\n", "L02-mod\n")
        .replace("L25\n", "L25-mod\n");
    write(repo, "a.txt", &modified);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged_before = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    let hunk_count_before = unstaged_before
        .lines()
        .filter(|l| l.starts_with("@@"))
        .count();
    assert_eq!(
        hunk_count_before, 2,
        "expected two hunks:\n{unstaged_before}"
    );

    let lines = unstaged_before.lines().collect::<Vec<_>>();
    let file_start = lines
        .iter()
        .position(|l| l.starts_with("diff --git "))
        .unwrap_or(0);
    let first_hunk = lines
        .iter()
        .position(|l| l.starts_with("@@"))
        .expect("first hunk header");
    let second_hunk = (first_hunk + 1..lines.len())
        .find(|&ix| lines.get(ix).is_some_and(|l| l.starts_with("@@")))
        .expect("second hunk header");

    let patch = lines[file_start..first_hunk]
        .iter()
        .chain(lines[first_hunk..second_hunk].iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    opened
        .apply_unified_patch_to_index_with_output(&patch, false)
        .unwrap();

    let staged_after = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert_eq!(
        staged_after.lines().filter(|l| l.starts_with("@@")).count(),
        1,
        "expected one staged hunk:\n{staged_after}"
    );
    assert!(staged_after.contains("-L02"));
    assert!(staged_after.contains("+L02-mod"));
    assert!(!staged_after.contains("L25-mod"));

    let unstaged_after = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert_eq!(
        unstaged_after
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        1,
        "expected one remaining unstaged hunk:\n{unstaged_after}"
    );
    assert!(!unstaged_after.contains("L02-mod"));
    assert!(unstaged_after.contains("-L25"));
    assert!(unstaged_after.contains("+L25-mod"));
}

#[test]
fn unstage_hunk_reverts_only_that_part_in_index() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let mut base = String::new();
    for i in 1..=30 {
        base.push_str(&format!("L{i:02}\n"));
    }
    write(repo, "a.txt", &base);
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let modified = base
        .replace("L02\n", "L02-mod\n")
        .replace("L25\n", "L25-mod\n");
    write(repo, "a.txt", &modified);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged_before = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert_eq!(
        unstaged_before
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        2,
        "expected two hunks:\n{unstaged_before}"
    );

    let lines = unstaged_before.lines().collect::<Vec<_>>();
    let file_start = lines
        .iter()
        .position(|l| l.starts_with("diff --git "))
        .unwrap_or(0);
    let first_hunk = lines
        .iter()
        .position(|l| l.starts_with("@@"))
        .expect("first hunk header");
    let second_hunk = (first_hunk + 1..lines.len())
        .find(|&ix| lines.get(ix).is_some_and(|l| l.starts_with("@@")))
        .expect("second hunk header");

    let patch = lines[file_start..first_hunk]
        .iter()
        .chain(lines[first_hunk..second_hunk].iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    opened
        .apply_unified_patch_to_index_with_output(&patch, false)
        .unwrap();

    let staged_after_stage = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert_eq!(
        staged_after_stage
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        1,
        "expected one staged hunk:\n{staged_after_stage}"
    );

    opened
        .apply_unified_patch_to_index_with_output(&patch, true)
        .unwrap();

    let staged_after_unstage = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert!(
        staged_after_unstage.trim().is_empty(),
        "expected staged diff to be empty:\n{staged_after_unstage}"
    );

    let unstaged_after_unstage = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert_eq!(
        unstaged_after_unstage
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        2,
        "expected two unstaged hunks:\n{unstaged_after_unstage}"
    );
    assert!(unstaged_after_unstage.contains("+L02-mod"));
    assert!(unstaged_after_unstage.contains("+L25-mod"));
}
