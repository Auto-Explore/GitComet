//! Setup for ordinary integration-test repositories, never for testing `git init` itself.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
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

/// An empty, configured repository whose files are never handed to a test.
/// Every copy gets independent config, refs, objects, locks and index files.
pub struct RepositorySeed(tempfile::TempDir);

impl RepositorySeed {
    pub fn new(initialize: impl FnOnce(&Path)) -> Self {
        let directory = tempfile::tempdir().expect("seed directory");
        initialize(directory.path());
        assert!(directory.path().join(".git/config").is_file());
        assert!(
            !directory.path().join(".git/index").exists(),
            "seed must be empty"
        );
        Self(directory)
    }

    pub fn copy_to(&self, repo: &Path) {
        fn copy_directory(source: &Path, destination: &Path) {
            fs::create_dir(destination).expect("new fixture directory");
            for entry in fs::read_dir(source).expect("seed entries") {
                let entry = entry.expect("seed entry");
                let kind = entry.file_type().expect("seed file type");
                let destination = destination.join(entry.file_name());
                if kind.is_dir() {
                    copy_directory(&entry.path(), &destination);
                } else {
                    assert!(kind.is_file(), "repository seed must not contain symlinks");
                    // No hardlinks: changing one fixture must not mutate its seed.
                    fs::copy(entry.path(), destination).expect("copy seed file");
                }
            }
        }
        copy_directory(&self.0.path().join(".git"), &repo.join(".git"));
    }
}

/// Windows libtest runs many tests in one process, amortizing one `git init`.
/// On Unix, nextest normally runs one test per process; copying adds no benefit.
pub fn init_repository(
    repo: &Path,
    seed: &'static OnceLock<RepositorySeed>,
    initialize: impl FnOnce(&Path),
) {
    let _timer = FixtureTimer::new("setup", "init-repository");
    if cfg!(windows) {
        seed.get_or_init(|| RepositorySeed::new(initialize))
            .copy_to(repo);
    } else {
        initialize(repo);
    }
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

    #[test]
    fn seed_copies_have_independent_config_refs_objects_and_indexes() {
        let seed = RepositorySeed::new(|path| {
            fs::create_dir_all(path.join(".git/refs/heads")).unwrap();
            fs::create_dir_all(path.join(".git/objects")).unwrap();
            fs::write(path.join(".git/config"), "seed config").unwrap();
            fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        });
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        seed.copy_to(a.path());
        seed.copy_to(b.path());
        fs::write(a.path().join(".git/config"), "changed config").unwrap();
        for name in ["index", "index.lock", "refs/heads/main", "objects/new"] {
            fs::write(a.path().join(".git").join(name), "private").unwrap();
            assert!(!b.path().join(".git").join(name).exists());
        }
        assert_eq!(
            fs::read_to_string(b.path().join(".git/config")).unwrap(),
            "seed config"
        );
        assert_eq!(
            fs::read_to_string(seed.0.path().join(".git/config")).unwrap(),
            "seed config"
        );
    }
}
