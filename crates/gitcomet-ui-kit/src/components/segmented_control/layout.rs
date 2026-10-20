use gpui::{Pixels, px};

#[derive(Clone, Copy)]
pub(super) struct SegmentMetrics {
    pub minimum: Pixels,
    pub label: Pixels,
    pub collapsible: bool,
}

pub(super) struct SegmentLayout {
    pub width: Pixels,
    pub label: bool,
}

pub(super) fn allocate(
    metrics: &[SegmentMetrics],
    available: Pixels,
    gap: Pixels,
    minimum_label: Pixels,
) -> Vec<SegmentLayout> {
    let minimum: Pixels = metrics.iter().map(|segment| segment.minimum).sum();
    let remaining = (available - minimum).max(px(0.0));
    let mut labelled: Vec<usize> = metrics
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| segment.collapsible.then_some(index))
        .collect();
    while labelled
        .iter()
        .map(|index| gap + metrics[*index].label.min(minimum_label))
        .sum::<Pixels>()
        > remaining
    {
        labelled.pop();
    }
    let mut sorted: Vec<Pixels> = labelled.iter().map(|index| metrics[*index].label).collect();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let mut budget = remaining - gap * labelled.len() as f32;
    let mut cap = px(0.0);
    for (index, width) in sorted.iter().enumerate() {
        cap = budget / (sorted.len() - index) as f32;
        if cap <= *width {
            break;
        }
        budget -= *width;
    }
    metrics
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let show_label = !segment.collapsible || labelled.contains(&index);
            SegmentLayout {
                width: segment.minimum
                    + if segment.collapsible && show_label {
                        gap + segment.label.min(cap)
                    } else {
                        px(0.0)
                    },
                label: show_label,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> [SegmentMetrics; 3] {
        [
            SegmentMetrics {
                minimum: px(100.0),
                label: px(60.0),
                collapsible: false,
            },
            SegmentMetrics {
                minimum: px(32.0),
                label: px(40.0),
                collapsible: true,
            },
            SegmentMetrics {
                minimum: px(32.0),
                label: px(70.0),
                collapsible: true,
            },
        ]
    }

    #[test]
    fn active_label_stays_full_while_inactive_labels_shorten_then_disappear() {
        let metrics = metrics();
        let full = allocate(&metrics, px(284.0), px(5.0), px(24.0));
        assert_eq!(
            full.iter().map(|s| s.width).collect::<Vec<_>>(),
            vec![px(100.0), px(77.0), px(107.0)]
        );
        let clipped = allocate(&metrics, px(234.0), px(5.0), px(24.0));
        assert_eq!(
            clipped.iter().map(|s| s.width).collect::<Vec<_>>(),
            vec![px(100.0), px(67.0), px(67.0)]
        );
        let one_label = allocate(&metrics, px(193.0), px(5.0), px(24.0));
        assert!(one_label[1].label);
        assert!(!one_label[2].label);
        assert_eq!(one_label[0].width, px(100.0));
        let icons = allocate(&metrics, px(164.0), px(5.0), px(24.0));
        assert_eq!(
            icons.iter().map(|s| s.label).collect::<Vec<_>>(),
            vec![true, false, false]
        );
        assert_eq!(icons.iter().map(|s| s.width).sum::<Pixels>(), px(164.0));
    }

    #[test]
    fn short_labels_keep_their_natural_width_and_empty_controls_are_valid() {
        let metrics = [
            SegmentMetrics {
                minimum: px(32.0),
                label: px(10.0),
                collapsible: true,
            },
            SegmentMetrics {
                minimum: px(32.0),
                label: px(70.0),
                collapsible: true,
            },
        ];
        let layout = allocate(&metrics, px(114.0), px(5.0), px(24.0));
        assert_eq!(layout[0].width, px(47.0));
        assert_eq!(layout[1].width, px(67.0));
        assert!(allocate(&[], px(0.0), px(5.0), px(24.0)).is_empty());
    }
}
