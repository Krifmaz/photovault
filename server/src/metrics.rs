//! Prometheus metrics exposed at `/metrics`.

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::Mutex;

/// Labels for HTTP metrics. `route` is the matched route template, never the raw path.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct HttpLabels {
    /// HTTP method.
    pub method: String,
    /// Matched route template, e.g. `/files/download/{file_id}`.
    pub route: String,
    /// Status code.
    pub status: u16,
}

/// All metrics.
#[derive(Debug)]
pub struct Metrics {
    registry: Mutex<Registry>,
    /// Requests served.
    pub http_requests: Family<HttpLabels, Counter>,
    /// Request latency (seconds).
    pub http_latency: Family<HttpLabels, Histogram>,
}

impl Default for Metrics {
    fn default() -> Self {
        let mut registry = Registry::with_prefix("photovault");
        let http_requests = Family::<HttpLabels, Counter>::default();
        let http_latency = Family::<HttpLabels, Histogram>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.001, 2.0, 16))
        });
        registry.register(
            "http_requests",
            "HTTP requests served",
            http_requests.clone(),
        );
        registry.register(
            "http_request_duration_seconds",
            "HTTP request latency",
            http_latency.clone(),
        );
        Self {
            registry: Mutex::new(registry),
            http_requests,
            http_latency,
        }
    }
}

impl Metrics {
    /// Render in OpenMetrics text format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let reg = self.registry.lock().expect("metrics registry poisoned");
        let _ = prometheus_client::encoding::text::encode(&mut out, &reg);
        out
    }
}
