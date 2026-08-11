//! Argument conventions for git's built-in merge tools.
//!
//! When a tool has no `mergetool.<tool>.cmd` configured, git does not simply
//! hand the four files to the executable: every built-in tool ships a
//! `merge_cmd` in `$(git --exec-path)/mergetools/<tool>` that spells out the
//! flags which put the tool into *merge* mode (an output file, a 3-way layout,
//! window labels, ...). Passing bare positional paths instead leaves tools like
//! KDiff3 in read-only 3-way *diff* mode with nowhere to write the result.
//!
//! This module mirrors those `merge_cmd` definitions so GitComet launches the
//! same command line git would.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// Files handed to a merge tool, mirroring git's `$BASE`/`$LOCAL`/`$REMOTE`/`$MERGED`.
pub(super) struct MergetoolFiles<'a> {
    pub base: &'a Path,
    pub local: &'a Path,
    pub remote: &'a Path,
    /// Absolute path of the conflicted worktree file the tool must write.
    pub merged: &'a Path,
    /// Repo-relative path of the conflicted file, used for window labels only.
    pub merged_label: &'a Path,
    /// Whether the index actually carries a stage 1 (merge base) entry. Tools
    /// take a different, two-way command line when the base is missing.
    pub base_present: bool,
}

/// Result of looking up a tool in the built-in table.
pub(super) enum BuiltinMergeCommand {
    /// Argument vector matching git's built-in convention for this tool.
    Args(Vec<OsString>),
    /// A known built-in tool that cannot perform this merge.
    Unsupported(String),
    /// Not one of git's built-ins; the caller picks a generic convention.
    Unknown,
}

macro_rules! argv {
    ($($item:expr),* $(,)?) => {
        vec![$(OsString::from($item)),*]
    };
}

