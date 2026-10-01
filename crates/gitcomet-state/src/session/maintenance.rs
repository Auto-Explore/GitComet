use super::*;

/// The daily "does git recommend maintenance" check of one repository, keyed
/// by its shared git directory so worktrees of a repository share it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct RepoMaintenanceSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) last_checked_unix_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) snoozed_until_unix_seconds: Option<u64>,
}

/// A repository is checked at most this often.
pub const MAINTENANCE_CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
/// How long "Remind me later" waits.
pub const MAINTENANCE_SNOOZE_SECONDS: u64 = 24 * 60 * 60;
/// Entries kept; the longest unchecked repositories go first.
const MAX_REPO_MAINTENANCE_ENTRIES: usize = 256;

/// Drops entries that fail to parse instead of failing the whole file.
pub(super) fn lenient_repo_maintenance<'de, D>(
    deserializer: D,
) -> Result<Option<BTreeMap<String, RepoMaintenanceSession>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let serde_json::Value::Object(entries) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(None);
    };
    Ok(Some(
        entries
            .into_iter()
            .filter_map(|(key, entry)| Some((key, serde_json::from_value(entry).ok()?)))
            .collect(),
    ))
}

/// Claims today's check for the repository at `common_dir`: `true` when a day
/// has passed since the last one and no snooze runs, recording the check.
/// Without a session file (tests) every check is due.
pub fn claim_repo_maintenance_check(common_dir: &Path) -> io::Result<bool> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(true);
    };
    claim_repo_maintenance_check_to_path(&session_file_path, common_dir, current_unix_seconds())
}

pub fn claim_repo_maintenance_check_to_path(
    session_file_path: &Path,
    common_dir: &Path,
    now_unix_seconds: u64,
) -> io::Result<bool> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let entries = file.repo_maintenance.get_or_insert_with(BTreeMap::new);
        let entry = entries.entry(path_storage_key(common_dir)).or_default();
        let due = entry.last_checked_unix_seconds.is_none_or(|last| {
            now_unix_seconds.saturating_sub(last) >= MAINTENANCE_CHECK_INTERVAL_SECONDS
        }) && entry
            .snoozed_until_unix_seconds
            .is_none_or(|until| until <= now_unix_seconds);
        if !due {
            return Ok(false);
        }
        entry.last_checked_unix_seconds = Some(now_unix_seconds);
        prune_repo_maintenance(entries);
        file.version = CURRENT_SESSION_FILE_VERSION;
        persist_to_path(session_file_path, &file)?;
        Ok(true)
    })
}

/// "Remind me later": no recommendation for a day.
pub fn persist_repo_maintenance_snooze(common_dir: &Path) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_repo_maintenance_snooze_to_path(&session_file_path, common_dir, current_unix_seconds())
}

pub fn persist_repo_maintenance_snooze_to_path(
    session_file_path: &Path,
    common_dir: &Path,
    now_unix_seconds: u64,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        let entries = file.repo_maintenance.get_or_insert_with(BTreeMap::new);
        entries
            .entry(path_storage_key(common_dir))
            .or_default()
            .snoozed_until_unix_seconds =
            Some(now_unix_seconds.saturating_add(MAINTENANCE_SNOOZE_SECONDS));
        prune_repo_maintenance(entries);
        file.version = CURRENT_SESSION_FILE_VERSION;
        persist_to_path(session_file_path, &file)
    })
}

fn prune_repo_maintenance(entries: &mut BTreeMap<String, RepoMaintenanceSession>) {
    while entries.len() > MAX_REPO_MAINTENANCE_ENTRIES {
        let Some(oldest) = entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_checked_unix_seconds)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        entries.remove(&oldest);
    }
}
