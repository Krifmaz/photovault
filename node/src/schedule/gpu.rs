//! GPU detection, utilisation and provider selection.
//!
//! - NVIDIA: `nvidia-smi` (run directly with an argument vector, never through
//!   a shell), falling back to sysfs for presence.
//! - AMD: sysfs `/sys/class/drm/card*/device/gpu_busy_percent`; `/dev/kfd`
//!   signals ROCm support; `rocm-smi --showproductname` for names when present.
//! - Intel: presence via sysfs; utilisation is a best-effort estimate from the
//!   current vs. maximum GT frequency (i915), otherwise unknown.
//!
//! On non-Linux systems only `nvidia-smi` is tried; everything else degrades to
//! "unknown" and the CPU provider.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

use crate::config::GpuProvider;

/// PCI vendor IDs.
const VENDOR_NVIDIA: u32 = 0x10de;
const VENDOR_AMD: u32 = 0x1002;
const VENDOR_INTEL: u32 = 0x8086;

/// How long a vendor tool may take before it is killed.
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

/// GPU vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    /// NVIDIA.
    Nvidia,
    /// AMD.
    Amd,
    /// Intel.
    Intel,
    /// Anything else.
    Other,
}

impl GpuVendor {
    fn from_pci(id: u32) -> Self {
        match id {
            VENDOR_NVIDIA => Self::Nvidia,
            VENDOR_AMD => Self::Amd,
            VENDOR_INTEL => Self::Intel,
            _ => Self::Other,
        }
    }
}

/// A detected GPU.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GpuInfo {
    /// Vendor.
    pub vendor: GpuVendor,
    /// Marketing name if known, else the PCI id.
    pub name: String,
    /// DRM card directory (`/sys/class/drm/cardN`) on Linux.
    pub sysfs: Option<PathBuf>,
    /// Total memory in MiB, if known.
    pub memory_mib: Option<u64>,
    /// Driver name or version, if known.
    pub driver: Option<String>,
}

/// GPUs and the inference providers they make available.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GpuReport {
    /// Detected GPUs.
    pub gpus: Vec<GpuInfo>,
    /// Usable providers, best first; always ends with `cpu`.
    pub providers: Vec<GpuProvider>,
}

impl GpuReport {
    /// The provider to use for `choice` (see [`resolve_provider`]).
    pub fn resolve(&self, choice: GpuProvider) -> GpuProvider {
        resolve_provider(choice, &self.providers)
    }
}

/// Detects GPUs and available providers.
pub async fn detect() -> GpuReport {
    let mut gpus = Vec::new();
    let nvidia = nvidia_smi_gpus().await;
    let have_nvidia_smi = !nvidia.is_empty();
    gpus.extend(nvidia);
    for g in sysfs_gpus(Path::new("/sys/class/drm")) {
        // nvidia-smi already gave richer data for NVIDIA cards.
        if g.vendor == GpuVendor::Nvidia && have_nvidia_smi {
            continue;
        }
        gpus.push(g);
    }
    if gpus.iter().any(|g| g.vendor == GpuVendor::Amd)
        && let Some(names) = rocm_smi_names().await
    {
        for (g, n) in gpus
            .iter_mut()
            .filter(|g| g.vendor == GpuVendor::Amd)
            .zip(names)
        {
            g.name = n;
        }
    }
    let providers = available_providers(
        &gpus,
        &ProviderHints {
            nvidia_driver: have_nvidia_smi || Path::new("/proc/driver/nvidia/version").exists(),
            rocm_kfd: Path::new("/dev/kfd").exists(),
            vulkan_icd: vulkan_icd_present(),
        },
    );
    GpuReport { gpus, providers }
}

/// Facts about the system that decide which providers can work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderHints {
    /// The NVIDIA kernel driver is loaded.
    pub nvidia_driver: bool,
    /// `/dev/kfd` exists (ROCm compute interface).
    pub rocm_kfd: bool,
    /// A Vulkan ICD is installed (or the OS has Metal/DX12).
    pub vulkan_icd: bool,
}

/// Providers usable on this machine, best first, always ending with `cpu`.
pub fn available_providers(gpus: &[GpuInfo], hints: &ProviderHints) -> Vec<GpuProvider> {
    let has = |v| gpus.iter().any(|g| g.vendor == v);
    let mut out = Vec::new();
    if has(GpuVendor::Nvidia) && hints.nvidia_driver {
        out.push(GpuProvider::Cuda);
    }
    if has(GpuVendor::Amd) && hints.rocm_kfd {
        out.push(GpuProvider::Rocm);
        out.push(GpuProvider::Migraphx);
    }
    if !gpus.is_empty() && hints.vulkan_icd {
        out.push(GpuProvider::Webgpu);
    }
    out.push(GpuProvider::Cpu);
    out
}

