//! Byte-to-text helpers for the Tauri IPC boundary.
//!
//! PTY output is arbitrary bytes (UTF-8 text, ANSI escapes, and partial
//! multi-byte sequences in the middle of a chunk). Serialising it as a JSON
//! number array would multiply the payload size by four, so it travels as
//! base64 and the terminal decides where the UTF-8 boundaries are.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (RFC 4648).
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode standard base64 (RFC 4648) with optional padding. Whitespace is
/// ignored; any other character is rejected.
pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    let mut padding = 0usize;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            other => return Err(format!("无效的 base64 字符：{}", other as char)),
        };
        if padding > 0 {
            return Err("base64 填充符后仍有多余数据".into());
        }
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    if bits >= 6 {
        return Err("base64 数据长度无效".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{base64_decode, base64_encode};

    #[test]
    fn matches_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn handles_ansi_escape_sequences_byte_for_byte() {
        // ESC [ 3 1 m — a colour change, as the shell would emit it.
        let bytes = b"\x1b[31mred\x1b[0m";
        let encoded = base64_encode(bytes);
        assert_eq!(encoded, "G1szMW1yZWQbWzBt");
        assert_eq!(encoded.len() % 4, 0);
    }

    #[test]
    fn keeps_incomplete_utf8_sequences_intact() {
        // "中" is E4 B8 AD; sending it split across two chunks must not corrupt it.
        let first = base64_encode(&[0xE4]);
        let second = base64_encode(&[0xB8, 0xAD]);
        assert_eq!(first, "5A==");
        assert_eq!(second, "uK0=");
    }

    #[test]
    fn base64_round_trips_bytes() {
        for input in [
            b"".as_slice(),
            b"f".as_slice(),
            b"fo".as_slice(),
            b"foo".as_slice(),
            b"\x00\xff\x10binary".as_slice(),
        ] {
            let encoded = base64_encode(input);
            assert_eq!(base64_decode(&encoded).unwrap(), input);
        }
    }

    #[test]
    fn base64_decoder_rejects_invalid_input() {
        assert!(base64_decode("not base64!").is_err());
        assert!(base64_decode("Zg==Zg==").is_err());
    }
}
