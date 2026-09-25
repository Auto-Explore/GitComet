//! Setup for ordinary integration-test repositories, never for testing `git init` itself.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Append fixture-only values, with Git's quoted-string escaping. Call once per
/// key on a fresh repository; use real Git for later changes and config tests.
pub fn append_config(repo: &Path, values: &[(&str, &str)]) {
    append_config_file(&repo.join(".git/config"), values);
}

/// As `append_config`, for the config path of a bare fixture repository.
pub fn append_config_file(path: &Path, values: &[(&str, &str)]) {
    let _timer = FixtureTimer::new("config", "write");
    let mut text = String::new();
    for (key, value) in values {
        let (section, rest) = key.split_once('.').expect("section.key");
        assert!(
            section
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        );
        let (subsection, name) = match rest.rsplit_once('.') {
            Some((subsection, name)) => (Some(subsection), name),
            None => (None, rest),
        };
        assert!(name.starts_with(|c: char| c.is_ascii_alphabetic()));
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        text.push_str(&format!("\n[{section}"));
        if let Some(subsection) = subsection {
            assert!(!subsection.contains(['\n', '\r', '\0']));
            text.push_str(&format!(" \"{}\"", quote(subsection)));
        }
        text.push_str(&format!("]\n\t{name} = \"{}\"\n", quote(value)));
    }
    OpenOptions::new()
        .append(true)
        .open(path)
        .expect("existing fixture config")
        .write_all(text.as_bytes())
        .expect("write fixture config");
}

fn quote(value: &str) -> String {
    assert!(
        !value.contains(['\0', '\r']),
        "unsupported Git config string"
    );
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\u{8}', "\\b")
}

/// One ordinary single-file commit. Use real porcelain for hooks, signing,
/// filters, index behavior, merges, or tests of the commit operation itself.
pub struct LinearCommit<'a> {
    pub author: &'a str,
    pub timestamp: i64,
    pub message: &'a str,
    pub path: &'a str,
    pub contents: &'a str,
}

