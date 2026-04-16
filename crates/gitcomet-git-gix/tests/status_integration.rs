use gitcomet_core::conflict_session::{ConflictPayload, ConflictResolverStrategy};
use gitcomet_core::domain::{
    CommitId, DiffArea, DiffLineKind, DiffPreviewTextSide, DiffTarget, FileConflictKind,
    FileStatusKind,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use gitcomet_core::services::ConflictSide;
use gitcomet_core::services::GitBackend;
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
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

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

#[test]
fn status_separates_staged_and_unstaged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn repeated_status_on_same_repo_instance_reuses_staged_state_and_invalidates_on_index_change() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "base\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened.status().unwrap();
    assert_eq!(first.staged.len(), 1);
    assert_eq!(first.staged[0].path, PathBuf::from("a.txt"));
    assert!(first.unstaged.is_empty());

    write(repo, "b.txt", "base\nworktree\n");
    let second = opened.status().unwrap();
    assert_eq!(second.staged.len(), 1);
    assert_eq!(second.staged[0].path, PathBuf::from("a.txt"));
    assert_eq!(second.unstaged.len(), 1);
    assert_eq!(second.unstaged[0].path, PathBuf::from("b.txt"));
    assert_eq!(second.unstaged[0].kind, FileStatusKind::Modified);

    run_git(repo, &["add", "b.txt"]);
    let third = opened.status().unwrap();
    assert_eq!(third.staged.len(), 2);
    assert!(
        third
            .staged
            .iter()
            .any(|entry| entry.path == Path::new("a.txt"))
    );
    assert!(
        third
            .staged
            .iter()
            .any(|entry| entry.path == Path::new("b.txt"))
    );
    assert!(third.unstaged.is_empty());
}

#[test]
fn status_does_not_rewrite_index_when_only_worktree_stat_is_stale() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    set_fixed_mtime(&repo.join("a.txt"));
    let index_before = fs::read(repo.join(".git").join("index")).unwrap();

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());

    let index_after = fs::read(repo.join(".git").join("index")).unwrap();
    assert_eq!(
        index_after, index_before,
        "status should not rewrite the index for metadata-only worktree changes"
    );
}

#[test]
fn repeated_status_does_not_rewrite_index_when_cached_staged_state_is_reused() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    let first = opened.status().unwrap();
    assert!(first.staged.is_empty());
    assert!(first.unstaged.is_empty());

    set_fixed_mtime(&repo.join("a.txt"));
    let index_before = fs::read(repo.join(".git").join("index")).unwrap();

    let second = opened.status().unwrap();
    assert!(second.staged.is_empty());
    assert!(second.unstaged.is_empty());

    let index_after = fs::read(repo.join(".git").join("index")).unwrap();
    assert_eq!(
        index_after, index_before,
        "cached repeated status should stay read-only for metadata-only worktree changes"
    );
}

#[test]
fn repeated_status_on_same_repo_instance_invalidates_when_head_moves_without_index_change() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let clean = opened.status().unwrap();
    assert!(clean.staged.is_empty());
    assert!(clean.unstaged.is_empty());

    run_git(repo, &["reset", "--soft", "HEAD~1"]);

    let after_reset = opened.status().unwrap();
    assert_eq!(after_reset.staged.len(), 1);
    assert_eq!(after_reset.staged[0].path, Path::new("a.txt"));
    assert_eq!(after_reset.staged[0].kind, FileStatusKind::Modified);
    assert!(after_reset.unstaged.is_empty());
}

#[test]
fn status_lists_untracked_files_in_directories() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn status_ignores_nested_target_directories_with_target_slash_pattern() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, ".gitignore", "target/\n");
    run_git(repo, &["add", ".gitignore"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init ignore"],
    );

    write(
        repo,
        "crates/gitcomet-ui-gpui/target/criterion/report/index.html",
        "ignored\n",
    );
    write(repo, "visible.txt", "untracked\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    assert!(
        status.unstaged.iter().all(|entry| !entry
            .path
            .starts_with(Path::new("crates/gitcomet-ui-gpui/target"))),
        "expected nested target/ contents to be ignored, got {status:?}"
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|entry| entry.path == Path::new("visible.txt")
                && entry.kind == FileStatusKind::Untracked),
        "expected visible.txt as untracked, got {status:?}"
    );
}

#[test]
fn diff_unified_works_for_staged_and_unstaged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn diff_working_tree_unstaged_ignores_crlf_only_line_ending_changes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\r\ntwo\r\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("a.txt"),
        area: DiffArea::Unstaged,
    };

    let unified = opened.diff_unified(&target).unwrap();
    assert!(
        unified.trim().is_empty(),
        "expected CRLF-only unstaged diff to be suppressed:\n{unified}"
    );

    let parsed = opened.diff_parsed(&target).unwrap();
    assert!(
        parsed.lines.is_empty(),
        "expected parsed diff to be empty for CRLF-only unstaged change: {parsed:?}"
    );
}

#[test]
fn diff_file_text_reports_old_and_new_for_working_tree_and_commits() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
            commit_id: gitcomet_core::domain::CommitId(head.into()),
            path: Some(PathBuf::from("a.txt")),
        })
        .unwrap()
        .expect("file diff for commit");
    assert_eq!(commit.old.as_deref(), Some("one\n"));
    assert_eq!(commit.new.as_deref(), Some("one\ntwo\n"));
}

#[test]
fn diff_file_text_unstaged_uses_git_normalized_worktree_content() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "true"]);

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\r\ntwo\r\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let diff = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("file diff for unstaged crlf-only change");

    assert_eq!(diff.old.as_deref(), Some("one\ntwo\n"));
    assert_eq!(
        diff.new.as_deref(),
        Some("one\ntwo\n"),
        "expected worktree text to match Git-normalized staged content"
    );
}

