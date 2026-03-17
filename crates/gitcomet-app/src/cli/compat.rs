use super::*;

#[derive(Clone, Copy)]
enum CompatLabelSlot {
    L1,
    L2,
    L3,
}

fn parse_numbered_label_flag(token: &str) -> Option<CompatLabelSlot> {
    let number = token
        .strip_prefix("--L")
        .or_else(|| token.strip_prefix("-L"))?;
    if number.len() != 1 {
        return None;
    }
    match number {
        "1" => Some(CompatLabelSlot::L1),
        "2" => Some(CompatLabelSlot::L2),
        "3" => Some(CompatLabelSlot::L3),
        _ => None,
    }
}

fn parse_attached_numbered_label(token: &str) -> Option<(CompatLabelSlot, String)> {
    if let Some(value) = token.strip_prefix("--L1=") {
        return Some((CompatLabelSlot::L1, value.to_string()));
    }
    if let Some(value) = token.strip_prefix("--L2=") {
        return Some((CompatLabelSlot::L2, value.to_string()));
    }
    if let Some(value) = token.strip_prefix("--L3=") {
        return Some((CompatLabelSlot::L3, value.to_string()));
    }
    if let Some(rest) = token.strip_prefix("-L") {
        let mut chars = rest.chars();
        let number = chars.next()?;
        let slot = match number {
            '1' => CompatLabelSlot::L1,
            '2' => CompatLabelSlot::L2,
            '3' => CompatLabelSlot::L3,
            _ => return None,
        };
        let remainder = chars.as_str();
        if remainder.is_empty() {
            return None;
        }
        let value = remainder.strip_prefix('=').unwrap_or(remainder);
        return Some((slot, value.to_string()));
    }
    None
}

#[derive(Default)]
struct CompatLabels {
    l1: Option<String>,
    l2: Option<String>,
    l3: Option<String>,
}

impl CompatLabels {
    fn assign_numbered(&mut self, slot: CompatLabelSlot, value: String) {
        match slot {
            CompatLabelSlot::L1 => self.l1 = Some(value),
            CompatLabelSlot::L2 => self.l2 = Some(value),
            CompatLabelSlot::L3 => self.l3 = Some(value),
        }
    }

    fn assign_next(&mut self, value: String) -> Result<(), String> {
        if self.l1.is_none() {
            self.l1 = Some(value);
            return Ok(());
        }
        if self.l2.is_none() {
            self.l2 = Some(value);
            return Ok(());
        }
        if self.l3.is_none() {
            self.l3 = Some(value);
            return Ok(());
        }
        Err("Invalid external invocation: too many label flags; expected at most 3 labels across --L1/--L2/--L3 and -L/--label.".to_string())
    }
}

enum CompatToken<'a> {
    Auto,
    AutoMerge,
    NumberedLabelFlag {
        slot: CompatLabelSlot,
        flag: &'a str,
    },
    NextLabelFlag {
        flag: &'a str,
    },
    AttachedNumberedLabel {
        slot: CompatLabelSlot,
        value: String,
    },
    AttachedNextLabel {
        value: String,
    },
    OutputFlag {
        flag: &'a str,
    },
    AttachedOutput {
        path: PathBuf,
    },
    BaseFlag {
        flag: &'a str,
    },
    AttachedBase {
        path: PathBuf,
    },
    DoubleDash,
    UnknownFlag,
    Positional,
}

fn classify_compat_token(token: &str) -> CompatToken<'_> {
    if token == "--auto" {
        return CompatToken::Auto;
    }

    if token == "--auto-merge" {
        return CompatToken::AutoMerge;
    }

    if let Some(slot) = parse_numbered_label_flag(token) {
        return CompatToken::NumberedLabelFlag { slot, flag: token };
    }

    if token == "-L" || token == "--label" {
        return CompatToken::NextLabelFlag { flag: token };
    }

    if let Some((slot, value)) = parse_attached_numbered_label(token) {
        return CompatToken::AttachedNumberedLabel { slot, value };
    }

    if let Some(value) = token.strip_prefix("--label=") {
        return CompatToken::AttachedNextLabel {
            value: value.to_string(),
        };
    }

    if token == "-o" || token == "--output" || token == "--out" {
        return CompatToken::OutputFlag { flag: token };
    }

    if token == "--base" {
        return CompatToken::BaseFlag { flag: token };
    }

    if let Some(value) = token.strip_prefix("--output=") {
        return CompatToken::AttachedOutput {
            path: PathBuf::from(value),
        };
    }
    if let Some(value) = token.strip_prefix("--out=") {
        return CompatToken::AttachedOutput {
            path: PathBuf::from(value),
        };
    }
    if let Some(value) = token.strip_prefix("--base=") {
        return CompatToken::AttachedBase {
            path: PathBuf::from(value),
        };
    }
    if token.starts_with("-o") && token.len() > 2 {
        return CompatToken::AttachedOutput {
            path: PathBuf::from(&token[2..]),
        };
    }
    if token.starts_with("-L") && token.len() > 2 {
        return CompatToken::AttachedNextLabel {
            value: token[2..].to_string(),
        };
    }

    if token == "--" {
        return CompatToken::DoubleDash;
    }

    if token.starts_with('-') {
        return CompatToken::UnknownFlag;
    }

    CompatToken::Positional
}

