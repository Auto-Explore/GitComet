//! Offline Mermaid recognition and SVG rendering, independent of the UI.
use std::path::Path;
use std::sync::Arc;

pub mod scene;
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiagramKind {
    Mermaid,
}

impl DiagramKind {
    pub fn label(self) -> &'static str {
        "Mermaid"
    }
    pub fn for_path(path: &Path) -> Option<Self> {
        Self::for_token(path.extension()?.to_str()?)
    }
    fn for_token(token: &str) -> Option<Self> {
        matches!(token.to_ascii_lowercase().as_str(), "mmd" | "mermaid").then_some(Self::Mermaid)
    }
    pub fn for_file(path: &Path, _source: &str) -> Option<Self> {
        Self::for_path(path)
    }
    pub fn for_fence(info: &str, source: &str) -> Option<Self> {
        let token = info
            .trim()
            .split(|c: char| c.is_ascii_whitespace() || c == ',')
            .next()?;
        let token = token
            .trim_matches(['{', '}'])
            .trim_start_matches('.')
            .to_ascii_lowercase();
        let token = token.strip_prefix("language-").unwrap_or(&token);
        if token.is_empty() {
            Self::detect(source)
        } else {
            Self::for_token(token)
        }
    }
    /// Header recognition shares aliases with parsing and performs no layout.
    pub fn detect(source: &str) -> Option<Self> {
        if source.len() > MAX_SOURCE_BYTES {
            return None;
        }
        mermaid_rs_renderer::parser::detect_diagram_kind(source.trim_start_matches('\u{feff}'))
            .map(|_| Self::Mermaid)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagramPage {
    pub label: String,
    pub svg: Arc<[u8]>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagramOutput {
    pub pages: Vec<DiagramPage>,
    pub diagnostics: Vec<String>,
}

/// Render source without spawning processes or resolving external resources.
pub fn render(kind: DiagramKind, source: &str) -> Result<DiagramOutput, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("Diagram exceeds the 1 MiB preview limit.".into());
    }
    if source.trim().is_empty() {
        return Err("Empty diagram.".into());
    }
    std::panic::catch_unwind(|| render_mermaid(kind, source.trim_start_matches('\u{feff}')))
        .map_err(|_| "The native Mermaid renderer could not process this source.".to_string())?
}

fn render_mermaid(kind: DiagramKind, source: &str) -> Result<DiagramOutput, String> {
    let mut diagnostics = Vec::new();
    let (source, frontmatter) = source_frontmatter(source)?;
    // Exactly one parse/layout/SVG pass. The convenience API discarded init_config.
    let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).map_err(|e| e.to_string())?;
    let mut config = mermaid_rs_renderer::Config {
        theme: mermaid_rs_renderer::Theme::modern(),
        ..Default::default()
    };
    for mut value in frontmatter.into_iter().chain(parsed.init_config) {
        diagnose_options(&value, "", &mut diagnostics);
        normalize_config_fonts(&mut value, &mut diagnostics);
        config = mermaid_rs_renderer::source_config::merge_init_config(config, value);
    }
    config.theme.font_family = bundled_family(&config.theme.font_family);
    let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &config.theme, &config.layout);
    let dimensions = mermaid_rs_renderer::measure_svg_dimensions(&layout, &config.layout, None);
    // Native SVGs for pie, C4, GitGraph and mindmap use CSS percentage widths.
    // Supply their natural size explicitly for an actual-size native viewport.
    let svg = mermaid_rs_renderer::render::render_svg_with_dimensions(
        &layout,
        &config.theme,
        &config.layout,
        Some((dimensions.width, dimensions.height)),
    );
    validate_svg(&svg)?;
    Ok(DiagramOutput {
        pages: vec![DiagramPage {
            label: kind.label().into(),
            svg: svg.into_bytes().into(),
        }],
        diagnostics,
    })
}

fn bundled_family(requested: &str) -> String {
    let family = requested
        .split(',')
        .find_map(|part| {
            let part = part.trim().trim_matches(['\'', '"']);
            [
                gitcomet_fonts::FIRA_CODE_FONT_FAMILY,
                gitcomet_fonts::LILEX_FONT_FAMILY,
                gitcomet_fonts::IBM_PLEX_SANS_FONT_FAMILY,
            ]
            .into_iter()
            .find(|family| part.eq_ignore_ascii_case(family))
        })
        .unwrap_or_else(|| {
            if requested.to_ascii_lowercase().contains("mono") {
                gitcomet_fonts::LILEX_FONT_FAMILY
            } else {
                gitcomet_fonts::IBM_PLEX_SANS_FONT_FAMILY
            }
        });
    // Canonical keys also prevent arbitrary unavailable family names from
    // accumulating duplicate font-face data in the native text measurer.
    format!("{family}, Noto Emoji")
}

