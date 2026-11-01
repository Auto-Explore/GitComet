use std::collections::BTreeMap;

use crate::config::LayoutConfig;
use crate::ir::{DiagramKind, Direction, Graph};

use super::super::label_placement;
use super::super::routing::{
    EdgePortInfo, Obstacle, anchor_point_for_node, compress_path, edge_label_anchor_from_points,
    insert_label_via_point, is_horizontal, path_length, path_point_at_progress,
    segment_intersects_rect,
};
use super::super::{NodeLayout, SubgraphLayout, TextBlock, anchor_layout_for_edge_towards};

#[derive(Clone)]
pub(super) struct RouteLabelPlan {
    pub(super) obstacle_id: String,
    pub(super) obstacle_index: usize,
    pub(super) progress: f32,
    pub(super) center: (f32, f32),
}

pub(super) struct RouteLabelSyncContext<'a> {
    pub(super) direction: Direction,
    pub(super) kind: DiagramKind,
    pub(super) route_label_plans: &'a mut [Option<RouteLabelPlan>],
    pub(super) label_anchors: &'a mut [Option<(f32, f32)>],
    pub(super) edge_route_labels: &'a [Option<TextBlock>],
    pub(super) route_label_obstacles: &'a mut [Obstacle],
    pub(super) edge_label_pad_x: f32,
    pub(super) edge_label_pad_y: f32,
    pub(super) update_obstacle: bool,
}

struct ProvisionalRouteLabelCenterContext<'a> {
    graph: &'a Graph,
    nodes: &'a BTreeMap<String, NodeLayout>,
    subgraphs: &'a [SubgraphLayout],
    edge_ports: &'a [EdgePortInfo],
    pair_index: &'a [usize],
    lane_offsets: &'a [f32],
    edge_label_pad_x: f32,
    edge_label_pad_y: f32,
    config: &'a LayoutConfig,
}

pub(super) fn should_route_labels_via(graph: &Graph, nodes: &BTreeMap<String, NodeLayout>) -> bool {
    let has_label_dummies = nodes
        .keys()
        .any(|id| id.starts_with("__elabel_") && id.ends_with("__"));
    !has_label_dummies && graph.kind != DiagramKind::Er
}

pub(super) fn route_label_centers(plans: &[Option<RouteLabelPlan>]) -> Vec<Option<(f32, f32)>> {
    plans
        .iter()
        .map(|plan| plan.as_ref().map(|plan| plan.center))
        .collect()
}

fn flowchart_label_needs_reserved_route_gap(label: &TextBlock, config: &LayoutConfig) -> bool {
    if label.lines.len() > 1 {
        return true;
    }
    let char_count: usize = label.lines.iter().map(|line| line.chars().count()).sum();
    label.width >= config.node_spacing * 0.9 || char_count >= 14
}

pub(super) fn initialize_route_label_plans(
    graph: &Graph,
    nodes: &BTreeMap<String, NodeLayout>,
    subgraphs: &[SubgraphLayout],
    edge_ports: &[EdgePortInfo],
    pair_index: &[usize],
    lane_offsets: &[f32],
    edge_route_labels: &[Option<TextBlock>],
    label_obstacles: Vec<Obstacle>,
    config: &LayoutConfig,
) -> (Vec<Option<RouteLabelPlan>>, Vec<Obstacle>) {
    let mut route_label_plans: Vec<Option<RouteLabelPlan>> = vec![None; graph.edges.len()];
    if !should_route_labels_via(graph, nodes) {
        return (route_label_plans, label_obstacles);
    }

    let (edge_label_pad_x, edge_label_pad_y) =
        label_placement::edge_label_padding(graph.kind, config);
    let mut route_label_obstacles = label_obstacles;

    for idx in 0..graph.edges.len() {
        let Some(label) = edge_route_labels.get(idx).and_then(|label| label.as_ref()) else {
            continue;
        };
        if label.width <= 0.0 || label.height <= 0.0 {
            continue;
        }
        if graph.kind == DiagramKind::Flowchart
            && !flowchart_label_needs_reserved_route_gap(label, config)
        {
            continue;
        }

        let Some(center) = provisional_route_label_center(
            idx,
            label,
            &ProvisionalRouteLabelCenterContext {
                graph,
                nodes,
                subgraphs,
                edge_ports,
                pair_index,
                lane_offsets,
                edge_label_pad_x,
                edge_label_pad_y,
                config,
            },
        ) else {
            continue;
        };
        let obstacle_id = format!("edge-label-reserved:{idx}");
        let obstacle_index = route_label_obstacles.len();
        route_label_obstacles.push(Obstacle {
            id: obstacle_id.clone(),
            x: center.0 - label.width / 2.0 - edge_label_pad_x,
            y: center.1 - label.height / 2.0 - edge_label_pad_y,
            width: label.width + 2.0 * edge_label_pad_x,
            height: label.height + 2.0 * edge_label_pad_y,
            members: None,
        });
        route_label_plans[idx] = Some(RouteLabelPlan {
            obstacle_id,
            obstacle_index,
            progress: 0.5,
            center,
        });
    }

    (route_label_plans, route_label_obstacles)
}