/// Picks the provider for `choice`: `auto` takes the best available; an
/// explicit choice is used if available, else `cpu`.
pub fn resolve_provider(choice: GpuProvider, available: &[GpuProvider]) -> GpuProvider {
    match choice {
        GpuProvider::Auto => GpuProvider::PREFERENCE
            .into_iter()
            .find(|p| available.contains(p))
            .unwrap_or(GpuProvider::Cpu),
        p if available.contains(&p) => p,
        _ => GpuProvider::Cpu,
    }
}

/// Highest utilisation across GPUs, 0–100, or `None` if none can be read.
pub async fn utilisation(report: &GpuReport) -> Option<f32> {
    let mut best: Option<f32> = None;
    let mut bump = |v: f32| best = Some(best.map_or(v, |b| b.max(v)));
    if report.gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia)
        && let Some(out) = run_tool(
            "nvidia-smi",
            &[
                "--query-gpu=utilization.gpu",
                "--format=csv,noheader,nounits",
            ],
        )
        .await
    {
        for v in parse_nvidia_utilisation(&out) {
            bump(v);
        }
    }
    for g in &report.gpus {
        let Some(dir) = &g.sysfs else { continue };
        let v = match g.vendor {
            GpuVendor::Amd => {
                read_trim(&dir.join("device/gpu_busy_percent")).and_then(|s| s.parse::<f32>().ok())
            }
            GpuVendor::Intel => intel_busy_estimate(dir),
            _ => None,
        };
        if let Some(v) = v {
            bump(v);
        }
    }
    best
}

/// Parses `nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits`.
pub fn parse_nvidia_utilisation(out: &str) -> Vec<f32> {
    out.lines()
        .filter_map(|l| l.trim().parse::<f32>().ok())
        .collect()
}

/// Parses `nvidia-smi --query-gpu=name,memory.total,driver_version
/// --format=csv,noheader,nounits`.
pub fn parse_nvidia_gpus(out: &str) -> Vec<GpuInfo> {
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.split(',').map(str::trim);
            let name = parts.next().unwrap_or("NVIDIA GPU").to_string();
            let memory_mib = parts.next().and_then(|m| m.parse().ok());
            let driver = parts.next().map(|d| format!("nvidia {d}"));
            GpuInfo {
                vendor: GpuVendor::Nvidia,
                name,
                sysfs: None,
                memory_mib,
                driver,
            }
        })
        .collect()
}

async fn nvidia_smi_gpus() -> Vec<GpuInfo> {
    match run_tool(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader,nounits",
        ],
    )
    .await
    {
        Some(out) => parse_nvidia_gpus(&out),
        None => Vec::new(),
    }
}

/// Product names from `rocm-smi --showproductname --csv`, in card order.
async fn rocm_smi_names() -> Option<Vec<String>> {
    let out = run_tool("rocm-smi", &["--showproductname", "--csv"]).await?;
    let names = parse_rocm_product_names(&out);
    (!names.is_empty()).then_some(names)
}

