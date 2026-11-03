//! Daily time windows such as `"01:00-07:00"` (may wrap midnight, e.g.
//! `"22:00-06:00"`), in a fixed offset from UTC such as `"+01:00"`.
//!
//! There is no time-zone database in the image, so daylight saving time is
//! not followed: pick the offset you want the window to follow.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A daily window; `None` inside means "always".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    span: Option<(u32, u32)>,
    offset_secs: i64,
}

fn parse_hm(s: &str) -> anyhow::Result<u32> {
    let (h, m) = s
        .trim()
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("time {s:?} is not HH:MM"))?;
    let (h, m): (u32, u32) = (h.parse()?, m.parse()?);
    anyhow::ensure!(
        h <= 24 && m < 60 && h * 60 + m <= 24 * 60,
        "time {s:?} out of range"
    );
    Ok(h * 60 + m)
}

/// Parse a UTC offset like `+01:00`, `-05:30`, `Z` or empty (UTC), in seconds.
pub fn parse_offset(s: &str) -> anyhow::Result<i64> {
    let s = s.trim();
    if s.is_empty() || s == "Z" || s == "UTC" {
        return Ok(0);
    }
    let (sign, rest) = match s.as_bytes()[0] {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => anyhow::bail!("UTC offset {s:?} must start with + or -"),
    };
    let mins = parse_hm(rest)?;
    anyhow::ensure!(mins <= 14 * 60, "UTC offset {s:?} out of range");
    Ok(sign * i64::from(mins) * 60)
}

impl Window {
    /// A window that is always open.
    pub const ALWAYS: Window = Window {
        span: None,
        offset_secs: 0,
    };

    /// Parse `"HH:MM-HH:MM"` (empty = always) with a UTC offset.
    pub fn parse(window: &str, utc_offset: &str) -> anyhow::Result<Self> {
        let offset_secs = parse_offset(utc_offset)?;
        let w = window.trim();
        if w.is_empty() {
            return Ok(Window {
                span: None,
                offset_secs,
            });
        }
        let (a, b) = w
            .split_once('-')
            .ok_or_else(|| anyhow::anyhow!("window {w:?} is not HH:MM-HH:MM"))?;
        let (a, b) = (parse_hm(a)?, parse_hm(b)?);
        anyhow::ensure!(a != b, "window {w:?} is empty");
        Ok(Window {
            span: Some((a, b)),
            offset_secs,
        })
    }

    fn local_minute(&self, now: SystemTime) -> u32 {
        let secs = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + self.offset_secs;
        (secs.rem_euclid(86_400) / 60) as u32
    }

    /// Whether `now` is inside the window.
    pub fn contains(&self, now: SystemTime) -> bool {
        let Some((a, b)) = self.span else {
            return true;
        };
        let m = self.local_minute(now);
        if a < b {
            m >= a && m < b
        } else {
            m >= a || m < b
        }
    }

    /// How long until the window next opens (zero when open now).
    pub fn until_open(&self, now: SystemTime) -> Duration {
        let Some((a, _)) = self.span else {
            return Duration::ZERO;
        };
        if self.contains(now) {
            return Duration::ZERO;
        }
        let m = self.local_minute(now);
        let wait = (a + 24 * 60 - m) % (24 * 60);
        Duration::from_secs(u64::from(wait) * 60)
    }

    /// Sleep until the window is open.
    pub async fn wait_open(&self) {
        loop {
            let d = self.until_open(SystemTime::now());
            if d.is_zero() {
                return;
            }
            tokio::time::sleep(d.min(Duration::from_secs(300))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u64, m: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(86_400 * 1000 + h * 3600 + m * 60)
    }

    #[test]
    fn windows() {
        let w = Window::parse("01:00-07:00", "").unwrap();
        assert!(w.contains(at(1, 0)) && w.contains(at(6, 59)));
        assert!(!w.contains(at(7, 0)) && !w.contains(at(0, 59)));
        assert_eq!(w.until_open(at(23, 0)), Duration::from_secs(2 * 3600));
        let wrap = Window::parse("22:00-06:00", "").unwrap();
        assert!(wrap.contains(at(23, 0)) && wrap.contains(at(5, 0)) && !wrap.contains(at(12, 0)));
        let off = Window::parse("01:00-07:00", "+01:00").unwrap();
        assert!(off.contains(at(0, 30)) && !off.contains(at(6, 30)));
        assert!(Window::parse("", "+02:00").unwrap().contains(at(12, 0)));
        assert!(Window::parse("1-2", "").is_err());
        assert!(Window::parse("01:00-01:00", "").is_err());
        assert_eq!(parse_offset("-05:30").unwrap(), -(5 * 3600 + 1800));
        assert!(parse_offset("05:00").is_err());
    }
}