#[test]
fn diff_file_text_root_commit_has_no_parent_side() {
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
        &["-c", "commit.gpgsign=false", "commit", "-m", "root"],
    );

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse to run");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let commit = opened
        .diff_file_text(&DiffTarget::Commit {
            commit_id: gitcomet_core::domain::CommitId(head.into()),
            path: Some(PathBuf::from("a.txt")),
        })
        .unwrap()
        .expect("file diff for root commit");
    assert_eq!(commit.old.as_deref(), None);
    assert_eq!(commit.new.as_deref(), Some("one\n"));
}

#[test]
fn diff_file_text_staged_add_and_delete_report_missing_sides() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn diff_preview_text_file_commit_added_file_returns_new_side_blob_path() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "docs/added.txt", "one\ntwo");
    run_git(repo, &["add", "docs/added.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "add file"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let commit_id = CommitId(run_git_output(repo, &["rev-parse", "HEAD"]).into());
    let preview_path = opened
        .diff_preview_text_file(
            &DiffTarget::Commit {
                commit_id,
                path: Some(PathBuf::from("docs/added.txt")),
            },
            DiffPreviewTextSide::New,
        )
        .unwrap()
        .expect("preview text file for committed added file");

    assert!(preview_path.is_file());
    assert_eq!(
        fs::read_to_string(&preview_path).expect("read committed added preview text file"),
        "one\ntwo"
    );
}

#[test]
fn diff_preview_text_file_commit_deleted_file_returns_old_side_blob_path() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "docs/delete-me.txt", "one\ntwo");
    run_git(repo, &["add", "docs/delete-me.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );
    run_git(repo, &["rm", "docs/delete-me.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "delete file"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let commit_id = CommitId(run_git_output(repo, &["rev-parse", "HEAD"]).into());
    let preview_path = opened
        .diff_preview_text_file(
            &DiffTarget::Commit {
                commit_id,
                path: Some(PathBuf::from("docs/delete-me.txt")),
            },
            DiffPreviewTextSide::Old,
        )
        .unwrap()
        .expect("preview text file for committed deleted file");

    assert!(preview_path.is_file());
    assert_eq!(
        fs::read_to_string(&preview_path).expect("read committed deleted preview text file"),
        "one\ntwo"
    );
}

#[test]
fn diff_preview_text_file_staged_deleted_file_returns_head_blob_path() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    run_git(repo, &["rm", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let preview_path = opened
        .diff_preview_text_file(
            &DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Staged,
            },
            DiffPreviewTextSide::Old,
        )
        .unwrap()
        .expect("preview text file for staged deleted file");

    assert!(preview_path.is_file());
    assert_eq!(
        fs::read_to_string(&preview_path).expect("read staged deleted preview text file"),
        "one\n"
    );
}

#[test]
fn diff_file_text_returns_none_for_directories() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    run_git(repo, &["add", "dir/a.txt"]);
    let staged_result = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("dir"),
            area: DiffArea::Staged,
        })
        .unwrap();

    assert!(staged_result.is_none());
}

#[test]
fn diff_file_image_reports_old_and_new_for_working_tree_and_commits() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let old_png = png_1x1_rgba(0, 0, 0, 255);
    let new_png = png_1x1_rgba(255, 0, 0, 255);

    write(repo, "img.png", &old_png);
    run_git(repo, &["add", "img.png"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "img.png", &new_png);

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
            commit_id: gitcomet_core::domain::CommitId(head.into()),
            path: Some(PathBuf::from("img.png")),
        })
        .unwrap()
        .expect("image diff for commit");
    assert_eq!(commit.old.as_deref(), Some(old_png.as_slice()));
    assert_eq!(commit.new.as_deref(), Some(new_png.as_slice()));
}

#[test]
fn diff_file_image_returns_none_for_directories() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    run_git(repo, &["add", "dir/a.png"]);
    let staged_result = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: PathBuf::from("dir"),
            area: DiffArea::Staged,
        })
        .unwrap();

    assert!(staged_result.is_none());
}

