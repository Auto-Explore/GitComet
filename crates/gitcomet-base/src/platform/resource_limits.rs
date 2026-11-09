//! Process-local descriptor headroom. A larger limit complements bounded
//! repository ownership; it does not replace it.

/// Raise the inherited soft descriptor limit before workers or child Git
/// processes start. Never lower a limit or change the hard limit. Failure is
/// nonfatal, including on restricted systems; callers may report it once.
pub fn ensure_file_descriptor_headroom() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, getrlimit, setrlimit};
        raise_limit(getrlimit(Resource::Nofile), |limit| {
            setrlimit(Resource::Nofile, limit).map_err(std::io::Error::from)
        })?;
    }
    Ok(())
}

#[cfg(unix)]
fn raise_limit(
    inherited: rustix::process::Rlimit,
    mut set: impl FnMut(rustix::process::Rlimit) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let Some(current) = inherited.current else {
        return Ok(()); // Already unlimited.
    };
    let mut previous = current;
    let mut failure = None;
    // macOS can impose a kernel ceiling below an unlimited hard limit. Try
    // smaller targets if necessary, without reading sysctls or changing them.
    for desired in [8192, 4096, 2048, 1024] {
        let target = inherited.maximum.map_or(desired, |hard| hard.min(desired));
        if target <= current || target == previous {
            continue;
        }
        previous = target;
        match set(rustix::process::Rlimit {
            current: Some(target),
            maximum: inherited.maximum,
        }) {
            Ok(()) => return Ok(()),
            Err(error) => failure = Some(error),
        }
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use rustix::process::Rlimit;

    #[test]
    fn respects_hard_limit_without_touching_it() {
        let mut attempts = Vec::new();
        raise_limit(
            Rlimit {
                current: Some(256),
                maximum: Some(3000),
            },
            |limit| {
                attempts.push((limit.current, limit.maximum));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(attempts, [(Some(3000), Some(3000))]);
    }

    #[test]
    fn never_lowers_an_existing_or_unlimited_limit() {
        for current in [Some(65536), None] {
            raise_limit(
                Rlimit {
                    current,
                    maximum: None,
                },
                |_| panic!("must not set limit"),
            )
            .unwrap();
        }
    }

    #[test]
    fn falls_back_below_kernel_ceiling() {
        let mut attempts = Vec::new();
        raise_limit(
            Rlimit {
                current: Some(256),
                maximum: None,
            },
            |limit| {
                let target = limit.current.unwrap();
                attempts.push(target);
                if target > 2048 {
                    Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(attempts, [8192, 4096, 2048]);
    }

    #[test]
    fn reports_denial_after_bounded_attempts() {
        let mut attempts = 0;
        let result = raise_limit(
            Rlimit {
                current: Some(256),
                maximum: Some(512),
            },
            |_| {
                attempts += 1;
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            },
        );
        assert_eq!(attempts, 1);
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