struct CompatArgCursor<'a> {
    raw_args: &'a [OsString],
    idx: usize,
}

impl<'a> CompatArgCursor<'a> {
    fn new(raw_args: &'a [OsString]) -> Self {
        Self { raw_args, idx: 0 }
    }

    fn next(&mut self) -> Option<&'a OsString> {
        let arg = self.raw_args.get(self.idx)?;
        self.idx += 1;
        Some(arg)
    }

    fn next_value(&mut self, flag: &str) -> Result<&'a OsString, String> {
        self.next().ok_or_else(|| {
            format!("Missing value for compatibility flag {flag} in external tool mode")
        })
    }

    fn take_remaining_paths(&mut self) -> Vec<PathBuf> {
        let remaining = self.raw_args[self.idx..]
            .iter()
            .map(PathBuf::from)
            .collect();
        self.idx = self.raw_args.len();
        remaining
    }
}

#[derive(Default)]
struct CompatParseState {
    labels: CompatLabels,
    base_flag: Option<PathBuf>,
    merged_output: Option<PathBuf>,
    positionals: Vec<PathBuf>,
    has_auto: bool,
    has_auto_merge: bool,
    has_kdiff3_label_flags: bool,
}

fn parse_compat_external_args(raw_args: &[OsString]) -> Result<Option<CompatParseState>, String> {
    let mut cursor = CompatArgCursor::new(raw_args);
    let mut state = CompatParseState::default();

    while let Some(arg) = cursor.next() {
        let Some(token) = arg.to_str() else {
            state.positionals.push(PathBuf::from(arg));
            continue;
        };

        match classify_compat_token(token) {
            CompatToken::Auto => state.has_auto = true,
            CompatToken::AutoMerge => state.has_auto_merge = true,
            CompatToken::NumberedLabelFlag { slot, flag } => {
                let value = cursor.next_value(flag)?;
                state
                    .labels
                    .assign_numbered(slot, compat_label_arg_text(value, flag)?);
                state.has_kdiff3_label_flags = true;
            }
            CompatToken::NextLabelFlag { flag } => {
                let value = cursor.next_value(flag)?;
                state
                    .labels
                    .assign_next(compat_label_arg_text(value, flag)?)?;
            }
            CompatToken::AttachedNumberedLabel { slot, value } => {
                state.labels.assign_numbered(slot, value);
                state.has_kdiff3_label_flags = true;
            }
            CompatToken::AttachedNextLabel { value } => state.labels.assign_next(value)?,
            CompatToken::OutputFlag { flag } => {
                state.merged_output = Some(PathBuf::from(cursor.next_value(flag)?));
            }
            CompatToken::AttachedOutput { path } => state.merged_output = Some(path),
            CompatToken::BaseFlag { flag } => {
                state.base_flag = Some(PathBuf::from(cursor.next_value(flag)?));
            }
            CompatToken::AttachedBase { path } => state.base_flag = Some(path),
            CompatToken::DoubleDash => {
                state.positionals.extend(cursor.take_remaining_paths());
                break;
            }
            CompatToken::UnknownFlag => return Ok(None),
            CompatToken::Positional => state.positionals.push(PathBuf::from(arg)),
        }
    }

    Ok(Some(state))
}

