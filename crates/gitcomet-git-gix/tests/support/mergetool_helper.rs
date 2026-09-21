//! Small native fixture: exercise the real shell/tool launch without PowerShell.
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, UNIX_EPOCH};

fn stage(name: &str) -> io::Result<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other(format!("missing {name}")))
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn run() -> io::Result<u8> {
    let action = env::args()
        .nth(1)
        .ok_or_else(|| io::Error::other("missing action"))?;
    let merged = stage("MERGED")?;
    match action.as_str() {
        "rewrite-fail" => {
            let bytes = fs::read(&merged)?;
            fs::write(&merged, vec![b'R'; bytes.len()])?;
            OpenOptions::new()
                .write(true)
                .open(&merged)?
                .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_000))?;
            return Ok(1);
        }
        "delete-fail" => {
            match fs::remove_file(&merged) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            return Ok(1);
        }
        "markers" => {
            let text = "<<<<<<< ours\nleft\n=======\nright\n>>>>>>> theirs\n";
            fs::write(
                &merged,
                if cfg!(windows) {
                    text.replace('\n', "\r\n")
                } else {
                    text.into()
                },
            )?;
        }
        "cli" | "gui" | "cmd" => fs::write(&merged, format!("{action}\n"))?,
        "copy" | "paths-copy" | "paths-fail" | "base-size-copy" => {
            if action.starts_with("paths-") {
                let paths = ["BASE", "LOCAL", "REMOTE"].map(stage);
                let mut output = String::new();
                for path in paths {
                    output.push_str(&path?.to_string_lossy());
                    output.push('\n');
                }
                fs::write(sidecar(&merged, ".env"), output)?;
            }
            if action == "paths-fail" {
                return Ok(1);
            }
            if action == "base-size-copy" {
                fs::write(
                    sidecar(&merged, ".base-size"),
                    fs::metadata(stage("BASE")?)?.len().to_string(),
                )?;
            }
            fs::write(&merged, fs::read(stage("REMOTE")?)?)?;
        }
        _ => return Err(io::Error::other(format!("unknown action: {action}"))),
    }
    Ok(0)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("mergetool fixture: {error}");
            ExitCode::from(2)
        }
    }
}