#[test]
fn gitlink_added_and_unstaged_modified_reports_expected_status_and_diff() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let nested = repo.join("chess3");

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(&nested).expect("create nested repo path");
    run_git(&nested, &["init"]);
    run_git(&nested, &["config", "user.email", "you@example.com"]);
    run_git(&nested, &["config", "user.name", "You"]);
    run_git(&nested, &["config", "commit.gpgsign", "false"]);

    write(&nested, "file.txt", "one\n");
    run_git(&nested, &["add", "file.txt"]);
    run_git(
        &nested,
        &["-c", "commit.gpgsign=false", "commit", "-m", "nested c1"],
    );

    run_git(repo, &["add", "chess3"]);

    write(&nested, "file.txt", "one\ntwo\n");
    run_git(&nested, &["add", "file.txt"]);
    run_git(
        &nested,
        &["-c", "commit.gpgsign=false", "commit", "-m", "nested c2"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("chess3") && e.kind == FileStatusKind::Added),
        "expected staged Added gitlink entry; status={status:?}"
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("chess3") && e.kind == FileStatusKind::Modified),
        "expected unstaged Modified gitlink entry; status={status:?}"
    );

    let diff = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("chess3"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert!(
        diff.contains("Subproject commit"),
        "expected unstaged gitlink unified diff to include subproject commit line; diff={diff}"
    );

    let file_text = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("chess3"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert!(
        file_text.is_none(),
        "expected no direct file text payload for directory-backed gitlink target"
    );
}

#[test]
fn committed_gitlink_unstaged_modified_reports_modified_status_and_diff() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let nested = repo.join("chess3");

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(&nested).expect("create nested repo path");
    run_git(&nested, &["init"]);
    run_git(&nested, &["config", "user.email", "you@example.com"]);
    run_git(&nested, &["config", "user.name", "You"]);
    run_git(&nested, &["config", "commit.gpgsign", "false"]);

    write(&nested, "file.txt", "one\n");
    run_git(&nested, &["add", "file.txt"]);
    run_git(
        &nested,
        &["-c", "commit.gpgsign=false", "commit", "-m", "nested c1"],
    );

    run_git(repo, &["add", "chess3"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "add gitlink"],
    );

    write(&nested, "file.txt", "one\ntwo\n");
    run_git(&nested, &["add", "file.txt"]);
    run_git(
        &nested,
        &["-c", "commit.gpgsign=false", "commit", "-m", "nested c2"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let status = opened.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("chess3") && e.kind == FileStatusKind::Modified),
        "expected unstaged Modified gitlink entry after nested repo advances; status={status:?}"
    );

    let diff = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("chess3"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert!(
        diff.contains("Subproject commit"),
        "expected unstaged gitlink unified diff to include subproject commit line; diff={diff}"
    );
}

#[test]
fn status_cache_invalidates_when_gitlink_appears_on_same_repo_instance() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let nested = repo.join("chess3");

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let initial_status = opened.status().unwrap();
    assert!(
        initial_status.staged.is_empty() && initial_status.unstaged.is_empty(),
        "expected clean repo before adding gitlink; status={initial_status:?}"
    );

    std::fs::create_dir_all(&nested).expect("create nested repo path");
    run_git(&nested, &["init"]);
    run_git(&nested, &["config", "user.email", "you@example.com"]);
    run_git(&nested, &["config", "user.name", "You"]);
    run_git(&nested, &["config", "commit.gpgsign", "false"]);

    write(&nested, "file.txt", "one\n");
    run_git(&nested, &["add", "file.txt"]);
    run_git(
        &nested,
        &["-c", "commit.gpgsign=false", "commit", "-m", "nested c1"],
    );

    run_git(repo, &["add", "chess3"]);

    let staged_gitlink = opened.status().unwrap();
    assert!(
        staged_gitlink
            .staged
            .iter()
            .any(|e| e.path == Path::new("chess3") && e.kind == FileStatusKind::Added),
        "expected staged Added gitlink entry after cached clean status; status={staged_gitlink:?}"
    );

    write(&nested, "file.txt", "one\ntwo\n");
    run_git(&nested, &["add", "file.txt"]);
    run_git(
        &nested,
        &["-c", "commit.gpgsign=false", "commit", "-m", "nested c2"],
    );

    let advanced_gitlink = opened.status().unwrap();
    assert!(
        advanced_gitlink
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("chess3") && e.kind == FileStatusKind::Modified),
        "expected cached gitlink capability to keep reporting nested repo advances; status={advanced_gitlink:?}"
    );
}

#[test]
fn diff_file_commit_target_without_path_returns_none() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    let head = run_git_output(repo, &["rev-parse", "HEAD"]);
    let target = DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId(head.into()),
        path: None,
    };

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    assert!(opened.diff_file_text(&target).unwrap().is_none());
    assert!(opened.diff_file_image(&target).unwrap().is_none());
}

#[test]
fn diff_unified_outside_repository_path_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let outside = dir.path().join("outside.txt");
    fs::create_dir_all(&repo).unwrap();
    fs::write(&outside, "outside\n").unwrap();

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

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let err = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: outside,
            area: DiffArea::Unstaged,
        })
        .expect_err("expected diff_unified to fail for outside path");
    assert_git_failure(&err, "git diff", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!("assert_git_failure() already checked the error kind");
    };
    assert_eq!(failure.exit_code(), Some(128));
}

#[test]
fn diff_parsed_outside_repository_path_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let outside = dir.path().join("outside.txt");
    fs::create_dir_all(&repo).unwrap();
    fs::write(&outside, "outside\n").unwrap();

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

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let err = opened
        .diff_parsed(&DiffTarget::WorkingTree {
            path: outside,
            area: DiffArea::Unstaged,
        })
        .expect_err("expected diff_parsed to fail for outside path");
    assert_git_failure(&err, "git diff", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!("assert_git_failure() already checked the error kind");
    };
    assert_eq!(failure.exit_code(), Some(128));
}

#[test]
fn diff_parsed_commit_rename_preserves_rename_headers_and_hunks() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "diff.renames", "true"]);

    write(repo, "docs/source.txt", "one\ntwo\n");
    run_git(repo, &["add", "docs/source.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    fs::create_dir_all(repo.join("docs/renamed")).unwrap();
    fs::rename(
        repo.join("docs/source.txt"),
        repo.join("docs/renamed/target.txt"),
    )
    .unwrap();
    fs::write(repo.join("docs/renamed/target.txt"), "one\ntwo\nthree\n").unwrap();
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "rename"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let commit_id = CommitId(run_git_output(repo, &["rev-parse", "HEAD"]).into());
    let diff = opened
        .diff_parsed(&DiffTarget::Commit {
            commit_id,
            path: None,
        })
        .expect("parse rename commit diff");

    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Header
                && line.text.as_ref() == "rename from docs/source.txt")
    );
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Header
                && line.text.as_ref() == "rename to docs/renamed/target.txt")
    );
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Hunk && line.text.as_ref().starts_with("@@")),
    );
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Add && line.text.as_ref() == "+three"),
    );
}

#[test]
fn diff_parsed_commit_added_file_matches_git_show_output() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "docs/added.txt", "one\ntwo");
    run_git(repo, &["add", "docs/added.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "add file"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let commit_id = CommitId(run_git_output(repo, &["rev-parse", "HEAD"]).into());
    let diff = opened
        .diff_parsed(&DiffTarget::Commit {
            commit_id: commit_id.clone(),
            path: Some(PathBuf::from("docs/added.txt")),
        })
        .expect("parse added file commit diff");
    let expected = run_git_output(
        repo,
        &[
            "show",
            "--no-ext-diff",
            "--pretty=format:",
            commit_id.as_ref(),
            "--",
            "docs/added.txt",
        ],
    );
    let actual = diff
        .lines
        .iter()
        .map(|line| line.text.as_ref())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(actual, expected.trim_end_matches('\n'));
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Header
                && line.text.as_ref().starts_with("new file mode ")),
    );
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Context
                && line.text.as_ref() == "\\ No newline at end of file"),
    );
}

