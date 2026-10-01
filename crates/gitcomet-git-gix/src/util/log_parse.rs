//! Parsing `git log` pretty output and remote listings.

use super::*;

#[derive(Default)]
pub(super) struct GitLogPrettyParseState {
    pub(super) repeated_author: Option<Arc<str>>,
    pub(super) next_commit_id_cache: Option<CommitId>,
}

impl GitLogPrettyParseState {
    pub(super) fn push_record(&mut self, record: &str, commits: &mut Vec<Commit>) {
        let record = record.trim();
        if record.is_empty() {
            return;
        }
        let mut parts = record.split('\u{001f}');
        let Some(id) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let parents = parts.next().unwrap_or_default();
        let author = parts.next().unwrap_or_default();
        let time_secs = parts
            .next()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let summary = parts.next().unwrap_or_default();

        let time = unix_seconds_to_system_time_or_epoch(time_secs);

        let id = if let Some(cached) = self.next_commit_id_cache.as_ref()
            && cached.as_ref() == id
        {
            cached.clone()
        } else {
            CommitId(id.into())
        };

        let parent_ids = parents
            .split_whitespace()
            .map(|p| CommitId(p.into()))
            .collect::<CommitParentIds>();

        self.next_commit_id_cache = parent_ids.first().cloned();

        let author = if let Some(cached) = self.repeated_author.as_ref()
            && cached.as_ref() == author
        {
            Arc::clone(cached)
        } else {
            let author: Arc<str> = author.into();
            self.repeated_author = Some(Arc::clone(&author));
            author
        };

        commits.push(Commit {
            id,
            parent_ids,
            summary: summary.into(),
            author,
            time,
        });
    }
}

#[cfg(test)]
pub(crate) fn parse_git_log_pretty_records(output: &str) -> LogPage {
    let approx_commits = output
        .as_bytes()
        .iter()
        .filter(|&&b| b == b'\x1e')
        .count()
        .saturating_add(1);
    let mut commits = Vec::with_capacity(approx_commits);
    let mut state = GitLogPrettyParseState::default();
    for record in output.split('\u{001e}') {
        state.push_record(record, &mut commits);
    }

    LogPage {
        commits,
        next_cursor: None,
    }
}

pub(crate) fn parse_git_log_pretty_records_from_reader(reader: impl io::Read) -> Result<LogPage> {
    let mut reader = io::BufReader::new(reader);
    let mut raw_record = Vec::new();
    let mut commits = Vec::new();
    let mut state = GitLogPrettyParseState::default();

    loop {
        raw_record.clear();
        let bytes_read = reader
            .read_until(b'\x1e', &mut raw_record)
            .map_err(io_err)?;
        if bytes_read == 0 {
            break;
        }
        if raw_record.last() == Some(&b'\x1e') {
            raw_record.pop();
        }
        // Valid UTF-8 (the usual case) is parsed in place; only a record
        // with invalid bytes pays for a repaired copy.
        match std::str::from_utf8(&raw_record) {
            Ok(record) => state.push_record(record, &mut commits),
            Err(_) => state.push_record(&bytes_to_text_preserving_utf8(&raw_record), &mut commits),
        }
    }

    Ok(LogPage {
        commits,
        next_cursor: None,
    })
}

pub(crate) fn unix_seconds_to_system_time(seconds: i64) -> Option<SystemTime> {
    if seconds >= 0 {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64))
    } else {
        None
    }
}

pub(crate) fn unix_seconds_to_system_time_or_epoch(seconds: i64) -> SystemTime {
    unix_seconds_to_system_time(seconds).unwrap_or(SystemTime::UNIX_EPOCH)
}

// Test helper: parses `git branch -r` output for remote branch integration tests.
#[cfg(test)]
pub(crate) fn parse_remote_branches(output: &str) -> Vec<RemoteBranch> {
    let approx_branches = output
        .as_bytes()
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        .saturating_add(1);
    let mut branches = Vec::with_capacity(approx_branches);
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(full_name) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if full_name.ends_with("/HEAD") {
            continue;
        }
        let Some(sha) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some((remote, name)) = full_name.split_once('/') else {
            continue;
        };
        branches.push(RemoteBranch {
            remote: remote.to_string(),
            name: name.to_string(),
            target: CommitId(sha.into()),
        });
    }
    branches.sort_unstable_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
    branches
}
