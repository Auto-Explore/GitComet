use gitcomet_core::conflict_session::{ConflictPayload, ConflictResolverStrategy};
use gitcomet_core::domain::{
    CommitId, DiffArea, DiffLineKind, DiffPreviewTextSide, DiffTarget, FileConflictKind,
    FileDiffText, FileDiffTextSource, FileStatusKind,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use gitcomet_core::services::{CheckoutRemoteBranchMode, GitBackend};
use gitcomet_core::services::{ConflictSide, InteractiveRebaseAction, InteractiveRebaseEntry};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::{Duration, Instant};
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

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
const GIT_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(windows)]
const GIT_PROBE_WAIT_POLL: Duration = Duration::from_millis(50);

#[cfg(windows)]
fn run_command_with_timeout(mut cmd: Command) -> Option<std::process::Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if start.elapsed() >= GIT_PROBE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(GIT_PROBE_WAIT_POLL);
            }
            Err(_) => return None,
        }
    }
}

#[cfg(windows)]
fn git_shell_available_for_status_integration_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match run_command_with_timeout({
            let mut cmd = Command::new("git");
            cmd.args(["difftool", "--tool-help"]);
            cmd
        }) {
            Some(output) => output,
            None => return false,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

#[cfg(windows)]
fn git_local_push_available_for_status_integration_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(_) => return true,
        };
        let remote_repo = dir.path().join("probe-remote.git");
        let work_repo = dir.path().join("probe-work");
        if fs::create_dir_all(&remote_repo).is_err() || fs::create_dir_all(&work_repo).is_err() {
            return true;
        }

        let init_remote = match run_command_with_timeout({
            let mut cmd = git_command();
            cmd.arg("-C").arg(&remote_repo).args(["init", "--bare"]);
            cmd
        }) {
            Some(output) => output.status.success(),
            None => false,
        };
        if !init_remote {
            return true;
        }

        let init_work = match run_command_with_timeout({
            let mut cmd = git_command();
            cmd.arg("-C").arg(&work_repo).args(["init"]);
            cmd
        }) {
            Some(output) => output.status.success(),
            None => false,
        };
        if !init_work {
            return true;
        }

        for args in [
            ["config", "user.email", "you@example.com"].as_slice(),
            ["config", "user.name", "You"].as_slice(),
            ["config", "commit.gpgsign", "false"].as_slice(),
            ["config", "core.autocrlf", "false"].as_slice(),
            ["config", "core.eol", "lf"].as_slice(),
        ] {
            let output = match run_command_with_timeout({
                let mut cmd = git_command();
                cmd.arg("-C").arg(&work_repo).args(args);
                cmd
            }) {
                Some(output) => output,
                None => return false,
            };
            if !output.status.success() {
                return true;
            }
        }

        if fs::write(work_repo.join("probe.txt"), "probe\n").is_err() {
            return true;
        }

        for args in [
            ["add", "probe.txt"].as_slice(),
            ["-c", "commit.gpgsign=false", "commit", "-m", "probe"].as_slice(),
        ] {
            let output = match run_command_with_timeout({
                let mut cmd = git_command();
                cmd.arg("-C").arg(&work_repo).args(args);
                cmd
            }) {
                Some(output) => output,
                None => return false,
            };
            if !output.status.success() {
                return true;
            }
        }

        let remote_url = git_remote_url(&remote_repo);
        let add_remote = match run_command_with_timeout({
            let mut cmd = git_command();
            cmd.arg("-C")
                .arg(&work_repo)
                .args(["remote", "add", "origin", remote_url.as_str()]);
            cmd
        }) {
            Some(output) => output.status.success(),
            None => false,
        };
        if !add_remote {
            return true;
        }

        let push_output = match run_command_with_timeout({
            let mut cmd = git_command();
            cmd.arg("-C")
                .arg(&work_repo)
                .args(["push", "-u", "origin", "HEAD"]);
            cmd
        }) {
            Some(output) => output,
            None => return false,
        };
        if push_output.status.success() {
            return true;
        }

        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&push_output.stdout),
            String::from_utf8_lossy(&push_output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_status_integration_tests() -> bool {
    let _ = ensure_isolated_git_test_env();
    #[cfg(windows)]
    {
        if !git_shell_available_for_status_integration_tests() {
            eprintln!(
                "skipping status integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
        if !git_local_push_available_for_status_integration_tests() {
            eprintln!(
                "skipping status integration test: Git-for-Windows local push shell startup failed in this environment"
            );
            return false;
        }
    }
    true
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
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);

    if args.first() == Some(&"init") {
        // Keep text-file assertions deterministic across platforms, regardless
        // of host/user git defaults.
        run_git(repo, &["config", "core.autocrlf", "false"]);
        run_git(repo, &["config", "core.eol", "lf"]);
        // Avoid host credential manager prompts/retries in backend commands.
        run_git(repo, &["config", "credential.helper", ""]);
        run_git(repo, &["config", "credential.interactive", "never"]);
        // Ensure local file:// remotes are always usable in this test repo.
        run_git(repo, &["config", "protocol.file.allow", "always"]);
    }
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

fn run_git_output(repo: &Path, args: &[&str]) -> String {
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
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
    // `touch -d` is GNU-specific; `-t [[CC]YY]MMDDhhmm[.ss]` is supported on
    // both GNU/Linux and BSD/macOS.
    let status = Command::new("touch")
        .arg("-t")
        .arg("202311142213.20")
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
    "len=$(wc -c < \"$MERGED\"); head -c \"$len\" /dev/zero | tr '\\0' 'R' > \"$MERGED\"; touch -t 202311142213.20 \"$MERGED\"; exit 1"
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
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, 'cli' + [char]10)""#
}

#[cfg(not(windows))]
fn cmd_write_cli_to_merged() -> &'static str {
    "printf 'cli\\n' > \"$MERGED\""
}

#[cfg(windows)]
fn cmd_write_gui_to_merged() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, 'gui' + [char]10)""#
}

#[cfg(not(windows))]
fn cmd_write_gui_to_merged() -> &'static str {
    "printf 'gui\\n' > \"$MERGED\""
}

#[allow(dead_code)]
#[cfg(windows)]
fn cmd_write_cmd_to_merged() -> &'static str {
    r#"powershell -NoProfile -Command "[System.IO.File]::WriteAllText($env:MERGED, 'cmd' + [char]10)""#
}

#[allow(dead_code)]
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

#[path = "status_integration/conflicts_and_mergetool.rs"]
mod conflicts_and_mergetool;
#[path = "status_integration/repository_operations.rs"]
mod repository_operations;
#[path = "status_integration/stash_and_staging.rs"]
mod stash_and_staging;
#[path = "status_integration/status_and_diff.rs"]
mod status_and_diff;
