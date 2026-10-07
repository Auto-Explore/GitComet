use std::collections::BTreeMap;

use crate::ir::{DiagramKind, Graph};

use super::super::invariants::{flowchart_quality_metrics, validate_layout_invariants};
use super::super::routing::EdgePortInfo;
use super::super::{Layout, NodeLayout};
use super::path_cleanup;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::layout) struct FlowchartStageReport {
    pub(in crate::layout) node_count: usize,
    pub(in crate::layout) edge_count: usize,
    pub(in crate::layout) missing_nodes: usize,
    pub(in crate::layout) invalid_nodes: usize,
    pub(in crate::layout) port_count_mismatch: usize,
    pub(in crate::layout) invalid_ports: usize,
    pub(in crate::layout) route_count_mismatch: usize,
    pub(in crate::layout) short_routes: usize,
    pub(in crate::layout) non_finite_route_points: usize,
    pub(in crate::layout) bad_endpoint_directions: usize,
    pub(in crate::layout) endpoint_node_intrusions: usize,
    pub(in crate::layout) endpoint_reentries: usize,
    pub(in crate::layout) non_endpoint_hits: usize,
    pub(in crate::layout) final_layout_errors: usize,
}

impl FlowchartStageReport {
    pub(in crate::layout) fn structural_error_count(self) -> usize {
        self.missing_nodes
            + self.invalid_nodes
            + self.port_count_mismatch
            + self.invalid_ports
            + self.route_count_mismatch
            + self.short_routes
            + self.non_finite_route_points
            + self.final_layout_errors
    }

    pub(in crate::layout) fn hard_geometry_error_count(self) -> usize {
        self.bad_endpoint_directions + self.endpoint_node_intrusions + self.non_endpoint_hits
    }

    pub(in crate::layout) fn geometry_debt_count(self) -> usize {
        self.hard_geometry_error_count() + self.endpoint_reentries
    }

    pub(in crate::layout) fn total_error_count(self) -> usize {
        self.structural_error_count() + self.geometry_debt_count()
    }
}

pub(in crate::layout) fn validate_node_placement(
    graph: &Graph,
    nodes: &BTreeMap<String, NodeLayout>,
) -> FlowchartStageReport {
    let mut report = FlowchartStageReport {
        node_count: nodes.len(),
        edge_count: graph.edges.len(),
        ..FlowchartStageReport::default()
    };
    if graph.kind != DiagramKind::Flowchart {
        return report;
    }

    for node_id in graph.nodes.keys() {
        if !nodes.contains_key(node_id) {
            report.missing_nodes += 1;
        }
    }
    for node in nodes.values() {
        if !node.x.is_finite()
            || !node.y.is_finite()
            || !node.width.is_finite()
            || !node.height.is_finite()
            || node.width <= 0.0
            || node.height <= 0.0
        {
            report.invalid_nodes += 1;
        }
    }
    report
}

pub(in crate::layout) fn validate_port_assignment(
    graph: &Graph,
    edge_ports: &[EdgePortInfo],
) -> FlowchartStageReport {
    let mut report = FlowchartStageReport {
        edge_count: graph.edges.len(),
        ..FlowchartStageReport::default()
    };
    if graph.kind != DiagramKind::Flowchart {
        return report;
    }
    if edge_ports.len() != graph.edges.len() {
        report.port_count_mismatch = 1;
    }
    for port in edge_ports.iter().take(graph.edges.len()) {
        if !port.start_offset.is_finite() || !port.end_offset.is_finite() {
            report.invalid_ports += 1;
        }
    }
    report
}

pub(in crate::layout) fn validate_routes(
    graph: &Graph,
    nodes: &BTreeMap<String, NodeLayout>,
    routed_points: &[Vec<(f32, f32)>],
) -> FlowchartStageReport {
    let mut report = FlowchartStageReport {
        node_count: nodes.len(),
        edge_count: graph.edges.len(),
        ..FlowchartStageReport::default()
    };
    if graph.kind != DiagramKind::Flowchart {
        return report;
    }
    if routed_points.len() != graph.edges.len() {
        report.route_count_mismatch = 1;
    }

    for (idx, edge) in graph.edges.iter().enumerate() {
        let Some(points) = routed_points.get(idx) else {
            report.short_routes += 1;
            continue;
        };
        if points.len() < 2 {
            report.short_routes += 1;
            continue;
        }
        report.non_finite_route_points += points
            .iter()
            .filter(|(x, y)| !x.is_finite() || !y.is_finite())
            .count();
        if report.non_finite_route_points > 0 {
            continue;
        }
        report.bad_endpoint_directions +=
            path_cleanup::flowchart_endpoint_direction_violation_count(points, edge, nodes);
        report.endpoint_reentries +=
            path_cleanup::flowchart_endpoint_reentry_count(points, edge, nodes);
        if path_cleanup::flowchart_path_hits_non_endpoint_nodes(points, &edge.from, &edge.to, nodes)
        {
            report.non_endpoint_hits += 1;
        }
    }
    report
}

