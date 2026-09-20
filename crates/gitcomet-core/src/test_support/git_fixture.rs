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
