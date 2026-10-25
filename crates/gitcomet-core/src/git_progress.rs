//! Git's progress meters (`Receiving objects:  45% (9/20)`), parsed for
//! progress notifications and stripped from captured output.
use std::sync::Arc;

/// One progress reading: git's phase title and how far through it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitProgressMeter {
    /// The phase, e.g. `Receiving objects`. Git localizes it; shown as is.
    pub title: Arc<str>,
    /// `None` for counting phases (`Enumerating objects: 1200`).
    pub percent: Option<u8>,
    /// Computed by GitComet rather than reported by git.
    pub estimated: bool,
}

impl GitProgressMeter {
    pub fn estimated(title: impl Into<Arc<str>>, percent: u8) -> Self {
        Self {
            title: title.into(),
            percent: Some(percent.min(100)),
            estimated: true,
        }
    }
}

/// Parses one redrawn segment of git's progress output. Git prints either
/// `<title>: NN% (x/y)[, throughput][, done.]` or `<title>: N[, done.]`,
/// optionally behind `remote: ` for the server's phases.
pub fn parse_progress_fragment(fragment: &str) -> Option<GitProgressMeter> {
    let fragment = fragment.trim();
    let fragment = fragment.strip_prefix("remote: ").unwrap_or(fragment);
    let (title, rest) = fragment.split_once(": ")?;
    if title.is_empty() || title.len() > 80 || title.contains(':') {
        return None;
    }
    let rest = rest.trim_start();
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let number = rest[..digits].parse::<u64>().ok()?;
    let after = &rest[digits..];
    let percent = if let Some(after_percent) = after.strip_prefix('%') {
        // `(x/y)` follows every percentage git prints.
        if !after_percent.trim_start().starts_with('(') {
            return None;
        }
        Some(number.min(100) as u8)
    } else if after.is_empty() || after.starts_with(", done") {
        None
    } else {
        return None;
    };
    Some(GitProgressMeter {
        title: Arc::from(title),
        percent,
        estimated: false,
    })
}

/// The newest complete meter in `output`: fragments are separated by `\r`
/// (redraw) or `\n`, and a trailing fragment may still be half written.
pub fn latest_progress(output: &str) -> Option<GitProgressMeter> {
    let complete = output.rfind(['\r', '\n']).map(|end| &output[..end])?;
    complete
        .rsplit(['\r', '\n'])
        .take(8)
        .find_map(parse_progress_fragment)
}

/// `text` without progress meters, so captured output reads as it did before
/// `--progress` was asked for. A line redrawn with `\r` keeps its last
/// non-progress segment.
pub fn strip_progress(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        let kept = body.split('\r').rfind(|segment| {
            !segment.trim().is_empty()
                && parse_progress_fragment(segment).is_none()
                && !is_pack_total(segment)
        });
        if let Some(kept) = kept {
            stripped.push_str(kept);
            stripped.push_str(newline);
        }
    }
    stripped
}

/// `Total 5 (delta 0), reused 0 (delta 0), pack-reused 0`: the closing line
/// of the same progress output, printed only when progress was asked for.
fn is_pack_total(segment: &str) -> bool {
    let segment = segment.trim();
    let segment = segment.strip_prefix("remote: ").unwrap_or(segment);
    segment.strip_prefix("Total ").is_some_and(|rest| {
        rest.starts_with(|c: char| c.is_ascii_digit()) && rest.contains("(delta ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(title: &str, percent: Option<u8>) -> Option<GitProgressMeter> {
        Some(GitProgressMeter {
            title: Arc::from(title),
            percent,
            estimated: false,
        })
    }

    #[test]
    fn parses_percent_and_counting_meters() {
        assert_eq!(
            parse_progress_fragment("Receiving objects:  45% (9/20), 1.20 MiB | 2.40 MiB/s"),
            meter("Receiving objects", Some(45))
        );
        assert_eq!(
            parse_progress_fragment("remote: Counting objects: 100% (5/5), done."),
            meter("Counting objects", Some(100))
        );
        assert_eq!(
            parse_progress_fragment("remote: Enumerating objects: 1200, done."),
            meter("Enumerating objects", None)
        );
        assert_eq!(
            parse_progress_fragment("Unpacking objects:  33% (1/3)"),
            meter("Unpacking objects", Some(33))
        );
        assert_eq!(
            parse_progress_fragment("Writing out commit graph in 4 passes:  25% (1/4)"),
            meter("Writing out commit graph in 4 passes", Some(25))
        );
        // Localized titles pass through untouched.
        assert_eq!(
            parse_progress_fragment("Objekte empfangen:  10% (1/10)"),
            meter("Objekte empfangen", Some(10))
        );
    }

    #[test]
    fn ignores_messages_that_are_not_meters() {
        for line in [
            "From https://example.com/repo",
            "   abc1234..def5678  main       -> origin/main",
            "remote: Total 5 (delta 0), reused 0 (delta 0), pack-reused 0",
            "fatal: unable to access 'https://x/': Failed to connect to x port 443: refused",
            "hint: Waiting for your editor to close the file...",
            "Auto packing the repository for optimum performance.",
            "error: 5 is not a valid object",
            "",
        ] {
            assert_eq!(parse_progress_fragment(line), None, "{line}");
        }
    }

    #[test]
    fn latest_progress_skips_a_half_written_fragment() {
        let output = "remote: Counting objects: 100% (5/5), done.\nReceiving objects:  40% (2/5)\rReceiving objects:  6";
        assert_eq!(
            latest_progress(output),
            meter("Receiving objects", Some(40))
        );
        assert_eq!(latest_progress("Receiving objects:  6"), None);
    }

    #[test]
    fn strip_progress_keeps_messages_and_drops_meters() {
        let stderr = "remote: Enumerating objects: 5, done.\n\
             remote: Counting objects:  40% (2/5)\rremote: Counting objects: 100% (5/5), done.\n\
             remote: Total 5 (delta 0), reused 0 (delta 0), pack-reused 0 (from 0)        \n\
             Unpacking objects:  33% (1/3)\rUnpacking objects: 100% (3/3), 280 bytes | 280.00 KiB/s, done.\n\
             From /tmp/remote\n   abc..def  main -> origin/main\n";
        assert_eq!(
            strip_progress(stderr),
            "From /tmp/remote\n   abc..def  main -> origin/main\n"
        );
        assert_eq!(strip_progress("fatal: denied\n"), "fatal: denied\n");
        assert_eq!(strip_progress("no newline"), "no newline");
    }
}