pub(in crate::layout) fn validate_final_layout(layout: &Layout) -> FlowchartStageReport {
    let mut report = FlowchartStageReport::default();
    if layout.kind != DiagramKind::Flowchart {
        return report;
    }
    report.node_count = layout.nodes.len();
    report.edge_count = layout.edges.len();
    if let Err(errors) = validate_layout_invariants(layout) {
        report.final_layout_errors = errors.len();
    }
    if let Some(metrics) = flowchart_quality_metrics(layout) {
        report.bad_endpoint_directions = metrics.bad_source_exits + metrics.bad_target_entries;
        report.endpoint_node_intrusions = metrics.endpoint_node_intrusions;
        report.endpoint_reentries = metrics.endpoint_node_reentries;
        report.non_endpoint_hits = metrics.non_endpoint_node_hits;
    }
    report
}

#[cfg(debug_assertions)]
pub(in crate::layout) fn debug_assert_structural(stage: &str, report: FlowchartStageReport) {
    debug_assert_eq!(
        report.structural_error_count(),
        0,
        "flowchart stage {stage} structural validation failed: {report:?}"
    );
}

#[cfg(debug_assertions)]
pub(in crate::layout) fn debug_assert_no_geometry_debt(stage: &str, report: FlowchartStageReport) {
    debug_assert_eq!(
        report.total_error_count(),
        0,
        "flowchart stage {stage} geometry validation failed: {report:?}"
    );
}

#[cfg(debug_assertions)]
pub(in crate::layout) fn debug_assert_no_hard_geometry_errors(
    stage: &str,
    report: FlowchartStageReport,
) {
    debug_assert_eq!(
        report.structural_error_count() + report.hard_geometry_error_count(),
        0,
        "flowchart stage {stage} hard geometry validation failed: {report:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Direction, Edge, EdgeStyle, Graph, Node, NodeShape, NodeStyle};
    use crate::layout::TextBlock;

    fn node_layout(id: &str, x: f32, y: f32) -> NodeLayout {
        NodeLayout {
            id: id.to_string(),
            x,
            y,
            width: 60.0,
            height: 40.0,
            label: TextBlock {
                lines: vec![id.to_string()],
                width: 20.0,
                height: 12.0,
            },
            shape: NodeShape::Rectangle,
            style: NodeStyle::default(),
            link: None,
            anchor_subgraph: None,
            hidden: false,
            icon: None,
        }
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            label: None,
            start_label: None,
            end_label: None,
            directed: true,
            arrow_start: false,
            arrow_end: true,
            arrow_start_kind: None,
            arrow_end_kind: None,
            start_decoration: None,
            end_decoration: None,
            style: EdgeStyle::Solid,
        }
    }

    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Flowchart;
        graph.direction = Direction::LeftRight;
        graph.nodes.insert(
            "A".to_string(),
            Node {
                id: "A".to_string(),
                label: "A".to_string(),
                shape: NodeShape::Rectangle,
                value: None,
                icon: None,
            },
        );
        graph.nodes.insert(
            "B".to_string(),
            Node {
                id: "B".to_string(),
                label: "B".to_string(),
                shape: NodeShape::Rectangle,
                value: None,
                icon: None,
            },
        );
        graph.edges.push(edge("A", "B"));
        graph
    }

    #[test]
    fn route_stage_validator_reports_bad_endpoint_direction() {
        let graph = graph();
        let nodes = BTreeMap::from([
            ("A".to_string(), node_layout("A", 0.0, 0.0)),
            ("B".to_string(), node_layout("B", 120.0, 0.0)),
        ]);
        let routes = vec![vec![(60.0, 20.0), (10.0, 20.0), (120.0, 20.0)]];

        let report = validate_routes(&graph, &nodes, &routes);
        assert!(report.structural_error_count() == 0);
        assert!(report.bad_endpoint_directions > 0);
        assert!(report.geometry_debt_count() > 0);
    }

    #[test]
    fn route_stage_validator_accepts_clean_route() {
        let graph = graph();
        let nodes = BTreeMap::from([
            ("A".to_string(), node_layout("A", 0.0, 0.0)),
            ("B".to_string(), node_layout("B", 120.0, 0.0)),
        ]);
        let routes = vec![vec![(60.0, 20.0), (90.0, 20.0), (120.0, 20.0)]];

        let report = validate_routes(&graph, &nodes, &routes);
        assert_eq!(report.total_error_count(), 0);
    }
}
