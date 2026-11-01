use super::*;
use crate::ir::ArchDir;
use std::collections::VecDeque;

const MARGIN: f32 = 24.0;
const SERVICE_SIZE: f32 = 64.0;
const JUNCTION_SIZE: f32 = 18.0;
/// Horizontal grid pitch: icon size plus breathing room (mirrors mermaid's
/// `idealEdgeLengthMultiplier * iconSize` spacing).
const CELL_W: f32 = SERVICE_SIZE + 80.0;
/// Vertical grid pitch: extra room for the service label drawn under the
/// icon, plus enough clearance that two vertically adjacent cells in
/// *different* groups keep their group boxes (pad top + bottom) apart.
const CELL_H: f32 = SERVICE_SIZE + 112.0;
const GROUP_PAD_X: f32 = 28.0;
const GROUP_PAD_TOP: f32 = 32.0;
const GROUP_PAD_BOTTOM: f32 = 44.0;
const GROUP_GAP_Y: f32 = 48.0;
/// Space consumed by the text label below a service icon.
const LABEL_SPACE: f32 = 24.0;
const GROUP_STROKE: &str = "hsl(240, 60%, 86.2745098039%)";
const ICON_FILL: &str = "#087ebf";

/// Grid cell in the spatial map. Follows mermaid-js conventions: x grows to
/// the right, y grows *upward* (flipped to screen coordinates at placement).
type Cell = (i32, i32);

/// Mirror of mermaid-js `shiftPositionByArchitectureDirectionPair`: given the
/// placed node's position and the ports of the edge (`source` = port on the
/// placed node, `target` = port on the node being placed), compute where the
/// neighbor belongs in the spatial map.
fn shift_position(pos: Cell, source: ArchDir, target: ArchDir) -> Cell {
    let (x, y) = pos;
    match (source.is_horizontal(), target.is_horizontal()) {
        (true, false) => (
            x + if source == ArchDir::Left { -1 } else { 1 },
            y + if target == ArchDir::Top { 1 } else { -1 },
        ),
        (true, true) => (x + if source == ArchDir::Left { -1 } else { 1 }, y),
        (false, true) => (
            x + if target == ArchDir::Left { 1 } else { -1 },
            y + if source == ArchDir::Top { 1 } else { -1 },
        ),
        (false, false) => (x, y + if source == ArchDir::Top { 1 } else { -1 }),
    }
}

/// A port pair is invalid when both ends leave from the same side (LL, RR,
/// TT, BB), matching mermaid-js `isValidArchitectureDirectionPair`.
fn valid_port_pair(source: ArchDir, target: ArchDir) -> bool {
    source != target
}

fn arch_side(dir: ArchDir) -> EdgeSide {
    match dir {
        ArchDir::Left => EdgeSide::Left,
        ArchDir::Right => EdgeSide::Right,
        ArchDir::Top => EdgeSide::Top,
        ArchDir::Bottom => EdgeSide::Bottom,
    }
}

/// Insert an adjacency entry keyed by port pair. Mermaid keys its adjacency
/// list on the direction pair, so a later edge with the same pair replaces
/// the earlier one while keeping its position in iteration order.
fn insert_adjacency(
    adjacency: &mut HashMap<String, Vec<((ArchDir, ArchDir), String)>>,
    id: &str,
    pair: (ArchDir, ArchDir),
    neighbor: &str,
) {
    let entries = adjacency.entry(id.to_string()).or_default();
    if let Some(existing) = entries.iter_mut().find(|(key, _)| *key == pair) {
        existing.1 = neighbor.to_string();
    } else {
        entries.push((pair, neighbor.to_string()));
    }
}

