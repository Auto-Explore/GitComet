use gitcomet_core::conflict_session::{ConflictPayload, ConflictResolverStrategy};
use gitcomet_core::domain::{DiffArea, DiffTarget, FileConflictKind, FileStatusKind};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::ConflictSide;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // Keep integration tests deterministic by isolating from host git config.
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", NULL_DEVICE);
    // Some scenarios clone local file:// remotes (submodules, temp-origin repos).
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn run_git_expect_failure(repo: &Path, args: &[&str]) {
    let status = git_command()
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
    let mut child = git_command()
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

    let mut child = git_command()
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

fn setup_both_modified_text_conflict(repo: &Path, path: &str, ours: &str, theirs: &str) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "mergetool.guiDefault", "false"]);
    run_git(repo, &["config", "merge.guitool", ""]);

    write(repo, path, "base\n");
    run_git(repo, &["add", path]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, path, theirs);
    run_git(repo, &["add", path]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, path, ours);
    run_git(repo, &["add", path]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);
}

fn setup_both_added_text_conflict(repo: &Path, path: &str, ours: &str, theirs: &str) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "mergetool.guiDefault", "false"]);
    run_git(repo, &["config", "merge.guitool", ""]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, path, theirs);
    run_git(repo, &["add", path]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs_add"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, path, ours);
    run_git(repo, &["add", path]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_add"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    fs::set_permissions(path, Permissions::from_mode(0o755)).unwrap();
}

#[cfg(windows)]
fn set_fixed_mtime(path: &Path) {
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-Item -LiteralPath $env:GITCOMET_TARGET).LastWriteTimeUtc=[DateTimeOffset]::FromUnixTimeSeconds(1700000000).UtcDateTime",
        ])
        .env("GITCOMET_TARGET", path)
        .status()
        .expect("powershell to run");
    assert!(status.success());
}

#[cfg(not(windows))]
fn set_fixed_mtime(path: &Path) {
    let status = Command::new("touch")
        .arg("-d")
        .arg("@1700000000")
        .arg(path)
        .status()
        .expect("touch to run");
    assert!(status.success());
}

#[cfg(windows)]
fn cmd_same_size_content_change_and_exit_failure() -> &'static str {
    r#"powershell -NoProfile -Command "$path=$env:MERGED; $len=(Get-Item -LiteralPath $path).Length; $bytes=New-Object byte[] $len; for ($i=0; $i -lt $len; $i++) { $bytes[$i]=[byte][char]'R' }; [System.IO.File]::WriteAllBytes($path, $bytes); (Get-Item -LiteralPath $path).LastWriteTimeUtc=[DateTimeOffset]::FromUnixTimeSeconds(1700000000).UtcDateTime" & exit /b 1"#
}

#[cfg(not(windows))]
fn cmd_same_size_content_change_and_exit_failure() -> &'static str {
    "len=$(wc -c < \"$MERGED\"); head -c \"$len\" /dev/zero | tr '\\0' 'R' > \"$MERGED\"; touch -d '@1700000000' \"$MERGED\"; exit 1"
}

#[cfg(windows)]
fn cmd_exit_success() -> &'static str {
    "exit /b 0"
}

#[cfg(not(windows))]
fn cmd_exit_success() -> &'static str {
    "exit 0"
}

#[cfg(windows)]
fn cmd_delete_merged_and_exit_failure() -> &'static str {
    r#"powershell -NoProfile -Command "Remove-Item -LiteralPath $env:MERGED -Force -ErrorAction SilentlyContinue" & exit /b 1"#
}

#[cfg(not(windows))]
fn cmd_delete_merged_and_exit_failure() -> &'static str {
    "rm -f \"$MERGED\"; exit 1"
}

#[cfg(windows)]
fn cmd_write_unresolved_markers_and_exit_success() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, ('<<<<<<< ours' + [Environment]::NewLine + 'left' + [Environment]::NewLine + '=======' + [Environment]::NewLine + 'right' + [Environment]::NewLine + '>>>>>>> theirs' + [Environment]::NewLine))" & exit /b 0"#
}

#[cfg(not(windows))]
fn cmd_write_unresolved_markers_and_exit_success() -> &'static str {
    "printf '<<<<<<< ours\nleft\n=======\nright\n>>>>>>> theirs\n' > \"$MERGED\"; exit 0"
}

#[cfg(windows)]
fn cmd_copy_remote_to_merged_and_exit_success() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllBytes($env:MERGED, [System.IO.File]::ReadAllBytes($env:REMOTE))""#
}

#[cfg(not(windows))]
fn cmd_copy_remote_to_merged_and_exit_success() -> &'static str {
    "cat \"$REMOTE\" > \"$MERGED\"; exit 0"
}

#[cfg(windows)]
fn cmd_write_cli_to_merged() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, 'cli' + [Environment]::NewLine)""#
}

#[cfg(not(windows))]
fn cmd_write_cli_to_merged() -> &'static str {
    "printf 'cli\\n' > \"$MERGED\""
}

#[cfg(windows)]
fn cmd_write_gui_to_merged() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, 'gui' + [Environment]::NewLine)""#
}

#[cfg(not(windows))]
fn cmd_write_gui_to_merged() -> &'static str {
    "printf 'gui\\n' > \"$MERGED\""
}

#[cfg(windows)]
fn cmd_write_cmd_to_merged() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, 'cmd' + [Environment]::NewLine)""#
}

#[cfg(not(windows))]
fn cmd_write_cmd_to_merged() -> &'static str {
    "printf 'cmd\\n' > \"$MERGED\"; exit 0"
}

#[cfg(windows)]
fn cmd_dump_stage_paths_and_copy_remote() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllLines($env:MERGED + '.env', @($env:BASE, $env:LOCAL, $env:REMOTE)); [System.IO.File]::WriteAllBytes($env:MERGED, [System.IO.File]::ReadAllBytes($env:REMOTE))""#
}

#[cfg(not(windows))]
fn cmd_dump_stage_paths_and_copy_remote() -> &'static str {
    "printf '%s\\n%s\\n%s\\n' \"$BASE\" \"$LOCAL\" \"$REMOTE\" > \"$MERGED.env\"; cat \"$REMOTE\" > \"$MERGED\""
}

#[cfg(windows)]
fn cmd_dump_stage_paths_and_exit_failure() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllLines($env:MERGED + '.env', @($env:BASE, $env:LOCAL, $env:REMOTE))" & exit /b 1"#
}

