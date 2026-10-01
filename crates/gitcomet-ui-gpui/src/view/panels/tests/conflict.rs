use super::*;
use palette::IntoColor;

fn build_conflict_scroll_matrix_current_text(ours_text: &str, theirs_text: &str) -> String {
    format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n")
}

fn build_conflict_scroll_matrix_text(label: &str, fill: char) -> String {
    (0..160)
        .map(|ix| format!("{label} line {ix:03} {}", fill.to_string().repeat(240)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn seed_conflict_scroll_matrix_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    file_rel: &std::path::Path,
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    current_text: &str,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.to_path_buf(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.to_path_buf(),
                base_text.to_string(),
                ours_text.to_string(),
                theirs_text.to_string(),
                current_text.to_string(),
            );
            let mut session = ConflictSession::from_merged_text(
                file_rel.to_path_buf(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.to_string().into()),
                ConflictPayload::Text(ours_text.to_string().into()),
                ConflictPayload::Text(theirs_text.to_string().into()),
                current_text,
            );
            for region in &mut session.regions {
                region.resolution =
                    gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs;
            }
            repo.conflict_state.conflict_session = Some(session);

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
}

fn reset_conflict_scroll_matrix_offsets(pane: &mut MainPaneView) {
    reset_uniform_list_offsets(&[
        &pane.conflict_resolver_diff_scroll,
        &pane.conflict_preview_ours_scroll,
        &pane.conflict_preview_theirs_scroll,
        &pane.conflict_resolved_preview_scroll,
        &pane.conflict_resolved_preview_gutter_scroll,
    ]);
    // The editable resolved output couples via its own `ScrollHandle`.
    set_scroll_handle_offset(
        &pane.conflict_resolved_output_editor_scroll,
        point(px(0.0), px(0.0)),
    );
}

struct SyntheticLargeConflictFixture {
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    abs_path: std::path::PathBuf,
    fixture_line_count: usize,
    conflict_block_count: usize,
    first_conflict_line: u32,
    base_text: String,
    ours_text: String,
    theirs_text: String,
    current_text: String,
}

impl SyntheticLargeConflictFixture {
    fn new(
        workdir_label: &str,
        file_rel: &str,
        fixture_line_count: usize,
        conflict_block_count: usize,
    ) -> Self {
        assert!(
            fixture_line_count >= conflict_block_count.saturating_add(3),
            "fixture needs room for 3 header lines plus at least 1 line per conflict"
        );
        assert!(
            conflict_block_count > 0,
            "synthetic large conflict fixture requires at least one conflict block"
        );

        let workdir = std::env::temp_dir().join(format!(
            "gitcomet_ui_test_{}_{}",
            std::process::id(),
            workdir_label
        ));
        let file_rel = std::path::PathBuf::from(file_rel);
        let abs_path = workdir.join(&file_rel);

        let mut base_lines = vec![
            "<!doctype html>".to_string(),
            "<html lang=\"en\">".to_string(),
            "<body class=\"fixture-root\">".to_string(),
        ];
        let mut ours_lines = base_lines.clone();
        let mut theirs_lines = base_lines.clone();
        let mut current_lines = base_lines.clone();

        let remaining_context = fixture_line_count
            .saturating_sub(base_lines.len())
            .saturating_sub(conflict_block_count);
        let context_per_slot = remaining_context / conflict_block_count;
        let context_remainder = remaining_context % conflict_block_count;
        let mut next_context_row = 0usize;
        let mut first_conflict_line = None;

        for conflict_ix in 0..conflict_block_count {
            let base_line = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"base\">base {conflict_ix}</main>"
            );
            let ours_line = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"ours\">ours {conflict_ix}</main>"
            );
            let theirs_line = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"theirs\">theirs {conflict_ix}</main>"
            );
            let conflict_line =
                u32::try_from(ours_lines.len().saturating_add(1)).unwrap_or(u32::MAX);
            first_conflict_line.get_or_insert(conflict_line);

            base_lines.push(base_line);
            ours_lines.push(ours_line.clone());
            theirs_lines.push(theirs_line.clone());
            current_lines.push("<<<<<<< ours".to_string());
            current_lines.push(ours_line);
            current_lines.push("=======".to_string());
            current_lines.push(theirs_line);
            current_lines.push(">>>>>>> theirs".to_string());

            let slot_lines = context_per_slot + usize::from(conflict_ix < context_remainder);
            append_synthetic_large_conflict_context(
                &mut base_lines,
                &mut ours_lines,
                &mut theirs_lines,
                &mut current_lines,
                &mut next_context_row,
                slot_lines,
            );
        }

        assert_eq!(base_lines.len(), fixture_line_count);
        assert_eq!(ours_lines.len(), fixture_line_count);
        assert_eq!(theirs_lines.len(), fixture_line_count);

        Self {
            workdir,
            file_rel,
            abs_path,
            fixture_line_count,
            conflict_block_count,
            first_conflict_line: first_conflict_line.unwrap_or(1),
            base_text: base_lines.join("\n"),
            ours_text: ours_lines.join("\n"),
            theirs_text: theirs_lines.join("\n"),
            current_text: current_lines.join("\n"),
        }
    }

    fn write(&self) {
        let _ = std::fs::remove_dir_all(&self.workdir);
        std::fs::create_dir_all(self.abs_path.parent().expect("fixture file parent"))
            .expect("create fixture dir");
        std::fs::write(&self.abs_path, &self.current_text).expect("write fixture");
    }

    fn repo_state(
        &self,
        repo_id: gitcomet_state::model::RepoId,
    ) -> gitcomet_state::model::RepoState {
        use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

        let mut repo = opening_repo_state(repo_id, &self.workdir);
        set_test_conflict_status(
            &mut repo,
            self.file_rel.clone(),
            gitcomet_core::domain::DiffArea::Unstaged,
        );
        set_test_conflict_file(
            &mut repo,
            self.file_rel.clone(),
            self.base_text.clone(),
            self.ours_text.clone(),
            self.theirs_text.clone(),
            self.current_text.clone(),
        );
        repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
            self.file_rel.clone(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(self.base_text.clone().into()),
            ConflictPayload::Text(self.ours_text.clone().into()),
            ConflictPayload::Text(self.theirs_text.clone().into()),
            &self.current_text,
        ));
        repo
    }

    fn cleanup(&self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup fixture");
    }
}