fn finish_compat_external_mode(
    mut state: CompatParseState,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<AppMode>, String> {
    if state.has_auto && state.merged_output.is_none() {
        return Err(
            "Invalid external merge invocation: --auto requires -o/--output/--out <MERGED>."
                .to_string(),
        );
    }

    if state.has_auto_merge && state.merged_output.is_none() {
        return Err(
            "Invalid external merge invocation: --auto-merge requires -o/--output/--out <MERGED>."
                .to_string(),
        );
    }

    let Some(merged) = state.merged_output.take() else {
        return finish_compat_diff_mode(state, env);
    };

    finish_compat_merge_mode(state, merged, env, git_config).map(Some)
}

fn finish_compat_merge_mode(
    state: CompatParseState,
    merged: PathBuf,
    env: &dyn EnvLookup,
    git_config: &dyn Fn(&str) -> Option<String>,
) -> Result<AppMode, String> {
    let CompatParseState {
        labels:
            CompatLabels {
                l1: label_l1,
                l2: label_l2,
                l3: label_l3,
            },
        base_flag,
        merged_output: _,
        mut positionals,
        has_auto,
        has_auto_merge,
        has_kdiff3_label_flags,
    } = state;

    let (base, local, remote, label_base, label_local, label_remote) = if let Some(explicit_base) =
        base_flag
    {
        match positionals.as_mut_slice() {
            [local, remote] => (
                Some(explicit_base),
                std::mem::take(local),
                std::mem::take(remote),
                label_l1,
                label_l2,
                label_l3,
            ),
            [] | [_] => {
                return Err("Invalid external merge invocation: expected exactly 2 positional paths (LOCAL REMOTE) when --base is provided.".to_string());
            }
            _ => {
                return Err("Invalid external merge invocation: --base already supplies BASE; expected exactly 2 positional paths (LOCAL REMOTE).".to_string());
            }
        }
    } else {
        match positionals.as_mut_slice() {
            [first, second, third] => {
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
                        Some(std::mem::take(first)),
                        std::mem::take(second),
                        std::mem::take(third),
                        label_l1,
                        label_l2,
                        label_l3,
                    )
                } else {
                    (
                        Some(std::mem::take(second)),
                        std::mem::take(first),
                        std::mem::take(third),
                        label_l2,
                        label_l1,
                        label_l3,
                    )
                }
            }
            [local, remote] => {
                if label_l3.is_some() {
                    return Err("Invalid external merge invocation: --L3 requires BASE input. Provide --base <BASE> or 3 positional paths (BASE LOCAL REMOTE).".to_string());
                }
                (
                    None,
                    std::mem::take(local),
                    std::mem::take(remote),
                    None,
                    label_l1,
                    label_l2,
                )
            }
            [] | [_] => {
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
    resolve_mergetool_with_config(args, env, git_config).map(AppMode::Mergetool)
}

fn finish_compat_diff_mode(
    state: CompatParseState,
    env: &dyn EnvLookup,
) -> Result<Option<AppMode>, String> {
    let CompatParseState {
        labels:
            CompatLabels {
                l1: label_l1,
                l2: label_l2,
                l3: label_l3,
            },
        base_flag,
        merged_output: _,
        mut positionals,
        has_auto: _,
        has_auto_merge: _,
        has_kdiff3_label_flags: _,
    } = state;

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

    match positionals.as_mut_slice() {
        [local, remote] => {
            let args = DifftoolArgs {
                local: Some(std::mem::take(local)),
                remote: Some(std::mem::take(remote)),
                path: None,
                label_left: label_l1,
                label_right: label_l2,
                gui: false,
            };
            resolve_difftool_with_env(args, env)
                .map(AppMode::Difftool)
                .map(Some)
        }
        [_, _, ..] => Err("Invalid external diff invocation: too many positional paths; expected exactly 2 (LOCAL REMOTE). Use -o/--output/--out for merge mode.".to_string()),
        _ => Ok(None),
    }
}

use crate::hex_encode;

fn compat_log_arg_text(arg: &std::ffi::OsStr) -> String {
    if let Some(text) = arg.to_str() {
        return text.to_string();
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        format!("gitcomet-argv-bytes:{}", hex_encode(arg.as_bytes()))
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let mut bytes = Vec::new();
        for unit in arg.encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        format!("gitcomet-argv-utf16le:{}", hex_encode(&bytes))
    }

    #[cfg(not(any(unix, windows)))]
    {
        format!("{arg:?}")
    }
}

fn compat_label_arg_text(value: &OsString, flag: &str) -> Result<String, String> {
    value
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("Invalid external invocation: {flag} value is not valid UTF-8 text"))
}

fn compat_argv_log_dir() -> PathBuf {
    std::env::temp_dir().join("gitcomet-compat-argv")
}

fn resolve_compat_argv_log_path(env: &dyn EnvLookup) -> Option<PathBuf> {
    let requested = PathBuf::from(env.var_os("GITCOMET_COMPAT_ARGV_LOG")?);
    let mut components = requested.components();
    let Some(std::path::Component::Normal(file_name)) = components.next() else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    Some(compat_argv_log_dir().join(file_name))
}

fn maybe_record_compat_argv(raw_args: &[OsString], env: &dyn EnvLookup) {
    let Some(path) = resolve_compat_argv_log_path(env) else {
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut dump = String::new();
    for arg in raw_args {
        dump.push_str(&compat_log_arg_text(arg.as_os_str()));
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
    let Some(state) = parse_compat_external_args(raw_args)? else {
        return Ok(None);
    };
    finish_compat_external_mode(state, env, git_config)
}

pub(super) fn normalize_empty_mergetool_base_arg(args: &[OsString]) -> Vec<OsString> {
    let mut normalized = Vec::with_capacity(args.len());
    let mut in_mergetool_subcommand = false;
    let mut idx = 0usize;

    while idx < args.len() {
        let token = args[idx].to_str();

        if !in_mergetool_subcommand && token == Some("mergetool") {
            in_mergetool_subcommand = true;
            normalized.push(args[idx].clone());
            idx += 1;
            continue;
        }

        if in_mergetool_subcommand
            && token == Some("--base")
            && let Some(next) = args.get(idx + 1)
            && next.is_empty()
        {
            // Accept shell-expanded empty `--base "$BASE"` as "no base"
            // for add/add and other no-base conflict scenarios.
            idx += 2;
            continue;
        }

        if in_mergetool_subcommand && token == Some("--base=") {
            // Treat explicit empty attached form (`--base=`) as omitted.
            idx += 1;
            continue;
        }

        normalized.push(args[idx].clone());
        idx += 1;
    }

    normalized
}