#[cfg(not(windows))]
fn cmd_dump_stage_paths_and_exit_failure() -> &'static str {
    "printf '%s\\n%s\\n%s\\n' \"$BASE\" \"$LOCAL\" \"$REMOTE\" > \"$MERGED.env\"; exit 1"
}

#[cfg(windows)]
fn cmd_dump_base_size_and_copy_remote() -> &'static str {
    r#"powershell -NoProfile -Command "$size=(Get-Item -LiteralPath $env:BASE).Length; [System.IO.File]::WriteAllText($env:MERGED + '.base-size', [string]$size); [System.IO.File]::WriteAllBytes($env:MERGED, [System.IO.File]::ReadAllBytes($env:REMOTE))""#
}

#[cfg(not(windows))]
fn cmd_dump_base_size_and_copy_remote() -> &'static str {
    "printf '%s' \"$(wc -c < \"$BASE\" | tr -d '[:space:]')\" > \"$MERGED.base-size\"; cat \"$REMOTE\" > \"$MERGED\""
}

fn read_stage_env_vars(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| line.trim().to_string())
        .collect()
}

fn normalize_stage_var(stage_var: &str) -> String {
    stage_var.trim().replace('\\', "/")
}

fn stage_var_to_fs_path(repo: &Path, stage_var: &str) -> PathBuf {
    let stage_path = Path::new(stage_var.trim());
    if stage_path.is_absolute() {
        stage_path.to_path_buf()
    } else if let Ok(relative) = stage_path.strip_prefix(".") {
        repo.join(relative)
    } else {
        repo.join(stage_path)
    }
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
    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse to run");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();

    let commit = opened
        .diff_file_text(&DiffTarget::Commit {
            commit_id: gitcomet_core::domain::CommitId(head),
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
    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse to run");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();

    let commit = opened
        .diff_file_image(&DiffTarget::Commit {
            commit_id: gitcomet_core::domain::CommitId(head),
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
fn accept_conflict_deletion_resolves_delete_outcome_conflicts() {
    #[derive(Clone, Copy)]
    struct ConflictDeleteFixture {
        kind: FileConflictKind,
        has_base: bool,
        has_ours: bool,
        has_theirs: bool,
    }

    let fixtures = [
        ConflictDeleteFixture {
            kind: FileConflictKind::BothDeleted,
            has_base: true,
            has_ours: false,
            has_theirs: false,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::AddedByUs,
            has_base: false,
            has_ours: true,
            has_theirs: false,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::AddedByThem,
            has_base: false,
            has_ours: false,
            has_theirs: true,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::DeletedByUs,
            has_base: true,
            has_ours: false,
            has_theirs: true,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::DeletedByThem,
            has_base: true,
            has_ours: true,
            has_theirs: false,
        },
    ];

    for fixture in fixtures {
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
            .expect("expected fixture path to appear as conflict");
        assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
        assert_eq!(conflict_entry.conflict, Some(fixture.kind));

        opened.accept_conflict_deletion(Path::new("a.txt")).unwrap();

        let after = opened.status().unwrap();
        assert!(
            !repo.join("a.txt").exists(),
            "expected path to be removed after accepting deletion for {:?}",
            fixture.kind
        );
        assert!(
            after
                .staged
                .iter()
                .chain(after.unstaged.iter())
                .all(|e| e.path != Path::new("a.txt")),
            "expected no status entry for deleted path after resolving {:?}; status={after:?}",
            fixture.kind
        );
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
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);
    assert!(!session.is_fully_resolved());
    assert!(matches!(session.base, ConflictPayload::Binary(_)));
    assert!(matches!(session.ours, ConflictPayload::Binary(_)));
    assert!(matches!(session.theirs, ConflictPayload::Binary(_)));
}

#[test]
fn checkout_conflict_side_resolves_non_utf8_binary_conflict() {
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

    let session = opened
        .conflict_session(Path::new("bin.dat"))
        .unwrap()
        .expect("binary conflict session");
    assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);

    opened
        .checkout_conflict_side(Path::new("bin.dat"), ConflictSide::Theirs)
        .unwrap();

    assert_eq!(fs::read(repo.join("bin.dat")).unwrap(), theirs_bytes);

    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("bin.dat") && e.kind == FileStatusKind::Conflicted),
        "binary conflict should be cleared after choosing theirs"
    );
    assert!(
        status_after
            .staged
            .iter()
            .any(|e| e.path == Path::new("bin.dat")),
        "chosen binary side should be staged"
    );
}

#[test]
fn conflict_session_both_deleted_binary_prefers_decision_strategy() {
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

    let base_blob = hash_blob(repo, b"\x00base\xff\n");
    set_unmerged_stages(repo, "gone.bin", Some(base_blob.as_str()), None, None);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("gone.bin"))
        .expect("expected conflict status entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::BothDeleted));

    let session = opened
        .conflict_session(Path::new("gone.bin"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.conflict_kind, FileConflictKind::BothDeleted);
    assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
    assert!(matches!(session.base, ConflictPayload::Binary(_)));
    assert!(session.ours.is_absent());
    assert!(session.theirs.is_absent());
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
fn launch_mergetool_trust_exit_false_detects_same_size_content_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    // Normalize pre-tool mtime to a fixed timestamp so metadata-only checks
    // cannot detect the edit when the command restores mtime.
    set_fixed_mtime(&repo.join("a.txt"));

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_same_size_content_change_and_exit_failure(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(1));

    let on_disk = fs::read(repo.join("a.txt")).unwrap();
    assert!(!on_disk.is_empty());
    assert_eq!(on_disk[0], b'R');
    assert_eq!(result.merged_contents.as_deref(), Some(on_disk.as_slice()));

    let status = opened.status().unwrap();
    assert!(status.unstaged.iter().all(|e| e.path != Path::new("a.txt")));
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Modified),
        "expected staged resolution after content-changing mergetool run, got {status:?}"
    );
}

#[test]
fn launch_mergetool_trust_exit_false_requires_content_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &["config", "mergetool.fake.cmd", cmd_exit_success()],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(!result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert!(result.merged_contents.is_none());

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt")),
        "unexpected staged resolution when mergetool did not change output: {status:?}"
    );
    let conflict_entry = status
        .unstaged
        .iter()
        .find(|entry| entry.path == Path::new("a.txt"))
        .expect("conflict should remain unresolved");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(
        conflict_entry.conflict,
        Some(FileConflictKind::BothModified)
    );
}

