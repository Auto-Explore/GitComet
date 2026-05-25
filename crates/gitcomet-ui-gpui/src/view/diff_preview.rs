use crate::view::diff_utils::UnifiedDiffLine;
use gitcomet_core::domain::{DiffLineKind, DiffTarget};

const NO_NEWLINE_AT_END_OF_FILE_MARKER: &str = "\\ No newline at end of file";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReconstructedFilePreview {
    pub(super) abs_path: std::path::PathBuf,
    pub(super) lines: Vec<String>,
    pub(super) source_len: usize,
}

fn preview_abs_path(
    workdir: &std::path::Path,
    target: Option<&DiffTarget>,
) -> Option<std::path::PathBuf> {
    let rel_path = match target? {
        DiffTarget::WorkingTree { path, .. } => path.clone(),
        DiffTarget::Commit {
            path: Some(path), ..
        }
        | DiffTarget::CommitRange {
            path: Some(path), ..
        } => path.clone(),
        _ => return None,
    };

    Some(if rel_path.is_absolute() {
        rel_path
    } else {
        workdir.join(rel_path)
    })
}

fn collect_preview_lines_and_source_len<T: UnifiedDiffLine>(
    diff: &[T],
    kind: DiffLineKind,
    prefix: char,
) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut source_len = 0usize;

    for (ix, line) in diff.iter().enumerate() {
        if line.kind() != kind {
            continue;
        }

        let text = line.text().strip_prefix(prefix).unwrap_or(line.text());
        lines.push(text.to_string());
        source_len = source_len.saturating_add(text.len());
        if !matches!(
            diff.get(ix + 1),
            Some(next) if next.text() == NO_NEWLINE_AT_END_OF_FILE_MARKER
        ) {
            source_len = source_len.saturating_add(1);
        }
    }

    (lines, source_len)
}

pub(super) fn build_new_file_preview_from_diff(
    diff: &[impl UnifiedDiffLine],
    workdir: &std::path::Path,
    target: Option<&DiffTarget>,
) -> Option<ReconstructedFilePreview> {
    let mut file_header_count = 0usize;
    let mut is_new_file = false;
    let mut has_remove = false;

    for line in diff {
        match line.kind() {
            DiffLineKind::Header => {
                let text = line.text();
                if text.starts_with("diff --git ") {
                    file_header_count += 1;
                }
                if text.starts_with("new file mode ") || text.eq_ignore_ascii_case("--- /dev/null")
                {
                    is_new_file = true;
                }
            }
            DiffLineKind::Remove => has_remove = true,
            _ => {}
        }
    }

    if file_header_count != 1 || !is_new_file || has_remove {
        return None;
    }

    let abs_path = preview_abs_path(workdir, target)?;
    let (lines, source_len) = collect_preview_lines_and_source_len(diff, DiffLineKind::Add, '+');
    Some(ReconstructedFilePreview {
        abs_path,
        lines,
        source_len,
    })
}