/// Import a linear history into a fresh branch. The caller supplies a command
/// with its isolated environment and repository already selected, then checks
/// out the branch separately if the test needs an index/worktree.
pub fn import_linear_history<'a>(
    command: &mut std::process::Command,
    branch: &str,
    commits: impl IntoIterator<Item = LinearCommit<'a>>,
) {
    let _timer = FixtureTimer::new("subprocess", "fast-import");
    assert!(!branch.contains(['\n', '\r', '\0']));
    let mut stream = String::new();
    for (index, commit) in commits.into_iter().enumerate() {
        assert!(!commit.author.contains(['\n', '\r', '\0']));
        // fast-import accepts Git's C-style quoted paths, including spaces,
        // UTF-8, quotes and newlines. Its paths always use forward slashes.
        assert!(
            commit
                .path
                .split('/')
                .all(|component| !matches!(component, "" | "." | ".."))
        );
        stream.push_str(&format!(
            "commit refs/heads/{branch}\nmark :{}\nauthor {} {} +0000\ncommitter You <you@example.com> {} +0000\ndata {}\n{}\n",
            index + 1, commit.author, commit.timestamp, commit.timestamp,
            commit.message.len(), commit.message,
        ));
        if index > 0 {
            stream.push_str(&format!("from :{index}\n"));
        }
        stream.push_str(&format!(
            "M 100644 inline \"{}\"\ndata {}\n{}\n",
            quote(commit.path),
            commit.contents.len(),
            commit.contents
        ));
    }
    stream.push_str("done\n");
    let mut child = command
        .args(["fast-import", "--quiet", "--done"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("start git fast-import");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .expect("write import stream");
    assert!(child.wait().unwrap().success(), "git fast-import failed");
}

/// Initialize each fixture in its own repository and record setup time.
pub fn init_repository(repo: &Path, initialize: impl FnOnce(&Path)) {
    let _timer = FixtureTimer::new("setup", "init-repository");
    initialize(repo);
}

/// Optional per-process TSV, collected as a CI artifact. Setup totals and nested
/// subprocess timings are separate categories, not additive wall-clock time.
pub struct FixtureTimer(Option<(Instant, PathBuf, String, String)>);

impl FixtureTimer {
    pub fn new(phase: &str, operation: &str) -> Self {
        Self(
            std::env::var_os("GITCOMET_CI_FIXTURE_TIMINGS")
                .filter(|v| !v.is_empty())
                .map(|path| {
                    (
                        Instant::now(),
                        PathBuf::from(path),
                        phase.to_owned(),
                        operation.to_owned(),
                    )
                }),
        )
    }
}

impl Drop for FixtureTimer {
    fn drop(&mut self) {
        let Some((start, directory, phase, operation)) = &self.0 else {
            return;
        };
        let elapsed = start.elapsed().as_micros();
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let result = (|| -> std::io::Result<()> {
            fs::create_dir_all(directory)?;
            let mut file = OpenOptions::new()
                .append(true)
                .create(true)
                .open(directory.join(format!("{}.tsv", std::process::id())))?;
            let thread = std::thread::current();
            let name = thread
                .name()
                .unwrap_or("unnamed")
                .replace(['\t', '\n', '\r'], " ");
            let operation = operation.replace(['\t', '\n', '\r'], " ");
            writeln!(file, "{name}\t{phase}\t{operation}\t{elapsed}")
        })();
        if let Err(error) = result {
            eprintln!("fixture timing report failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn imported_history_preserves_parent_authors_messages_and_file_bytes() {
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "")
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        git(&["init", "-b", "main"]);
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(repo.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "");
        import_linear_history(
            &mut command,
            "main",
            [
                LinearCommit {
                    author: "First <first@example.com>",
                    timestamp: 1_600_000_000,
                    message: "日本語\nbody",
                    path: "file.txt",
                    contents: "first\n",
                },
                LinearCommit {
                    author: "Second <second@example.com>",
                    timestamp: 1_600_000_001,
                    message: "second",
                    path: "file.txt",
                    contents: "last without newline",
                },
            ],
        );
        assert_eq!(git(&["rev-list", "--count", "main"]), "2\n");
        assert_eq!(git(&["show", "main^:file.txt"]), "first\n");
        assert_eq!(git(&["show", "main:file.txt"]), "last without newline");
        assert_eq!(
            git(&["log", "-1", "--format=%an|%at|%s", "main"]),
            "Second|1600000001|second\n"
        );
        assert_eq!(
            git(&["log", "-1", "--format=%B", "main^"]),
            "日本語\nbody\n"
        );
        git(&["-c", "core.autocrlf=false", "reset", "--hard", "main"]);
        assert_eq!(
            fs::read_to_string(repo.path().join("file.txt")).unwrap(),
            "last without newline"
        );
        assert!(git(&["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn imported_paths_preserve_spaces_quotes_controls_and_utf8() {
        let repo = tempfile::tempdir().unwrap();
        let command = || {
            let mut cmd = Command::new("git");
            cmd.arg("-C")
                .arg(repo.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "");
            cmd
        };
        assert!(
            command()
                .args(["init", "-b", "main"])
                .output()
                .unwrap()
                .status
                .success()
        );
        let paths = [
            "docs/spaced 日本語 file.txt",
            // Windows cannot check out these characters in filenames.
            if cfg!(windows) {
                "docs/other.txt"
            } else {
                "docs/\"quote\\tab\tline\n.txt"
            },
        ];
        import_linear_history(
            &mut command(),
            "main",
            paths.iter().map(|path| LinearCommit {
                author: "You <you@example.com>",
                timestamp: 1_600_000_000,
                message: "path fixture",
                path,
                contents: "bytes\n",
            }),
        );
        assert!(
            command()
                .args(["-c", "core.autocrlf=false", "reset", "--hard", "main"])
                .output()
                .unwrap()
                .status
                .success()
        );
        for path in paths {
            assert_eq!(fs::read(repo.path().join(path)).unwrap(), b"bytes\n");
        }
        let status = command().args(["status", "--porcelain"]).output().unwrap();
        assert!(status.status.success());
        assert!(status.stdout.is_empty());
    }

    #[test]
    fn config_roundtrips_shell_commands_and_subsections_through_git() {
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir(repo.path().join(".git")).unwrap();
        fs::write(
            repo.path().join(".git/config"),
            "[core]\nrepositoryformatversion = 0\n",
        )
        .unwrap();
        let values = [
            ("user.name", " # Unicode 日本語 ; \"quoted\" "),
            (
                "mergetool.fixture.cmd",
                "\"C:\\Program Files\\tool.exe\" '$REMOTE'\n\tend",
            ),
            ("remote.a.quoted\"name.url", "path with spaces"),
        ];
        append_config(repo.path(), &values);
        for (key, value) in values {
            let output = Command::new("git")
                .args(["config", "--file"])
                .arg(repo.path().join(".git/config"))
                .args(["--get", key])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{value}\n")
            );
        }
    }
}
