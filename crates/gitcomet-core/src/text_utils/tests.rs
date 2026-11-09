#[test]
fn common_affix_len_never_counts_an_element_twice() {
    use super::common_affix_len;
    for (a, b, expected) in [
        ("", "", (0, 0)),
        ("abc", "abc", (3, 0)),
        ("aa", "aaa", (2, 0)),
        ("axc", "ayc", (1, 1)),
        ("abc", "xbc", (0, 2)),
        ("ab", "", (0, 0)),
    ] {
        assert_eq!(
            common_affix_len(a.as_bytes(), b.as_bytes()),
            expected,
            "{a:?} vs {b:?}"
        );
    }
}

#[test]
fn line_starts_begin_after_every_newline() {
    use super::line_starts;
    assert_eq!(line_starts(""), vec![0]);
    assert_eq!(line_starts("a"), vec![0]);
    assert_eq!(line_starts("a\n"), vec![0, 2]);
    assert_eq!(line_starts("a\r\nbc\n\nd"), vec![0, 3, 6, 7]);
}

#[test]
fn line_ranges_exclude_newlines_and_keep_the_trailing_empty_line() {
    use super::{line_byte_range, line_count_from_starts, line_starts};
    let text = "ab\n\ncd\n";
    let starts = line_starts(text);
    assert_eq!(line_count_from_starts(text.len(), &starts), 4);
    let ranges: Vec<_> = (0..5)
        .map(|ix| line_byte_range(&starts, text.len(), ix))
        .collect();
    assert_eq!(
        ranges,
        vec![Some(0..2), Some(3..3), Some(4..6), Some(7..7), None]
    );
    assert_eq!(line_count_from_starts(0, &line_starts("")), 0);
    assert_eq!(line_byte_range(&line_starts(""), 0, 0), None);
}

#[test]
fn redact_url_userinfo_masks_secrets_and_keeps_the_rest() {
    use super::redact_url_userinfo;
    assert_eq!(
        redact_url_userinfo("https://user:s3cret@example.com/org/repo.git"),
        "https://user:***@example.com/org/repo.git"
    );
    assert_eq!(
        redact_url_userinfo("https://ghp_token@github.com/org/repo.git"),
        "https://***@github.com/org/repo.git"
    );
    assert_eq!(
        redact_url_userinfo("HTTP://user@example.com:8080/repo?x=a@b#f"),
        "HTTP://***@example.com:8080/repo?x=a@b#f"
    );
    assert_eq!(
        redact_url_userinfo("ssh://git@example.com/org/repo.git"),
        "ssh://git@example.com/org/repo.git"
    );
    assert_eq!(
        redact_url_userinfo("ssh://git:pass@example.com/org/repo.git"),
        "ssh://git:***@example.com/org/repo.git"
    );
    for untouched in [
        "git@github.com:org/repo.git",
        "/tmp/repo.git",
        "https://example.com/repo?token=a@b",
        "",
    ] {
        assert_eq!(redact_url_userinfo(untouched), untouched);
    }
}

use super::{LineEndingDetectionMode, detect_line_ending_from_texts};

#[test]
fn detect_line_ending_presence_prefers_crlf_when_present() {
    assert_eq!(
        detect_line_ending_from_texts(
            ["ours\n", "theirs\r\n", "base\n"],
            LineEndingDetectionMode::Presence,
        ),
        "\r\n"
    );
}

#[test]
fn detect_line_ending_presence_prefers_cr_when_no_crlf() {
    assert_eq!(
        detect_line_ending_from_texts(
            ["ours\rmore", "theirs\n", "base"],
            LineEndingDetectionMode::Presence,
        ),
        "\r"
    );
}

#[test]
fn detect_line_ending_presence_defaults_to_lf() {
    assert_eq!(
        detect_line_ending_from_texts(["no separators"], LineEndingDetectionMode::Presence),
        "\n"
    );
    assert_eq!(
        detect_line_ending_from_texts([], LineEndingDetectionMode::Presence),
        "\n"
    );
}

#[test]
fn detect_line_ending_dominant_prefers_majority() {
    assert_eq!(
        detect_line_ending_from_texts(["a\r\nb\r\nc\n"], LineEndingDetectionMode::DominantCrlfVsLf,),
        "\r\n"
    );
    assert_eq!(
        detect_line_ending_from_texts(["a\r\nb\nc\n"], LineEndingDetectionMode::DominantCrlfVsLf,),
        "\n"
    );
}

#[test]
fn detect_line_ending_dominant_tie_defaults_to_lf() {
    assert_eq!(
        detect_line_ending_from_texts(["a\r\nb\n"], LineEndingDetectionMode::DominantCrlfVsLf,),
        "\n"
    );
    assert_eq!(
        detect_line_ending_from_texts([], LineEndingDetectionMode::DominantCrlfVsLf),
        "\n"
    );
}
