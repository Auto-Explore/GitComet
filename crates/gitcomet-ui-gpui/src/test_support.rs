pub(crate) use gitcomet_ui_kit::test_support::{painted_control_quads, refresh_and_draw};

/// Runs `git -C dir args…`, asserting it succeeds; returns its stdout.
pub(crate) fn git(dir: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .args(["-c", "commit.gpgsign=false"])
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