/// Find the nearest free cell to `desired`, scanning outward by Manhattan
/// radius. Keeps placement deterministic when two nodes contend for the same
/// logical position (mermaid-js simply overlaps them; we separate instead).
fn find_free_cell(occupied: &HashSet<Cell>, desired: Cell) -> Cell {
    if !occupied.contains(&desired) {
        return desired;
    }
    for radius in 1i32..64 {
        for dy in -radius..=radius {
            let rem = radius - dy.abs();
            for dx in [-rem, rem] {
                let candidate = (desired.0 + dx, desired.1 + dy);
                if !occupied.contains(&candidate) {
                    return candidate;
                }
                if rem == 0 {
                    break;
                }
            }
        }
    }
    desired
}

/// Route an architecture edge orthogonally between two port anchors.
///
/// * Same-axis ports with an offset get a Z-shaped path (two bends).
/// * Mixed-axis ports get an L-shaped path (one 90-degree bend).
/// * Aligned same-axis ports stay straight, with a detour when another
///   service sits on the segment.
fn route_arch_edge(
    start: (f32, f32),
    end: (f32, f32),
    start_side: EdgeSide,
    end_side: EdgeSide,
    nodes: &BTreeMap<String, NodeLayout>,
    from_id: &str,
    to_id: &str,
) -> Vec<(f32, f32)> {
    const EPS: f32 = 1e-3;
    const DETOUR_GAP: f32 = 16.0;
    let start_horizontal = matches!(start_side, EdgeSide::Left | EdgeSide::Right);
    let end_horizontal = matches!(end_side, EdgeSide::Left | EdgeSide::Right);
    let mut points = vec![start];
    match (start_horizontal, end_horizontal) {
        (true, true) => {
            if (start.1 - end.1).abs() > EPS {
                // Z-shaped: out horizontally, jog vertically at the midpoint.
                let mid_x = (start.0 + end.0) / 2.0;
                points.push((mid_x, start.1));
                points.push((mid_x, end.1));
            } else {
                // Straight horizontal; detour around blocking services.
                let y = start.1;
                let seg_min_x = start.0.min(end.0);
                let seg_max_x = start.0.max(end.0);
                let mut block_top = f32::MAX;
                let mut block_bottom = f32::MIN;
                let mut has_blocker = false;
                for node in nodes.values() {
                    if node.id == from_id || node.id == to_id {
                        continue;
                    }
                    if y > node.y
                        && y < node.y + node.height
                        && seg_max_x > node.x
                        && seg_min_x < node.x + node.width
                    {
                        has_blocker = true;
                        block_top = block_top.min(node.y);
                        block_bottom = block_bottom.max(node.y + node.height);
                    }
                }
                if has_blocker {
                    let above = block_top - DETOUR_GAP;
                    let below = block_bottom + DETOUR_GAP;
                    let detour_y = if (y - above).abs() <= (below - y).abs() {
                        above
                    } else {
                        below
                    };
                    points.push((start.0, detour_y));
                    points.push((end.0, detour_y));
                }
            }
        }
        (false, false) => {
            if (start.0 - end.0).abs() > EPS {
                // Z-shaped: out vertically, jog horizontally at the midpoint.
                let mid_y = (start.1 + end.1) / 2.0;
                points.push((start.0, mid_y));
                points.push((end.0, mid_y));
            } else {
                // Straight vertical; detour around blocking services.
                let x = start.0;
                let seg_min_y = start.1.min(end.1);
                let seg_max_y = start.1.max(end.1);
                let mut block_left = f32::MAX;
                let mut block_right = f32::MIN;
                let mut has_blocker = false;
                for node in nodes.values() {
                    if node.id == from_id || node.id == to_id {
                        continue;
                    }
                    if x > node.x
                        && x < node.x + node.width
                        && seg_max_y > node.y
                        && seg_min_y < node.y + node.height
                    {
                        has_blocker = true;
                        block_left = block_left.min(node.x);
                        block_right = block_right.max(node.x + node.width);
                    }
                }
                if has_blocker {
                    let left = block_left - DETOUR_GAP;
                    let right = block_right + DETOUR_GAP;
                    let detour_x = if (x - left).abs() <= (right - x).abs() {
                        left
                    } else {
                        right
                    };
                    points.push((detour_x, start.1));
                    points.push((detour_x, end.1));
                }
            }
        }
        // Mixed ports: single 90-degree bend, leaving along the start port axis.
        (true, false) => points.push((end.0, start.1)),
        (false, true) => points.push((start.0, end.1)),
    }
    points.push(end);
    points
}

