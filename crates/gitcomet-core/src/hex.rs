//! Hexadecimal byte encoding and decoding.

/// Renders bytes as lowercase hexadecimal text.
pub fn encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Decodes hexadecimal text (either case) into bytes.
pub fn decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.as_chunks::<2>().0 {
        let high = nibble(pair[0])?;
        let low = nibble(pair[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

/// Encode a supported Git object ID with a stack buffer and one shared allocation.
pub fn encode_object_id(bytes: &[u8]) -> std::sync::Arc<str> {
    assert!(matches!(bytes.len(), 20 | 32));
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut buffer = [0u8; 64];
    for (byte, pair) in bytes.iter().zip(buffer.as_chunks_mut::<2>().0) {
        pair[0] = HEX[(byte >> 4) as usize];
        pair[1] = HEX[(byte & 15) as usize];
    }
    std::sync::Arc::from(std::str::from_utf8(&buffer[..bytes.len() * 2]).unwrap())
}

/// Compare binary bytes to hexadecimal text without a temporary buffer.
pub fn matches(bytes: &[u8], hex: &str) -> bool {
    hex.len() == bytes.len() * 2
        && bytes
            .iter()
            .zip(hex.as_bytes().as_chunks::<2>().0)
            .all(|(&byte, pair)| {
                nibble(pair[0])
                    .zip(nibble(pair[1]))
                    .is_some_and(|(hi, lo)| byte == hi * 16 + lo)
            })
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
}