#[test]
fn launch_mergetool_trust_exit_false_detects_deleted_output_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_delete_merged_and_exit_failure(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(1));
    assert!(
        result.merged_contents.is_none(),
        "deleted-output resolution should not return merged file bytes"
    );
    assert!(
        !repo.join("a.txt").exists(),
        "mergetool delete output should remove the worktree file"
    );

    let status = opened.status().unwrap();
    assert!(
        status.unstaged.iter().all(|e| e.path != Path::new("a.txt")),
        "expected conflict to clear from unstaged after delete-output mergetool run, got {status:?}"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Deleted),
        "expected delete-output mergetool run to stage file deletion, got {status:?}"
    );
}

#[test]
fn launch_mergetool_rejects_unresolved_marker_output() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_write_unresolved_markers_and_exit_success(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .launch_mergetool(Path::new("a.txt"))
        .expect_err("mergetool should fail when merged output still has markers");

    match err.kind() {
        ErrorKind::Backend(msg) => {
            assert!(
                msg.contains("left unresolved conflict markers"),
                "unexpected backend error: {msg}"
            );
            assert!(
                msg.contains("a.txt"),
                "backend error should include conflicted path: {msg}"
            );
        }
        other => panic!("expected backend error, got {other:?}"),
    }

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt")),
        "unexpected staged resolution when mergetool left markers: {status:?}"
    );
    let conflict_entry = status
        .unstaged
        .iter()
        .find(|entry| entry.path == Path::new("a.txt"))
        .expect("conflict should remain unresolved");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(
        conflict_entry.conflict,
        Some(FileConflictKind::BothModified)
    );
}

#[cfg(not(windows))]
#[test]
fn launch_mergetool_custom_cmd_supports_braced_env_variables() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/a space.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            "cat \"${REMOTE}\" > \"${MERGED}\"; exit 0",
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(
        result.success,
        "expected braced variable expansion to succeed, got {result:?}"
    );
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));

    let on_disk = fs::read_to_string(repo.join(conflicted_path)).unwrap();
    assert_eq!(on_disk, "theirs\n");
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );

    let status = opened.status().unwrap();
    assert!(
        status.unstaged.iter().all(|e| e.path != path),
        "expected conflict to clear after mergetool resolution: {status:?}"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == path && e.kind == FileStatusKind::Modified),
        "expected resolved file to be staged after mergetool run: {status:?}"
    );
}

#[test]
#[cfg(windows)]
fn launch_mergetool_custom_cmd_supports_cmd_percent_env_variables() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/a space.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            "copy /Y \"%REMOTE%\" \"%MERGED%\" > NUL && exit /b 0",
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(fs::read_to_string(repo.join(conflicted_path)).unwrap(), "theirs\n");
}

#[test]
fn launch_mergetool_custom_cmd_supports_unicode_conflicted_path() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/spaced 日本語 file.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_copy_remote_to_merged_and_exit_success(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(
        result.success,
        "expected unicode conflicted path to resolve, got {result:?}"
    );
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));

    let on_disk = fs::read_to_string(repo.join(conflicted_path)).unwrap();
    assert_eq!(on_disk, "theirs\n");
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );

    let status = opened.status().unwrap();
    assert!(
        status.unstaged.iter().all(|entry| entry.path != path),
        "expected unicode conflict to clear after mergetool resolution: {status:?}"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|entry| entry.path == path && entry.kind == FileStatusKind::Modified),
        "expected resolved unicode path to be staged after mergetool run: {status:?}"
    );
}

#[test]
fn launch_mergetool_prefers_merge_guitool_when_gui_default_true() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "cli"]);
    run_git(repo, &["config", "merge.guitool", "gui"]);
    run_git(repo, &["config", "mergetool.guiDefault", "true"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.cli.cmd",
            cmd_write_cli_to_merged(),
        ],
    );
    run_git(
        repo,
        &[
            "config",
            "mergetool.gui.cmd",
            cmd_write_gui_to_merged(),
        ],
    );
    run_git(repo, &["config", "mergetool.cli.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.gui.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "gui");
    assert_eq!(result.merged_contents.as_deref(), Some("gui\n".as_bytes()));
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "gui\n");
}

#[cfg(unix)]
#[test]
fn launch_mergetool_uses_tool_path_override_without_custom_cmd() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let script_path = repo.join("fake-merge-tool.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\n# args: local base remote merged\ncat \"$3\" > \"$4\"\n",
    )
    .unwrap();
    make_executable(&script_path);

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.path",
            script_path.to_string_lossy().as_ref(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "theirs\n");
}

#[cfg(unix)]
#[test]
fn launch_mergetool_prefers_custom_cmd_over_tool_path_override() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let script_path = repo.join("fake-merge-tool.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\nprintf 'path\\n' > \"$4\"\ntouch \"$PWD/path_invoked\"\n",
    )
    .unwrap();
    make_executable(&script_path);

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.path",
            script_path.to_string_lossy().as_ref(),
        ],
    );
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_write_cmd_to_merged(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(result.merged_contents.as_deref(), Some("cmd\n".as_bytes()));
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "cmd\n");
    assert!(
        !repo.join("path_invoked").exists(),
        "tool path executable should not run when mergetool.<tool>.cmd is configured"
    );
}

