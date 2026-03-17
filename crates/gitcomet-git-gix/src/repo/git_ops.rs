use super::GixRepo;
use crate::util::{bytes_to_text_preserving_utf8, run_git_raw_output};
use gitcomet_core::domain::{Branch, CommitId, Upstream, UpstreamDivergence};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use std::process::Output;

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

    pub(super) fn list_branches_impl(&self) -> Result<Vec<Branch>> {
        // Upstream tracking is config-driven (`branch.*`) and can change while the backend stays
        // open, e.g. after `push -u`. Re-open for a fresh config snapshot before reading refs.
        let repo = self.reopen_repo()?;
        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;
        let iter = refs
            .local_branches()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix local_branches: {e}"))))?;

        let mut branches = Vec::new();
        for reference in iter {
            let mut reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
            let name = reference.name().shorten().to_str_lossy().into_owned();

            let target = match reference.try_id() {
                Some(id) => id.detach(),
                None => reference
                    .peel_to_id()
                    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel branch: {e}"))))?
                    .detach(),
            };

            let (upstream, divergence) = branch_upstream_and_divergence(&repo, &reference, target)?;

            branches.push(Branch {
                name,
                target: CommitId(target.to_string().into()),
                upstream,
                divergence,
            });
        }

        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
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
    let tracking_ref_name = match branch_ref.remote_tracking_ref_name(gix::remote::Direction::Fetch)
    {
        Some(Ok(name)) => name,
        Some(Err(_)) | None => return Ok((None, None)),
    };

    let upstream_short = tracking_ref_name.shorten().to_str_lossy().into_owned();
    let upstream = parse_upstream_short(&upstream_short);

    let Some(mut tracking_ref) = repo
        .try_find_reference(tracking_ref_name.as_ref())
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
    else {
        return Ok((upstream, None));
    };

    let upstream_tip = match tracking_ref.try_id() {
        Some(id) => id.detach(),
        None => match tracking_ref.peel_to_id() {
            Ok(id) => id.detach(),
            Err(_) => return Ok((upstream, None)),
        },
    };

    let divergence = upstream
        .as_ref()
        .map(|_| divergence_between(repo, local_tip, upstream_tip))
        .transpose()?;

    Ok((upstream, divergence))
}

#[cfg(test)]
mod tests {
    use super::parse_upstream_short;

    #[test]
    fn parse_upstream_short_requires_remote_and_branch() {
        assert!(parse_upstream_short("").is_none());
        assert!(parse_upstream_short("origin").is_none());
        assert_eq!(
            parse_upstream_short("origin/main").map(|upstream| (upstream.remote, upstream.branch)),
            Some(("origin".to_string(), "main".to_string()))
        );
    }
}