fn append_synthetic_large_conflict_context(
    base_lines: &mut Vec<String>,
    ours_lines: &mut Vec<String>,
    theirs_lines: &mut Vec<String>,
    current_lines: &mut Vec<String>,
    next_context_row: &mut usize,
    count: usize,
) {
    for _ in 0..count {
        let row = *next_context_row;
        let line = format!(
            "<section id=\"panel-{row}\" data-row=\"{row}\"><div class=\"copy\">row {row}</div></section>"
        );
        base_lines.push(line.clone());
        ours_lines.push(line.clone());
        theirs_lines.push(line.clone());
        current_lines.push(line);
        *next_context_row = next_context_row.saturating_add(1);
    }
}

struct SyntheticWholeFileConflictFixture {
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    abs_path: std::path::PathBuf,
    line_count: usize,
    base_text: String,
    ours_text: String,
    theirs_text: String,
    current_text: String,
}

impl SyntheticWholeFileConflictFixture {
    fn new(workdir_label: &str, file_rel: &str, line_count: usize) -> Self {
        assert!(
            line_count >= 5,
            "whole-file conflict fixture needs room for html wrapper lines"
        );

        let workdir = std::env::temp_dir().join(format!(
            "gitcomet_ui_test_{}_{}",
            std::process::id(),
            workdir_label
        ));
        let file_rel = std::path::PathBuf::from(file_rel);
        let abs_path = workdir.join(&file_rel);

        let build_side = |side: &str| {
            let mut lines = vec![
                "<!doctype html>".to_string(),
                "<html lang=\"en\">".to_string(),
                format!("<body class=\"whole-file-{side}\">"),
            ];
            let middle_count = line_count.saturating_sub(5);
            for row in 0..middle_count {
                lines.push(format!(
                    "<section id=\"panel-{row}\" data-side=\"{side}\"><div>{side} {row}</div></section>"
                ));
            }
            lines.push("</body>".to_string());
            lines.push("</html>".to_string());
            lines
        };

        let base_lines = build_side("base");
        let ours_lines = build_side("ours");
        let theirs_lines = build_side("theirs");
        assert_eq!(base_lines.len(), line_count);
        assert_eq!(ours_lines.len(), line_count);
        assert_eq!(theirs_lines.len(), line_count);

        let base_text = base_lines.join("\n");
        let ours_text = ours_lines.join("\n");
        let theirs_text = theirs_lines.join("\n");
        let current_text =
            format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");

        Self {
            workdir,
            file_rel,
            abs_path,
            line_count,
            base_text,
            ours_text,
            theirs_text,
            current_text,
        }
    }

    fn write(&self) {
        let _ = std::fs::remove_dir_all(&self.workdir);
        std::fs::create_dir_all(self.abs_path.parent().expect("fixture file parent"))
            .expect("create fixture dir");
        std::fs::write(&self.abs_path, &self.current_text).expect("write fixture");
    }