pub(super) fn compute_architecture_layout(
    graph: &Graph,
    theme: &Theme,
    config: &LayoutConfig,
) -> Layout {
    let mut nodes = BTreeMap::new();

    for node in graph.nodes.values() {
        let is_junction = node.icon.as_deref() == Some("junction")
            || (node.shape == crate::ir::NodeShape::Circle && node.label.trim().is_empty());
        let label = measure_label(&node.label, theme, config);
        let mut style = resolve_node_style(node.id.as_str(), graph);
        if style.fill.is_none() {
            style.fill = Some(if is_junction {
                theme.line_color.clone()
            } else {
                ICON_FILL.to_string()
            });
        }
        if style.stroke.is_none() {
            style.stroke = Some("none".to_string());
        }
        if style.stroke_width.is_none() {
            style.stroke_width = Some(0.0);
        }
        let size = if is_junction {
            JUNCTION_SIZE
        } else {
            SERVICE_SIZE
        };
        let mut nl = build_node_layout(node, label, size, size, style, graph);
        nl.shape = if is_junction {
            crate::ir::NodeShape::Circle
        } else {
            crate::ir::NodeShape::Rectangle
        };
        nl.icon = node.icon.clone();
        nodes.insert(node.id.clone(), nl);
    }

    // ── Spatial map (mermaid-js BFS grid) ────────────────────────────
    // Build a port-pair adjacency list from the edge declarations, then BFS
    // from the first declared node, placing each neighbor one cell away in
    // the direction implied by the ports (db:L -- R:server puts server one
    // cell to the *left* of db).
    let mut order: Vec<String> = nodes.keys().cloned().collect();
    order.sort_by(|a, b| {
        let order_a = graph.node_order.get(a).copied().unwrap_or(usize::MAX);
        let order_b = graph.node_order.get(b).copied().unwrap_or(usize::MAX);
        order_a.cmp(&order_b).then_with(|| a.cmp(b))
    });

    let mut adjacency: HashMap<String, Vec<((ArchDir, ArchDir), String)>> = HashMap::new();
    for (idx, edge) in graph.edges.iter().enumerate() {
        let Some(&(Some(from_port), Some(to_port))) = graph.arch_edge_ports.get(&idx) else {
            continue;
        };
        if !nodes.contains_key(&edge.from) || !nodes.contains_key(&edge.to) {
            continue;
        }
        if !valid_port_pair(from_port, to_port) {
            continue;
        }
        insert_adjacency(&mut adjacency, &edge.from, (from_port, to_port), &edge.to);
        insert_adjacency(&mut adjacency, &edge.to, (to_port, from_port), &edge.from);
    }

    let mut visited: HashSet<String> = HashSet::new();
    // Raw BFS components: nodes in placement order with their grid cell.
    let mut raw_components: Vec<Vec<(String, Cell)>> = Vec::new();
    for seed in &order {
        if visited.contains(seed) {
            continue;
        }
        let mut placed: HashMap<String, Cell> = HashMap::new();
        let mut occupied: HashSet<Cell> = HashSet::new();
        let mut component: Vec<(String, Cell)> = Vec::new();
        placed.insert(seed.clone(), (0, 0));
        occupied.insert((0, 0));
        visited.insert(seed.clone());
        component.push((seed.clone(), (0, 0)));
        let mut queue = VecDeque::from([seed.clone()]);
        while let Some(id) = queue.pop_front() {
            let pos = placed[&id];
            let Some(neighbors) = adjacency.get(&id) else {
                continue;
            };
            for ((source_dir, target_dir), neighbor) in neighbors {
                if visited.contains(neighbor) {
                    continue;
                }
                let desired = shift_position(pos, *source_dir, *target_dir);
                let cell = find_free_cell(&occupied, desired);
                placed.insert(neighbor.clone(), cell);
                occupied.insert(cell);
                visited.insert(neighbor.clone());
                component.push((neighbor.clone(), cell));
                queue.push_back(neighbor.clone());
            }
        }
        raw_components.push(component);
    }

    // ── Merge components that share a group ──────────────────────────
    // Mermaid-js groups are fcose compound nodes, which pull group members
    // of disconnected spatial maps together. Approximate that: if a later
    // component contains a member of a group that already has placed nodes,
    // anchor it below that group's cells instead of starting a new band.
    let node_group: HashMap<&str, usize> = graph
        .subgraphs
        .iter()
        .enumerate()
        .flat_map(|(idx, sub)| sub.nodes.iter().map(move |id| (id.as_str(), idx)))
        .collect();

    struct Canvas {
        cells: Vec<(String, Cell)>,
        occupied: HashSet<Cell>,
        groups: HashSet<usize>,
    }
    let mut canvases: Vec<Canvas> = Vec::new();
    for component in raw_components {
        let comp_groups: HashSet<usize> = component
            .iter()
            .filter_map(|(id, _)| node_group.get(id.as_str()).copied())
            .collect();
        let target = canvases
            .iter()
            .position(|canvas| !canvas.groups.is_disjoint(&comp_groups));
        match target {
            Some(idx) => {
                let canvas = &mut canvases[idx];
                // Anchor: place the component just below the shared group's
                // existing cells, keeping its internal structure.
                let shared: Vec<Cell> = canvas
                    .cells
                    .iter()
                    .filter(|(id, _)| {
                        node_group
                            .get(id.as_str())
                            .is_some_and(|g| comp_groups.contains(g))
                    })
                    .map(|(_, cell)| *cell)
                    .collect();
                let anchor_x = shared.iter().map(|c| c.0).min().unwrap_or(0);
                let anchor_y = shared.iter().map(|c| c.1).min().unwrap_or(0);
                let comp_min_x = component.iter().map(|(_, c)| c.0).min().unwrap_or(0);
                let comp_max_y = component.iter().map(|(_, c)| c.1).max().unwrap_or(0);
                let offset = (anchor_x - comp_min_x, anchor_y - 1 - comp_max_y);
                for (id, cell) in component {
                    let desired = (cell.0 + offset.0, cell.1 + offset.1);
                    let placed = find_free_cell(&canvas.occupied, desired);
                    canvas.occupied.insert(placed);
                    canvas.cells.push((id, placed));
                }
                canvas.groups.extend(comp_groups);
            }
            None => {
                let occupied = component.iter().map(|(_, cell)| *cell).collect();
                canvases.push(Canvas {
                    cells: component,
                    occupied,
                    groups: comp_groups,
                });
            }
        }
    }
    let components: Vec<Vec<(String, Cell)>> =
        canvases.into_iter().map(|canvas| canvas.cells).collect();

    // ── Grid -> pixel placement ──────────────────────────────────────
    // The spatial map is y-up (mermaid convention); flip to screen rows.
    // Disconnected components stack vertically.
    let origin_x = MARGIN + GROUP_PAD_X;
    let mut current_top = MARGIN;
    for component in &components {
        let min_x = component.iter().map(|(_, c)| c.0).min().unwrap_or(0);
        let max_y = component.iter().map(|(_, c)| c.1).max().unwrap_or(0);
        let base_y = current_top + GROUP_PAD_TOP;
        let mut comp_bottom = base_y;
        for (id, (gx, gy)) in component {
            let col = (gx - min_x) as f32;
            let row = (max_y - gy) as f32;
            if let Some(node) = nodes.get_mut(id) {
                node.x = origin_x + col * CELL_W + (SERVICE_SIZE - node.width) / 2.0;
                node.y = base_y + row * CELL_H + (SERVICE_SIZE - node.height) / 2.0;
                comp_bottom = comp_bottom.max(node.y + node.height + LABEL_SPACE);
            }
        }
        current_top = comp_bottom + GROUP_PAD_BOTTOM + GROUP_GAP_Y;
    }

    // ── Group boxes around their members ─────────────────────────────
    let mut subgraphs = Vec::new();
    for sub in &graph.subgraphs {
        let members: Vec<&NodeLayout> = sub
            .nodes
            .iter()
            .filter_map(|id| nodes.get(id.as_str()))
            .collect();
        if members.is_empty() {
            continue;
        }
        let min_x = members.iter().map(|n| n.x).fold(f32::MAX, f32::min);
        let min_y = members.iter().map(|n| n.y).fold(f32::MAX, f32::min);
        let max_x = members
            .iter()
            .map(|n| n.x + n.width)
            .fold(f32::MIN, f32::max);
        let max_y = members
            .iter()
            .map(|n| n.y + n.height + LABEL_SPACE)
            .fold(f32::MIN, f32::max);

        let label_block = measure_label(&sub.label, theme, config);
        let mut style = resolve_subgraph_style(sub, graph);
        style.fill = Some("none".to_string());
        style.stroke = Some(GROUP_STROKE.to_string());
        style.stroke_width = Some(2.0);
        style.stroke_dasharray = Some("8".to_string());
        if style.text_color.is_none() {
            style.text_color = Some(theme.primary_text_color.clone());
        }

        subgraphs.push(SubgraphLayout {
            label: sub.label.clone(),
            label_block,
            nodes: sub
                .nodes
                .iter()
                .filter(|id| nodes.contains_key(id.as_str()))
                .cloned()
                .collect(),
            x: min_x - GROUP_PAD_X,
            y: min_y - GROUP_PAD_TOP,
            width: max_x - min_x + GROUP_PAD_X * 2.0,
            height: max_y - min_y + GROUP_PAD_TOP + GROUP_PAD_BOTTOM,
            style,
            icon: sub.icon.clone(),
        });
    }

    // ── Port-aware edge routing ──────────────────────────────────────
    let mut edges = Vec::new();
    for (idx, edge) in graph.edges.iter().enumerate() {
        let Some(from) = nodes.get(&edge.from) else {
            continue;
        };
        let Some(to) = nodes.get(&edge.to) else {
            continue;
        };
        let (from_port, to_port) = graph
            .arch_edge_ports
            .get(&idx)
            .copied()
            .unwrap_or((None, None));
        let (fallback_start, fallback_end, _is_backward) = edge_sides(from, to, graph.direction);
        let start_side = from_port.map(arch_side).unwrap_or(fallback_start);
        let end_side = to_port.map(arch_side).unwrap_or(fallback_end);
        let start = anchor_point_for_node(from, start_side, 0.0);
        let end = anchor_point_for_node(to, end_side, 0.0);
        let points = route_arch_edge(
            start, end, start_side, end_side, &nodes, &edge.from, &edge.to,
        );
        let mut override_style = resolve_edge_style(idx, graph);
        if override_style.stroke.is_none() {
            override_style.stroke = Some(theme.line_color.clone());
        }
        override_style.stroke_width = Some(override_style.stroke_width.unwrap_or(3.0));

        edges.push(EdgeLayout {
            from: edge.from.clone(),
            to: edge.to.clone(),
            label: None,
            start_label: None,
            end_label: None,
            label_anchor: None,
            start_label_anchor: None,
            end_label_anchor: None,
            points: compress_path(&points),
            directed: edge.directed,
            arrow_start: edge.arrow_start,
            arrow_end: edge.arrow_end,
            arrow_start_kind: None,
            arrow_end_kind: None,
            start_decoration: None,
            end_decoration: None,
            style: edge.style,
            override_style,
        });
    }

    let (max_x, max_y) = bounds_with_edges(&nodes, &subgraphs, &edges);
    let width = (max_x + MARGIN).max(200.0);
    let height = (max_y + MARGIN).max(200.0);

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
