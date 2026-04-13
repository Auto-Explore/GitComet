use super::{BranchTrackingConfigCacheEntry, GixRepo, RepoFileStamp, oid_to_arc_str};
use crate::util::{bytes_to_text_preserving_utf8, run_git_capture, run_git_raw_output};
use gitcomet_core::domain::{Branch, CommitId, Upstream, UpstreamDivergence};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap as HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Output;

const LOCAL_BRANCH_PREFIX: &[u8] = b"refs/heads/";

pub(super) fn head_upstream_divergence(
    repo: &gix::Repository,
) -> Result<Option<UpstreamDivergence>> {
    let head = repo
        .head()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head: {e}"))))?;
    let Some(mut branch_ref) = head.try_into_referent() else {
        return Ok(None);
    };

    let local_tip = match branch_ref.peel_to_id() {
        Ok(id) => id.detach(),
        Err(_) => return Ok(None),
    };

    let (_upstream, divergence) = branch_upstream_and_divergence(repo, &branch_ref, local_tip)?;
    Ok(divergence)
}

impl GixRepo {
    pub(super) fn current_branch_impl(&self) -> Result<String> {
        self.current_branch_gix().or_else(|gix_err| {
            self.current_branch_cli().map_err(|cli_err| {
                Error::new(ErrorKind::Backend(format!(
                    "current branch: gix path failed ({gix_err}); cli fallback failed ({cli_err})"
                )))
            })
        })
    }

    fn branch_tracking_config_present(&self) -> Result<bool> {
        let repo = self._repo.to_thread_local();
        let local_config = repo_file_stamp(repo.common_dir().join("config").as_path());
        let worktree_config = repo_file_stamp(repo.git_dir().join("config.worktree").as_path());

        {
            let cache = self
                .branch_tracking_config
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = cache.as_ref().filter(|cached| {
                cached.local_config == local_config && cached.worktree_config == worktree_config
            }) {
                return Ok(cached.has_branch_sections);
            }
        }

        let repo = self.reopen_repo()?;
        let has_branch_sections = repo_has_branch_tracking_config(&repo);

        *self
            .branch_tracking_config
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(BranchTrackingConfigCacheEntry {
                local_config,
                worktree_config,
                has_branch_sections,
            });
        Ok(has_branch_sections)
    }

    pub(super) fn list_branches_impl(&self) -> Result<Vec<Branch>> {
        let has_branch_tracking = self.branch_tracking_config_present()?;
        if has_branch_tracking {
            // Upstream tracking is config-driven (`branch.*`) and can change while the backend
            // stays open, e.g. after `push -u`. Re-open only while those sections exist so branch
            // listing reflects config edits without paying the reopen cost for ref-only repos.
            return match self.list_branches_cli_with_tracking() {
                Ok(branches) => Ok(branches),
                Err(cli_err) => {
                    let repo = self.reopen_repo()?;
                    collect_local_branches(&repo, true).map_err(|gix_err| {
                        Error::new(ErrorKind::Backend(format!(
                            "list branches: git fallback failed ({cli_err}); gix fallback failed ({gix_err})"
                        )))
                    })
                }
            };
        }

        let repo = self._repo.to_thread_local();
        if let Some(branches) = try_collect_loose_local_branches_fast(&repo)? {
            return Ok(branches);
        }
        collect_local_branches(&repo, false)
    }

    fn current_branch_gix(&self) -> Result<String> {
        let repo = self._repo.to_thread_local();
        let head = repo
            .head()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head: {e}"))))?;

        Ok(match head.referent_name() {
            Some(referent) => referent.shorten().to_str_lossy().into_owned(),
            None => "HEAD".to_string(),
        })
    }

