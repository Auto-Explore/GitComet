use super::*;
use gitcomet_ui_kit::text_layout::TextLayoutExt as _;

#[derive(Clone, Copy)]
pub(in crate::view) struct DiffTextHit {
    pub(in crate::view) pos: DiffTextPos,
    pub(in crate::view) past_painted_text: bool,
}

pub(in crate::view) struct TextGeometry<'a> {
    pub(in crate::view) hitboxes: &'a FxHashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    pub(in crate::view) motion_targets: &'a [DiffTextMotionTarget],
    pub(in crate::view) layout_cache: Option<&'a FxHashMap<u64, DiffTextLayoutCacheEntry>>,
    pub(in crate::view) anchor: Option<DiffTextPos>,
    pub(in crate::view) selecting: bool,
}

impl TextGeometry<'_> {
    pub(in crate::view) fn diff_text_pos_from_hitbox(
        &self,
        row: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        let hitbox = self.hitboxes.get(&(row, region))?;
        if !hitbox.bounds.contains(&position) {
            return None;
        }
        self.diff_text_hit_in_hitbox(hitbox, region, position)
            .map(|hit| hit.pos)
    }

    pub(in crate::view) fn diff_text_pos_from_nearest_hitbox(
        &self,
        row: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        self.diff_text_hit_in_hitbox(self.hitboxes.get(&(row, region))?, region, position)
            .map(|hit| hit.pos)
    }

    pub(in crate::view) fn diff_text_cell_for_position(
        hitbox: &DiffTextHitbox,
        position: Point<Pixels>,
    ) -> Option<&DiffTextHitbox> {
        let distance = |bounds: &Bounds<Pixels>| {
            let dy = (bounds.top() - position.y)
                .max(position.y - bounds.bottom())
                .max(px(0.0));
            let dx = (bounds.left() - position.x)
                .max(position.x - bounds.right())
                .max(px(0.0));
            (dy, dx)
        };
        hitbox.cells.iter().min_by(|a, b| {
            distance(&a.bounds)
                .partial_cmp(&distance(&b.bounds))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    pub(in crate::view) fn diff_text_pos_from_motion_target(
        &self,
        target: &DiffTextMotionTarget,
        position: Point<Pixels>,
    ) -> DiffTextPos {
        if position.y < target.bounds.top() {
            return target.start;
        }
        if position.y > target.bounds.bottom() {
            return target.end;
        }

        if let Some(anchor) = self.anchor {
            if anchor.cmp_key() >= target.end.cmp_key() {
                return target.start;
            }
            if anchor.cmp_key() <= target.start.cmp_key() {
                return target.end;
            }
        }

        if position.y <= target.bounds.center().y {
            target.start
        } else {
            target.end
        }
    }

    pub(in crate::view) fn diff_text_hit_in_hitbox(
        &self,
        hitbox: &DiffTextHitbox,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextHit> {
        if !hitbox.cells.is_empty() {
            let cell = Self::diff_text_cell_for_position(hitbox, position)?;
            return self.diff_text_hit_in_hitbox(cell, region, position);
        }
        if hitbox.atomic {
            let after = position.y > hitbox.bounds.bottom()
                || (position.y >= hitbox.bounds.top() && position.x >= hitbox.bounds.center().x);
            return Some(DiffTextHit {
                pos: DiffTextPos {
                    source_visible_ix: hitbox.source_visible_ix,
                    region,
                    offset: hitbox.text_start_offset + if after { hitbox.text_len } else { 0 },
                },
                past_painted_text: true,
            });
        }
        if let Some(wrapped) = &hitbox.wrapped {
            let (painted_offset, past_painted_text) = if position.y < hitbox.bounds.top() {
                (0, true)
            } else if position.y > hitbox.bounds.bottom() {
                (wrapped.layout.len(), true)
            } else {
                match wrapped.layout.index_for_position(position) {
                    Ok(offset) => (offset, false),
                    Err(offset) => (offset, true),
                }
            };
            return Some(DiffTextHit {
                pos: DiffTextPos {
                    source_visible_ix: hitbox.source_visible_ix,
                    region,
                    offset: hitbox
                        .text_start_offset
                        .saturating_add(wrapped.row_offset(painted_offset).min(hitbox.text_len)),
                },
                past_painted_text,
            });
        }
        let (local_offset, past_painted_text) = if position.y < hitbox.bounds.top() {
            (0, true)
        } else if position.y > hitbox.bounds.bottom() {
            (hitbox.text_len, true)
        } else {
            let x = (position.x - hitbox.bounds.left()).max(px(0.0));
            if let Some(cell_width) = hitbox.streamed_ascii_monospace_cell_width {
                if cell_width <= px(0.0) {
                    (0, x > px(0.0))
                } else {
                    let text_width = cell_width * hitbox.text_len as f32;
                    (
                        (((x / cell_width) + 0.5).floor() as usize).min(hitbox.text_len),
                        x > text_width,
                    )
                }
            } else {
                let layout = &self.layout_cache?.get(&hitbox.layout_key)?.layout;
                (
                    layout
                        .closest_index_for_x(x)
                        .min(layout.len())
                        .min(hitbox.text_len),
                    x > layout.width,
                )
            }
        };
        let local_offset = hitbox
            .offset_map
            .as_ref()
            .map(|map| map.source_offset_for_display(local_offset))
            .unwrap_or(local_offset);
        Some(DiffTextHit {
            pos: DiffTextPos {
                source_visible_ix: hitbox.source_visible_ix,
                region,
                offset: hitbox.text_start_offset.saturating_add(local_offset),
            },
            past_painted_text,
        })
    }

    pub(in crate::view) fn diff_text_bounds_in_hitbox(
        &self,
        hitbox: &DiffTextHitbox,
        range: Range<usize>,
        near: Point<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        if !hitbox.cells.is_empty() {
            let cell = hitbox.cells.iter().find(|cell| {
                (cell.text_start_offset..=cell.text_start_offset + cell.text_len)
                    .contains(&range.start)
            })?;
            return self.diff_text_bounds_in_hitbox(cell, range, near);
        }
        if hitbox.atomic {
            return Some(hitbox.bounds);
        }
        let local = |offset: usize| {
            offset
                .saturating_sub(hitbox.text_start_offset)
                .min(hitbox.text_len)
        };
        let (start, end) = (local(range.start), local(range.end));

        if let Some(wrapped) = &hitbox.wrapped {
            let rects = rows::markdown_flow_range_rects(
                &wrapped.layout,
                wrapped.painted_offset(start),
                wrapped.painted_offset(end),
            );
            let distance = |rect: &Bounds<Pixels>| {
                f32::from((rect.top() - near.y).max(near.y - rect.bottom()))
            };
            return rects
                .into_iter()
                .min_by(|a, b| distance(a).total_cmp(&distance(b)));
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
            let layout = &self.layout_cache?.get(&hitbox.layout_key)?.layout;
            Some(layout.x_for_index(display_offset.min(layout.len())))
        };
        let left = hitbox.bounds.left() + x_for(start)?;
        let right = hitbox.bounds.left() + x_for(end)?;
        Some(Bounds::from_corners(
            point(left, hitbox.bounds.top()),
            point(right.max(left), hitbox.bounds.bottom()),
        ))
    }

    pub(in crate::view) fn diff_text_pos_for_mouse(
        &self,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        let restrict_region = self
            .selecting
            .then_some(self.anchor)
            .flatten()
            .map(|p| p.region)
            .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight));

        for ((visible_ix, region), hitbox) in self.hitboxes {
            if restrict_region.is_some_and(|restrict| restrict != *region) {
                continue;
            }
            if hitbox.bounds.contains(&position) {
                return self.diff_text_pos_from_hitbox(*visible_ix, *region, position);
            }
        }
        for target in self.motion_targets {
            if restrict_region.is_some_and(|restrict| restrict != target.start.region) {
                continue;
            }
            if target.bounds.contains(&position) {
                return Some(self.diff_text_pos_from_motion_target(target, position));
            }
        }

        let mut best: Option<((usize, DiffTextRegion), (Pixels, Pixels))> = None;
        for (key, hitbox) in self.hitboxes {
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

        let mut best_motion: Option<(usize, (Pixels, Pixels, usize, u8))> = None;
        for (target_ix, target) in self.motion_targets.iter().enumerate() {
            if restrict_region.is_some_and(|restrict| restrict != target.start.region) {
                continue;
            }
            let dy = if position.y < target.bounds.top() {
                target.bounds.top() - position.y
            } else if position.y > target.bounds.bottom() {
                position.y - target.bounds.bottom()
            } else {
                px(0.0)
            };
            let dx = if position.x < target.bounds.left() {
                target.bounds.left() - position.x
            } else if position.x > target.bounds.right() {
                position.x - target.bounds.right()
            } else {
                px(0.0)
            };
            let rank = (
                dy,
                dx,
                target.start.source_visible_ix,
                target.start.region.order(),
            );
            if best_motion.is_none_or(|(_, best_rank)| rank < best_rank) {
                best_motion = Some((target_ix, rank));
            }
        }

        match (best, best_motion) {
            (None, None) => None,
            (Some(((visible_ix, region), _)), None) => {
                self.diff_text_pos_from_nearest_hitbox(visible_ix, region, position)
            }
            (None, Some((target_ix, _))) => Some(
                self.diff_text_pos_from_motion_target(&self.motion_targets[target_ix], position),
            ),
            (Some(((visible_ix, region), (dy, dx))), Some((target_ix, motion_rank))) => {
                let text_rank = (dy, dx, visible_ix, region.order());
                if text_rank <= motion_rank {
                    self.diff_text_pos_from_nearest_hitbox(visible_ix, region, position)
                } else {
                    Some(self.diff_text_pos_from_motion_target(
                        &self.motion_targets[target_ix],
                        position,
                    ))
                }
            }
        }
    }
}

pub(in crate::view) fn normalized_selection(
    anchor: Option<DiffTextPos>,
    head: Option<DiffTextPos>,
) -> Option<(DiffTextPos, DiffTextPos)> {
    let (anchor, head) = (anchor?, head?);
    Some(if anchor.cmp_key() <= head.cmp_key() {
        (anchor, head)
    } else {
        (head, anchor)
    })
}

pub(in crate::view) fn source_selection_range(
    anchor: Option<DiffTextPos>,
    head: Option<DiffTextPos>,
    split: bool,
    source_visible_ix: usize,
    region: DiffTextRegion,
    text_len: usize,
) -> Option<Range<usize>> {
    let (start, end) = normalized_selection(anchor, head)?;
    if start == end {
        return None;
    }
    if source_visible_ix < start.source_visible_ix || source_visible_ix > end.source_visible_ix {
        return None;
    }

    let split_region = (split
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

pub(in crate::view) fn add_cell_hitbox(
    hitboxes: &mut FxHashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    visible_ix: usize,
    region: DiffTextRegion,
    row_len: usize,
    cell: DiffTextHitbox,
) {
    let row = hitboxes
        .entry((visible_ix, region))
        .or_insert_with(|| DiffTextHitbox {
            atomic: false,
            bounds: cell.bounds,
            layout_key: 0,
            source_visible_ix: cell.source_visible_ix,
            text_start_offset: 0,
            text_len: row_len,
            offset_map: None,
            painted_text: SharedString::default(),
            streamed_ascii_monospace_cell_width: None,
            wrapped: None,
            cells: Vec::new(),
        });
    row.bounds = row.bounds.union(&cell.bounds);
    row.cells.push(cell);
}