#[test]
fn diff_parsed_commit_deleted_file_matches_git_show_output() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "docs/delete-me.txt", "one\ntwo");
    run_git(repo, &["add", "docs/delete-me.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );
    run_git(repo, &["rm", "docs/delete-me.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "delete file"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let commit_id = CommitId(run_git_output(repo, &["rev-parse", "HEAD"]).into());
    let diff = opened
        .diff_parsed(&DiffTarget::Commit {
            commit_id: commit_id.clone(),
            path: Some(PathBuf::from("docs/delete-me.txt")),
        })
        .expect("parse deleted file commit diff");
    let expected = run_git_output(
        repo,
        &[
            "show",
            "--no-ext-diff",
            "--pretty=format:",
            commit_id.as_ref(),
            "--",
            "docs/delete-me.txt",
        ],
    );
    let actual = diff
        .lines
        .iter()
        .map(|line| line.text.as_ref())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(actual, expected.trim_end_matches('\n'));
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Header
                && line.text.as_ref().starts_with("deleted file mode ")),
    );
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Context
                && line.text.as_ref() == "\\ No newline at end of file"),
    );
}

#[test]
fn diff_working_tree_with_absolute_file_path_reads_current_file() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    let absolute = repo.join("a.txt");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let text = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: absolute.clone(),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("text diff for absolute path");
    assert_eq!(text.old, None);
    assert_eq!(text.new.as_deref(), Some("one\ntwo\n"));

    let image = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: absolute,
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("image diff for absolute path");
    assert_eq!(image.old, None);
    assert_eq!(image.new.as_deref(), Some("one\ntwo\n".as_bytes()));
}

#[test]
fn staged_diff_for_unmerged_conflict_prefers_ours_for_text_and_image() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let text = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .expect("staged text diff for conflict");
    assert_eq!(text.old.as_deref(), Some("ours\n"));
    assert_eq!(text.new.as_deref(), Some("ours\n"));

    let image = opened
        .diff_file_image(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap()
        .expect("staged image diff for conflict");
    assert_eq!(image.old.as_deref(), Some("ours\n".as_bytes()));
    assert_eq!(image.new.as_deref(), Some("ours\n".as_bytes()));
}

#[test]
fn diff_commit_with_unknown_revision_and_outside_conflict_path_are_handled() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let outside = dir.path().join("outside.txt");
    fs::create_dir_all(&repo).unwrap();
    fs::write(&outside, "outside\n").unwrap();

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

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let unknown_target = DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("not-a-real-revision".into()),
        path: Some(PathBuf::from("a.txt")),
    };

    let text = opened
        .diff_file_text(&unknown_target)
        .unwrap()
        .expect("text diff object for unknown revision");
    assert_eq!(text.old, None);
    assert_eq!(text.new, None);

    let image = opened
        .diff_file_image(&unknown_target)
        .unwrap()
        .expect("image diff object for unknown revision");
    assert_eq!(image.old, None);
    assert_eq!(image.new, None);

    let err = opened
        .conflict_session(&outside)
        .expect_err("outside absolute path should be rejected");
    assert!(
        matches!(err.kind(), ErrorKind::Backend(_)),
        "expected backend error for outside path, got {err:?}"
    );
}

#[test]
fn diff_file_text_uses_ours_and_theirs_for_conflicted_paths() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        if stages.base.is_some() {
            assert!(
                stages.base_bytes.is_none(),
                "utf-8 base stage should not retain duplicate bytes for {}",
                fixture.path
            );
        }
        assert_eq!(
            stages.ours.is_some(),
            fixture.has_ours,
            "ours stage mismatch for {}",
            fixture.path
        );
        if stages.ours.is_some() {
            assert!(
                stages.ours_bytes.is_none(),
                "utf-8 ours stage should not retain duplicate bytes for {}",
                fixture.path
            );
        }
        assert_eq!(
            stages.theirs.is_some(),
            fixture.has_theirs,
            "theirs stage mismatch for {}",
            fixture.path
        );
        if stages.theirs.is_some() {
            assert!(
                stages.theirs_bytes.is_none(),
                "utf-8 theirs stage should not retain duplicate bytes for {}",
                fixture.path
            );
        }

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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let base_bytes = b"\x00base\xff\n".to_vec();
    let ours_bytes = b"\x00ours\xff\n".to_vec();
    let theirs_bytes = b"\x00theirs\xff\n".to_vec();

    write(repo, "bin.dat", &base_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "bin.dat", &theirs_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "bin.dat", &ours_bytes);
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let base_bytes = b"\x00base\xff\n".to_vec();
    let ours_bytes = b"\x00ours\xff\n".to_vec();
    let theirs_bytes = b"\x00theirs\xff\n".to_vec();

    write(repo, "bin.dat", &base_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "bin.dat", &theirs_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "bin.dat", &ours_bytes);
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    // Normalize pre-tool mtime to a fixed timestamp so metadata-only checks
    // cannot detect the edit when the command restores mtime.
    set_fixed_mtime(&repo.join("a.txt"));

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_same_size_content_change_and_exit_failure(),
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
fn launch_mergetool_reflects_config_written_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_copy_remote_to_merged_and_exit_success(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "theirs\n");

    let status = opened.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt"))
    );
    assert!(
        status
            .staged
            .iter()
            .any(|entry| entry.path == Path::new("a.txt") && entry.kind == FileStatusKind::Modified),
        "expected mergetool resolution after config refresh, got {status:?}"
    );
}

#[test]
fn launch_mergetool_trust_exit_false_requires_content_change() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_exit_success());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_delete_merged_and_exit_failure());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_write_unresolved_markers_and_exit_success(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/a space.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        "cat \"${REMOTE}\" > \"${MERGED}\"; exit 0",
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/a space.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        "copy /Y \"%REMOTE%\" \"%MERGED%\" > NUL && exit /b 0",
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(
        fs::read_to_string(repo.join(conflicted_path)).unwrap(),
        "theirs\n"
    );
}

