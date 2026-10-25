use crate::cli::{MergetoolConfig, exit_code};
use gitcomet_core::text_format::{
    SideKind, TextAttributes, TextEncoding, TextFormat, decode_bytes,
};
use gitcomet_core::{
    conflict_labels::{BaseLabelScenario, format_base_label},
    conflict_session::try_autosolve_merge_plan,
    merge::{
        MergeError, MergeLabels, MergeOptions, build_merge_plan_bytes_with_optional_base,
        render_merge_plan,
    },
};
use std::{fs, path::Path};

/// Result of running the dedicated mergetool mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergetoolRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
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

    let base_bytes = config
        .base
        .as_ref()
        .map(|base_path| {
            fs::read(base_path)
                .map_err(|e| format!("Failed to read base file {}: {e}", base_path.display()))
        })
        .transpose()?;

    // Text in another encoding merges as UTF-8 and is written back as it was.
    // Keep the original bytes for fallback if any decoded side is binary.
    let decoded = decode_merge_inputs(base_bytes.as_deref(), &local_bytes, &remote_bytes);
    let (merge_base, merge_local, merge_remote, output_format) = match &decoded {
        Some((base, local, remote, format)) => (
            base.as_deref().map(str::as_bytes),
            local.as_bytes(),
            remote.as_bytes(),
            Some(*format),
        ),
        None => (
            base_bytes.as_deref(),
            local_bytes.as_slice(),
            remote_bytes.as_slice(),
            None,
        ),
    };
    let encode_output = |text: &str| -> Result<Vec<u8>, String> {
        match output_format {
            Some(format) => gitcomet_core::text_format::encode(text, format)
                .map(|bytes| bytes.into_owned())
                .map_err(|unmappable| format!("Failed to write merged output: {unmappable}")),
            None => Ok(text.as_bytes().to_vec()),
        }
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
    let plan = match build_merge_plan_bytes_with_optional_base(
        merge_base,
        merge_local,
        merge_remote,
        &options,
    ) {
        Ok(result) => result,
        Err(MergeError::BinaryContent) => {
            return handle_binary_merge(
                config,
                config.base.is_some(),
                base_bytes.as_deref().unwrap_or_default(),
                &local_bytes,
                &remote_bytes,
            );
        }
    };
    let result = render_merge_plan(&plan, &options);
    let is_clean = result.is_clean();
    let conflict_count = result.conflict_count;

    // Write merged output to MERGED path.
    let bytes = match encode_output(&result.output) {
        Ok(bytes) => bytes,
        Err(error) => return handle_encoding_conflict(config, &local_bytes, &error),
    };
    write_merged_output(config, &bytes)?;

    if is_clean {
        let display_name = merged_display_name(config);
        Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {display_name}\n"),
            exit_code: exit_code::SUCCESS,
        })
    } else if config.auto {
        // Auto mode: try heuristic passes on conflict blocks.
        if let Some(clean_output) = try_autosolve_merge_plan(&plan, &options) {
            // All conflicts resolved by heuristics — write clean output.
            let bytes = match encode_output(&clean_output) {
                Ok(bytes) => bytes,
                Err(error) => return handle_encoding_conflict(config, &local_bytes, &error),
            };
            write_merged_output(config, &bytes)?;
            let display_name = merged_display_name(config);
            Ok(MergetoolRunResult {
                stdout: String::new(),
                stderr: format!("Auto-resolved {display_name}\n"),
                exit_code: exit_code::SUCCESS,
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
        })
    }
}

/// The inputs decoded to UTF-8, and the format to write the result in, when
/// they are text that is not all UTF-8. The local side (the file as it was)
/// decides the encoding; `None` leaves the bytes to the byte-level merge, which
/// treats anything undecodable as binary.
fn decode_merge_inputs(
    base: Option<&[u8]>,
    local: &[u8],
    remote: &[u8],
) -> Option<(Option<String>, String, String, TextFormat)> {
    let sides = || base.into_iter().chain([local, remote]);
    if sides().all(|bytes| std::str::from_utf8(bytes).is_ok()) {
        return None;
    }
    let attributes = TextAttributes::default();
    let decoded_local = decode_bytes(local, SideKind::Worktree, &attributes, None);
    if !decoded_local.format.is_writable() {
        return None;
    }
    let output_format = decoded_local.format.format;
    let decode_side = |bytes: &[u8]| -> Option<String> {
        // Each input can announce its own encoding. Only BOM-less sides
        // inherit LOCAL's encoding; the shared decoder also rejects UTF-32.
        let encoding =
            TextEncoding::for_bom(bytes).map_or(output_format.encoding, |(encoding, _)| encoding);
        let decoded = decode_bytes(bytes, SideKind::Worktree, &attributes, Some(encoding));
        decoded
            .format
            .is_writable()
            .then(|| decoded.text.into_owned())
    };
    Some((
        match base {
            Some(base) => Some(decode_side(base)?),
            None => None,
        },
        decoded_local.text.into_owned(),
        decode_side(remote)?,
        output_format,
    ))
}

/// Extract a human-readable display name from the MERGED output path.
fn merged_display_name(config: &MergetoolConfig) -> String {
    config
        .merged
        .file_name()
        .and_then(|n| n.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", config.merged))
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
        .and_then(|name| name.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{path:?}"))
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
    let filename = merged_display_name(config);

    if local_bytes == remote_bytes {
        write_merged_output(config, local_bytes)?;
        return Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {filename} (binary identical on both sides)\n"),
            exit_code: exit_code::SUCCESS,
        });
    }

    if has_base && local_bytes == base_bytes && remote_bytes != base_bytes {
        write_merged_output(config, remote_bytes)?;
        return Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {filename} (binary remote changed from base)\n"),
            exit_code: exit_code::SUCCESS,
        });
    }

    if has_base && remote_bytes == base_bytes && local_bytes != base_bytes {
        write_merged_output(config, local_bytes)?;
        return Ok(MergetoolRunResult {
            stdout: String::new(),
            stderr: format!("Auto-merged {filename} (binary local changed from base)\n"),
            exit_code: exit_code::SUCCESS,
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

/// An encoding mismatch needs a user's decision, just like a binary conflict.
/// Preserve LOCAL exactly and leave Git's stages unresolved.
fn handle_encoding_conflict(
    config: &MergetoolConfig,
    local: &[u8],
    error: &str,
) -> Result<MergetoolRunResult, String> {
    write_merged_output(config, local)?;
    Ok(MergetoolRunResult {
        stdout: String::new(),
        stderr: format!(
            "CONFLICT (encoding): {} — keeping local version.\n{error}\n",
            merged_display_name(config)
        ),
        exit_code: exit_code::CANCELED,
    })
}

#[cfg(test)]
mod tests;