/// Build the argument vector git's `mergetools/<tool>` script would use.
pub(super) fn builtin_merge_command(tool: &str, files: &MergetoolFiles<'_>) -> BuiltinMergeCommand {
    let Some(key) = builtin_tool_key(tool) else {
        return BuiltinMergeCommand::Unknown;
    };

    let MergetoolFiles {
        base,
        local,
        remote,
        merged,
        merged_label,
        base_present,
    } = *files;

    let args = match key {
        "kdiff3" => {
            if base_present {
                argv![
                    "--auto",
                    "--L1",
                    label(merged_label, " (Base)"),
                    "--L2",
                    label(merged_label, " (Local)"),
                    "--L3",
                    label(merged_label, " (Remote)"),
                    "-o",
                    merged,
                    base,
                    local,
                    remote,
                ]
            } else {
                argv![
                    "--auto",
                    "--L1",
                    label(merged_label, " (Local)"),
                    "--L2",
                    label(merged_label, " (Remote)"),
                    "-o",
                    merged,
                    local,
                    remote,
                ]
            }
        }
        // git probes `meld --help` for `--output` support; every meld release
        // since 1.5 has it, so take the modern branch unconditionally.
        "meld" => argv![prefixed("--output=", merged), local, base, remote],
        "bc" => {
            if base_present {
                argv![local, remote, base, prefixed("-mergeoutput=", merged)]
            } else {
                argv![local, remote, prefixed("-mergeoutput=", merged)]
            }
        }
        // git synthesises a virtual base when stage 1 is missing; the empty
        // BASE file GitComet materialises serves the same purpose here.
        "p4merge" => argv![base, remote, local, merged],
        "diffmerge" => {
            if base_present {
                argv![
                    "--merge",
                    prefixed("--result=", merged),
                    local,
                    base,
                    remote
                ]
            } else {
                argv!["--merge", prefixed("--result=", merged), local, remote]
            }
        }
        "tkdiff" => {
            if base_present {
                argv!["-a", base, "-o", merged, local, remote]
            } else {
                argv!["-o", merged, local, remote]
            }
        }
        "xxdiff" => {
            let mut args = argv![
                "-X",
                "--show-merged-pane",
                "-R",
                "Accel.SaveAsMerged: \"Ctrl+S\"",
                "-R",
                "Accel.Search: \"Ctrl+F\"",
                "-R",
                "Accel.SearchForward: \"Ctrl+G\"",
                "--merged-file",
                merged,
                local,
            ];
            if base_present {
                args.push(base.into());
            }
            args.push(remote.into());
            args
        }
        "opendiff" => {
            if base_present {
                argv![local, remote, "-ancestor", base, "-merge", merged]
            } else {
                argv![local, remote, "-merge", merged]
            }
        }
        "winmerge" => argv![
            "-u", "-e", "-dl", "Local", "-dr", "Remote", local, remote, merged
        ],
        "tortoisemerge" => {
            if !base_present {
                return BuiltinMergeCommand::Unsupported(format!(
                    "Merge tool '{tool}' cannot be used without a merge base. \
                     Resolve this conflict in GitComet or configure a different merge.tool."
                ));
            }
            argv![
                prefixed("-base:", base),
                prefixed("-mine:", local),
                prefixed("-theirs:", remote),
                prefixed("-merged:", merged),
            ]
        }
        "tortoisegitmerge" => {
            if !base_present {
                return BuiltinMergeCommand::Unsupported(format!(
                    "Merge tool '{tool}' cannot be used without a merge base. \
                     Resolve this conflict in GitComet or configure a different merge.tool."
                ));
            }
            argv![
                "-base", base, "-mine", local, "-theirs", remote, "-merged", merged,
            ]
        }
        "araxis" => {
            if base_present {
                argv!["-wait", "-merge", "-3", "-a1", base, local, remote, merged]
            } else {
                argv!["-wait", "-2", local, remote, merged]
            }
        }
        "ecmerge" => {
            if base_present {
                argv![
                    base,
                    local,
                    remote,
                    "--default",
                    "--mode=merge3",
                    prefixed("--to=", merged),
                ]
            } else {
                argv![
                    local,
                    remote,
                    "--default",
                    "--mode=merge2",
                    prefixed("--to=", merged),
                ]
            }
        }
        "diffuse" => {
            if base_present {
                argv![local, merged, remote, base]
            } else {
                argv![local, merged, remote]
            }
        }
        "vscode" => argv!["--wait", "--merge", remote, local, base, merged],
        "smerge" => {
            if base_present {
                argv!["mergetool", base, local, remote, "-o", merged]
            } else {
                argv!["mergetool", local, remote, "-o", merged]
            }
        }
        "codecompare" => {
            if base_present {
                argv![
                    prefixed("-MF=", local),
                    prefixed("-TF=", remote),
                    prefixed("-BF=", base),
                    prefixed("-RF=", merged),
                ]
            } else {
                argv![
                    prefixed("-MF=", local),
                    prefixed("-TF=", remote),
                    prefixed("-RF=", merged),
                ]
            }
        }
        "deltawalker" => {
            if base_present {
                argv![local, remote, base, prefixed("-merged=", merged)]
            } else {
                argv![local, remote, prefixed("-merged=", merged)]
            }
        }
        "examdiff" => {
            if base_present {
                argv![
                    "-merge",
                    local,
                    base,
                    remote,
                    prefixed("-o:", merged),
                    "-nh",
                ]
            } else {
                argv!["-merge", local, remote, prefixed("-o:", merged), "-nh"]
            }
        }
        "guiffy" => {
            if base_present {
                argv!["-s", local, remote, base, merged]
            } else {
                argv!["-m", local, remote, merged]
            }
        }
        "emerge" => {
            let merged_name = merged.file_name().unwrap_or(merged.as_os_str());
            if base_present {
                argv![
                    "-f",
                    "emerge-files-with-ancestor-command",
                    local,
                    remote,
                    base,
                    merged_name,
                ]
            } else {
                argv!["-f", "emerge-files-command", local, remote, merged_name]
            }
        }
        // Pre-layout-engine vimdiff convention: it produces the same window
        // arrangement as the default `(LOCAL,BASE,REMOTE)/MERGED` layout and,
        // crucially, opens `$MERGED` for editing.
        "vimdiff" | "gvimdiff" | "nvimdiff" => {
            if base_present {
                argv![
                    "-f",
                    "-d",
                    "-c",
                    "4wincmd w | wincmd J",
                    local,
                    base,
                    remote,
                    merged,
                ]
            } else {
                argv!["-f", "-d", "-c", "wincmd l", local, merged, remote]
            }
        }
        "kompare" => {
            return BuiltinMergeCommand::Unsupported(format!(
                "Merge tool '{tool}' is a diff viewer and cannot merge. \
                 Configure a different merge.tool."
            ));
        }
        _ => return BuiltinMergeCommand::Unknown,
    };

    BuiltinMergeCommand::Args(args)
}

