use super::*;

#[derive(Clone, Copy)]
enum DiffTextOffsetBias {
    Start,
    End,
}

fn diff_text_local_range_from_source_ranges(
    selected: Range<usize>,
    visual: Range<usize>,
) -> Option<Range<usize>> {
    let start = selected.start.max(visual.start);
    let end = selected.end.min(visual.end);
    if start >= end {
        return None;
    }
    Some(start.saturating_sub(visual.start)..end.saturating_sub(visual.start))
}

impl MainPaneView {
    fn diff_text_normalized_selection(&self) -> Option<(DiffTextPos, DiffTextPos)> {
        let a = self.diff_text_anchor?;
        let b = self.diff_text_head?;
        Some(if a.cmp_key() <= b.cmp_key() {
            (a, b)
        } else {
            (b, a)
        })
    }

    pub(in super::super::super) fn diff_text_selection_visible_range(
        &self,
    ) -> Option<(usize, usize)> {
        let (start, end) = self.diff_text_normalized_selection()?;
        if start == end {
            return None;
        }
        let start_ix = self.diff_text_visible_ix_for_source_pos(start, DiffTextOffsetBias::Start);
        let end_ix = self.diff_text_visible_ix_for_source_pos(end, DiffTextOffsetBias::End);
        Some((start_ix.min(end_ix), start_ix.max(end_ix)))
    }

    pub(in super::super::super) fn sync_diff_focus_to_text_selection(&mut self) {
        if let Some((start, end)) = self.diff_text_normalized_selection()
            && start != end
        {
            self.diff_selection_anchor =
                Some(self.diff_text_visible_ix_for_source_pos(end, DiffTextOffsetBias::End));
            self.diff_selection_range = None;
        }
    }

    pub(in super::super::super) fn clear_diff_text_selection(&mut self) {
        self.diff_text_selecting = false;
        self.diff_text_anchor = None;
        self.diff_text_head = None;
        self.diff_text_autoscroll_target = None;
        // Reached on every file switch, view-mode change and diff rebuild, all of
        // which move the row indices the pair spans were projected onto.
        self.diff_text_pair_match = None;
    }

    pub(in super::super::super) fn clear_diff_selection_state(&mut self) {
        self.diff_selection_anchor = None;
        self.diff_selection_range = None;
        self.clear_diff_text_selection();
    }

    pub(in super::super::super) fn diff_text_selection_color(&self) -> gpui::Rgba {
        self.theme.colors.editor.selection_background
    }

    pub(in super::super::super) fn set_diff_text_hitbox(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        hitbox: DiffTextHitbox,
    ) {
        self.diff_text_hitboxes.insert((visible_ix, region), hitbox);
    }

    /// Record where a row painted its stage/unstage gutter button. Hover and
    /// click routing go through the row's hitbox; this map exists so tests can
    /// aim at the button without re-deriving its geometry.
    pub(in super::super::super) fn set_diff_stage_gutter_cell(
        &mut self,
        visible_ix: usize,
        slot: rows::DiffStageSlot,
        bounds: gpui::Bounds<Pixels>,
    ) {
        self.diff_stage_gutter_cells
            .insert((visible_ix, slot), bounds);
    }

    fn diff_text_pos_from_hitbox(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        let hitbox = self.diff_text_hitboxes.get(&(visible_ix, region))?;
        // A press off the text is not a press on it: the caller has to be able
        // to tell "not this row" from "the start of this row", or clicking a
        // row's padding would begin a selection and follow whatever link the
        // nearest character happens to sit in.
        if !hitbox.bounds.contains(&position) {
            return None;
        }
        self.diff_text_pos_in_hitbox(hitbox, region, position)
    }

    /// Where a point outside every row belongs, once the nearest row is known.
    ///
    /// Unlike [`Self::diff_text_pos_from_hitbox`] this pulls the point onto the
    /// row rather than rejecting it. Leaving the text is how a drag selects
    /// past the end of a line, and the flowing markdown preview is full of
    /// places to leave it: the margins between blocks, the padding inside a
    /// code block, a picture, and every line shorter than its neighbours.
    fn diff_text_pos_from_nearest_hitbox(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        let hitbox = self.diff_text_hitboxes.get(&(visible_ix, region))?;
        self.diff_text_pos_in_hitbox(hitbox, region, position)
    }

    /// The offset a point resolves to inside one row, clamping to the row's
    /// edges. For a point the row already contains the clamp does nothing.
    fn diff_text_pos_in_hitbox(
        &self,
        hitbox: &DiffTextHitbox,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        if let Some(wrapped) = &hitbox.wrapped {
            // A wrapped row spans several visual lines, so the click resolves
            // against the layout it was painted with; `Err` is the clamp to the
            // nearest boundary, which is what a drag past the text wants.
            let painted_offset = match wrapped.layout.index_for_position(position) {
                Ok(offset) | Err(offset) => offset,
            };
            return Some(DiffTextPos {
                source_visible_ix: hitbox.source_visible_ix,
                region,
                offset: hitbox
                    .text_start_offset
                    .saturating_add(wrapped.row_offset(painted_offset).min(hitbox.text_len)),
            });
        }
        // A single shaped line lies wholly below a point above it and wholly
        // above one below it, so those resolve to its ends rather than to
        // whatever character shares their x.
        let local_offset = if position.y < hitbox.bounds.top() {
            0
        } else if position.y > hitbox.bounds.bottom() {
            hitbox.text_len
        } else {
            let x = (position.x - hitbox.bounds.left()).max(px(0.0));
            if let Some(cell_width) = hitbox.streamed_ascii_monospace_cell_width {
                if cell_width <= px(0.0) {
                    0
                } else {
                    (((x / cell_width) + 0.5).floor() as usize).min(hitbox.text_len)
                }
            } else {
                let layout = &self.diff_text_layout_cache.get(&hitbox.layout_key)?.layout;
                layout
                    .closest_index_for_x(x)
                    .min(layout.len())
                    .min(hitbox.text_len)
            }
        };
        let local_offset = hitbox
            .offset_map
            .as_ref()
            .map(|map| map.source_offset_for_display(local_offset))
            .unwrap_or(local_offset);
        Some(DiffTextPos {
            source_visible_ix: hitbox.source_visible_ix,
            region,
            offset: hitbox.text_start_offset.saturating_add(local_offset),
        })
    }

    /// The box a range of one row's text occupies on screen — the inverse of
    /// [`Self::diff_text_pos_in_hitbox`], for anchoring a menu to the run of
    /// text it acts on.
    ///
    /// `range` is in the offset space that method reports. A range that wraps
    /// reports its first visual line only: that is where it begins, and an end
    /// x taken from a later line says nothing about how far the first one runs.
    fn diff_text_bounds_in_hitbox(
        &self,
        hitbox: &DiffTextHitbox,
        range: Range<usize>,
    ) -> Option<Bounds<Pixels>> {
        let local = |offset: usize| {
            offset
                .saturating_sub(hitbox.text_start_offset)
                .min(hitbox.text_len)
        };
        let (start, end) = (local(range.start), local(range.end));

        if let Some(wrapped) = &hitbox.wrapped {
            let top_left = wrapped
                .layout
                .position_for_index(wrapped.painted_offset(start))?;
            let right = wrapped
                .layout
                .position_for_index(wrapped.painted_offset(end))
                .filter(|tail| tail.y <= top_left.y)
                .map_or(hitbox.bounds.right(), |tail| tail.x);
            return Some(Bounds::from_corners(
                top_left,
                point(
                    right.max(top_left.x),
                    top_left.y + wrapped.layout.line_height(),
                ),
            ));
        }

        let x_for = |offset: usize| -> Option<Pixels> {
            let display_offset = hitbox
                .offset_map
                .as_ref()
                .map(|map| map.display_offset_for_source(offset))
                .unwrap_or(offset);
            if let Some(cell_width) = hitbox.streamed_ascii_monospace_cell_width {
                return Some(cell_width * display_offset as f32);
            }
            let layout = &self.diff_text_layout_cache.get(&hitbox.layout_key)?.layout;
            Some(layout.x_for_index(display_offset.min(layout.len())))
        };
        let left = hitbox.bounds.left() + x_for(start)?;
        let right = hitbox.bounds.left() + x_for(end)?;
        Some(Bounds::from_corners(
            point(left, hitbox.bounds.top()),
            point(right.max(left), hitbox.bounds.bottom()),
        ))
    }

    /// Bring the current quick-search match into view sideways.
    ///
    /// Runs from the render pass, just before the hitbox map is rebuilt, so it
    /// reads the geometry the previous frame painted — which is the first frame
    /// where the row is at its post-vertical-scroll position. Long lines are the
    /// whole point: without this, jumping to a match 300 columns out scrolls the
    /// row into view and leaves the hit off the right edge.
    ///
    /// Both split columns are considered: in split view the two sides hold
    /// different text and scroll independently, so each one is revealed only if
    /// the match is actually in it.
    pub(in super::super::super) fn apply_pending_diff_search_horizontal_reveal(
        &mut self,
        window: &mut gpui::Window,
    ) {
        let Some((visible_ix, attempts_left)) = self.diff_search_horizontal_reveal else {
            return;
        };
        if self.diff_text_hitboxes.is_empty() && self.conflict_text_hitboxes.is_empty() {
            // Nothing painted at all — the pane has not drawn since the jump.
            // Keep the request without spending an attempt on it, and ask for
            // the frame it is waiting on: neither `set_offset` nor
            // `scroll_to_item_strict` schedules one, so on an idle app the
            // reveal would simply never happen.
            window.request_animation_frame();
            return;
        }

        let matcher = self.diff_search_current_matcher();
        if matcher.is_empty() || matcher.regex_error().is_some() {
            self.diff_search_horizontal_reveal = None;
            return;
        }

        let revealed = if self.conflict_text_hitboxes.is_empty() {
            // Every region, not the first that matches: the split columns are
            // separate scrollables and a hit can be in both.
            let mut revealed = false;
            for region in [
                DiffTextRegion::Inline,
                DiffTextRegion::SplitLeft,
                DiffTextRegion::SplitRight,
            ] {
                revealed |= self.reveal_diff_search_match_in_region(visible_ix, region, &matcher);
            }
            revealed
        } else {
            self.reveal_conflict_search_match_horizontally(visible_ix, &matcher)
        };

        // A frame that painted the row settles the matter either way: it either
        // moved or it did not need to. Only a frame that has not painted it yet
        // is worth retrying.
        self.diff_search_horizontal_reveal = if revealed || attempts_left <= 1 {
            None
        } else {
            // Still waiting on the frame that paints the row where the vertical
            // scroll put it, so ask for one.
            window.request_animation_frame();
            Some((visible_ix, attempts_left - 1))
        };
    }

