//! Timing limits are calibrated from five accepted release runs on the runner.
use super::*;

#[derive(Deserialize)]
struct Calibration {
    profile: String,
    samples_ns: std::collections::BTreeMap<String, Vec<f64>>,
}

fn limit(samples: &[f64]) -> Result<f64, String> {
    if samples.len() != 5
        || samples
            .iter()
            .any(|sample| !sample.is_finite() || *sample <= 0.0)
    {
        return Err("indexed history calibration requires five positive finite samples".into());
    }
    let mut samples = samples.to_vec();
    samples.sort_by(f64::total_cmp);
    Ok(samples[2] * 1.25)
}

pub(super) fn calibrated_specs() -> Result<Vec<PerfBudgetSpec>, String> {
    let Some(path) = env::var_os("GITCOMET_INDEXED_HISTORY_BASELINE") else {
        return Ok(Vec::new());
    };
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let calibration: Calibration =
        serde_json::from_slice(&fs::read(path).map_err(|err| err.to_string())?)
            .map_err(|err| err.to_string())?;
    if calibration.profile != "release" {
        return Err("indexed history shipping budgets require release calibration".into());
    }
    let mut specs = Vec::new();
    for &(label, estimate_path) in CASES {
        let Some(samples) = calibration.samples_ns.get(label) else {
            continue;
        };
        specs.push(PerfBudgetSpec {
            label,
            estimate_path,
            threshold_ns: limit(samples)?,
        });
    }
    if specs.is_empty() {
        return Err("indexed history calibration contains no recognized cases".into());
    }
    Ok(specs)
}

const CASES: &[(&str, &str)] = &[
    (
        "indexed_history/100000_rows_1_columns/build_graph",
        "indexed_history/100000_rows_1_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_1_columns/first_touch",
        "indexed_history/100000_rows_1_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_1_columns/warm",
        "indexed_history/100000_rows_1_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_1_columns/distant_jump",
        "indexed_history/100000_rows_1_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_64_columns/build_graph",
        "indexed_history/100000_rows_64_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_64_columns/first_touch",
        "indexed_history/100000_rows_64_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_64_columns/warm",
        "indexed_history/100000_rows_64_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_64_columns/distant_jump",
        "indexed_history/100000_rows_64_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_512_columns/build_graph",
        "indexed_history/100000_rows_512_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_512_columns/first_touch",
        "indexed_history/100000_rows_512_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_512_columns/warm",
        "indexed_history/100000_rows_512_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_512_columns/distant_jump",
        "indexed_history/100000_rows_512_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_5261_columns/build_graph",
        "indexed_history/100000_rows_5261_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_5261_columns/first_touch",
        "indexed_history/100000_rows_5261_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_5261_columns/warm",
        "indexed_history/100000_rows_5261_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_5261_columns/distant_jump",
        "indexed_history/100000_rows_5261_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_1_columns/build_graph",
        "indexed_history/2000000_rows_1_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_1_columns/first_touch",
        "indexed_history/2000000_rows_1_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_1_columns/warm",
        "indexed_history/2000000_rows_1_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_1_columns/distant_jump",
        "indexed_history/2000000_rows_1_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_64_columns/build_graph",
        "indexed_history/2000000_rows_64_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_64_columns/first_touch",
        "indexed_history/2000000_rows_64_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_64_columns/warm",
        "indexed_history/2000000_rows_64_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_64_columns/distant_jump",
        "indexed_history/2000000_rows_64_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_512_columns/build_graph",
        "indexed_history/2000000_rows_512_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_512_columns/first_touch",
        "indexed_history/2000000_rows_512_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_512_columns/warm",
        "indexed_history/2000000_rows_512_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_512_columns/distant_jump",
        "indexed_history/2000000_rows_512_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_5261_columns/build_graph",
        "indexed_history/2000000_rows_5261_columns/build_graph/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_5261_columns/first_touch",
        "indexed_history/2000000_rows_5261_columns/first_touch/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_5261_columns/warm",
        "indexed_history/2000000_rows_5261_columns/warm/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_5261_columns/distant_jump",
        "indexed_history/2000000_rows_5261_columns/distant_jump/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_1_columns/changing_selection",
        "indexed_history/100000_rows_1_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_64_columns/changing_selection",
        "indexed_history/100000_rows_64_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_512_columns/changing_selection",
        "indexed_history/100000_rows_512_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/100000_rows_5261_columns/changing_selection",
        "indexed_history/100000_rows_5261_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/index_100000_hash_20",
        "indexed_history/index_100000_hash_20/new/estimates.json",
    ),
    (
        "indexed_history/index_100000_hash_32",
        "indexed_history/index_100000_hash_32/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_1_columns/changing_selection",
        "indexed_history/2000000_rows_1_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_64_columns/changing_selection",
        "indexed_history/2000000_rows_64_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_512_columns/changing_selection",
        "indexed_history/2000000_rows_512_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/2000000_rows_5261_columns/changing_selection",
        "indexed_history/2000000_rows_5261_columns/changing_selection/new/estimates.json",
    ),
    (
        "indexed_history/index_2000000_hash_20",
        "indexed_history/index_2000000_hash_20/new/estimates.json",
    ),
    (
        "indexed_history/index_2000000_hash_32",
        "indexed_history/index_2000000_hash_32/new/estimates.json",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn median_of_five_sets_a_twenty_five_percent_margin() {
        assert_eq!(limit(&[120.0, 100.0, 110.0, 130.0, 500.0]).unwrap(), 150.0);
        assert!(limit(&[1.0; 4]).is_err());
        assert!(limit(&[f64::NAN; 5]).is_err());
        assert!(limit(&[-1.0; 5]).is_err());
    }
}
