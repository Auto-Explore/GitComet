use std::collections::VecDeque;
use std::sync::LazyLock;

pub const OUTPUT_LIMIT: usize = 16 * 1024;
pub const METADATA_LIMIT: usize = 16 * 1024;
const HEAD_LIMIT: usize = 4 * 1024;
const TAIL_LIMIT: usize = OUTPUT_LIMIT - HEAD_LIMIT;
const REDACTION_LOOKAHEAD: usize = 1024;
const REDACTION_OVERLAP: usize = 256;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputPreview {
    pub text: String,
    pub total_bytes: u64,
    pub omitted_bytes: u64,
    pub truncated: bool,
}

#[derive(Default)]
pub(super) struct OutputBuffer {
    retained: RetainedOutput,
    pending: Vec<u8>,
    redacting_long_line: bool,
    total: u64,
}

#[derive(Clone, Default)]
struct RetainedOutput {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
}

impl RetainedOutput {
    fn append(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len() as u64);
        let head = bytes.len().min(HEAD_LIMIT.saturating_sub(self.head.len()));
        self.head.extend_from_slice(&bytes[..head]);
        let remaining = &bytes[head..];
        if remaining.len() >= TAIL_LIMIT {
            self.tail.clear();
            self.tail.extend(&remaining[remaining.len() - TAIL_LIMIT..]);
            return;
        }
        let remove = (self.tail.len() + remaining.len()).saturating_sub(TAIL_LIMIT);
        self.tail.drain(..remove);
        self.tail.extend(remaining);
    }
}

impl OutputBuffer {
    pub(super) fn append(&mut self, mut bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len() as u64);
        while !bytes.is_empty() {
            if self.redacting_long_line {
                let Some(newline) = bytes.iter().position(|b| *b == b'\n') else {
                    return;
                };
                bytes = &bytes[newline + 1..];
                self.redacting_long_line = false;
                self.retained.append(b"\n");
                continue;
            }
            let count = bytes.len().min(REDACTION_LOOKAHEAD - self.pending.len());
            let part = &bytes[..count];
            let newline = part.iter().position(|b| *b == b'\n');
            let take = newline.map_or(count, |index| index + 1);
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if newline.is_some() {
                self.flush_pending();
            } else if self.pending.len() == REDACTION_LOOKAHEAD {
                let text = String::from_utf8_lossy(&self.pending);
                if text.contains("://") || sensitive_key(&text) {
                    self.retained
                        .append(b"<redacted oversized diagnostic line>");
                    self.pending.clear();
                    self.redacting_long_line = true;
                } else {
                    let safe =
                        complete_prefix_len(&self.pending, self.pending.len() - REDACTION_OVERLAP);
                    self.retained
                        .append(display_bytes(&self.pending[..safe]).as_bytes());
                    self.pending.drain(..safe);
                }
            }
        }
    }

    pub(super) fn flush_pending(&mut self) {
        self.retained
            .append(sanitize_pending(&self.pending).as_bytes());
        self.pending.clear();
    }

    pub(super) fn snapshot(&self, sensitive: bool) -> OutputPreview {
        let mut retained = self.retained.clone();
        retained.append(sanitize_pending(&self.pending).as_bytes());
        let omitted = self.total.saturating_sub(OUTPUT_LIMIT as u64);
        let truncated = omitted > 0 || retained.total > OUTPUT_LIMIT as u64;
        if sensitive && self.total > 0 {
            return OutputPreview {
                text: "<redacted>".into(),
                total_bytes: self.total,
                omitted_bytes: omitted,
                truncated,
            };
        }
        let tail: Vec<u8> = retained.tail.iter().copied().collect();
        let mut text = display_bytes(&retained.head);
        if omitted > 0 {
            text.push_str(&format!("\n… {omitted} bytes omitted …\n"));
        }
        text.push_str(&display_bytes(&tail));
        OutputPreview {
            text: bounded_output(&text),
            total_bytes: self.total,
            omitted_bytes: omitted,
            truncated,
        }
    }
}

fn complete_prefix_len(bytes: &[u8], end: usize) -> usize {
    let mut cursor = 0;
    while cursor < end {
        match std::str::from_utf8(&bytes[cursor..end]) {
            Ok(_) => return end,
            Err(error) => {
                cursor += error.valid_up_to();
                let Some(length) = error.error_len() else {
                    return cursor;
                };
                cursor += length;
            }
        }
    }
    end
}

fn sanitize_pending(bytes: &[u8]) -> String {
    static UNFINISHED_AUTHORITY: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"([a-zA-Z][a-zA-Z0-9+.-]*://)[^/\s?#]*$").unwrap());
    let text = display_bytes(bytes);
    let withheld = UNFINISHED_AUTHORITY.replace_all(&text, "${1}<withheld authority>");
    sanitize(&withheld)
}

fn bounded_output(text: &str) -> String {
    if text.len() <= OUTPUT_LIMIT {
        return text.to_owned();
    }
    let mut end = HEAD_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    let marker = "\n… display truncated …\n";
    let mut start = text.len() - (OUTPUT_LIMIT - head.len() - marker.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("{head}{marker}{}", &text[start..])
}

pub fn bounded_text(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

pub fn sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "token",
        "authorization",
        "extraheader",
        "secret",
    ]
    .iter()
    .any(|sensitive| lower.contains(sensitive))
}

pub fn sanitize(text: &str) -> String {
    static URL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"[a-zA-Z][a-zA-Z0-9+.-]*://[^\s<>\"']+"#).unwrap());
    static AUTH: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?im)((?:proxy-)?authorization\s*[:=]\s*)[^\r\n]+|((?:password|passwd|access_token|token|secret)\s*=\s*)[^\s&]+").unwrap()
    });
    let urls = URL.replace_all(text, |capture: &regex::Captures<'_>| {
        let url = &capture[0];
        let (scheme, rest) = url.split_once("://").expect("URL regex includes scheme");
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        if authority.contains(':') && !authority.contains('@') {
            return format!("{scheme}://<redacted>{}", &rest[authority.len()..]);
        }
        crate::text_utils::redact_url_userinfo(url)
    });
    AUTH.replace_all(&urls, |capture: &regex::Captures<'_>| {
        format!(
            "{}<redacted>",
            capture
                .get(1)
                .or_else(|| capture.get(2))
                .map_or("", |m| m.as_str())
        )
    })
    .into_owned()
}

fn display_bytes(bytes: &[u8]) -> String {
    let text = crate::process::bytes_to_text_preserving_utf8(bytes);
    let mut result = String::new();
    for ch in text.chars() {
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            result.extend(ch.escape_default());
        } else {
            result.push(ch);
        }
    }
    result
}
