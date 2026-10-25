#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use crate::config::LayoutConfig;
use crate::ir::{DiagramKind, EdgeStyle, Graph};

use super::super::TextBlock;

#[derive(Clone, Debug, Default)]
pub(in crate::layout) struct FlowchartNodeRelation {
    pub(in crate::layout) rank: usize,
    pub(in crate::layout) component: usize,
    pub(in crate::layout) in_degree: usize,
    pub(in crate::layout) out_degree: usize,
    pub(in crate::layout) boundary_edges: usize,
    pub(in crate::layout) incident_label_area: f32,
    pub(in crate::layout) incident_weight: f32,
    pub(in crate::layout) in_cycle: bool,
    pub(in crate::layout) subgraphs: Vec<usize>,
}

impl FlowchartNodeRelation {
    pub(in crate::layout) fn degree(&self) -> usize {
        self.in_degree + self.out_degree
    }

    pub(in crate::layout) fn hub_score(&self) -> f32 {
        let degree = self.degree() as f32;
        let fan_imbalance = self.in_degree.max(self.out_degree).saturating_sub(1) as f32;
        degree
            + fan_imbalance * 0.4
            + self.boundary_edges as f32 * 0.6
            + self.incident_weight * 0.12
            + if self.in_cycle { 0.8 } else { 0.0 }
    }

    pub(in crate::layout) fn extra_cross_padding(&self, config: &LayoutConfig) -> f32 {
        if self.degree() <= 3 {
            return 0.0;
        }
        let fanout_pad = (self.degree().saturating_sub(3) as f32 * config.node_spacing * 0.06)
            .min(config.node_spacing * 0.24);
        let label_pad = (self.incident_label_area.sqrt() * 0.04).min(config.node_spacing * 0.12);
        (fanout_pad + label_pad)
            .min(config.node_spacing * 0.32)
            .min(24.0)
    }
}

#[derive(Clone, Debug)]
pub(in crate::layout) struct FlowchartEdgeRelation {
    pub(in crate::layout) edge_idx: usize,
    pub(in crate::layout) from: String,
    pub(in crate::layout) to: String,
    pub(in crate::layout) from_rank: usize,
    pub(in crate::layout) to_rank: usize,
    pub(in crate::layout) rank_span: isize,
    pub(in crate::layout) component: Option<usize>,
    pub(in crate::layout) is_cycle_edge: bool,
    pub(in crate::layout) is_back_edge: bool,
    pub(in crate::layout) crosses_subgraph_boundary: bool,
    pub(in crate::layout) has_center_label: bool,
    pub(in crate::layout) has_endpoint_label: bool,
    pub(in crate::layout) label_area: f32,
    pub(in crate::layout) label_chars: usize,
    pub(in crate::layout) weight: f32,
}

#[derive(Clone, Debug, Default)]
pub(in crate::layout) struct FlowchartRelationshipAnalysis {
    pub(in crate::layout) nodes: HashMap<String, FlowchartNodeRelation>,
    pub(in crate::layout) edges: Vec<FlowchartEdgeRelation>,
    pub(in crate::layout) max_rank: usize,
    pub(in crate::layout) has_cycles: bool,
    pub(in crate::layout) boundary_edge_count: usize,
    edge_pair_weight: HashMap<(String, String), f32>,
}