fn normalize_config_fonts(value: &mut serde_json::Value, diagnostics: &mut Vec<String>) {
    if let Some(object) = value.as_object_mut() {
        for (key, child) in object {
            if (key == "fontFamily" || key.ends_with("FontFamily"))
                && let Some(requested) = child.as_str()
            {
                let bundled = bundled_family(requested);
                if !requested.split(',').any(|family| {
                    matches!(
                        family.trim().trim_matches(['\'', '"']),
                        "IBM Plex Sans"
                            | "Lilex"
                            | "Fira Code"
                            | "sans-serif"
                            | "serif"
                            | "monospace"
                            | "system-ui"
                            | "Arial"
                            | "Inter"
                            | "Open Sans"
                    )
                }) {
                    diagnostics.push(format!(
                        "Mermaid font '{requested}' is unavailable; using bundled IBM Plex Sans."
                    ));
                }
                *child = bundled.into();
            } else {
                normalize_config_fonts(child, diagnostics);
            }
        }
    }
}

fn diagnose_options(value: &serde_json::Value, prefix: &str, diagnostics: &mut Vec<String>) {
    if let Some(object) = value.as_object() {
        for (key, child) in object {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if mermaid_rs_renderer::source_config::SUPPORTED_OPTIONS.contains(&path.as_str()) {
                continue;
            }
            if child.is_object()
                && mermaid_rs_renderer::source_config::SUPPORTED_OPTIONS
                    .iter()
                    .any(|s| s.starts_with(&format!("{path}.")))
            {
                diagnose_options(child, &path, diagnostics);
            } else {
                diagnostics.push(format!(
                    "Mermaid setting '{path}' is not supported by the native preview."
                ));
            }
        }
    } else {
        diagnostics.push("Mermaid configuration must be an object.".into());
    }
}

fn source_frontmatter(source: &str) -> Result<(&str, Option<serde_json::Value>), String> {
    let trimmed = source.trim_start();
    let Some(rest) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    else {
        return Ok((source, None));
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim() == "---" {
            let docs = yaml_rust2::YamlLoader::load_from_str(&rest[..offset])
                .map_err(|e| format!("Invalid Mermaid frontmatter: {e}"))?;
            let config = docs
                .first()
                .and_then(|doc| doc["config"].as_hash())
                .map(|hash| yaml_json(&yaml_rust2::Yaml::Hash(hash.clone())));
            return Ok((&rest[offset + line.len()..], config));
        }
        offset += line.len();
    }
    Err("Mermaid frontmatter is missing its closing '---'.".into())
}
fn yaml_json(value: &yaml_rust2::Yaml) -> serde_json::Value {
    use yaml_rust2::Yaml;
    match value {
        Yaml::Hash(map) => serde_json::Value::Object(
            map.iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), yaml_json(v))))
                .collect(),
        ),
        Yaml::Array(items) => items.iter().map(yaml_json).collect(),
        Yaml::String(s) => s.clone().into(),
        Yaml::Integer(n) => (*n).into(),
        Yaml::Real(s) => s
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or_default(),
        Yaml::Boolean(b) => (*b).into(),
        _ => serde_json::Value::Null,
    }
}
fn validate_svg(svg: &str) -> Result<(), String> {
    let document = roxmltree::Document::parse(svg).map_err(|e| e.to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return Err("Invalid Mermaid SVG root.".into());
    }
    for name in ["width", "height"] {
        let size: f32 = root
            .attribute(name)
            .ok_or("Missing Mermaid SVG dimensions.")?
            .parse()
            .map_err(|_| "Invalid Mermaid SVG dimensions.")?;
        if !size.is_finite() || size <= 0.0 {
            return Err("Invalid Mermaid SVG dimensions.".into());
        }
    }
    for node in document.descendants().filter(|node| node.is_element()) {
        if node.tag_name().name() == "foreignObject" {
            return Err("HTML diagram labels are not supported by the native preview. View the source instead.".into());
        }
        if node.tag_name().name() == "image"
            && node
                .attributes()
                .any(|a| a.name() == "href" && !a.value().starts_with("data:"))
        {
            return Err("External diagram images are not supported by the offline preview.".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
