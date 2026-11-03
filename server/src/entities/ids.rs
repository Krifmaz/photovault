//! Identifiers in museum's formats: random v4 UUIDs (stored as lowercase,
//! hyphenated TEXT) and prefixed nano-ids (`<prefix>_<22 chars>`, museum's `base.NewID`).

const NANO_ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// A random RFC 4122 version-4 UUID, lowercase and hyphenated.
pub fn new_uuid() -> String {
    let mut b = crate::crypto::random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format_uuid(&b)
}

fn format_uuid(b: &[u8]) -> String {
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Parse a UUID the way Go's `uuid.Parse` accepts it (hyphenated, plain 32 hex,
/// `urn:uuid:` prefix or `{…}` braces), returning the canonical lowercase form.
pub fn parse_uuid(s: &str) -> Option<String> {
    let mut t = s;
    if t.len() == 36 + 9 && t[..9].eq_ignore_ascii_case("urn:uuid:") {
        t = &t[9..];
    } else if t.len() == 38 && t.starts_with('{') && t.ends_with('}') {
        t = &t[1..37];
    }
    let hexpart: String = match t.len() {
        36 => {
            let bytes = t.as_bytes();
            if [8, 13, 18, 23].iter().any(|&i| bytes[i] != b'-') {
                return None;
            }
            t.chars().filter(|c| *c != '-').collect()
        }
        32 => t.to_string(),
        _ => return None,
    };
    let raw = hex::decode(&hexpart).ok()?;
    if raw.len() != 16 {
        return None;
    }
    Some(format_uuid(&raw))
}

/// museum's `base.NewID(prefix)`: `prefix` must be at least two lowercase ASCII letters.
pub fn new_nano_id(prefix: &str) -> Option<String> {
    if prefix.len() < 2 || !prefix.bytes().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let mut out = String::with_capacity(prefix.len() + 23);
    out.push_str(prefix);
    out.push('_');
    // Rejection sampling keeps the distribution uniform over the 62-char alphabet.
    while out.len() < prefix.len() + 23 {
        for byte in crate::crypto::random_bytes(32) {
            if byte < 248 && out.len() < prefix.len() + 23 {
                out.push(NANO_ALPHABET[(byte % 62) as usize] as char);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_roundtrip() {
        let u = new_uuid();
        assert_eq!(parse_uuid(&u).as_deref(), Some(u.as_str()));
        assert_eq!(
            parse_uuid("6BA7B810-9DAD-11D1-80B4-00C04FD430C8").as_deref(),
            Some("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
        );
        assert!(parse_uuid("not-a-uuid").is_none());
        assert!(parse_uuid("6ba7b8109dad11d180b400c04fd430c8").is_some());
    }

    #[test]
    fn nano_ids() {
        let id = new_nano_id("ct").unwrap();
        assert!(id.starts_with("ct_") && id.len() == 25);
        assert!(new_nano_id("smart_album").is_none());
        assert!(new_nano_id("x").is_none());
    }
}
