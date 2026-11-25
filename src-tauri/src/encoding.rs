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

#[cfg(test)]
mod tests {
    use super::base64_encode;

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
}
