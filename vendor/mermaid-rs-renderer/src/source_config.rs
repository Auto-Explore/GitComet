//! Source configuration merger shared with the CLI.
use crate::config::Config;
fn parse_aspect_ratio_value(raw: &str) -> Result<f32, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("aspect ratio cannot be empty".to_string());
    }
    let parse_pair = |parts: (&str, &str)| -> Result<f32, String> {
        let w = parts
            .0
            .trim()
            .parse::<f32>()
            .map_err(|_| "invalid ratio width".to_string())?;
        let h = parts
            .1
            .trim()
            .parse::<f32>()
            .map_err(|_| "invalid ratio height".to_string())?;
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return Err("ratio values must be finite and > 0".to_string());
        }
        Ok(w / h)
    };

    if let Some((w, h)) = value.split_once(':') {
        return parse_pair((w, h));
    }
    if let Some((w, h)) = value.split_once('/') {
        return parse_pair((w, h));
    }

    let ratio = value
        .parse::<f32>()
        .map_err(|_| "invalid aspect ratio".to_string())?;
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err("ratio must be finite and > 0".to_string());
    }
    Ok(ratio)
}

fn parse_aspect_ratio_json(value: &serde_json::Value) -> Option<f32> {
    match value {
        serde_json::Value::Number(num) => num
            .as_f64()
            .map(|val| val as f32)
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0),
        serde_json::Value::String(text) => parse_aspect_ratio_value(text).ok(),
        serde_json::Value::Object(map) => {
            let width = map.get("width").and_then(|v| v.as_f64())? as f32;
            let height = map.get("height").and_then(|v| v.as_f64())? as f32;
            if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
                Some(width / height)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn merge_init_config(mut config: Config, init: serde_json::Value) -> Config {
    if let Some(theme_name) = init.get("theme").and_then(|v| v.as_str())
        && let Some(theme) = crate::theme::Theme::from_name(theme_name)
    {
        config.theme = theme;
    }
    if let Some(theme_vars) = init.get("themeVariables") {
        let tag_label_border_explicit = theme_vars
            .get("tagLabelBorder")
            .and_then(|v| v.as_str())
            .is_some();
        let primary_border_override = theme_vars
            .get("primaryBorderColor")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        if let Some(val) = theme_vars.get("primaryColor").and_then(|v| v.as_str()) {
            config.theme.primary_color = val.to_string();
        }
        if let Some(val) = theme_vars.get("primaryTextColor").and_then(|v| v.as_str()) {
            config.theme.primary_text_color = val.to_string();
        }
        if let Some(val) = theme_vars
            .get("primaryBorderColor")
            .and_then(|v| v.as_str())
        {
            config.theme.primary_border_color = val.to_string();
        }
        if let Some(val) = theme_vars.get("lineColor").and_then(|v| v.as_str()) {
            config.theme.line_color = val.to_string();
        }
        if let Some(val) = theme_vars.get("secondaryColor").and_then(|v| v.as_str()) {
            config.theme.secondary_color = val.to_string();
        }
        if let Some(val) = theme_vars.get("tertiaryColor").and_then(|v| v.as_str()) {
            config.theme.tertiary_color = val.to_string();
        }
        if let Some(val) = theme_vars.get("textColor").and_then(|v| v.as_str()) {
            config.theme.text_color = val.to_string();
        }
        if let Some(val) = theme_vars
            .get("edgeLabelBackground")
            .and_then(|v| v.as_str())
        {
            config.theme.edge_label_background = val.to_string();
        }
        if let Some(val) = theme_vars.get("clusterBkg").and_then(|v| v.as_str()) {
            config.theme.cluster_background = val.to_string();
        }
        if let Some(val) = theme_vars.get("clusterBorder").and_then(|v| v.as_str()) {
            config.theme.cluster_border = val.to_string();
        }
        if let Some(val) = theme_vars.get("background").and_then(|v| v.as_str()) {
            config.theme.background = val.to_string();
        }
        if let Some(val) = theme_vars.get("actorBkg").and_then(|v| v.as_str()) {
            config.theme.sequence_actor_fill = val.to_string();
        }
        if let Some(val) = theme_vars.get("actorBorder").and_then(|v| v.as_str()) {
            config.theme.sequence_actor_border = val.to_string();
        }
        if let Some(val) = theme_vars.get("actorLine").and_then(|v| v.as_str()) {
            config.theme.sequence_actor_line = val.to_string();
        }
        if let Some(val) = theme_vars.get("noteBkg").and_then(|v| v.as_str()) {
            config.theme.sequence_note_fill = val.to_string();
        }
        if let Some(val) = theme_vars.get("noteBorderColor").and_then(|v| v.as_str()) {
            config.theme.sequence_note_border = val.to_string();
        }
        if let Some(val) = theme_vars
            .get("activationBkgColor")
            .and_then(|v| v.as_str())
        {
            config.theme.sequence_activation_fill = val.to_string();
        }
        if let Some(val) = theme_vars
            .get("activationBorderColor")
            .and_then(|v| v.as_str())
        {
            config.theme.sequence_activation_border = val.to_string();
        }
        if let Some(val) = theme_vars.get("git0").and_then(|v| v.as_str()) {
            config.theme.git_colors[0] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git1").and_then(|v| v.as_str()) {
            config.theme.git_colors[1] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git2").and_then(|v| v.as_str()) {
            config.theme.git_colors[2] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git3").and_then(|v| v.as_str()) {
            config.theme.git_colors[3] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git4").and_then(|v| v.as_str()) {
            config.theme.git_colors[4] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git5").and_then(|v| v.as_str()) {
            config.theme.git_colors[5] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git6").and_then(|v| v.as_str()) {
            config.theme.git_colors[6] = val.to_string();
        }
        if let Some(val) = theme_vars.get("git7").and_then(|v| v.as_str()) {
            config.theme.git_colors[7] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv0").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[0] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv1").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[1] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv2").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[2] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv3").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[3] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv4").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[4] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv5").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[5] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv6").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[6] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitInv7").and_then(|v| v.as_str()) {
            config.theme.git_inv_colors[7] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel0").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[0] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel1").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[1] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel2").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[2] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel3").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[3] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel4").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[4] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel5").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[5] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel6").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[6] = val.to_string();
        }
        if let Some(val) = theme_vars.get("gitBranchLabel7").and_then(|v| v.as_str()) {
            config.theme.git_branch_label_colors[7] = val.to_string();
        }
        if let Some(val) = theme_vars.get("commitLabelColor").and_then(|v| v.as_str()) {
            config.theme.git_commit_label_color = val.to_string();
        }
        if let Some(val) = theme_vars
            .get("commitLabelBackground")
            .and_then(|v| v.as_str())
        {
            config.theme.git_commit_label_background = val.to_string();
        }
        if let Some(val) = theme_vars.get("tagLabelColor").and_then(|v| v.as_str()) {
            config.theme.git_tag_label_color = val.to_string();
        }
        if let Some(val) = theme_vars
            .get("tagLabelBackground")
            .and_then(|v| v.as_str())
        {
            config.theme.git_tag_label_background = val.to_string();
        }
        if let Some(val) = theme_vars.get("tagLabelBorder").and_then(|v| v.as_str()) {
            config.theme.git_tag_label_border = val.to_string();
        }
        if !tag_label_border_explicit && primary_border_override.is_some() {
            config.theme.git_tag_label_border = config.theme.primary_border_color.clone();
        }
        if let Some(val) = theme_vars.get("fontFamily").and_then(|v| v.as_str()) {
            config.theme.font_family = val.to_string();
        }
        if let Some(val) = theme_vars.get("fontSize").and_then(|v| v.as_f64()) {
            config.theme.font_size = val as f32;
        }
    }
    if let Some(ratio) = init
        .get("preferredAspectRatio")
        .and_then(parse_aspect_ratio_json)
    {
        config.layout.preferred_aspect_ratio = Some(ratio);
    }
    if let Some(flowchart) = init.get("flowchart") {
        if let Some(val) = flowchart.get("nodeSpacing").and_then(|v| v.as_f64()) {
            config.layout.node_spacing = val as f32;
        }
        if let Some(val) = flowchart.get("rankSpacing").and_then(|v| v.as_f64()) {
            config.layout.rank_spacing = val as f32;
        }
        if let Some(val) = flowchart.get("orderPasses").and_then(|v| v.as_u64()) {
            config.layout.flowchart.order_passes = val as usize;
        }
        if let Some(val) = flowchart.get("portPadRatio").and_then(|v| v.as_f64()) {
            config.layout.flowchart.port_pad_ratio = val as f32;
        }
        if let Some(val) = flowchart.get("portPadMin").and_then(|v| v.as_f64()) {
            config.layout.flowchart.port_pad_min = val as f32;
        }
        if let Some(val) = flowchart.get("portPadMax").and_then(|v| v.as_f64()) {
            config.layout.flowchart.port_pad_max = val as f32;
        }
        if let Some(val) = flowchart.get("portSideBias").and_then(|v| v.as_f64()) {
            config.layout.flowchart.port_side_bias = val as f32;
        }
    }
    if let Some(direction) = init
        .get("timeline")
        .and_then(|timeline| timeline.get("defaultDirection"))
        .and_then(|v| v.as_str())
    {
        config.layout.timeline.direction = direction.to_ascii_uppercase();
    }
    if let Some(gitgraph) = init.get("gitGraph") {
        let mut commit_step_set = false;
        if let Some(val) = gitgraph.get("diagramPadding").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.diagram_padding = val as f32;
        }
        if let Some(val) = gitgraph.get("titleTopMargin").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.title_top_margin = val as f32;
        }
        if let Some(val) = gitgraph.get("useMaxWidth").and_then(|v| v.as_bool()) {
            config.layout.gitgraph.use_max_width = val;
        }
        if let Some(val) = gitgraph.get("mainBranchName").and_then(|v| v.as_str()) {
            config.layout.gitgraph.main_branch_name = val.to_string();
        }
        if let Some(val) = gitgraph.get("mainBranchOrder").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.main_branch_order = val as f32;
        }
        if let Some(val) = gitgraph.get("showCommitLabel").and_then(|v| v.as_bool()) {
            config.layout.gitgraph.show_commit_label = val;
        }
        if let Some(val) = gitgraph.get("showBranches").and_then(|v| v.as_bool()) {
            config.layout.gitgraph.show_branches = val;
        }
        if let Some(val) = gitgraph.get("rotateCommitLabel").and_then(|v| v.as_bool()) {
            config.layout.gitgraph.rotate_commit_label = val;
        }
        if let Some(val) = gitgraph.get("parallelCommits").and_then(|v| v.as_bool()) {
            config.layout.gitgraph.parallel_commits = val;
        }
        if let Some(val) = gitgraph.get("commitStep").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.commit_step = val as f32;
            commit_step_set = true;
        }
        if let Some(val) = gitgraph.get("layoutOffset").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.layout_offset = val as f32;
        }
        if let Some(val) = gitgraph.get("defaultPos").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.default_pos = val as f32;
        }
        if let Some(val) = gitgraph.get("branchSpacing").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.branch_spacing = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchSpacingRotateExtra")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_spacing_rotate_extra = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelRotateExtra")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_rotate_extra = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelTranslateX")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_translate_x = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelBgOffsetX")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_bg_offset_x = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelBgOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_bg_offset_y = val as f32;
        }
        if let Some(val) = gitgraph.get("branchLabelBgPadX").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.branch_label_bg_pad_x = val as f32;
        }
        if let Some(val) = gitgraph.get("branchLabelBgPadY").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.branch_label_bg_pad_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelTextOffsetX")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_text_offset_x = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelTextOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_text_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelTbBgOffsetX")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_tb_bg_offset_x = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelTbTextOffsetX")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_tb_text_offset_x = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelTbOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_tb_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelBtOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_bt_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelCornerRadius")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_corner_radius = val as f32;
        }
        if let Some(val) = gitgraph.get("branchLabelFontSize").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.branch_label_font_size = val as f32;
        }
        if let Some(val) = gitgraph
            .get("branchLabelLineHeight")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.branch_label_line_height = val as f32;
        }
        if let Some(val) = gitgraph.get("textWidthScale").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.text_width_scale = val as f32;
        }
        if let Some(val) = gitgraph.get("commitLabelFontSize").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.commit_label_font_size = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelLineHeight")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_line_height = val as f32;
        }
        if let Some(val) = gitgraph.get("commitLabelOffsetY").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.commit_label_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelBgOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_bg_offset_y = val as f32;
        }
        if let Some(val) = gitgraph.get("commitLabelPadding").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.commit_label_padding = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelBgOpacity")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_bg_opacity = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelRotateAngle")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_rotate_angle = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelRotateTranslateXBase")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_rotate_translate_x_base = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelRotateTranslateXScale")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_rotate_translate_x_scale = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelRotateTranslateXWidthOffset")
            .and_then(|v| v.as_f64())
        {
            config
                .layout
                .gitgraph
                .commit_label_rotate_translate_x_width_offset = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelRotateTranslateYBase")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_rotate_translate_y_base = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelRotateTranslateYScale")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_rotate_translate_y_scale = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelTbTextExtra")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_tb_text_extra = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelTbBgExtra")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_tb_bg_extra = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelTbTextOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_tb_text_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("commitLabelTbBgOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.commit_label_tb_bg_offset_y = val as f32;
        }
        if let Some(val) = gitgraph.get("tagLabelFontSize").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_label_font_size = val as f32;
        }
        if let Some(val) = gitgraph.get("tagLabelLineHeight").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_label_line_height = val as f32;
        }
        if let Some(val) = gitgraph.get("tagTextOffsetY").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_text_offset_y = val as f32;
        }
        if let Some(val) = gitgraph.get("tagPolygonOffsetY").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_polygon_offset_y = val as f32;
        }
        if let Some(val) = gitgraph.get("tagSpacingY").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_spacing_y = val as f32;
        }
        if let Some(val) = gitgraph.get("tagPaddingX").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_padding_x = val as f32;
        }
        if let Some(val) = gitgraph.get("tagPaddingY").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_padding_y = val as f32;
        }
        if let Some(val) = gitgraph.get("tagHoleRadius").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_hole_radius = val as f32;
        }
        if let Some(val) = gitgraph.get("tagRotateTranslate").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_rotate_translate = val as f32;
        }
        if let Some(val) = gitgraph
            .get("tagTextRotateTranslate")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.tag_text_rotate_translate = val as f32;
        }
        if let Some(val) = gitgraph.get("tagRotateAngle").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_rotate_angle = val as f32;
        }
        if let Some(val) = gitgraph.get("tagTextOffsetXTb").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_text_offset_x_tb = val as f32;
        }
        if let Some(val) = gitgraph.get("tagTextOffsetYTb").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.tag_text_offset_y_tb = val as f32;
        }
        if let Some(val) = gitgraph.get("arrowRerouteRadius").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.arrow_reroute_radius = val as f32;
        }
        if let Some(val) = gitgraph.get("arrowRadius").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.arrow_radius = val as f32;
        }
        if let Some(val) = gitgraph.get("laneSpacing").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.lane_spacing = val as f32;
        }
        if let Some(val) = gitgraph.get("laneMaxDepth").and_then(|v| v.as_u64()) {
            config.layout.gitgraph.lane_max_depth = val as usize;
        }
        if let Some(val) = gitgraph.get("commitRadius").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.commit_radius = val as f32;
        }
        if let Some(val) = gitgraph.get("mergeRadiusOuter").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.merge_radius_outer = val as f32;
        }
        if let Some(val) = gitgraph.get("mergeRadiusInner").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.merge_radius_inner = val as f32;
        }
        if let Some(val) = gitgraph.get("highlightOuterSize").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.highlight_outer_size = val as f32;
        }
        if let Some(val) = gitgraph.get("highlightInnerSize").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.highlight_inner_size = val as f32;
        }
        if let Some(val) = gitgraph.get("reverseCrossSize").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.reverse_cross_size = val as f32;
        }
        if let Some(val) = gitgraph.get("reverseStrokeWidth").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.reverse_stroke_width = val as f32;
        }
        if let Some(val) = gitgraph.get("cherryPickDotRadius").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.cherry_pick_dot_radius = val as f32;
        }
        if let Some(val) = gitgraph
            .get("cherryPickDotOffsetX")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.cherry_pick_dot_offset_x = val as f32;
        }
        if let Some(val) = gitgraph
            .get("cherryPickDotOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.cherry_pick_dot_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("cherryPickStemStartOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.cherry_pick_stem_start_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("cherryPickStemEndOffsetY")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.cherry_pick_stem_end_offset_y = val as f32;
        }
        if let Some(val) = gitgraph
            .get("cherryPickStemStrokeWidth")
            .and_then(|v| v.as_f64())
        {
            config.layout.gitgraph.cherry_pick_stem_stroke_width = val as f32;
        }
        if let Some(val) = gitgraph
            .get("cherryPickAccentColor")
            .and_then(|v| v.as_str())
        {
            config.layout.gitgraph.cherry_pick_accent_color = val.to_string();
        }
        if let Some(val) = gitgraph.get("arrowStrokeWidth").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.arrow_stroke_width = val as f32;
        }
        if let Some(val) = gitgraph.get("branchStrokeWidth").and_then(|v| v.as_f64()) {
            config.layout.gitgraph.branch_stroke_width = val as f32;
        }
        if let Some(val) = gitgraph.get("branchDasharray").and_then(|v| v.as_str()) {
            config.layout.gitgraph.branch_dasharray = val.to_string();
        }
        if let Some(val) = gitgraph.get("commitSpacing").and_then(|v| v.as_f64())
            && !commit_step_set
        {
            let step = (val as f32 - config.layout.gitgraph.layout_offset).max(1.0);
            config.layout.gitgraph.commit_step = step;
        }
    }
    if let Some(c4) = init.get("c4").and_then(|v| v.as_object()) {
        let get_f32 =
            |map: &serde_json::Map<String, serde_json::Value>, key: &str| -> Option<f32> {
                map.get(key).and_then(|val| match val {
                    serde_json::Value::Number(num) => num.as_f64().map(|v| v as f32),
                    serde_json::Value::String(text) => text.trim().parse::<f32>().ok(),
                    _ => None,
                })
            };
        let get_usize =
            |map: &serde_json::Map<String, serde_json::Value>, key: &str| -> Option<usize> {
                map.get(key).and_then(|val| match val {
                    serde_json::Value::Number(num) => num.as_u64().map(|v| v as usize),
                    serde_json::Value::String(text) => text.trim().parse::<usize>().ok(),
                    _ => None,
                })
            };
        let get_bool = |map: &serde_json::Map<String, serde_json::Value>,
                        key: &str|
         -> Option<bool> { map.get(key).and_then(|val| val.as_bool()) };
        let get_string =
            |map: &serde_json::Map<String, serde_json::Value>, key: &str| -> Option<String> {
                map.get(key)
                    .and_then(|val| val.as_str())
                    .map(|val| val.to_string())
            };
        let get_num_or_string_f32 =
            |map: &serde_json::Map<String, serde_json::Value>, key: &str| -> Option<f32> {
                map.get(key).and_then(|val| match val {
                    serde_json::Value::Number(num) => num.as_f64().map(|v| v as f32),
                    serde_json::Value::String(text) => text.trim().parse::<f32>().ok(),
                    _ => None,
                })
            };
        let get_num_or_string_string =
            |map: &serde_json::Map<String, serde_json::Value>, key: &str| -> Option<String> {
                map.get(key).and_then(|val| match val {
                    serde_json::Value::String(text) => Some(text.to_string()),
                    serde_json::Value::Number(num) => num.as_f64().map(|v| v.to_string()),
                    _ => None,
                })
            };

        if let Some(val) = get_bool(c4, "useMaxWidth") {
            config.layout.c4.use_max_width = val;
        }
        if let Some(val) = get_f32(c4, "diagramMarginX") {
            config.layout.c4.diagram_margin_x = val;
        }
        if let Some(val) = get_f32(c4, "diagramMarginY") {
            config.layout.c4.diagram_margin_y = val;
        }
        if let Some(val) = get_f32(c4, "c4ShapeMargin") {
            config.layout.c4.c4_shape_margin = val;
        }
        if let Some(val) = get_f32(c4, "c4ShapePadding") {
            config.layout.c4.c4_shape_padding = val;
        }
        if let Some(val) = get_f32(c4, "width") {
            config.layout.c4.width = val;
        }
        if let Some(val) = get_f32(c4, "height") {
            config.layout.c4.height = val;
        }
        if let Some(val) = get_f32(c4, "boxMargin") {
            config.layout.c4.box_margin = val;
        }
        if let Some(val) = get_usize(c4, "c4ShapeInRow") {
            config.layout.c4.c4_shape_in_row = val;
        }
        if let Some(val) = get_f32(c4, "nextLinePaddingX") {
            config.layout.c4.next_line_padding_x = val;
        }
        if let Some(val) = get_usize(c4, "c4BoundaryInRow") {
            config.layout.c4.c4_boundary_in_row = val;
        }
        if let Some(val) = get_bool(c4, "wrap") {
            config.layout.c4.wrap = val;
        }
        if let Some(val) = get_f32(c4, "wrapPadding") {
            config.layout.c4.wrap_padding = val;
        }
        if let Some(val) = get_f32(c4, "textLineHeight") {
            config.layout.c4.text_line_height = val;
        }
        if let Some(val) = get_f32(c4, "textLineHeightSmallAdd") {
            config.layout.c4.text_line_height_small_add = val;
        }
        if let Some(val) = get_f32(c4, "textLineHeightSmallThreshold") {
            config.layout.c4.text_line_height_small_threshold = val;
        }
        if let Some(val) = get_f32(c4, "shapeCornerRadius") {
            config.layout.c4.shape_corner_radius = val;
        }
        if let Some(val) = get_f32(c4, "shapeStrokeWidth") {
            config.layout.c4.shape_stroke_width = val;
        }
        if let Some(val) = get_f32(c4, "boundaryCornerRadius") {
            config.layout.c4.boundary_corner_radius = val;
        }
        if let Some(val) = get_f32(c4, "personIconSize") {
            config.layout.c4.person_icon_size = val;
        }
        if let Some(val) = get_f32(c4, "dbEllipseHeight") {
            config.layout.c4.db_ellipse_height = val;
        }
        if let Some(val) = get_f32(c4, "queueCurveRadius") {
            config.layout.c4.queue_curve_radius = val;
        }
        if let Some(val) = get_string(c4, "boundaryStroke") {
            config.layout.c4.boundary_stroke = val;
        }
        if let Some(val) = get_string(c4, "boundaryDasharray") {
            config.layout.c4.boundary_dasharray = val;
        }
        if let Some(val) = get_f32(c4, "boundaryStrokeWidth") {
            config.layout.c4.boundary_stroke_width = val;
        }
        if let Some(val) = get_string(c4, "boundaryFill") {
            config.layout.c4.boundary_fill = val;
        }
        if let Some(val) = get_f32(c4, "boundaryFillOpacity") {
            config.layout.c4.boundary_fill_opacity = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "personFontSize") {
            config.layout.c4.person_font_size = val;
        }
        if let Some(val) = get_string(c4, "personFontFamily") {
            config.layout.c4.person_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "personFontWeight") {
            config.layout.c4.person_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalPersonFontSize") {
            config.layout.c4.external_person_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalPersonFontFamily") {
            config.layout.c4.external_person_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalPersonFontWeight") {
            config.layout.c4.external_person_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "systemFontSize") {
            config.layout.c4.system_font_size = val;
        }
        if let Some(val) = get_string(c4, "systemFontFamily") {
            config.layout.c4.system_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "systemFontWeight") {
            config.layout.c4.system_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalSystemFontSize") {
            config.layout.c4.external_system_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalSystemFontFamily") {
            config.layout.c4.external_system_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalSystemFontWeight") {
            config.layout.c4.external_system_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "systemDbFontSize") {
            config.layout.c4.system_db_font_size = val;
        }
        if let Some(val) = get_string(c4, "systemDbFontFamily") {
            config.layout.c4.system_db_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "systemDbFontWeight") {
            config.layout.c4.system_db_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalSystemDbFontSize") {
            config.layout.c4.external_system_db_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalSystemDbFontFamily") {
            config.layout.c4.external_system_db_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalSystemDbFontWeight") {
            config.layout.c4.external_system_db_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "systemQueueFontSize") {
            config.layout.c4.system_queue_font_size = val;
        }
        if let Some(val) = get_string(c4, "systemQueueFontFamily") {
            config.layout.c4.system_queue_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "systemQueueFontWeight") {
            config.layout.c4.system_queue_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalSystemQueueFontSize") {
            config.layout.c4.external_system_queue_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalSystemQueueFontFamily") {
            config.layout.c4.external_system_queue_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalSystemQueueFontWeight") {
            config.layout.c4.external_system_queue_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "boundaryFontSize") {
            config.layout.c4.boundary_font_size = val;
        }
        if let Some(val) = get_string(c4, "boundaryFontFamily") {
            config.layout.c4.boundary_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "boundaryFontWeight") {
            config.layout.c4.boundary_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "messageFontSize") {
            config.layout.c4.message_font_size = val;
        }
        if let Some(val) = get_string(c4, "messageFontFamily") {
            config.layout.c4.message_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "messageFontWeight") {
            config.layout.c4.message_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "containerFontSize") {
            config.layout.c4.container_font_size = val;
        }
        if let Some(val) = get_string(c4, "containerFontFamily") {
            config.layout.c4.container_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "containerFontWeight") {
            config.layout.c4.container_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalContainerFontSize") {
            config.layout.c4.external_container_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalContainerFontFamily") {
            config.layout.c4.external_container_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalContainerFontWeight") {
            config.layout.c4.external_container_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "containerDbFontSize") {
            config.layout.c4.container_db_font_size = val;
        }
        if let Some(val) = get_string(c4, "containerDbFontFamily") {
            config.layout.c4.container_db_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "containerDbFontWeight") {
            config.layout.c4.container_db_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalContainerDbFontSize") {
            config.layout.c4.external_container_db_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalContainerDbFontFamily") {
            config.layout.c4.external_container_db_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalContainerDbFontWeight") {
            config.layout.c4.external_container_db_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "containerQueueFontSize") {
            config.layout.c4.container_queue_font_size = val;
        }
        if let Some(val) = get_string(c4, "containerQueueFontFamily") {
            config.layout.c4.container_queue_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "containerQueueFontWeight") {
            config.layout.c4.container_queue_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalContainerQueueFontSize") {
            config.layout.c4.external_container_queue_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalContainerQueueFontFamily") {
            config.layout.c4.external_container_queue_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalContainerQueueFontWeight") {
            config.layout.c4.external_container_queue_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "componentFontSize") {
            config.layout.c4.component_font_size = val;
        }
        if let Some(val) = get_string(c4, "componentFontFamily") {
            config.layout.c4.component_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "componentFontWeight") {
            config.layout.c4.component_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalComponentFontSize") {
            config.layout.c4.external_component_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalComponentFontFamily") {
            config.layout.c4.external_component_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalComponentFontWeight") {
            config.layout.c4.external_component_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "componentDbFontSize") {
            config.layout.c4.component_db_font_size = val;
        }
        if let Some(val) = get_string(c4, "componentDbFontFamily") {
            config.layout.c4.component_db_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "componentDbFontWeight") {
            config.layout.c4.component_db_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalComponentDbFontSize") {
            config.layout.c4.external_component_db_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalComponentDbFontFamily") {
            config.layout.c4.external_component_db_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalComponentDbFontWeight") {
            config.layout.c4.external_component_db_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "componentQueueFontSize") {
            config.layout.c4.component_queue_font_size = val;
        }
        if let Some(val) = get_string(c4, "componentQueueFontFamily") {
            config.layout.c4.component_queue_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "componentQueueFontWeight") {
            config.layout.c4.component_queue_font_weight = val;
        }
        if let Some(val) = get_num_or_string_f32(c4, "externalComponentQueueFontSize") {
            config.layout.c4.external_component_queue_font_size = val;
        }
        if let Some(val) = get_string(c4, "externalComponentQueueFontFamily") {
            config.layout.c4.external_component_queue_font_family = val;
        }
        if let Some(val) = get_num_or_string_string(c4, "externalComponentQueueFontWeight") {
            config.layout.c4.external_component_queue_font_weight = val;
        }
        if let Some(val) = get_string(c4, "personBgColor") {
            config.layout.c4.person_bg_color = val;
        }
        if let Some(val) = get_string(c4, "personBorderColor") {
            config.layout.c4.person_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalPersonBgColor") {
            config.layout.c4.external_person_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalPersonBorderColor") {
            config.layout.c4.external_person_border_color = val;
        }
        if let Some(val) = get_string(c4, "systemBgColor") {
            config.layout.c4.system_bg_color = val;
        }
        if let Some(val) = get_string(c4, "systemBorderColor") {
            config.layout.c4.system_border_color = val;
        }
        if let Some(val) = get_string(c4, "systemDbBgColor") {
            config.layout.c4.system_db_bg_color = val;
        }
        if let Some(val) = get_string(c4, "systemDbBorderColor") {
            config.layout.c4.system_db_border_color = val;
        }
        if let Some(val) = get_string(c4, "systemQueueBgColor") {
            config.layout.c4.system_queue_bg_color = val;
        }
        if let Some(val) = get_string(c4, "systemQueueBorderColor") {
            config.layout.c4.system_queue_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalSystemBgColor") {
            config.layout.c4.external_system_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalSystemBorderColor") {
            config.layout.c4.external_system_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalSystemDbBgColor") {
            config.layout.c4.external_system_db_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalSystemDbBorderColor") {
            config.layout.c4.external_system_db_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalSystemQueueBgColor") {
            config.layout.c4.external_system_queue_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalSystemQueueBorderColor") {
            config.layout.c4.external_system_queue_border_color = val;
        }
        if let Some(val) = get_string(c4, "containerBgColor") {
            config.layout.c4.container_bg_color = val;
        }
        if let Some(val) = get_string(c4, "containerBorderColor") {
            config.layout.c4.container_border_color = val;
        }
        if let Some(val) = get_string(c4, "containerDbBgColor") {
            config.layout.c4.container_db_bg_color = val;
        }
        if let Some(val) = get_string(c4, "containerDbBorderColor") {
            config.layout.c4.container_db_border_color = val;
        }
        if let Some(val) = get_string(c4, "containerQueueBgColor") {
            config.layout.c4.container_queue_bg_color = val;
        }
        if let Some(val) = get_string(c4, "containerQueueBorderColor") {
            config.layout.c4.container_queue_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalContainerBgColor") {
            config.layout.c4.external_container_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalContainerBorderColor") {
            config.layout.c4.external_container_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalContainerDbBgColor") {
            config.layout.c4.external_container_db_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalContainerDbBorderColor") {
            config.layout.c4.external_container_db_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalContainerQueueBgColor") {
            config.layout.c4.external_container_queue_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalContainerQueueBorderColor") {
            config.layout.c4.external_container_queue_border_color = val;
        }
        if let Some(val) = get_string(c4, "componentBgColor") {
            config.layout.c4.component_bg_color = val;
        }
        if let Some(val) = get_string(c4, "componentBorderColor") {
            config.layout.c4.component_border_color = val;
        }
        if let Some(val) = get_string(c4, "componentDbBgColor") {
            config.layout.c4.component_db_bg_color = val;
        }
        if let Some(val) = get_string(c4, "componentDbBorderColor") {
            config.layout.c4.component_db_border_color = val;
        }
        if let Some(val) = get_string(c4, "componentQueueBgColor") {
            config.layout.c4.component_queue_bg_color = val;
        }
        if let Some(val) = get_string(c4, "componentQueueBorderColor") {
            config.layout.c4.component_queue_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalComponentBgColor") {
            config.layout.c4.external_component_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalComponentBorderColor") {
            config.layout.c4.external_component_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalComponentDbBgColor") {
            config.layout.c4.external_component_db_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalComponentDbBorderColor") {
            config.layout.c4.external_component_db_border_color = val;
        }
        if let Some(val) = get_string(c4, "externalComponentQueueBgColor") {
            config.layout.c4.external_component_queue_bg_color = val;
        }
        if let Some(val) = get_string(c4, "externalComponentQueueBorderColor") {
            config.layout.c4.external_component_queue_border_color = val;
        }
    }
    if let Some(mindmap) = init.get("mindmap").and_then(|v| v.as_object())
        && let Some(val) = mindmap.get("layoutAlgorithm").and_then(|v| v.as_str())
    {
        config.layout.mindmap.layout_algorithm = val.to_string();
    }
    config.render.background = config.theme.background.clone();
    config
}