/// Executable name git would run for a built-in tool whose command differs from
/// the tool name (git's `translate_merge_tool_path`). Returns `None` when the
/// tool name is already the command to run, or is not a known built-in.
pub(super) fn builtin_tool_program(tool: &str) -> Option<String> {
    let candidates: &[&str] = match builtin_tool_key(tool)? {
        "bc" => &["bcomp", "bcompare"],
        "vscode" => &["code"],
        "araxis" => &["compare"],
        "emerge" => &["emacs"],
        "deltawalker" => &["DeltaWalker"],
        "codecompare" => &["CodeMerge"],
        "tortoisemerge" => &["tortoisegitmerge", "tortoisemerge"],
        "winmerge" => &["WinMergeU"],
        "examdiff" => &["ExamDiff"],
        "gvimdiff" => &["gvim"],
        "vimdiff" => &["vim"],
        "nvimdiff" => &["nvim"],
        _ => return None,
    };
    Some(first_program_on_path(candidates))
}

/// Map a configured tool name onto a built-in table key.
///
/// Git looks for `mergetools/<tool>` first and falls back to the name with a
/// single trailing digit removed, which is how variants such as `bc3`, `bc4` or
/// `vimdiff2` resolve to their base tool.
fn builtin_tool_key(tool: &str) -> Option<&'static str> {
    const BUILTIN_TOOLS: &[&str] = &[
        "araxis",
        "bc",
        "codecompare",
        "deltawalker",
        "diffmerge",
        "diffuse",
        "ecmerge",
        "emerge",
        "examdiff",
        "guiffy",
        "gvimdiff",
        "kdiff3",
        "kompare",
        "meld",
        "nvimdiff",
        "opendiff",
        "p4merge",
        "smerge",
        "tkdiff",
        "tortoisegitmerge",
        "tortoisemerge",
        "vimdiff",
        "vscode",
        "winmerge",
        "xxdiff",
    ];

    let name = tool.trim().to_ascii_lowercase();
    let lookup = |name: &str| BUILTIN_TOOLS.iter().copied().find(|tool| *tool == name);

    lookup(&name).or_else(|| {
        let stripped = name.strip_suffix(|c: char| c.is_ascii_digit())?;
        lookup(stripped)
    })
}

fn label(path: &Path, suffix: &str) -> OsString {
    let mut label = path.as_os_str().to_os_string();
    label.push(suffix);
    label
}

fn prefixed(prefix: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(prefix);
    arg.push(path);
    arg
}

fn first_program_on_path(candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|candidate| program_exists_on_path(candidate))
        .or_else(|| candidates.last())
        .map(|candidate| (*candidate).to_string())
        .unwrap_or_default()
}

fn program_exists_on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    program_exists_in(&path, program)
}

fn program_exists_in(search_path: &OsStr, program: &str) -> bool {
    std::env::split_paths(search_path).any(|dir| {
        if dir.as_os_str().is_empty() {
            return false;
        }
        executable_suffixes()
            .iter()
            .any(|suffix| is_executable_file(&dir.join(with_suffix(program, suffix))))
    })
}

fn with_suffix(program: &str, suffix: &OsStr) -> OsString {
    let mut name = OsString::from(program);
    name.push(suffix);
    name
}