    fn current_branch_cli(&self) -> Result<String> {
        let mut symbolic = self.git_workdir_cmd();
        symbolic
            .arg("symbolic-ref")
            .arg("--quiet")
            .arg("--short")
            .arg("HEAD");
        let symbolic_label = "git symbolic-ref --short HEAD";
        let symbolic_output = run_git_raw_output(symbolic, symbolic_label)?;

        if symbolic_output.status.success() {
            let branch = bytes_to_text_preserving_utf8(&symbolic_output.stdout)
                .trim()
                .to_string();
            if !branch.is_empty() {
                return Ok(branch);
            }
        }

        let mut verify = self.git_workdir_cmd();
        verify.arg("rev-parse").arg("--verify").arg("HEAD");
        let verify_label = "git rev-parse --verify HEAD";
        let verify_output = run_git_raw_output(verify, verify_label)?;
        if verify_output.status.success() {
            return Ok("HEAD".to_string());
        }

        let symbolic_reason = probe_failure_reason(symbolic_label, &symbolic_output);
        let verify_reason = probe_failure_reason(verify_label, &verify_output);
        Err(Error::new(ErrorKind::Backend(format!(
            "{symbolic_reason}; {verify_reason}"
        ))))
    }

    fn list_branches_cli_with_tracking(&self) -> Result<Vec<Branch>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("for-each-ref")
            .arg("--format=%(refname:short)\t%(objectname)\t%(upstream:short)\t%(upstream:track)")
            .arg("refs/heads");
        let output = run_git_capture(
            cmd,
            "git for-each-ref --format=%(refname:short)\\t%(objectname)\\t%(upstream:short)\\t%(upstream:track) refs/heads",
        )?;
        parse_local_branches_for_each_ref(&output)
    }
}

fn collect_local_branches(
    repo: &gix::Repository,
    has_branch_tracking: bool,
) -> Result<Vec<Branch>> {
    let refs = repo
        .references()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;
    let iter = refs
        .local_branches()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix local_branches: {e}"))))?;

    let (branch_count_lower_bound, _) = iter.size_hint();
    let mut branches = Vec::with_capacity(branch_count_lower_bound);
    let mut target_ids = HashMap::default();
    let mut last_target = None;
    for reference in iter {
        let mut reference =
            reference.map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
        let target_id = branch_target_id(&mut reference)?;
        let name = local_branch_name(reference.name());
        let target = cached_commit_id(&mut target_ids, &mut last_target, target_id);

        let (upstream, divergence) = if has_branch_tracking {
            branch_upstream_and_divergence_best_effort(repo, &reference, target_id)?
        } else {
            (None, None)
        };

        branches.push(Branch {
            name,
            target,
            upstream,
            divergence,
        });
    }
    Ok(branches)
}

fn try_collect_loose_local_branches_fast(repo: &gix::Repository) -> Result<Option<Vec<Branch>>> {
    if repo_file_stamp(repo.common_dir().join("packed-refs").as_path()).exists {
        return Ok(None);
    }

    let root = repo.common_dir().join("refs").join("heads");
    if !root.exists() {
        return Ok(Some(Vec::new()));
    }

    let mut branches = Vec::new();
    let mut scratch = Vec::new();
    let mut target_ids = HashMap::default();
    let mut last_target = None;
    if !collect_loose_local_branches_fast(
        &root,
        &root,
        &mut scratch,
        &mut target_ids,
        &mut last_target,
        &mut branches,
    )? {
        return Ok(None);
    }
    branches.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(Some(branches))
}

fn parse_local_branches_for_each_ref(output: &str) -> Result<Vec<Branch>> {
    let mut branches = Vec::new();
    let mut target_ids = HashMap::default();
    let mut last_target = None;

    for (line_ix, line) in output.lines().enumerate() {
        if line.is_empty() {
            continue;
        }

        let mut fields = line.split('\t');
        let name = fields.next().unwrap_or_default();
        let target_hex = fields.next().unwrap_or_default();
        let upstream_short = fields.next().unwrap_or_default();
        let upstream_track = fields.next().unwrap_or_default();
        if fields.next().is_some() || name.is_empty() || target_hex.is_empty() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "git for-each-ref branches line {} malformed: {line}",
                line_ix + 1
            ))));
        }

        let target_id = gix::ObjectId::from_hex(target_hex.as_bytes()).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "git for-each-ref branch target {} invalid: {e}",
                target_hex
            )))
        })?;
        let upstream = parse_upstream_short(upstream_short);
        let divergence = upstream.as_ref().and_then(|_| {
            if upstream_track.trim().is_empty() {
                Some(UpstreamDivergence {
                    ahead: 0,
                    behind: 0,
                })
            } else {
                parse_upstream_track_divergence(upstream_track)
            }
        });

        branches.push(Branch {
            name: name.to_string(),
            target: cached_commit_id(&mut target_ids, &mut last_target, target_id),
            upstream,
            divergence,
        });
    }

    Ok(branches)
}

