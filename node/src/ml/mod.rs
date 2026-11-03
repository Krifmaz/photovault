//! Face and CLIP indexing, producing Ente's `mldata` format.
//!
//! - [`mldata`] (always built): the wire format: serde types matching the
//!   web and mobile apps, version and flag constants, the skip rules
//!   ([`mldata::IndexPlan`]), merging that keeps unknown keys, gzip with the
//!   4 MiB limit, and encryption with the file key.
//! - `indexer` (cargo feature `ml`): runs Ente's own Rust ML crate
//!   (`ente-ml`, vendored under `node/vendor/ente`) on ONNX Runtime, loaded
//!   at run time from a library the user installs.
//!
//! A worker indexes one file like this:
//!
//! 1. fetch and [`mldata::open`] the existing `mldata`, if any;
//! 2. ask [`mldata::IndexPlan::for_remote`] what to run and skip the file
//!    when [`mldata::IndexPlan::is_done`];
//! 3. decode the original (for videos: the thumbnail, as both Ente apps do)
//!    and run the indexer with that plan;
//! 4. [`mldata::merge`] the results into the existing object and upload
//!    [`mldata::seal`]ed bytes with the fetched `updatedAt` as
//!    `lastUpdatedAt`.
//!
//! Based on Ente (ente.com).

pub mod mldata;

#[cfg(feature = "ml")]
pub mod indexer;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Which ONNX Runtime execution provider the indexer should use.
///
/// Every GPU choice falls back to CPU if the provider is missing from the
/// installed ONNX Runtime build, fails to create a session, or fails Ente's
/// golden self-test for a model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderPreference {
    /// Try CUDA, MIGraphX, ROCm and WebGPU in that order, then CPU.
    #[default]
    Auto,
    /// NVIDIA CUDA.
    Cuda,
    /// AMD ROCm (then MIGraphX).
    Rocm,
    /// AMD MIGraphX.
    Migraphx,
    /// WebGPU through Dawn: Vulkan on Linux, D3D12 on Windows.
    Webgpu,
    /// CPU only.
    Cpu,
}

impl ProviderPreference {
    /// All values, in the order shown to users.
    pub const ALL: [Self; 6] = [
        Self::Auto,
        Self::Cuda,
        Self::Rocm,
        Self::Migraphx,
        Self::Webgpu,
        Self::Cpu,
    ];

    /// The config-file spelling (`auto`, `cuda`, ...).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::Migraphx => "migraphx",
            Self::Webgpu => "webgpu",
            Self::Cpu => "cpu",
        }
    }
}

impl fmt::Display for ProviderPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error for an unknown [`ProviderPreference`] spelling.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown execution provider {0:?}; expected auto, cuda, rocm, migraphx, webgpu or cpu")]
pub struct UnknownProvider(pub String);

impl FromStr for ProviderPreference {
    type Err = UnknownProvider;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == lower)
            .ok_or_else(|| UnknownProvider(s.to_owned()))
    }
}

/// The execution provider that actually ran a file's models.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsedProvider {
    /// Everything ran on CPU (or nothing ran).
    #[default]
    Cpu,
    /// At least one model ran on CUDA.
    Cuda,
    /// At least one model ran on ROCm.
    Rocm,
    /// At least one model ran on MIGraphX.
    Migraphx,
    /// At least one model ran on WebGPU.
    Webgpu,
    /// At least one model ran on CoreML (macOS).
    Coreml,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_preference_round_trips() {
        for p in ProviderPreference::ALL {
            assert_eq!(p.as_str().parse::<ProviderPreference>(), Ok(p));
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                format!("\"{}\"", p.as_str())
            );
        }
        assert_eq!(" CUDA ".parse(), Ok(ProviderPreference::Cuda));
        assert!("vulkan".parse::<ProviderPreference>().is_err());
        assert_eq!(ProviderPreference::default(), ProviderPreference::Auto);
    }
}
