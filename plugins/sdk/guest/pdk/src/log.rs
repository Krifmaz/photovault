//! Logging to the host's plugin log.
//!
//! Use the macros, which take `format!` arguments:
//!
//! ```
//! # use photovault_plugin_pdk as pdk;
//! pdk::info!("added {} files to {}", 3, "Holidays");
//! pdk::warn!("rule {} never matches", 2);
//! ```
//!
//! Messages go to Extism's log in Wasm (the host decides the level and where
//! they end up) and to `testing::MockHost::logs` in native tests.
//! Never log photo contents; metadata such as file names is fine because the
//! log stays on the user's device.

/// Severity of a log message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Very detailed tracing.
    Trace,
    /// Debugging detail.
    Debug,
    /// Normal operation.
    Info,
    /// Something looks wrong but the hook carries on.
    Warn,
    /// Something failed.
    Error,
}

impl Level {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn to_extism(self) -> extism_pdk::LogLevel {
        match self {
            Level::Trace => extism_pdk::LogLevel::Trace,
            Level::Debug => extism_pdk::LogLevel::Debug,
            Level::Info => extism_pdk::LogLevel::Info,
            Level::Warn => extism_pdk::LogLevel::Warn,
            Level::Error => extism_pdk::LogLevel::Error,
        }
    }
}

/// Write one message at `level`. The macros call this.
pub fn log(level: Level, message: &str) {
    crate::backend::log(level, message);
}

/// Log at an explicit [`Level`]: `log!(Level::Info, "x = {}", x)`.
#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)+) => {
        $crate::log::log($level, &::std::format!($($arg)+))
    };
}

/// Log at [`Level::Trace`].
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Trace, $($arg)+) };
}

/// Log at [`Level::Debug`].
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Debug, $($arg)+) };
}

/// Log at [`Level::Info`].
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Info, $($arg)+) };
}

/// Log at [`Level::Warn`].
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Warn, $($arg)+) };
}

/// Log at [`Level::Error`].
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Error, $($arg)+) };
}
