use super::GixRepo;
use crate::util::run_git_capture;
use gitcomet_core::domain::{Branch, CommitId, Upstream, UpstreamDivergence};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use std::process::Command;

pub(super) struct GitOps<'repo> {
    gix: GixOps<'repo>,
    cli: CliOps<'repo>,
}

impl<'repo> GitOps<'repo> {
    pub(super) fn new(repo: &'repo GixRepo) -> Self {
        Self {
            gix: GixOps { repo },
            cli: CliOps { repo },
        }
    }

    pub(super) fn current_branch(&self) -> Result<String> {
        prefer_gix_with_fallback(
            || self.gix.current_branch(),
            || self.cli.current_branch(),
            "current branch",
        )
    }

    pub(super) fn list_branches(&self) -> Result<Vec<Branch>> {
        // Branch upstream tracking is config-driven (`branch.*`) and may change during the app
        // lifetime (e.g. after `push -u`). Prefer CLI reads so we always observe the latest
        // on-disk config without requiring a repo reopen.
        prefer_cli_with_fallback(
            || self.cli.list_branches(),
            || self.gix.list_branches(),
            "list branches",
        )
    }
}

fn prefer_gix_with_fallback<T>(
    gix_call: impl FnOnce() -> Result<T>,
    cli_call: impl FnOnce() -> Result<T>,
    op_label: &str,
) -> Result<T> {
    match gix_call() {
        Ok(value) => Ok(value),
        Err(gix_err) => cli_call().map_err(|cli_err| {
            Error::new(ErrorKind::Backend(format!(
                "{op_label}: gix path failed ({gix_err}); cli fallback failed ({cli_err})"
            )))
        }),
    }
}

fn prefer_cli_with_fallback<T>(
    cli_call: impl FnOnce() -> Result<T>,
    gix_call: impl FnOnce() -> Result<T>,
    op_label: &str,
) -> Result<T> {
    match cli_call() {
        Ok(value) => Ok(value),
        Err(cli_err) => gix_call().map_err(|gix_err| {
            Error::new(ErrorKind::Backend(format!(
                "{op_label}: cli path failed ({cli_err}); gix fallback failed ({gix_err})"
            )))
        }),
    }
}

struct GixOps<'repo> {
    repo: &'repo GixRepo,
}

impl GixOps<'_> {
    fn current_branch(&self) -> Result<String> {
        let repo = self.repo._repo.to_thread_local();
        let head = repo
            .head()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head: {e}"))))?;

        Ok(match head.referent_name() {
            Some(referent) => referent.shorten().to_str_lossy().into_owned(),
            None => "HEAD".to_string(),
        })
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        let repo = self.repo._repo.to_thread_local();
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
                target: CommitId(target.to_string()),
                upstream,
                divergence,
            });
        }

        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }
}

struct CliOps<'repo> {
    repo: &'repo GixRepo,
}

impl CliOps<'_> {
    fn current_branch(&self) -> Result<String> {
        let mut symbolic = Command::new("git");
        symbolic
            .arg("-C")
            .arg(&self.repo.spec.workdir)
            .arg("symbolic-ref")
            .arg("--quiet")
            .arg("--short")
            .arg("HEAD");
        let symbolic_output = symbolic
            .output()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

        if symbolic_output.status.success() {
            let branch = String::from_utf8_lossy(&symbolic_output.stdout)
                .trim()
                .to_string();
            if !branch.is_empty() {
                return Ok(branch);
            }
        }

        let mut verify = Command::new("git");
        verify
            .arg("-C")
            .arg(&self.repo.spec.workdir)
            .arg("rev-parse")
            .arg("--verify")
            .arg("HEAD");
        let verify_output = verify
            .output()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        if verify_output.status.success() {
            return Ok("HEAD".to_string());
        }

        let symbolic_stderr = String::from_utf8_lossy(&symbolic_output.stderr)
            .trim()
            .to_string();
        let verify_stderr = String::from_utf8_lossy(&verify_output.stderr)
            .trim()
            .to_string();
        let reason = [symbolic_stderr, verify_stderr]
            .into_iter()
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>()
            .join("; ");

        Err(Error::new(ErrorKind::Backend(if reason.is_empty() {
            "git symbolic-ref --short HEAD and git rev-parse --verify HEAD failed".to_string()
        } else {
            format!(
                "git symbolic-ref --short HEAD and git rev-parse --verify HEAD failed: {reason}"
            )
        })))
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.repo.spec.workdir)
            .arg("for-each-ref")
            .arg("--format=%(refname:short)%00%(objectname)%00%(upstream:short)")
            .arg("refs/heads");
        let output = run_git_capture(cmd, "git for-each-ref refs/heads")?;

        let mut branches = Vec::new();
        for line in output.lines() {
            let Some((name, target, upstream_short)) = parse_branch_record(line) else {
                continue;
            };

            let upstream = parse_upstream_short(upstream_short);
            let divergence = if upstream.is_some() {
                self.branch_divergence(name, upstream_short)?
            } else {
                None
            };

            branches.push(Branch {
                name: name.to_string(),
                target: CommitId(target.to_string()),
                upstream,
                divergence,
            });
        }

        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }

    fn branch_divergence(
        &self,
        local_branch: &str,
        upstream_ref: &str,
    ) -> Result<Option<UpstreamDivergence>> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.repo.spec.workdir)
            .arg("rev-list")
            .arg("--left-right")
            .arg("--count")
            .arg(format!("{upstream_ref}...{local_branch}"));

        let output = cmd
            .output()
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        if !output.status.success() {
            return Ok(None);
        }

        Ok(parse_rev_list_counts(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }
}

