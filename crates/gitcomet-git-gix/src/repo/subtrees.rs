use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_raw_output,
    run_git_with_output,
};
use gitcomet_core::domain::{Subtree, SubtreeSourceConfig};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SUBTREE_CONFIG_KEY_PREFIX: &str = "gitcomet.subtree.";
const SUBTREE_PATH_FIELD: &str = "path";
const SUBTREE_REPOSITORY_FIELD: &str = "repository";
const SUBTREE_REFERENCE_FIELD: &str = "ref";
const SUBTREE_PUSH_REFSPEC_FIELD: &str = "pushrefspec";
const SUBTREE_SQUASH_FIELD: &str = "squash";

#[derive(Clone, Debug, Default)]
struct StoredSubtreeConfigRow {
    path: Option<PathBuf>,
    repository: Option<String>,
    reference: Option<String>,
    push_refspec: Option<String>,
    squash: Option<bool>,
}

impl GixRepo {
    pub(super) fn list_subtrees_impl(&self) -> Result<Vec<Subtree>> {
        let mut source_configs = self.read_stored_subtree_source_configs()?;
        let discovered_paths = self.discover_subtree_paths()?;
        Ok(discovered_paths
            .into_iter()
            .map(|path| Subtree {
                source: source_configs.remove(&path),
                path,
            })
            .collect())
    }

    pub(super) fn add_subtree_with_output_impl(
        &self,
        repository: &str,
        reference: &str,
        path: &Path,
        squash: bool,
    ) -> Result<CommandOutput> {
        let path_text = path_to_config_value(path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("protocol.file.allow=always")
            .arg("subtree")
            .arg("add")
            .arg("--prefix")
            .arg(&path_text);
        if squash {
            cmd.arg("--squash");
        }
        cmd.arg(repository).arg(reference);
        let output = map_subtree_command_error(run_git_with_output(
            cmd,
            &format!("git subtree add --prefix={path_text} {repository} {reference}"),
        ))?;

        let existing = self.read_stored_subtree_source_configs()?;
        let source = SubtreeSourceConfig {
            repository: repository.to_string(),
            reference: reference.to_string(),
            push_refspec: existing.get(path).and_then(|cfg| cfg.push_refspec.clone()),
            squash,
        };
        self.write_stored_subtree_source_config(path, &source)?;
        Ok(output)
    }

    pub(super) fn pull_subtree_with_output_impl(
        &self,
        repository: &str,
        reference: &str,
        path: &Path,
        squash: bool,
    ) -> Result<CommandOutput> {
        let path_text = path_to_config_value(path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("protocol.file.allow=always")
            .arg("subtree")
            .arg("pull")
            .arg("--prefix")
            .arg(&path_text);
        if squash {
            cmd.arg("--squash");
        }
        cmd.arg(repository).arg(reference);
        let output = map_subtree_command_error(run_git_with_output(
            cmd,
            &format!("git subtree pull --prefix={path_text} {repository} {reference}"),
        ))?;

        let existing = self.read_stored_subtree_source_configs()?;
        let source = SubtreeSourceConfig {
            repository: repository.to_string(),
            reference: reference.to_string(),
            push_refspec: existing.get(path).and_then(|cfg| cfg.push_refspec.clone()),
            squash,
        };
        self.write_stored_subtree_source_config(path, &source)?;
        Ok(output)
    }

    pub(super) fn push_subtree_with_output_impl(
        &self,
        repository: &str,
        refspec: &str,
        path: &Path,
    ) -> Result<CommandOutput> {
        let path_text = path_to_config_value(path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("protocol.file.allow=always")
            .arg("subtree")
            .arg("push")
            .arg("--prefix")
            .arg(&path_text)
            .arg(repository)
            .arg(refspec);
        let output = map_subtree_command_error(run_git_with_output(
            cmd,
            &format!("git subtree push --prefix={path_text} {repository} {refspec}"),
        ))?;

        let existing = self.read_stored_subtree_source_configs()?;
        let existing_source = existing.get(path);
        let source = SubtreeSourceConfig {
            repository: existing_source
                .map(|cfg| cfg.repository.clone())
                .unwrap_or_else(|| repository.to_string()),
            reference: existing_source
                .map(|cfg| cfg.reference.clone())
                .unwrap_or_else(|| refspec.to_string()),
            push_refspec: Some(refspec.to_string()),
            squash: existing_source.map(|cfg| cfg.squash).unwrap_or(true),
        };
        self.write_stored_subtree_source_config(path, &source)?;
        Ok(output)
    }

    pub(super) fn split_subtree_with_output_impl(
        &self,
        path: &Path,
        branch: Option<&str>,
    ) -> Result<CommandOutput> {
        let path_text = path_to_config_value(path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("subtree")
            .arg("split")
            .arg("--prefix")
            .arg(&path_text);
        if let Some(branch) = branch {
            cmd.arg("--branch").arg(branch);
        }
        map_subtree_command_error(run_git_with_output(
            cmd,
            &format!("git subtree split --prefix={path_text}"),
        ))
    }

    pub(super) fn remove_subtree_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rm").arg("-r").arg("--").arg(path);
        let output = run_git_with_output(cmd, &format!("git rm -r {}", path.display()))?;
        self.remove_stored_subtree_source_configs_under(path)?;
        Ok(CommandOutput {
            command: format!("Remove subtree {}", path.display()),
            ..output
        })
    }

    fn discover_subtree_paths(&self) -> Result<Vec<PathBuf>> {
        let tracked_directories = self.tracked_directory_paths()?;
        if tracked_directories.is_empty() {
            return Ok(Vec::new());
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("--grep=^git-subtree-dir: ")
            .arg("--format=%B%x1e")
            .arg("HEAD");
        let output = run_git_raw_output(cmd, "git log subtree history")?;
        if !output.status.success() {
            if is_missing_head_output(&output) {
                return Ok(Vec::new());
            }
            return Err(git_command_failed_error("git log subtree history", output));
        }

        let mut paths = BTreeSet::new();
        for record in bytes_to_text_preserving_utf8(&output.stdout).split('\u{001e}') {
            for line in record.lines() {
                let Some(path) = parse_subtree_dir_trailer(line) else {
                    continue;
                };
                if tracked_directories.contains(&path) {
                    paths.insert(path);
                }
                break;
            }
        }

        Ok(paths.into_iter().collect())
    }

    fn tracked_directory_paths(&self) -> Result<BTreeSet<PathBuf>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("ls-files").arg("-z");
        let output = run_git_raw_output(cmd, "git ls-files subtree directories")?;
        if !output.status.success() {
            return Err(git_command_failed_error(
                "git ls-files subtree directories",
                output,
            ));
        }

        let mut directories = BTreeSet::new();
        for raw_path in output.stdout.split(|byte| *byte == b'\0') {
            if raw_path.is_empty() {
                continue;
            }
            let path = PathBuf::from(bytes_to_text_preserving_utf8(raw_path));
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent.as_os_str().is_empty() {
                    break;
                }
                directories.insert(parent.to_path_buf());
                current = parent.parent();
            }
        }

        Ok(directories)
    }

    fn read_stored_subtree_source_configs(&self) -> Result<BTreeMap<PathBuf, SubtreeSourceConfig>> {
        let rows = self.read_stored_subtree_config_rows()?;
        Ok(rows
            .into_values()
            .filter_map(|row| {
                let path = row.path?;
                let repository = row.repository?;
                let reference = row.reference?;
                Some((
                    path,
                    SubtreeSourceConfig {
                        repository,
                        reference,
                        push_refspec: row.push_refspec,
                        squash: row.squash.unwrap_or(true),
                    },
                ))
            })
            .collect())
    }

    fn read_stored_subtree_config_rows(&self) -> Result<BTreeMap<String, StoredSubtreeConfigRow>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("config")
            .arg("--local")
            .arg("--null")
            .arg("--get-regexp")
            .arg(r"^gitcomet\.subtree\.");
        let output = run_git_raw_output(cmd, "git config --local subtree metadata")?;
        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(BTreeMap::new());
            }
            return Err(git_command_failed_error(
                "git config --local subtree metadata",
                output,
            ));
        }

