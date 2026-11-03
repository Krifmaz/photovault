//! Prometheus metrics exposed at `/metrics`.
//!
//! Labels never carry user data: HTTP metrics use the matched route template
//! (`/files/download/{file_id}`), storage metrics the backend and operation name.

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::{LazyLock, Mutex};

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

/// Labels for object-store operations.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct StorageLabels {
    /// Backend (`local`, `s3`).
    pub backend: &'static str,
    /// Operation (`presign_put`, `head`, `copy`, …).
    pub op: &'static str,
    /// `ok`, `not_found` or `error`.
    pub result: &'static str,
}

/// Labels for local-store signed-URL traffic (bytes in/out).
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct DirectionLabels {
    /// `upload` or `download`.
    pub direction: &'static str,
}

/// Storage metric families. Process-wide (the store is built before
/// [`Metrics`]), and registered into every [`Metrics`] registry.
#[derive(Debug, Clone)]
pub struct StorageMetrics {
    /// Operations performed against the object store.
    pub ops: Family<StorageLabels, Counter>,
    /// Operation latency (seconds).
    pub latency: Family<StorageLabels, Histogram>,
    /// Bytes moved through the local backend's `/storage/*` URLs.
    pub bytes: Family<DirectionLabels, Counter>,
}

/// Process-wide storage metrics.
pub static STORAGE: LazyLock<StorageMetrics> = LazyLock::new(|| StorageMetrics {
    ops: Family::default(),
    latency: Family::new_with_constructor(|| Histogram::new(exponential_buckets(0.0005, 2.0, 16))),
    bytes: Family::default(),
});

/// All metrics.
#[derive(Debug)]
pub struct Metrics {
    registry: Mutex<Registry>,
    /// Requests served.
    pub http_requests: Family<HttpLabels, Counter>,
    /// Request latency (seconds).
    pub http_latency: Family<HttpLabels, Histogram>,
    /// Database pool: open connections (sampled at scrape time).
    pub db_pool_connections: Gauge,
    /// Database pool: idle connections (sampled at scrape time).
    pub db_pool_idle: Gauge,
    /// Database pool: configured maximum.
    pub db_pool_max: Gauge,
    /// `/healthz` database round trip (seconds), sampled at scrape time.
    pub db_ping_seconds: Histogram,
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
        let db_pool_connections = Gauge::default();
        let db_pool_idle = Gauge::default();
        let db_pool_max = Gauge::default();
        let db_ping_seconds = Histogram::new(exponential_buckets(0.0001, 2.0, 16));
        registry.register(
            "db_pool_connections",
            "Open database connections",
            db_pool_connections.clone(),
        );
        registry.register(
            "db_pool_idle_connections",
            "Idle database connections",
            db_pool_idle.clone(),
        );
        registry.register(
            "db_pool_max_connections",
            "Maximum database connections",
            db_pool_max.clone(),
        );
        registry.register(
            "db_ping_duration_seconds",
            "Database round trip measured at scrape time",
            db_ping_seconds.clone(),
        );
        let st = &*STORAGE;
        registry.register(
            "storage_operations",
            "Object store operations",
            st.ops.clone(),
        );
        registry.register(
            "storage_operation_duration_seconds",
            "Object store operation latency",
            st.latency.clone(),
        );
        registry.register(
            "storage_transfer_bytes",
            "Bytes transferred through local-store signed URLs",
            st.bytes.clone(),
        );
        Self {
            registry: Mutex::new(registry),
            http_requests,
            http_latency,
            db_pool_connections,
            db_pool_idle,
            db_pool_max,
            db_ping_seconds,
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
