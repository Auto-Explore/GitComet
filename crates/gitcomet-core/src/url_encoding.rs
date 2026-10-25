//! URL escaping policies shared by links, file-manager URIs and crash reports.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, PercentEncode};
use std::borrow::Cow;

// RFC 3986 unreserved characters remain readable in every component.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
const PATH: &AsciiSet = &COMPONENT.remove(b'/');

pub fn encode_component(value: &str) -> PercentEncode<'_> {
    percent_encoding::utf8_percent_encode(value, COMPONENT)
}

/// Preserve path separators and encode arbitrary filesystem bytes, including
/// non-UTF-8 Unix filenames. Separator normalization belongs to the caller.
pub fn encode_path(path: &[u8]) -> PercentEncode<'_> {
    percent_encoding::percent_encode(path, PATH)
}

/// Decode link text only when every escape and the resulting UTF-8 are valid.
/// `percent-encoding` tolerates malformed escapes; links retain their entire
/// original spelling in that case, rather than partially decoding a filename.
pub fn decode_utf8(value: &str) -> Cow<'_, str> {
    let Some(first_escape) = value.find('%') else {
        return Cow::Borrowed(value);
    };
    let escaped = &value.as_bytes()[first_escape..];
    if memchr::memchr_iter(b'%', escaped).any(|offset| {
        !escaped
            .get(offset + 1..offset + 3)
            .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
    }) {
        return Cow::Borrowed(value);
    }
    // Reserve once instead of growing the library's initial decoded prefix.
    let mut decoded = Vec::with_capacity(value.len());
    decoded.extend_from_slice(&value.as_bytes()[..first_escape]);
    decoded.extend(percent_encoding::percent_decode_str(&value[first_escape..]));
    String::from_utf8(decoded)
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_and_path_policies_preserve_existing_url_spellings() {
        let input = "AZaz09-_.~/ä +?#%\\\0";
        assert_eq!(
            encode_component(input).to_string(),
            "AZaz09-_.~%2F%C3%A4%20%2B%3F%23%25%5C%00"
        );
        assert_eq!(
            encode_path(input.as_bytes()).to_string(),
            "AZaz09-_.~/%C3%A4%20%2B%3F%23%25%5C%00"
        );
        assert_eq!(encode_path(b"/tmp/\xff").to_string(), "/tmp/%FF");
    }

    #[test]
    fn decoding_preserves_malformed_input_and_does_not_use_form_rules() {
        assert!(matches!(decode_utf8("plain/ä+file"), Cow::Borrowed(_)));
        assert_eq!(decode_utf8("%c3%a4+%2B%252F"), "ä++%2F");
        for invalid in ["a%20b%", "%2", "%GG", "%20%%32", "%20%FF", "%C3%28", "%é"] {
            assert_eq!(decode_utf8(invalid), invalid);
            assert!(matches!(decode_utf8(invalid), Cow::Borrowed(_)));
        }
    }
}
