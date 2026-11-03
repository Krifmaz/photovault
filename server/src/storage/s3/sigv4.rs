//! AWS Signature Version 4, implemented with `hmac` + `sha2`.
//!
//! Covers what the S3 backend needs: header-signed requests (server-side
//! calls) and query-string presigning (URLs handed to the apps), including
//! extra signed headers such as `content-length` and `content-md5`.
//! Tested against AWS's published SigV4 test-suite and S3 examples.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// SHA-256 of the empty string (hex).
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// Payload hash used in presigned URLs.
pub const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";
const ALGO: &str = "AWS4-HMAC-SHA256";

/// Credentials and scope.
#[derive(Clone)]
pub struct Signer {
    /// Access key id.
    pub access_key: String,
    /// Secret access key.
    pub secret_key: String,
    /// Region (`us-east-1`, `garage`, …).
    pub region: String,
    /// Service (`s3`).
    pub service: String,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("access_key", &self.access_key)
            .field("region", &self.region)
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

/// A request to sign. `path` must already be URI-encoded exactly as it is
/// sent (see [`uri_encode`]); `query` and `headers` hold raw values.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// Encoded absolute path, e.g. `/bucket/1/abc`.
    pub path: &'a str,
    /// Query parameters (raw, unencoded).
    pub query: Vec<(String, String)>,
    /// Headers to sign (any case; must include `host`).
    pub headers: Vec<(String, String)>,
}