#[test]
fn launch_mergetool_custom_cmd_supports_unicode_conflicted_path() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/spaced 日本語 file.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_copy_remote_to_merged_and_exit_success(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "cli"]);
    run_git(repo, &["config", "merge.guitool", "gui"]);
    run_git(repo, &["config", "mergetool.guiDefault", "true"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "cli", cmd_write_cli_to_merged());
    set_repo_local_mergetool_cmd_with_consent(repo, "gui", cmd_write_gui_to_merged());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
            git_path_arg(&script_path).as_str(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
            git_path_arg(&script_path).as_str(),
        ],
    );
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_write_cmd_to_merged());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_dump_stage_paths_and_exit_failure(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_dump_stage_paths_and_exit_failure(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_added_text_conflict(repo, "new.txt", "ours added\n", "theirs added\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_base_size_and_copy_remote());
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn unstage_empty_paths_with_head_unstages_all_index_changes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "base\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    write(repo, "b.txt", "base\nnext\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.unstage(&[]).unwrap();

    let staged = run_git_output(repo, &["diff", "--cached", "--name-only"]);
    assert!(
        staged.is_empty(),
        "expected empty staged diff, got {staged:?}"
    );

    let unstaged = run_git_output(repo, &["diff", "--name-only"]);
    assert!(
        unstaged.lines().any(|line| line == "a.txt"),
        "expected a.txt to be unstaged-modified, got {unstaged:?}"
    );
    assert!(
        unstaged.lines().any(|line| line == "b.txt"),
        "expected b.txt to be unstaged-modified, got {unstaged:?}"
    );
}

#[test]
fn unstage_empty_paths_without_head_unstages_all_added_paths() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "two\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.unstage(&[]).unwrap();

    let staged = run_git_output(repo, &["diff", "--cached", "--name-only"]);
    assert!(
        staged.is_empty(),
        "expected empty staged diff, got {staged:?}"
    );

    let short = run_git_output(repo, &["status", "--short"]);
    assert!(
        short.lines().any(|line| line == "?? a.txt"),
        "expected a.txt to be untracked after unstage-all, got {short:?}"
    );
    assert!(
        short.lines().any(|line| line == "?? b.txt"),
        "expected b.txt to be untracked after unstage-all, got {short:?}"
    );
}

#[test]
fn unstage_paths_without_head_only_unstages_selected_entries() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "two\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.unstage(&[Path::new("a.txt")]).unwrap();

    let short = run_git_output(repo, &["status", "--short"]);
    assert!(
        short.lines().any(|line| line == "?? a.txt"),
        "expected a.txt to be untracked after targeted unstage, got {short:?}"
    );
    assert!(
        short.lines().any(|line| line == "A  b.txt"),
        "expected b.txt to remain staged after targeted unstage, got {short:?}"
    );
}

#[test]
fn commit_creates_new_commit_and_cleans_status() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        .revert(&gitcomet_core::domain::CommitId(c2.clone().into()))
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn squash_ref_stages_changes_without_creating_merge_commit() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    let output = opened
        .squash_ref_with_output("feature")
        .expect("squash should succeed");
    assert_eq!(output.exit_code, Some(0));

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
    assert_eq!(parent_count, 1, "squash should not create a merge commit");

    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .any(|f| f.path.as_path() == Path::new("b.txt")),
        "expected squashed changes to be staged"
    );
}

#[test]
fn merge_commit_message_is_available_during_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
fn rebase_in_progress_and_abort_round_trip() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "main"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    run_git(repo, &["checkout", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(!opened.rebase_in_progress().unwrap());
    assert!(opened.rebase_with_output("main").is_err());
    assert!(opened.rebase_in_progress().unwrap());

    opened.rebase_abort_with_output().unwrap();
    assert!(!opened.rebase_in_progress().unwrap());
}

#[test]
fn rebase_continue_without_in_progress_rebase_returns_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.rebase_continue_with_output().is_err());
}

#[test]
fn rebase_abort_falls_back_to_git_am_abort() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    let patch_output = git_command()
        .arg("-C")
        .arg(repo)
        .args(["format-patch", "-1", "HEAD", "--stdout"])
        .output()
        .expect("git format-patch to run");
    assert!(
        patch_output.status.success(),
        "git format-patch failed: {}",
        String::from_utf8_lossy(&patch_output.stderr)
    );

    let patch_file = tempfile::NamedTempFile::new().expect("create patch temp file");
    fs::write(patch_file.path(), &patch_output.stdout).expect("write patch file");

    run_git(repo, &["checkout", "main"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.apply_patch_with_output(patch_file.path()).is_err());
    assert!(
        opened.rebase_in_progress().unwrap(),
        "expected apply-patch sequencer state to be in progress"
    );

    let abort_output = opened.rebase_abort_with_output().unwrap();
    assert_eq!(
        abort_output.command, "git am --abort",
        "expected rebase abort fallback to use git am --abort"
    );
    assert!(!opened.rebase_in_progress().unwrap());

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "main\n");
}

#[test]
fn merge_abort_with_output_clears_conflict_state() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert!(opened.merge_commit_message().unwrap().is_some());

    opened.merge_abort_with_output().unwrap();

    assert!(opened.merge_commit_message().unwrap().is_none());
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn create_and_delete_local_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout)
        .expect("HEAD is utf-8")
        .trim()
        .to_owned();

    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId(head.into()))
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
fn create_branch_existing_branch_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["branch", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_branch("feature", &gitcomet_core::domain::CommitId(head.into()))
        .expect_err("creating an existing branch should fail");
    assert_git_failure(&err, "git branch", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!();
    };
    assert_eq!(failure.exit_code(), Some(128));
    assert_eq!(
        failure.detail(),
        Some("fatal: a branch named 'feature' already exists")
    );
}