#[test]
fn launch_mergetool_write_to_temp_true_uses_temp_stage_paths() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_stage_paths_and_copy_remote(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);

    let vars = read_stage_env_vars(&repo.join("a.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let var_path = Path::new(&var);
        let normalized_var = normalize_stage_var(&var);
        assert!(
            var_path.is_absolute(),
            "writeToTemp=true should pass absolute temp paths, got {var}"
        );
        assert!(
            normalized_var.contains("gitcomet-mergetool-"),
            "expected temporary mergetool prefix in path, got {var}"
        );
        assert!(
            !normalized_var.starts_with("./"),
            "writeToTemp=true should not use workdir-prefixed paths: {var}"
        );
        assert!(
            !var_path.exists(),
            "writeToTemp=true with default keepTemporaries=false should cleanup stage files: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_false_uses_workdir_prefixed_stage_paths() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_stage_paths_and_copy_remote(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("docs/note.txt")).unwrap();
    assert!(result.success, "{result:?}");

    let vars = read_stage_env_vars(&repo.join("docs/note.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let normalized_var = normalize_stage_var(&var);
        assert!(
            normalized_var.starts_with("./docs/note_"),
            "writeToTemp=false should use './' prefixed workdir paths, got {var}"
        );
        assert!(
            normalized_var.contains("_BASE_")
                || normalized_var.contains("_LOCAL_")
                || normalized_var.contains("_REMOTE_"),
            "unexpected stage-file naming: {var}"
        );
        let fs_path = stage_var_to_fs_path(repo, &var);
        assert!(
            !fs_path.exists(),
            "writeToTemp=false with default keepTemporaries=false should cleanup stage files: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_false_keep_temporaries_preserves_stage_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_stage_paths_and_copy_remote(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("docs/note.txt")).unwrap();
    assert!(result.success, "{result:?}");

    let vars = read_stage_env_vars(&repo.join("docs/note.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let normalized_var = normalize_stage_var(&var);
        assert!(
            normalized_var.starts_with("./docs/note_"),
            "writeToTemp=false should use './' prefixed workdir paths, got {var}"
        );
        let fs_path = stage_var_to_fs_path(repo, &var);
        assert!(
            fs_path.exists(),
            "keepTemporaries=true should keep stage file in workdir mode: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_false_keep_temporaries_preserves_stage_files_on_abort() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_stage_paths_and_exit_failure(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("docs/note.txt")).unwrap();
    assert!(
        !result.success,
        "tool exit failure should be reported as unresolved"
    );

    let vars = read_stage_env_vars(&repo.join("docs/note.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let normalized_var = normalize_stage_var(&var);
        assert!(
            normalized_var.starts_with("./docs/note_"),
            "writeToTemp=false should use './' prefixed workdir paths, got {var}"
        );
        let fs_path = stage_var_to_fs_path(repo, &var);
        assert!(
            fs_path.exists(),
            "keepTemporaries=true should keep stage file on abort in workdir mode: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_true_keep_temporaries_preserves_stage_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_stage_paths_and_copy_remote(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success, "{result:?}");

    let vars = read_stage_env_vars(&repo.join("a.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");

    let mut temp_dirs: Vec<PathBuf> = Vec::new();
    for var in vars {
        let var_path = Path::new(&var);
        let normalized_var = normalize_stage_var(&var);
        assert!(
            var_path.is_absolute(),
            "writeToTemp=true should pass absolute temp paths, got {var}"
        );
        assert!(
            normalized_var.contains("gitcomet-mergetool-"),
            "expected temporary mergetool prefix in path, got {var}"
        );
        assert!(
            var_path.exists(),
            "keepTemporaries=true should keep stage file in temp mode: {var}"
        );
        if let Some(parent) = var_path.parent()
            && !temp_dirs.iter().any(|dir| dir == parent)
        {
            temp_dirs.push(parent.to_path_buf());
        }
    }

    // Keep test environment clean even though behavior keeps temp files.
    for dir in temp_dirs {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn launch_mergetool_write_to_temp_true_keep_temporaries_preserves_stage_files_on_abort() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_stage_paths_and_exit_failure(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(
        !result.success,
        "tool exit failure should be reported as unresolved"
    );

    let vars = read_stage_env_vars(&repo.join("a.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");

    let mut temp_dirs: Vec<PathBuf> = Vec::new();
    for var in vars {
        let var_path = Path::new(&var);
        let normalized_var = normalize_stage_var(&var);
        assert!(
            var_path.is_absolute(),
            "writeToTemp=true should pass absolute temp paths, got {var}"
        );
        assert!(
            normalized_var.contains("gitcomet-mergetool-"),
            "expected temporary mergetool prefix in path, got {var}"
        );
        assert!(
            var_path.exists(),
            "keepTemporaries=true should keep stage file on abort in temp mode: {var}"
        );
        if let Some(parent) = var_path.parent()
            && !temp_dirs.iter().any(|dir| dir == parent)
        {
            temp_dirs.push(parent.to_path_buf());
        }
    }

    // Keep test environment clean even though behavior keeps temp files.
    for dir in temp_dirs {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn launch_mergetool_no_base_conflict_passes_empty_base_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_added_text_conflict(repo, "new.txt", "ours added\n", "theirs added\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.cmd",
            cmd_dump_base_size_and_copy_remote(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("new.txt")).unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.join("new.txt.base-size")).unwrap(),
        "0",
        "BASE should be an empty file for both-added/no-base conflicts"
    );
    assert_eq!(
        fs::read_to_string(repo.join("new.txt")).unwrap(),
        "theirs added\n"
    );
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

    let msg = git_command()
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
    let c1 = git_command()
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
        .reset_with_output("HEAD~1", gitcomet_core::services::ResetMode::Soft)
        .unwrap();

    let head = git_command()
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
    let c1 = git_command()
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
        .reset_with_output("HEAD~1", gitcomet_core::services::ResetMode::Mixed)
        .unwrap();

    let head = git_command()
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
    let c1 = git_command()
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
        .reset_with_output("HEAD~1", gitcomet_core::services::ResetMode::Hard)
        .unwrap();

    let head = git_command()
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

    let c2 = git_command()
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
        .revert(&gitcomet_core::domain::CommitId(c2.clone()))
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());

    let head = git_command()
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

    let head_before = git_command()
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

    let head_after = git_command()
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

    let count = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("rev-list --count");
    assert!(count.status.success());
    assert_eq!(String::from_utf8(count.stdout).unwrap().trim(), "1");

    let msg = git_command()
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

    let parents = git_command()
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

    let parents = git_command()
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

    let msg = git_command()
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
fn commit_finishes_merge_when_resolved_tree_matches_head() {
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
    run_git(repo, &["checkout", "--ours", "a.txt"]);
    run_git(repo, &["add", "a.txt"]);

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty(), "expected no staged changes");
    assert!(status.unstaged.is_empty(), "expected no unstaged changes");

    opened
        .commit("Merge branch 'feature'")
        .expect("merge commit should succeed even without tree changes");

    assert!(opened.merge_commit_message().unwrap().is_none());

    let parents = git_command()
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
}

#[test]
fn rebase_replays_commits_onto_target_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
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
    let master_head = git_command()
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

    opened.rebase_with_output("main").unwrap();

    let parent = git_command()
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
            &gitcomet_core::domain::CommitId("HEAD".to_string()),
        )
        .unwrap();
    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );

    opened.delete_branch("feature").unwrap();
    let deleted = git_command()
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
    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected tag to be deleted");
}

#[test]
fn push_and_delete_remote_tag() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();

    opened.create_tag_with_output("v1.0.0", "HEAD").unwrap();
    opened.push_tag_with_output("origin", "v1.0.0").unwrap();
    run_git(
        &origin,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );

    opened
        .delete_remote_tag_with_output("origin", "v1.0.0")
        .unwrap();
    let deleted = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected remote tag to be deleted");
}

#[test]
fn prune_merged_branches_deletes_local_branches_missing_on_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    write(&repo, "feature.txt", "feature\n");
    run_git(&repo, &["add", "feature.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&repo, &["push", "-u", "origin", "feature"]);

    run_git(&repo, &["checkout", "main"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "feature",
            "-m",
            "merge feature",
        ],
    );
    run_git(&repo, &["push", "origin", "main"]);
    run_git(&repo, &["push", "origin", "--delete", "feature"]);

    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.prune_merged_branches_with_output().unwrap();

    let deleted = git_command()
        .arg("-C")
        .arg(&repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(
        !deleted.success(),
        "expected merged local branch to be deleted"
    );
}

#[test]
fn prune_local_tags_deletes_tags_missing_from_remotes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    run_git(&repo, &["tag", "v1.0.0"]);
    run_git(&repo, &["tag", "stale-local"]);
    run_git(&repo, &["push", "origin", "refs/tags/v1.0.0"]);
    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/stale-local"],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.prune_local_tags_with_output().unwrap();

    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );
    let stale_deleted = git_command()
        .arg("-C")
        .arg(&repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/stale-local"])
        .status()
        .expect("show-ref");
    assert!(
        !stale_deleted.success(),
        "expected stale local tag to be deleted"
    );
}

#[test]
fn list_remote_branches_includes_fetched_remote_tracking_refs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
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
    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

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
            .any(|b| b.remote == "origin" && b.name == "main")
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

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    write(&repo, "a.txt", "one\ntwo\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let head_local = git_command()
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

    let head_remote = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .expect("rev-parse origin/main");
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

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

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
    let head_local = git_command()
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

    let head_remote = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .expect("rev-parse refs/heads/main");
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

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&repo_a, &["init", "-b", "main"]);
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
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

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

    let head_origin = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
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
        .pull_with_output(gitcomet_core::services::PullMode::FastForwardOnly)
        .unwrap();

    let head_b = git_command()
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

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&repo_a, &["init", "-b", "main"]);
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
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

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

    let head_origin = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
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
        .pull_with_output(gitcomet_core::services::PullMode::Merge)
        .unwrap();

    let head_b = git_command()
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse b");
    assert!(head_b.status.success());
    let head_b = String::from_utf8(head_b.stdout).unwrap().trim().to_string();
    assert_eq!(head_b, head_origin);

    let parents = git_command()
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
    assert_eq!(stashes[0].index, 0);
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
fn stash_apply_conflict_is_mergeable() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    opened.stash_create("wip", false).unwrap();

    write(repo, "a.txt", "base\nbranch-change\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "branch-change",
        ],
    );

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply conflict should report failure");
    assert!(
        err.to_string().contains("git stash apply failed"),
        "unexpected error: {err}"
    );

    let status = opened.status().unwrap();
    let conflict_entry = status
        .unstaged
        .iter()
        .find(|entry| entry.path == Path::new("a.txt"))
        .expect("expected conflicted path after stash apply merge");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(
        conflict_entry.conflict,
        Some(FileConflictKind::BothModified)
    );

    let contents = fs::read_to_string(repo.join("a.txt")).unwrap();
    assert!(contents.contains("<<<<<<<"));
    assert!(contents.contains("======="));
    assert!(contents.contains(">>>>>>>"));
}

#[test]
fn stash_apply_still_errors_when_merge_does_not_start() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    opened.stash_create("wip", false).unwrap();

    write(repo, "a.txt", "base\nlocal-uncommitted-change\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should fail when local edits would be overwritten");
    assert!(
        err.to_string().contains("overwritten by merge"),
        "unexpected error: {err}"
    );

    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("a.txt"))
        .expect("expected modified file in unstaged status");
    assert_eq!(entry.kind, FileStatusKind::Modified);
    assert_eq!(entry.conflict, None);
}

#[test]
fn stash_apply_allows_merge_when_only_untracked_restore_fails() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    // Existing untracked file blocks restoration of untracked payload from stash.
    write(repo, "Cargo.toml.orig", "local copy\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report untracked restore failure");
    assert!(
        err.to_string()
            .contains("could not restore untracked files from stash")
            || err.to_string().contains("already exists, no checkout"),
        "unexpected error: {err}"
    );

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "base\nstash-change\n"
    );
    let untracked_merged = fs::read_to_string(repo.join("Cargo.toml.orig")).unwrap();
    assert!(untracked_merged.contains("<<<<<<< Current file"));
    assert!(untracked_merged.contains("local copy"));
    assert!(untracked_merged.contains("======="));
    assert!(untracked_merged.contains("from stash"));
    assert!(untracked_merged.contains(">>>>>>> Stashed file"));

    let status = opened.status().unwrap();
    let tracked = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("a.txt"))
        .expect("expected tracked stash change to be present");
    assert_eq!(tracked.kind, FileStatusKind::Modified);
    assert_eq!(tracked.conflict, None);
    assert!(status.unstaged.iter().any(|candidate| {
        candidate.path == Path::new("Cargo.toml.orig")
            && candidate.kind == FileStatusKind::Untracked
    }));
}

#[test]
fn stash_apply_allows_untracked_restore_failure_when_stash_has_tracked_payload() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Stash contains tracked and untracked payload.
    write(repo, "a.txt", "base\nstash-change\n");
    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    // Apply the same tracked change on the branch first, so stash apply has no
    // tracked-status delta even though stash had tracked payload.
    write(repo, "a.txt", "base\nstash-change\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "same-tracked-change",
        ],
    );

    // Existing untracked file blocks restoration of stash untracked payload.
    write(repo, "Cargo.toml.orig", "local copy\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report untracked restore failure");
    assert!(
        err.to_string()
            .contains("could not restore untracked files from stash")
            || err.to_string().contains("already exists, no checkout"),
        "unexpected error: {err}"
    );

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "base\nstash-change\n"
    );
    let untracked_merged = fs::read_to_string(repo.join("Cargo.toml.orig")).unwrap();
    assert!(untracked_merged.contains("<<<<<<< Current file"));
    assert!(untracked_merged.contains("local copy"));
    assert!(untracked_merged.contains("======="));
    assert!(untracked_merged.contains("from stash"));
    assert!(untracked_merged.contains(">>>>>>> Stashed file"));

    let status = opened.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt"))
    );
    assert!(status.unstaged.iter().any(|candidate| {
        candidate.path == Path::new("Cargo.toml.orig")
            && candidate.kind == FileStatusKind::Untracked
    }));
}