fn provisional_route_label_center(
    idx: usize,
    label: &TextBlock,
    ctx: &ProvisionalRouteLabelCenterContext<'_>,
) -> Option<(f32, f32)> {
    let graph = ctx.graph;
    let edge = &graph.edges[idx];
    let (Some(from_layout), Some(to_layout)) = (ctx.nodes.get(&edge.from), ctx.nodes.get(&edge.to))
    else {
        return None;
    };
    let temp_from = from_layout.anchor_subgraph.and_then(|anchor_idx| {
        ctx.subgraphs.get(anchor_idx).map(|sub| {
            anchor_layout_for_edge_towards(from_layout, sub, to_layout, graph.direction, true)
        })
    });
    let temp_to = to_layout.anchor_subgraph.and_then(|anchor_idx| {
        ctx.subgraphs.get(anchor_idx).map(|sub| {
            anchor_layout_for_edge_towards(to_layout, sub, from_layout, graph.direction, false)
        })
    });
    let from = temp_from.as_ref().unwrap_or(from_layout);
    let to = temp_to.as_ref().unwrap_or(to_layout);
    let port_info = ctx.edge_ports.get(idx).copied().unwrap_or(EdgePortInfo {
        start_side: super::super::routing::EdgeSide::Right,
        end_side: super::super::routing::EdgeSide::Left,
        start_offset: 0.0,
        end_offset: 0.0,
    });
    let start = anchor_point_for_node(from, port_info.start_side, port_info.start_offset);
    let end = anchor_point_for_node(to, port_info.end_side, port_info.end_offset);

    let base_offset = ctx.lane_offsets.get(idx).copied().unwrap_or_default();

    let mut center = ((start.0 + end.0) * 0.5, (start.1 + end.1) * 0.5);
    if is_horizontal(graph.direction) {
        center.0 += base_offset;
    } else {
        center.1 += base_offset;
    }
    if graph.kind == DiagramKind::Flowchart {
        let main_span = if is_horizontal(graph.direction) {
            (end.0 - start.0).abs()
        } else {
            (end.1 - start.1).abs()
        };
        let label_main = if is_horizontal(graph.direction) {
            label.width + 2.0 * ctx.edge_label_pad_x
        } else {
            label.height + 2.0 * ctx.edge_label_pad_y
        };
        let label_cross = if is_horizontal(graph.direction) {
            label.height + 2.0 * ctx.edge_label_pad_y
        } else {
            label.width + 2.0 * ctx.edge_label_pad_x
        };
        let margin = (ctx.config.node_spacing * 0.35).max(14.0);
        let preferred_sign = if is_horizontal(graph.direction) {
            let dy = end.1 - start.1;
            if dy.abs() > 2.0 {
                dy.signum()
            } else if ctx.pair_index.get(idx).copied().unwrap_or_default() % 2 == 0 {
                -1.0
            } else {
                1.0
            }
        } else {
            let dx = end.0 - start.0;
            if dx.abs() > 2.0 {
                dx.signum()
            } else if ctx.pair_index.get(idx).copied().unwrap_or_default() % 2 == 0 {
                -1.0
            } else {
                1.0
            }
        };
        let needs_cross_axis_lift = label_main + margin * 2.0 >= main_span.max(1.0);
        if needs_cross_axis_lift {
            let multiline_lift = if label.lines.len() > 1 {
                label_cross * 0.75
            } else {
                0.0
            };
            let clearance = label_cross * 0.5 + margin + multiline_lift;
            for sign in [preferred_sign, -preferred_sign] {
                let mut candidate = center;
                if is_horizontal(graph.direction) {
                    candidate.1 += sign * clearance;
                } else {
                    candidate.0 += sign * clearance;
                }
                let half_w = label.width * 0.5 + ctx.edge_label_pad_x;
                let half_h = label.height * 0.5 + ctx.edge_label_pad_y;
                let overlaps_start = start.0 >= candidate.0 - half_w
                    && start.0 <= candidate.0 + half_w
                    && start.1 >= candidate.1 - half_h
                    && start.1 <= candidate.1 + half_h;
                let overlaps_end = end.0 >= candidate.0 - half_w
                    && end.0 <= candidate.0 + half_w
                    && end.1 >= candidate.1 - half_h
                    && end.1 <= candidate.1 + half_h;
                if !overlaps_start && !overlaps_end {
                    center = candidate;
                    break;
                }
            }
        }
    }
    Some(center)
}