impl FlowchartRelationshipAnalysis {
    pub(in crate::layout) fn analyze(
        graph: &Graph,
        node_ids: &[String],
        layout_edges: &[crate::ir::Edge],
        center_labels: &[Option<TextBlock>],
        ranks: &HashMap<String, usize>,
    ) -> Option<Self> {
        if graph.kind != DiagramKind::Flowchart || node_ids.is_empty() {
            return None;
        }

        let node_set: HashSet<&str> = node_ids.iter().map(String::as_str).collect();
        let components = strongly_connected_components(node_ids, layout_edges);
        let mut node_to_component: HashMap<String, usize> = HashMap::new();
        let mut cyclic_components: HashSet<usize> = HashSet::new();
        for (component_idx, component) in components.iter().enumerate() {
            let component_is_cycle = component.len() > 1
                || layout_edges.iter().any(|edge| {
                    edge.from == edge.to && component.iter().any(|id| id == &edge.from)
                });
            if component_is_cycle {
                cyclic_components.insert(component_idx);
            }
            for node_id in component {
                node_to_component.insert(node_id.clone(), component_idx);
            }
        }

        let subgraphs = node_subgraph_memberships(graph);
        let mut nodes: HashMap<String, FlowchartNodeRelation> = node_ids
            .iter()
            .map(|id| {
                let component = node_to_component.get(id).copied().unwrap_or(usize::MAX);
                (
                    id.clone(),
                    FlowchartNodeRelation {
                        rank: ranks.get(id).copied().unwrap_or(0),
                        component,
                        in_cycle: cyclic_components.contains(&component),
                        subgraphs: subgraphs.get(id.as_str()).cloned().unwrap_or_default(),
                        ..FlowchartNodeRelation::default()
                    },
                )
            })
            .collect();

        let mut edges = Vec::with_capacity(layout_edges.len());
        let mut edge_pair_weight: HashMap<(String, String), f32> = HashMap::new();
        let mut max_rank = 0usize;
        let mut boundary_edge_count = 0usize;

        for (edge_idx, edge) in layout_edges.iter().enumerate() {
            if !node_set.contains(edge.from.as_str()) || !node_set.contains(edge.to.as_str()) {
                continue;
            }
            let from_rank = ranks.get(&edge.from).copied().unwrap_or(0);
            let to_rank = ranks.get(&edge.to).copied().unwrap_or(0);
            max_rank = max_rank.max(from_rank.max(to_rank));
            let from_component = node_to_component.get(&edge.from).copied();
            let to_component = node_to_component.get(&edge.to).copied();
            let is_cycle_edge = edge.from == edge.to
                || matches!((from_component, to_component), (Some(from), Some(to)) if from == to);
            let component = if is_cycle_edge { from_component } else { None };
            let is_back_edge = to_rank <= from_rank;
            let from_subgraphs = subgraphs.get(edge.from.as_str());
            let to_subgraphs = subgraphs.get(edge.to.as_str());
            let crosses_subgraph_boundary = from_subgraphs != to_subgraphs;
            if crosses_subgraph_boundary {
                boundary_edge_count += 1;
            }
            let has_center_label = edge
                .label
                .as_deref()
                .is_some_and(|label| !label.trim().is_empty());
            let has_endpoint_label = edge
                .start_label
                .as_deref()
                .is_some_and(|label| !label.trim().is_empty())
                || edge
                    .end_label
                    .as_deref()
                    .is_some_and(|label| !label.trim().is_empty());
            let label = center_labels.get(edge_idx).and_then(|label| label.as_ref());
            let label_area = label.map(|label| label.width * label.height).unwrap_or(0.0);
            let label_chars = edge
                .label
                .as_deref()
                .map(|label| label.chars().count())
                .unwrap_or(0)
                + edge
                    .start_label
                    .as_deref()
                    .map(|label| label.chars().count())
                    .unwrap_or(0)
                + edge
                    .end_label
                    .as_deref()
                    .map(|label| label.chars().count())
                    .unwrap_or(0);
            let weight = edge_weight(
                edge.style,
                is_cycle_edge,
                is_back_edge,
                crosses_subgraph_boundary,
                has_center_label,
                has_endpoint_label,
                label_area,
                label_chars,
            );

            if let Some(from) = nodes.get_mut(&edge.from) {
                from.out_degree += 1;
                from.incident_label_area += label_area;
                from.incident_weight += weight;
                if crosses_subgraph_boundary {
                    from.boundary_edges += 1;
                }
            }
            if let Some(to) = nodes.get_mut(&edge.to) {
                to.in_degree += 1;
                to.incident_label_area += label_area;
                to.incident_weight += weight;
                if crosses_subgraph_boundary {
                    to.boundary_edges += 1;
                }
            }

            edge_pair_weight
                .entry((edge.from.clone(), edge.to.clone()))
                .and_modify(|current| *current = (*current).max(weight))
                .or_insert(weight);

            edges.push(FlowchartEdgeRelation {
                edge_idx,
                from: edge.from.clone(),
                to: edge.to.clone(),
                from_rank,
                to_rank,
                rank_span: to_rank as isize - from_rank as isize,
                component,
                is_cycle_edge,
                is_back_edge,
                crosses_subgraph_boundary,
                has_center_label,
                has_endpoint_label,
                label_area,
                label_chars,
                weight,
            });
        }

        Some(Self {
            nodes,
            edges,
            max_rank,
            has_cycles: !cyclic_components.is_empty(),
            boundary_edge_count,
            edge_pair_weight,
        })
    }

    pub(in crate::layout) fn edge_weight_between(&self, from: &str, to: &str) -> f32 {
        self.edge_pair_weight
            .get(&(from.to_string(), to.to_string()))
            .copied()
            .unwrap_or(1.0)
    }

    pub(in crate::layout) fn node_relation(&self, node_id: &str) -> Option<&FlowchartNodeRelation> {
        self.nodes.get(node_id)
    }

    pub(in crate::layout) fn important_edge_count(&self) -> usize {
        self.edges
            .iter()
            .filter(|edge| {
                edge.weight >= 1.45 || edge.is_back_edge || edge.crosses_subgraph_boundary
            })
            .count()
    }
}

