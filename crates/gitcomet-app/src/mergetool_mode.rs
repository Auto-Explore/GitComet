use crate::cli::{MergetoolConfig, exit_code};
use gitcomet_core::{
    conflict_labels::{BaseLabelScenario, format_base_label},
    conflict_session::try_autosolve_merged_text,
    merge::{MergeError, MergeLabels, MergeOptions, MergeResult, merge_file_bytes},
};
use std::{fs, path::Path};

/// Result of running the dedicated mergetool mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergetoolRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// The merge result details (available when merge was attempted).
    pub merge_result: Option<MergeResult>,
}

/// Execute mergetool mode using the built-in 3-way merge algorithm.
///
/// Reads base, local, and remote files, performs a 3-way merge, and writes
/// the result to the merged output path. Returns SUCCESS (0) on clean merge,
/// CANCELED (1) if conflicts remain in the output.
///
/// When invoked by `git mergetool`, the contract is:
/// - Exit 0: merge succeeded, MERGED file contains resolved content
/// - Exit 1: merge has unresolved conflicts (MERGED contains markers)
/// - Exit ≥2: operational error (bad input, I/O failure, etc.)
pub fn run_mergetool(config: &MergetoolConfig) -> Result<MergetoolRunResult, String> {
    // Read the three input sides.
    let local_bytes = fs::read(&config.local)
        .map_err(|e| format!("Failed to read local file {}: {e}", config.local.display()))?;
    let remote_bytes = fs::read(&config.remote).map_err(|e| {
        format!(
            "Failed to read remote file {}: {e}",
            config.remote.display()
        )
    })?;

    let base_bytes = match &config.base {
        Some(base_path) => fs::read(base_path)
            .map_err(|e| format!("Failed to read base file {}: {e}", base_path.display()))?,
        // No base file: treat as empty (add/add conflict scenario).
        None => Vec::new(),
    };

    // Build merge options from config labels and algorithm preferences.
    let options = MergeOptions {
        style: config.conflict_style,
        diff_algorithm: config.diff_algorithm,
        marker_size: config.marker_size,
        labels: derive_effective_labels(config),
        ..MergeOptions::default()
    };

    // Run the 3-way merge algorithm with byte-level binary detection.
    let result = match merge_file_bytes(&base_bytes, &local_bytes, &remote_bytes, &options) {
        Ok(result) => result,
        Err(MergeError::BinaryContent) => {
            return handle_binary_merge(
                config,
                config.base.is_some(),
                &base_bytes,
                &local_bytes,
                &remote_bytes,
            );
        }
    };
    let is_clean = result.is_clean();
    let conflict_count = result.conflict_count;

    // Write merged output to MERGED path.
    write_merged_output(config, result.output.as_bytes())?;

    if is_clean {
        let display_name = merged_display_name(config);
        Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {display_name}\n"),
            exit_code: exit_code::SUCCESS,
            merge_result: Some(result),
        })
    } else if config.auto {
        // Auto mode: try heuristic passes on conflict blocks.
        if let Some(clean_output) = try_autosolve_merged_text(&result.output) {
            // All conflicts resolved by heuristics — write clean output.
            write_merged_output(config, clean_output.as_bytes())?;
            let display_name = merged_display_name(config);
            Ok(MergetoolRunResult {
                stdout: String::new(),
                stderr: format!("Auto-resolved {display_name}\n"),
                exit_code: exit_code::SUCCESS,
                merge_result: Some(result),
            })
        } else {
            // Some conflicts remain — write original markers.
            let display_name = merged_display_name(config);
            Ok(MergetoolRunResult {
                stdout: String::new(),
                stderr: format!(
                    "Auto-merging {display_name}\nCONFLICT (content): Merge conflict in {display_name}\n\
                     Automatic merge failed; {conflict_count} conflict(s) remain.\n",
                ),
                exit_code: exit_code::CANCELED,
                merge_result: Some(result),
            })
        }
    } else {
        let display_name = merged_display_name(config);
        Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!(
                "Auto-merging {display_name}\nCONFLICT (content): Merge conflict in {display_name}\n\
                 Automatic merge failed; {conflict_count} conflict(s) remain.\n",
            ),
            exit_code: exit_code::CANCELED,
            merge_result: Some(result),
        })
    }
}

/// Extract a human-readable display name from the MERGED output path.
fn merged_display_name(config: &MergetoolConfig) -> String {
    config
        .merged
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| config.merged.display().to_string())
}

fn derive_effective_labels(config: &MergetoolConfig) -> MergeLabels {
    let ours = Some(
        config
            .label_local
            .clone()
            .unwrap_or_else(|| default_path_label(&config.local)),
    );
    let theirs = Some(
        config
            .label_remote
            .clone()
            .unwrap_or_else(|| default_path_label(&config.remote)),
    );
    let base = Some(match (&config.label_base, &config.base) {
        (Some(label), _) => label.clone(),
        (None, Some(base_path)) => default_path_label(base_path),
        (None, None) => format_base_label(&BaseLabelScenario::NoBase),
    });

    MergeLabels { ours, base, theirs }
}

fn default_path_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn merged_filename(config: &MergetoolConfig) -> String {
    config
        .merged
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| config.merged.display().to_string())
}

/// Handle binary files with conservative 3-way heuristics:
/// - clean when both sides are identical
/// - clean when exactly one side changed from BASE (if BASE exists)
/// - conflict fallback when both sides changed differently
fn handle_binary_merge(
    config: &MergetoolConfig,
    has_base: bool,
    base_bytes: &[u8],
    local_bytes: &[u8],
    remote_bytes: &[u8],
) -> Result<MergetoolRunResult, String> {
    let filename = merged_filename(config);

    if local_bytes == remote_bytes {
        write_merged_output(config, local_bytes)?;
        return Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {filename} (binary identical on both sides)\n"),
            exit_code: exit_code::SUCCESS,
            merge_result: None,
        });
    }

    if has_base && local_bytes == base_bytes && remote_bytes != base_bytes {
        write_merged_output(config, remote_bytes)?;
        return Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {filename} (binary remote changed from base)\n"),
            exit_code: exit_code::SUCCESS,
            merge_result: None,
        });
    }

    if has_base && remote_bytes == base_bytes && local_bytes != base_bytes {
        write_merged_output(config, local_bytes)?;
        return Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {filename} (binary local changed from base)\n"),
            exit_code: exit_code::SUCCESS,
            merge_result: None,
        });
    }

    // Conflict fallback: keep local bytes in output so users can resolve by
    // explicitly choosing a side in follow-up tooling.
    write_merged_output(config, local_bytes)?;

    Ok(MergetoolRunResult {
        stdout: String::new(),
        stderr: format!(
            "warning: Cannot merge binary files: {filename}\n\
             CONFLICT (binary): {filename} — keeping local version.\n"
        ),
        exit_code: exit_code::CANCELED,
        merge_result: None,
    })
}

fn write_merged_output(config: &MergetoolConfig, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = config.merged.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create merged output directory {}: {e}",
                parent.display()
            )
        })?;
    }

    fs::write(&config.merged, bytes).map_err(|e| {
        format!(
            "Failed to write merged output to {}: {e}",
            config.merged.display()
        )
    })
}

#[cfg(test)]
mod tests;
