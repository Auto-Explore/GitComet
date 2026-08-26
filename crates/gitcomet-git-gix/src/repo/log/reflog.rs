use super::*;

pub(crate) const RECENT_COMMIT_MESSAGES_MAX_LIMIT: usize = 100;

/// Upper bound on how much of a caller-supplied reflog limit we pre-reserve.
///
/// The limit itself is still enforced while iterating; capping the reservation
/// only keeps a huge limit (`usize::MAX` reads as "all entries") from asking for
/// that much capacity before we know how long the reflog actually is.
pub(crate) const REFLOG_RESERVE_MAX: usize = 512;

pub(crate) fn recent_commit_message_limits(limit: usize) -> Option<(usize, usize)> {
    let limit = limit.min(RECENT_COMMIT_MESSAGES_MAX_LIMIT);
    if limit == 0 {
        return None;
    }

    let scan_limit = limit
        .saturating_mul(5)
        .min(RECENT_COMMIT_MESSAGES_MAX_LIMIT)
        .max(limit);
    Some((limit, scan_limit))
}

pub(crate) struct CursorGate<'a> {
    last_seen: Option<&'a str>,
    started: bool,
}

impl<'a> CursorGate<'a> {
    pub(crate) fn new(cursor: Option<&'a LogCursor>) -> Self {
        Self {
            last_seen: cursor.map(|cursor| cursor.last_seen.as_ref()),
            started: cursor.is_none(),
        }
    }

    pub(crate) fn should_skip(&mut self, id: &str) -> bool {
        self.should_skip_hex(id)
    }

    pub(crate) fn should_skip_oid(&mut self, id: &gix::oid) -> bool {
        if self.started {
            return false;
        }

        let mut buf = gix::hash::Kind::hex_buf();
        self.should_skip_hex(id.hex_to_buf(&mut buf))
    }

    pub(crate) fn should_skip_hex(&mut self, id: &str) -> bool {
        if self.started {
            return false;
        }

        let Some(last_seen) = self.last_seen else {
            self.started = true;
            return false;
        };

        if last_seen == id {
            self.started = true;
        }

        true
    }
}

pub(crate) fn reflog_lines_rev(
    platform: &mut gix::refs::file::log::iter::Platform<'_, '_>,
    context: &str,
    limit: Option<usize>,
) -> Result<Vec<gix::refs::log::Line>> {
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    let Some(iter) = platform
        .rev()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix reflog {context}: {e}"))))?
    else {
        return Ok(Vec::new());
    };

    let mut lines = Vec::with_capacity(limit.unwrap_or(0).min(REFLOG_RESERVE_MAX));
    for line in iter {
        let line =
            line.map_err(|e| Error::new(ErrorKind::Backend(format!("gix reflog {context}: {e}"))))?;
        lines.push(line);
        if let Some(limit) = limit
            && lines.len() >= limit
        {
            break;
        }
    }
    Ok(lines)
}

pub(crate) fn stash_reflog_lines(
    repo: &gix::Repository,
    limit: Option<usize>,
) -> Result<Vec<gix::refs::log::Line>> {
    let Some(reference) = repo.try_find_reference("refs/stash").map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix try_find_reference refs/stash: {e}"
        )))
    })?
    else {
        return Ok(Vec::new());
    };

    let mut platform = reference.log_iter();
    reflog_lines_rev(&mut platform, "refs/stash", limit)
}

pub(crate) fn stash_reflog_entries(repo: &gix::Repository) -> Result<Vec<StashEntry>> {
    stash_reflog_lines(repo, None)?
        .into_iter()
        .enumerate()
        .filter(|(_, line)| !line.new_oid.is_null())
        .map(|(index, line)| {
            let created_at = unix_seconds_to_system_time(line.signature.time.seconds);
            Ok(StashEntry {
                index,
                id: CommitId(oid_to_arc_str(&line.new_oid)),
                message: bstr_to_arc_str(line.message.as_ref()),
                created_at,
            })
        })
        .collect()
}

pub(crate) fn stash_reflog_tips(
    repo: &gix::Repository,
    limit: usize,
) -> Result<Vec<gix::ObjectId>> {
    let reserve = limit.min(REFLOG_RESERVE_MAX);
    let mut tips = Vec::with_capacity(reserve);
    let mut seen = FxHashSet::with_capacity_and_hasher(reserve, Default::default());
    for line in stash_reflog_lines(repo, Some(limit))? {
        let id = line.new_oid;
        if !id.is_null() && seen.insert(id) {
            tips.push(id);
        }
    }
    Ok(tips)
}
