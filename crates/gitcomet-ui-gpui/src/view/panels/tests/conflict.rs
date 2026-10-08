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

// Use a .txt path for geometry and outline tests so the large document does
// not also schedule HTML parsing. Syntax regressions use .html explicitly.
struct SyntheticLargeConflictFixture {
    _directory: tempfile::TempDir,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    abs_path: std::path::PathBuf,
    fixture_line_count: usize,
    conflict_block_count: usize,
    first_conflict_line: u32,
    base_text: Arc<str>,
    ours_text: Arc<str>,
    theirs_text: Arc<str>,
    current_text: Arc<str>,
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

        let directory = tempfile::Builder::new()
            .prefix(&format!("gitcomet_{workdir_label}_"))
            .tempdir()
            .unwrap();
        let workdir = directory.path().to_path_buf();
        let file_rel = std::path::PathBuf::from(file_rel);
        let abs_path = workdir.join(&file_rel);

        let capacity = fixture_line_count.saturating_mul(128);
        let header = "<!doctype html>\n<html lang=\"en\">\n<body class=\"fixture-root\">";
        let mut base_text = String::with_capacity(capacity);
        let mut ours_text = String::with_capacity(capacity);
        let mut theirs_text = String::with_capacity(capacity);
        let mut current_text =
            String::with_capacity(capacity.saturating_add(conflict_block_count * 160));
        for text in [
            &mut base_text,
            &mut ours_text,
            &mut theirs_text,
            &mut current_text,
        ] {
            text.push_str(header);
        }
        let remaining_context = fixture_line_count - 3 - conflict_block_count;
        let context_per_slot = remaining_context / conflict_block_count;
        let context_remainder = remaining_context % conflict_block_count;
        let mut next_context_row = 0usize;
        let mut side_line_count = 3usize;
        let mut first_conflict_line = None;
        for conflict_ix in 0..conflict_block_count {
            let base = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"base\">base {conflict_ix}</main>"
            );
            let ours = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"ours\">ours {conflict_ix}</main>"
            );
            let theirs = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"theirs\">theirs {conflict_ix}</main>"
            );
            first_conflict_line
                .get_or_insert(u32::try_from(side_line_count + 1).unwrap_or(u32::MAX));
            append_synthetic_conflict_line(&mut base_text, &base);
            append_synthetic_conflict_line(&mut ours_text, &ours);
            append_synthetic_conflict_line(&mut theirs_text, &theirs);
            for line in ["<<<<<<< ours", &ours, "=======", &theirs, ">>>>>>> theirs"] {
                append_synthetic_conflict_line(&mut current_text, line);
            }
            let slot_lines = context_per_slot + usize::from(conflict_ix < context_remainder);
            append_synthetic_large_conflict_context(
                &mut base_text,
                &mut ours_text,
                &mut theirs_text,
                &mut current_text,
                &mut next_context_row,
                slot_lines,
            );
            side_line_count += 1 + slot_lines;
        }
        assert_eq!(side_line_count, fixture_line_count);
        Self {
            _directory: directory,
            workdir,
            file_rel,
            abs_path,
            fixture_line_count,
            conflict_block_count,
            first_conflict_line: first_conflict_line.unwrap_or(1),
            base_text: base_text.into(),
            ours_text: ours_text.into(),
            theirs_text: theirs_text.into(),
            current_text: current_text.into(),
        }
    }

    fn write(&self) {
        std::fs::create_dir_all(self.abs_path.parent().expect("fixture file parent"))
            .expect("create fixture dir");
        std::fs::write(&self.abs_path, self.current_text.as_bytes()).expect("write fixture");
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
            ConflictPayload::Text(Arc::clone(&self.base_text)),
            ConflictPayload::Text(Arc::clone(&self.ours_text)),
            ConflictPayload::Text(Arc::clone(&self.theirs_text)),
            &self.current_text,
        ));
        repo
    }

    fn cleanup(&self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup fixture");
    }
}

fn append_synthetic_conflict_line(text: &mut String, line: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(line);
}

fn append_synthetic_large_conflict_context(
    base: &mut String,
    ours: &mut String,
    theirs: &mut String,
    current: &mut String,
    next_context_row: &mut usize,
    count: usize,
) {
    use std::fmt::Write as _;
    let mut line = String::with_capacity(128);
    for _ in 0..count {
        let row = *next_context_row;
        line.clear();
        write!(line, "<section id=\"panel-{row}\" data-row=\"{row}\"><div class=\"copy\">row {row}</div></section>").unwrap();
        for text in [&mut *base, &mut *ours, &mut *theirs, &mut *current] {
            append_synthetic_conflict_line(text, &line);
        }
        *next_context_row = next_context_row.saturating_add(1);
    }
}

struct SyntheticWholeFileConflictFixture {
    _directory: tempfile::TempDir,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    abs_path: std::path::PathBuf,
    line_count: usize,
    base_text: Arc<str>,
    ours_text: Arc<str>,
    theirs_text: Arc<str>,
    current_text: Arc<str>,
}

impl SyntheticWholeFileConflictFixture {
    fn new(workdir_label: &str, file_rel: &str, line_count: usize) -> Self {
        assert!(
            line_count >= 5,
            "whole-file conflict fixture needs room for html wrapper lines"
        );

        let directory = tempfile::Builder::new()
            .prefix(&format!("gitcomet_{workdir_label}_"))
            .tempdir()
            .unwrap();
        let workdir = directory.path().to_path_buf();
        let file_rel = std::path::PathBuf::from(file_rel);
        let abs_path = workdir.join(&file_rel);

        let build_side = |side: &str| {
            use std::fmt::Write as _;
            let mut text = String::with_capacity(line_count.saturating_mul(128));
            write!(
                text,
                "<!doctype html>\n<html lang=\"en\">\n<body class=\"whole-file-{side}\">"
            )
            .unwrap();
            for row in 0..line_count.saturating_sub(5) {
                write!(text, "\n<section id=\"panel-{row}\" data-side=\"{side}\"><div>{side} {row}</div></section>").unwrap();
            }
            text.push_str("\n</body>\n</html>");
            assert_eq!(text.lines().count(), line_count);
            text
        };
        let base_text = build_side("base");
        let ours_text = build_side("ours");
        let theirs_text = build_side("theirs");
        let current_text =
            format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");
        Self {
            _directory: directory,
            workdir,
            file_rel,
            abs_path,
            line_count,
            base_text: base_text.into(),
            ours_text: ours_text.into(),
            theirs_text: theirs_text.into(),
            current_text: current_text.into(),
        }
    }

    fn write(&self) {
        std::fs::create_dir_all(self.abs_path.parent().expect("fixture file parent"))
            .expect("create fixture dir");
        std::fs::write(&self.abs_path, self.current_text.as_bytes()).expect("write fixture");
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
            ConflictPayload::Text(Arc::clone(&self.base_text)),
            ConflictPayload::Text(Arc::clone(&self.ours_text)),
            ConflictPayload::Text(Arc::clone(&self.theirs_text)),
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
