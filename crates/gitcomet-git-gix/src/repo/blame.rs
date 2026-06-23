use super::{
    GixRepo, bstr_to_arc_str,
    conflict_stages::{gix_index_stage_blob_bytes_optional, gix_index_stage_exists},
    oid_to_arc_str,
};
use crate::util::{
    bytes_to_text_preserving_utf8, path_buf_from_git_bytes, run_git_capture_bytes,
    run_git_with_output, run_git_with_stdin_capture,
};
use gitcomet_core::domain::{DiffArea, is_uncommitted_commit_id};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{BlameLine, CommandOutput, ConflictSide, Result};
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap as HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct BlameCommitMetadata {
    commit_id_text: Arc<str>,
    author: Arc<str>,
    author_time_unix: Option<i64>,
    summary: Arc<str>,
    body: Option<Arc<str>>,
    prior_exists: bool,
}

/// Whether `path` exists in the tree of `commit`'s first parent. Returns
/// `false` for root commits (no parent) or when the path is absent there,
/// which is exactly when navigating to the parent revision is a dead end.
fn file_exists_at_first_parent(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    path: &Path,
) -> bool {
    let Some(parent_id) = commit.parent_ids().next() else {
        return false;
    };
    let Ok(parent) = repo.find_object(parent_id) else {
        return false;
    };
    let Ok(tree) = parent.peel_to_tree() else {
        return false;
    };
    matches!(tree.lookup_entry_by_path(path), Ok(Some(_)))
}

/// Whether `path` exists in the tree of the current `HEAD`. Returns `false`
/// when there is no `HEAD` (unborn branch) or the path is absent there — exactly
/// when `git blame` would fail with "no such path ... in HEAD" because the file
/// has no committed history and all of its lines are local changes.
fn path_exists_at_head(repo: &gix::Repository, path: &Path) -> bool {
    let Ok(id) = repo.head_id() else {
        return false;
    };
    let Ok(object) = repo.find_object(id) else {
        return false;
    };
    let Ok(tree) = object.peel_to_tree() else {
        return false;
    };
    matches!(tree.lookup_entry_by_path(path), Ok(Some(_)))
}

/// Read the staged (index) content for `path` to blame. This is normally the
/// stage-0 blob, but a merge-conflicted file has no stage-0 entry — only the
/// base/ours/theirs stages (1/2/3). In that case fall back to "ours" (stage 2),
/// then "theirs" (stage 3), so toggling blame on the staged side of a conflicted
/// file still produces a blame instead of erroring with "no staged content".
fn staged_blob_for_blame(repo: &gix::Repository, path: &Path) -> Result<Vec<u8>> {
    for stage in [0u8, 2, 3] {
        if let Some(bytes) = gix_index_stage_blob_bytes_optional(repo, path, stage)? {
            return Ok(bytes);
        }
    }
    Err(Error::new(ErrorKind::Backend(format!(
        "no staged content for {}",
        path.display()
    ))))
}

/// Build an all-"Not Committed Yet" blame for `contents`, used for a file with no
/// committed history (newly added / untracked) where every line is local. The
/// all-zero object id matches what `git blame --line-porcelain` emits for
/// uncommitted lines, so the UI treats these rows the same way.
fn synthesize_uncommitted_blame(contents: &[u8]) -> Vec<BlameLine> {
    const NOT_COMMITTED: &str = "Not Committed Yet";
    const UNCOMMITTED_ID: &str = "0000000000000000000000000000000000000000";

    let commit_id: Arc<str> = Arc::from(UNCOMMITTED_ID);
    let author: Arc<str> = Arc::from(NOT_COMMITTED);
    blame_blob_lines(contents)
        .map(|line| BlameLine {
            commit_id: commit_id.clone(),
            author: author.clone(),
            author_time_unix: None,
            summary: author.clone(),
            body: None,
            line: blame_line_text(line),
            prior_exists: false,
            source_path: None,
            // The file has no committed base, so there is no parent to open.
            prior_commit: None,
        })
        .collect()
}

