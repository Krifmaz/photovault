//! TOTP (RFC 6238) as museum uses `github.com/pquerna/otp/totp`: SHA-1, 6 digits,
//! 30 s period, 20-byte secrets encoded as unpadded base32, ±1 period skew on validation.
//! Also renders the `otpauth://` URI as a PNG QR code (museum returns a 200×200 PNG).

use hmac::{Hmac, Mac};
use sha1::Sha1;

const PERIOD: u64 = 30;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Unpadded RFC 4648 base32.
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buf: u64 = 0;
    let mut bits = 0;
    for &b in data {
        buf = (buf << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Decode base32 (case-insensitive, padding and spaces ignored).
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf: u64 = 0;
    let mut bits = 0;
    for c in s.chars().filter(|c| *c != '=' && !c.is_whitespace()) {
        let v = ALPHABET
            .iter()
            .position(|a| *a as char == c.to_ascii_uppercase())? as u64;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// New random secret (20 bytes, base32).
pub fn generate_secret() -> String {
    base32_encode(&crate::crypto::random_bytes(20))
}

/// HOTP code for a counter.
fn hotp(key: &[u8], counter: u64) -> String {
    let mut m = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts keys of any length");
    m.update(&counter.to_be_bytes());
    let d = m.finalize().into_bytes();
    let off = (d[19] & 0x0f) as usize;
    let bin = ((d[off] as u32 & 0x7f) << 24)
        | ((d[off + 1] as u32) << 16)
        | ((d[off + 2] as u32) << 8)
        | d[off + 3] as u32;
    format!("{:06}", bin % 1_000_000)
}

/// Code for a unix time (seconds).
pub fn code_at(secret: &str, unix_secs: u64) -> Option<String> {
    Some(hotp(&base32_decode(secret)?, unix_secs / PERIOD))
}

/// Validate `code` against `secret` at the current time with ±1 step skew (pquerna default).
pub fn validate(code: &str, secret: &str) -> bool {
    let code = code.trim();
    if code.len() != 6 {
        return false;
    }
    let Some(key) = base32_decode(secret) else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let step = now / PERIOD;
    [step.saturating_sub(1), step, step + 1].iter().any(|c| {
        bool::from(subtle::ConstantTimeEq::ct_eq(
            hotp(&key, *c).as_bytes(),
            code.as_bytes(),
        ))
    })
}

/// `otpauth://` URI like pquerna's `Key.String()`.
pub fn uri(issuer: &str, account: &str, secret: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let path = format!("{issuer}:{account}");
    let path = path
        .split(':')
        .map(|p| enc(p).replace('+', "%20"))
        .collect::<Vec<_>>()
        .join(":");
    format!(
        "otpauth://totp/{path}?algorithm=SHA1&digits=6&issuer={}&period=30&secret={secret}",
        enc(issuer)
    )
}

/// Render `data` as a QR code PNG of roughly `size`×`size` pixels (grayscale, quiet zone).
pub fn qr_png(data: &str, size: u32) -> Option<Vec<u8>> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    let w = code.width() as u32;
    let modules = w + 8; // 4-module quiet zone on each side
    let scale = (size / modules).max(1);
    let px = modules * scale;
    let colors = code.to_colors();
    let mut raw = Vec::with_capacity(((px + 1) * px) as usize);
    for y in 0..px {
        raw.push(0u8); // filter: none
        for x in 0..px {
            let (mx, my) = ((x / scale) as i64 - 4, (y / scale) as i64 - 4);
            let dark = mx >= 0
                && my >= 0
                && (mx as u32) < w
                && (my as u32) < w
                && colors[(my as u32 * w + mx as u32) as usize] == qrcode::Color::Dark;
            raw.push(if dark { 0 } else { 255 });
        }
    }
    Some(png_gray8(px, px, &raw))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Minimal PNG encoder: 8-bit grayscale, zlib with stored (uncompressed) deflate blocks.
fn png_gray8(w: u32, h: u32, filtered_rows: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = filtered_rows.chunks(65535).collect();
    for (i, blk) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = blk.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(blk);
    }
    z.extend_from_slice(&adler32(filtered_rows).to_be_bytes());
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_vectors() {
        // RFC 6238 Appendix B secret "12345678901234567890", 8 digits truncated to 6.
        let secret = base32_encode(b"12345678901234567890");
        assert_eq!(code_at(&secret, 59).unwrap(), "287082");
        assert_eq!(code_at(&secret, 1111111109).unwrap(), "081804");
        assert_eq!(base32_decode(&secret).unwrap(), b"12345678901234567890");
    }

    #[test]
    fn qr_is_png() {
        let png = qr_png(&uri("ente", "a@b.c", &generate_secret()), 200).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
    }
}
