use crate::cli::{MergetoolConfig, exit_code};
use gitgpui_core::merge::{MergeError, MergeLabels, MergeOptions, MergeResult, merge_file_bytes};
use std::fs;

/// Result of running the dedicated mergetool mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergetoolRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// The merge result details (available when merge was attempted).
    pub merge_result: Option<MergeResult>,
}

/// Execute mergetool mode using the built-in 3-way merge algorithm.
///
/// Reads base, local, and remote files, performs a 3-way merge, and writes
/// the result to the merged output path. Returns SUCCESS (0) on clean merge,
/// CANCELED (1) if conflicts remain in the output.
///
/// When invoked by `git mergetool`, the contract is:
/// - Exit 0: merge succeeded, MERGED file contains resolved content
/// - Exit 1: merge has unresolved conflicts (MERGED contains markers)
/// - Exit ≥2: operational error (bad input, I/O failure, etc.)
pub fn run_mergetool(config: &MergetoolConfig) -> Result<MergetoolRunResult, String> {
    // Read the three input sides.
    let local_bytes = fs::read(&config.local)
        .map_err(|e| format!("Failed to read local file {}: {e}", config.local.display()))?;
    let remote_bytes = fs::read(&config.remote).map_err(|e| {
        format!(
            "Failed to read remote file {}: {e}",
            config.remote.display()
        )
    })?;

    let base_bytes = match &config.base {
        Some(base_path) => fs::read(base_path)
            .map_err(|e| format!("Failed to read base file {}: {e}", base_path.display()))?,
        // No base file: treat as empty (add/add conflict scenario).
        None => Vec::new(),
    };

    // Build merge options from config labels and algorithm preferences.
    let options = MergeOptions {
        style: config.conflict_style,
        diff_algorithm: config.diff_algorithm,
        labels: MergeLabels {
            ours: config.label_local.clone(),
            base: config.label_base.clone(),
            theirs: config.label_remote.clone(),
        },
        ..MergeOptions::default()
    };

    // Run the 3-way merge algorithm with byte-level binary detection.
    let result = match merge_file_bytes(&base_bytes, &local_bytes, &remote_bytes, &options) {
        Ok(result) => result,
        Err(MergeError::BinaryContent) => return handle_binary_conflict(config, &local_bytes),
    };
    let is_clean = result.is_clean();
    let conflict_count = result.conflict_count;

    // Write merged output to MERGED path.
    fs::write(&config.merged, &result.output).map_err(|e| {
        format!(
            "Failed to write merged output to {}: {e}",
            config.merged.display()
        )
    })?;

    if is_clean {
        Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!(
                "Auto-merged {}\n",
                config
                    .merged
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| config.merged.display().to_string())
            ),
            exit_code: exit_code::SUCCESS,
            merge_result: Some(result),
        })
    } else {
        Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!(
                "Auto-merging {}\nCONFLICT (content): Merge conflict in {}\n\
                 Automatic merge failed; {} conflict(s) remain.\n",
                config
                    .merged
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| config.merged.display().to_string()),
                config
                    .merged
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| config.merged.display().to_string()),
                conflict_count,
            ),
            exit_code: exit_code::CANCELED,
            merge_result: Some(result),
        })
    }
}