/// Detect rename tracking configuration (`diff.renames` / `diff.renameLimit`),
/// falling back to git's defaults (rename detection on at 50% similarity, copy
/// detection off) when unconfigured. Returns `None` to disable rename tracking
/// when `diff.renames` is explicitly set to a false value.
fn configured_rewrites(repo: &gix::Repository) -> Option<gix::diff::Rewrites> {
    use gix::diff::rewrites::Copies;

    let default = gix::diff::Rewrites::default();
    let snapshot = repo.config_snapshot();
    // `diff.renames` is a boolean, except for the special `copies`/`copy` values
    // that also enable copy detection. Read it as a raw string so both forms are
    // handled the way git does.
    let copies = match snapshot.string("diff.renames") {
        None => None,
        Some(value) => match value.to_ascii_lowercase().as_slice() {
            b"copy" | b"copies" => Some(Copies::default()),
            b"false" | b"no" | b"off" | b"0" => return None,
            _ => None,
        },
    };
    // gix skips rename detection entirely once the number of add/delete
    // permutations exceeds `limit`, so a single large rename commit (e.g. a
    // repo-wide move touching thousands of files) would silently break the
    // rename chain. Default to unlimited (0) so following stays reliable;
    // honor an explicit `diff.renameLimit` when the user set one.
    let limit = snapshot
        .integer("diff.renameLimit")
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);

    Some(gix::diff::Rewrites {
        copies,
        limit,
        ..default
    })
}

fn blame_commit_metadata<'a>(
    repo: &gix::Repository,
    cache: &'a mut HashMap<(gix::ObjectId, Option<PathBuf>), BlameCommitMetadata>,
    commit_id: gix::ObjectId,
    source_path: Option<&Path>,
    blamed_path: &Path,
) -> Result<&'a BlameCommitMetadata> {
    match cache.entry((commit_id, source_path.map(Path::to_path_buf))) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => {
            let commit = repo.find_commit(commit_id).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix find_commit {commit_id}: {e}"
                )))
            })?;

            // For a renamed file we must check whether the *historical* path (the
            // file's name at this commit) existed in the first parent, not the
            // current name which may not exist in that older tree.
            let prior_exists =
                file_exists_at_first_parent(repo, &commit, source_path.unwrap_or(blamed_path));

            let (author, author_time_unix) = match commit.author() {
                Ok(signature) => (
                    bstr_to_arc_str(signature.name.as_ref()),
                    signature.time().ok().map(|time| time.seconds),
                ),
                Err(_) => (Arc::<str>::default(), None),
            };
            // Split subject from body the way git itself does, via gix's message
            // parser: the subject is the whole first paragraph (folded into one
            // line, so a multi-line subject keeps every line) and the body is
            // everything after the blank-line separator. Hand-rolling this with a
            // `\n\n` scan dropped middle subject lines and missed CRLF separators.
            let message = commit.message().ok();
            let summary = message
                .as_ref()
                .map(|m| {
                    let folded = m.summary();
                    let bytes: &[u8] = &folded;
                    bstr_to_arc_str(bytes)
                })
                .unwrap_or_default();
            let body = message.as_ref().and_then(|m| m.body).and_then(|b| {
                let bytes: &[u8] = b;
                let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
                let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
                (!bytes.is_empty()).then(|| bstr_to_arc_str(bytes))
            });

            Ok(entry.insert(BlameCommitMetadata {
                commit_id_text: oid_to_arc_str(&commit_id),
                author,
                author_time_unix,
                summary,
                body,
                prior_exists,
            }))
        }
    }
}

fn blame_line_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes_to_text_preserving_utf8(bytes),
    }
}

struct BlameBlobLines<'a> {
    blob: &'a [u8],
    cursor: usize,
}

impl<'a> Iterator for BlameBlobLines<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.blob.len() {
            return None;
        }

        let start = self.cursor;
        let remaining = &self.blob[start..];
        if let Some(offset) = remaining.iter().position(|byte| *byte == b'\n') {
            let end = start + offset + 1;
            self.cursor = end;
            Some(&self.blob[start..end])
        } else {
            self.cursor = self.blob.len();
            Some(&self.blob[start..])
        }
    }
}

