use super::*;

pub(super) fn compute_treemap_layout(
    graph: &Graph,
    theme: &Theme,
    config: &LayoutConfig,
) -> Layout {
    let mut nodes: BTreeMap<String, NodeLayout> = BTreeMap::new();
    let edges = Vec::new();
    let subgraphs = Vec::new();

    let width = config.treemap.width.max(1.0);
    let height = config.treemap.height.max(1.0);
    let root_rect = TreemapRect::new(0.0, 0.0, width, height);

    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut parents: HashMap<String, String> = HashMap::new();
    for edge in &graph.edges {
        children
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
        parents.insert(edge.to.clone(), edge.from.clone());
    }
    for list in children.values_mut() {
        list.sort_by_key(|id| graph.node_order.get(id).copied().unwrap_or(usize::MAX));
    }

    let mut roots: Vec<String> = graph
        .nodes
        .keys()
        .filter(|id| !parents.contains_key(*id))
        .cloned()
        .collect();
    roots.sort_by_key(|id| graph.node_order.get(id).copied().unwrap_or(usize::MAX));

    let mut weight_cache: HashMap<String, f32> = HashMap::new();
    if !roots.is_empty() {
        layout_treemap_nodes(
            &roots,
            root_rect,
            0,
            graph,
            &children,
            &mut weight_cache,
            &mut nodes,
            theme,
            config,
        );
    }

    Layout {
        kind: graph.kind,
        nodes,
        edges,
        subgraphs,
        width,
        height,
        diagram: DiagramData::Graph {
            state_notes: Vec::new(),
        },
    }
}

#[derive(Debug, Clone, Copy)]
struct TreemapRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl TreemapRect {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn inset(self, padding: f32) -> Self {
        let pad = padding.max(0.0);
        let w = (self.w - pad * 2.0).max(0.0);
        let h = (self.h - pad * 2.0).max(0.0);
        Self {
            x: self.x + pad,
            y: self.y + pad,
            w,
            h,
        }
    }
}

fn layout_treemap_nodes(
    ids: &[String],
    rect: TreemapRect,
    depth: usize,
    graph: &Graph,
    children: &HashMap<String, Vec<String>>,
    weight_cache: &mut HashMap<String, f32>,
    nodes_out: &mut BTreeMap<String, NodeLayout>,
    theme: &Theme,
    config: &LayoutConfig,
) {
    if ids.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let total_weight: f32 = ids
        .iter()
        .map(|id| treemap_weight(id, graph, children, weight_cache))
        .sum();
    if total_weight <= 0.0 {
        return;
    }
    let gap = config.treemap.gap.max(0.0);
    let horizontal = depth.is_multiple_of(2);
    let count = ids.len();
    let available = if horizontal {
        (rect.w - gap * (count.saturating_sub(1) as f32)).max(0.0)
    } else {
        (rect.h - gap * (count.saturating_sub(1) as f32)).max(0.0)
    };

    let mut offset = 0.0;
    for id in ids {
        let weight = treemap_weight(id, graph, children, weight_cache);
        let ratio = (weight / total_weight).max(0.0);
        let span = available * ratio;
        let node_rect = if horizontal {
            let x = rect.x + offset;
            offset += span + gap;
            TreemapRect::new(x, rect.y, span, rect.h)
        } else {
            let y = rect.y + offset;
            offset += span + gap;
            TreemapRect::new(rect.x, y, rect.w, span)
        };

        let mut child_header_reserve = 0.0_f32;
        if let Some(node) = graph.nodes.get(id) {
            let mut style = resolve_node_style(id, graph);
            if style.fill.is_none() {
                style.fill = Some(treemap_depth_color(depth, theme));
            }
            if style.stroke.is_none() {
                style.stroke = Some(theme.primary_border_color.clone());
            }
            if style.stroke_width.is_none() {
                style.stroke_width = Some(1.0);
            }
            if style.text_color.is_none() {
                style.text_color = Some(theme.primary_text_color.clone());
            }

            let label = measure_label(&node.label, theme, config);
            let pad_x = config.treemap.label_padding_x;
            let pad_y = config.treemap.label_padding_y;
            let fits = label.width <= (node_rect.w - pad_x * 2.0).max(0.0)
                && label.height <= (node_rect.h - pad_y * 2.0).max(0.0);
            let area = node_rect.w * node_rect.h;
            let label = if fits && area >= config.treemap.min_label_area {
                child_header_reserve = (label.height + pad_y * 2.0).max(0.0);
                label
            } else {
                TextBlock {
                    lines: vec![String::new()],
                    width: 0.0,
                    height: 0.0,
                }
            };

            nodes_out.insert(
                id.clone(),
                NodeLayout {
                    id: node.id.clone(),
                    x: node_rect.x,
                    y: node_rect.y,
                    width: node_rect.w,
                    height: node_rect.h,
                    label,
                    shape: crate::ir::NodeShape::Rectangle,
                    style,
                    link: graph.node_links.get(id).cloned(),
                    anchor_subgraph: None,
                    hidden: false,
                    icon: None,
                },
            );
        }

        if let Some(children_ids) = children.get(id) {
            let mut child_rect = node_rect.inset(config.treemap.padding);
            if child_header_reserve > 0.0 {
                let reserve = child_header_reserve.min(child_rect.h * 0.35);
                child_rect.y += reserve;
                child_rect.h = (child_rect.h - reserve).max(0.0);
            }
            if child_rect.w > 1.0 && child_rect.h > 1.0 {
                layout_treemap_nodes(
                    children_ids,
                    child_rect,
                    depth + 1,
                    graph,
                    children,
                    weight_cache,
                    nodes_out,
                    theme,
                    config,
                );
            }
        }
    }
}

fn treemap_weight(
    id: &str,
    graph: &Graph,
    children: &HashMap<String, Vec<String>>,
    cache: &mut HashMap<String, f32>,
) -> f32 {
    if let Some(value) = cache.get(id) {
        return *value;
    }
    let mut weight = graph
        .nodes
        .get(id)
        .and_then(|node| node.value)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    if weight <= 0.0
        && let Some(child_ids) = children.get(id)
    {
        weight = child_ids
            .iter()
            .map(|child| treemap_weight(child, graph, children, cache))
            .sum();
    }
    if weight <= 0.0 {
        weight = 1.0;
    }
    cache.insert(id.to_string(), weight);
    weight
}

fn treemap_depth_color(depth: usize, theme: &Theme) -> String {
    match depth % 3 {
        0 => theme.primary_color.clone(),
        1 => theme.secondary_color.clone(),
        _ => theme.tertiary_color.clone(),
    }
}
