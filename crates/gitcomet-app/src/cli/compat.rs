use super::*;

fn assign_next_compat_label(
    label_l1: &mut Option<String>,
    label_l2: &mut Option<String>,
    label_l3: &mut Option<String>,
    value: String,
) -> Result<(), String> {
    if label_l1.is_none() {
        *label_l1 = Some(value);
        return Ok(());
    }
    if label_l2.is_none() {
        *label_l2 = Some(value);
        return Ok(());
    }
    if label_l3.is_none() {
        *label_l3 = Some(value);
        return Ok(());
    }
    Err("Invalid external invocation: too many label flags; expected at most 3 labels across --L1/--L2/--L3 and -L/--label.".to_string())
}

fn maybe_record_compat_argv(raw_args: &[OsString], env: &dyn EnvLookup) {
    let Some(path) = env.var_os("GITCOMET_COMPAT_ARGV_LOG").map(PathBuf::from) else {
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut dump = String::new();
    for arg in raw_args {
        dump.push_str(&arg.to_string_lossy());
        dump.push('\n');
    }

    let _ = std::fs::write(path, dump);
}

pub(super) fn parse_compat_external_mode_with_config(
    raw_args: &[OsString],
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<AppMode>, String> {
    maybe_record_compat_argv(raw_args, env);

    let mut label_l1: Option<String> = None;
    let mut label_l2: Option<String> = None;
    let mut label_l3: Option<String> = None;
    let mut base_flag: Option<PathBuf> = None;
    let mut merged_output: Option<PathBuf> = None;
    let mut positionals: Vec<PathBuf> = Vec::new();
    let mut has_auto = false;
    let mut has_auto_merge = false;
    let mut has_kdiff3_label_flags = false;

    let mut idx = 0usize;
    while idx < raw_args.len() {
        let arg = &raw_args[idx];
        let token = arg.to_string_lossy();

        if token == "--auto" {
            has_auto = true;
            idx += 1;
            continue;
        }

        if token == "--auto-merge" {
            has_auto_merge = true;
            idx += 1;
            continue;
        }

        if token == "--L1" || token == "--L2" || token == "--L3" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            let value = value.to_string_lossy().into_owned();
            match token.as_ref() {
                "--L1" => label_l1 = Some(value),
                "--L2" => label_l2 = Some(value),
                "--L3" => label_l3 = Some(value),
                _ => unreachable!(),
            }
            has_kdiff3_label_flags = true;
            idx += 2;
            continue;
        }

        if token == "-L1" || token == "-L2" || token == "-L3" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            let value = value.to_string_lossy().into_owned();
            match token.as_ref() {
                "-L1" => label_l1 = Some(value),
                "-L2" => label_l2 = Some(value),
                "-L3" => label_l3 = Some(value),
                _ => unreachable!(),
            }
            has_kdiff3_label_flags = true;
            idx += 2;
            continue;
        }

        if token == "-L" || token == "--label" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            assign_next_compat_label(
                &mut label_l1,
                &mut label_l2,
                &mut label_l3,
                value.to_string_lossy().into_owned(),
            )?;
            idx += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--L1=") {
            label_l1 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--L2=") {
            label_l2 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--L3=") {
            label_l3 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("-L1=") {
            label_l1 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("-L2=") {
            label_l2 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("-L3=") {
            label_l3 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("-L1")
            && !value.is_empty()
        {
            label_l1 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("-L2")
            && !value.is_empty()
        {
            label_l2 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("-L3")
            && !value.is_empty()
        {
            label_l3 = Some(value.to_string());
            has_kdiff3_label_flags = true;
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--label=") {
            assign_next_compat_label(
                &mut label_l1,
                &mut label_l2,
                &mut label_l3,
                value.to_string(),
            )?;
            idx += 1;
            continue;
        }

        if token == "-o" || token == "--output" || token == "--out" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                format!("Missing value for compatibility flag {token} in external tool mode")
            })?;
            merged_output = Some(PathBuf::from(value));
            idx += 2;
            continue;
        }

        if token == "--base" {
            let next_idx = idx + 1;
            let value = raw_args.get(next_idx).ok_or_else(|| {
                "Missing value for compatibility flag --base in external tool mode".to_string()
            })?;
            base_flag = Some(PathBuf::from(value));
            idx += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--output=") {
            merged_output = Some(PathBuf::from(value));
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--out=") {
            merged_output = Some(PathBuf::from(value));
            idx += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--base=") {
            base_flag = Some(PathBuf::from(value));
            idx += 1;
            continue;
        }
        if token.starts_with("-o") && token.len() > 2 {
            merged_output = Some(PathBuf::from(token[2..].to_string()));
            idx += 1;
            continue;
        }
        if token.starts_with("-L") && token.len() > 2 {
            assign_next_compat_label(
                &mut label_l1,
                &mut label_l2,
                &mut label_l3,
                token[2..].to_string(),
            )?;
            idx += 1;
            continue;
        }

        if token == "--" {
            positionals.extend(raw_args[idx + 1..].iter().map(PathBuf::from));
            idx = raw_args.len();
            continue;
        }

        if token.starts_with('-') {
            return Ok(None);
        }

        positionals.push(PathBuf::from(arg));
        idx += 1;
    }

    if has_auto && merged_output.is_none() {
        return Err(
            "Invalid external merge invocation: --auto requires -o/--output/--out <MERGED>."
                .to_string(),
        );
    }

    if has_auto_merge && merged_output.is_none() {
        return Err(
            "Invalid external merge invocation: --auto-merge requires -o/--output/--out <MERGED>."
                .to_string(),
        );
    }

    if let Some(merged) = merged_output {
        let (base, local, remote, label_base, label_local, label_remote) = if let Some(
            explicit_base,
        ) = base_flag
        {
            match positionals.len() {
                2 => (
                    Some(explicit_base),
                    positionals[0].clone(),
                    positionals[1].clone(),
                    label_l1,
                    label_l2,
                    label_l3,
                ),
                0 | 1 => {
                    return Err("Invalid external merge invocation: expected exactly 2 positional paths (LOCAL REMOTE) when --base is provided.".to_string());
                }
                _ => {
                    return Err("Invalid external merge invocation: --base already supplies BASE; expected exactly 2 positional paths (LOCAL REMOTE).".to_string());
                }
            }
        } else {
            match positionals.len() {
                3 => {
                    // Ambiguous 3-path merge-mode compatibility input:
                    // - KDiff3 style: BASE LOCAL REMOTE
                    // - Meld style:   LOCAL BASE REMOTE
                    //
                    // Prefer KDiff3 order when KDiff3-specific hints are
                    // present (`--auto`/`--L*`). Otherwise default to Meld's
                    // LOCAL BASE REMOTE ordering for broad path-override
                    // compatibility.
                    if has_auto || has_kdiff3_label_flags {
                        (
                            Some(positionals[0].clone()),
                            positionals[1].clone(),
                            positionals[2].clone(),
                            label_l1,
                            label_l2,
                            label_l3,
                        )
                    } else {
                        (
                            Some(positionals[1].clone()),
                            positionals[0].clone(),
                            positionals[2].clone(),
                            label_l2,
                            label_l1,
                            label_l3,
                        )
                    }
                }
                2 => {
                    if label_l3.is_some() {
                        return Err("Invalid external merge invocation: --L3 requires BASE input. Provide --base <BASE> or 3 positional paths (BASE LOCAL REMOTE).".to_string());
                    }
                    (
                        None,
                        positionals[0].clone(),
                        positionals[1].clone(),
                        None,
                        label_l1,
                        label_l2,
                    )
                }
                0 | 1 => {
                    return Err("Invalid external merge invocation: expected 2 positional paths (LOCAL REMOTE) or 3 (BASE LOCAL REMOTE) after -o/--output/--out.".to_string());
                }
                _ => {
                    return Err("Invalid external merge invocation: too many positional paths; expected 2 (LOCAL REMOTE) or 3 (BASE LOCAL REMOTE).".to_string());
                }
            }
        };

        let args = MergetoolArgs {
            merged: Some(merged),
            local: Some(local),
            remote: Some(remote),
            base,
            label_base,
            label_local,
            label_remote,
            conflict_style: None,
            diff_algorithm: None,
            marker_size: None,
            auto: has_auto || has_auto_merge,
            gui: false,
        };
        return resolve_mergetool_with_config(args, env, git_config)
            .map(AppMode::Mergetool)
            .map(Some);
    }

    if base_flag.is_some() {
        return Err(
            "Invalid external diff invocation: --base is only valid for merge mode with -o/--output/--out."
                .to_string(),
        );
    }

    if label_l3.is_some() {
        return Err(
            "Invalid external diff invocation: --L3 is only valid for merge mode with -o/--output/--out."
                .to_string(),
        );
    }

    if positionals.is_empty() && (label_l1.is_some() || label_l2.is_some()) {
        return Err(
            "Invalid external diff invocation: expected 2 positional paths (LOCAL REMOTE)."
                .to_string(),
        );
    }

    if positionals.len() == 2 {
        let args = DifftoolArgs {
            local: Some(positionals[0].clone()),
            remote: Some(positionals[1].clone()),
            path: None,
            label_left: label_l1,
            label_right: label_l2,
            gui: false,
        };
        return resolve_difftool_with_env(args, env)
            .map(AppMode::Difftool)
            .map(Some);
    }

    if positionals.len() > 2 {
        return Err("Invalid external diff invocation: too many positional paths; expected exactly 2 (LOCAL REMOTE). Use -o/--output/--out for merge mode.".to_string());
    }

    Ok(None)
}

pub(super) fn normalize_empty_mergetool_base_arg(args: &[OsString]) -> Vec<OsString> {
    let mut normalized = Vec::with_capacity(args.len());
    let mut in_mergetool_subcommand = false;
    let mut idx = 0usize;

    while idx < args.len() {
        let token = args[idx].to_string_lossy();

        if !in_mergetool_subcommand && token == "mergetool" {
            in_mergetool_subcommand = true;
            normalized.push(args[idx].clone());
            idx += 1;
            continue;
        }

        if in_mergetool_subcommand
            && token == "--base"
            && let Some(next) = args.get(idx + 1)
            && next.is_empty()
        {
            // Accept shell-expanded empty `--base "$BASE"` as "no base"
            // for add/add and other no-base conflict scenarios.
            idx += 2;
            continue;
        }

        if in_mergetool_subcommand && token == "--base=" {
            // Treat explicit empty attached form (`--base=`) as omitted.
            idx += 1;
            continue;
        }

        normalized.push(args[idx].clone());
        idx += 1;
    }

    normalized
}
