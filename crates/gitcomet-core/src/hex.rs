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