    /// Find the match inside a row's *painted* text.
    ///
    /// With "reveal whitespace characters" on, the painted text has every space
    /// swapped for `·` and every tab for `→`, so a query holding either would
    /// never be found again and the reveal would silently do nothing. The query
    /// is put through the same substitution to match it — literal queries only,
    /// since rewriting a regex would change what it means.
    pub(super) fn painted_search_range(
        &self,
        painted: &str,
        matcher: &super::diff_search::DiffSearchMatcher,
    ) -> Option<Range<usize>> {
        let mut ranges = Vec::new();
        matcher.find_ranges_into(painted, &mut ranges, 1);
        if let Some(range) = ranges.first() {
            return Some(range.clone());
        }

        let options = self.diff_search_options_or_default();
        if !self.reveal_whitespace_chars || options.regex {
            return None;
        }
        let revealed = crate::view::rows::whitespace_visible_line_text(matcher.query())
            .as_ref()
            .to_string();
        if revealed == matcher.query() {
            return None;
        }
        let revealed = super::diff_search::DiffSearchMatcher::new(&revealed, options);
        ranges.clear();
        revealed.find_ranges_into(painted, &mut ranges, 1);
        ranges.first().cloned()
    }

    /// Reveals the match in one region, reporting whether the row was painted
    /// there at all — which is what tells the caller to stop retrying.
    fn reveal_diff_search_match_in_region(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        matcher: &super::diff_search::DiffSearchMatcher,
    ) -> bool {
        let Some(hitbox) = self.diff_text_hitboxes.get(&(visible_ix, region)) else {
            return false;
        };
        // A wrapped row has no off-screen right edge to chase; it already broke
        // the line to fit the pane.
        if hitbox.wrapped.is_some() {
            return true;
        }

        // Searched against the painted text, so the offsets are the display
        // offsets `x_for_index` measures in and no source→display remapping is
        // needed on the way back out.
        let painted_text = hitbox.painted_text.clone();
        let Some(range) = self.painted_search_range(painted_text.as_ref(), matcher) else {
            return true;
        };
        let Some(hitbox) = self.diff_text_hitboxes.get(&(visible_ix, region)) else {
            return true;
        };

        let (local_left, local_right) =
            if let Some(cell_width) = hitbox.streamed_ascii_monospace_cell_width {
                (
                    cell_width * range.start as f32,
                    cell_width * range.end as f32,
                )
            } else {
                let Some(entry) = self.diff_text_layout_cache.get(&hitbox.layout_key) else {
                    return true;
                };
                let layout = &entry.layout;
                (
                    layout.x_for_index(range.start.min(layout.len())),
                    layout.x_for_index(range.end.min(layout.len())),
                )
            };

        let row_left = hitbox.bounds.left();
        let handle = self.scroll_handle_for_diff_text_autoscroll_target(
            self.diff_text_autoscroll_target_for_region(region),
        );
        let viewport = handle.bounds();
        let offset = handle.offset();
        // Hitbox bounds are window space with the scroll already applied.
        let to_content = |x: Pixels| row_left + x - viewport.origin.x - offset.x;
        let Some(target_x) = super::helpers::reveal_scroll_x(
            to_content(local_left),
            to_content(local_right),
            viewport.size.width,
            handle.max_offset().x,
            offset.x,
        ) else {
            return true;
        };
        handle.set_offset(point(target_x, offset.y));
        true
    }

    /// Which scrollable a region's rows live in, without needing a mouse
    /// position to disambiguate the split columns.
    fn diff_text_autoscroll_target_for_region(
        &self,
        region: DiffTextRegion,
    ) -> DiffTextAutoscrollTarget {
        if self.is_file_preview_active() {
            return DiffTextAutoscrollTarget::WorktreePreview;
        }
        match region {
            DiffTextRegion::SplitRight => DiffTextAutoscrollTarget::DiffSplitRight,
            _ => DiffTextAutoscrollTarget::DiffLeftOrInline,
        }
    }