        let mut rows: BTreeMap<String, StoredSubtreeConfigRow> = BTreeMap::new();
        for entry in output.stdout.split(|byte| *byte == b'\0') {
            if entry.is_empty() {
                continue;
            }
            let Some((raw_key, raw_value)) = entry.split_first_newline() else {
                continue;
            };
            let key = bytes_to_text_preserving_utf8(raw_key);
            let value = bytes_to_text_preserving_utf8(raw_value);
            let Some((config_id, field)) = parse_stored_subtree_key(&key) else {
                continue;
            };
            let row = rows.entry(config_id.to_string()).or_default();
            match field {
                SUBTREE_PATH_FIELD => {
                    let value = value.trim();
                    if !value.is_empty() {
                        row.path = Some(PathBuf::from(value));
                    }
                }
                SUBTREE_REPOSITORY_FIELD => {
                    let value = value.trim();
                    if !value.is_empty() {
                        row.repository = Some(value.to_string());
                    }
                }
                SUBTREE_REFERENCE_FIELD => {
                    let value = value.trim();
                    if !value.is_empty() {
                        row.reference = Some(value.to_string());
                    }
                }
                SUBTREE_PUSH_REFSPEC_FIELD => {
                    let value = value.trim();
                    if !value.is_empty() {
                        row.push_refspec = Some(value.to_string());
                    }
                }
                SUBTREE_SQUASH_FIELD => {
                    row.squash = parse_git_bool(&value);
                }
                _ => {}
            }
        }
        Ok(rows)
    }

    fn write_stored_subtree_source_config(
        &self,
        path: &Path,
        source: &SubtreeSourceConfig,
    ) -> Result<()> {
        let config_id = config_id_for_path(path);
        self.set_local_config_value(
            subtree_config_key(&config_id, SUBTREE_PATH_FIELD),
            path_to_config_value(path)?,
        )?;
        self.set_local_config_value(
            subtree_config_key(&config_id, SUBTREE_REPOSITORY_FIELD),
            source.repository.clone(),
        )?;
        self.set_local_config_value(
            subtree_config_key(&config_id, SUBTREE_REFERENCE_FIELD),
            source.reference.clone(),
        )?;
        match &source.push_refspec {
            Some(push_refspec) => self.set_local_config_value(
                subtree_config_key(&config_id, SUBTREE_PUSH_REFSPEC_FIELD),
                push_refspec.clone(),
            )?,
            None => self.unset_local_config_key(&subtree_config_key(
                &config_id,
                SUBTREE_PUSH_REFSPEC_FIELD,
            ))?,
        }
        self.set_local_config_value(
            subtree_config_key(&config_id, SUBTREE_SQUASH_FIELD),
            if source.squash { "true" } else { "false" }.to_string(),
        )?;
        Ok(())
    }

    fn remove_stored_subtree_source_configs_under(&self, prefix: &Path) -> Result<()> {
        let rows = self.read_stored_subtree_config_rows()?;
        for (config_id, row) in rows {
            let Some(path) = row.path else {
                continue;
            };
            if !is_same_or_child_path(prefix, &path) {
                continue;
            }
            self.delete_stored_subtree_config_record(&config_id)?;
        }
        Ok(())
    }

    fn delete_stored_subtree_config_record(&self, config_id: &str) -> Result<()> {
        for field in [
            SUBTREE_PATH_FIELD,
            SUBTREE_REPOSITORY_FIELD,
            SUBTREE_REFERENCE_FIELD,
            SUBTREE_PUSH_REFSPEC_FIELD,
            SUBTREE_SQUASH_FIELD,
        ] {
            self.unset_local_config_key(&subtree_config_key(config_id, field))?;
        }
        Ok(())
    }

    fn set_local_config_value(&self, key: String, value: String) -> Result<()> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("config")
            .arg("--local")
            .arg("--replace-all")
            .arg(&key)
            .arg(value);
        run_git_with_output(cmd, &format!("git config --local --replace-all {key}")).map(|_| ())
    }

    fn unset_local_config_key(&self, key: &str) -> Result<()> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("config").arg("--local").arg("--unset-all").arg(key);
        let output = run_git_raw_output(cmd, &format!("git config --local --unset-all {key}"))?;
        if output.status.success() || output.status.code() == Some(5) {
            return Ok(());
        }
        Err(git_command_failed_error(
            &format!("git config --local --unset-all {key}"),
            output,
        ))
    }
}

