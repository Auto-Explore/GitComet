use gitcomet_core::conflict_session::{ConflictPayload, ConflictResolverStrategy};
use gitcomet_core::domain::{
    CommitId, Diff, DiffArea, DiffLineKind, DiffPreviewTextSide, DiffTarget, FileConflictKind,
    FileDiffText, FileDiffTextSource, FileStatusKind,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use gitcomet_core::services::{CancellationToken, CheckoutRemoteBranchMode, GitBackend};
use gitcomet_core::services::{ConflictSide, InteractiveRebaseAction, InteractiveRebaseEntry};
use gitcomet_core::test_support::git_fixture::{FixtureTimer, append_config, init_repository};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
#[cfg(unix)]
use std::{
    fs::Permissions,
    os::unix::fs::{PermissionsExt, symlink},
};

fn read_file_diff_text_source(source: Option<&FileDiffTextSource>) -> Option<String> {
    source.map(|source| {
        fs::read_to_string(&source.path).unwrap_or_else(|err| {
            panic!(
                "read file diff text source '{}': {err}",
                source.path.display()
            )
        })
    })
}

fn assert_file_diff_text_sources(diff: &FileDiffText, old: Option<&str>, new: Option<&str>) {
    assert_eq!(diff.old.as_deref(), None);
    assert_eq!(diff.new.as_deref(), None);
    assert_eq!(
        read_file_diff_text_source(diff.old_source.as_ref()).as_deref(),
        old
    );
    assert_eq!(
        read_file_diff_text_source(diff.new_source.as_ref()).as_deref(),
        new
    );
}

struct TestGitEnv {
    _root: tempfile::TempDir,
    global_config: PathBuf,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    gnupg_home: PathBuf,
}

fn ensure_isolated_git_test_env() -> &'static TestGitEnv {
    static ENV: OnceLock<TestGitEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("test git env tempdir");
        let home_dir = root.path().join("home");
        let xdg_config_home = root.path().join("xdg");
        let gnupg_home = root.path().join("gnupg");
        let global_config = root.path().join("gitconfig");

        fs::create_dir_all(&home_dir).expect("test git home");
        fs::create_dir_all(&xdg_config_home).expect("test git xdg config home");
        fs::create_dir_all(&gnupg_home).expect("test gnupg home");
        fs::write(&global_config, b"").expect("test global git config");

        #[cfg(unix)]
        fs::set_permissions(&gnupg_home, Permissions::from_mode(0o700))
            .expect("test gnupg home permissions");

        gitcomet_git_gix::install_test_git_command_environment(
            global_config.clone(),
            home_dir.clone(),
            xdg_config_home.clone(),
            gnupg_home.clone(),
        );

        TestGitEnv {
            _root: root,
            global_config,
            home_dir,
            xdg_config_home,
            gnupg_home,
        }
    })
}

fn git_path_arg(path: &Path) -> String {
    let path = path.to_str().expect("test path should be unicode");
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

fn git_remote_url(path: &Path) -> String {
    git_path_arg(path)
}

fn allow_repo_local_mergetool_cmd(repo: &Path, tool_name: &str) {
    let _ = ensure_isolated_git_test_env();
    gitcomet_git_gix::allow_test_repo_local_mergetool_command(repo, tool_name);
}

fn set_repo_local_mergetool_cmd_with_consent(repo: &Path, tool_name: &str, command: &str) {
    let cmd_key = format!("mergetool.{tool_name}.cmd");
    run_git(repo, &["config", &cmd_key, command]);
    allow_repo_local_mergetool_cmd(repo, tool_name);
}

fn git_command() -> Command {
    let env = ensure_isolated_git_test_env();
    let mut cmd = Command::new("git");
    // Keep integration tests deterministic by isolating from host git config.
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", &env.global_config);
    cmd.env("HOME", &env.home_dir);
    cmd.env("XDG_CONFIG_HOME", &env.xdg_config_home);
    cmd.env("GNUPGHOME", &env.gnupg_home);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GCM_INTERACTIVE", "Never");
    // Some scenarios clone local file:// remotes (submodules, temp-origin repos).
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd
}

fn run_git(repo: &Path, args: &[&str]) {
    run_git_output(repo, args);

    if args.first() == Some(&"init") {
        gitcomet_core::test_support::git_fixture::append_config_file(
            &repo.join(if args.contains(&"--bare") {
                "config"
            } else {
                ".git/config"
            }),
            &[
                ("core.autocrlf", "false"),
                ("core.eol", "lf"),
                ("credential.helper", ""),
                ("credential.interactive", "never"),
                ("protocol.file.allow", "always"),
            ],
        );
    }
}

fn run_git_expect_failure(repo: &Path, args: &[&str]) {
    let _timer = FixtureTimer::new("subprocess", args.first().copied().unwrap_or("git"));
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        !output.status.success(),
        "expected git {:?} to fail:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_output(repo: &Path, args: &[&str]) -> String {
    let _timer = FixtureTimer::new("subprocess", args.first().copied().unwrap_or("git"));
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn assert_git_failure(error: &Error, expected_command: &str, expected_id: GitFailureId) {
    match error.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), expected_command);
            assert_eq!(failure.id(), expected_id);
        }
        other => panic!("expected structured git error, got {other:?}"),
    }
}