fn parse_branch_record(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.split('\0');
    let name = parts.next()?.trim();
    let target = parts.next()?.trim();
    let upstream = parts.next().unwrap_or_default().trim();
    if name.is_empty() || target.is_empty() {
        return None;
    }
    Some((name, target, upstream))
}

fn parse_rev_list_counts(stdout: &str) -> Option<UpstreamDivergence> {
    let mut parts = stdout.split_whitespace();
    let behind = parts.next()?.parse::<usize>().ok()?;
    let ahead = parts.next()?.parse::<usize>().ok()?;
    Some(UpstreamDivergence { ahead, behind })
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

    let divergence = match upstream {
        Some(_) => Some(divergence_between(repo, local_tip, upstream_tip)?),
        None => None,
    };

    Ok((upstream, divergence))
}

#[cfg(test)]
mod tests {
    use super::{
        parse_branch_record, parse_rev_list_counts, parse_upstream_short, prefer_gix_with_fallback,
    };
    use gitcomet_core::domain::UpstreamDivergence;
    use gitcomet_core::error::ErrorKind;

    #[test]
    fn parse_branch_record_parses_name_target_and_upstream() {
        assert_eq!(
            parse_branch_record("feature\0abc123\0origin/feature"),
            Some(("feature", "abc123", "origin/feature"))
        );
    }

    #[test]
    fn parse_branch_record_accepts_empty_upstream() {
        assert_eq!(
            parse_branch_record("main\0deadbeef\0"),
            Some(("main", "deadbeef", ""))
        );
    }

    #[test]
    fn parse_branch_record_rejects_missing_required_fields() {
        assert_eq!(parse_branch_record(""), None);
        assert_eq!(parse_branch_record("\0deadbeef\0origin/main"), None);
        assert_eq!(parse_branch_record("main\0\0origin/main"), None);
    }

    #[test]
    fn parse_rev_list_counts_maps_behind_then_ahead() {
        assert_eq!(
            parse_rev_list_counts("3\t5\n"),
            Some(UpstreamDivergence {
                ahead: 5,
                behind: 3
            })
        );
    }

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
    fn prefer_gix_with_fallback_uses_gix_on_success() {
        let value = prefer_gix_with_fallback(
            || Ok::<_, gitcomet_core::error::Error>("gix".to_string()),
            || Ok::<_, gitcomet_core::error::Error>("cli".to_string()),
            "op",
        )
        .expect("gix should succeed");

        assert_eq!(value, "gix");
    }

    #[test]
    fn prefer_gix_with_fallback_uses_cli_when_gix_fails() {
        let value = prefer_gix_with_fallback(
            || {
                Err(gitcomet_core::error::Error::new(ErrorKind::Backend(
                    "gix failed".to_string(),
                )))
            },
            || Ok::<_, gitcomet_core::error::Error>("cli".to_string()),
            "op",
        )
        .expect("cli fallback should succeed");

        assert_eq!(value, "cli");
    }

    #[test]
    fn prefer_gix_with_fallback_reports_both_failures() {
        let err = prefer_gix_with_fallback::<String>(
            || {
                Err(gitcomet_core::error::Error::new(ErrorKind::Backend(
                    "gix failed".to_string(),
                )))
            },
            || {
                Err(gitcomet_core::error::Error::new(ErrorKind::Backend(
                    "cli failed".to_string(),
                )))
            },
            "op",
        )
        .expect_err("both paths should fail");

        let ErrorKind::Backend(message) = err.kind() else {
            panic!("expected backend error, got {:?}", err.kind());
        };
        assert!(message.contains("op: gix path failed"));
        assert!(message.contains("gix failed"));
        assert!(message.contains("cli fallback failed"));
        assert!(message.contains("cli failed"));
    }
}