fn subtree_config_key(config_id: &str, field: &str) -> String {
    format!("{SUBTREE_CONFIG_KEY_PREFIX}{config_id}.{field}")
}

fn parse_subtree_dir_trailer(line: &str) -> Option<PathBuf> {
    let value = line
        .strip_prefix("git-subtree-dir:")?
        .trim()
        .trim_end_matches('/');
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn parse_stored_subtree_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix(SUBTREE_CONFIG_KEY_PREFIX)?;
    let (config_id, field) = rest.split_once('.')?;
    if config_id.is_empty() || field.is_empty() {
        return None;
    }
    Some((config_id, field))
}

fn parse_git_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn config_id_for_path(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        hex_encode(path.as_os_str().as_bytes())
    }

    #[cfg(windows)]
    {
        hex_encode(path.to_string_lossy().as_bytes())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(HEX[(byte >> 4) as usize]));
        out.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    out
}

fn path_to_config_value(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| Error::new(ErrorKind::Backend("subtree path is not valid UTF-8".into())))
}

fn is_same_or_child_path(prefix: &Path, candidate: &Path) -> bool {
    candidate == prefix || candidate.starts_with(prefix)
}

fn is_missing_head_output(output: &std::process::Output) -> bool {
    let detail = [output.stderr.as_slice(), output.stdout.as_slice()]
        .into_iter()
        .map(bytes_to_text_preserving_utf8)
        .collect::<Vec<_>>()
        .join("\n");
    detail.contains("Not a valid object name HEAD")
        || detail.contains("bad revision 'HEAD'")
        || detail.contains("ambiguous argument 'HEAD'")
}

fn map_subtree_command_error(result: Result<CommandOutput>) -> Result<CommandOutput> {
    result.map_err(|error| match error.kind() {
        ErrorKind::Git(failure)
            if failure
                .detail()
                .is_some_and(|detail| detail.contains("git: 'subtree' is not a git command")) =>
        {
            Error::new(ErrorKind::Backend(
                "git subtree is not available in this Git installation".to_string(),
            ))
        }
        _ => error,
    })
}

trait SplitFirstNewline {
    fn split_first_newline(&self) -> Option<(&[u8], &[u8])>;
}

impl SplitFirstNewline for [u8] {
    fn split_first_newline(&self) -> Option<(&[u8], &[u8])> {
        let ix = self.iter().position(|byte| *byte == b'\n')?;
        Some((&self[..ix], &self[ix + 1..]))
    }
}