#[cfg(test)]
pub(super) fn build_deleted_file_preview_from_diff(
    diff: &[impl UnifiedDiffLine],
    workdir: &std::path::Path,
    target: Option<&DiffTarget>,
) -> Option<ReconstructedFilePreview> {
    let mut file_header_count = 0usize;
    let mut is_deleted_file = false;
    let mut has_add = false;

    for line in diff {
        match line.kind() {
            DiffLineKind::Header => {
                let text = line.text();
                if text.starts_with("diff --git ") {
                    file_header_count += 1;
                }
                if text.starts_with("deleted file mode ")
                    || text.eq_ignore_ascii_case("+++ /dev/null")
                {
                    is_deleted_file = true;
                }
            }
            DiffLineKind::Add => has_add = true,
            _ => {}
        }
    }

    if file_header_count != 1 || !is_deleted_file || has_add {
        return None;
    }

    let abs_path = preview_abs_path(workdir, target)?;
    let (lines, source_len) = collect_preview_lines_and_source_len(diff, DiffLineKind::Remove, '-');
    Some(ReconstructedFilePreview {
        abs_path,
        lines,
        source_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::diff::AnnotatedDiffLine;
    use gitcomet_core::domain::{DiffArea, DiffLineKind};
    use std::path::PathBuf;

    fn line(kind: DiffLineKind, text: &str) -> AnnotatedDiffLine {
        AnnotatedDiffLine {
            kind,
            text: text.into(),
            old_line: None,
            new_line: None,
        }
    }

    #[test]
    fn new_file_preview_extracts_added_lines() {
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("new.txt"),
            area: DiffArea::Unstaged,
        };
        let diff = vec![
            line(DiffLineKind::Header, "diff --git a/new.txt b/new.txt"),
            line(DiffLineKind::Header, "new file mode 100644"),
            line(DiffLineKind::Add, "+hello"),
            line(DiffLineKind::Add, "+world"),
        ];

        let preview = build_new_file_preview_from_diff(&diff, &workdir, Some(&target)).unwrap();
        assert_eq!(preview.abs_path, workdir.join("new.txt"));
        assert_eq!(
            preview.lines,
            vec!["hello".to_string(), "world".to_string()]
        );
        assert_eq!(preview.source_len, "hello\nworld\n".len());
    }

    #[test]
    fn new_file_preview_returns_none_when_diff_has_removes() {
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("new.txt"),
            area: DiffArea::Unstaged,
        };
        let diff = vec![
            line(DiffLineKind::Header, "diff --git a/new.txt b/new.txt"),
            line(DiffLineKind::Header, "new file mode 100644"),
            line(DiffLineKind::Remove, "-nope"),
            line(DiffLineKind::Add, "+ok"),
        ];

        assert!(build_new_file_preview_from_diff(&diff, &workdir, Some(&target)).is_none());
    }

    #[test]
    fn deleted_file_preview_extracts_removed_lines() {
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("old.txt"),
            area: DiffArea::Unstaged,
        };
        let diff = vec![
            line(DiffLineKind::Header, "diff --git a/old.txt b/old.txt"),
            line(DiffLineKind::Header, "deleted file mode 100644"),
            line(DiffLineKind::Remove, "-hello"),
            line(DiffLineKind::Remove, "-world"),
        ];

        let preview = build_deleted_file_preview_from_diff(&diff, &workdir, Some(&target)).unwrap();
        assert_eq!(preview.abs_path, workdir.join("old.txt"));
        assert_eq!(
            preview.lines,
            vec!["hello".to_string(), "world".to_string()]
        );
        assert_eq!(preview.source_len, "hello\nworld\n".len());
    }

    #[test]
    fn deleted_file_preview_returns_none_when_diff_has_adds() {
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("old.txt"),
            area: DiffArea::Unstaged,
        };
        let diff = vec![
            line(DiffLineKind::Header, "diff --git a/old.txt b/old.txt"),
            line(DiffLineKind::Header, "deleted file mode 100644"),
            line(DiffLineKind::Remove, "-ok"),
            line(DiffLineKind::Add, "+nope"),
        ];

        assert!(build_deleted_file_preview_from_diff(&diff, &workdir, Some(&target)).is_none());
    }

    #[test]
    fn new_file_preview_source_len_respects_missing_final_newline_marker() {
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("new.txt"),
            area: DiffArea::Unstaged,
        };
        let diff = vec![
            line(DiffLineKind::Header, "diff --git a/new.txt b/new.txt"),
            line(DiffLineKind::Header, "new file mode 100644"),
            line(DiffLineKind::Add, "+hello"),
            line(DiffLineKind::Context, NO_NEWLINE_AT_END_OF_FILE_MARKER),
        ];

        let preview = build_new_file_preview_from_diff(&diff, &workdir, Some(&target)).unwrap();
        assert_eq!(preview.lines, vec!["hello".to_string()]);
        assert_eq!(preview.source_len, "hello".len());
    }

    #[test]
    fn deleted_file_preview_source_len_respects_missing_final_newline_marker() {
        let workdir = PathBuf::from("repo");
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("old.txt"),
            area: DiffArea::Unstaged,
        };
        let diff = vec![
            line(DiffLineKind::Header, "diff --git a/old.txt b/old.txt"),
            line(DiffLineKind::Header, "deleted file mode 100644"),
            line(DiffLineKind::Remove, "-hello"),
            line(DiffLineKind::Context, NO_NEWLINE_AT_END_OF_FILE_MARKER),
        ];

        let preview = build_deleted_file_preview_from_diff(&diff, &workdir, Some(&target)).unwrap();
        assert_eq!(preview.lines, vec!["hello".to_string()]);
        assert_eq!(preview.source_len, "hello".len());
    }
}