/// Handle binary files by copying LOCAL to MERGED and returning a conflict.
fn handle_binary_conflict(
    config: &MergetoolConfig,
    local_bytes: &[u8],
) -> Result<MergetoolRunResult, String> {
    // Write LOCAL side to MERGED (user can pick sides manually).
    fs::write(&config.merged, local_bytes).map_err(|e| {
        format!(
            "Failed to write merged output to {}: {e}",
            config.merged.display()
        )
    })?;

    let filename = config
        .merged
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| config.merged.display().to_string());

    Ok(MergetoolRunResult {
        stdout: String::new(),
        stderr: format!(
            "warning: Cannot merge binary files: {filename}\n\
             CONFLICT (binary): {filename} — keeping local version.\n"
        ),
        exit_code: exit_code::CANCELED,
        merge_result: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &std::path::Path, content: &str) {
        fs::write(path, content).expect("write fixture file");
    }

    fn write_bytes(path: &std::path::Path, content: &[u8]) {
        fs::write(path, content).expect("write fixture file");
    }

    fn make_config(
        dir: &std::path::Path,
        base: Option<&str>,
        local: &str,
        remote: &str,
        merged_content: &str,
    ) -> MergetoolConfig {
        let merged_path = dir.join("merged.txt");
        let local_path = dir.join("local.txt");
        let remote_path = dir.join("remote.txt");

        write_file(&merged_path, merged_content);
        write_file(&local_path, local);
        write_file(&remote_path, remote);

        let base_path = base.map(|b| {
            let p = dir.join("base.txt");
            write_file(&p, b);
            p
        });

        MergetoolConfig {
            merged: merged_path,
            local: local_path,
            remote: remote_path,
            base: base_path,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        }
    }

    // ── Clean merge ──────────────────────────────────────────────────

    #[test]
    fn clean_merge_non_overlapping_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("line1\nline2\nline3\n"),
            "LINE1\nline2\nline3\n", // local changes line 1
            "line1\nline2\nLINE3\n", // remote changes line 3
            "",                      // MERGED placeholder
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(result.merge_result.as_ref().unwrap().is_clean());

        // Verify the merged output was written correctly.
        let merged = fs::read_to_string(&config.merged).unwrap();
        assert_eq!(merged, "LINE1\nline2\nLINE3\n");
        assert!(result.stderr.contains("Auto-merged"));
    }

    #[test]
    fn clean_merge_identical_files() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "same content\n";
        let config = make_config(tmp.path(), Some(content), content, content, "");

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert_eq!(result.merge_result.as_ref().unwrap().conflict_count, 0);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert_eq!(merged, content);
    }

    // ── Conflicts ────────────────────────────────────────────────────

    #[test]
    fn conflicting_merge_returns_canceled_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("original\n"),
            "local change\n",
            "remote change\n",
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);
        assert_eq!(result.merge_result.as_ref().unwrap().conflict_count, 1);

        // MERGED should contain conflict markers.
        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(merged.contains("<<<<<<<"));
        assert!(merged.contains("======="));
        assert!(merged.contains(">>>>>>>"));
        assert!(merged.contains("local change"));
        assert!(merged.contains("remote change"));
        assert!(result.stderr.contains("CONFLICT"));
        assert!(result.stderr.contains("1 conflict(s)"));
    }

    #[test]
    fn conflict_with_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = make_config(tmp.path(), Some("original\n"), "ours\n", "theirs\n", "");
        config.label_local = Some("HEAD".to_string());
        config.label_remote = Some("feature-branch".to_string());

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(merged.contains("<<<<<<< HEAD"), "output: {merged}");
        assert!(
            merged.contains(">>>>>>> feature-branch"),
            "output: {merged}"
        );
    }

    // ── No base (add/add conflict) ───────────────────────────────────

    #[test]
    fn no_base_uses_empty_base() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            None, // no base
            "added by local\n",
            "added by remote\n",
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        // With empty base, both sides adding different content = conflict.
        assert_eq!(result.exit_code, exit_code::CANCELED);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(merged.contains("<<<<<<<"));
        assert!(merged.contains("added by local"));
        assert!(merged.contains("added by remote"));
    }

    #[test]
    fn no_base_identical_content_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            None, // no base
            "same content\n",
            "same content\n",
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert_eq!(merged, "same content\n");
    }

    // ── Binary content ───────────────────────────────────────────────

    #[test]
    fn binary_content_copies_local_and_returns_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("merged.bin");
        let local_path = tmp.path().join("local.bin");
        let remote_path = tmp.path().join("remote.bin");

        let local_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG header
        let remote_bytes: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG header

        write_bytes(&merged_path, b"placeholder");
        write_bytes(&local_path, &local_bytes);
        write_bytes(&remote_path, &remote_bytes);

        let config = MergetoolConfig {
            merged: merged_path.clone(),
            local: local_path,
            remote: remote_path,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);
        assert!(
            result.merge_result.is_none(),
            "binary files skip text merge"
        );
        assert!(result.stderr.contains("binary"));

        // MERGED should contain the local bytes.
        let output = fs::read(&merged_path).unwrap();
        assert_eq!(output, local_bytes);
    }

    #[test]
    fn binary_base_with_text_sides_is_binary_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("merged.txt");
        let local_path = tmp.path().join("local.txt");
        let remote_path = tmp.path().join("remote.txt");
        let base_path = tmp.path().join("base.bin");

        write_file(&merged_path, "");
        write_file(&local_path, "text local\n");
        write_file(&remote_path, "text remote\n");
        write_bytes(&base_path, &[0x00, 0xFF, 0xFE]);

        let config = MergetoolConfig {
            merged: merged_path,
            local: local_path,
            remote: remote_path,
            base: Some(base_path),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);
        assert!(result.stderr.contains("binary"));
    }

    #[test]
    fn null_byte_content_is_treated_as_binary_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("merged.bin");
        let local_path = tmp.path().join("local.bin");
        let remote_path = tmp.path().join("remote.bin");
        let base_path = tmp.path().join("base.txt");

        let local_bytes = b"hello\0world\n".to_vec();
        let remote_bytes = b"hello world\n".to_vec();

        write_bytes(&merged_path, b"placeholder");
        write_bytes(&local_path, &local_bytes);
        write_bytes(&remote_path, &remote_bytes);
        write_file(&base_path, "hello world\n");

        let config = MergetoolConfig {
            merged: merged_path.clone(),
            local: local_path,
            remote: remote_path,
            base: Some(base_path),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);
        assert!(
            result.merge_result.is_none(),
            "binary files skip text merge"
        );
        assert!(result.stderr.contains("binary"));

        let output = fs::read(&merged_path).unwrap();
        assert_eq!(output, local_bytes);
    }

    // ── File I/O errors ──────────────────────────────────────────────

    #[test]
    fn missing_local_file_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("merged.txt");
        let remote_path = tmp.path().join("remote.txt");
        write_file(&merged_path, "");
        write_file(&remote_path, "remote\n");

        let config = MergetoolConfig {
            merged: merged_path,
            local: tmp.path().join("nonexistent_local.txt"),
            remote: remote_path,
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let err = run_mergetool(&config).expect_err("expected error");
        assert!(err.contains("Failed to read local file"), "error: {err}");
    }

    #[test]
    fn missing_remote_file_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("merged.txt");
        let local_path = tmp.path().join("local.txt");
        write_file(&merged_path, "");
        write_file(&local_path, "local\n");

        let config = MergetoolConfig {
            merged: merged_path,
            local: local_path,
            remote: tmp.path().join("nonexistent_remote.txt"),
            base: None,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let err = run_mergetool(&config).expect_err("expected error");
        assert!(err.contains("Failed to read remote file"), "error: {err}");
    }

    #[test]
    fn missing_base_file_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("merged.txt");
        let local_path = tmp.path().join("local.txt");
        let remote_path = tmp.path().join("remote.txt");
        write_file(&merged_path, "");
        write_file(&local_path, "local\n");
        write_file(&remote_path, "remote\n");

        let config = MergetoolConfig {
            merged: merged_path,
            local: local_path,
            remote: remote_path,
            base: Some(tmp.path().join("nonexistent_base.txt")),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let err = run_mergetool(&config).expect_err("expected error");
        assert!(err.contains("Failed to read base file"), "error: {err}");
    }

    // ── CRLF preservation ────────────────────────────────────────────

    #[test]
    fn crlf_content_preserved_in_clean_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("line1\r\nline2\r\nline3\r\n"),
            "LINE1\r\nline2\r\nline3\r\n",
            "line1\r\nline2\r\nLINE3\r\n",
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert_eq!(merged, "LINE1\r\nline2\r\nLINE3\r\n");
    }

    #[test]
    fn crlf_conflict_markers_match_input_line_endings() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("original\r\n"),
            "local\r\n",
            "remote\r\n",
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);

        let merged = fs::read_to_string(&config.merged).unwrap();
        // Conflict markers should use CRLF when input uses CRLF.
        assert!(merged.contains("<<<<<<<"), "output: {merged}");
        assert!(merged.contains("======="), "output: {merged}");
        assert!(merged.contains(">>>>>>>"), "output: {merged}");
    }

    // ── Merged output path ───────────────────────────────────────────

    #[test]
    fn merged_output_overwrites_existing_content() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("base\n"),
            "local change\n",
            "local change\n", // same as local = clean merge
            "<<<<<<< this was the old conflict content\nold stuff\n>>>>>>>",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert_eq!(merged, "local change\n");
        assert!(
            !merged.contains("<<<<<<<"),
            "old conflict markers should be gone"
        );
    }

    // ── Multi-region conflicts ───────────────────────────────────────

    #[test]
    fn multiple_conflicts_reported_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("a\nb\nc\nd\ne\n"),
            "A\nb\nC\nd\nE\n", // changes a, c, e
            "X\nb\nY\nd\nZ\n", // changes a, c, e differently
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);
        let count = result.merge_result.as_ref().unwrap().conflict_count;
        assert!(count >= 1, "expected at least 1 conflict, got {count}");
        assert!(result.stderr.contains("conflict(s)"));
    }

    // ── Paths with spaces ────────────────────────────────────────────

    #[test]
    fn handles_paths_with_spaces() {
        let tmp = tempfile::tempdir().unwrap();
        let merged_path = tmp.path().join("my merged file.txt");
        let local_path = tmp.path().join("my local file.txt");
        let remote_path = tmp.path().join("my remote file.txt");
        let base_path = tmp.path().join("my base file.txt");

        write_file(&merged_path, "");
        write_file(&local_path, "local\n");
        write_file(&remote_path, "local\n"); // same = clean merge
        write_file(&base_path, "base\n");

        let config = MergetoolConfig {
            merged: merged_path.clone(),
            local: local_path,
            remote: remote_path,
            base: Some(base_path),
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: gitgpui_core::merge::ConflictStyle::default(),
            diff_algorithm: gitgpui_core::merge::DiffAlgorithm::default(),
        };

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        let merged = fs::read_to_string(&merged_path).unwrap();
        assert_eq!(merged, "local\n");
    }

    // ── Empty files ──────────────────────────────────────────────────

    #[test]
    fn all_empty_files_produce_clean_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path(), Some(""), "", "", "");

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(merged.is_empty());
    }

    #[test]
    fn empty_base_with_identical_additions_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path(), Some(""), "new content\n", "new content\n", "");

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert_eq!(merged, "new content\n");
    }

    // ── Trailing newline preservation ────────────────────────────────

    #[test]
    fn preserves_missing_trailing_newline() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(
            tmp.path(),
            Some("line1\nline2"), // no trailing LF
            "LINE1\nline2",       // changed line1, no trailing LF
            "line1\nline2",       // unchanged, no trailing LF
            "",
        );

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(
            !merged.ends_with('\n'),
            "should preserve missing trailing LF"
        );
        assert!(merged.contains("LINE1"));
    }

    // ── Conflict style selection ──────────────────────────────────────

    #[test]
    fn diff3_style_includes_base_section() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = make_config(
            tmp.path(),
            Some("original\n"),
            "local change\n",
            "remote change\n",
            "",
        );
        config.conflict_style = gitgpui_core::merge::ConflictStyle::Diff3;

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(merged.contains("<<<<<<<"), "output: {merged}");
        assert!(merged.contains("|||||||"), "diff3 should include base section: {merged}");
        assert!(merged.contains("original"), "base content should appear: {merged}");
        assert!(merged.contains("======="), "output: {merged}");
        assert!(merged.contains(">>>>>>>"), "output: {merged}");
    }

    #[test]
    fn zdiff3_style_extracts_common_prefix_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        // base=1..9, local=1..4+ABCDE+7..9, remote=1..4+AXCYE+7..9
        let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let local = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
        let remote = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
        let mut config = make_config(tmp.path(), Some(base), local, remote, "");
        config.conflict_style = gitgpui_core::merge::ConflictStyle::Zdiff3;

        let result = run_mergetool(&config).expect("mergetool run");
        assert_eq!(result.exit_code, exit_code::CANCELED);

        let merged = fs::read_to_string(&config.merged).unwrap();
        // zdiff3 should extract common prefix "A" and suffix "E" outside markers.
        assert!(merged.contains("A\n<<<<<<<"), "common prefix A should be outside markers: {merged}");
        assert!(merged.contains(">>>>>>>\nE\n"), "common suffix E should be outside markers: {merged}");
    }

    // ── Diff algorithm selection ──────────────────────────────────────

    #[test]
    fn histogram_algorithm_produces_clean_merge_on_structural_code() {
        let tmp = tempfile::tempdir().unwrap();
        // C code case where Myers produces spurious conflicts but histogram doesn't.
        let base = "void f() {\n    x = 1;\n}\nvoid g() {\n    y = 1;\n}\n";
        let ours = "void h() {\n    z = 1;\n}\nvoid g() {\n    y = 1;\n}\n";
        let theirs = "void f() {\n    x = 1;\n}\nvoid g() {\n    y = 2;\n}\n";

        let mut config = make_config(tmp.path(), Some(base), ours, theirs, "");
        config.diff_algorithm = gitgpui_core::merge::DiffAlgorithm::Histogram;

        let result = run_mergetool(&config).expect("mergetool run");
        // Histogram should produce a clean merge for this case.
        assert_eq!(
            result.exit_code,
            exit_code::SUCCESS,
            "histogram should cleanly merge structural code changes"
        );

        let merged = fs::read_to_string(&config.merged).unwrap();
        assert!(merged.contains("void h()"), "should have ours' h()");
        assert!(merged.contains("y = 2"), "should have theirs' y = 2");
    }
}
