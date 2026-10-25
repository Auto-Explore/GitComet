use std::collections::{HashMap, HashSet};

use crate::ir::Graph;

use super::super::ranking::compute_ranks_subset;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::layout) struct FlowchartEdgeRole {
    pub(in crate::layout) is_cycle_edge: bool,
    pub(in crate::layout) is_back_edge: bool,
    pub(in crate::layout) crosses_subgraph_boundary: bool,
    pub(in crate::layout) has_center_label: bool,
    pub(in crate::layout) has_endpoint_label: bool,
}

pub(in crate::layout) fn classify_edge_roles(graph: &Graph) -> Vec<FlowchartEdgeRole> {
    if graph.edges.is_empty() {
        return Vec::new();
    }

    let node_ids: Vec<String> = graph.nodes.keys().cloned().collect();
    let ranks = compute_ranks_subset(&node_ids, &graph.edges, &graph.node_order);
    let node_components = node_to_component(node_ids.as_slice(), &graph.edges);
    let node_subgraphs = node_subgraph_memberships(graph);

    graph
        .edges
        .iter()
        .map(|edge| {
            let from_comp = node_components.get(&edge.from).copied();
            let to_comp = node_components.get(&edge.to).copied();
            let is_self_loop = edge.from == edge.to;
            let is_cycle_edge = is_self_loop
                || matches!((from_comp, to_comp), (Some(from), Some(to)) if from == to);
            let is_back_edge = matches!(
                (ranks.get(&edge.from), ranks.get(&edge.to)),
                (Some(from_rank), Some(to_rank)) if to_rank <= from_rank
            );
            let from_subgraphs = node_subgraphs.get(edge.from.as_str());
            let to_subgraphs = node_subgraphs.get(edge.to.as_str());
            let crosses_subgraph_boundary = from_subgraphs != to_subgraphs;

            FlowchartEdgeRole {
                is_cycle_edge,
                is_back_edge,
                crosses_subgraph_boundary,
                has_center_label: edge
                    .label
                    .as_deref()
                    .is_some_and(|label| !label.trim().is_empty()),
                has_endpoint_label: edge
                    .start_label
                    .as_deref()
                    .is_some_and(|label| !label.trim().is_empty())
                    || edge
                        .end_label
                        .as_deref()
                        .is_some_and(|label| !label.trim().is_empty()),
            }
        })
        .collect()
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

fn node_to_component(node_ids: &[String], edges: &[crate::ir::Edge]) -> HashMap<String, usize> {
    let components = strongly_connected_components(node_ids, edges);
    let mut mapping = HashMap::new();
    for (idx, component) in components.iter().enumerate() {
        for node_id in component {
            mapping.insert(node_id.clone(), idx);
        }
    }
    mapping
}

fn strongly_connected_components(
    node_ids: &[String],
    edges: &[crate::ir::Edge],
) -> Vec<Vec<String>> {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
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
    use crate::ir::{DiagramKind, Direction, Edge, EdgeStyle, Graph, NodeShape, Subgraph};

    fn make_flowchart_graph() -> Graph {
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Flowchart;
        graph.direction = Direction::TopDown;
        for id in ["A", "B", "C", "D", "E"] {
            graph.ensure_node(id, Some(id.to_string()), Some(NodeShape::Rectangle));
        }
        graph.edges = vec![
            Edge {
                from: "A".to_string(),
                to: "B".to_string(),
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
            },
            Edge {
                from: "B".to_string(),
                to: "C".to_string(),
                label: Some("yes".to_string()),
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
            },
            Edge {
                from: "C".to_string(),
                to: "D".to_string(),
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
            },
            Edge {
                from: "D".to_string(),
                to: "B".to_string(),
                label: Some("loop".to_string()),
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
            },
            Edge {
                from: "D".to_string(),
                to: "E".to_string(),
                label: None,
                start_label: Some("start".to_string()),
                end_label: None,
                directed: true,
                arrow_start: false,
                arrow_end: true,
                arrow_start_kind: None,
                arrow_end_kind: None,
                start_decoration: None,
                end_decoration: None,
                style: EdgeStyle::Solid,
            },
        ];
        graph.subgraphs.push(Subgraph {
            id: Some("Loop".to_string()),
            label: "Loop".to_string(),
            nodes: vec!["B".to_string(), "C".to_string()],
            direction: None,
            icon: None,
        });
        graph
    }

    #[test]
    fn classify_edge_roles_marks_cycle_and_back_edges() {
        let graph = make_flowchart_graph();
        let roles = classify_edge_roles(&graph);
        assert!(roles[1].is_cycle_edge);
        assert!(!roles[1].is_back_edge);
        assert!(roles[3].is_cycle_edge);
        assert!(roles[3].is_back_edge);
    }

    #[test]
    fn classify_edge_roles_marks_subgraph_boundaries_and_labels() {
        let graph = make_flowchart_graph();
        let roles = classify_edge_roles(&graph);
        assert!(roles[0].crosses_subgraph_boundary);
        assert!(roles[1].has_center_label);
        assert!(roles[4].has_endpoint_label);
        assert!(!roles[4].crosses_subgraph_boundary);
    }
}