#[test]
fn stash_apply_merges_when_only_untracked_restore_fails_without_tracked_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    write(repo, "Cargo.toml.orig", "local copy\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report untracked restore failure");
    assert!(
        err.to_string()
            .contains("could not restore untracked files from stash")
            || err.to_string().contains("already exists, no checkout"),
        "unexpected error: {err}"
    );

    let contents = fs::read_to_string(repo.join("Cargo.toml.orig")).unwrap();
    assert!(contents.contains("<<<<<<< Current file"));
    assert!(contents.contains("local copy"));
    assert!(contents.contains("======="));
    assert!(contents.contains("from stash"));
    assert!(contents.contains(">>>>>>> Stashed file"));

    let status = opened.status().unwrap();
    assert!(status.unstaged.iter().any(|entry| {
        entry.path == Path::new("Cargo.toml.orig") && entry.kind == FileStatusKind::Untracked
    }));
}

#[test]
fn stash_list_reports_reflog_indices_for_drop() {
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
    opened.stash_create("wip-1", false).unwrap();

    write(repo, "a.txt", "one\nthree\n");
    opened.stash_create("wip-2", false).unwrap();

    let stashes = opened.stash_list().unwrap();
    assert_eq!(stashes.len(), 2);
    assert_eq!(stashes[0].index, 0);
    assert_eq!(stashes[1].index, 1);

    // Drop the older stash by the index returned from `stash_list`.
    opened.stash_drop(stashes[1].index).unwrap();
    let stashes = opened.stash_list().unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].index, 0);
    assert!(stashes[0].message.contains("wip-2"));
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

    let sha = git_command()
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
        .checkout_commit(&gitcomet_core::domain::CommitId(sha.clone()))
        .unwrap();

    let head_name = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("rev-parse --abbrev-ref");
    assert!(head_name.status.success());
    assert_eq!(String::from_utf8(head_name.stdout).unwrap().trim(), "HEAD");

    let head_sha = git_command()
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