fn executable_suffixes() -> Vec<OsString> {
    #[cfg(windows)]
    {
        let mut suffixes = vec![OsString::new()];
        if let Some(pathext) = std::env::var_os("PATHEXT") {
            let pathext = pathext.to_string_lossy().into_owned();
            suffixes.extend(
                pathext
                    .split(';')
                    .filter(|ext| !ext.is_empty())
                    .map(OsString::from),
            );
        }
        suffixes
    }
    #[cfg(not(windows))]
    {
        vec![OsString::new()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(base_present: bool) -> MergetoolFiles<'static> {
        MergetoolFiles {
            base: Path::new("./a_BASE_1.txt"),
            local: Path::new("./a_LOCAL_1.txt"),
            remote: Path::new("./a_REMOTE_1.txt"),
            merged: Path::new("/repo/a.txt"),
            merged_label: Path::new("a.txt"),
            base_present,
        }
    }

    fn args(tool: &str, base_present: bool) -> Vec<String> {
        match builtin_merge_command(tool, &files(base_present)) {
            BuiltinMergeCommand::Args(args) => args
                .into_iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            other => panic!(
                "expected built-in args for {tool}, got {}",
                match other {
                    BuiltinMergeCommand::Unsupported(message) => message,
                    _ => "unknown tool".to_string(),
                }
            ),
        }
    }

    #[test]
    fn kdiff3_merge_uses_output_file_and_three_way_labels() {
        assert_eq!(
            args("kdiff3", true),
            vec![
                "--auto",
                "--L1",
                "a.txt (Base)",
                "--L2",
                "a.txt (Local)",
                "--L3",
                "a.txt (Remote)",
                "-o",
                "/repo/a.txt",
                "./a_BASE_1.txt",
                "./a_LOCAL_1.txt",
                "./a_REMOTE_1.txt",
            ]
        );
    }

    #[test]
    fn kdiff3_without_base_uses_two_way_merge() {
        assert_eq!(
            args("kdiff3", false),
            vec![
                "--auto",
                "--L1",
                "a.txt (Local)",
                "--L2",
                "a.txt (Remote)",
                "-o",
                "/repo/a.txt",
                "./a_LOCAL_1.txt",
                "./a_REMOTE_1.txt",
            ]
        );
    }

    #[test]
    fn every_builtin_merge_command_names_the_merged_output() {
        for tool in [
            "kdiff3",
            "meld",
            "bc",
            "p4merge",
            "diffmerge",
            "tkdiff",
            "xxdiff",
            "opendiff",
            "winmerge",
            "araxis",
            "ecmerge",
            "diffuse",
            "vscode",
            "smerge",
            "codecompare",
            "deltawalker",
            "examdiff",
            "guiffy",
            "vimdiff",
        ] {
            for base_present in [true, false] {
                let args = args(tool, base_present);
                assert!(
                    args.iter().any(|arg| arg.contains("/repo/a.txt")),
                    "{tool} (base_present={base_present}) must pass the merged output path: {args:?}"
                );
            }
        }
    }

    #[test]
    fn tool_name_variants_fall_back_to_base_tool() {
        assert_eq!(builtin_tool_key("bc3"), Some("bc"));
        assert_eq!(builtin_tool_key("bc4"), Some("bc"));
        assert_eq!(builtin_tool_key("vimdiff3"), Some("vimdiff"));
        assert_eq!(builtin_tool_key("gvimdiff2"), Some("gvimdiff"));
        assert_eq!(builtin_tool_key("KDiff3"), Some("kdiff3"));
        assert_eq!(builtin_tool_key("kdiff"), None);
    }

    #[test]
    fn unknown_tool_leaves_convention_to_caller() {
        assert!(matches!(
            builtin_merge_command("fake", &files(true)),
            BuiltinMergeCommand::Unknown
        ));
    }

    #[test]
    fn diff_only_and_base_requiring_tools_report_unsupported() {
        assert!(matches!(
            builtin_merge_command("kompare", &files(true)),
            BuiltinMergeCommand::Unsupported(message) if message.contains("cannot merge")
        ));
        assert!(matches!(
            builtin_merge_command("tortoisemerge", &files(false)),
            BuiltinMergeCommand::Unsupported(message) if message.contains("without a merge base")
        ));
        assert!(matches!(
            builtin_merge_command("tortoisemerge", &files(true)),
            BuiltinMergeCommand::Args(_)
        ));
    }

    #[test]
    fn builtin_tool_program_translates_only_renamed_tools() {
        assert_eq!(builtin_tool_program("kdiff3"), None);
        assert_eq!(builtin_tool_program("meld"), None);
        assert_eq!(builtin_tool_program("fake"), None);
        assert_eq!(builtin_tool_program("vscode").as_deref(), Some("code"));
        assert_eq!(builtin_tool_program("emerge").as_deref(), Some("emacs"));
        // `bc3`/`bc4` are variants of the same Beyond Compare launcher.
        let bc = builtin_tool_program("bc3").expect("bc variant should translate");
        assert!(bc == "bcomp" || bc == "bcompare", "{bc}");
    }

    #[test]
    fn program_lookup_finds_executables_on_the_search_path() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("gitcomet-fake-mergetool");
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let search_path = dir.path().as_os_str();
        assert!(program_exists_in(search_path, "gitcomet-fake-mergetool"));
        assert!(!program_exists_in(
            search_path,
            "gitcomet-missing-mergetool"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn program_lookup_ignores_non_executable_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gitcomet-not-executable"), b"data").unwrap();

        assert!(!program_exists_in(
            dir.path().as_os_str(),
            "gitcomet-not-executable"
        ));
    }
}