#[test]
fn create_branch_on_unborn_head_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("HEAD".into()))
        .expect_err("creating a branch on unborn HEAD should fail");
    assert_git_failure(&err, "git branch", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!();
    };
    assert_eq!(failure.exit_code(), Some(128));
    assert_eq!(
        failure.detail(),
        Some("fatal: not a valid object name: 'HEAD'")
    );

    let exists = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref feature");
    assert!(!exists.success(), "feature branch should not be created");
}

#[test]
fn create_branch_from_detached_head_using_head_revision() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "first"],
    );

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );

    let first_commit = run_git_output(repo, &["rev-parse", "HEAD~1"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_commit(&gitcomet_core::domain::CommitId(
            first_commit.clone().into(),
        ))
        .unwrap();
    opened
        .create_branch("rescue", &gitcomet_core::domain::CommitId("HEAD".into()))
        .unwrap();

    let rescue_target = run_git_output(repo, &["rev-parse", "rescue"]);
    assert_eq!(rescue_target, first_commit);
}

#[test]
fn create_branch_from_annotated_tag_peels_to_commit() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["tag", "-a", "v1", "-m", "v1"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("v1".into()))
        .unwrap();

    let feature_target = run_git_output(repo, &["rev-parse", "feature"]);
    assert_eq!(feature_target, head);
    let feature_kind = run_git_output(repo, &["cat-file", "-t", "feature"]);
    assert_eq!(feature_kind, "commit");
}

#[test]
fn create_branch_from_blob_target_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let blob = hash_blob(repo, b"blob target\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_branch(
            "feature",
            &gitcomet_core::domain::CommitId(blob.clone().into()),
        )
        .expect_err("creating a branch from a blob should fail");
    assert_git_failure(&err, "git branch", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!();
    };
    assert_eq!(failure.exit_code(), Some(128));
    let detail = failure.detail().expect("git detail");
    assert!(
        detail.contains("not a valid branch point"),
        "unexpected create-branch detail: {detail}"
    );
    assert!(
        detail.contains(&blob),
        "expected blob id in create-branch detail: {detail}"
    );
}

#[test]
fn create_branch_head_target_reflects_move_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "first"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let second_commit = run_git_output(repo, &["rev-parse", "HEAD"]);

    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("HEAD".into()))
        .unwrap();

    let feature_target = run_git_output(repo, &["rev-parse", "feature"]);
    assert_eq!(feature_target, second_commit);
}

#[test]
fn create_branch_target_branch_created_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    run_git(repo, &["branch", "source"]);

    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("source".into()))
        .unwrap();

    let feature_target = run_git_output(repo, &["rev-parse", "feature"]);
    assert_eq!(feature_target, head);
}

#[test]
fn create_branch_succeeds_without_persisted_user_identity() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "user.email=you@example.com",
            "-c",
            "user.name=You",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "init",
        ],
    );

    let head = run_git_output(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId(head.into()))
        .unwrap();

    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );
}

#[test]
fn checkout_branch_switches_head_to_target_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.checkout_branch("feature").unwrap();

    let head = run_git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "feature");
}

#[test]
fn delete_branch_force_removes_unmerged_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "feature.txt", "feature\n");
    run_git(repo, &["add", "feature.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let err = opened
        .delete_branch("feature")
        .expect_err("safe delete should fail for unmerged branch");
    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git branch -d");
            let msg = failure.to_string();
            assert!(
                msg.contains("not fully merged") || msg.contains("cannot delete branch"),
                "unexpected delete-branch error: {msg}"
            );
        }
        other => panic!("expected structured git error, got {other:?}"),
    }

    opened.delete_branch_force("feature").unwrap();

    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected force-delete to remove branch");
}

#[test]
fn delete_branch_force_removes_branch_config_section() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);
    run_git(repo, &["config", "branch.feature.remote", "origin"]);
    run_git(
        repo,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.delete_branch_force("feature").unwrap();

    let branch_config = git_command()
        .arg("-C")
        .arg(repo)
        .args(["config", "--local", "--get-regexp", "^branch\\.feature\\."])
        .status()
        .expect("git config --get-regexp");
    assert_eq!(
        branch_config.code(),
        Some(1),
        "expected branch config section to be removed"
    );
}

#[test]
fn delete_branch_force_keeps_branch_config_when_local_config_is_locked() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);
    run_git(repo, &["config", "branch.feature.remote", "origin"]);
    run_git(
        repo,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    );
    fs::write(repo.join(".git").join("config.lock"), b"held elsewhere").unwrap();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.delete_branch_force("feature").unwrap();

    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected force-delete to remove branch");

    let branch_config = git_command()
        .arg("-C")
        .arg(repo)
        .args(["config", "--local", "--get-regexp", "^branch\\.feature\\."])
        .output()
        .expect("git config --get-regexp");
    assert!(
        branch_config.status.success(),
        "expected branch config section to remain when .git/config is locked"
    );
    let branch_config_stdout = String::from_utf8_lossy(&branch_config.stdout);
    assert!(
        branch_config_stdout.contains("branch.feature.remote origin")
            && branch_config_stdout.contains("branch.feature.merge refs/heads/feature"),
        "unexpected branch config after locked cleanup skip: {branch_config_stdout}"
    );
}

#[test]
fn delete_branch_force_missing_branch_is_structured_git_failure() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .delete_branch_force("missing")
        .expect_err("missing branch must surface as a git command failure");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("branch 'missing' not found"),
        "unexpected delete-branch-force error: {msg}"
    );
}

#[test]
fn delete_branch_force_rejects_unborn_current_branch_before_missing_ref_check() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .delete_branch_force("main")
        .expect_err("unborn checked-out branch must still be treated as in-use");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("used by worktree") && msg.contains(&git_path_arg(repo)),
        "unexpected delete-branch-force error: {msg}"
    );
}

#[test]
fn delete_branch_force_rejects_branch_checked_out_in_linked_worktree() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let linked_worktree = dir.path().join("feature-worktree");

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);

    let linked_worktree_arg = git_path_arg(&linked_worktree);
    run_git(repo, &["worktree", "add", &linked_worktree_arg, "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .delete_branch_force("feature")
        .expect_err("branch checked out in linked worktree must not be deleted");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("used by worktree") && msg.contains(&linked_worktree_arg),
        "unexpected delete-branch-force error: {msg}"
    );

    let still_exists = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(
        still_exists.success(),
        "branch should remain after linked-worktree rejection"
    );
}