/// Parses `rocm-smi --showproductname --csv` output: a header row with a
/// "Card series" (or "Card SKU") column.
pub fn parse_rocm_product_names(out: &str) -> Vec<String> {
    let mut lines = out.lines().filter(|l| l.contains(','));
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let cols: Vec<String> = header.split(',').map(|c| c.trim().to_lowercase()).collect();
    let Some(idx) = cols
        .iter()
        .position(|c| c == "card series")
        .or_else(|| cols.iter().position(|c| c.contains("series")))
    else {
        return Vec::new();
    };
    lines
        .filter_map(|l| l.split(',').nth(idx).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Lists GPUs from a DRM sysfs directory (`/sys/class/drm`).
pub fn sysfs_gpus(drm: &Path) -> Vec<GpuInfo> {
    let Ok(rd) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut cards: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("card") && !n.contains('-'))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .filter_map(|card| {
            let vendor_s = read_trim(&card.join("device/vendor"))?;
            let vendor_id = u32::from_str_radix(vendor_s.trim_start_matches("0x"), 16).ok()?;
            let device = read_trim(&card.join("device/device")).unwrap_or_default();
            let vendor = GpuVendor::from_pci(vendor_id);
            let name = read_trim(&card.join("device/product_name"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("{vendor:?} GPU [{vendor_s}:{device}]"));
            let memory_mib = read_trim(&card.join("device/mem_info_vram_total"))
                .and_then(|s| s.parse::<u64>().ok())
                .map(|b| b / (1024 * 1024));
            let driver = std::fs::read_link(card.join("device/driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            Some(GpuInfo {
                vendor,
                name,
                sysfs: Some(card),
                memory_mib,
                driver,
            })
        })
        .collect()
}

/// Intel i915: current/maximum GT frequency as a rough busy estimate.
fn intel_busy_estimate(card: &Path) -> Option<f32> {
    let act: f32 = read_trim(&card.join("gt_act_freq_mhz"))?.parse().ok()?;
    let max: f32 = read_trim(&card.join("gt_max_freq_mhz"))?.parse().ok()?;
    (max > 0.0).then(|| (act / max * 100.0).clamp(0.0, 100.0))
}

fn vulkan_icd_present() -> bool {
    if !cfg!(target_os = "linux") {
        // Metal (macOS) and DX12 (Windows) are always there.
        return true;
    }
    [
        "/usr/share/vulkan/icd.d",
        "/etc/vulkan/icd.d",
        "/usr/local/share/vulkan/icd.d",
    ]
    .iter()
    .any(|d| {
        std::fs::read_dir(d)
            .map(|mut r| r.next().is_some())
            .unwrap_or(false)
    })
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Runs a vendor tool with fixed arguments (no shell) and a timeout;
/// returns stdout on success.
async fn run_tool(program: &str, args: &[&str]) -> Option<String> {
    let child = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(TOOL_TIMEOUT, child).await {
        Ok(Ok(out)) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(vendor: GpuVendor) -> GpuInfo {
        GpuInfo {
            vendor,
            name: "x".into(),
            sysfs: None,
            memory_mib: None,
            driver: None,
        }
    }

    #[test]
    fn nvidia_parsing() {
        assert_eq!(parse_nvidia_utilisation("37\n 5 \n"), vec![37.0, 5.0]);
        assert!(parse_nvidia_utilisation("[N/A]\n").is_empty());
        let g = parse_nvidia_gpus("NVIDIA GeForce RTX 4080, 16376, 560.35.03\n");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "NVIDIA GeForce RTX 4080");
        assert_eq!(g[0].memory_mib, Some(16376));
        assert_eq!(g[0].driver.as_deref(), Some("nvidia 560.35.03"));
    }

    #[test]
    fn rocm_parsing() {
        let out = "device,Card Series,Card Model,Card Vendor,Card SKU\ncard0,Radeon RX 7900 XTX,0x744c,Advanced Micro Devices,\n";
        assert_eq!(parse_rocm_product_names(out), vec!["Radeon RX 7900 XTX"]);
        assert!(parse_rocm_product_names("garbage").is_empty());
    }

    #[test]
    fn providers() {
        let hints = ProviderHints {
            nvidia_driver: true,
            rocm_kfd: true,
            vulkan_icd: true,
        };
        assert_eq!(available_providers(&[], &hints), vec![GpuProvider::Cpu]);
        let amd = available_providers(&[gpu(GpuVendor::Amd)], &hints);
        assert_eq!(
            amd,
            vec![
                GpuProvider::Rocm,
                GpuProvider::Migraphx,
                GpuProvider::Webgpu,
                GpuProvider::Cpu
            ]
        );
        let amd_no_rocm = available_providers(
            &[gpu(GpuVendor::Amd)],
            &ProviderHints {
                rocm_kfd: false,
                ..hints
            },
        );
        assert_eq!(amd_no_rocm, vec![GpuProvider::Webgpu, GpuProvider::Cpu]);
        assert_eq!(resolve_provider(GpuProvider::Auto, &amd), GpuProvider::Rocm);
        assert_eq!(resolve_provider(GpuProvider::Cuda, &amd), GpuProvider::Cpu);
        assert_eq!(
            resolve_provider(GpuProvider::Webgpu, &amd),
            GpuProvider::Webgpu
        );
        let nv = available_providers(&[gpu(GpuVendor::Nvidia)], &hints);
        assert_eq!(resolve_provider(GpuProvider::Auto, &nv), GpuProvider::Cuda);
    }

    #[test]
    fn sysfs_listing() {
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("card1");
        std::fs::create_dir_all(card.join("device")).unwrap();
        std::fs::write(card.join("device/vendor"), "0x1002\n").unwrap();
        std::fs::write(card.join("device/device"), "0x744c\n").unwrap();
        std::fs::write(card.join("device/mem_info_vram_total"), "25753026560\n").unwrap();
        // Connector directories are skipped.
        std::fs::create_dir_all(dir.path().join("card1-DP-1")).unwrap();
        let g = sysfs_gpus(dir.path());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].vendor, GpuVendor::Amd);
        assert_eq!(g[0].memory_mib, Some(24560));
        assert!(g[0].name.contains("0x744c"));
    }
}
