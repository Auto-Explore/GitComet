//! Hexadecimal adapters using faster-hex, shared by persisted paths and Git IDs.

pub use faster_hex::hex_string as encode;

/// Decodes hexadecimal text (either case) into bytes.
pub fn decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = vec![0; hex.len() / 2];
    faster_hex::hex_decode(hex.as_bytes(), &mut out).ok()?;
    Some(out)
}

/// Decode a fixed-size value without a heap allocation. Require an exact
/// length: the library also supports decoding into a shorter destination.
pub fn decode_array<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != N * 2 {
        return None;
    }
    let mut out = [0; N];
    faster_hex::hex_decode(hex.as_bytes(), &mut out).ok()?;
    Some(out)
}

/// Encode a supported Git object ID with a stack buffer and one shared allocation.
pub fn encode_object_id(bytes: &[u8]) -> std::sync::Arc<str> {
    assert!(matches!(bytes.len(), 20 | 32));
    let mut buffer = [0u8; 64];
    let hex: &str = faster_hex::hex_encode(bytes, &mut buffer[..bytes.len() * 2])
        .expect("Git object ID fits the buffer");
    std::sync::Arc::from(hex)
}

/// Compare binary bytes to hexadecimal text without a heap allocation.
pub fn matches(bytes: &[u8], hex: &str) -> bool {
    if bytes.len().checked_mul(2) != Some(hex.len()) {
        return false;
    }
    let mut buffer = [0; 32];
    bytes
        .chunks(32)
        .zip(hex.as_bytes().chunks(64))
        .all(|(bytes, hex)| {
            let decoded = &mut buffer[..bytes.len()];
            faster_hex::hex_decode(hex, decoded).is_ok() && decoded == bytes
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_produces_lowercase_hex() {
        assert_eq!(encode(&[]), "");
        assert_eq!(encode(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[test]
    fn decode_accepts_both_cases_and_rejects_odd_length() {
        assert_eq!(decode(""), Some(Vec::new()));
        assert_eq!(decode("000fa0ff"), Some(vec![0x00, 0x0f, 0xa0, 0xff]));
        assert_eq!(decode("000FA0FF"), Some(vec![0x00, 0x0f, 0xa0, 0xff]));
        assert_eq!(decode("0"), None);
        assert_eq!(decode("0g"), None);
    }

    #[test]
    fn encode_decode_round_trips() {
        let bytes = b"round trip \xff bytes";
        assert_eq!(decode(&encode(bytes)).as_deref(), Some(&bytes[..]));
    }

    #[test]
    fn git_id_adapters_keep_exact_lengths_and_reject_invalid_input() {
        for len in [20, 32] {
            let bytes: Vec<_> = (0..len).map(|i| 0xa0 + i as u8).collect();
            let hex = encode_object_id(&bytes);
            assert_eq!(hex.as_ref(), encode(&bytes));
            assert!(matches(&bytes, &hex));
            assert!(matches(&bytes, &hex.to_ascii_uppercase()));
            assert!(!matches(&bytes, &hex[..hex.len() - 1]));
            assert!(!matches(&bytes, &format!("{hex}00")));
            assert!(!matches(&bytes, &format!("{}z0", &hex[..hex.len() - 2])));
        }
        assert_eq!(decode_array::<2>("aB01"), Some([0xab, 1]));
        for invalid in ["", "ab", "abc", "abcdef", "zzzz", "é00"] {
            assert_eq!(decode_array::<2>(invalid), None, "{invalid:?}");
        }
        let bytes: Vec<u8> = (0..=255).collect();
        assert!(
            matches(&bytes, &encode(&bytes)),
            "comparison crosses stack-buffer boundaries"
        );
        assert!(matches(&[], ""));
        assert_eq!(decode_array::<0>(""), Some([]));
    }
}
