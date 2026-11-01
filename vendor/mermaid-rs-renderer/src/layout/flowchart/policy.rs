use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::LayoutConfig;
use crate::ir::{DiagramKind, Graph};
use crate::theme::Theme;

use super::super::NodeLayout;

pub(in crate::layout) fn apply_initial_config_heuristics(
    graph: &Graph,
    config: &LayoutConfig,
) -> LayoutConfig {
    let mut effective_config = config.clone();

    if graph.kind == DiagramKind::Requirement {
        effective_config.max_label_width_chars = effective_config.max_label_width_chars.max(32);
    }

    if graph.kind == DiagramKind::Er {
        // ER diagrams are relationship-dense; tighter packing improves readability
        // and significantly reduces long connector spans.
        effective_config.node_spacing *= 0.80;
        effective_config.rank_spacing *= 0.80;
        // Extra rank-order sweeps reduce crossing-prone left/right inversions
        // in dense relationship graphs.
        effective_config.flowchart.order_passes = effective_config.flowchart.order_passes.max(10);
    }

    if graph.kind == DiagramKind::Flowchart {
        let (node_count, edge_count, density, hub_ratio) = flowchart_density_profile(graph);
        let auto = &config.flowchart.auto_spacing;
        if auto.enabled && !auto.buckets.is_empty() {
            let mut scale = auto.buckets[0].scale;
            for bucket in &auto.buckets {
                if node_count >= bucket.min_nodes {
                    scale = bucket.scale;
                }
            }
            if density > auto.density_threshold {
                scale = scale.max(auto.dense_scale_floor);
            }
            effective_config.node_spacing =
                (effective_config.node_spacing * scale).max(auto.min_spacing);
            effective_config.rank_spacing =
                (effective_config.rank_spacing * scale).max(auto.min_spacing);
        }

        if node_count >= 12 && hub_ratio >= 0.40 && density <= 2.5 {
            effective_config.flowchart.routing.enable_grid_router = false;
            effective_config.flowchart.routing.snap_ports_to_grid = false;
        }

        if is_tiny_graph_layout(graph) {
            effective_config.flowchart.order_passes = 1;
            effective_config.flowchart.routing.enable_grid_router = false;
            effective_config.flowchart.routing.snap_ports_to_grid = false;
        }

        let _ = edge_count;
    }

    effective_config
}

pub(in crate::layout) fn apply_measured_spacing_heuristics(
    graph: &Graph,
    theme: &Theme,
    config: &mut LayoutConfig,
    nodes: &BTreeMap<String, NodeLayout>,
) {
    let auto = &config.flowchart.auto_spacing;
    if auto.enabled {
        let adaptive_node_spacing =
            adaptive_spacing_for_nodes(nodes, auto.min_spacing, config.node_spacing);
        let adaptive_rank_spacing =
            adaptive_spacing_for_nodes(nodes, auto.min_spacing, config.rank_spacing);
        if adaptive_node_spacing < config.node_spacing {
            config.node_spacing = adaptive_node_spacing;
        }
        if adaptive_rank_spacing < config.rank_spacing {
            config.rank_spacing = adaptive_rank_spacing;
        }
    }

    if graph.kind != DiagramKind::Flowchart {
        return;
    }

    let (_, edge_count, density, hub_ratio) = flowchart_density_profile(graph);

    if auto.enabled && graph.nodes.len() >= 10 && hub_ratio >= 0.30 && density <= 3.0 {
        let hub_scale = (0.92 - (hub_ratio - 0.30) * 0.55).clamp(0.62, 0.92);
        let floor = (auto.min_spacing * 0.5).max(14.0);
        config.node_spacing = (config.node_spacing * hub_scale).max(floor);
        config.rank_spacing = (config.rank_spacing * hub_scale).max(floor);
    }

    if let Some(label_floor) = flowchart_label_spacing_floor(graph, theme, config, edge_count) {
        config.node_spacing = config.node_spacing.max(label_floor);
        config.rank_spacing = config.rank_spacing.max(label_floor);
    }
}

pub(in crate::layout) fn is_tiny_graph_layout(graph: &Graph) -> bool {
    graph.subgraphs.is_empty()
        && graph.nodes.len() <= 4
        && graph.edges.len() <= 4
        && !graph_has_directed_cycle(graph)
}

fn adaptive_spacing_for_nodes(
    nodes: &BTreeMap<String, NodeLayout>,
    min_spacing: f32,
    max_spacing: f32,
) -> f32 {
    let mut total = 0.0f32;
    let mut count = 0usize;
    for node in nodes.values() {
        if node.hidden || node.anchor_subgraph.is_some() {
            continue;
        }
        total += (node.width + node.height) * 0.5;
        count += 1;
    }
    if count == 0 {
        return max_spacing;
    }
    let avg = total / count as f32;
    let target = (avg * 0.5).max(min_spacing);
    target.min(max_spacing)
}