#[test]
fn delete_branch_force_rejects_branch_checked_out_in_main_worktree_when_opened_from_linked() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let linked_worktree = dir.path().join("feature-worktree");

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);

    let linked_worktree_arg = git_path_arg(&linked_worktree);
    run_git(repo, &["worktree", "add", &linked_worktree_arg, "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&linked_worktree).unwrap();
    let err = opened
        .delete_branch_force("main")
        .expect_err("main-worktree branch use must block deletion from linked worktree");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("used by worktree") && msg.contains(&git_path_arg(repo)),
        "unexpected delete-branch-force error: {msg}"
    );

    let still_exists = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/main"])
        .status()
        .expect("show-ref");
    assert!(
        still_exists.success(),
        "branch should remain after main-worktree rejection"
    );
}

#[test]
fn cherry_pick_applies_commit_onto_current_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature commit",
        ],
    );
    let feature_sha = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .cherry_pick(&gitcomet_core::domain::CommitId(feature_sha.into()))
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn create_and_delete_local_tag() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);

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
    let tag_type = git_command()
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "-t", "refs/tags/v1.0.0"])
        .output()
        .expect("cat-file");
    assert!(
        tag_type.status.success(),
        "expected refs/tags/v1.0.0 to exist"
    );
    assert_eq!(String::from_utf8_lossy(&tag_type.stdout).trim(), "tag");

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
fn create_tag_respects_tag_gpgsign_config() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "true"]);
    run_git(
        repo,
        &["config", "gpg.program", "gitcomet-missing-gpg-program"],
    );

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_tag_with_output("v1.0.0", "HEAD")
        .expect_err("tag creation should fail when signing is required and gpg is missing");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git tag -m v1.0.0 -- v1.0.0 HEAD");
            let msg = failure.to_string();
            assert!(
                msg.contains("git tag -m v1.0.0 -- v1.0.0 HEAD failed"),
                "unexpected git error: {msg}"
            );
            let lower = msg.to_ascii_lowercase();
            assert!(
                msg.contains("gitcomet-missing-gpg-program") || lower.contains("sign"),
                "expected signing failure details in git error: {msg}"
            );
        }
        other => panic!("expected structured git error, got {other:?}"),
    }

    let tag_present = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(
        !tag_present.success(),
        "tag should not exist when signing failed"
    );
}

#[test]
fn list_tags_returns_sorted_names_with_commit_targets() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    run_git(repo, &["tag", "-a", "a-first", "-m", "a-first"]);
    run_git(repo, &["tag", "z-last"]);
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let tags = opened.list_tags().unwrap();

    let names = tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, vec!["a-first", "z-last"]);
    assert!(tags.iter().all(|tag| tag.target.as_ref() == head));
}

#[test]
fn list_remote_tags_collects_sorted_results_and_skips_unavailable_remote() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    let backup = dir.path().join("backup.git");
    let missing = dir.path().join("missing.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&backup).unwrap();

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
    run_git(&backup, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(
        &repo,
        &["remote", "add", "backup", git_remote_url(&backup).as_str()],
    );
    run_git(
        &repo,
        &["remote", "add", "broken", git_remote_url(&missing).as_str()],
    );

    run_git(&repo, &["tag", "origin-tag"]);
    run_git(&repo, &["tag", "backup-tag"]);
    run_git(&repo, &["push", "origin", "refs/tags/origin-tag"]);
    run_git(&repo, &["push", "backup", "refs/tags/backup-tag"]);

    let head = run_git_output(&repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let remote_tags = opened.list_remote_tags().unwrap();
    let tuples = remote_tags
        .iter()
        .map(|tag| {
            (
                tag.remote.as_str(),
                tag.name.as_str(),
                tag.target.as_ref().to_string(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        tuples,
        vec![
            ("backup", "backup-tag", head.clone()),
            ("origin", "origin-tag", head)
        ]
    );
}

#[test]
fn push_and_delete_remote_tag() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "tag.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
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
fn prune_local_tags_with_output_no_remotes_is_noop() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
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
    run_git(&repo, &["tag", "local-only"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let output = opened.prune_local_tags_with_output().unwrap();

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stdout
            .contains("No remotes configured; skipping tag prune."),
        "unexpected stdout: {}",
        output.stdout
    );
    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/local-only"],
    );
}

#[test]
fn prune_local_tags_with_output_reports_noop_when_all_tags_exist_remotely() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["tag", "v1.0.0"]);
    run_git(&repo, &["push", "origin", "refs/tags/v1.0.0"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let output = opened.prune_local_tags_with_output().unwrap();

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.contains("No local tags to prune."),
        "unexpected stdout: {}",
        output.stdout
    );
    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );
}

#[test]
fn list_remote_branches_includes_fetched_remote_tracking_refs() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
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
fn checkout_remote_branch_creates_tracking_branch_when_missing_locally() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "feature.txt", "feature\n");
    run_git(&seed, &["add", "feature.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    opened
        .checkout_remote_branch("origin", "feature", "feature")
        .unwrap();

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "feature");

    let upstream = run_git_output(
        &clone,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn checkout_remote_branch_existing_local_branch_updates_upstream_and_checks_out() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "feature.txt", "feature\n");
    run_git(&seed, &["add", "feature.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );
    run_git(&clone, &["checkout", "-b", "topic"]);
    run_git(&clone, &["checkout", "main"]);

    let upstream_before = git_command()
        .arg("-C")
        .arg(&clone)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "topic@{upstream}",
        ])
        .status()
        .expect("topic upstream probe");
    assert!(
        !upstream_before.success(),
        "topic should start without upstream tracking"
    );

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    opened
        .checkout_remote_branch("origin", "feature", "topic")
        .unwrap();

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "topic");
    let upstream = run_git_output(
        &clone,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn checkout_remote_branch_sees_local_branch_created_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "feature.txt", "feature\n");
    run_git(&seed, &["add", "feature.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();

    run_git(&clone, &["checkout", "-b", "topic"]);
    run_git(&clone, &["checkout", "main"]);

    let upstream_before = git_command()
        .arg("-C")
        .arg(&clone)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "topic@{upstream}",
        ])
        .status()
        .expect("topic upstream probe");
    assert!(
        !upstream_before.success(),
        "topic should start without upstream tracking"
    );

    opened
        .checkout_remote_branch("origin", "feature", "topic")
        .unwrap();

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "topic");
    let upstream = run_git_output(
        &clone,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn checkout_remote_branch_returns_structured_git_error_for_missing_remote_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&repo).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

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
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let err = opened
        .checkout_remote_branch("origin", "missing-branch", "topic")
        .expect_err("missing remote branch should return structured git error");
    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.id(), GitFailureId::CommandFailed);
            assert_eq!(failure.command(), "git checkout --track");
            assert!(
                failure.exit_code().is_some(),
                "git checkout failure should preserve exit code"
            );
            assert!(
                failure
                    .detail()
                    .is_some_and(|detail| !detail.trim().is_empty()),
                "git checkout failure should preserve stderr detail"
            );
        }
        other => panic!("expected structured git error, got {other:?}"),
    }
}