/// RFC 3986 encoding as SigV4 wants it: everything but `A-Za-z0-9-_.~` is
/// `%XX`-escaped; `/` is kept when `keep_slash`.
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Encoded, sorted query string (canonical form; also valid on the wire).
pub fn canonical_query(query: &[(String, String)]) -> String {
    let mut q: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    q.sort();
    q.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn canonical_headers(headers: &[(String, String)]) -> (String, String) {
    let mut h: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.to_ascii_lowercase(),
                v.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    h.sort_by(|a, b| a.0.cmp(&b.0));
    // Merge repeated names (comma-joined) as SigV4 requires.
    let mut merged: Vec<(String, String)> = Vec::with_capacity(h.len());
    for (k, v) in h {
        match merged.last_mut() {
            Some((lk, lv)) if *lk == k => {
                lv.push(',');
                lv.push_str(&v);
            }
            _ => merged.push((k, v)),
        }
    }
    let canon = merged.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed = merged
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    (canon, signed)
}

fn hmac(key: &[u8], msg: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    m.update(msg.as_bytes());
    m.finalize().into_bytes().to_vec()
}

/// Hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// `(YYYYMMDD, YYYYMMDDTHHMMSSZ)` for unix seconds `t` (UTC).
pub fn amz_dates(t: u64) -> (String, String) {
    let days = (t / 86_400) as i64;
    let secs = t % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let date = format!("{y:04}{m:02}{d:02}");
    let stamp = format!(
        "{date}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    );
    (date, stamp)
}

impl Signer {
    fn scope(&self, date: &str) -> String {
        format!("{date}/{}/{}/aws4_request", self.region, self.service)
    }

    fn signing_key(&self, date: &str) -> Vec<u8> {
        let k = hmac(format!("AWS4{}", self.secret_key).as_bytes(), date);
        let k = hmac(&k, &self.region);
        let k = hmac(&k, &self.service);
        hmac(&k, "aws4_request")
    }

    fn signature(&self, req: &Request<'_>, payload_hash: &str, stamp: &str) -> (String, String) {
        let date = &stamp[..8];
        let (canon_headers, signed_headers) = canonical_headers(&req.headers);
        let canonical = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            req.method,
            req.path,
            canonical_query(&req.query),
            canon_headers,
            signed_headers,
            payload_hash
        );
        let to_sign = format!(
            "{ALGO}\n{stamp}\n{}\n{}",
            self.scope(date),
            sha256_hex(canonical.as_bytes())
        );
        let sig = hex::encode(hmac(&self.signing_key(date), &to_sign));
        (sig, signed_headers)
    }

    /// Sign with an `Authorization` header. `req.headers` must already hold
    /// `host`, `x-amz-date` (= `stamp`) and, for S3, `x-amz-content-sha256`.
    /// Returns the `Authorization` header value.
    pub fn authorization(&self, req: &Request<'_>, payload_hash: &str, stamp: &str) -> String {
        let (sig, signed) = self.signature(req, payload_hash, stamp);
        format!(
            "{ALGO} Credential={}/{}, SignedHeaders={signed}, Signature={sig}",
            self.access_key,
            self.scope(&stamp[..8])
        )
    }

    /// Presign: returns the full query string (canonical order, signature last)
    /// to append after `?`. `req.query` holds the operation's own parameters;
    /// `req.headers` the headers the client must send (at least `host`).
    pub fn presign_query(&self, req: &Request<'_>, now: u64, expires_secs: u64) -> String {
        let (date, stamp) = amz_dates(now);
        let (_, signed_headers) = canonical_headers(&req.headers);
        let mut full = req.clone();
        full.query.extend([
            ("X-Amz-Algorithm".to_string(), ALGO.to_string()),
            (
                "X-Amz-Credential".to_string(),
                format!("{}/{}", self.access_key, self.scope(&date)),
            ),
            ("X-Amz-Date".to_string(), stamp.clone()),
            ("X-Amz-Expires".to_string(), expires_secs.to_string()),
            ("X-Amz-SignedHeaders".to_string(), signed_headers),
        ]);
        let (sig, _) = self.signature(&full, UNSIGNED_PAYLOAD, &stamp);
        format!("{}&X-Amz-Signature={sig}", canonical_query(&full.query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    /// 2013-05-24T00:00:00Z, the date of AWS's S3 SigV4 examples.
    const T_2013: u64 = 1_369_353_600;
    /// 2015-08-30T12:36:00Z, the date of the SigV4 test suite.
    const T_2015: u64 = 1_440_938_160;

    fn s3_example_signer() -> Signer {
        Signer {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            service: "s3".into(),
        }
    }

    fn suite_signer() -> Signer {
        Signer {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            service: "service".into(),
        }
    }

    #[test]
    fn dates() {
        assert_eq!(
            amz_dates(T_2013),
            ("20130524".into(), "20130524T000000Z".into())
        );
        assert_eq!(amz_dates(T_2015).1, "20150830T123600Z");
        assert_eq!(amz_dates(951_782_400).1, "20000229T000000Z");
    }

    fn sig_of(auth: &str) -> &str {
        auth.rsplit("Signature=").next().unwrap_or("")
    }

    /// aws-sig-v4-test-suite: get-vanilla.
    #[test]
    fn suite_get_vanilla() {
        let req = Request {
            method: "GET",
            path: "/",
            query: vec![],
            headers: vec![
                s("Host", "example.amazonaws.com"),
                s("X-Amz-Date", "20150830T123600Z"),
            ],
        };
        let auth = suite_signer().authorization(&req, EMPTY_SHA256, "20150830T123600Z");
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    /// aws-sig-v4-test-suite: get-vanilla-query-order-key-case.
    #[test]
    fn suite_query_order() {
        let req = Request {
            method: "GET",
            path: "/",
            query: vec![s("Param2", "value2"), s("Param1", "value1")],
            headers: vec![
                s("Host", "example.amazonaws.com"),
                s("X-Amz-Date", "20150830T123600Z"),
            ],
        };
        let auth = suite_signer().authorization(&req, EMPTY_SHA256, "20150830T123600Z");
        assert_eq!(
            sig_of(&auth),
            "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    /// S3 docs, "Example: GET Object" (header auth with Range).
    #[test]
    fn s3_get_object_range() {
        let req = Request {
            method: "GET",
            path: "/test.txt",
            query: vec![],
            headers: vec![
                s("Host", "examplebucket.s3.amazonaws.com"),
                s("Range", "bytes=0-9"),
                s("x-amz-content-sha256", EMPTY_SHA256),
                s("x-amz-date", "20130524T000000Z"),
            ],
        };
        let auth = s3_example_signer().authorization(&req, EMPTY_SHA256, "20130524T000000Z");
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    /// S3 docs, "Example: PUT Object" (`$` in the key, extra amz header).
    #[test]
    fn s3_put_object() {
        let body = b"Welcome to Amazon S3.";
        let hash = sha256_hex(body);
        assert_eq!(
            hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let path = format!("/{}", uri_encode("test$file.text", true));
        let req = Request {
            method: "PUT",
            path: &path,
            query: vec![],
            headers: vec![
                s("Host", "examplebucket.s3.amazonaws.com"),
                s("Date", "Fri, 24 May 2013 00:00:00 GMT"),
                s("x-amz-date", "20130524T000000Z"),
                s("x-amz-storage-class", "REDUCED_REDUNDANCY"),
                s("x-amz-content-sha256", &hash),
            ],
        };
        let auth = s3_example_signer().authorization(&req, &hash, "20130524T000000Z");
        assert_eq!(
            sig_of(&auth),
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    /// S3 docs, "Example: GET Bucket Lifecycle" (valueless sub-resource).
    #[test]
    fn s3_get_lifecycle() {
        let req = Request {
            method: "GET",
            path: "/",
            query: vec![s("lifecycle", "")],
            headers: vec![
                s("Host", "examplebucket.s3.amazonaws.com"),
                s("x-amz-date", "20130524T000000Z"),
                s("x-amz-content-sha256", EMPTY_SHA256),
            ],
        };
        let auth = s3_example_signer().authorization(&req, EMPTY_SHA256, "20130524T000000Z");
        assert_eq!(
            sig_of(&auth),
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
    }

    /// S3 docs, "Example: Get Bucket (List Objects)".
    #[test]
    fn s3_list_objects() {
        let req = Request {
            method: "GET",
            path: "/",
            query: vec![s("max-keys", "2"), s("prefix", "J")],
            headers: vec![
                s("Host", "examplebucket.s3.amazonaws.com"),
                s("x-amz-date", "20130524T000000Z"),
                s("x-amz-content-sha256", EMPTY_SHA256),
            ],
        };
        let auth = s3_example_signer().authorization(&req, EMPTY_SHA256, "20130524T000000Z");
        assert_eq!(
            sig_of(&auth),
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    /// S3 docs, "Authenticating Requests: Using Query Parameters" presigned GET.
    #[test]
    fn s3_presigned_get() {
        let req = Request {
            method: "GET",
            path: "/test.txt",
            query: vec![],
            headers: vec![s("host", "examplebucket.s3.amazonaws.com")],
        };
        let q = s3_example_signer().presign_query(&req, T_2013, 86_400);
        assert_eq!(
            q,
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    #[test]
    fn encoding() {
        assert_eq!(uri_encode("a b/c+d~e", true), "a%20b/c%2Bd~e");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("é", false), "%C3%A9");
        assert_eq!(
            canonical_query(&[s("b", "2"), s("a", "x y"), s("uploads", "")]),
            "a=x%20y&b=2&uploads="
        );
    }

    #[test]
    fn debug_hides_secret() {
        assert!(!format!("{:?}", s3_example_signer()).contains("EXAMPLEKEY"));
    }
}
