//! The kit's shared test helpers; its locks serialize host and kit tests alike.

pub(crate) use gitcomet_ui_kit::test_support::{
    lock_clipboard_test, lock_visual_test, painted_control_quads, refresh_and_draw,
};

/// Runs `git -C dir args…`, asserting it succeeds; returns its stdout.
pub(crate) fn git(dir: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
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
