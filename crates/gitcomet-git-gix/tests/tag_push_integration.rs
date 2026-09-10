use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_core::tag_push::{TagPushMode, TagPushRequest};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "protocol.file.allow=always",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

struct Fixture {
    _dir: tempfile::TempDir,
    local: PathBuf,
    remote: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("local");
        let remote = dir.path().join("remote.git");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "--bare"]);
        git(&local, &["init", "-b", "main"]);
        git(&local, &["config", "user.name", "Tag push test"]);
        git(&local, &["config", "user.email", "tag-push@example.test"]);
        git(&local, &["config", "push.gpgSign", "false"]);
        git(&local, &["commit", "--allow-empty", "-m", "initial"]);
        git(
            &local,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        Self {
            _dir: dir,
            local,
            remote,
        }
    }

    fn request(&self, mode: TagPushMode) -> TagPushRequest {
        TagPushRequest {
            mode,
            remote: "origin".into(),
            branch: "release".into(),
            local_branch: "main".into(),
            head: CommitId(git(&self.local, &["rev-parse", "HEAD"]).into()),
            set_upstream: true,
        }
    }

    fn tags(&self) {
        git(&self.local, &["tag", "light"]);
        git(&self.local, &["tag", "-a", "annotated", "-m", "release"]);
        git(&self.local, &["switch", "--orphan", "other"]);
        git(&self.local, &["commit", "--allow-empty", "-m", "unrelated"]);
        git(
            &self.local,
            &["tag", "-a", "unrelated", "-m", "other history"],
        );
        git(&self.local, &["switch", "main"]);
    }
}

#[test]
fn annotated_preview_and_push_use_reachability_and_exclude_lightweight_tags() {
    let fixture = Fixture::new();
    fixture.tags();
    let repo = GixBackend.open(&fixture.local).unwrap();
    let request = fixture.request(TagPushMode::FollowAnnotated);
    let before = git(&fixture.local, &["show-ref"]);
    let preview = repo
        .preview_tag_push(&request, &CancellationToken::new())
        .unwrap();
    assert_eq!(preview.new_tags, ["annotated"]);
    assert!(preview.conflicting_tags.is_empty());
    assert_eq!(
        git(&fixture.remote, &["for-each-ref", "--format=%(refname)"]),
        ""
    );
    assert_eq!(git(&fixture.local, &["show-ref"]), before);
    assert!(!git(&fixture.local, &["config", "--local", "--list"]).contains("branch.main.remote="));
    let output = repo.push_with_tags(&request).unwrap();
    assert!(output.command.contains("--follow-tags"));
    assert_eq!(
        git(&fixture.remote, &["for-each-ref", "--format=%(refname)"]),
        "refs/heads/release\nrefs/tags/annotated"
    );
    assert_eq!(
        git(&fixture.local, &["config", "branch.main.merge"]),
        "refs/heads/release"
    );
    let preview = repo
        .preview_tag_push(&request, &CancellationToken::new())
        .unwrap();
    assert!(
        preview.new_tags.is_empty(),
        "up-to-date tags must not be counted"
    );
}

#[test]
fn all_tags_push_includes_branch_lightweight_and_unrelated_tags() {
    let fixture = Fixture::new();
    fixture.tags();
    let repo = GixBackend.open(&fixture.local).unwrap();
    let request = fixture.request(TagPushMode::All);
    let preview = repo
        .preview_tag_push(&request, &CancellationToken::new())
        .unwrap();
    assert_eq!(preview.new_tags, ["annotated", "light", "unrelated"]);
    repo.push_with_tags(&request).unwrap();
    assert_eq!(
        git(&fixture.remote, &["rev-parse", "refs/heads/release"]),
        request.head.0.as_ref()
    );
    assert_eq!(
        git(&fixture.remote, &["tag", "--list"]),
        "annotated\nlight\nunrelated"
    );
}

