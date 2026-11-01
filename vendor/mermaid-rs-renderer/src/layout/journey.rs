use super::*;

fn parse_journey_task_label(label: &str) -> (String, Vec<String>) {
    let mut lines = split_lines(label);
    if lines.is_empty() {
        return (String::new(), Vec::new());
    }
    let title = lines.remove(0).trim().to_string();
    let mut actors = Vec::new();
    for line in lines {
        for part in line.split(',') {
            let actor = part.trim();
            if !actor.is_empty() {
                actors.push(actor.to_string());
            }
        }
    }
    (title, actors)
}

fn journey_score_color(score: f32) -> String {
    let clamped = score.clamp(1.0, 5.0);
    let t = (clamped - 1.0) / 4.0;
    let start = (248.0, 113.0, 113.0);
    let end = (74.0, 222.0, 128.0);
    let r = (start.0 + (end.0 - start.0) * t).round() as i32;
    let g = (start.1 + (end.1 - start.1) * t).round() as i32;
    let b = (start.2 + (end.2 - start.2) * t).round() as i32;
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

pub(super) fn compute_journey_layout(
    graph: &Graph,
    theme: &Theme,
    config: &LayoutConfig,
) -> Layout {
    let mut section_defs: Vec<(String, Vec<String>)> = Vec::new();
    let mut assigned: HashSet<String> = HashSet::new();
    if graph.subgraphs.is_empty() {
        let mut ordered: Vec<String> = graph.nodes.keys().cloned().collect();
        ordered.sort_by_key(|id| graph.node_order.get(id).copied().unwrap_or(usize::MAX));
        section_defs.push((String::new(), ordered));
    } else {
        for sub in &graph.subgraphs {
            let mut nodes = Vec::new();
            for id in &sub.nodes {
                if graph.nodes.contains_key(id) {
                    nodes.push(id.clone());
                    assigned.insert(id.clone());
                }
            }
            section_defs.push((sub.label.clone(), nodes));
        }
        let mut extras: Vec<String> = graph
            .nodes
            .keys()
            .filter(|id| !assigned.contains(*id))
            .cloned()
            .collect();
        if !extras.is_empty() {
            extras.sort_by_key(|id| graph.node_order.get(id).copied().unwrap_or(usize::MAX));
            section_defs.push(("Other".to_string(), extras));
        }
    }

    struct TaskData {
        id: String,
        label: TextBlock,
        score: Option<f32>,
        actors: Vec<String>,
        section_idx: usize,
        order_idx: usize,
    }

    let mut tasks_data: Vec<TaskData> = Vec::new();
    let mut section_ranges: Vec<(usize, usize)> = Vec::new();
    let mut order_idx = 0usize;
    for (section_idx, (_label, nodes)) in section_defs.iter().enumerate() {
        let start_idx = order_idx;
        for node_id in nodes {
            if let Some(node) = graph.nodes.get(node_id) {
                let (title, actors) = parse_journey_task_label(&node.label);
                let title_text = if title.is_empty() {
                    node.label.clone()
                } else {
                    title
                };
                let label = measure_label(&title_text, theme, config);
                tasks_data.push(TaskData {
                    id: node_id.clone(),
                    label,
                    score: node.value,
                    actors,
                    section_idx,
                    order_idx,
                });
                order_idx += 1;
            }
        }
        let end_idx = order_idx.saturating_sub(1);
        section_ranges.push((start_idx, end_idx));
    }

    let mut actor_order: Vec<String> = Vec::new();
    let mut actor_set: HashSet<String> = HashSet::new();
    for task in &tasks_data {
        for actor in &task.actors {
            if actor_set.insert(actor.clone()) {
                actor_order.push(actor.clone());
            }
        }
    }

    let mut max_label_w = theme.font_size * 4.0;
    let mut max_label_h = theme.font_size * config.label_line_height;
    for task in &tasks_data {
        max_label_w = max_label_w.max(task.label.width);
        max_label_h = max_label_h.max(task.label.height);
    }

    let margin_x = theme.font_size * 2.0;
    let margin_y = theme.font_size * 2.0;
    let task_gap_x = theme.font_size * 1.6;
    let section_gap_y = theme.font_size * 1.8;
    let header_height = theme.font_size * 1.6;
    let card_gap_y = theme.font_size * 0.6;
    let score_radius = (theme.font_size * 0.55).max(6.0);
    let actor_radius = (theme.font_size * 0.35).max(4.0);
    let actor_gap = theme.font_size * 0.5;
    let task_pad_x = theme.font_size * 0.9;
    let task_pad_y = theme.font_size * 0.6;

    let task_width = (max_label_w + task_pad_x * 2.0).max(theme.font_size * 6.0);
    let task_height = (max_label_h + task_pad_y * 2.0).max(theme.font_size * 2.4);

    let title_block = graph
        .journey_title
        .as_ref()
        .map(|title| measure_label(title, theme, config));
    let mut cursor_y = margin_y;
    let title_y = if let Some(ref title) = title_block {
        let y = cursor_y + title.height / 2.0;
        cursor_y += title.height + theme.font_size * 0.6;
        y
    } else {
        0.0
    };

    let mut actors = Vec::new();
    let mut actor_label_y = 0.0;
    if !actor_order.is_empty() {
        let mut x = margin_x;
        let legend_y = cursor_y + actor_radius;
        actor_label_y = legend_y + theme.font_size * 0.35;
        for (idx, actor) in actor_order.iter().enumerate() {
            let label = measure_label(actor, theme, config);
            let color = theme.git_colors[idx % theme.git_colors.len()].clone();
            actors.push(JourneyActorLayout {
                name: actor.clone(),
                color: color.clone(),
                x: x + actor_radius,
                y: legend_y,
                radius: actor_radius,
            });
            x += actor_radius * 2.0 + actor_gap + label.width + theme.font_size * 0.8;
        }
        cursor_y += actor_radius * 2.0 + theme.font_size * 0.8;
    }

    let content_y = cursor_y;
    let has_actor_rows = tasks_data.iter().any(|task| !task.actors.is_empty());
    let actor_row_height = if has_actor_rows {
        actor_radius * 2.0
    } else {
        0.0
    };
    let actor_row_gap = if has_actor_rows {
        theme.font_size * 0.4
    } else {
        0.0
    };
    let row_height = header_height
        + score_radius * 2.0
        + card_gap_y
        + task_height
        + actor_row_gap
        + actor_row_height
        + theme.font_size * 0.6;

    let content_x = margin_x;
    let total_tasks = tasks_data.len();
    let task_area_width = if total_tasks > 0 {
        total_tasks as f32 * task_width + (total_tasks.saturating_sub(1)) as f32 * task_gap_x
    } else {
        0.0
    };

    let mut tasks = Vec::new();
    for task in &tasks_data {
        let row_top = content_y + task.section_idx as f32 * (row_height + section_gap_y);
        let card_y = row_top + header_height + score_radius * 2.0 + card_gap_y;
        let score_y = row_top + header_height + score_radius;
        let actor_y = if has_actor_rows {
            Some(card_y + task_height + actor_row_gap + actor_radius)
        } else {
            None
        };
        let x = content_x + task.order_idx as f32 * (task_width + task_gap_x);
        let score_color = task
            .score
            .map(journey_score_color)
            .unwrap_or_else(|| theme.secondary_color.clone());
        tasks.push(JourneyTaskLayout {
            id: task.id.clone(),
            label: task.label.clone(),
            x,
            y: card_y,
            width: task_width,
            height: task_height,
            score: task.score,
            score_color,
            score_y,
            actors: task.actors.clone(),
            actor_y,
            section_idx: task.section_idx,
        });
    }

    let mut sections = Vec::new();
    let section_pad_x = theme.font_size * 0.6;
    for (section_idx, (label, _nodes)) in section_defs.iter().enumerate() {
        let (start_idx, end_idx) = section_ranges.get(section_idx).copied().unwrap_or((0, 0));
        if start_idx > end_idx || total_tasks == 0 {
            continue;
        }
        let row_top = content_y + section_idx as f32 * (row_height + section_gap_y);
        let x = content_x + start_idx as f32 * (task_width + task_gap_x) - section_pad_x;
        let span = end_idx.saturating_sub(start_idx) + 1;
        let width = span as f32 * task_width
            + (span.saturating_sub(1)) as f32 * task_gap_x
            + section_pad_x * 2.0;
        let label_block = measure_label(label, theme, config);
        let color = theme.git_colors[section_idx % theme.git_colors.len()].clone();
        sections.push(JourneySectionLayout {
            label: label_block,
            x,
            y: row_top,
            width,
            height: header_height,
            color,
        });
    }

    let baseline = if total_tasks > 0 {
        let rows = section_defs.len();
        let total_rows_height = if rows > 0 {
            rows as f32 * row_height + (rows.saturating_sub(1)) as f32 * section_gap_y
        } else {
            0.0
        };
        let y = content_y + total_rows_height + theme.font_size * 0.5;
        Some((content_x, y, content_x + task_area_width))
    } else {
        None
    };

    let width = (content_x + task_area_width + margin_x).max(1.0);
    let height = baseline
        .map(|(_, y, _)| y + theme.font_size * 1.6)
        .unwrap_or(content_y + theme.font_size * 4.0)
        .max(1.0);

    let mut nodes = BTreeMap::new();
    nodes.insert(
        "__journey_metrics_content".to_string(),
        NodeLayout {
            id: "__journey_metrics_content".to_string(),
            x: margin_x,
            y: margin_y,
            width: (width - margin_x * 2.0).max(1.0),
            height: (height - margin_y * 2.0).max(1.0),
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
        },
    );

    Layout {
        kind: graph.kind,
        nodes,
        edges: Vec::new(),
        subgraphs: Vec::new(),
        diagram: DiagramData::Journey(JourneyLayout {
            title: title_block,
            title_y,
            actors,
            actor_label_y,
            tasks,
            sections,
            baseline,
            score_radius,
            actor_radius,
            actor_gap,
            card_gap_y,
            width,
            height,
        }),
        width,
        height,
    }
}
