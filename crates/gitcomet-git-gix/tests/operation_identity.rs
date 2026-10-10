//! Per-operation overrides must never change another repository's config.
use gitcomet_core::commit_identity::CommitIdentity;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::path::Path;
use std::process::Command;

#[path = "support/test_git_env.rs"]
mod test_git_env;

fn git(path: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(path).args(args);
    test_git_env::apply(&mut command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn identity_is_scoped_to_the_commit_and_amend_keeps_the_original_author() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.name", "Configured"]);
    git(
        root.path(),
        &["config", "user.email", "configured@example.invalid"],
    );
    let config = std::fs::read(root.path().join(".git/config")).unwrap();
    std::fs::write(root.path().join("file"), "one").unwrap();
    git(root.path(), &["add", "file"]);
    let repository = GixBackend.open(root.path()).unwrap();
    let identity = CommitIdentity {
        name: "Project".into(),
        email: "project@example.invalid".into(),
    };
    repository
        .commit_with_identity("one", Some(&identity))
        .unwrap();
    assert_eq!(
        git(root.path(), &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]),
        "Project <project@example.invalid>|Project <project@example.invalid>"
    );
    let second = CommitIdentity {
        name: "Second".into(),
        email: "second@example.invalid".into(),
    };
    repository
        .commit_amend_with_identity("amended", Some(&second))
        .unwrap();
    assert_eq!(
        git(root.path(), &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]),
        "Project <project@example.invalid>|Second <second@example.invalid>"
    );
    std::fs::write(root.path().join("file"), "two").unwrap();
    git(root.path(), &["add", "file"]);
    repository.commit_with_identity("two", None).unwrap();
    assert_eq!(
        git(root.path(), &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]),
        "Configured <configured@example.invalid>|Configured <configured@example.invalid>"
    );
    assert_eq!(
        std::fs::read(root.path().join(".git/config")).unwrap(),
        config
    );
    let invalid = CommitIdentity {
        name: "bad\nname".into(),
        email: "email".into(),
    };
    assert!(
        repository
            .commit_amend_with_identity("invalid", Some(&invalid))
            .is_err()
    );
    assert_eq!(git(root.path(), &["log", "-1", "--format=%s"]), "two");
}