fn flowchart_density_profile(graph: &Graph) -> (usize, f32, f32, f32) {
    let node_count = graph.nodes.len();
    let edge_count = graph.edges.len() as f32;
    let density = if node_count > 0 {
        edge_count / node_count as f32
    } else {
        0.0
    };
    let mut degree_by_node: HashMap<&str, usize> = HashMap::new();
    for edge in &graph.edges {
        *degree_by_node.entry(edge.from.as_str()).or_insert(0) += 1;
        *degree_by_node.entry(edge.to.as_str()).or_insert(0) += 1;
    }
    let max_degree = degree_by_node.values().copied().max().unwrap_or(0) as f32;
    let hub_ratio = if node_count > 0 {
        max_degree / node_count as f32
    } else {
        0.0
    };
    (node_count, edge_count, density, hub_ratio)
}

fn flowchart_label_spacing_floor(
    graph: &Graph,
    theme: &Theme,
    config: &LayoutConfig,
    edge_count: f32,
) -> Option<f32> {
    if edge_count <= 0.0 {
        return None;
    }

    let mut label_char_total = 0usize;
    let mut label_count = 0usize;
    let mut endpoint_labeled_edges = 0usize;
    for edge in &graph.edges {
        let mut has_endpoint_label = false;
        if let Some(label) = edge.label.as_ref() {
            label_char_total += label.chars().count();
            label_count += 1;
        }
        if let Some(label) = edge.start_label.as_ref() {
            label_char_total += label.chars().count();
            label_count += 1;
            has_endpoint_label = true;
        }
        if let Some(label) = edge.end_label.as_ref() {
            label_char_total += label.chars().count();
            label_count += 1;
            has_endpoint_label = true;
        }
        if has_endpoint_label {
            endpoint_labeled_edges += 1;
        }
    }

    if label_count == 0 {
        return None;
    }

    let avg_chars = label_char_total as f32 / label_count as f32;
    let text_pressure = ((avg_chars - 10.0) / 26.0).clamp(0.0, 1.0);
    let endpoint_pressure = (endpoint_labeled_edges as f32 / edge_count.max(1.0)).clamp(0.0, 1.0);
    let pressure = (text_pressure * 0.7 + endpoint_pressure * 0.3).clamp(0.0, 1.0);
    if pressure <= 0.0 {
        return None;
    }

    let boost = pressure * (theme.font_size * 1.1).max(8.0);
    Some(config.flowchart.auto_spacing.min_spacing + boost)
}

fn graph_has_directed_cycle(graph: &Graph) -> bool {
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        outgoing
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut active: HashSet<&str> = HashSet::new();
    for node_id in graph.nodes.keys().map(String::as_str) {
        if dfs_has_cycle(node_id, &outgoing, &mut visited, &mut active) {
            return true;
        }
    }
    false
}

fn dfs_has_cycle<'a>(
    node_id: &'a str,
    outgoing: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    active: &mut HashSet<&'a str>,
) -> bool {
    if active.contains(node_id) {
        return true;
    }
    if !visited.insert(node_id) {
        return false;
    }

    active.insert(node_id);
    if let Some(neighbors) = outgoing.get(node_id) {
        for &next in neighbors {
            if dfs_has_cycle(next, outgoing, visited, active) {
                return true;
            }
        }
    }
    active.remove(node_id);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::TextBlock;

    fn make_node(id: &str, width: f32, height: f32) -> NodeLayout {
        NodeLayout {
            id: id.to_string(),
            x: 0.0,
            y: 0.0,
            width,
            height,
            label: TextBlock {
                lines: vec![String::new()],
                width: 0.0,
                height: 0.0,
            },
            shape: crate::ir::NodeShape::Rectangle,
            style: crate::ir::NodeStyle::default(),
            link: None,
            anchor_subgraph: None,
            hidden: false,
            icon: None,
        }
    }

    #[test]
    fn auto_spacing_disabled_preserves_user_spacing() {
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Flowchart;
        graph.nodes.insert(
            "A".to_string(),
            crate::ir::Node {
                id: "A".to_string(),
                label: "A".to_string(),
                shape: crate::ir::NodeShape::Rectangle,
                value: None,
                icon: None,
            },
        );

        let mut nodes = BTreeMap::new();
        nodes.insert("A".to_string(), make_node("A", 60.0, 36.0));

        let mut config = LayoutConfig::default();
        config.node_spacing = 180.0;
        config.rank_spacing = 180.0;
        config.flowchart.auto_spacing.enabled = false;

        apply_measured_spacing_heuristics(&graph, &Theme::modern(), &mut config, &nodes);

        assert_eq!(config.node_spacing, 180.0);
        assert_eq!(config.rank_spacing, 180.0);
    }

    #[test]
    fn tiny_graph_shortcut_excludes_directed_cycles() {
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Flowchart;
        for id in ["A", "B", "C"] {
            graph.nodes.insert(
                id.to_string(),
                crate::ir::Node {
                    id: id.to_string(),
                    label: id.to_string(),
                    shape: crate::ir::NodeShape::Rectangle,
                    value: None,
                    icon: None,
                },
            );
        }
        graph.edges.push(crate::ir::Edge {
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
            style: crate::ir::EdgeStyle::Solid,
        });
        graph.edges.push(crate::ir::Edge {
            from: "B".to_string(),
            to: "C".to_string(),
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
            style: crate::ir::EdgeStyle::Solid,
        });
        graph.edges.push(crate::ir::Edge {
            from: "C".to_string(),
            to: "A".to_string(),
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
            style: crate::ir::EdgeStyle::Solid,
        });

        assert!(!is_tiny_graph_layout(&graph));
    }
}