// ---------------------------------------------------------------------------
// End-to-end conflict resolution workflow tests
// ---------------------------------------------------------------------------

/// End-to-end test: create a merge conflict, load the conflict session,
/// resolve all regions manually, generate resolved text, write it to disk,
/// stage the file, and verify the conflict is fully resolved.
#[test]
fn resolve_conflict_write_and_stage_clears_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    // Create a BothModified conflict: both sides change the same lines.
    let base_content = "header\nconflict-line\nfooter\n";
    let ours_content = "header\nours-version\nfooter\n";
    let theirs_content = "header\ntheirs-version\nfooter\n";

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "doc.txt", base_content);
    run_git(repo, &["add", "doc.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "doc.txt", theirs_content);
    run_git(repo, &["add", "doc.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "doc.txt", ours_content);
    run_git(repo, &["add", "doc.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // 1. Verify file is in conflict status
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("doc.txt"))
        .expect("expected conflict entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::BothModified));

    // 2. Load conflict session via backend API
    let session = opened
        .conflict_session(Path::new("doc.txt"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.conflict_kind, FileConflictKind::BothModified);

    // 3. Verify worktree file contains conflict markers
    let worktree_content = fs::read_to_string(repo.join("doc.txt")).unwrap();
    let validation = gitcomet_core::services::validate_conflict_resolution_text(&worktree_content);
    assert!(
        validation.has_conflict_markers,
        "worktree file should contain conflict markers"
    );

    // 4. Write manually resolved content (pick ours version)
    let resolved_content = "header\nours-version\nfooter\n";
    let resolved_validation =
        gitcomet_core::services::validate_conflict_resolution_text(resolved_content);
    assert!(
        !resolved_validation.has_conflict_markers,
        "resolved content should have no conflict markers"
    );

    // 5. Write resolved text to worktree and stage
    fs::write(repo.join("doc.txt"), resolved_content).unwrap();
    opened.stage(&[Path::new("doc.txt")]).unwrap();

    // 6. Verify conflict is resolved — no more conflict status
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("doc.txt") && e.kind == FileStatusKind::Conflicted),
        "doc.txt should no longer be conflicted after staging resolved content"
    );
}

#[test]
fn resolve_both_added_conflict_write_and_stage_clears_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_added_text_conflict(repo, "new.txt", "ours added\n", "theirs added\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let before = opened.status().unwrap();
    let conflict_entry = before
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("new.txt"))
        .expect("expected both-added conflict path in unstaged status");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(conflict_entry.conflict, Some(FileConflictKind::BothAdded));

    let merged_before = fs::read_to_string(repo.join("new.txt")).unwrap();
    assert!(
        merged_before.contains("<<<<<<<"),
        "expected merge markers before resolution"
    );

    let session = opened
        .conflict_session(Path::new("new.txt"))
        .unwrap()
        .expect("conflict session for both-added path");
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.conflict_kind, FileConflictKind::BothAdded);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);

    let resolved = "resolved both-added\n";
    write(repo, "new.txt", resolved);
    opened.stage(&[Path::new("new.txt")]).unwrap();

    let validation = gitcomet_core::services::validate_conflict_resolution_text(resolved);
    assert!(!validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 0);

    let after = opened.status().unwrap();
    assert!(
        after
            .unstaged
            .iter()
            .all(|e| e.path != Path::new("new.txt")),
        "expected conflict path to be removed from unstaged after save+stage; status={after:?}"
    );
    assert!(
        after.staged.iter().any(|e| {
            e.path == Path::new("new.txt")
                && matches!(e.kind, FileStatusKind::Modified | FileStatusKind::Added)
        }),
        "expected resolved both-added file to be staged as modified/added; status={after:?}"
    );
    assert_eq!(fs::read_to_string(repo.join("new.txt")).unwrap(), resolved);
}

/// End-to-end test: autosolve Pass 1 correctly resolves trivial regions
/// using synthetic conflict stages where some regions are trivially
/// resolvable (one side equals base) while others are genuine conflicts.
#[test]
fn autosolve_safe_resolves_trivial_conflict_regions_end_to_end() {
    use gitcomet_core::conflict_session::{
        ConflictPayload, ConflictRegionResolution, ConflictSession,
    };

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

    // Create a BothModified conflict using synthetic stages.
    // Write a worktree file with conflict markers containing three regions:
    //   Region 0: only ours changed (trivial → OnlyOursChanged)
    //   Region 1: both changed differently (genuine conflict)
    //   Region 2: both sides identical (trivial → IdenticalSides)
    let base_blob = hash_blob(repo, b"base-r0\nbase-r1\nbase-r2\n");
    let ours_blob = hash_blob(repo, b"ours-r0\nours-r1\nsame-r2\n");
    let theirs_blob = hash_blob(repo, b"base-r0\ntheirs-r1\nsame-r2\n");
    set_unmerged_stages(
        repo,
        "multi.txt",
        Some(&base_blob),
        Some(&ours_blob),
        Some(&theirs_blob),
    );

    // Write worktree file with three conflict marker blocks
    let merged_markers = concat!(
        "<<<<<<< HEAD\n",
        "ours-r0\n",
        "||||||| base\n",
        "base-r0\n",
        "=======\n",
        "base-r0\n",
        ">>>>>>> feature\n",
        "<<<<<<< HEAD\n",
        "ours-r1\n",
        "||||||| base\n",
        "base-r1\n",
        "=======\n",
        "theirs-r1\n",
        ">>>>>>> feature\n",
        "<<<<<<< HEAD\n",
        "same-r2\n",
        "||||||| base\n",
        "base-r2\n",
        "=======\n",
        "same-r2\n",
        ">>>>>>> feature\n",
    );
    write(repo, "multi.txt", merged_markers);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Build a ConflictSession from the backend
    let session_opt = opened.conflict_session(Path::new("multi.txt")).unwrap();
    // The backend may or may not build the session (depending on status
    // detection of the synthetic stages). Build one manually if needed.
    let mut session = session_opt.unwrap_or_else(|| {
        ConflictSession::from_merged_text(
            PathBuf::from("multi.txt"),
            FileConflictKind::BothModified,
            ConflictPayload::Text("base-r0\nbase-r1\nbase-r2\n".into()),
            ConflictPayload::Text("ours-r0\nours-r1\nsame-r2\n".into()),
            ConflictPayload::Text("base-r0\ntheirs-r1\nsame-r2\n".into()),
            merged_markers,
        )
    });

    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.total_regions(), 3);
    assert_eq!(
        session.unsolved_count(),
        3,
        "all regions should start unresolved"
    );

    // Apply auto-resolve Pass 1
    let auto_resolved = session.auto_resolve_safe();
    assert_eq!(
        auto_resolved, 2,
        "expected 2 trivial regions to be auto-resolved"
    );
    assert_eq!(
        session.unsolved_count(),
        1,
        "1 genuine conflict should remain"
    );

    // Verify specific rules
    match &session.regions[0].resolution {
        ConflictRegionResolution::AutoResolved { rule, content, .. } => {
            assert_eq!(
                *rule,
                gitcomet_core::conflict_session::AutosolveRule::OnlyOursChanged,
            );
            assert_eq!(content, "ours-r0\n");
        }
        other => panic!("region 0 should be auto-resolved, got {:?}", other),
    }
    assert!(
        !session.regions[1].resolution.is_resolved(),
        "region 1 (genuine conflict) should remain unresolved"
    );
    match &session.regions[2].resolution {
        ConflictRegionResolution::AutoResolved { rule, content, .. } => {
            assert_eq!(
                *rule,
                gitcomet_core::conflict_session::AutosolveRule::IdenticalSides,
            );
            assert_eq!(content, "same-r2\n");
        }
        other => panic!("region 2 should be auto-resolved, got {:?}", other),
    }

    // Navigation should point to the remaining unresolved region
    assert_eq!(session.next_unresolved_after(0), Some(1));
    assert_eq!(session.prev_unresolved_before(2), Some(1));
}

