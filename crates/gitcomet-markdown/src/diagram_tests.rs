use super::*;
use gitcomet_diagrams::DiagramKind;

#[test]
fn diagram_fences_keep_source_metadata_and_full_fence_range() {
    for (info, source, kind) in [
        (
            "mermaid title=Architecture",
            "flowchart LR
A-->B
",
            DiagramKind::Mermaid,
        ),
        (
            "{.mermaid}",
            "flowchart LR
A-->B
",
            DiagramKind::Mermaid,
        ),
        (
            "language-mermaid",
            "flowchart LR;A-->B
",
            DiagramKind::Mermaid,
        ),
        (
            "",
            "sequenceDiagram
A->>B: hello
",
            DiagramKind::Mermaid,
        ),
    ] {
        let markdown = format!("Before\n\n```{info}\n{source}```\n\nAfter\n");
        let document = parse_markdown(&markdown).unwrap();
        let blocks = markdown_document_blocks(&document);
        let range = blocks
            .iter()
            .find_map(|block| match block {
                MarkdownBlock::Diagram(range) => Some(range.clone()),
                _ => None,
            })
            .expect("diagram block");
        for row in &document.rows[range] {
            let diagram = row.diagram.as_ref().unwrap();
            assert_eq!(diagram.kind, kind);
            assert_eq!(diagram.info.as_ref(), info);
            assert_eq!(diagram.source.as_ref(), source);
            assert_eq!(diagram.fence_lines, 2..4 + source.lines().count());
            assert!(diagram.fence_lines.start < row.source_line_range.start);
        }
    }
}

#[test]
fn ordinary_fences_and_indented_code_stay_code() {
    for source in [
        "```rust\nflowchart LR\nA-->B\n```",
        "```\na -> b\n```",
        "```json\n{\"signal\":[1,2]}\n```",
        "    flowchart LR\n    A-->B",
    ] {
        let document = parse_markdown(source).unwrap();
        assert!(document.rows.iter().all(|row| row.diagram.is_none()));
        assert!(
            !markdown_document_blocks(&document)
                .iter()
                .any(|block| matches!(block, MarkdownBlock::Diagram(_)))
        );
    }
}

#[test]
fn diagram_changes_are_whole_old_and_new_blocks() {
    let old = "```mermaid\nflowchart LR\nA-->B\nB-->C\n```\n";
    let new = "```mermaid\nflowchart LR\nA-->D\nB-->C\n```\n";
    let diff = build_markdown_diff_preview(old, new).unwrap();
    let blocks = markdown_document_blocks(&diff.inline);
    assert_eq!(blocks.len(), 2);
    for (block, expected) in blocks.iter().zip([
        "flowchart LR\nA-->B\nB-->C\n",
        "flowchart LR\nA-->D\nB-->C\n",
    ]) {
        let MarkdownBlock::Diagram(range) = block else {
            panic!("whole diagram expected")
        };
        assert_eq!(range.len(), 3);
        assert_eq!(
            diff.inline.rows[range.start]
                .diagram
                .as_ref()
                .unwrap()
                .source
                .as_ref(),
            expected
        );
    }
    let unchanged = build_markdown_diff_preview(old, old).unwrap();
    assert_eq!(markdown_document_blocks(&unchanged.inline).len(), 1);
}

#[test]
fn changing_only_fence_language_or_deleting_diagram_keeps_source_atomic() {
    let old = "```mermaid
flowchart LR;A-->B
```
";
    let new = "```rust
flowchart LR;A-->B
```
";
    let diff = build_markdown_diff_preview(old, new).unwrap();
    let blocks = markdown_document_blocks(&diff.inline);
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        diff.inline.rows[0].diagram.as_ref().unwrap().kind,
        DiagramKind::Mermaid
    );
    assert!(diff.inline.rows[1].diagram.is_none());
    for (old, new) in [(Some(old), None), (None, Some(old))] {
        let diff = build_markdown_diff_preview_of(old, new).unwrap();
        assert_eq!(markdown_document_blocks(&diff.inline).len(), 1);
    }
}

#[test]
fn adjacent_diagrams_stay_separate_inside_quotes() {
    let document = parse_markdown(
        "> ```mermaid\n> flowchart LR;A-->B\n> ```\n> ```mermaid\n> flowchart LR;B-->C\n> ```\n",
    )
    .unwrap();
    let blocks = markdown_blocks_in(&document, 0..document.rows.len(), 1);
    assert_eq!(blocks.len(), 2);
    assert!(
        blocks
            .iter()
            .all(|block| matches!(block, MarkdownBlock::Diagram(_)))
    );
}

#[test]
fn a_large_unchanged_diagram_remains_one_complete_inline_block() {
    let source = format!("```mermaid\n{}```\n", "flowchart LR;A-->B\n".repeat(5000));
    let diff = build_markdown_diff_preview(&source, &source).unwrap();
    let blocks = markdown_document_blocks(&diff.inline);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].row_range().len(), 5000);
}

#[test]
fn diagram_change_hints_scan_each_complete_fence_once_per_side() {
    let source = format!(
        "```mermaid\n{}```\n```mermaid\nflowchart LR;A-->C\nC-->D\n```\n",
        "flowchart LR;A-->B\n".repeat(19_000)
    );
    let (mut old, mut new) = parse_markdown_diff(&source, &source).unwrap();
    let first_fence = old.rows[0].diagram.as_ref().unwrap().fence_lines.clone();
    let second_fence = old
        .rows
        .last()
        .unwrap()
        .diagram
        .as_ref()
        .unwrap()
        .fence_lines
        .clone();
    let mut old_mask = vec![false; source.lines().count()];
    let mut new_mask = old_mask.clone();
    // A change to just the opening fence applies to every code row.
    old_mask[first_fence.start] = true;
    old_mask[second_fence.clone()].fill(true);
    new_mask[second_fence.clone()].fill(true);
    for (document, mask, is_old) in [(&mut old, &old_mask, true), (&mut new, &new_mask, false)] {
        let mut scanned_ranges = Vec::new();
        annotate_document_change_hints(document, |range| {
            scanned_ranges.push(range.clone());
            line_range_change_hint(range, mask, is_old)
        });
        assert_eq!(scanned_ranges.len(), 2, "one scan per diagram fence");
        assert_eq!(scanned_ranges, [first_fence.clone(), second_fence.clone()]);
        for row in &document.rows {
            let expected = if row.diagram.as_ref().unwrap().fence_lines == first_fence {
                if is_old {
                    MarkdownChangeHint::Modified
                } else {
                    MarkdownChangeHint::None
                }
            } else if is_old {
                MarkdownChangeHint::Removed
            } else {
                MarkdownChangeHint::Added
            };
            assert_eq!(row.change_hint, expected);
        }
    }
}