    fn diff_text_pos_for_mouse(&self, position: Point<Pixels>) -> Option<DiffTextPos> {
        if self.diff_text_hitboxes.is_empty() {
            return None;
        }

        let restrict_region = self
            .diff_text_selecting
            .then_some(self.diff_text_anchor)
            .flatten()
            .map(|p| p.region)
            .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight));

        for ((visible_ix, region), hitbox) in &self.diff_text_hitboxes {
            if restrict_region.is_some_and(|restrict| restrict != *region) {
                continue;
            }
            if hitbox.bounds.contains(&position) {
                return self.diff_text_pos_from_hitbox(*visible_ix, *region, position);
            }
        }

        // Vertical distance decides first. Text runs in lines, so the row a
        // point off the text belongs to is the one beside it, however far along
        // the line it is — adding the two distances instead let a point in the
        // margin between two blocks pick a row several blocks away that merely
        // shared its column.
        let mut best: Option<((usize, DiffTextRegion), (Pixels, Pixels))> = None;
        for (key, hitbox) in &self.diff_text_hitboxes {
            if restrict_region.is_some_and(|restrict| restrict != key.1) {
                continue;
            }
            let dy = if position.y < hitbox.bounds.top() {
                hitbox.bounds.top() - position.y
            } else if position.y > hitbox.bounds.bottom() {
                position.y - hitbox.bounds.bottom()
            } else {
                px(0.0)
            };
            let dx = if position.x < hitbox.bounds.left() {
                hitbox.bounds.left() - position.x
            } else if position.x > hitbox.bounds.right() {
                position.x - hitbox.bounds.right()
            } else {
                px(0.0)
            };
            // Rows are stored in a hash map, so ties are broken on the row
            // itself rather than on iteration order.
            let rank = (dy, dx, key.0, key.1.order());
            let closer = match best {
                None => true,
                Some((best_key, (best_dy, best_dx))) => {
                    rank < (best_dy, best_dx, best_key.0, best_key.1.order())
                }
            };
            if closer {
                best = Some((*key, (dy, dx)));
            }
        }
        let ((visible_ix, region), _) = best?;
        self.diff_text_pos_from_nearest_hitbox(visible_ix, region, position)
    }

    #[cfg(test)]
    pub(in crate::view) fn diff_text_hitbox_bounds_for_tests(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<Bounds<Pixels>> {
        self.diff_text_hitboxes
            .get(&(visible_ix, region))
            .map(|hitbox| hitbox.bounds)
    }

    /// Byte offset in the text a row painted, for a point inside that row.
    pub(in crate::view) fn diff_text_offset_for_position(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<usize> {
        self.diff_text_pos_from_hitbox(visible_ix, region, position)
            .map(|pos| pos.offset)
    }

    pub(in super::super::super) fn diff_text_visual_source_range_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> (usize, Range<usize>) {
        let source_visible_ix = self
            .diff_source_visible_ix_for_visible_ix(visible_ix)
            .unwrap_or(visible_ix);
        let range = if let Some(wrap) = self.diff_text_wrap_for_visible_ix(visible_ix) {
            wrap.range_for_region(region)
        } else {
            0..self.diff_text_line_len_for_region(visible_ix, region)
        };
        (source_visible_ix, range)
    }

    fn diff_text_full_line_len_for_region(
        &self,
        source_visible_ix: usize,
        region: DiffTextRegion,
    ) -> usize {
        self.diff_text_full_line_for_region(source_visible_ix, region)
            .len()
    }

    fn diff_text_visible_ix_for_source_pos(
        &self,
        pos: DiffTextPos,
        bias: DiffTextOffsetBias,
    ) -> usize {
        if !(self.diff_word_wrap && self.diff_wrap_visible_cache_key.is_some()) {
            return pos.source_visible_ix;
        }

        let target_offset = match bias {
            DiffTextOffsetBias::Start => pos.offset,
            DiffTextOffsetBias::End => pos.offset.saturating_sub(1),
        };
        let mut first_for_source = None;
        let mut last_for_source = None;
        for (visible_ix, row) in self.diff_wrap_visible_rows.iter().enumerate() {
            if row.source_visible_ix != pos.source_visible_ix {
                continue;
            }
            first_for_source.get_or_insert(visible_ix);
            last_for_source = Some(visible_ix);
            let (_, range) = self.diff_text_visual_source_range_for_region(visible_ix, pos.region);
            if range.is_empty() {
                if pos.offset == range.start {
                    return visible_ix;
                }
                continue;
            }
            if range.start <= target_offset && target_offset < range.end {
                return visible_ix;
            }
        }

        match bias {
            DiffTextOffsetBias::Start => first_for_source,
            DiffTextOffsetBias::End => last_for_source.or(first_for_source),
        }
        .unwrap_or(pos.source_visible_ix)
    }

    fn diff_text_visible_range_for_source_range(
        &self,
        start_source_visible_ix: usize,
        end_source_visible_ix: usize,
    ) -> Option<(usize, usize)> {
        if !(self.diff_word_wrap && self.diff_wrap_visible_cache_key.is_some()) {
            return Some((start_source_visible_ix, end_source_visible_ix));
        }

        let mut start_visible_ix = None;
        let mut end_visible_ix = None;
        for (visible_ix, row) in self.diff_wrap_visible_rows.iter().enumerate() {
            if row.source_visible_ix < start_source_visible_ix
                || row.source_visible_ix > end_source_visible_ix
            {
                continue;
            }
            start_visible_ix.get_or_insert(visible_ix);
            end_visible_ix = Some(visible_ix);
        }
        Some((start_visible_ix?, end_visible_ix?))
    }

    fn set_diff_text_selection(
        &mut self,
        anchor: DiffTextPos,
        head: DiffTextPos,
        suppress_clicks: usize,
    ) {
        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(anchor);
        self.diff_text_head = Some(head);
        self.diff_selection_range = None;
        self.sync_diff_focus_to_text_selection();
        self.diff_suppress_clicks_remaining = suppress_clicks.min(u8::MAX as usize) as u8;
    }

    fn select_diff_text_token_at_mouse(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) {
        let Some(pos) = self.diff_text_pos_from_hitbox(visible_ix, region, position) else {
            return;
        };
        let text = self.diff_text_full_line_for_region(pos.source_visible_ix, pos.region);
        let range = crate::text_selection::token_range_for_offset(text.as_ref(), pos.offset);
        let anchor = DiffTextPos {
            source_visible_ix: pos.source_visible_ix,
            region: pos.region,
            offset: range.start,
        };
        let head = DiffTextPos {
            source_visible_ix: pos.source_visible_ix,
            region: pos.region,
            offset: range.end,
        };
        self.set_diff_text_selection(anchor, head, 1);
    }

    fn select_diff_text_line_at_mouse(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) {
        let Some(pos) = self.diff_text_pos_from_hitbox(visible_ix, region, position) else {
            return;
        };
        let line_len = self.diff_text_full_line_len_for_region(pos.source_visible_ix, pos.region);
        let anchor = DiffTextPos {
            source_visible_ix: pos.source_visible_ix,
            region: pos.region,
            offset: 0,
        };
        let head = DiffTextPos {
            source_visible_ix: pos.source_visible_ix,
            region: pos.region,
            offset: line_len,
        };
        self.set_diff_text_selection(anchor, head, 1);
    }

    /// Open the link menu when a plain click lands on a web link in the
    /// rendered markdown preview, and report whether it did.
    ///
    /// A double or triple click is still a text selection — only a single
    /// click follows the link, so selecting the words of a link keeps working.
    pub(in super::super::super) fn handle_markdown_preview_link_click(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
        click_count: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if click_count > 1 || self.diff_text_has_selection() {
            return false;
        }
        let Some((url, span)) = self.markdown_preview_link_span_at(visible_ix, region, position)
        else {
            return false;
        };

        // Anchor on the link's own box, so the menu opens flush under the words
        // it describes rather than under the row that happens to hold them.
        let anchor = self
            .diff_text_hitboxes
            .get(&(visible_ix, region))
            .and_then(|hitbox| self.diff_text_bounds_in_hitbox(hitbox, span));
        match anchor {
            Some(bounds) => {
                self.open_popover_for_bounds(PopoverKind::WebLinkMenu { url }, bounds, window, cx)
            }
            None => self.open_popover_at(PopoverKind::WebLinkMenu { url }, position, window, cx),
        }
        true
    }

    /// Open the link menu for something that is a link in its own right — a
    /// badge or any other picture wrapped in one — rather than a span of text.
    ///
    /// The picture's own box is what the menu wants to hang off; the click
    /// point stands in for the frames where it has not been painted yet.
    pub(in crate::view) fn open_markdown_preview_link_menu(
        &mut self,
        url: SharedString,
        anchor_bounds: Option<Bounds<Pixels>>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match anchor_bounds {
            Some(bounds) => {
                self.open_popover_for_bounds(PopoverKind::WebLinkMenu { url }, bounds, window, cx)
            }
            None => self.open_popover_at(PopoverKind::WebLinkMenu { url }, position, window, cx),
        }
    }

    pub(in super::super::super) fn handle_diff_text_mouse_down(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
        click_count: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        // Deliberately does not claim the press: the diff row's own release
        // handler reads the claim, and this gesture starts on that same row.
        // A drag that actually moved is suppressed by
        // `diff_suppress_clicks_remaining` instead, which a plain click leaves
        // alone.
        // Cleared for every press, not just the ones that resolve to a position:
        // a double- or triple-click selects a span, and a press that misses the
        // text entirely is a press away from the pair. Both must dismiss it, and
        // both skip `begin_diff_text_selection`'s set below.
        self.diff_text_pair_match = None;
        match click_count {
            3.. => {
                self.select_diff_text_line_at_mouse(visible_ix, region, position);
            }
            2 => {
                self.select_diff_text_token_at_mouse(visible_ix, region, position);
            }
            _ if self.diff_text_has_selection() => {
                self.begin_diff_text_selection(visible_ix, region, position);
                if self.diff_text_selecting {
                    self.diff_suppress_clicks_remaining = 1;
                }
            }
            _ => {
                self.begin_diff_text_selection(visible_ix, region, position);
                self.begin_diff_text_scroll_tracking(position, cx);
            }
        }
    }

    pub(in super::super::super) fn begin_diff_text_selection(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) {
        let Some(pos) = self.diff_text_pos_from_hitbox(visible_ix, region, position) else {
            return;
        };
        self.diff_text_selecting = true;
        self.diff_text_anchor = Some(pos);
        self.diff_text_head = Some(pos);
        self.diff_selection_range = None;
        self.diff_text_last_mouse_pos = position;
        self.diff_suppress_clicks_remaining = 0;
        self.diff_text_pair_match = self.diff_text_pair_match_for_pos(&pos);
    }

    pub(in super::super::super) fn begin_diff_text_scroll_tracking(
        &mut self,
        position: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.diff_text_selecting {
            return;
        }

        self.diff_text_last_mouse_pos = position;
        self.diff_text_autoscroll_target =
            Some(self.diff_text_autoscroll_target_for_position(position));
        self.diff_text_autoscroll_seq = self.diff_text_autoscroll_seq.wrapping_add(1);

        let autoscroll_seq = self.diff_text_autoscroll_seq;
        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let mut keep_going = false;
                let _ = view.update(cx, |this, cx| {
                    if !this.diff_text_selecting {
                        return;
                    }
                    if this.diff_text_autoscroll_seq != autoscroll_seq {
                        return;
                    }

                    keep_going = true;
                    let changed = this.tick_diff_text_selection_autoscroll(cx);
                    if changed {
                        cx.notify();
                    }
                });

                if !keep_going {
                    break;
                }
            },
        )
        .detach();
    }

    pub(in super::super::super) fn update_diff_text_selection_from_mouse(
        &mut self,
        position: Point<Pixels>,
    ) {
        if !self.diff_text_selecting {
            return;
        }
        self.diff_text_last_mouse_pos = position;
        let Some(pos) = self.diff_text_pos_for_mouse(position) else {
            return;
        };
        if self.diff_text_head != Some(pos) {
            self.diff_text_head = Some(pos);
            if self
                .diff_text_normalized_selection()
                .is_some_and(|(a, b)| a != b)
            {
                // A selection is the user working on a span, not sitting in one;
                // a pair lit at each end of it reads as part of the selection.
                self.diff_text_pair_match = None;
                self.sync_diff_focus_to_text_selection();
                self.diff_suppress_clicks_remaining = 1;
            }
        }
    }

    pub(in super::super::super) fn end_diff_text_selection(&mut self) {
        self.diff_text_selecting = false;
        self.diff_text_autoscroll_target = None;
    }

    pub(in super::super::super) fn diff_text_has_selection(&self) -> bool {
        self.diff_text_normalized_selection()
            .is_some_and(|(a, b)| a != b)
    }

    pub(in super::super::super) fn diff_text_local_selection_range(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<Range<usize>> {
        let (source_visible_ix, visual_range) =
            self.diff_text_visual_source_range_for_region(visible_ix, region);
        let selected =
            self.diff_text_source_selection_range(source_visible_ix, region, visual_range.end)?;
        diff_text_local_range_from_source_ranges(selected, visual_range)
    }

    #[cfg(test)]
    pub(in crate::view) fn diff_text_pair_match_for_tests(&self) -> Option<&DiffTextPairMatch> {
        self.diff_text_pair_match.as_ref()
    }

    pub(in super::super::super) fn diff_text_pair_match_color(&self) -> gpui::Rgba {
        self.theme.colors.editor.bracket_match_background
    }

    /// Whether the row's painted text is a truncated stand-in for its line.
    ///
    /// `file_diff_display_text` replaces lines past a size gate with a prefix
    /// plus an ellipsis, so display offsets into them do not convert back to
    /// real bytes.
    fn diff_text_row_is_display_truncated(
        &self,
        source_visible_ix: usize,
        region: DiffTextRegion,
    ) -> bool {
        use crate::view::file_diff_display::should_truncate_file_diff_display as truncated;
        if self.is_file_preview_active() {
            return self
                .worktree_preview_line_raw_text(source_visible_ix)
                .is_some_and(|raw| truncated(&raw));
        }
        let Some(row_ix) = self.diff_source_mapped_ix_for_visible_ix(source_visible_ix) else {
            return false;
        };
        match self.diff_view {
            DiffViewMode::Inline => self
                .file_diff_inline_render_data(row_ix)
                .is_some_and(|row| truncated(&row.text)),
            DiffViewMode::Split => self
                .file_diff_split_render_data(row_ix)
                .and_then(|row| match region {
                    DiffTextRegion::SplitLeft => row.old,
                    DiffTextRegion::SplitRight => row.new,
                    DiffTextRegion::Inline => None,
                })
                .is_some_and(|text| truncated(&text)),
        }
    }

    /// Which real document a row's text came from.
    ///
    /// A diff interleaves two file versions, so a pair found in one of them can
    /// only ever be projected back through that same side's line map. This is
    /// also why a pair can never span the two halves of a split view: the two
    /// sides are different documents and the matcher only ever walks one tree.
    fn diff_text_pair_document_for_row(
        &self,
        source_visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<(rows::PreparedDiffSyntaxDocument, usize, DiffTextPairSide)> {
        if self.is_markdown_preview_active() {
            return None;
        }

        if self.is_file_preview_active() {
            if region != DiffTextRegion::Inline {
                return None;
            }
            let document = self.worktree_preview_prepared_syntax_document()?;
            return Some((document, source_visible_ix, DiffTextPairSide::Preview));
        }

        // Deliberately not gated on `is_file_diff_view_active()`: that is a
        // *mode* flag (it requires `DiffContentMode::Full`), and file-diff rows
        // also render under the collapsed projection. Asking the row model and
        // the document cache directly covers every mode and fails closed when
        // either is absent, which is what a patch/commit diff does.
        let row_ix = self.diff_source_mapped_ix_for_visible_ix(source_visible_ix)?;
        // `line_to_row` maps are indexed by 0-based line; the row models carry
        // 1-based diff line numbers.
        let zero_based = |line: Option<u32>| usize::try_from(line?).ok()?.checked_sub(1);

        match self.diff_view {
            DiffViewMode::Inline => {
                if region != DiffTextRegion::Inline {
                    return None;
                }
                use gitcomet_core::domain::DiffLineKind;
                let row = self.file_diff_inline_render_data(row_ix)?;
                let (side, line) = match row.kind {
                    DiffLineKind::Remove => (DiffTextPairSide::Old, row.old_line),
                    DiffLineKind::Add | DiffLineKind::Context => {
                        (DiffTextPairSide::New, row.new_line)
                    }
                    // Headers and hunk markers are not document text.
                    // Headers and hunk markers are not document text.
                    DiffLineKind::Header | DiffLineKind::Hunk => return None,
                };
                let document = self.file_diff_pair_syntax_document(match side {
                    DiffTextPairSide::Old => DiffTextRegion::SplitLeft,
                    _ => DiffTextRegion::SplitRight,
                })?;
                Some((document, zero_based(line)?, side))
            }
            DiffViewMode::Split => {
                let row = self.file_diff_split_render_data(row_ix)?;
                let (side, line) = match region {
                    DiffTextRegion::SplitLeft => (DiffTextPairSide::Old, row.old_line),
                    DiffTextRegion::SplitRight => (DiffTextPairSide::New, row.new_line),
                    DiffTextRegion::Inline => return None,
                };
                let document = self.file_diff_pair_syntax_document(region)?;
                Some((document, zero_based(line)?, side))
            }
        }
    }

    /// The inverse: a document line back onto the row rendering it, if any.
    ///
    /// `None` means that end is simply not on screen as a row -- outside the
    /// diff, or folded into a collapsed hunk. Its partner is still painted.
    fn diff_text_pair_row_for_document_line(
        &self,
        side: DiffTextPairSide,
        line_ix: usize,
    ) -> Option<(usize, DiffTextRegion)> {
        if side == DiffTextPairSide::Preview {
            return Some((line_ix, DiffTextRegion::Inline));
        }
        let inline = self.diff_view == DiffViewMode::Inline;
        let map: &[Option<usize>] = match (side, inline) {
            (DiffTextPairSide::Old, true) => self.file_diff_old_line_to_inline_row.as_ref(),
            (DiffTextPairSide::Old, false) => self.file_diff_old_line_to_row.as_ref(),
            (_, true) => self.file_diff_new_line_to_inline_row.as_ref(),
            (_, false) => self.file_diff_new_line_to_row.as_ref(),
        };
        let row_ix = (*map.get(line_ix)?)?;
        let source_visible_ix = self.diff_text_pair_source_visible_ix_for_row(row_ix)?;
        let region = match (inline, side) {
            (true, _) => DiffTextRegion::Inline,
            (false, DiffTextPairSide::Old) => DiffTextRegion::SplitLeft,
            (false, _) => DiffTextRegion::SplitRight,
        };
        Some((source_visible_ix, region))
    }

    /// The inverse of [`Self::diff_source_mapped_ix_for_visible_ix`].
    ///
    /// Only the collapsed arm is new: that projection keeps no reverse index, so
    /// it scans. Everything else delegates to the existing inverse rather than
    /// growing a second one, and the scan runs twice per click, never per frame.
    fn diff_text_pair_source_visible_ix_for_row(&self, row_ix: usize) -> Option<usize> {
        if self.is_collapsed_diff_projection_active() {
            return self
                .collapsed_diff_visible_rows
                .iter()
                .position(|row| row.row_ix() == Some(row_ix));
        }
        self.diff_source_visible_ix_for_mapped_ix(row_ix)
    }

    /// The matching delimiter pair for a click, projected onto rows.
    fn diff_text_pair_match_for_pos(&self, pos: &DiffTextPos) -> Option<DiffTextPairMatch> {
        // A row the display truncated is not the tab-expansion of its line, so
        // offsets into it do not convert. Better no answer than a confident one
        // pointing at the wrong character.
        if self.diff_text_row_is_display_truncated(pos.source_visible_ix, pos.region) {
            return None;
        }
        let (document, line_ix, side) =
            self.diff_text_pair_document_for_row(pos.source_visible_ix, pos.region)?;
        let hit = rows::prepared_diff_syntax_pair_at_display_offset(document, line_ix, pos.offset)?;

        let spans: Vec<DiffTextPairSpan> = hit
            .open
            .into_iter()
            .chain(hit.close)
            .filter_map(|end| {
                let (source_visible_ix, region) =
                    self.diff_text_pair_row_for_document_line(side, end.line_ix)?;
                Some(DiffTextPairSpan {
                    source_visible_ix,
                    region,
                    range: end.display_range,
                })
            })
            .collect();
        // Both ends off-screen is not a pair worth remembering.
        (!spans.is_empty()).then_some(DiffTextPairMatch {
            kind: hit.kind,
            spans,
        })
    }

    /// The parts of the pair that fall on one row, in that row's local offsets.
    ///
    /// Mirrors [`Self::diff_text_local_selection_range`] so both quads are
    /// clipped to the same wrap slice in the same coordinate space.
    pub(in super::super::super) fn diff_text_local_pair_ranges(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> smallvec::SmallVec<[Range<usize>; 2]> {
        let Some(pair) = self.diff_text_pair_match.as_ref() else {
            return smallvec::SmallVec::new();
        };
        let (source_visible_ix, visual_range) =
            self.diff_text_visual_source_range_for_region(visible_ix, region);
        pair.ranges_on_row(source_visible_ix, region)
            .into_iter()
            .filter_map(|range| {
                diff_text_local_range_from_source_ranges(range, visual_range.clone())
            })
            .collect()
    }

    /// The part of one row a selection covers, or `None` when the selection
    /// does not reach that row at all.
    ///
    /// An empty range is a real answer, not a miss: a blank line inside a
    /// selection covers no characters and is still one of its lines, and so is
    /// a line the selection only touches the edge of. Callers that paint the
    /// highlight ignore an empty range; the one that copies has to keep it, or
    /// every blank line falls out of the text.
    fn diff_text_source_selection_range(
        &self,
        source_visible_ix: usize,
        region: DiffTextRegion,
        text_len: usize,
    ) -> Option<Range<usize>> {
        let (start, end) = self.diff_text_normalized_selection()?;
        if start == end {
            return None;
        }
        if source_visible_ix < start.source_visible_ix || source_visible_ix > end.source_visible_ix
        {
            return None;
        }

        let split_region = (self.diff_view == DiffViewMode::Split
            && start.region == end.region
            && matches!(
                start.region,
                DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
            ))
        .then_some(start.region);
        if split_region.is_some_and(|r| r != region) {
            return None;
        }

        let region_order = region.order();
        let start_order = start.region.order();
        let end_order = end.region.order();

        let mut a = 0usize;
        let mut b = text_len;

        if start.source_visible_ix == end.source_visible_ix
            && source_visible_ix == start.source_visible_ix
        {
            if region_order < start_order || region_order > end_order {
                return None;
            }
            if region == start.region {
                a = start.offset.min(text_len);
            }
            if region == end.region {
                b = end.offset.min(text_len);
            }
        } else if source_visible_ix == start.source_visible_ix {
            if region_order < start_order {
                return None;
            }
            if region == start.region {
                a = start.offset.min(text_len);
            }
        } else if source_visible_ix == end.source_visible_ix {
            if region_order > end_order {
                return None;
            }
            if region == end.region {
                b = end.offset.min(text_len);
            }
        }

        Some(a..b.max(a))
    }

    fn diff_text_wrap_range_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<Range<usize>> {
        let wrap = self.diff_text_wrap_for_visible_ix(visible_ix)?;
        Some(wrap.range_for_region(region))
    }

    fn diff_text_apply_wrap_to_line(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        text: SharedString,
    ) -> SharedString {
        let Some(range) = self.diff_text_wrap_range_for_region(visible_ix, region) else {
            return text;
        };
        if range.start >= range.end {
            return SharedString::default();
        }
        text.as_ref()
            .get(range)
            .map(|slice| SharedString::from(slice.to_owned()))
            .unwrap_or_default()
    }

    pub(in super::super::super) fn diff_text_line_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> SharedString {
        let source_visible_ix = self
            .diff_source_visible_ix_for_visible_ix(visible_ix)
            .unwrap_or(visible_ix);
        let text = self.diff_text_full_line_for_region(source_visible_ix, region);
        self.diff_text_apply_wrap_to_line(visible_ix, region, text)
    }

    pub(in crate::view) fn diff_text_full_line_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> SharedString {
        let fallback = SharedString::default();
        let expand_tabs = |s: &str| -> SharedString {
            if !s.contains('\t') {
                return SharedString::new(s);
            }
            let mut out = String::with_capacity(crate::view::diff_utils::diff_text_display_len(s));
            for ch in s.chars() {
                match ch {
                    '\t' => out.push_str("    "),
                    _ => out.push(ch),
                }
            }
            out.into()
        };

        // When markdown rendered preview is active, rows come from the
        // markdown preview document rather than from source text lines or
        // patch diff rows.
        if self.is_markdown_preview_active() {
            return self.markdown_preview_row_text(visible_ix, region);
        }

        if self.is_file_preview_active() {
            if region != DiffTextRegion::Inline {
                return fallback;
            }
            return self
                .worktree_preview_line_raw_text(visible_ix)
                .map(|line| file_diff_display_text(&line))
                .unwrap_or(fallback);
        }

        if self.is_collapsed_diff_projection_active() {
            let Some(row) = self.collapsed_visible_row(visible_ix) else {
                return fallback;
            };
            match row {
                CollapsedDiffVisibleRow::HunkHeader { .. } => {
                    if self.diff_view == DiffViewMode::Inline && region != DiffTextRegion::Inline {
                        return fallback;
                    }
                    if self.diff_view == DiffViewMode::Split
                        && !matches!(
                            region,
                            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
                        )
                    {
                        return fallback;
                    }
                    return row
                        .header_display_src_ix()
                        .and_then(|src_ix| self.collapsed_diff_hunk_header_display(src_ix))
                        .unwrap_or(fallback);
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } => match self.diff_view {
                    DiffViewMode::Inline => {
                        if region != DiffTextRegion::Inline {
                            return fallback;
                        }
                        let Some(row) = self.file_diff_inline_render_data(row_ix) else {
                            return fallback;
                        };
                        let cache_epoch = self.file_diff_style_cache_epochs.inline_epoch(row.kind);
                        if let Some(styled) = self.diff_text_segments_cache_get(row_ix, cache_epoch)
                        {
                            return styled.text.clone();
                        }
                        return file_diff_display_text(&row.text);
                    }
                    DiffViewMode::Split => {
                        if !matches!(
                            region,
                            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
                        ) {
                            return fallback;
                        }
                        let cache_epoch = self.file_diff_split_style_cache_epoch(region);
                        if let Some(key) = self.file_diff_split_cache_key(row_ix, region)
                            && let Some(styled) =
                                self.diff_text_segments_cache_get(key, cache_epoch)
                        {
                            return styled.text.clone();
                        }
                        let Some(row) = self.file_diff_split_render_data(row_ix) else {
                            return fallback;
                        };
                        let text = match region {
                            DiffTextRegion::SplitLeft => row.old.as_ref(),
                            DiffTextRegion::SplitRight => row.new.as_ref(),
                            DiffTextRegion::Inline => unreachable!(),
                        };
                        return text.map(file_diff_display_text).unwrap_or(fallback);
                    }
                },
            }
        }

        let Some(mapped_ix) = self.diff_source_mapped_ix_for_visible_ix(visible_ix) else {
            return fallback;
        };

        if self.diff_view == DiffViewMode::Inline {
            if region != DiffTextRegion::Inline {
                return fallback;
            }
            if self.is_file_diff_view_active() {
                if let Some(row) = self.file_diff_inline_render_data(mapped_ix) {
                    let cache_epoch = self.file_diff_style_cache_epochs.inline_epoch(row.kind);
                    if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, cache_epoch)
                    {
                        return styled.text.clone();
                    }
                    return file_diff_display_text(&row.text);
                } else if let Some(line) = self.file_diff_inline_row(mapped_ix) {
                    let cache_epoch = self.file_diff_inline_style_cache_epoch(&line);
                    if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, cache_epoch)
                    {
                        return styled.text.clone();
                    }
                    return expand_tabs(diff_content_text(&line));
                }
                return fallback;
            }

            if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, 0) {
                return styled.text.clone();
            }
            let Some(line) = self.patch_diff_row(mapped_ix) else {
                return fallback;
            };
            let click_kind = self
                .diff_click_kinds
                .get(mapped_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if matches!(
                click_kind,
                DiffClickKind::HunkHeader | DiffClickKind::FileHeader
            ) && let Some(display) = self.diff_header_display_cache.get(&mapped_ix)
            {
                return display.clone();
            }
            return expand_tabs(line.text.as_ref());
        }

        match region {
            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => {}
            DiffTextRegion::Inline => return fallback,
        }

        if self.is_file_diff_view_active() {
            let cache_epoch = self.file_diff_split_style_cache_epoch(region);
            if let Some(key) = self.file_diff_split_cache_key(mapped_ix, region)
                && let Some(styled) = self.diff_text_segments_cache_get(key, cache_epoch)
            {
                return styled.text.clone();
            }
            let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                return fallback;
            };
            let text = match region {
                DiffTextRegion::SplitLeft => row.old.as_ref(),
                DiffTextRegion::SplitRight => row.new.as_ref(),
                DiffTextRegion::Inline => unreachable!(),
            };
            return text.map(file_diff_display_text).unwrap_or(fallback);
        }

        let Some(split_row) = self.patch_diff_split_row(mapped_ix) else {
            return fallback;
        };
        match split_row {
            PatchSplitRow::Raw { src_ix, click_kind } => {
                let Some(line) = self.patch_diff_row(src_ix) else {
                    return fallback;
                };
                if matches!(
                    click_kind,
                    DiffClickKind::HunkHeader | DiffClickKind::FileHeader
                ) && let Some(display) = self.diff_header_display_cache.get(&src_ix)
                {
                    return display.clone();
                }
                expand_tabs(line.text.as_ref())
            }
            PatchSplitRow::Aligned { row, .. } => {
                let text = match region {
                    DiffTextRegion::SplitLeft => row.old.as_deref().unwrap_or(""),
                    DiffTextRegion::SplitRight => row.new.as_deref().unwrap_or(""),
                    DiffTextRegion::Inline => unreachable!(),
                };
                expand_tabs(text)
            }
        }
    }

    pub(in super::super::super) fn diff_text_line_len_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> usize {
        let display_len = crate::view::diff_utils::diff_text_display_len;

        if self.diff_text_wrap_for_visible_ix(visible_ix).is_some() {
            return self.diff_text_line_for_region(visible_ix, region).len();
        }

        // Markdown preview rows already come from pre-rendered preview text, so
        // fall back to the existing materialized path there.
        if self.is_markdown_preview_active() {
            return self.markdown_preview_row_text_len(visible_ix, region);
        }

        if self.is_file_preview_active() {
            if region != DiffTextRegion::Inline {
                return 0;
            }
            return self
                .worktree_preview_line_raw_text(visible_ix)
                .map(|line| file_diff_display_len(&line))
                .unwrap_or(0);
        }

        if self.is_collapsed_diff_projection_active() {
            let Some(row) = self.collapsed_visible_row(visible_ix) else {
                return 0;
            };
            match row {
                CollapsedDiffVisibleRow::HunkHeader { .. } => {
                    if self.diff_view == DiffViewMode::Inline && region != DiffTextRegion::Inline {
                        return 0;
                    }
                    if self.diff_view == DiffViewMode::Split
                        && !matches!(
                            region,
                            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
                        )
                    {
                        return 0;
                    }
                    return row
                        .header_display_src_ix()
                        .and_then(|src_ix| {
                            self.collapsed_diff_hunk_header_display(src_ix)
                                .map(|display| display_len(display.as_ref()))
                        })
                        .unwrap_or(0);
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } => match self.diff_view {
                    DiffViewMode::Inline => {
                        if region != DiffTextRegion::Inline {
                            return 0;
                        }
                        let Some(row) = self.file_diff_inline_render_data(row_ix) else {
                            return 0;
                        };
                        let cache_epoch = self.file_diff_style_cache_epochs.inline_epoch(row.kind);
                        if let Some(styled) = self.diff_text_segments_cache_get(row_ix, cache_epoch)
                        {
                            return styled.text.len();
                        }
                        return file_diff_display_len(&row.text);
                    }
                    DiffViewMode::Split => {
                        if !matches!(
                            region,
                            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
                        ) {
                            return 0;
                        }
                        let cache_epoch = self.file_diff_split_style_cache_epoch(region);
                        if let Some(key) = self.file_diff_split_cache_key(row_ix, region)
                            && let Some(styled) =
                                self.diff_text_segments_cache_get(key, cache_epoch)
                        {
                            return styled.text.len();
                        }
                        let Some(row) = self.file_diff_split_render_data(row_ix) else {
                            return 0;
                        };
                        let text = match region {
                            DiffTextRegion::SplitLeft => row.old.as_ref(),
                            DiffTextRegion::SplitRight => row.new.as_ref(),
                            DiffTextRegion::Inline => unreachable!(),
                        };
                        return text.map(file_diff_display_len).unwrap_or(0);
                    }
                },
            }
        }

        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            return 0;
        };

        if self.diff_view == DiffViewMode::Inline {
            if region != DiffTextRegion::Inline {
                return 0;
            }
            if self.is_file_diff_view_active() {
                if let Some(row) = self.file_diff_inline_render_data(mapped_ix) {
                    let cache_epoch = self.file_diff_style_cache_epochs.inline_epoch(row.kind);
                    if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, cache_epoch)
                    {
                        return styled.text.len();
                    }
                    return file_diff_display_len(&row.text);
                } else if let Some(line) = self.file_diff_inline_row(mapped_ix) {
                    let cache_epoch = self.file_diff_inline_style_cache_epoch(&line);
                    if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, cache_epoch)
                    {
                        return styled.text.len();
                    }
                    return display_len(diff_content_text(&line));
                }
                return 0;
            }

            if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, 0) {
                return styled.text.len();
            }
            let Some(line) = self.patch_diff_row(mapped_ix) else {
                return 0;
            };
            let click_kind = self
                .diff_click_kinds
                .get(mapped_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if matches!(
                click_kind,
                DiffClickKind::HunkHeader | DiffClickKind::FileHeader
            ) && let Some(display) = self.diff_header_display_cache.get(&mapped_ix)
            {
                return display.len();
            }
            return display_len(line.text.as_ref());
        }

        match region {
            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => {}
            DiffTextRegion::Inline => return 0,
        }

        if self.is_file_diff_view_active() {
            let cache_epoch = self.file_diff_split_style_cache_epoch(region);
            if let Some(key) = self.file_diff_split_cache_key(mapped_ix, region)
                && let Some(styled) = self.diff_text_segments_cache_get(key, cache_epoch)
            {
                return styled.text.len();
            }
            let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                return 0;
            };
            let text = match region {
                DiffTextRegion::SplitLeft => row.old.as_ref(),
                DiffTextRegion::SplitRight => row.new.as_ref(),
                DiffTextRegion::Inline => unreachable!(),
            };
            return text.map(file_diff_display_len).unwrap_or(0);
        }

        let Some(split_row) = self.patch_diff_split_row(mapped_ix) else {
            return 0;
        };
        match split_row {
            PatchSplitRow::Raw { src_ix, click_kind } => {
                let Some(line) = self.patch_diff_row(src_ix) else {
                    return 0;
                };
                if matches!(
                    click_kind,
                    DiffClickKind::HunkHeader | DiffClickKind::FileHeader
                ) && let Some(display) = self.diff_header_display_cache.get(&src_ix)
                {
                    return display.len();
                }
                display_len(line.text.as_ref())
            }
            PatchSplitRow::Aligned { row, .. } => {
                let text = match region {
                    DiffTextRegion::SplitLeft => row.old.as_deref().unwrap_or(""),
                    DiffTextRegion::SplitRight => row.new.as_deref().unwrap_or(""),
                    DiffTextRegion::Inline => unreachable!(),
                };
                display_len(text)
            }
        }
    }

    fn diff_text_combined_offset(&self, pos: DiffTextPos, left_len: usize) -> usize {
        match self.diff_view {
            DiffViewMode::Inline => pos.offset,
            DiffViewMode::Split => match pos.region {
                DiffTextRegion::SplitLeft => pos.offset,
                DiffTextRegion::SplitRight => left_len.saturating_add(1).saturating_add(pos.offset),
                DiffTextRegion::Inline => pos.offset,
            },
        }
    }

    fn diff_text_source_combined_selection_range(
        &self,
        source_visible_ix: usize,
        left_len: usize,
        right_len: usize,
    ) -> Option<Range<usize>> {
        let (start, end) = self.diff_text_normalized_selection()?;
        if start == end
            || source_visible_ix < start.source_visible_ix
            || source_visible_ix > end.source_visible_ix
        {
            return None;
        }

        let combined_len = left_len.saturating_add(1).saturating_add(right_len);
        let mut a = 0usize;
        let mut b = combined_len;

        if start.source_visible_ix == end.source_visible_ix
            && source_visible_ix == start.source_visible_ix
        {
            a = self
                .diff_text_combined_offset(start, left_len)
                .min(combined_len);
            b = self
                .diff_text_combined_offset(end, left_len)
                .min(combined_len);
        } else if source_visible_ix == start.source_visible_ix {
            a = self
                .diff_text_combined_offset(start, left_len)
                .min(combined_len);
        } else if source_visible_ix == end.source_visible_ix {
            b = self
                .diff_text_combined_offset(end, left_len)
                .min(combined_len);
        }

        (a < b).then_some(a..b)
    }

    fn append_diff_text_region_slice(
        &self,
        out: &mut String,
        visible_ix: usize,
        region: DiffTextRegion,
        range: Range<usize>,
        expanded_tabs: &mut String,
    ) {
        if range.start >= range.end {
            return;
        }

        if self.diff_text_wrap_for_visible_ix(visible_ix).is_some() {
            let text = self.diff_text_line_for_region(visible_ix, region);
            append_diff_display_text_slice(out, text.as_ref(), range, expanded_tabs);
            return;
        }

        self.append_diff_text_source_region_slice(out, visible_ix, region, range, expanded_tabs);
    }

    fn append_diff_text_source_region_slice(
        &self,
        out: &mut String,
        source_visible_ix: usize,
        region: DiffTextRegion,
        range: Range<usize>,
        expanded_tabs: &mut String,
    ) {
        if range.start >= range.end {
            return;
        }

        if self.is_markdown_preview_active() {
            let text = self.markdown_preview_row_text(source_visible_ix, region);
            append_diff_display_text_slice(out, text.as_ref(), range, expanded_tabs);
            return;
        }

        if self.is_file_preview_active() {
            if region != DiffTextRegion::Inline {
                return;
            }
            if let Some(raw_text) = self.worktree_preview_line_raw_text(source_visible_ix) {
                append_file_diff_display_text_slice(out, &raw_text, range, expanded_tabs);
            }
            return;
        }

        if self.is_collapsed_diff_projection_active() {
            if let Some(row) = self.collapsed_visible_row(source_visible_ix) {
                match (row, self.diff_view, region) {
                    (
                        CollapsedDiffVisibleRow::FileRow { row_ix },
                        DiffViewMode::Inline,
                        DiffTextRegion::Inline,
                    ) => {
                        if let Some(row) = self.file_diff_inline_render_data(row_ix) {
                            append_file_diff_display_text_slice(
                                out,
                                &row.text,
                                range,
                                expanded_tabs,
                            );
                            return;
                        }
                    }
                    (
                        CollapsedDiffVisibleRow::FileRow { row_ix },
                        DiffViewMode::Split,
                        DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight,
                    ) => {
                        let raw_text =
                            self.file_diff_split_render_data(row_ix)
                                .and_then(|row| match region {
                                    DiffTextRegion::SplitLeft => row.old,
                                    DiffTextRegion::SplitRight => row.new,
                                    DiffTextRegion::Inline => None,
                                });
                        if let Some(raw_text) = raw_text {
                            append_file_diff_display_text_slice(
                                out,
                                &raw_text,
                                range,
                                expanded_tabs,
                            );
                            return;
                        }
                    }
                    _ => {}
                }
            }
            let text = self.diff_text_full_line_for_region(source_visible_ix, region);
            append_diff_display_text_slice(out, text.as_ref(), range, expanded_tabs);
            return;
        }

        let Some(mapped_ix) = self.diff_source_mapped_ix_for_visible_ix(source_visible_ix) else {
            return;
        };

        if self.diff_view == DiffViewMode::Inline && self.is_file_diff_view_active() {
            if region != DiffTextRegion::Inline {
                return;
            }
            if let Some(row) = self.file_diff_inline_render_data(mapped_ix) {
                append_file_diff_display_text_slice(out, &row.text, range, expanded_tabs);
            }
            return;
        }

        if self.diff_view == DiffViewMode::Split && self.is_file_diff_view_active() {
            let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                return;
            };
            let text = match region {
                DiffTextRegion::SplitLeft => row.old.as_ref(),
                DiffTextRegion::SplitRight => row.new.as_ref(),
                DiffTextRegion::Inline => return,
            };
            if let Some(text) = text {
                append_file_diff_display_text_slice(out, text, range, expanded_tabs);
            }
            return;
        }

        let text = self.diff_text_full_line_for_region(source_visible_ix, region);
        append_diff_display_text_slice(out, text.as_ref(), range, expanded_tabs);
    }

    fn diff_text_string_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<String> {
        let line_len = self.diff_text_line_len_for_region(visible_ix, region);
        if line_len == 0 {
            return None;
        }

        let mut out = String::with_capacity(line_len);
        let mut expanded_tabs = String::new();
        self.append_diff_text_region_slice(
            &mut out,
            visible_ix,
            region,
            0..line_len,
            &mut expanded_tabs,
        );
        (!out.is_empty()).then_some(out)
    }

    fn selected_diff_text_string(&self) -> Option<String> {
        let (start, end) = self.diff_text_normalized_selection()?;
        if start == end {
            return None;
        }

        let force_inline = self.is_file_preview_active();
        let selected_line_count = end
            .source_visible_ix
            .saturating_sub(start.source_visible_ix)
            .saturating_add(1);

        let mut out = String::with_capacity(
            crate::view::diff_utils::multiline_text_copy_capacity_hint(selected_line_count),
        );
        let mut expanded_tabs = String::new();
        // Separators are counted rather than inferred from `out` being empty:
        // the first row of a selection can be a blank line, and it still opens
        // the text with a line of its own.
        let mut rows_written = 0usize;
        // A picture is one line of the document however many rows it was given,
        // and every one of them carries its description. The row the selection
        // starts on always contributes, so a selection that begins inside a
        // picture still describes it once.
        let repeats_a_picture = |this: &Self, source_visible_ix: usize, region| {
            source_visible_ix != start.source_visible_ix
                && this.markdown_preview_row_repeats_a_picture(source_visible_ix, region)
        };
        for source_visible_ix in start.source_visible_ix..=end.source_visible_ix {
            if force_inline || self.diff_view == DiffViewMode::Inline {
                if repeats_a_picture(self, source_visible_ix, DiffTextRegion::Inline) {
                    continue;
                }
                let line_len = self
                    .diff_text_full_line_len_for_region(source_visible_ix, DiffTextRegion::Inline);
                let Some(range) = self.diff_text_source_selection_range(
                    source_visible_ix,
                    DiffTextRegion::Inline,
                    line_len,
                ) else {
                    continue;
                };
                if rows_written > 0 {
                    out.push('\n');
                }
                rows_written += 1;
                self.append_diff_text_source_region_slice(
                    &mut out,
                    source_visible_ix,
                    DiffTextRegion::Inline,
                    range,
                    &mut expanded_tabs,
                );
                continue;
            }

            let split_region = (start.region == end.region
                && matches!(
                    start.region,
                    DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
                ))
            .then_some(start.region);

            if let Some(region) = split_region {
                if repeats_a_picture(self, source_visible_ix, region) {
                    continue;
                }
                let line_len = self.diff_text_full_line_len_for_region(source_visible_ix, region);
                let Some(range) =
                    self.diff_text_source_selection_range(source_visible_ix, region, line_len)
                else {
                    continue;
                };
                if rows_written > 0 {
                    out.push('\n');
                }
                rows_written += 1;
                self.append_diff_text_source_region_slice(
                    &mut out,
                    source_visible_ix,
                    region,
                    range,
                    &mut expanded_tabs,
                );
            } else {
                let left_full_len = self.diff_text_full_line_len_for_region(
                    source_visible_ix,
                    DiffTextRegion::SplitLeft,
                );
                let right_full_len = self.diff_text_full_line_len_for_region(
                    source_visible_ix,
                    DiffTextRegion::SplitRight,
                );
                let combined_source_range = self.diff_text_source_combined_selection_range(
                    source_visible_ix,
                    left_full_len,
                    right_full_len,
                );
                let left_range = self.diff_text_source_selection_range(
                    source_visible_ix,
                    DiffTextRegion::SplitLeft,
                    left_full_len,
                );
                let right_range = self.diff_text_source_selection_range(
                    source_visible_ix,
                    DiffTextRegion::SplitRight,
                    right_full_len,
                );
                let include_tab = combined_source_range.as_ref().is_some_and(|range| {
                    range.start < left_full_len.saturating_add(1) && range.end > left_full_len
                });
                if left_range.is_none() && right_range.is_none() && !include_tab {
                    continue;
                }

                if rows_written > 0 {
                    out.push('\n');
                }
                rows_written += 1;
                if let Some(range) = left_range {
                    self.append_diff_text_source_region_slice(
                        &mut out,
                        source_visible_ix,
                        DiffTextRegion::SplitLeft,
                        range,
                        &mut expanded_tabs,
                    );
                }
                if include_tab {
                    out.push('\t');
                }
                if let Some(range) = right_range {
                    self.append_diff_text_source_region_slice(
                        &mut out,
                        source_visible_ix,
                        DiffTextRegion::SplitRight,
                        range,
                        &mut expanded_tabs,
                    );
                }
            }
        }

        // A selection of nothing but blank lines is still a selection, so this
        // asks whether any row was written rather than whether text came out.
        if rows_written == 0 { None } else { Some(out) }
    }

    pub(in super::super::super) fn copy_selected_diff_text_to_clipboard(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(text) = self.selected_diff_text_string() else {
            return;
        };
        crate::clipboard::write_text(cx, text, self.diff_copy_source());
    }

    pub(in super::super::super) fn copy_diff_text_for_context_menu_to_clipboard(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(text) = self
            .selected_diff_text_string()
            .or_else(|| self.diff_text_string_for_region(visible_ix, region))
        else {
            return;
        };
        crate::clipboard::write_text(cx, text, crate::clipboard::CopySource::DiffContextMenu);
    }

    fn diff_copy_source(&self) -> crate::clipboard::CopySource {
        match self
            .active_repo()
            .and_then(|repo| repo.diff_state.diff_target.as_ref())
        {
            Some(DiffTarget::Commit { .. }) => crate::clipboard::CopySource::CommitDetailsDiff,
            Some(DiffTarget::CommitRange { .. }) => crate::clipboard::CopySource::CommitRangeDiff,
            Some(DiffTarget::WorkingTree {
                area: DiffArea::Staged,
                ..
            }) => crate::clipboard::CopySource::StagedDiff,
            Some(DiffTarget::WorkingTree { .. }) | None => {
                crate::clipboard::CopySource::UnstagedDiff
            }
        }
    }

    pub(in super::super::super) fn open_diff_editor_context_menu(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.is_inline_submodule_diff_active() {
            return;
        }
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let workdir = repo.spec.workdir.clone();

        let (area, allow_apply) = match repo.diff_state.diff_target.as_ref() {
            Some(DiffTarget::WorkingTree { area, .. }) => (*area, true),
            _ => (DiffArea::Unstaged, false),
        };
        let is_file_preview = self.is_file_preview_active();

        let selected_copy_text = self.selected_diff_text_string();
        let copy_target = if selected_copy_text.is_none()
            && self.is_file_preview_active()
            && region == DiffTextRegion::Inline
            && self
                .worktree_preview_line_raw_text(visible_ix)
                .is_some_and(|line| rows::is_streamable_diff_text(&line))
        {
            Some((visible_ix, region))
        } else {
            None
        };
        let copy_text = if copy_target.is_some() {
            selected_copy_text
        } else {
            selected_copy_text.or_else(|| self.diff_text_string_for_region(visible_ix, region))
        };

        let list_len = if is_file_preview {
            self.worktree_preview_line_count().unwrap_or(0)
        } else {
            self.diff_visible_len()
        };
        let clicked_visible_ix = if list_len == 0 {
            visible_ix
        } else {
            visible_ix.min(list_len - 1)
        };

        let clicked_source_visible_ix = self
            .diff_source_visible_ix_for_visible_ix(clicked_visible_ix)
            .unwrap_or(clicked_visible_ix);
        let text_selection = context_menu_selection_range_from_diff_text(
            self.diff_text_normalized_selection(),
            if is_file_preview {
                DiffViewMode::Inline
            } else {
                self.diff_view
            },
            clicked_source_visible_ix,
            region,
        )
        .and_then(|(a, b)| self.diff_text_visible_range_for_source_range(a, b));

        if list_len > 0 && text_selection.is_none() {
            let existing = self
                .diff_selection_range
                .map(|(a, b)| (a.min(b), a.max(b)))
                .filter(|(a, b)| clicked_visible_ix >= *a && clicked_visible_ix <= *b);
            if existing.is_none() {
                self.diff_selection_anchor = Some(clicked_visible_ix);
                self.diff_selection_range = Some((clicked_visible_ix, clicked_visible_ix));
            }
        }

        struct FileDiffSrcLookup {
            file_rel: std::path::PathBuf,
            add_by_new_line: FxHashMap<u32, usize>,
            remove_by_old_line: FxHashMap<u32, usize>,
            context_by_old_line: FxHashMap<u32, usize>,
        }

        let file_diff_lookup = if self.is_file_diff_view_active() {
            self.file_diff_cache_path.as_ref().map(|abs| {
                let rel = abs.strip_prefix(&workdir).unwrap_or(abs);
                let file_rel = rel.to_path_buf();
                // Git diffs use forward slashes even on Windows.
                let rel_str = file_rel.to_str().map(|text| text.replace('\\', "/"));

                let approx_map_len = match self.diff_view {
                    DiffViewMode::Inline => self.file_diff_inline_row_len(),
                    DiffViewMode::Split => self.file_diff_split_row_len(),
                };
                let mut add_by_new_line: FxHashMap<u32, usize> =
                    FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());
                let mut remove_by_old_line: FxHashMap<u32, usize> =
                    FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());
                let mut context_by_old_line: FxHashMap<u32, usize> =
                    FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());

                for ix in 0..self.patch_diff_row_len() {
                    let Some(line) = self.patch_diff_row(ix) else {
                        continue;
                    };
                    if self.diff_file_for_src_ix.get(ix).and_then(|p| p.as_deref())
                        != rel_str.as_deref()
                    {
                        continue;
                    }
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => {
                            if let Some(n) = line.new_line {
                                add_by_new_line.insert(n, ix);
                            }
                        }
                        gitcomet_core::domain::DiffLineKind::Remove => {
                            if let Some(o) = line.old_line {
                                remove_by_old_line.insert(o, ix);
                            }
                        }
                        gitcomet_core::domain::DiffLineKind::Context => {
                            if let Some(o) = line.old_line {
                                context_by_old_line.insert(o, ix);
                            }
                        }
                        gitcomet_core::domain::DiffLineKind::Header
                        | gitcomet_core::domain::DiffLineKind::Hunk => {}
                    }
                }

                FileDiffSrcLookup {
                    file_rel,
                    add_by_new_line,
                    remove_by_old_line,
                    context_by_old_line,
                }
            })
        } else {
            None
        };

        let src_ixs_for_visible_ix = |visible_ix: usize| -> Vec<usize> {
            if let Some(lookup) = file_diff_lookup.as_ref() {
                let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return Vec::new();
                };
                match self.diff_view {
                    DiffViewMode::Inline => {
                        let Some(line) = self.file_diff_inline_render_data(mapped_ix) else {
                            return Vec::new();
                        };
                        match line.kind {
                            gitcomet_core::domain::DiffLineKind::Add => line
                                .new_line
                                .and_then(|n| lookup.add_by_new_line.get(&n).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::domain::DiffLineKind::Remove => line
                                .old_line
                                .and_then(|o| lookup.remove_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::domain::DiffLineKind::Context => line
                                .old_line
                                .and_then(|o| lookup.context_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::domain::DiffLineKind::Header
                            | gitcomet_core::domain::DiffLineKind::Hunk => Vec::new(),
                        }
                    }
                    DiffViewMode::Split => {
                        let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                            return Vec::new();
                        };
                        match row.kind {
                            gitcomet_core::file_diff::FileDiffRowKind::Context => row
                                .old_line
                                .and_then(|o| lookup.context_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::file_diff::FileDiffRowKind::Add => row
                                .new_line
                                .and_then(|n| lookup.add_by_new_line.get(&n).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::file_diff::FileDiffRowKind::Remove => row
                                .old_line
                                .and_then(|o| lookup.remove_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::file_diff::FileDiffRowKind::Modify => {
                                let mut out = Vec::with_capacity(2);
                                if let Some(o) = row.old_line
                                    && let Some(ix) = lookup.remove_by_old_line.get(&o).copied()
                                {
                                    out.push(ix);
                                }
                                if let Some(n) = row.new_line
                                    && let Some(ix) = lookup.add_by_new_line.get(&n).copied()
                                    && !out.contains(&ix)
                                {
                                    out.push(ix);
                                }
                                out
                            }
                        }
                    }
                }
            } else {
                self.diff_src_ixs_for_visible_ix(visible_ix)
            }
        };

        let clicked_src_ix = src_ixs_for_visible_ix(clicked_visible_ix)
            .into_iter()
            .next();
        let hunk_src_ix = clicked_src_ix.and_then(|src_ix| self.diff_enclosing_hunk_src_ix(src_ix));

        let path = hunk_src_ix
            .or(clicked_src_ix)
            .and_then(|ix| self.diff_file_for_src_ix.get(ix))
            .and_then(|p| p.as_deref())
            .map(std::path::PathBuf::from);
        let path = path
            .or_else(|| file_diff_lookup.as_ref().map(|l| l.file_rel.clone()))
            .or_else(|| {
                self.worktree_preview_path.as_ref().map(|abs| {
                    let rel = abs.strip_prefix(&workdir).unwrap_or(abs);
                    rel.to_path_buf()
                })
            });

        let allow_patch_actions = allow_apply && !is_file_preview;

        let selection = text_selection
            .or_else(|| self.diff_selection_range.map(|(a, b)| (a.min(b), a.max(b))))
            .or_else(|| (list_len > 0).then_some((clicked_visible_ix, clicked_visible_ix)))
            .map(|(a, b)| {
                if list_len == 0 {
                    (0, 0)
                } else {
                    (a.min(list_len - 1), b.min(list_len - 1))
                }
            });

        let (hunks_count, hunk_patch, lines_count, lines_patch, discard_lines_patch) =
            if allow_patch_actions && let Some((sel_a, sel_b)) = selection {
                let approx_selected = sel_b
                    .saturating_sub(sel_a)
                    .saturating_add(1)
                    .saturating_mul(2);
                let mut selected_src_ixs: FxHashSet<usize> =
                    FxHashSet::with_capacity_and_hasher(approx_selected, Default::default());
                let mut selected_change_src_ixs: FxHashSet<usize> =
                    FxHashSet::with_capacity_and_hasher(approx_selected, Default::default());

                for vix in sel_a..=sel_b {
                    for src_ix in src_ixs_for_visible_ix(vix) {
                        let Some(line) = self.patch_diff_row(src_ix) else {
                            continue;
                        };
                        selected_src_ixs.insert(src_ix);
                        if matches!(
                            line.kind,
                            gitcomet_core::domain::DiffLineKind::Add
                                | gitcomet_core::domain::DiffLineKind::Remove
                        ) {
                            selected_change_src_ixs.insert(src_ix);
                        }
                    }
                }

                let mut selected_hunks: Vec<usize> = selected_src_ixs
                    .into_iter()
                    .filter_map(|ix| self.diff_enclosing_hunk_src_ix(ix))
                    .collect();
                selected_hunks.sort_unstable();
                selected_hunks.dedup();

                let materialized_diff = self.patch_diff_rows_slice(0, self.patch_diff_row_len());
                let hunk_patch = build_unified_patch_for_hunks(&materialized_diff, &selected_hunks);
                let hunks_count = hunk_patch
                    .as_ref()
                    .map(|_| selected_hunks.len())
                    .unwrap_or(0);

                // "Stage line(s)" applies forward to the index; "Unstage
                // line(s)" applies the same selection in reverse, which needs
                // the opposite treatment of the unselected changes around it or
                // git rejects the patch.
                let lines_patch = match area {
                    DiffArea::Unstaged => build_unified_patch_for_selected_lines_across_hunks(
                        &materialized_diff,
                        &selected_change_src_ixs,
                    ),
                    DiffArea::Staged => {
                        build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                            &materialized_diff,
                            &selected_change_src_ixs,
                        )
                    }
                };
                let discard_lines_patch = if area == DiffArea::Unstaged {
                    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                        &materialized_diff,
                        &selected_change_src_ixs,
                    )
                } else {
                    None
                };
                let lines_count = lines_patch
                    .as_ref()
                    .map(|_| selected_change_src_ixs.len())
                    .unwrap_or(0);

                (
                    hunks_count,
                    hunk_patch,
                    lines_count,
                    lines_patch,
                    discard_lines_patch,
                )
            } else {
                (0, None, 0, None, None)
            };

        self.activate_context_menu_invoker("diff_editor_menu".into(), cx);
        self.open_popover_at(
            PopoverKind::DiffEditorMenu {
                repo_id,
                area,
                path,
                hunk_patch,
                hunks_count,
                lines_patch,
                discard_lines_patch,
                lines_count,
                copy_text,
                copy_target,
            },
            anchor,
            window,
            cx,
        );
    }
}