#[test]
fn preview_uses_push_urls_and_reports_conflicts_separately() {
    let fixture = Fixture::new();
    fixture.tags();
    git(&fixture.local, &["push", "origin", "refs/tags/light"]);
    git(&fixture.local, &["tag", "-f", "light", "unrelated^{}"]);
    let fetch_only = fixture._dir.path().join("fetch-only.git");
    fs::create_dir_all(&fetch_only).unwrap();
    git(&fetch_only, &["init", "--bare"]);
    git(
        &fixture.local,
        &["remote", "set-url", "origin", fetch_only.to_str().unwrap()],
    );
    git(
        &fixture.local,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            fixture.remote.to_str().unwrap(),
        ],
    );
    let repo = GixBackend.open(&fixture.local).unwrap();
    let preview = repo
        .preview_tag_push(
            &fixture.request(TagPushMode::All),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(preview.new_tags, ["annotated", "unrelated"]);
    assert_eq!(preview.conflicting_tags, ["light"]);
    assert_eq!(preview.destinations, [fixture.remote.to_string_lossy()]);
}

#[test]
fn multiple_push_urls_deduplicate_new_tags_and_transport_failure_is_unavailable() {
    let fixture = Fixture::new();
    fixture.tags();
    let second = fixture._dir.path().join("second.git");
    fs::create_dir_all(&second).unwrap();
    git(&second, &["init", "--bare"]);
    git(
        &fixture.local,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            fixture.remote.to_str().unwrap(),
        ],
    );
    git(
        &fixture.local,
        &[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            second.to_str().unwrap(),
        ],
    );
    let repo = GixBackend.open(&fixture.local).unwrap();
    let request = fixture.request(TagPushMode::All);
    let preview = repo
        .preview_tag_push(&request, &CancellationToken::new())
        .unwrap();
    assert_eq!(preview.new_tags.len(), 3);
    assert_eq!(preview.destinations.len(), 2);
    git(
        &fixture.local,
        &[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            fixture._dir.path().join("missing.git").to_str().unwrap(),
        ],
    );
    assert!(
        repo.preview_tag_push(&request, &CancellationToken::new())
            .is_err()
    );
}

#[test]
fn stale_requests_and_cancelled_previews_never_push() {
    let fixture = Fixture::new();
    let request = fixture.request(TagPushMode::All);
    let repo = GixBackend.open(&fixture.local).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(repo.preview_tag_push(&request, &cancellation).is_err());
    git(&fixture.local, &["commit", "--allow-empty", "-m", "moved"]);
    assert!(repo.push_with_tags(&request).is_err());
    assert!(
        repo.preview_tag_push(&request, &CancellationToken::new())
            .is_err()
    );
    assert_eq!(
        git(&fixture.remote, &["for-each-ref", "--format=%(refname)"]),
        ""
    );
}

#[cfg(unix)]
#[test]
fn preview_skips_hooks_and_signing_but_real_push_runs_hooks() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.tags();
    let hook = fixture.local.join(".git/hooks/pre-push");
    fs::write(&hook, "#!/bin/sh\necho ran > hook-ran\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    git(&fixture.local, &["config", "push.gpgSign", "true"]);
    let repo = GixBackend.open(&fixture.local).unwrap();
    let request = fixture.request(TagPushMode::All);
    assert_eq!(
        repo.preview_tag_push(&request, &CancellationToken::new())
            .unwrap()
            .new_tags
            .len(),
        3
    );
    assert!(!fixture.local.join("hook-ran").exists());
    git(&fixture.local, &["config", "push.gpgSign", "false"]);
    assert!(repo.push_with_tags(&request).is_err());
    assert!(fixture.local.join("hook-ran").exists());
    assert_eq!(
        git(&fixture.remote, &["for-each-ref", "--format=%(refname)"]),
        ""
    );
}

#[test]
fn preview_does_not_consume_credentials_staged_for_a_push() {
    use gitcomet_core::auth::{
        GitAuthKind, ScopedStagedGitAuth, StagedGitAuth, take_staged_git_auth,
    };
    let fixture = Fixture::new();
    let repo = GixBackend.open(&fixture.local).unwrap();
    let _auth = ScopedStagedGitAuth::stage(StagedGitAuth {
        kind: GitAuthKind::UsernamePassword,
        username: Some("alice".into()),
        secret: "test-token".into(),
    });
    repo.preview_tag_push(
        &fixture.request(TagPushMode::All),
        &CancellationToken::new(),
    )
    .unwrap();
    let pending = take_staged_git_auth().expect("preview must leave the push's credentials staged");
    assert_eq!(pending.username.as_deref(), Some("alice"));
    assert_eq!(pending.secret, "test-token");
}