fn collect_loose_local_branches_fast(
    root: &Path,
    dir: &Path,
    scratch: &mut Vec<u8>,
    target_ids: &mut HashMap<gix::ObjectId, CommitId>,
    last_target: &mut Option<(gix::ObjectId, CommitId)>,
    branches: &mut Vec<Branch>,
) -> Result<bool> {
    for entry in std::fs::read_dir(dir).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "read refs dir {}: {e}",
            dir.display()
        )))
    })? {
        let entry = entry.map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "read refs dir entry {}: {e}",
                dir.display()
            )))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "read refs file type {}: {e}",
                path.display()
            )))
        })?;

        if file_type.is_dir() {
            if !collect_loose_local_branches_fast(
                root,
                &path,
                scratch,
                target_ids,
                last_target,
                branches,
            )? {
                return Ok(false);
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        scratch.clear();
        File::open(&path)
            .and_then(|mut file| file.read_to_end(scratch))
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "read branch ref {}: {e}",
                    path.display()
                )))
            })?;

        let Some(target_id) = parse_loose_ref_target_id(scratch) else {
            return Ok(false);
        };

        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let name = path_to_git_ref_name(relative);
        let target = cached_commit_id(target_ids, last_target, target_id);
        branches.push(Branch {
            name,
            target,
            upstream: None,
            divergence: None,
        });
    }
    Ok(true)
}

fn parse_loose_ref_target_id(buf: &[u8]) -> Option<gix::ObjectId> {
    let trimmed = buf.strip_suffix(b"\n").unwrap_or(buf);
    let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
    if trimmed.starts_with(b"ref: ") {
        return None;
    }
    gix::ObjectId::from_hex(trimmed).ok()
}

fn path_to_git_ref_name(path: &Path) -> String {
    let mut name = String::new();
    for component in path.components() {
        if !name.is_empty() {
            name.push('/');
        }
        name.push_str(component.as_os_str().to_string_lossy().as_ref());
    }
    name
}

fn probe_failure_reason(label: &str, output: &Output) -> String {
    if output.status.success() {
        return format!("{label} returned empty stdout");
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        format!("{label} failed")
    } else {
        format!("{label} failed: {detail}")
    }
}

fn parse_upstream_short(s: &str) -> Option<Upstream> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (remote, branch) = s.split_once('/')?;
    Some(Upstream {
        remote: remote.to_string(),
        branch: branch.to_string(),
    })
}

fn repo_file_stamp(path: &Path) -> RepoFileStamp {
    match std::fs::metadata(path) {
        Ok(metadata) => RepoFileStamp {
            exists: true,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        },
        Err(_) => RepoFileStamp::default(),
    }
}

fn repo_has_branch_tracking_config(repo: &gix::Repository) -> bool {
    repo.config_snapshot()
        .plumbing()
        .sections_by_name("branch")
        .is_some_and(|mut sections| sections.next().is_some())
}

fn local_branch_name(name: &gix::refs::FullNameRef) -> String {
    name.as_bstr()
        .strip_prefix(LOCAL_BRANCH_PREFIX)
        .unwrap_or_else(|| name.as_bstr())
        .to_str_lossy()
        .into_owned()
}

fn branch_target_id(reference: &mut gix::Reference<'_>) -> Result<gix::ObjectId> {
    match &reference.inner.target {
        gix::refs::Target::Object(oid) => Ok(oid.to_owned()),
        gix::refs::Target::Symbolic(_) => reference
            .peel_to_id()
            .map(|id| id.detach())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel branch: {e}")))),
    }
}

