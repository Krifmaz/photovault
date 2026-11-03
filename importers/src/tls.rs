//! TLS setup: rustls with the ring provider.

/// Install the process-wide rustls crypto provider. Safe to call many times.
pub fn install_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