pub(super) fn sync_route_label_plan_with_points(
    idx: usize,
    points: &mut Vec<(f32, f32)>,
    ctx: &mut RouteLabelSyncContext<'_>,
) {
    let Some(plan) = ctx
        .route_label_plans
        .get_mut(idx)
        .and_then(|plan| plan.as_mut())
    else {
        return;
    };
    if points.len() < 2 {
        return;
    }

    let preserve_reserved_center = ctx.kind == DiagramKind::Flowchart;
    let label_center = if preserve_reserved_center {
        plan.center
    } else {
        path_point_at_progress(points, plan.progress)
            .or_else(|| edge_label_anchor_from_points(points))
            .unwrap_or(plan.center)
    };
    plan.center = label_center;
    ctx.label_anchors[idx] = Some(label_center);
    if preserve_reserved_center
        && let Some(label) = ctx
            .edge_route_labels
            .get(idx)
            .and_then(|label| label.as_ref())
    {
        let label_obstacle = Obstacle {
            id: plan.obstacle_id.clone(),
            x: label_center.0 - label.width / 2.0 - ctx.edge_label_pad_x,
            y: label_center.1 - label.height / 2.0 - ctx.edge_label_pad_y,
            width: label.width + 2.0 * ctx.edge_label_pad_x,
            height: label.height + 2.0 * ctx.edge_label_pad_y,
            members: None,
        };
        if let Some(detoured) = detour_flowchart_path_around_label(
            points,
            &label_obstacle,
            ctx.direction,
            (ctx.edge_label_pad_y.max(ctx.edge_label_pad_x) * 0.5).max(8.0),
        ) {
            *points = detoured;
        }
    }

    if !preserve_reserved_center && !matches!(ctx.kind, DiagramKind::Flowchart | DiagramKind::State)
    {
        insert_label_via_point(points, label_center, ctx.direction);
    }

    if !ctx.update_obstacle {
        return;
    }
    if let Some(label) = ctx
        .edge_route_labels
        .get(idx)
        .and_then(|label| label.as_ref())
        && let Some(obstacle) = ctx.route_label_obstacles.get_mut(plan.obstacle_index)
    {
        obstacle.x = label_center.0 - label.width / 2.0 - ctx.edge_label_pad_x;
        obstacle.y = label_center.1 - label.height / 2.0 - ctx.edge_label_pad_y;
        obstacle.width = label.width + 2.0 * ctx.edge_label_pad_x;
        obstacle.height = label.height + 2.0 * ctx.edge_label_pad_y;
    }
}

