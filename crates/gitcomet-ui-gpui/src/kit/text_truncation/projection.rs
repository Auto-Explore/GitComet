use super::TruncatedLineLayout;
use gpui::{Pixels, ShapedLine};
use smallvec::SmallVec;
use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TruncationProjection {
    pub(super) source_len: usize,
    pub(super) display_len: usize,
    pub(super) segments: SmallVec<[ProjectionSegment; 4]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionSegment {
    Source {
        source_range: Range<usize>,
        display_range: Range<usize>,
    },
    Ellipsis {
        hidden_range: Range<usize>,
        display_range: Range<usize>,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Affinity {
    Start,
    #[cfg(test)]
    End,
}
fn extend_offset_bounds(bounds: &mut Option<(usize, usize)>, start: usize, end: usize) {
    match bounds {
        Some((min, max)) => {
            *min = (*min).min(start);
            *max = (*max).max(end);
        }
        None => *bounds = Some((start, end)),
    }
}
impl TruncationProjection {
    pub(crate) fn source_to_display_offset(&self, offset: usize) -> usize {
        self.source_to_display_offset_with_affinity(offset, Affinity::Start)
    }

    pub(crate) fn source_to_display_offset_with_affinity(
        &self,
        offset: usize,
        affinity: Affinity,
    ) -> usize {
        let offset = offset.min(self.source_len);
        let mut bounds = None;
        for segment in &self.segments {
            match segment {
                ProjectionSegment::Source {
                    source_range,
                    display_range,
                } if offset >= source_range.start && offset <= source_range.end => {
                    let display_offset =
                        display_range.start + offset.saturating_sub(source_range.start);
                    extend_offset_bounds(&mut bounds, display_offset, display_offset);
                }
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } => {
                    if offset == hidden_range.start {
                        extend_offset_bounds(&mut bounds, display_range.start, display_range.start);
                    } else if offset == hidden_range.end {
                        extend_offset_bounds(&mut bounds, display_range.end, display_range.end);
                    } else if offset > hidden_range.start && offset < hidden_range.end {
                        extend_offset_bounds(&mut bounds, display_range.start, display_range.end);
                    }
                }
                _ => {}
            }
        }

        match (affinity, bounds) {
            (Affinity::Start, Some((min, _))) => min,
            #[cfg(test)]
            (Affinity::End, Some((_, max))) => max,
            (_, None) => self.display_len,
        }
    }

    pub(crate) fn display_to_source_offset(
        &self,
        display_offset: usize,
        affinity: Affinity,
    ) -> usize {
        let display_offset = display_offset.min(self.display_len);
        let mut bounds = None;
        for segment in &self.segments {
            match segment {
                ProjectionSegment::Source {
                    source_range,
                    display_range,
                } if display_offset >= display_range.start
                    && display_offset <= display_range.end =>
                {
                    let source_offset =
                        source_range.start + display_offset.saturating_sub(display_range.start);
                    extend_offset_bounds(&mut bounds, source_offset, source_offset);
                }
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } if display_offset >= display_range.start
                    && display_offset <= display_range.end =>
                {
                    extend_offset_bounds(&mut bounds, hidden_range.start, hidden_range.end);
                }
                _ => {}
            }
        }

        match (affinity, bounds) {
            (Affinity::Start, Some((min, _))) => min,
            #[cfg(test)]
            (Affinity::End, Some((_, max))) => max,
            (_, None) => self.source_len,
        }
    }

    pub(crate) fn display_to_source_start_offset(&self, display_offset: usize) -> usize {
        self.display_to_source_offset(display_offset, Affinity::Start)
    }

    pub(crate) fn selection_display_ranges(
        &self,
        selection: Range<usize>,
    ) -> SmallVec<[Range<usize>; 4]> {
        let mut ranges = SmallVec::new();
        if selection.is_empty() {
            return ranges;
        }

        for segment in &self.segments {
            match segment {
                ProjectionSegment::Source {
                    source_range,
                    display_range,
                } => {
                    let start = selection.start.max(source_range.start);
                    let end = selection.end.min(source_range.end);
                    if start >= end {
                        continue;
                    }
                    ranges.push(
                        display_range.start + start.saturating_sub(source_range.start)
                            ..display_range.start + end.saturating_sub(source_range.start),
                    );
                }
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } => {
                    if selection.start < hidden_range.end && selection.end > hidden_range.start {
                        ranges.push(display_range.clone());
                    }
                }
            }
        }
        ranges
    }

    pub(crate) fn ellipsis_segment_at_display_offset(
        &self,
        display_offset: usize,
    ) -> Option<(Range<usize>, Range<usize>)> {
        self.segments.iter().find_map(|segment| match segment {
            ProjectionSegment::Ellipsis {
                hidden_range,
                display_range,
            } if display_offset >= display_range.start && display_offset <= display_range.end => {
                Some((hidden_range.clone(), display_range.clone()))
            }
            _ => None,
        })
    }

    pub(crate) fn ellipsis_segment_for_source_offset(
        &self,
        source_offset: usize,
    ) -> Option<(Range<usize>, Range<usize>)> {
        self.segments.iter().find_map(|segment| match segment {
            ProjectionSegment::Ellipsis {
                hidden_range,
                display_range,
            } if source_offset >= hidden_range.start && source_offset <= hidden_range.end => {
                Some((hidden_range.clone(), display_range.clone()))
            }
            _ => None,
        })
    }
}

fn first_ellipsis_display_start(projection: &TruncationProjection) -> Option<usize> {
    projection
        .segments
        .iter()
        .find_map(|segment| match segment {
            ProjectionSegment::Ellipsis { display_range, .. } => Some(display_range.start),
            _ => None,
        })
}

pub(super) fn ellipsis_x_for_projection_and_line(
    projection: &TruncationProjection,
    shaped_line: &ShapedLine,
) -> Option<Pixels> {
    Some(shaped_line.x_for_index(first_ellipsis_display_start(projection)?))
}

pub(crate) fn truncated_line_ellipsis_x(line: &TruncatedLineLayout) -> Option<Pixels> {
    ellipsis_x_for_projection_and_line(line.projection.as_ref(), &line.shaped_line)
}
