//! Face and CLIP inference with Ente's own Rust ML crate (feature `ml`).
//!
//! This wraps the vendored `ente-ml` exactly as Ente's desktop app uses it
//! (`rust/bindings/napi/photos`): the same models from `models.ente.com`, the
//! same preprocessing, and the same golden self-tests for GPU sessions, so
//! the indexes it returns are the ones the apps would compute themselves.
//!
//! ONNX Runtime is loaded at run time (`ort`'s `load-dynamic`), from
//! [`IndexerConfig::onnxruntime_path`], else `ORT_DYLIB_PATH`, else the
//! platform's default library name. Nothing is downloaded or linked at build
//! time.
//!
//! Based on Ente (ente.com).

use std::path::{Path, PathBuf};

use ente_ml::indexing::{self, AnalyzeImageRequest, DesktopExecutionProvider, ImageSource};
use ente_ml::{ModelPaths, error::MlError};

use super::mldata::{Face, RemoteClipIndex, RemoteFaceIndex, flags};
use super::{ProviderPreference, UsedProvider};

/// Errors from the indexer.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// The ONNX Runtime library could not be loaded or is too old.
    #[error("could not load ONNX Runtime from {path}: {message}")]
    OnnxRuntime {
        /// The library path that was tried.
        path: PathBuf,
        /// Why loading failed.
        message: String,
    },
    /// Downloading or verifying the models failed.
    #[error("could not prepare ML models: {0}")]
    Models(String),
    /// Ente's ML pipeline failed on this file.
    #[error(transparent)]
    Ml(#[from] MlError),
}

impl IndexerError {
    /// `true` when the failure is inherent to the file (it cannot be decoded
    /// or processed), so retrying is pointless. The web app marks such files
    /// as failed (`ml_decode` / `ml_image`); everything else is retried.
    #[must_use]
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Ml(MlError::Decode(_) | MlError::Image(_)))
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, IndexerError>;

/// Indexer settings.
#[derive(Clone, Debug)]
pub struct IndexerConfig {
    /// Where models are downloaded and cached (an Ente asset store). Model
    /// files placed in `<models_dir>/import` under the mobile app's cache
    /// names are imported instead of downloaded.
    pub models_dir: PathBuf,
    /// Execution provider preference.
    pub provider: ProviderPreference,
    /// Path of the ONNX Runtime shared library (`libonnxruntime.so`,
    /// `onnxruntime.dll`, `libonnxruntime.dylib`). `None` means
    /// `ORT_DYLIB_PATH`, then the default library name.
    pub onnxruntime_path: Option<PathBuf>,
}

/// One file's results, already in Ente's `mldata` shapes.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexOutput {
    /// New face index, when faces were requested.
    pub face: Option<RemoteFaceIndex>,
    /// New CLIP index, when CLIP was requested.
    pub clip: Option<RemoteClipIndex>,
    /// The fastest provider any model ran on for this file.
    pub provider: UsedProvider,
    /// The `flags` value written into the new indexes.
    pub flags: i64,
}

/// Runs Ente's face and CLIP models.
///
/// Sessions are global inside `ente-ml` and serialised per model, so one
/// `Indexer` per process is enough; [`Indexer::index`] blocks (call it from
/// `tokio::task::spawn_blocking`).
#[derive(Debug)]
pub struct Indexer {
    model_paths: ModelPaths,
    provider: ProviderPreference,
}

impl Indexer {
    /// Loads ONNX Runtime, applies the provider preference and makes sure
    /// the face and CLIP image models are downloaded and verified.
    pub async fn new(cfg: IndexerConfig) -> Result<Self> {
        load_onnxruntime(cfg.onnxruntime_path.as_deref())?;

        indexing::set_desktop_execution_provider(desktop_provider(cfg.provider));
        indexing::set_ml_execution_config(matches!(
            cfg.provider,
            ProviderPreference::Auto | ProviderPreference::Webgpu
        ));

        std::fs::create_dir_all(&cfg.models_dir)
            .map_err(|e| IndexerError::Models(format!("{}: {e}", cfg.models_dir.display())))?;
        let store = ente_assets::AssetStore::new(cfg.models_dir.clone());
        let import_dir = cfg.models_dir.join("import");
        let model_paths =
            ente_ml::assets::ensure_indexing_models(&store, &import_dir, true, true, false)
                .await
                .map_err(|e| IndexerError::Models(e.to_string()))?;
        indexing::init_ml_runtime(model_paths.clone());
        Ok(Self {
            model_paths,
            provider: cfg.provider,
        })
    }

    /// Paths of the downloaded, checksum-verified model files.
    #[must_use]
    pub fn model_paths(&self) -> &ModelPaths {
        &self.model_paths
    }

