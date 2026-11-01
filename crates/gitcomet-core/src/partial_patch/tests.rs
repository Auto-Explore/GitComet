use super::*;
use crate::diff::annotate_unified;
use crate::domain::{Diff, DiffArea, DiffTarget};
use std::time::{Duration, Instant};

#[test]
fn large_created_file_block_prepares_exact_patch_in_both_directions() {
    let text = format!(
        "diff --git a/file.rs b/file.rs\n--- /dev/null\n+++ b/file.rs\n@@ -0,0 +1,199002 @@\n+fn f() {{\n{}+}}\n",
        "+a();\n".repeat(199_000)
    );
    let rows = annotate_unified(&Diff::from_unified(
        DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
        &text,
    ));
    let selected = (4..rows.len()).collect();
    let start = Instant::now();
    let forward = build_unified_patch_for_selected_lines_across_hunks(&rows, &selected).unwrap();
    let reverse =
        build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(&rows, &selected)
            .unwrap();
    assert_eq!(forward, text.as_bytes());
    assert_eq!(reverse, text.as_bytes());
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "large block preparation exceeded five seconds"
    );
}