impl MainPaneView {
    fn tick_diff_text_selection_autoscroll(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        if let Ok(pos) = self.root_view.update(cx, |root, _cx| root.last_mouse_pos) {
            self.diff_text_last_mouse_pos = pos;
        }

        let Some(target) = self.diff_text_autoscroll_target else {
            // Still update selection periodically so it can expand while the user scrolls.
            let before = self.diff_text_head;
            self.update_diff_text_selection_from_mouse(self.diff_text_last_mouse_pos);
            return self.diff_text_head != before;
        };

        let handle = self.scroll_handle_for_diff_text_autoscroll_target(target);
        let bounds = handle.bounds();
        if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
            return false;
        }

        let max_offset = handle.max_offset();
        let old_offset = handle.offset();
        let mouse = self.diff_text_last_mouse_pos;

        let delta_x = autoscroll_delta_for_axis(mouse.x, bounds.left(), bounds.right());
        let delta_y = autoscroll_delta_for_axis(mouse.y, bounds.top(), bounds.bottom());

        let new_x = (old_offset.x + delta_x).clamp(-max_offset.x, px(0.0));
        let new_y = (old_offset.y + delta_y).clamp(-max_offset.y, px(0.0));

        let scrolled = new_x != old_offset.x || new_y != old_offset.y;
        if scrolled {
            handle.set_offset(point(new_x, new_y));
        }