    fn repo_state(
        &self,
        repo_id: gitcomet_state::model::RepoId,
    ) -> gitcomet_state::model::RepoState {
        use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

        let mut repo = opening_repo_state(repo_id, &self.workdir);
        set_test_conflict_status(
            &mut repo,
            self.file_rel.clone(),
            gitcomet_core::domain::DiffArea::Unstaged,
        );
        set_test_conflict_file(
            &mut repo,
            self.file_rel.clone(),
            self.base_text.clone(),
            self.ours_text.clone(),
            self.theirs_text.clone(),
            self.current_text.clone(),
        );
        repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
            self.file_rel.clone(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(self.base_text.clone().into()),
            ConflictPayload::Text(self.ours_text.clone().into()),
            ConflictPayload::Text(self.theirs_text.clone().into()),
            &self.current_text,
        ));
        repo
    }

    fn cleanup(&self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup fixture");
    }
}

fn load_synthetic_whole_file_conflict(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture: &SyntheticWholeFileConflictFixture,
) {
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });
}

fn assert_streamed_whole_file_two_way_state(pane: &MainPaneView, line_count: usize) -> usize {
    assert_eq!(
        pane.conflict_resolver.rendering_mode(),
        crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile,
        "whole-file conflicts past the large threshold should enter streamed mode",
    );
    assert_eq!(
        pane.conflict_resolver.three_way_len, line_count,
        "three-way line count should still reflect the full document",
    );
    let index = pane
        .conflict_resolver
        .split_row_index()
        .expect("streamed whole-file mode should build a paged split-row index");
    let projection = pane
        .conflict_resolver
        .two_way_split_projection()
        .expect("streamed whole-file mode should expose a split projection");
    assert_eq!(
        pane.conflict_resolver.two_way_row_counts(),
        (index.total_rows(), 0),
        "streamed whole-file mode should expose paged split rows without inline materialization",
    );
    assert_eq!(
        projection.visible_len(),
        pane.conflict_resolver.two_way_split_visible_len(),
        "streamed whole-file mode should expose a split projection",
    );
    assert!(
        index.total_rows() >= line_count,
        "paged split row index should expose at least the full line count, got {}",
        index.total_rows(),
    );

    let total = pane.conflict_resolver.two_way_split_visible_len();
    assert!(
        total >= line_count,
        "streamed two-way visible length should cover the full file, got {total}",
    );

    let deep_ix = total / 2;
    let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
        source_row_ix: _source_ix,
        row,
        conflict_ix: _conflict_ix,
    } = pane
        .conflict_resolver
        .two_way_split_visible_row(deep_ix)
        .expect("deep streamed two-way row should resolve on demand");
    assert!(
        row.old.is_some() || row.new.is_some(),
        "deep streamed two-way row should expose real source text",
    );

    total
}

/// Snapshot of every vertically synced conflict-resolver scroll offset.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ConflictScrollSnapshot {
    base: Pixels,
    ours: Pixels,
    theirs: Pixels,
    output: Pixels,
    gutter: Pixels,
}

fn conflict_scroll_snapshot(pane: &MainPaneView) -> ConflictScrollSnapshot {
    ConflictScrollSnapshot {
        base: uniform_list_offset(&pane.conflict_resolver_diff_scroll).y,
        ours: uniform_list_offset(&pane.conflict_preview_ours_scroll).y,
        theirs: uniform_list_offset(&pane.conflict_preview_theirs_scroll).y,
        output: scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).y,
        gutter: uniform_list_offset(&pane.conflict_resolved_preview_gutter_scroll).y,
    }
}

fn read_conflict_scroll_snapshot(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> ConflictScrollSnapshot {
    cx.update(|_window, app| conflict_scroll_snapshot(view.read(app).main_pane.read(app)))
}

/// A multi-conflict fixture whose two sides have different line counts per
/// block, so the aligned column row space and the resolved output line space
/// genuinely diverge and the conflict-anchored remap has real work to do.
///
/// The divergence comes from the *settled* blocks: an unresolved block now
/// covers its full aligned span in the output too (one named placeholder row
/// plus blank rows), so leaving every block conflicted would make the two
/// spaces line up 1:1 and prove nothing. Every other block is therefore already
/// merged to ours in `current` — it occupies `ours_len` output lines against
/// `max(ours_len, theirs_len)` aligned rows — while the blocks in between stay
/// conflicted so the output still has markers to anchor on.
mod navigation_and_search;
use navigation_and_search::{
    assert_resolved_output_carries_treesitter_classes, other_dark_theme,
    resolved_output_placeholder_protected_ranges_for_test,
};

mod bootstrap;
mod editing;
mod previews;
mod scrolling;