fn cached_commit_id(
    cache: &mut HashMap<gix::ObjectId, CommitId>,
    last_target: &mut Option<(gix::ObjectId, CommitId)>,
    target_id: gix::ObjectId,
) -> CommitId {
    if let Some((cached_oid, commit_id)) = last_target.as_ref()
        && *cached_oid == target_id
    {
        return commit_id.clone();
    }

    if let Some(commit_id) = cache.get(&target_id) {
        let commit_id = commit_id.clone();
        *last_target = Some((target_id, commit_id.clone()));
        return commit_id;
    }

    let commit_id = CommitId(oid_to_arc_str(&target_id));
    cache.insert(target_id, commit_id.clone());
    *last_target = Some((target_id, commit_id.clone()));
    commit_id
}

fn parse_upstream_track_divergence(raw: &str) -> Option<UpstreamDivergence> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "[gone]" || raw == "gone" {
        return None;
    }

    let inner = raw
        .strip_prefix('[')
        .and_then(|raw| raw.strip_suffix(']'))
        .unwrap_or(raw);

    let mut ahead = 0usize;
    let mut behind = 0usize;
    let mut saw_count = false;
    for component in inner
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(value) = component.strip_prefix("ahead ") {
            ahead = value.parse().ok()?;
            saw_count = true;
            continue;
        }
        if let Some(value) = component.strip_prefix("behind ") {
            behind = value.parse().ok()?;
            saw_count = true;
            continue;
        }
        return None;
    }

    saw_count.then_some(UpstreamDivergence { ahead, behind })
}

fn count_unique_commits(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    hidden_tip: gix::ObjectId,
) -> Result<usize> {
    let walk = repo
        .rev_walk([tip])
        .with_hidden([hidden_tip])
        .all()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev_walk: {e}"))))?;

    let mut count = 0usize;
    for info in walk {
        info.map_err(|e| Error::new(ErrorKind::Backend(format!("gix rev_walk item: {e}"))))?;
        count = count.saturating_add(1);
    }
    Ok(count)
}

fn divergence_between(
    repo: &gix::Repository,
    local_tip: gix::ObjectId,
    upstream_tip: gix::ObjectId,
) -> Result<UpstreamDivergence> {
    let ahead = count_unique_commits(repo, local_tip, upstream_tip)?;
    let behind = count_unique_commits(repo, upstream_tip, local_tip)?;
    Ok(UpstreamDivergence { ahead, behind })
}

fn branch_upstream_and_divergence(
    repo: &gix::Repository,
    branch_ref: &gix::Reference<'_>,
    local_tip: gix::ObjectId,
) -> Result<(Option<Upstream>, Option<UpstreamDivergence>)> {
    let Some((upstream, upstream_tip)) = branch_upstream_target(repo, branch_ref)? else {
        return Ok((None, None));
    };

    let divergence = divergence_between(repo, local_tip, upstream_tip).map(Some)?;

    Ok((Some(upstream), divergence))
}

fn branch_upstream_and_divergence_best_effort(
    repo: &gix::Repository,
    branch_ref: &gix::Reference<'_>,
    local_tip: gix::ObjectId,
) -> Result<(Option<Upstream>, Option<UpstreamDivergence>)> {
    let Some((upstream, upstream_tip)) = branch_upstream_target(repo, branch_ref)? else {
        return Ok((None, None));
    };

    let divergence = divergence_between(repo, local_tip, upstream_tip).ok();
    Ok((Some(upstream), divergence))
}

fn branch_upstream_target(
    repo: &gix::Repository,
    branch_ref: &gix::Reference<'_>,
) -> Result<Option<(Upstream, gix::ObjectId)>> {
    let tracking_ref_name = match branch_ref.remote_tracking_ref_name(gix::remote::Direction::Fetch)
    {
        Some(Ok(name)) => name,
        Some(Err(_)) | None => return Ok(None),
    };

    let upstream_short = tracking_ref_name.shorten().to_str_lossy().into_owned();
    let Some(upstream) = parse_upstream_short(&upstream_short) else {
        return Ok(None);
    };

    let Some(mut tracking_ref) = repo
        .try_find_reference(tracking_ref_name.as_ref())
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
    else {
        return Ok(None);
    };

    let upstream_tip = match tracking_ref.try_id() {
        Some(id) => id.detach(),
        None => match tracking_ref.peel_to_id() {
            Ok(id) => id.detach(),
            Err(_) => return Ok(None),
        },
    };

    Ok(Some((upstream, upstream_tip)))
}