fn write(repo: &Path, rel: &str, contents: impl AsRef<[u8]>) -> PathBuf {
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

fn init_conflict_fixture(repo: &Path) {
    init_repository(repo, |repo| {
        run_git(repo, &["init"]);
        append_config(
            repo,
            &[
                ("user.email", "you@example.com"),
                ("user.name", "You"),
                ("commit.gpgsign", "false"),
                ("mergetool.guiDefault", "false"),
                ("merge.guitool", ""),
            ],
        );
    });
}

fn setup_both_modified_text_conflict(repo: &Path, path: &str, ours: &str, theirs: &str) {
    init_conflict_fixture(repo);

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

#[cfg(unix)]
fn setup_both_modified_symlink_conflict(repo: &Path, path: &str, ours: &str, theirs: &str) {
    init_conflict_fixture(repo);

    let link = repo.join(path);
    let relink = |target: &str| {
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(target, &link).expect("symlink");
    };

    write(repo, "base.txt", "base\n");
    relink("base.txt");
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    relink(theirs);
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    relink(ours);
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);
}

fn setup_both_added_text_conflict(repo: &Path, path: &str, ours: &str, theirs: &str) {
    init_conflict_fixture(repo);

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

fn set_fixed_mtime(path: &Path) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
}

fn fixture_command(action: &str) -> &'static str {
    static COMMANDS: OnceLock<std::collections::HashMap<&str, String>> = OnceLock::new();
    COMMANDS.get_or_init(|| {
        let exe = env!("CARGO_BIN_EXE_gitcomet-test-mergetool");
        #[cfg(windows)]
        let quoted = format!("\"{}\"", exe.replace('%', "%%"));
        #[cfg(not(windows))]
        let quoted = format!("'{}'", exe.replace('\'', "'\"'\"'"));
        [
            "rewrite-fail",
            "delete-fail",
            "markers",
            "copy",
            "cli",
            "gui",
            "cmd",
            "paths-copy",
            "paths-fail",
            "base-size-copy",
        ]
        .into_iter()
        .map(|action| (action, format!("{quoted} {action}")))
        .collect()
    })[action]
        .as_str()
}

#[cfg(windows)]
fn cmd_exit_success() -> &'static str {
    "exit /b 0"
}
#[cfg(not(windows))]
fn cmd_exit_success() -> &'static str {
    "exit 0"
}

#[allow(dead_code)]
fn cmd_same_size_content_change_and_exit_failure() -> &'static str {
    fixture_command("rewrite-fail")
}

#[allow(dead_code)]
fn cmd_delete_merged_and_exit_failure() -> &'static str {
    fixture_command("delete-fail")
}

#[allow(dead_code)]
fn cmd_write_unresolved_markers_and_exit_success() -> &'static str {
    fixture_command("markers")
}

#[allow(dead_code)]
fn cmd_copy_remote_to_merged_and_exit_success() -> &'static str {
    fixture_command("copy")
}

#[allow(dead_code)]
fn cmd_write_cli_to_merged() -> &'static str {
    fixture_command("cli")
}

#[allow(dead_code)]
fn cmd_write_gui_to_merged() -> &'static str {
    fixture_command("gui")
}

#[allow(dead_code)]
fn cmd_write_cmd_to_merged() -> &'static str {
    fixture_command("cmd")
}

#[allow(dead_code)]
fn cmd_dump_stage_paths_and_copy_remote() -> &'static str {
    fixture_command("paths-copy")
}

#[allow(dead_code)]
fn cmd_dump_stage_paths_and_exit_failure() -> &'static str {
    fixture_command("paths-fail")
}

#[allow(dead_code)]
fn cmd_dump_base_size_and_copy_remote() -> &'static str {
    fixture_command("base-size-copy")
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

#[path = "status_integration/conflicts_and_mergetool.rs"]
mod conflicts_and_mergetool;
#[path = "status_integration/repository_operations.rs"]
mod repository_operations;
#[path = "status_integration/stash_and_staging.rs"]
mod stash_and_staging;
#[path = "status_integration/status_and_diff.rs"]
mod status_and_diff;
