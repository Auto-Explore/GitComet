use super::*;

fn finite_chart_value(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

pub(super) fn compute_xychart_layout(
    graph: &Graph,
    theme: &Theme,
    config: &LayoutConfig,
) -> Layout {
    let data = &graph.xychart;
    let font_size = theme.font_size;
    let padding = 40.0;
    let y_axis_width = 60.0;
    let x_axis_height = 40.0;
    let title_height = if data.title.is_some() { 30.0 } else { 0.0 };

    let plot_width = 400.0;
    let plot_height = 250.0;

    let width = padding * 2.0 + y_axis_width + plot_width;
    let height = padding * 2.0 + title_height + plot_height + x_axis_height;

    let plot_x = padding + y_axis_width;
    let plot_y = padding + title_height;

    // Find min/max values
    let all_values: Vec<f32> = data
        .series
        .iter()
        .flat_map(|s| s.values.iter().copied().map(finite_chart_value))
        .collect();
    let min_val = data
        .y_axis_min
        .filter(|v| v.is_finite())
        .unwrap_or_else(|| all_values.iter().copied().fold(0.0_f32, f32::min).min(0.0));
    let max_val = data
        .y_axis_max
        .filter(|v| v.is_finite())
        .unwrap_or_else(|| all_values.iter().copied().fold(0.0_f32, f32::max));
    let range = (max_val - min_val).max(1.0);

    // Number of categories
    let num_categories = data
        .x_axis_categories
        .len()
        .max(
            data.series
                .iter()
                .map(|s| s.values.len())
                .max()
                .unwrap_or(0),
        )
        .max(1);

    let bar_group_width = plot_width / num_categories as f32;
    let bar_padding = bar_group_width * 0.1;

    // Count bar series for width calculation
    let bar_count = data
        .series
        .iter()
        .filter(|s| s.kind == crate::ir::XYSeriesKind::Bar)
        .count()
        .max(1);
    let bar_width = (bar_group_width - bar_padding * 2.0) / bar_count as f32;

    let colors = [
        "#4e79a7".to_string(),
        "#f28e2c".to_string(),
        "#e15759".to_string(),
        "#76b7b2".to_string(),
        "#59a14f".to_string(),
        "#edc949".to_string(),
        "#af7aa1".to_string(),
        "#ff9da7".to_string(),
    ];

    let mut bars = Vec::new();
    let mut lines = Vec::new();
    let mut bar_series_idx = 0;

    for (series_idx, series) in data.series.iter().enumerate() {
        let color = colors
            .get(series_idx % colors.len())
            .cloned()
            .unwrap_or_else(|| "#333".to_string());

        match series.kind {
            crate::ir::XYSeriesKind::Bar => {
                for (i, raw_value) in series.values.iter().copied().enumerate() {
                    let value = finite_chart_value(raw_value);
                    let bar_height = ((value - min_val) / range) * plot_height;
                    let x = plot_x
                        + i as f32 * bar_group_width
                        + bar_padding
                        + bar_series_idx as f32 * bar_width;
                    let y = plot_y + plot_height - bar_height;

                    bars.push(XYChartBarLayout {
                        x,
                        y,
                        width: bar_width,
                        height: bar_height,
                        value,
                        color: color.clone(),
                    });
                }
                bar_series_idx += 1;
            }
            crate::ir::XYSeriesKind::Line => {
                let points: Vec<(f32, f32)> = series
                    .values
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(i, raw_value)| {
                        let value = finite_chart_value(raw_value);
                        let x = plot_x + i as f32 * bar_group_width + bar_group_width / 2.0;
                        let y = plot_y + plot_height - ((value - min_val) / range) * plot_height;
                        (x, y)
                    })
                    .collect();

                lines.push(XYChartLineLayout { points, color });
            }
        }
    }

    // X-axis categories
    let x_axis_categories: Vec<(String, f32)> = data
        .x_axis_categories
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let x = plot_x + i as f32 * bar_group_width + bar_group_width / 2.0;
            (cat.clone(), x)
        })
        .collect();

    // Y-axis ticks
    let num_ticks = 5;
    let y_axis_ticks: Vec<(String, f32)> = (0..=num_ticks)
        .map(|i| {
            let value = min_val + (i as f32 / num_ticks as f32) * range;
            let y = plot_y + plot_height - (i as f32 / num_ticks as f32) * plot_height;
            (format!("{:.0}", value), y)
        })
        .collect();

    let title = data.title.as_ref().map(|t| measure_label(t, theme, config));
    let x_axis_label = data
        .x_axis_label
        .as_ref()
        .map(|l| measure_label(l, theme, config));
    let y_axis_label = data
        .y_axis_label
        .as_ref()
        .map(|l| measure_label(l, theme, config));

    Layout {
        kind: graph.kind,
        nodes: BTreeMap::new(),
        edges: Vec::new(),
        subgraphs: Vec::new(),
        diagram: DiagramData::XYChart(XYChartLayout {
            title,
            title_y: padding + font_size,
            x_axis_label,
            x_axis_label_y: plot_y + plot_height + x_axis_height - 10.0,
            y_axis_label,
            y_axis_label_x: padding,
            x_axis_categories,
            y_axis_ticks,
            bars,
            lines,
            plot_x,
            plot_y,
            plot_width,
            plot_height,
            width,
            height,
        }),
        width,
        height,
    }
}