/// End-to-end test: conflict session for a modify/delete conflict
/// produces correct strategy and payloads, and the "keep" side can be
/// staged to resolve the conflict.
#[test]
fn conflict_session_modify_delete_keep_resolves_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base content\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    // Feature branch modifies the file
    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "modified by feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "modify"],
    );

    // Main branch deletes the file
    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify conflict session for modify/delete
    let session = opened
        .conflict_session(Path::new("a.txt"))
        .unwrap()
        .expect("conflict session for modify/delete");
    assert_eq!(
        session.strategy,
        ConflictResolverStrategy::TwoWayKeepDelete,
        "modify/delete conflicts should use TwoWayKeepDelete strategy"
    );
    assert_eq!(session.conflict_kind, FileConflictKind::DeletedByUs);

    // Ours deleted (absent), theirs has content
    assert!(
        session.ours.is_absent(),
        "ours (delete side) should be absent"
    );
    assert!(
        session.theirs.as_text().is_some(),
        "theirs (modify side) should have text"
    );
    assert_eq!(
        session.unsolved_count(),
        1,
        "two-way non-marker conflict sessions should expose one unresolved decision region"
    );
    assert_eq!(session.regions[0].ours, "");
    assert_eq!(session.regions[0].theirs, "modified by feature\n");

    // Resolve by keeping theirs (the modified version)
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Theirs)
        .unwrap();

    // Verify file is restored and no longer conflicted
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "modified by feature\n"
    );
    let status = opened.status().unwrap();
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted),
        "a.txt should no longer be conflicted after keeping theirs"
    );
}

/// Validates the safety gate: `validate_conflict_resolution_text` correctly
/// detects remaining markers in partially-resolved text.
#[test]
fn validate_conflict_resolution_detects_partial_resolution() {
    use gitcomet_core::services::validate_conflict_resolution_text;

    // Fully resolved text — no markers
    let clean = "line1\nline2\nline3\n";
    assert!(!validate_conflict_resolution_text(clean).has_conflict_markers);

    // Partially resolved — one conflict block remains
    let partial = concat!(
        "resolved section\n",
        "<<<<<<< HEAD\n",
        "ours\n",
        "=======\n",
        "theirs\n",
        ">>>>>>> feature\n",
        "another resolved section\n",
    );
    let v = validate_conflict_resolution_text(partial);
    assert!(v.has_conflict_markers);
    assert_eq!(v.marker_lines, 3); // <<<<<<<, =======, >>>>>>>

    // diff3-style markers
    let diff3 = concat!(
        "<<<<<<< HEAD\n",
        "ours\n",
        "||||||| base\n",
        "base\n",
        "=======\n",
        "theirs\n",
        ">>>>>>> feature\n",
    );
    let v3 = validate_conflict_resolution_text(diff3);
    assert!(v3.has_conflict_markers);
    assert_eq!(v3.marker_lines, 4); // <<<<<<<, |||||||, =======, >>>>>>>
}