fn blame_blob_lines(blob: &[u8]) -> BlameBlobLines<'_> {
    BlameBlobLines { blob, cursor: 0 }
}

fn is_hex_object_id(token: &str) -> bool {
    // SHA-1 ids are 40 hex chars, SHA-256 ids are 64. Accept both so blame
    // parsing works on repositories using either object format.
    matches!(token.len(), 40 | 64) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parse `git blame --line-porcelain` output into per-line blame entries.
///
/// With `--line-porcelain` the full commit header is repeated before every
/// content line, so each `\t`-prefixed content line is emitted using the header
/// fields accumulated since the preceding entry. The all-zero object id marks a
/// line that is not yet committed; those are surfaced as "Not Committed Yet".
fn parse_blame_porcelain(output: &[u8], blamed_path: &Path) -> Vec<BlameLine> {
    const NOT_COMMITTED: &str = "Not Committed Yet";

    let mut lines: Vec<BlameLine> = Vec::new();
    let mut commit_id: Option<String> = None;
    let mut author: Option<String> = None;
    let mut author_time_unix: Option<i64> = None;
    let mut summary: Option<String> = None;
    let mut prior_exists = false;
    // For uncommitted lines, git emits `previous <sha> <path>` pointing at the
    // base revision the working-tree change was made against. Captured so the UI
    // can offer "view file at parent commit" on a not-yet-committed line.
    let mut prior_commit: Option<String> = None;
    // The file's name at the current commit (porcelain `filename` line). Kept as
    // raw bytes so non-UTF-8 Unix paths survive; surfaced as `source_path` when
    // it differs from the blamed path (i.e. the line came from before a rename).
    let mut filename: Option<Vec<u8>> = None;

    for raw_line in output.split(|b| *b == b'\n') {
        if raw_line.first() == Some(&b'\t') {
            let text = blame_line_text(&raw_line[1..]);
            let sha = commit_id.clone().unwrap_or_default();
            let uncommitted = is_uncommitted_commit_id(&sha);
            let (author_text, summary_text, time, prior) = if uncommitted {
                (
                    NOT_COMMITTED.to_string(),
                    NOT_COMMITTED.to_string(),
                    None,
                    false,
                )
            } else {
                (
                    author.clone().unwrap_or_default(),
                    summary.clone().unwrap_or_default(),
                    author_time_unix,
                    prior_exists,
                )
            };
            // The base revision to open for "view file at parent commit" applies
            // only to uncommitted lines; committed lines resolve their parent
            // from `commit_id` instead.
            let prior_commit_id = if uncommitted {
                prior_commit.as_deref().map(Arc::from)
            } else {
                None
            };
            // A `filename` differing from the blamed path means this line predates
            // a rename; surface it as the historical path so navigation uses a name
            // that exists at the line's commit. Uncommitted lines have no history.
            let source_path = if uncommitted {
                None
            } else {
                filename
                    .as_deref()
                    .and_then(|bytes| path_buf_from_git_bytes(bytes, "git blame filename").ok())
                    .filter(|p| p.as_path() != blamed_path)
            };
            lines.push(BlameLine {
                commit_id: Arc::from(sha.as_str()),
                author: Arc::from(author_text.as_str()),
                author_time_unix: time,
                summary: Arc::from(summary_text.as_str()),
                body: None,
                line: text,
                prior_exists: prior,
                source_path,
                prior_commit: prior_commit_id,
            });

            commit_id = None;
            author = None;
            author_time_unix = None;
            summary = None;
            prior_exists = false;
            prior_commit = None;
            filename = None;
            continue;
        }

        // Capture the historical filename from raw bytes before the UTF-8 decode
        // below (which would otherwise drop a non-UTF-8 path). The
        // `previous <sha> <path>` line is left to the `"previous"` arm, which only
        // flags prior existence.
        if let Some(rest) = raw_line.strip_prefix(b"filename ") {
            filename = Some(rest.to_vec());
            continue;
        }

        let Ok(line_str) = std::str::from_utf8(raw_line) else {
            continue;
        };
        let (key, rest) = match line_str.split_once(' ') {
            Some((key, rest)) => (key, rest),
            None => (line_str, ""),
        };
        match key {
            "author" => author = Some(rest.to_string()),
            "author-time" => author_time_unix = rest.trim().parse::<i64>().ok(),
            "summary" => summary = Some(rest.to_string()),
            "previous" => {
                prior_exists = true;
                // `previous <sha> <path>`: keep the sha so an uncommitted line
                // can navigate to the revision it was edited from.
                prior_commit = rest.split_whitespace().next().map(str::to_string);
            }
            _ if is_hex_object_id(key) => commit_id = Some(key.to_string()),
            _ => {}
        }
    }

    lines
}

impl GixRepo {
    pub(super) fn blame_file_impl(&self, path: &Path, rev: Option<&str>) -> Result<Vec<BlameLine>> {
        const BLOB_LINE_MISMATCH: &str = "gix blame blob line count did not match blame entries";

        let repo = self._repo.to_thread_local();
        let spec = rev.unwrap_or("HEAD");
        let suspect = repo
            .rev_parse_single(spec)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev-parse {spec}: {e}"))))?
            .detach();
        let git_path = gix::path::os_str_into_bstr(path.as_os_str())
            .map(gix::path::to_unix_separators_on_windows)
            .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))?;
        let options = gix::repository::blame_file::Options {
            rewrites: configured_rewrites(&repo),
            ..Default::default()
        };
        let outcome = match repo.blame_file(git_path.as_ref(), suspect, options) {
            Ok(outcome) => outcome,
            Err(e) => {
                let msg = format!("gix blame {}: {e}", path.display());
                return Err(Error::new(ErrorKind::Backend(msg)));
            }
        };

        let mut metadata_cache = HashMap::default();
        let total_lines = outcome
            .entries
            .last()
            .map(|entry| entry.start_in_blamed_file as usize + entry.len.get() as usize)
            .unwrap_or_default();
        let mut lines = Vec::with_capacity(total_lines);
        let mut blob_lines = blame_blob_lines(&outcome.blob);
        let mut blob_line_ix = 0usize;
        for entry in &outcome.entries {
            let entry_start = entry.start_in_blamed_file as usize;
            let entry_len = entry.len.get() as usize;
            while blob_line_ix < entry_start {
                if blob_lines.next().is_none() {
                    return Err(Error::new(ErrorKind::Backend(
                        BLOB_LINE_MISMATCH.to_string(),
                    )));
                }
                blob_line_ix += 1;
            }
            // When rename tracking attributes a hunk to a commit where the file
            // had a different name, `source_file_name` is that historical path.
            let source_path = entry
                .source_file_name
                .as_ref()
                .map(|name| gix::path::from_bstr(name.as_bstr()).into_owned())
                .filter(|source| source.as_path() != path);
            let metadata = blame_commit_metadata(
                &repo,
                &mut metadata_cache,
                entry.commit_id,
                source_path.as_deref(),
                path,
            )?;
            for _ in 0..entry_len {
                let Some(line) = blob_lines.next() else {
                    return Err(Error::new(ErrorKind::Backend(
                        BLOB_LINE_MISMATCH.to_string(),
                    )));
                };
                blob_line_ix += 1;
                lines.push(BlameLine {
                    commit_id: metadata.commit_id_text.clone(),
                    author: metadata.author.clone(),
                    author_time_unix: metadata.author_time_unix,
                    summary: metadata.summary.clone(),
                    body: metadata.body.clone(),
                    line: blame_line_text(line),
                    prior_exists: metadata.prior_exists,
                    source_path: source_path.clone(),
                    // Committed lines resolve their parent from `commit_id`.
                    prior_commit: None,
                });
            }
        }
        Ok(lines)
    }

    /// Blame the working-tree content shown on the new side of a staged/unstaged
    /// diff via `git blame --line-porcelain`. Unstaged blames the worktree file
    /// directly; staged feeds the index blob to `--contents -` so attribution
    /// matches the index content shown in the diff. Lines that do not (yet) exist
    /// in committed history come back as "Not Committed Yet" entries.
    pub(super) fn blame_worktree_file_impl(
        &self,
        path: &Path,
        area: DiffArea,
    ) -> Result<Vec<BlameLine>> {
        // A file with no committed history (newly added / untracked) is absent
        // from HEAD, so `git blame` fails with "no such path ... in HEAD". Every
        // line is local, so synthesize the blame directly from the shown content
        // (working tree for unstaged, the staged blob for staged).
        let repo = self._repo.to_thread_local();
        if !path_exists_at_head(&repo, path) {
            let contents = match area {
                DiffArea::Unstaged => {
                    let abs_path = self.spec.workdir.join(path);
                    fs::read(&abs_path).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?
                }
                DiffArea::Staged => staged_blob_for_blame(&repo, path)?,
            };
            return Ok(synthesize_uncommitted_blame(&contents));
        }

        let mut cmd = self.git_workdir_cmd();
        // `core.quotePath=false` keeps the porcelain `filename` value raw so the
        // historical path parsed into `source_path` is not C-quoted.
        cmd.arg("-c")
            .arg("core.quotePath=false")
            .arg("blame")
            .arg("--line-porcelain");

        let output = match area {
            DiffArea::Unstaged => {
                cmd.arg("--").arg(path);
                run_git_capture_bytes(cmd, "git blame --line-porcelain")?
            }
            DiffArea::Staged => {
                let contents = staged_blob_for_blame(&repo, path)?;
                cmd.arg("--contents").arg("-").arg("--").arg(path);
                run_git_with_stdin_capture(cmd, contents, "git blame --contents")?
            }
        };

        Ok(parse_blame_porcelain(&output, path))
    }

    pub(super) fn checkout_conflict_side_impl(
        &self,
        path: &Path,
        side: ConflictSide,
    ) -> Result<CommandOutput> {
        let desired_stage = match side {
            ConflictSide::Ours => 2,
            ConflictSide::Theirs => 3,
        };

        let repo = self._repo.to_thread_local();

        if !gix_index_stage_exists(&repo, path, desired_stage)? {
            let mut rm = self.git_workdir_cmd();
            rm.arg("rm").arg("--").arg(path);
            return run_git_with_output(rm, "git rm --");
        }

        let mut checkout = self.git_workdir_cmd();
        checkout.arg("checkout");
        match side {
            ConflictSide::Ours => {
                checkout.arg("--ours");
            }
            ConflictSide::Theirs => {
                checkout.arg("--theirs");
            }
        }
        checkout.arg("--").arg(path);
        let checkout_out = run_git_with_output(checkout, "git checkout --ours/--theirs")?;

        let mut add = self.git_workdir_cmd();
        add.arg("add").arg("--").arg(path);
        let add_out = run_git_with_output(add, "git add --")?;

        Ok(CommandOutput {
            command: checkout_out.command,
            stdout: [checkout_out.stdout, add_out.stdout]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            stderr: [checkout_out.stderr, add_out.stderr]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            exit_code: add_out.exit_code.or(checkout_out.exit_code),
        })
    }

    pub(super) fn accept_conflict_deletion_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut rm = self.git_workdir_cmd();
        rm.arg("rm").arg("--").arg(path);
        run_git_with_output(rm, "git rm --")
    }

    pub(super) fn checkout_conflict_base_impl(&self, path: &Path) -> Result<CommandOutput> {
        let repo = self._repo.to_thread_local();
        let base_bytes = gix_index_stage_blob_bytes_optional(&repo, path, 1)?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "base conflict stage is not available for {}",
                path.display()
            )))
        })?;
        let abs_path = self.spec.workdir.join(path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        }
        fs::write(&abs_path, base_bytes).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        let mut add = self.git_workdir_cmd();
        add.arg("add").arg("--").arg(path);
        let add_out = run_git_with_output(add, "git add --")?;

        Ok(CommandOutput {
            command: format!("git show :1:{} + git add --", path.display()),
            stdout: add_out.stdout,
            stderr: add_out.stderr,
            exit_code: add_out.exit_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_line_text_trims_crlf_and_lf() {
        assert_eq!(blame_line_text(b"hello\n"), "hello");
        assert_eq!(blame_line_text(b"hello\r\n"), "hello");
        assert_eq!(blame_line_text(b"hello"), "hello");
    }

    #[test]
    fn blame_blob_lines_preserves_terminators_and_final_line() {
        let blob = b"first\nsecond\r\nthird";
        let lines = blame_blob_lines(blob).collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![&b"first\n"[..], &b"second\r\n"[..], &b"third"[..]]
        );
    }

    #[test]
    fn blame_blob_lines_is_empty_for_empty_blob() {
        assert_eq!(blame_blob_lines(b"").count(), 0);
    }

    #[test]
    fn parse_blame_porcelain_handles_committed_and_uncommitted_lines() {
        // `git blame --line-porcelain` repeats the full commit header before
        // each content line. The all-zero object id marks an uncommitted line.
        let output = concat!(
            "1111111111111111111111111111111111111111 1 1 1\n",
            "author Ada Lovelace\n",
            "author-mail <ada@example.com>\n",
            "author-time 1700000000\n",
            "author-tz +0000\n",
            "summary initial commit\n",
            "previous 0000000000000000000000000000000000000001 src/lib.rs\n",
            "filename src/lib.rs\n",
            "\tcommitted line\n",
            "0000000000000000000000000000000000000000 2 2 1\n",
            "author Not Committed Yet\n",
            "author-mail <not.committed.yet>\n",
            "author-time 1700000100\n",
            "author-tz +0000\n",
            "summary Version of src/lib.rs from src/lib.rs\n",
            "previous 1111111111111111111111111111111111111111 src/lib.rs\n",
            "filename src/lib.rs\n",
            "\tuncommitted line\n",
        );

        let lines = parse_blame_porcelain(output.as_bytes(), std::path::Path::new("src/lib.rs"));
        assert_eq!(lines.len(), 2);

        let committed = &lines[0];
        assert_eq!(
            committed.commit_id.as_ref(),
            "1111111111111111111111111111111111111111"
        );
        assert_eq!(committed.author.as_ref(), "Ada Lovelace");
        assert_eq!(committed.author_time_unix, Some(1700000000));
        assert_eq!(committed.summary.as_ref(), "initial commit");
        assert_eq!(committed.line, "committed line");
        assert!(committed.prior_exists);
        // `filename` equals the blamed path, so there is no distinct historical path.
        assert_eq!(committed.source_path, None);
        // Committed lines navigate to their parent via `commit_id`, not `prior_commit`.
        assert_eq!(committed.prior_commit, None);

        let uncommitted = &lines[1];
        assert_eq!(
            uncommitted.commit_id.as_ref(),
            "0000000000000000000000000000000000000000"
        );
        assert_eq!(uncommitted.author.as_ref(), "Not Committed Yet");
        assert_eq!(uncommitted.summary.as_ref(), "Not Committed Yet");
        assert_eq!(uncommitted.author_time_unix, None);
        assert_eq!(uncommitted.line, "uncommitted line");
        assert!(!uncommitted.prior_exists);
        assert_eq!(uncommitted.source_path, None);
        // The porcelain `previous` sha is the base revision to open for
        // "view file at parent commit" on the uncommitted line.
        assert_eq!(
            uncommitted.prior_commit.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
    }

    #[test]
    fn is_hex_object_id_accepts_sha1_and_sha256_widths() {
        assert!(is_hex_object_id(&"a".repeat(40))); // SHA-1
        assert!(is_hex_object_id(&"a".repeat(64))); // SHA-256
        // Other lengths and non-hex tokens are not object ids.
        assert!(!is_hex_object_id(&"a".repeat(39)));
        assert!(!is_hex_object_id(&"a".repeat(41)));
        assert!(!is_hex_object_id(&"a".repeat(63)));
        assert!(!is_hex_object_id("author"));
        assert!(!is_hex_object_id(&"g".repeat(40)));
    }

    #[test]
    fn parse_blame_porcelain_recognizes_sha256_commit_ids() {
        // On a SHA-256 repository the per-line commit-id header is 64 hex chars;
        // it must be recognized so the committed line keeps its attribution
        // instead of falling through with an empty commit id.
        let sha256 = "a".repeat(64);
        let output = format!(
            concat!(
                "{sha} 1 1 1\n",
                "author Ada Lovelace\n",
                "author-time 1700000000\n",
                "summary initial commit\n",
                "filename src/lib.rs\n",
                "\tcommitted line\n",
            ),
            sha = sha256
        );

        let lines = parse_blame_porcelain(output.as_bytes(), std::path::Path::new("src/lib.rs"));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].commit_id.as_ref(), sha256);
        assert_eq!(lines[0].author.as_ref(), "Ada Lovelace");
        assert_eq!(lines[0].summary.as_ref(), "initial commit");
        assert_eq!(lines[0].author_time_unix, Some(1700000000));
    }

    #[test]
    fn synthesize_uncommitted_blame_marks_every_line_local() {
        // A newly added file with no committed history: every line is surfaced as
        // an uncommitted ("Not Committed Yet") entry with the all-zero object id
        // and no parent revision to navigate to.
        let lines = synthesize_uncommitted_blame(b"first\nsecond\nthird");
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
        for line in &lines {
            assert_eq!(
                line.commit_id.as_ref(),
                "0000000000000000000000000000000000000000"
            );
            assert_eq!(line.author.as_ref(), "Not Committed Yet");
            assert_eq!(line.summary.as_ref(), "Not Committed Yet");
            assert_eq!(line.author_time_unix, None);
            assert!(!line.prior_exists);
            assert_eq!(line.prior_commit, None);
            assert_eq!(line.source_path, None);
        }
    }

    #[test]
    fn synthesize_uncommitted_blame_is_empty_for_empty_file() {
        assert!(synthesize_uncommitted_blame(b"").is_empty());
    }

    #[test]
    fn parse_blame_porcelain_surfaces_historical_path_on_rename() {
        // A committed line whose `filename` differs from the blamed path predates a
        // rename and must surface that historical name; a same-name line must not;
        // an uncommitted line never carries a historical path even if `filename`
        // differs.
        let output = concat!(
            "1111111111111111111111111111111111111111 1 1 1\n",
            "author Ada Lovelace\n",
            "author-time 1700000000\n",
            "summary rename era\n",
            "filename old/dir/lib.rs\n",
            "\trenamed line\n",
            "2222222222222222222222222222222222222222 2 2 1\n",
            "author Ada Lovelace\n",
            "author-time 1700000100\n",
            "summary current era\n",
            "filename new/dir/lib.rs\n",
            "\tcurrent line\n",
            "0000000000000000000000000000000000000000 3 3 1\n",
            "author Not Committed Yet\n",
            "author-time 1700000200\n",
            "summary working copy\n",
            "filename old/dir/lib.rs\n",
            "\tuncommitted line\n",
        );

        let lines =
            parse_blame_porcelain(output.as_bytes(), std::path::Path::new("new/dir/lib.rs"));
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0].source_path.as_deref(),
            Some(std::path::Path::new("old/dir/lib.rs"))
        );
        assert_eq!(lines[1].source_path, None);
        assert_eq!(lines[2].source_path, None);
    }

    #[cfg(unix)]
    #[test]
    fn parse_blame_porcelain_preserves_non_utf8_historical_path() {
        use std::os::unix::ffi::OsStrExt as _;

        // A non-UTF-8 historical filename must survive into `source_path` (the raw
        // bytes are captured before the porcelain header's UTF-8 decode).
        let mut output: Vec<u8> = Vec::new();
        output.extend_from_slice(b"1111111111111111111111111111111111111111 1 1 1\n");
        output.extend_from_slice(b"author Ada Lovelace\n");
        output.extend_from_slice(b"author-time 1700000000\n");
        output.extend_from_slice(b"summary non utf8\n");
        output.extend_from_slice(b"filename docs/\xff-old.md\n");
        output.extend_from_slice(b"\tline\n");

        let lines = parse_blame_porcelain(&output, std::path::Path::new("docs/new.md"));
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0]
                .source_path
                .as_deref()
                .map(|p| p.as_os_str().as_bytes()),
            Some(&b"docs/\xff-old.md"[..])
        );
    }
}