fn path_intersects_label_obstacle(points: &[(f32, f32)], obstacle: &Obstacle) -> bool {
    points
        .windows(2)
        .any(|segment| segment_intersects_rect(segment[0], segment[1], obstacle))
}

fn detour_flowchart_path_around_label(
    points: &[(f32, f32)],
    obstacle: &Obstacle,
    _direction: Direction,
    clearance: f32,
) -> Option<Vec<(f32, f32)>> {
    if points.len() < 2 || !path_intersects_label_obstacle(points, obstacle) {
        return None;
    }
    let first = points
        .windows(2)
        .position(|segment| segment_intersects_rect(segment[0], segment[1], obstacle))?;
    let last = points
        .windows(2)
        .rposition(|segment| segment_intersects_rect(segment[0], segment[1], obstacle))?;
    let entry = points[first];
    let exit = points[last + 1];
    let left = obstacle.x - clearance;
    let right = obstacle.x + obstacle.width + clearance;
    let top = obstacle.y - clearance;
    let bottom = obstacle.y + obstacle.height + clearance;
    let mut candidates = Vec::new();

    let via_left = (entry.0 - left).abs() + (exit.0 - left).abs();
    let via_right = (entry.0 - right).abs() + (exit.0 - right).abs();
    let xs = if via_left <= via_right {
        [left, right]
    } else {
        [right, left]
    };
    for x in xs {
        let mut candidate = Vec::with_capacity(points.len() + 2);
        candidate.extend_from_slice(&points[..=first]);
        candidate.push((x, entry.1));
        candidate.push((x, exit.1));
        candidate.extend_from_slice(&points[(last + 1)..]);
        let candidate = compress_path(&candidate);
        if !path_intersects_label_obstacle(&candidate, obstacle) {
            candidates.push(candidate);
        }
    }

    let via_top = (entry.1 - top).abs() + (exit.1 - top).abs();
    let via_bottom = (entry.1 - bottom).abs() + (exit.1 - bottom).abs();
    let ys = if via_top <= via_bottom {
        [top, bottom]
    } else {
        [bottom, top]
    };
    for y in ys {
        let mut candidate = Vec::with_capacity(points.len() + 2);
        candidate.extend_from_slice(&points[..=first]);
        candidate.push((entry.0, y));
        candidate.push((exit.0, y));
        candidate.extend_from_slice(&points[(last + 1)..]);
        let candidate = compress_path(&candidate);
        if !path_intersects_label_obstacle(&candidate, obstacle) {
            candidates.push(candidate);
        }
    }

    candidates.into_iter().min_by(|a, b| {
        path_length(a)
            .partial_cmp(&path_length(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

pub(super) fn apply_label_dummy_anchors(
    nodes: &BTreeMap<String, NodeLayout>,
    label_dummy_ids: &[Option<String>],
    routed_points: &mut [Vec<(f32, f32)>],
    label_anchors: &mut [Option<(f32, f32)>],
    direction: Direction,
    kind: DiagramKind,
) {
    for (idx, dummy_id_opt) in label_dummy_ids.iter().enumerate() {
        let Some(dummy_id) = dummy_id_opt else {
            continue;
        };
        let Some(dummy_node) = nodes.get(dummy_id) else {
            continue;
        };
        let center = (
            dummy_node.x + dummy_node.width / 2.0,
            dummy_node.y + dummy_node.height / 2.0,
        );
        label_anchors[idx] = Some(center);

        let points = &mut routed_points[idx];
        if !matches!(kind, DiagramKind::Flowchart | DiagramKind::State) && points.len() >= 2 {
            insert_label_via_point(points, center, direction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::NodeShape;
    use crate::layout::{NodeLayout, TextBlock, polyline_point_distance};

    fn make_node(id: &str, x: f32, y: f32, width: f32, height: f32) -> NodeLayout {
        NodeLayout {
            id: id.to_string(),
            x,
            y,
            width,
            height,
            label: TextBlock {
                lines: vec![id.to_string()],
                width: 10.0,
                height: 10.0,
            },
            shape: NodeShape::Rectangle,
            style: crate::ir::NodeStyle::default(),
            link: None,
            anchor_subgraph: None,
            hidden: false,
            icon: None,
        }
    }

    #[test]
    fn sync_route_label_plan_preserves_reserved_center_for_flowcharts() {
        let mut points = vec![(0.0, 0.0), (40.0, 0.0)];
        let mut plans = vec![Some(RouteLabelPlan {
            obstacle_id: "edge-label-reserved:0".to_string(),
            obstacle_index: 0,
            progress: 0.5,
            center: (20.0, 6.0),
        })];
        let mut label_anchors = vec![None];
        let labels = vec![Some(TextBlock {
            lines: vec!["hello".to_string()],
            width: 20.0,
            height: 10.0,
        })];
        let mut obstacles = vec![Obstacle {
            id: "edge-label-reserved:0".to_string(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            members: None,
        }];

        let mut sync_ctx = RouteLabelSyncContext {
            direction: Direction::LeftRight,
            kind: DiagramKind::Flowchart,
            route_label_plans: &mut plans,
            label_anchors: &mut label_anchors,
            edge_route_labels: &labels,
            route_label_obstacles: &mut obstacles,
            edge_label_pad_x: 4.0,
            edge_label_pad_y: 2.0,
            update_obstacle: true,
        };
        sync_route_label_plan_with_points(0, &mut points, &mut sync_ctx);

        assert_eq!(label_anchors[0], Some((20.0, 6.0)));
        assert!(
            !path_intersects_label_obstacle(
                &points,
                &Obstacle {
                    id: "edge-label-reserved:0".to_string(),
                    x: 6.0,
                    y: -1.0,
                    width: 28.0,
                    height: 14.0,
                    members: None,
                }
            ),
            "flowchart label sync should detour around reserved label box"
        );
        assert!((obstacles[0].x - 6.0).abs() < 0.1);
        assert!((obstacles[0].y + 1.0).abs() < 0.1);
    }

    #[test]
    fn sync_route_label_plan_tracks_post_cleanup_path_shift_for_non_flowcharts() {
        let mut points = vec![(0.0, 10.0), (40.0, 10.0)];
        let mut plans = vec![Some(RouteLabelPlan {
            obstacle_id: "edge-label-reserved:0".to_string(),
            obstacle_index: 0,
            progress: 0.5,
            center: (20.0, 0.0),
        })];
        let mut label_anchors = vec![None];

        let mut sync_ctx = RouteLabelSyncContext {
            direction: Direction::LeftRight,
            kind: DiagramKind::Class,
            route_label_plans: &mut plans,
            label_anchors: &mut label_anchors,
            edge_route_labels: &[],
            route_label_obstacles: &mut [],
            edge_label_pad_x: 0.0,
            edge_label_pad_y: 0.0,
            update_obstacle: false,
        };
        sync_route_label_plan_with_points(0, &mut points, &mut sync_ctx);

        assert_eq!(label_anchors[0], Some((20.0, 10.0)));
        assert!(polyline_point_distance(&points, (20.0, 10.0)) <= 0.6);
    }

    #[test]
    fn apply_label_dummy_anchors_uses_dummy_node_center() {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "__elabel_0__".to_string(),
            make_node("__elabel_0__", 10.0, 20.0, 12.0, 8.0),
        );
        let mut routed_points = vec![vec![(0.0, 24.0), (40.0, 24.0)]];
        let mut label_anchors = vec![None];

        apply_label_dummy_anchors(
            &nodes,
            &[Some("__elabel_0__".to_string())],
            &mut routed_points,
            &mut label_anchors,
            Direction::LeftRight,
            DiagramKind::Flowchart,
        );

        assert_eq!(label_anchors[0], Some((16.0, 24.0)));
        assert!(polyline_point_distance(&routed_points[0], (16.0, 24.0)) <= 0.6);
    }
}