fn edge_weight(
    style: EdgeStyle,
    is_cycle_edge: bool,
    is_back_edge: bool,
    crosses_subgraph_boundary: bool,
    has_center_label: bool,
    has_endpoint_label: bool,
    label_area: f32,
    label_chars: usize,
) -> f32 {
    let style_weight = match style {
        EdgeStyle::Dotted => 0.72,
        EdgeStyle::Thick => 1.25,
        EdgeStyle::Solid => 1.0,
    };
    let label_weight = (label_area.sqrt() / 110.0).min(0.9)
        + (label_chars as f32 / 36.0).min(0.55)
        + if has_center_label { 0.22 } else { 0.0 }
        + if has_endpoint_label { 0.18 } else { 0.0 };
    let structure_weight = if crosses_subgraph_boundary { 0.35 } else { 0.0 }
        + if is_cycle_edge { 0.18 } else { 0.0 }
        + if is_back_edge { 0.16 } else { 0.0 };
    (style_weight + label_weight + structure_weight).clamp(0.45, 3.25)
}

fn node_subgraph_memberships(graph: &Graph) -> HashMap<&str, Vec<usize>> {
    let mut memberships: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, subgraph) in graph.subgraphs.iter().enumerate() {
        for node_id in &subgraph.nodes {
            memberships.entry(node_id.as_str()).or_default().push(idx);
        }
    }
    for indexes in memberships.values_mut() {
        indexes.sort_unstable();
        indexes.dedup();
    }
    memberships
}

fn strongly_connected_components(
    node_ids: &[String],
    edges: &[crate::ir::Edge],
) -> Vec<Vec<String>> {
    let node_set: HashSet<&str> = node_ids.iter().map(String::as_str).collect();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        if !node_set.contains(edge.from.as_str()) || !node_set.contains(edge.to.as_str()) {
            continue;
        }
        adj.entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        rev.entry(edge.to.as_str())
            .or_default()
            .push(edge.from.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut finish_order = Vec::with_capacity(node_ids.len());
    for node_id in node_ids {
        dfs_finish_order(node_id.as_str(), &adj, &mut visited, &mut finish_order);
    }

    let mut assigned: HashSet<&str> = HashSet::new();
    let mut components = Vec::new();
    while let Some(node_id) = finish_order.pop() {
        if !assigned.insert(node_id) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![node_id];
        while let Some(current) = stack.pop() {
            component.push(current.to_string());
            if let Some(prevs) = rev.get(current) {
                for prev in prevs {
                    if assigned.insert(prev) {
                        stack.push(prev);
                    }
                }
            }
        }
        component.sort_by_key(|id| graph_order_key(id, node_ids));
        components.push(component);
    }
    components
}

fn dfs_finish_order<'a>(
    node_id: &'a str,
    adj: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    finish_order: &mut Vec<&'a str>,
) {
    if !visited.insert(node_id) {
        return;
    }
    if let Some(nexts) = adj.get(node_id) {
        for next in nexts {
            dfs_finish_order(next, adj, visited, finish_order);
        }
    }
    finish_order.push(node_id);
}

fn graph_order_key(id: &str, node_ids: &[String]) -> usize {
    node_ids
        .iter()
        .position(|candidate| candidate == id)
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Direction, Edge, Graph, NodeShape, Subgraph};

    fn edge(from: &str, to: &str, label: Option<&str>) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            label: label.map(str::to_string),
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

    #[test]
    fn analysis_marks_cycles_boundaries_hubs_and_weights() {
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Flowchart;
        graph.direction = Direction::TopDown;
        for id in ["A", "B", "C", "D"] {
            graph.ensure_node(id, Some(id.to_string()), Some(NodeShape::Rectangle));
        }
        graph.edges = vec![
            edge("A", "B", None),
            edge("B", "C", Some("long label")),
            edge("C", "B", None),
            edge("C", "D", None),
        ];
        graph.subgraphs.push(Subgraph {
            id: Some("cluster".to_string()),
            label: "cluster".to_string(),
            nodes: vec!["B".to_string(), "C".to_string()],
            direction: None,
            icon: None,
        });
        let node_ids = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let ranks = HashMap::from([
            ("A".to_string(), 0usize),
            ("B".to_string(), 1usize),
            ("C".to_string(), 2usize),
            ("D".to_string(), 3usize),
        ]);
        let labels = vec![
            None,
            Some(TextBlock {
                lines: vec!["long label".to_string()],
                width: 92.0,
                height: 18.0,
            }),
            None,
            None,
        ];

        let analysis = FlowchartRelationshipAnalysis::analyze(
            &graph,
            &node_ids,
            &graph.edges,
            &labels,
            &ranks,
        )
        .expect("flowchart analysis");

        assert!(analysis.has_cycles);
        assert_eq!(analysis.boundary_edge_count, 2);
        assert!(analysis.node_relation("B").unwrap().in_cycle);
        assert!(analysis.node_relation("C").unwrap().hub_score() > 3.0);
        assert!(analysis.edge_weight_between("B", "C") > analysis.edge_weight_between("A", "B"));
        assert_eq!(analysis.important_edge_count(), 4);
    }
}