        let before_head = self.diff_text_head;
        self.update_diff_text_selection_from_mouse(mouse);
        let selection_changed = self.diff_text_head != before_head;

        scrolled || selection_changed
    }

    fn diff_text_autoscroll_target_for_position(
        &self,
        position: Point<Pixels>,
    ) -> DiffTextAutoscrollTarget {
        if self.is_file_preview_active() {
            return DiffTextAutoscrollTarget::WorktreePreview;
        }

        if self.is_conflict_resolver_active() {
            return DiffTextAutoscrollTarget::ConflictResolvedPreview;
        }

        if self.diff_view == DiffViewMode::Split {
            let right_bounds = self.diff_split_right_scroll.0.borrow().base_handle.bounds();
            if right_bounds.contains(&position) {
                return DiffTextAutoscrollTarget::DiffSplitRight;
            }
        }

        DiffTextAutoscrollTarget::DiffLeftOrInline
    }

    fn scroll_handle_for_diff_text_autoscroll_target(
        &self,
        target: DiffTextAutoscrollTarget,
    ) -> ScrollHandle {
        match target {
            DiffTextAutoscrollTarget::DiffLeftOrInline => {
                self.diff_scroll.0.borrow().base_handle.clone()
            }
            DiffTextAutoscrollTarget::DiffSplitRight => {
                self.diff_split_right_scroll.0.borrow().base_handle.clone()
            }
            DiffTextAutoscrollTarget::WorktreePreview => {
                self.worktree_preview_scroll.0.borrow().base_handle.clone()
            }
            DiffTextAutoscrollTarget::ConflictResolvedPreview => self
                .conflict_resolved_preview_scroll
                .0
                .borrow()
                .base_handle
                .clone(),
        }
    }
}

fn autoscroll_delta_for_axis(cursor: Pixels, min: Pixels, max: Pixels) -> Pixels {
    fn speed(distance: Pixels) -> Pixels {
        // 2–48px per tick, scaling with how far outside the container the cursor is.
        let min_step = px(2.0);
        let max_step = px(48.0);
        (distance * 0.4).max(min_step).min(max_step)
    }

    if cursor < min {
        speed(min - cursor)
    } else if cursor > max {
        -speed(cursor - max)
    } else {
        px(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::diff_text_local_range_from_source_ranges;

    #[test]
    fn local_selection_range_returns_none_when_selection_ends_before_visual_slice() {
        assert_eq!(diff_text_local_range_from_source_ranges(4..8, 12..20), None);
    }

    #[test]
    fn local_selection_range_clips_to_visual_slice() {
        assert_eq!(
            diff_text_local_range_from_source_ranges(8..16, 12..20),
            Some(0..4)
        );
        assert_eq!(
            diff_text_local_range_from_source_ranges(14..24, 12..20),
            Some(2..8)
        );
    }
}
