//! A token-bucket byte rate limiter, shared by every transfer of one job
//! (scrub reads, replication uploads), and a stream adapter that applies it.

use crate::storage::ByteStream;
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Limits throughput to `rate` bytes per second, with up to one second of burst.
#[derive(Debug)]
pub struct RateLimiter {
    rate: u64,
    state: Mutex<(f64, Instant)>,
}

impl RateLimiter {
    /// `bytes_per_sec == 0` means unlimited.
    pub fn new(bytes_per_sec: u64) -> Arc<Self> {
        Arc::new(Self {
            rate: bytes_per_sec,
            state: Mutex::new((bytes_per_sec as f64, Instant::now())),
        })
    }

    /// Configured rate (0 = unlimited).
    pub fn rate(&self) -> u64 {
        self.rate
    }

    /// Wait until `n` bytes may pass.
    pub async fn acquire(&self, n: usize) {
        if self.rate == 0 || n == 0 {
            return;
        }
        let rate = self.rate as f64;
        let wait = {
            let mut s = self.state.lock().await;
            let now = Instant::now();
            let (tokens, last) = *s;
            let tokens = (tokens + now.duration_since(last).as_secs_f64() * rate).min(rate);
            let left = tokens - n as f64;
            *s = (left, now);
            if left < 0.0 {
                Duration::from_secs_f64(-left / rate)
            } else {
                Duration::ZERO
            }
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Pass `body` through `limiter`.
pub fn throttle(body: ByteStream, limiter: Arc<RateLimiter>) -> ByteStream {
    if limiter.rate() == 0 {
        return body;
    }
    Box::pin(body.then(move |chunk| {
        let l = limiter.clone();
        async move {
            if let Ok(c) = &chunk {
                l.acquire(c.len()).await;
            }
            chunk
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn limits_rate() {
        let l = RateLimiter::new(1000);
        let start = tokio::time::Instant::now();
        for _ in 0..5 {
            l.acquire(1000).await;
        }
        // One second of burst, then 1000 B/s.
        let e = start.elapsed();
        assert!(e >= Duration::from_millis(3900), "{e:?}");
        let u = RateLimiter::new(0);
        u.acquire(1 << 30).await;
    }
}