#[cfg(test)]
mod tests {
    use super::{
        LOCAL_BRANCH_PREFIX, cached_commit_id, local_branch_name,
        parse_local_branches_for_each_ref, parse_upstream_short, parse_upstream_track_divergence,
    };
    use gitcomet_core::domain::UpstreamDivergence;
    use rustc_hash::FxHashMap as HashMap;
    use std::sync::Arc;

    #[test]
    fn parse_upstream_short_requires_remote_and_branch() {
        assert!(parse_upstream_short("").is_none());
        assert!(parse_upstream_short("origin").is_none());
        assert_eq!(
            parse_upstream_short("origin/main").map(|upstream| (upstream.remote, upstream.branch)),
            Some(("origin".to_string(), "main".to_string()))
        );
    }

    #[test]
    fn parse_upstream_short_preserves_nested_branch_names() {
        assert_eq!(
            parse_upstream_short("origin/feature/topic")
                .map(|upstream| (upstream.remote, upstream.branch)),
            Some(("origin".to_string(), "feature/topic".to_string()))
        );
    }

    #[test]
    fn cached_commit_id_reuses_existing_arc_for_same_object_id() {
        let oid = gix::ObjectId::from_hex(b"0123456789abcdef0123456789abcdef01234567")
            .expect("valid object id");
        let mut cache = HashMap::default();
        let mut last_target = None;

        let first = cached_commit_id(&mut cache, &mut last_target, oid);
        let second = cached_commit_id(&mut cache, &mut last_target, oid);

        assert_eq!(first, second);
        assert!(Arc::ptr_eq(&first.0, &second.0));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn local_branch_name_strips_heads_prefix() {
        let full_name = gix::refs::FullName::try_from(format!(
            "{}feature/topic",
            std::str::from_utf8(LOCAL_BRANCH_PREFIX).expect("utf8 prefix")
        ))
        .expect("valid ref name");

        assert_eq!(local_branch_name(full_name.as_ref()), "feature/topic");
    }

    #[test]
    fn parse_upstream_track_divergence_supports_ahead_and_behind_counts() {
        assert_eq!(
            parse_upstream_track_divergence("[ahead 3, behind 2]"),
            Some(UpstreamDivergence {
                ahead: 3,
                behind: 2,
            })
        );
        assert_eq!(
            parse_upstream_track_divergence("[ahead 4]"),
            Some(UpstreamDivergence {
                ahead: 4,
                behind: 0,
            })
        );
        assert_eq!(
            parse_upstream_track_divergence("[behind 5]"),
            Some(UpstreamDivergence {
                ahead: 0,
                behind: 5,
            })
        );
        assert_eq!(parse_upstream_track_divergence("[gone]"), None);
        assert_eq!(parse_upstream_track_divergence(""), None);
    }

    #[test]
    fn parse_local_branches_for_each_ref_parses_upstream_metadata() {
        let branches = parse_local_branches_for_each_ref(
            "feature\t0123456789abcdef0123456789abcdef01234567\torigin/feature\t[ahead 1, behind 2]\nmain\t89abcdef0123456789abcdef0123456789abcdef\t\t\n",
        )
        .expect("parse branch output");

        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].name, "feature");
        assert_eq!(
            branches[0]
                .upstream
                .as_ref()
                .map(|upstream| (upstream.remote.as_str(), upstream.branch.as_str(),)),
            Some(("origin", "feature"))
        );
        assert_eq!(
            branches[0].divergence,
            Some(UpstreamDivergence {
                ahead: 1,
                behind: 2,
            })
        );
        assert_eq!(branches[1].name, "main");
        assert_eq!(branches[1].upstream, None);
        assert_eq!(branches[1].divergence, None);
    }

    #[test]
    fn parse_local_branches_for_each_ref_treats_empty_track_as_in_sync() {
        let branches = parse_local_branches_for_each_ref(
            "main\t0123456789abcdef0123456789abcdef01234567\torigin/main\t\n",
        )
        .expect("parse branch output");

        assert_eq!(
            branches[0].divergence,
            Some(UpstreamDivergence {
                ahead: 0,
                behind: 0,
            })
        );
    }
}