/// Leaf source options implemented by the native merger.
pub const SUPPORTED_OPTIONS: &[&str] = &[
    "theme",
    "preferredAspectRatio",
    "timeline.defaultDirection",
    "themeVariables.activationBkgColor",
    "themeVariables.activationBorderColor",
    "themeVariables.actorBkg",
    "themeVariables.actorBorder",
    "themeVariables.actorLine",
    "themeVariables.background",
    "themeVariables.clusterBkg",
    "themeVariables.clusterBorder",
    "themeVariables.commitLabelBackground",
    "themeVariables.commitLabelColor",
    "themeVariables.edgeLabelBackground",
    "themeVariables.fontFamily",
    "themeVariables.fontSize",
    "themeVariables.git0",
    "themeVariables.git1",
    "themeVariables.git2",
    "themeVariables.git3",
    "themeVariables.git4",
    "themeVariables.git5",
    "themeVariables.git6",
    "themeVariables.git7",
    "themeVariables.gitBranchLabel0",
    "themeVariables.gitBranchLabel1",
    "themeVariables.gitBranchLabel2",
    "themeVariables.gitBranchLabel3",
    "themeVariables.gitBranchLabel4",
    "themeVariables.gitBranchLabel5",
    "themeVariables.gitBranchLabel6",
    "themeVariables.gitBranchLabel7",
    "themeVariables.gitInv0",
    "themeVariables.gitInv1",
    "themeVariables.gitInv2",
    "themeVariables.gitInv3",
    "themeVariables.gitInv4",
    "themeVariables.gitInv5",
    "themeVariables.gitInv6",
    "themeVariables.gitInv7",
    "themeVariables.lineColor",
    "themeVariables.noteBkg",
    "themeVariables.noteBorderColor",
    "themeVariables.primaryBorderColor",
    "themeVariables.primaryColor",
    "themeVariables.primaryTextColor",
    "themeVariables.secondaryColor",
    "themeVariables.tagLabelBackground",
    "themeVariables.tagLabelBorder",
    "themeVariables.tagLabelColor",
    "themeVariables.tertiaryColor",
    "themeVariables.textColor",
    "flowchart.nodeSpacing",
    "flowchart.orderPasses",
    "flowchart.portPadMax",
    "flowchart.portPadMin",
    "flowchart.portPadRatio",
    "flowchart.portSideBias",
    "flowchart.rankSpacing",
    "gitGraph.arrowRadius",
    "gitGraph.arrowRerouteRadius",
    "gitGraph.arrowStrokeWidth",
    "gitGraph.branchDasharray",
    "gitGraph.branchLabelBgOffsetX",
    "gitGraph.branchLabelBgOffsetY",
    "gitGraph.branchLabelBgPadX",
    "gitGraph.branchLabelBgPadY",
    "gitGraph.branchLabelBtOffsetY",
    "gitGraph.branchLabelCornerRadius",
    "gitGraph.branchLabelFontSize",
    "gitGraph.branchLabelLineHeight",
    "gitGraph.branchLabelRotateExtra",
    "gitGraph.branchLabelTbBgOffsetX",
    "gitGraph.branchLabelTbOffsetY",
    "gitGraph.branchLabelTbTextOffsetX",
    "gitGraph.branchLabelTextOffsetX",
    "gitGraph.branchLabelTextOffsetY",
    "gitGraph.branchLabelTranslateX",
    "gitGraph.branchSpacing",
    "gitGraph.branchSpacingRotateExtra",
    "gitGraph.branchStrokeWidth",
    "gitGraph.cherryPickAccentColor",
    "gitGraph.cherryPickDotOffsetX",
    "gitGraph.cherryPickDotOffsetY",
    "gitGraph.cherryPickDotRadius",
    "gitGraph.cherryPickStemEndOffsetY",
    "gitGraph.cherryPickStemStartOffsetY",
    "gitGraph.cherryPickStemStrokeWidth",
    "gitGraph.commitLabelBgOffsetY",
    "gitGraph.commitLabelBgOpacity",
    "gitGraph.commitLabelFontSize",
    "gitGraph.commitLabelLineHeight",
    "gitGraph.commitLabelOffsetY",
    "gitGraph.commitLabelPadding",
    "gitGraph.commitLabelRotateAngle",
    "gitGraph.commitLabelRotateTranslateXBase",
    "gitGraph.commitLabelRotateTranslateXScale",
    "gitGraph.commitLabelRotateTranslateXWidthOffset",
    "gitGraph.commitLabelRotateTranslateYBase",
    "gitGraph.commitLabelRotateTranslateYScale",
    "gitGraph.commitLabelTbBgExtra",
    "gitGraph.commitLabelTbBgOffsetY",
    "gitGraph.commitLabelTbTextExtra",
    "gitGraph.commitLabelTbTextOffsetY",
    "gitGraph.commitRadius",
    "gitGraph.commitSpacing",
    "gitGraph.commitStep",
    "gitGraph.defaultPos",
    "gitGraph.diagramPadding",
    "gitGraph.highlightInnerSize",
    "gitGraph.highlightOuterSize",
    "gitGraph.laneMaxDepth",
    "gitGraph.laneSpacing",
    "gitGraph.layoutOffset",
    "gitGraph.mainBranchName",
    "gitGraph.mainBranchOrder",
    "gitGraph.mergeRadiusInner",
    "gitGraph.mergeRadiusOuter",
    "gitGraph.parallelCommits",
    "gitGraph.reverseCrossSize",
    "gitGraph.reverseStrokeWidth",
    "gitGraph.rotateCommitLabel",
    "gitGraph.showBranches",
    "gitGraph.showCommitLabel",
    "gitGraph.tagHoleRadius",
    "gitGraph.tagLabelFontSize",
    "gitGraph.tagLabelLineHeight",
    "gitGraph.tagPaddingX",
    "gitGraph.tagPaddingY",
    "gitGraph.tagPolygonOffsetY",
    "gitGraph.tagRotateAngle",
    "gitGraph.tagRotateTranslate",
    "gitGraph.tagSpacingY",
    "gitGraph.tagTextOffsetXTb",
    "gitGraph.tagTextOffsetY",
    "gitGraph.tagTextOffsetYTb",
    "gitGraph.tagTextRotateTranslate",
    "gitGraph.textWidthScale",
    "gitGraph.titleTopMargin",
    "gitGraph.useMaxWidth",
    "c4.boundaryCornerRadius",
    "c4.boundaryDasharray",
    "c4.boundaryFill",
    "c4.boundaryFillOpacity",
    "c4.boundaryFontFamily",
    "c4.boundaryFontSize",
    "c4.boundaryFontWeight",
    "c4.boundaryStroke",
    "c4.boundaryStrokeWidth",
    "c4.boxMargin",
    "c4.c4BoundaryInRow",
    "c4.c4ShapeInRow",
    "c4.c4ShapeMargin",
    "c4.c4ShapePadding",
    "c4.componentBgColor",
    "c4.componentBorderColor",
    "c4.componentDbBgColor",
    "c4.componentDbBorderColor",
    "c4.componentDbFontFamily",
    "c4.componentDbFontSize",
    "c4.componentDbFontWeight",
    "c4.componentFontFamily",
    "c4.componentFontSize",
    "c4.componentFontWeight",
    "c4.componentQueueBgColor",
    "c4.componentQueueBorderColor",
    "c4.componentQueueFontFamily",
    "c4.componentQueueFontSize",
    "c4.componentQueueFontWeight",
    "c4.containerBgColor",
    "c4.containerBorderColor",
    "c4.containerDbBgColor",
    "c4.containerDbBorderColor",
    "c4.containerDbFontFamily",
    "c4.containerDbFontSize",
    "c4.containerDbFontWeight",
    "c4.containerFontFamily",
    "c4.containerFontSize",
    "c4.containerFontWeight",
    "c4.containerQueueBgColor",
    "c4.containerQueueBorderColor",
    "c4.containerQueueFontFamily",
    "c4.containerQueueFontSize",
    "c4.containerQueueFontWeight",
    "c4.dbEllipseHeight",
    "c4.diagramMarginX",
    "c4.diagramMarginY",
    "c4.externalComponentBgColor",
    "c4.externalComponentBorderColor",
    "c4.externalComponentDbBgColor",
    "c4.externalComponentDbBorderColor",
    "c4.externalComponentDbFontFamily",
    "c4.externalComponentDbFontSize",
    "c4.externalComponentDbFontWeight",
    "c4.externalComponentFontFamily",
    "c4.externalComponentFontSize",
    "c4.externalComponentFontWeight",
    "c4.externalComponentQueueBgColor",
    "c4.externalComponentQueueBorderColor",
    "c4.externalComponentQueueFontFamily",
    "c4.externalComponentQueueFontSize",
    "c4.externalComponentQueueFontWeight",
    "c4.externalContainerBgColor",
    "c4.externalContainerBorderColor",
    "c4.externalContainerDbBgColor",
    "c4.externalContainerDbBorderColor",
    "c4.externalContainerDbFontFamily",
    "c4.externalContainerDbFontSize",
    "c4.externalContainerDbFontWeight",
    "c4.externalContainerFontFamily",
    "c4.externalContainerFontSize",
    "c4.externalContainerFontWeight",
    "c4.externalContainerQueueBgColor",
    "c4.externalContainerQueueBorderColor",
    "c4.externalContainerQueueFontFamily",
    "c4.externalContainerQueueFontSize",
    "c4.externalContainerQueueFontWeight",
    "c4.externalPersonBgColor",
    "c4.externalPersonBorderColor",
    "c4.externalPersonFontFamily",
    "c4.externalPersonFontSize",
    "c4.externalPersonFontWeight",
    "c4.externalSystemBgColor",
    "c4.externalSystemBorderColor",
    "c4.externalSystemDbBgColor",
    "c4.externalSystemDbBorderColor",
    "c4.externalSystemDbFontFamily",
    "c4.externalSystemDbFontSize",
    "c4.externalSystemDbFontWeight",
    "c4.externalSystemFontFamily",
    "c4.externalSystemFontSize",
    "c4.externalSystemFontWeight",
    "c4.externalSystemQueueBgColor",
    "c4.externalSystemQueueBorderColor",
    "c4.externalSystemQueueFontFamily",
    "c4.externalSystemQueueFontSize",
    "c4.externalSystemQueueFontWeight",
    "c4.height",
    "c4.messageFontFamily",
    "c4.messageFontSize",
    "c4.messageFontWeight",
    "c4.nextLinePaddingX",
    "c4.personBgColor",
    "c4.personBorderColor",
    "c4.personFontFamily",
    "c4.personFontSize",
    "c4.personFontWeight",
    "c4.personIconSize",
    "c4.queueCurveRadius",
    "c4.shapeCornerRadius",
    "c4.shapeStrokeWidth",
    "c4.systemBgColor",
    "c4.systemBorderColor",
    "c4.systemDbBgColor",
    "c4.systemDbBorderColor",
    "c4.systemDbFontFamily",
    "c4.systemDbFontSize",
    "c4.systemDbFontWeight",
    "c4.systemFontFamily",
    "c4.systemFontSize",
    "c4.systemFontWeight",
    "c4.systemQueueBgColor",
    "c4.systemQueueBorderColor",
    "c4.systemQueueFontFamily",
    "c4.systemQueueFontSize",
    "c4.systemQueueFontWeight",
    "c4.textLineHeight",
    "c4.textLineHeightSmallAdd",
    "c4.textLineHeightSmallThreshold",
    "c4.useMaxWidth",
    "c4.width",
    "c4.wrap",
    "c4.wrapPadding",
    "mindmap.layoutAlgorithm",
];
