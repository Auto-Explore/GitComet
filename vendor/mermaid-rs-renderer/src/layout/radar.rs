use std::collections::BTreeMap;

use crate::config::LayoutConfig;
use crate::ir::Graph;
use crate::theme::Theme;

use super::text::measure_label;
use super::{DiagramData, Layout, build_node_layout, resolve_node_style};

pub(super) fn compute_radar_layout(graph: &Graph, theme: &Theme, config: &LayoutConfig) -> Layout {
    const WIDTH: f32 = 680.0;
    const HEIGHT: f32 = 680.0;
    const CENTER_X: f32 = WIDTH / 2.0;
    const CENTER_Y: f32 = HEIGHT / 2.0;
    const MAX_RADIUS: f32 = 290.0;
    const LEGEND_BOX_SIZE: f32 = 11.0;
    const LEGEND_GAP: f32 = 3.0;

    let legend_offset = MAX_RADIUS * 0.875;
    let legend_base_x = CENTER_X + legend_offset;
    let legend_base_y = CENTER_Y - legend_offset;
    let legend_row_height = theme.font_size + 6.0;

    let mut node_ids: Vec<String> = graph.nodes.keys().cloned().collect();
    node_ids.sort_by(|a, b| {
        let order_a = graph.node_order.get(a).copied().unwrap_or(usize::MAX);
        let order_b = graph.node_order.get(b).copied().unwrap_or(usize::MAX);
        order_a.cmp(&order_b).then_with(|| a.cmp(b))
    });

    let mut nodes = BTreeMap::new();
    for (idx, node_id) in node_ids.iter().enumerate() {
        let Some(node) = graph.nodes.get(node_id) else {
            continue;
        };
        let label = measure_label(&node.label, theme, config);
        let width = LEGEND_BOX_SIZE + LEGEND_GAP + label.width;
        let height = label.height.max(LEGEND_BOX_SIZE);
        let mut style = resolve_node_style(node.id.as_str(), graph);
        if style.stroke.is_none() {
            style.stroke = Some("none".to_string());
        }
        if style.stroke_width.is_none() {
            style.stroke_width = Some(0.0);
        }
        let mut nl = build_node_layout(node, label, width, height, style, graph);
        nl.x = legend_base_x;
        nl.y = legend_base_y + idx as f32 * legend_row_height;
        nodes.insert(node.id.clone(), nl);
    }

    Layout {
        kind: graph.kind,
        nodes,
        edges: Vec::new(),
        subgraphs: Vec::new(),
        width: WIDTH,
        height: HEIGHT,
        diagram: DiagramData::Graph {
            state_notes: Vec::new(),
        },
    }
}