/// End-to-end test: BothDeleted text conflict session uses DecisionOnly
/// strategy, and restoring from base via `checkout_conflict_side(Base)`
/// resolves the conflict.
#[test]
fn conflict_session_both_deleted_restore_from_base_resolves_conflict() {
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

    // BothDeleted: only base stage present, no ours or theirs
    let base_blob = hash_blob(repo, b"original content\n");
    set_unmerged_stages(repo, "removed.txt", Some(base_blob.as_str()), None, None);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("removed.txt"))
        .unwrap()
        .expect("conflict session for BothDeleted");
    assert_eq!(session.conflict_kind, FileConflictKind::BothDeleted);
    assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
    assert!(matches!(session.base, ConflictPayload::Text(ref t) if t == "original content\n"));
    assert!(session.ours.is_absent());
    assert!(session.theirs.is_absent());
    assert_eq!(session.unsolved_count(), 1);

    // Resolve by accepting deletion
    opened
        .accept_conflict_deletion(Path::new("removed.txt"))
        .unwrap();

    // Verify conflict is resolved
    let status = opened.status().unwrap();
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("removed.txt") && e.kind == FileStatusKind::Conflicted),
        "removed.txt should no longer be conflicted after accepting deletion"
    );
    assert!(
        !repo.join("removed.txt").exists(),
        "file should be deleted after accepting deletion"
    );
}

/// End-to-end test: AddedByUs conflict session uses TwoWayKeepDelete
/// strategy, and keeping the file via `checkout_conflict_side(Ours)`
/// resolves the conflict.
#[test]
fn conflict_session_added_by_us_keep_resolves_conflict() {
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

    // AddedByUs: only ours stage present (no base, no theirs)
    let ours_blob = hash_blob(repo, b"added by us\n");
    set_unmerged_stages(repo, "new.txt", None, Some(ours_blob.as_str()), None);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify status
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("new.txt"))
        .expect("expected AddedByUs conflict entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::AddedByUs));

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("new.txt"))
        .unwrap()
        .expect("conflict session for AddedByUs");
    assert_eq!(session.conflict_kind, FileConflictKind::AddedByUs);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.base.is_absent());
    assert!(matches!(session.ours, ConflictPayload::Text(ref t) if t == "added by us\n"));
    assert!(session.theirs.is_absent());
    assert_eq!(session.unsolved_count(), 1);

    // Resolve by keeping ours (the added file)
    opened
        .checkout_conflict_side(Path::new("new.txt"), ConflictSide::Ours)
        .unwrap();

    // Verify file exists and conflict is resolved
    assert_eq!(
        fs::read_to_string(repo.join("new.txt")).unwrap(),
        "added by us\n"
    );
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("new.txt") && e.kind == FileStatusKind::Conflicted),
        "new.txt should no longer be conflicted after keeping ours"
    );
    assert!(
        status_after
            .staged
            .iter()
            .any(|e| e.path == Path::new("new.txt")),
        "new.txt should be staged after resolution"
    );
}

/// End-to-end test: AddedByThem conflict session uses TwoWayKeepDelete
/// strategy, and keeping the file via `checkout_conflict_side(Theirs)`
/// resolves the conflict.
#[test]
fn conflict_session_added_by_them_keep_resolves_conflict() {
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

    // AddedByThem: only theirs stage present (no base, no ours)
    let theirs_blob = hash_blob(repo, b"added by them\n");
    set_unmerged_stages(
        repo,
        "their_new.txt",
        None,
        None,
        Some(theirs_blob.as_str()),
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify status
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("their_new.txt"))
        .expect("expected AddedByThem conflict entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::AddedByThem));

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("their_new.txt"))
        .unwrap()
        .expect("conflict session for AddedByThem");
    assert_eq!(session.conflict_kind, FileConflictKind::AddedByThem);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.base.is_absent());
    assert!(session.ours.is_absent());
    assert!(matches!(session.theirs, ConflictPayload::Text(ref t) if t == "added by them\n"));
    assert_eq!(session.unsolved_count(), 1);

    // Resolve by keeping theirs (the added file)
    opened
        .checkout_conflict_side(Path::new("their_new.txt"), ConflictSide::Theirs)
        .unwrap();

    // Verify file exists and conflict is resolved
    assert_eq!(
        fs::read_to_string(repo.join("their_new.txt")).unwrap(),
        "added by them\n"
    );
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("their_new.txt") && e.kind == FileStatusKind::Conflicted),
        "their_new.txt should no longer be conflicted after keeping theirs"
    );
    assert!(
        status_after
            .staged
            .iter()
            .any(|e| e.path == Path::new("their_new.txt")),
        "their_new.txt should be staged after resolution"
    );
}

/// End-to-end test: DeletedByThem conflict session uses TwoWayKeepDelete
/// strategy (base+ours present, theirs absent), and keeping ours
/// via `checkout_conflict_side(Ours)` resolves the conflict.
#[test]
fn conflict_session_deleted_by_them_keep_ours_resolves_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base content\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    // Feature branch deletes the file
    run_git(repo, &["checkout", "-b", "feature"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "delete"],
    );

    // Main branch modifies the file
    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "modified by us\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "modify"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify status shows DeletedByThem
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted)
        .expect("expected DeletedByThem conflict entry");
    assert_eq!(entry.conflict, Some(FileConflictKind::DeletedByThem));

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("a.txt"))
        .unwrap()
        .expect("conflict session for DeletedByThem");
    assert_eq!(session.conflict_kind, FileConflictKind::DeletedByThem);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.base.as_text().is_some());
    assert!(
        matches!(session.ours, ConflictPayload::Text(ref t) if t == "modified by us\n"),
        "ours (modified side) should have text"
    );
    assert!(
        session.theirs.is_absent(),
        "theirs (delete side) should be absent"
    );
    assert_eq!(session.unsolved_count(), 1);
    assert_eq!(session.regions[0].ours, "modified by us\n");
    assert_eq!(session.regions[0].theirs, "");

    // Resolve by keeping ours (the modified version)
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Ours)
        .unwrap();

    // Verify file is kept and conflict is resolved
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "modified by us\n"
    );
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted),
        "a.txt should no longer be conflicted after keeping ours"
    );
}