#[test]
fn checkout_remote_branch_with_existing_local_branch_and_missing_remote_keeps_head_unchanged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );
    run_git(&clone, &["checkout", "-b", "topic"]);
    run_git(&clone, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    let err = opened
        .checkout_remote_branch("origin", "missing-branch", "topic")
        .expect_err("missing remote branch should not switch to the existing local branch");
    assert_git_failure(&err, "git checkout --track", GitFailureId::CommandFailed);

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "main");

    let upstream = git_command()
        .arg("-C")
        .arg(&clone)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "topic@{upstream}",
        ])
        .status()
        .expect("topic upstream probe");
    assert!(
        !upstream.success(),
        "topic should remain without upstream tracking after the failed checkout"
    );
}

#[test]
fn checkout_remote_branch_dirty_worktree_failure_does_not_create_local_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "a.txt", "feature\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );
    write(&clone, "a.txt", "dirty\n");

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    let err = opened
        .checkout_remote_branch("origin", "feature", "topic")
        .expect_err("dirty checkout should fail");
    assert_git_failure(&err, "git checkout --track", GitFailureId::CommandFailed);

    let topic_exists = git_command()
        .arg("-C")
        .arg(&clone)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/topic"])
        .status()
        .expect("show-ref topic");
    assert!(
        !topic_exists.success(),
        "topic branch should not be created when checkout fails"
    );

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "main");
    assert_eq!(fs::read_to_string(clone.join("a.txt")).unwrap(), "dirty\n");
}

#[test]
fn push_with_output_updates_remote_head() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&repo_b).as_str(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&repo_b).as_str(),
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert_git_failure(&err, "git stash apply", GitFailureId::StashApplyConflict);
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::WorktreeWouldBeOverwritten,
    );
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
fn stash_apply_tracked_payload_overwriting_untracked_file_is_worktree_overwrite() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "new.txt", "from stash\n");
    run_git(repo, &["add", "new.txt"]);
    opened.stash_create("wip", false).unwrap();

    write(repo, "new.txt", "local untracked\n");

    let err = opened.stash_apply(0).expect_err(
        "stash apply should fail when tracked stash payload would overwrite an untracked file",
    );
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::WorktreeWouldBeOverwritten,
    );
    assert!(
        err.to_string().contains("overwritten by merge"),
        "unexpected error: {err}"
    );

    assert_eq!(
        fs::read_to_string(repo.join("new.txt")).unwrap(),
        "local untracked\n"
    );
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("new.txt"))
        .expect("expected blocked untracked file to remain in the worktree");
    assert_eq!(entry.kind, FileStatusKind::Untracked);
    assert_eq!(entry.conflict, None);
}

#[test]
fn stash_apply_staged_overlap_still_merges_into_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    write(repo, "a.txt", "base\nlocal-staged-change\n");
    run_git(repo, &["add", "a.txt"]);

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report a conflict when only the index overlaps");
    assert_git_failure(&err, "git stash apply", GitFailureId::StashApplyConflict);

    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("a.txt"))
        .expect("expected conflicted file after stash apply merge");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::BothModified));
}

#[test]
fn stash_apply_allows_merge_when_only_untracked_restore_fails() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
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
fn stash_apply_preserves_original_error_when_untracked_merge_markers_fail() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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

    let local_binary = b"\xff\xfe\x00\x80";
    write(repo, "Cargo.toml.orig", local_binary);

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should still report the original untracked restore failure");
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
    assert!(
        !err.to_string().contains("cannot merge binary"),
        "unexpected recovery error replaced the original stash failure: {err}"
    );
    assert_eq!(
        fs::read(repo.join("Cargo.toml.orig")).unwrap(),
        local_binary,
    );
}

#[test]
fn stash_apply_allows_untracked_restore_failure_when_stash_has_tracked_payload() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        .checkout_commit(&gitcomet_core::domain::CommitId(sha.clone().into()))
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert!(
        matches!(session.base, ConflictPayload::Text(ref t) if t.as_ref() == "original content\n")
    );
    assert!(session.ours.is_absent());
    assert!(session.theirs.is_absent());
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert!(matches!(session.ours, ConflictPayload::Text(ref t) if t.as_ref() == "added by us\n"));
    assert!(session.theirs.is_absent());
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
    assert!(
        matches!(session.theirs, ConflictPayload::Text(ref t) if t.as_ref() == "added by them\n")
    );
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
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
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
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
        matches!(session.ours, ConflictPayload::Text(ref t) if t.as_ref() == "modified by us\n"),
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
