//! Real inference through Ente's models on ONNX Runtime (feature `ml`).
//!
//! These tests are `#[ignore]`d because they need things the repository does
//! not ship:
//!
//! - `ORT_DYLIB_PATH`: an ONNX Runtime >= 1.31 shared library, e.g.
//!   `libonnxruntime.so.1.31.1` from the official release tarball or the
//!   `onnxruntime` (CPU) / `onnxruntime-gpu` (CUDA) Python wheel;
//! - `PHOTOVAULT_ML_MODELS_DIR` (optional): a model cache directory; models
//!   are otherwise downloaded from `models.ente.com` into a temp dir;
//! - `PHOTOVAULT_ML_PROVIDER` (optional): `cpu` (default), `auto`, `cuda`,
//!   `rocm`, `migraphx` or `webgpu`.
//!
//! Run with:
//! `ORT_DYLIB_PATH=... cargo test -p photovault-node --features ml --test ml_inference -- --ignored`
#![cfg(feature = "ml")]

use std::io::Write;
use std::path::PathBuf;

use photovault_node::ml::indexer::{Indexer, IndexerConfig, load_onnxruntime};
use photovault_node::ml::mldata::{self, RemoteMlData};
use photovault_node::ml::{ProviderPreference, UsedProvider};

fn provider() -> ProviderPreference {
    std::env::var("PHOTOVAULT_ML_PROVIDER")
        .ok()
        .map_or(ProviderPreference::Cpu, |p| p.parse().unwrap())
}

/// A minimal RGB PNG encoder, so the test needs no image crate.
fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&raw).unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z.finish().unwrap());
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
#[ignore = "needs ORT_DYLIB_PATH"]
fn onnxruntime_loads_and_is_new_enough() {
    load_onnxruntime(None).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ORT_DYLIB_PATH and Ente's models"]
async fn real_inference_matches_ente_goldens_and_formats() {
    let tmp = tempfile::tempdir().unwrap();
    let models_dir = std::env::var_os("PHOTOVAULT_ML_MODELS_DIR")
        .map_or_else(|| tmp.path().join("models"), PathBuf::from);
    let indexer = Indexer::new(IndexerConfig {
        models_dir,
        provider: provider(),
        onnxruntime_path: None,
    })
    .await
    .unwrap();

    // Ente's golden outputs (vendor/ente/ml/src/onnx/golden_test/data.rs),
    // recomputed on CPU for each downloaded model.
    let paths = indexer.model_paths().clone();
    for model in [
        &paths.face_detection,
        &paths.face_embedding,
        &paths.clip_image,
    ] {
        let sep = ente_ml::golden_tooling::measure_zero_golden_separation(model).unwrap();
        assert!(
            sep.golden_distance <= sep.threshold && sep.zero_distance > sep.threshold,
            "{}: golden {} zero {} threshold {}",
            sep.model_file,
            sep.golden_distance,
            sep.zero_distance,
            sep.threshold
        );
    }

    // Full pipeline on a synthetic image (no faces), through the configured
    // provider; GPU sessions run Ente's golden self-test before use.
    let image = png(320, 240, |x, y| {
        [(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]
    });
    let out = tokio::task::spawn_blocking(move || indexer.index(42, &image, true, true))
        .await
        .unwrap()
        .unwrap();
    eprintln!("provider used: {:?}", out.provider);
    if provider() == ProviderPreference::Cpu {
        assert_eq!(out.provider, UsedProvider::Cpu);
    }
    let face = out.face.unwrap();
    let clip = out.clip.unwrap();
    assert_eq!((face.width, face.height), (320.0, 240.0));
    assert!(face.faces.is_empty(), "synthetic image has no faces");
    assert_eq!(clip.embedding.len(), 512);
    let norm: f64 = clip.embedding.iter().map(|v| v * v).sum::<f64>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-3,
        "CLIP embedding is normalised: {norm}"
    );
    assert_eq!(
        out.flags & mldata::flags::RUNTIME_RUST,
        mldata::flags::RUNTIME_RUST
    );

    // What would be uploaded reads back as complete and current.
    let raw = mldata::merge(None, Some(&face), Some(&clip)).unwrap();
    let back = RemoteMlData::from_json(&serde_json::to_vec(&raw).unwrap(), None).unwrap();
    assert!(back.plan().is_done());
}