    /// The configured provider preference.
    #[must_use]
    pub fn provider_preference(&self) -> ProviderPreference {
        self.provider
    }

    /// Indexes one decoded-able image (JPEG, PNG, HEIC, AVIF, JXL, TIFF,
    /// WebP, ...). Pass the original for images and live photos' image
    /// part, and the thumbnail for videos, as the Ente apps do.
    ///
    /// `file_id` is the Ente file ID; it prefixes every face ID.
    pub fn index(
        &self,
        file_id: i64,
        bytes: &[u8],
        run_faces: bool,
        run_clip: bool,
    ) -> Result<IndexOutput> {
        let result = indexing::analyze_image(AnalyzeImageRequest {
            file_id,
            source: ImageSource::Bytes(bytes.to_vec()),
            run_faces,
            run_clip,
            run_pets: false,
            generate_face_crops: false,
            model_paths: self.model_paths.clone(),
        })?;

        let flags = flags::for_rust_result(result.used_coreml, result.used_webgpu);
        let provider = if result.used_cuda {
            UsedProvider::Cuda
        } else if result.used_migraphx {
            UsedProvider::Migraphx
        } else if result.used_rocm {
            UsedProvider::Rocm
        } else if result.used_webgpu {
            UsedProvider::Webgpu
        } else if result.used_coreml {
            UsedProvider::Coreml
        } else {
            UsedProvider::Cpu
        };

        let dims = &result.decoded_image_size;
        let face = result.faces.map(|faces| {
            let faces = faces
                .into_iter()
                .map(|f| {
                    Face::from_rust_result(
                        f.face_id,
                        f.detection.box_xyxy,
                        &f.detection.keypoints,
                        f.detection.score,
                        f.blur_value,
                        &f.embedding,
                    )
                })
                .collect();
            RemoteFaceIndex::new(dims.width, dims.height, faces, flags)
        });
        let clip = result
            .clip
            .map(|c| RemoteClipIndex::new(c.embedding.into_iter().map(f64::from).collect(), flags));
        Ok(IndexOutput {
            face,
            clip,
            provider,
            flags,
        })
    }

    /// Frees model sessions (and GPU memory) until the next
    /// [`Indexer::index`], e.g. while the machine is busy.
    pub fn release_sessions(&self) {
        indexing::release_ml_runtime();
    }
}

fn desktop_provider(p: ProviderPreference) -> DesktopExecutionProvider {
    match p {
        ProviderPreference::Auto => DesktopExecutionProvider::Auto,
        ProviderPreference::Cuda => DesktopExecutionProvider::Cuda,
        ProviderPreference::Rocm => DesktopExecutionProvider::Rocm,
        ProviderPreference::Migraphx => DesktopExecutionProvider::MiGraphX,
        ProviderPreference::Webgpu => DesktopExecutionProvider::WebGpu,
        ProviderPreference::Cpu => DesktopExecutionProvider::Cpu,
    }
}

/// Loads and version-checks the ONNX Runtime library (once per process), so
/// a missing or outdated library is an error here instead of a panic inside
/// `ort` later. `None` means `ORT_DYLIB_PATH`, then the default library
/// name. [`Indexer::new`] calls this; it is public for diagnostics.
pub fn load_onnxruntime(explicit: Option<&Path>) -> Result<()> {
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        std::env::var_os("ORT_DYLIB_PATH")
            .filter(|v| !v.is_empty())
            .map_or_else(
                || {
                    PathBuf::from(format!(
                        "{}onnxruntime{}",
                        std::env::consts::DLL_PREFIX,
                        std::env::consts::DLL_SUFFIX
                    ))
                },
                PathBuf::from,
            )
    });
    // The builder is dropped unused: `ente-ml` builds its own environment;
    // this call only loads and version-checks the library once per process.
    ort::init_from(&path)
        .map(drop)
        .map_err(|e| IndexerError::OnnxRuntime {
            path,
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_errors_are_permanent() {
        assert!(IndexerError::Ml(MlError::Decode("x".into())).is_permanent());
        assert!(IndexerError::Ml(MlError::Image("x".into())).is_permanent());
        assert!(!IndexerError::Ml(MlError::Ort("x".into())).is_permanent());
        assert!(!IndexerError::Models("x".into()).is_permanent());
    }

    #[test]
    fn missing_onnxruntime_is_an_error_not_a_panic() {
        let err = load_onnxruntime(Some(Path::new("/nonexistent/libonnxruntime.so"))).unwrap_err();
        assert!(matches!(err, IndexerError::OnnxRuntime { .. }), "{err}");
    }

    #[test]
    fn every_preference_maps_to_a_desktop_provider() {
        for p in ProviderPreference::ALL {
            let d = desktop_provider(p);
            assert_ne!(d, DesktopExecutionProvider::PlatformDefault, "{p}");
        }
    }
}
