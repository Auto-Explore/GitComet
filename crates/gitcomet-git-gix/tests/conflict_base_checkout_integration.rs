use gitcomet_core::domain::FileStatusKind;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

fn run_git(repo: &Path, args: &[&str]) {
    let status = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn hash_blob(repo: &Path, contents: &[u8]) -> String {
    let mut child = gitcomet_core::test_support::git_fixture::git_command()
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

    let mut child = gitcomet_core::test_support::git_fixture::git_command()
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

#[test]
fn checkout_conflict_base_restores_non_utf8_stage_and_resolves_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    fs::write(repo.join("seed.txt"), b"seed\n").unwrap();
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let base_bytes = vec![0x00, 0x9f, 0x92, 0x96, 0xff, b'\n'];
    let base_blob = hash_blob(repo, &base_bytes);
    set_unmerged_stages(repo, "bin.dat", Some(base_blob.as_str()), None, None);

    let opened = GixBackend.open(repo).unwrap();
    let before = opened.status().unwrap();
    assert!(
        before
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("bin.dat") && e.kind == FileStatusKind::Conflicted),
        "expected staged-shape fixture to appear as conflicted"
    );

    opened.checkout_conflict_base(Path::new("bin.dat")).unwrap();

    assert_eq!(fs::read(repo.join("bin.dat")).unwrap(), base_bytes);
    let after = opened.status().unwrap();
    assert!(
        after
            .unstaged
            .iter()
            .all(|e| !(e.path == Path::new("bin.dat") && e.kind == FileStatusKind::Conflicted)),
        "expected bin.dat conflict to be cleared after base restore; status={after:?}"
    );
    assert!(
        after.staged.iter().any(|e| e.path == Path::new("bin.dat")),
        "expected restored base path to be staged; status={after:?}"
    );
}

#[test]
fn checkout_conflict_base_errors_when_base_stage_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    fs::write(repo.join("seed.txt"), b"seed\n").unwrap();
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let ours_blob = hash_blob(repo, b"ours\n");
    let theirs_blob = hash_blob(repo, b"theirs\n");
    set_unmerged_stages(
        repo,
        "a.txt",
        None,
        Some(ours_blob.as_str()),
        Some(theirs_blob.as_str()),
    );

    let opened = GixBackend.open(repo).unwrap();
    let before = opened.status().unwrap();
    assert!(
        before
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted),
        "expected fixture to appear as conflicted"
    );

    let err = opened
        .checkout_conflict_base(Path::new("a.txt"))
        .expect_err("base checkout should fail when stage-1 is absent");
    let msg = format!("{err}");
    assert!(
        msg.contains("base conflict stage is not available"),
        "unexpected error: {msg}"
    );

    let after = opened.status().unwrap();
    assert!(
        after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted),
        "expected conflict to remain unresolved when base stage is missing"
    );
}
