use super::*;

pub(crate) const DEFAULT_CRITERION_ROOTS: &[&str] = &["target/criterion", "criterion"];

pub(crate) fn default_criterion_roots() -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(DEFAULT_CRITERION_ROOTS.len());
    for root in DEFAULT_CRITERION_ROOTS {
        push_unique_criterion_root(&mut roots, PathBuf::from(root));
    }
    roots
}

pub(crate) fn push_unique_criterion_root(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    if roots.iter().any(|root| root == &candidate) {
        return;
    }
    roots.push(candidate);
}

pub(crate) fn parse_cli_args<I>(args: I) -> Result<(CliParseResult, CliArgs), String>
where
    I: IntoIterator<Item = String>,
{
    let mut criterion_roots = default_criterion_roots();
    let mut explicit_criterion_roots = false;
    let mut strict = strict_from_env();
    let mut skip_missing = false;
    let mut fresh_reference = None;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--criterion-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--criterion-root requires a path argument".to_string())?;
                if !explicit_criterion_roots {
                    criterion_roots.clear();
                    explicit_criterion_roots = true;
                }
                push_unique_criterion_root(&mut criterion_roots, PathBuf::from(value));
            }
            "--strict" => strict = true,
            "--skip-missing" => skip_missing = true,
            "--fresh-reference" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--fresh-reference requires a path argument".to_string())?;
                fresh_reference = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                return Ok((
                    CliParseResult::Help,
                    CliArgs {
                        criterion_roots,
                        strict,
                        skip_missing,
                        fresh_reference,
                    },
                ));
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    Ok((
        CliParseResult::Run,
        CliArgs {
            criterion_roots,
            strict,
            skip_missing,
            fresh_reference,
        },
    ))
}

pub(crate) fn strict_from_env() -> bool {
    match env::var("GITCOMET_PERF_BUDGET_STRICT") {
        Ok(value) => is_truthy(&value),
        Err(_) => false,
    }
}

pub(crate) fn is_truthy(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}

pub(crate) fn usage() -> &'static str {
    "Usage: cargo run -p gitcomet-ui-gpui --bin perf_budget_report -- [--criterion-root PATH]... [--strict] [--skip-missing] [--fresh-reference PATH]"
}
