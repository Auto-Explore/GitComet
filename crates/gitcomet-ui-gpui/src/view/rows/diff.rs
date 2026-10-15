use super::diff_canvas;
use super::diff_text::*;
use super::*;
use crate::view::panes::diff::diff_search::DiffSearchMatcher;
use crate::view::panes::diff::diff_search::DiffSearchOptions;
use crate::view::panes::diff::helpers::state::DiffHorizontalScrollColumn;
use crate::view::panes::diff::helpers::state::VersionedCachedDiffStyledText;
use crate::view::panes::diff::helpers::state::versioned_query_cached_diff_styled_text_is_current;
use blame_presentation::BlameRenderCtx;

pub(super) mod blame_presentation;
mod file_rows;
mod patch_rows;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PatchSplitColumn {
    Left,
    Right,
}

struct DiffRowsRenderPass {
    tab_width: usize,
    annot_hover: Option<(usize, AnnotArea)>,
    stage_area: Option<DiffArea>,
    stage_hover: Option<diff_canvas::DiffStageHover>,
    min_width: Pixels,
    query: SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    reveal_whitespace_chars: bool,
    ui_scale_percent: u32,
    annotation_width: Pixels,
    blame_ctx: Option<BlameRenderCtx>,
}

struct SplitDiffRowsRenderPass {
    common: DiffRowsRenderPass,
    column: PatchSplitColumn,
    is_left: bool,
    region: DiffTextRegion,
    id_missing: &'static str,
    id_oob: &'static str,
    id_src_oob: &'static str,
    id_hidden: &'static str,
}

impl DiffRenderer {
    fn diff_text_segments_cache_get_for_query(
        &mut self,
        key: usize,
        query: &str,
        options: DiffSearchOptions,
        syntax_epoch: u64,
    ) -> Option<&CachedDiffStyledText> {
        if query.is_empty() {
            return self.diff_text_segments_cache_get(key, syntax_epoch);
        }

        self.sync_diff_text_query_overlay_cache(query, options);
        let query_generation = self.diff_text_query_cache_generation;
        if self.diff_text_query_segments_cache.len() <= key {
            self.diff_text_query_segments_cache
                .resize_with(key + 1, || None);
        }

        if versioned_query_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
            query_generation,
        )
        .is_none()
        {
            let base = self
                .diff_text_segments_cache_get(key, syntax_epoch)?
                .clone();
            // The diff view marks its current match by selecting the row, so
            // every match here wears the same wash.
            let overlaid = build_cached_diff_query_overlay_styled_text(
                self.theme,
                &base,
                self.diff_text_query_cache_matcher_shared.as_ref()?,
                DiffSearchMatchEmphasis::Other,
            );
            self.diff_text_query_segments_cache[key] = Some(VersionedCachedDiffStyledText {
                syntax_epoch,
                query_generation,
                styled: overlaid,
            });
        }

        versioned_query_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
            query_generation,
        )
    }

    pub(in super::super) fn render_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let tab_width = this.display_tab_width;

        let annot_hover = this.blame_annot_hover;
        let stage_area = this.diff_stage_gutter_area();
        let stage_hover = this.diff_stage_gutter_hover;
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = this.diff_search_query_matcher_shared();
        let reveal_whitespace_chars = this.reveal_whitespace_chars;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let annotation_width = if this.annotation_active() {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        // The annotation column is painted before the text, so the scroll
        // range has to include it.
        let min_width = this.diff_horizontal_layout_min_width(DiffHorizontalScrollColumn::Primary)
            + annotation_width;
        let blame_ctx = this.blame_render_ctx();

        let pass = DiffRowsRenderPass {
            tab_width,
            annot_hover,
            stage_area,
            stage_hover,
            min_width,
            query,
            query_options,
            query_matcher,
            reveal_whitespace_chars,
            ui_scale_percent,
            annotation_width,
            blame_ctx,
        };

        if this.is_collapsed_diff_projection_active() {
            return file_rows::render_collapsed_diff_rows(this, range, pass, cx);
        }

        if this.is_file_diff_view_active() {
            return file_rows::render_file_diff_rows(this, range, pass, cx);
        }

        patch_rows::render_patch_diff_rows(this, range, pass, cx)
    }

    pub(in super::super) fn render_diff_split_left_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        Self::render_diff_split_rows(this, PatchSplitColumn::Left, range, annot_hover, cx)
    }

    pub(in super::super) fn render_diff_split_right_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        Self::render_diff_split_rows(this, PatchSplitColumn::Right, range, annot_hover, cx)
    }

    fn render_diff_split_rows(
        this: &mut Self,
        column: PatchSplitColumn,
        range: Range<usize>,
        annot_hover: Option<(usize, AnnotArea)>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let tab_width = this.display_tab_width;

        let stage_area = this.diff_stage_gutter_area();
        let stage_hover = this.diff_stage_gutter_hover;
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = this.diff_search_query_matcher_shared();
        let reveal_whitespace_chars = this.reveal_whitespace_chars;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();

        let is_left = matches!(column, PatchSplitColumn::Left);
        // The annotation column is only drawn in the left split column. Reserve
        // its width whenever annotate mode is on — even before blame data is
        // ready — so the column space is stable and content does not shift when
        // blame finishes loading. Only the left column needs the blame context;
        // building it for the right column would clone the blame data and rescan
        // the time range for nothing.
        let blame_ctx = if is_left {
            this.blame_render_ctx()
        } else {
            None
        };
        let annotation_width = if this.annotation_active() && is_left {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        // As in inline: the scroll range includes the annotation column.
        let min_width = this.diff_horizontal_layout_min_width(if is_left {
            DiffHorizontalScrollColumn::Primary
        } else {
            DiffHorizontalScrollColumn::SplitRight
        }) + annotation_width;
        let region = if is_left {
            DiffTextRegion::SplitLeft
        } else {
            DiffTextRegion::SplitRight
        };
        // Static ID tags to avoid format!/String allocation in element IDs.
        let (id_missing, id_oob, id_src_oob, id_hidden) = if is_left {
            (
                "diff_split_left_missing",
                "diff_split_left_oob",
                "diff_split_left_src_oob",
                "diff_split_left_hidden_header",
            )
        } else {
            (
                "diff_split_right_missing",
                "diff_split_right_oob",
                "diff_split_right_src_oob",
                "diff_split_right_hidden_header",
            )
        };

        let pass = SplitDiffRowsRenderPass {
            common: DiffRowsRenderPass {
                tab_width,
                annot_hover,
                stage_area,
                stage_hover,
                min_width,
                query,
                query_options,
                query_matcher,
                reveal_whitespace_chars,
                ui_scale_percent,
                annotation_width,
                blame_ctx,
            },
            column,
            is_left,
            region,
            id_missing,
            id_oob,
            id_src_oob,
            id_hidden,
        };

        if this.is_collapsed_diff_projection_active() {
            return file_rows::render_collapsed_diff_split_rows(this, range, pass, cx);
        }

        if this.is_file_diff_view_active() {
            return file_rows::render_file_diff_split_rows(this, range, pass, cx);
        }

        patch_rows::render_patch_diff_split_rows(this, range, pass, cx)
    }
}

#[cfg(test)]
mod tests;
