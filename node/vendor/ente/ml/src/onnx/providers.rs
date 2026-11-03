use ort::{
    environment::Environment,
    ep::{CPU, ExecutionProviderDispatch},
    session::{
        Session,
        builder::{AutoDevicePolicy, GraphOptimizationLevel},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

#[cfg(target_os = "android")]
use ort::ep::XNNPACK;
#[cfg(any(target_os = "ios", target_os = "macos"))]
use ort::ep::{
    CoreML,
    coreml::{ComputeUnits, ModelFormat, SpecializationStrategy},
};
#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
use ort::ep::{
    WebGPU,
    webgpu::{DawnBackendType, PreferredLayout},
};
#[cfg(target_os = "android")]
use std::num::NonZeroUsize;
// PHOTOVAULT PATCH: desktop GPU execution providers (CUDA, ROCm, MIGraphX).
#[cfg(any(target_os = "linux", target_os = "windows"))]
use ort::ep::{CUDA, MIGraphX, ROCm};
// PHOTOVAULT PATCH: process-wide desktop execution provider preference.
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(any(target_os = "ios", target_os = "macos"))]
use super::coreml_cache;
#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
use super::golden_test;
#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
use super::model_file_label;
#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
use super::webgpu;
use super::{AccelerationValidation, GpuOptions, session_load_error};
use crate::error::MlResult;

#[cfg(any(target_os = "ios", target_os = "macos"))]
const ENABLE_PERSISTENT_COREML_CACHE: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionMode {
    PlatformDefault,
    GpuPreferred,
    CpuOnly,
}

// Identifies the successful attempt's preferred provider, not its registered
// fallback providers, for result attribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExecutionProvider {
    CoreMl,
    WebGpu,
    Xnnpack,
    Cpu,
    // PHOTOVAULT PATCH: desktop GPU execution providers.
    Cuda,
    Rocm,
    MiGraphX,
}

// PHOTOVAULT PATCH: desktop execution provider preference.
/// Execution provider preference for Linux and Windows desktops.
///
/// `PlatformDefault` keeps Ente's upstream behaviour (WebGPU when enabled via
/// `set_ml_execution_config`, then CPU). Every other value is a Photovault
/// addition for the compute node. Each GPU choice falls back to CPU when the
/// provider is missing from the loaded ONNX Runtime build, cannot create a
/// session, or fails the golden self-test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum DesktopExecutionProvider {
    /// Ente's upstream provider order.
    #[default]
    PlatformDefault = 0,
    /// Try CUDA, MIGraphX, ROCm and WebGPU in that order, then CPU.
    Auto = 1,
    /// NVIDIA CUDA (needs an ONNX Runtime GPU build), then CPU.
    Cuda = 2,
    /// AMD ROCm (needs an ONNX Runtime build with the ROCm EP), then MIGraphX, then CPU.
    Rocm = 3,
    /// AMD MIGraphX (needs an ONNX Runtime build with the MIGraphX EP), then CPU.
    MiGraphX = 4,
    /// WebGPU via Dawn (Vulkan on Linux, D3D12 on Windows), then CPU.
    WebGpu = 5,
    /// CPU only.
    Cpu = 6,
}

// PHOTOVAULT PATCH: desktop execution provider preference.
static DESKTOP_PROVIDER: AtomicU8 = AtomicU8::new(DesktopExecutionProvider::PlatformDefault as u8);

// PHOTOVAULT PATCH: desktop execution provider preference.
pub(crate) fn set_desktop_execution_provider(provider: DesktopExecutionProvider) {
    DESKTOP_PROVIDER.store(provider as u8, Ordering::Relaxed);
}

// PHOTOVAULT PATCH: desktop execution provider preference.
pub(crate) fn desktop_execution_provider() -> DesktopExecutionProvider {
    match DESKTOP_PROVIDER.load(Ordering::Relaxed) {
        1 => DesktopExecutionProvider::Auto,
        2 => DesktopExecutionProvider::Cuda,
        3 => DesktopExecutionProvider::Rocm,
        4 => DesktopExecutionProvider::MiGraphX,
        5 => DesktopExecutionProvider::WebGpu,
        6 => DesktopExecutionProvider::Cpu,
        _ => DesktopExecutionProvider::PlatformDefault,
    }
}

#[derive(Debug)]
pub(super) struct ProviderPlan {
    providers: Vec<ExecutionProvider>,
    next: usize,
}

impl ProviderPlan {
    pub(super) fn new(
        mode: ExecutionMode,
        model_path: &str,
        validation: AccelerationValidation,
    ) -> Self {
        let providers = match mode {
            ExecutionMode::PlatformDefault => platform_default_providers(model_path, validation),
            ExecutionMode::GpuPreferred => platform_default_providers(model_path, validation)
                .into_iter()
                .filter(|provider| *provider != ExecutionProvider::Xnnpack)
                .collect(),
            ExecutionMode::CpuOnly => vec![ExecutionProvider::Cpu],
        };
        Self::from_providers(providers)
    }

    fn from_providers(providers: Vec<ExecutionProvider>) -> Self {
        Self { providers, next: 0 }
    }

    fn next_provider(&mut self) -> Option<ExecutionProvider> {
        let provider = self.providers.get(self.next).copied()?;
        self.next += 1;
        Some(provider)
    }

    pub(super) fn has_fallback(&self) -> bool {
        self.next < self.providers.len()
    }

    fn retain_last_provider_for_retry(&mut self) {
        self.next = self.providers.len().saturating_sub(1);
    }
}

pub(super) fn run_provider_plan<T, E>(
    plan: &mut ProviderPlan,
    mut attempt: impl FnMut(ExecutionProvider) -> Result<T, E>,
) -> Result<(T, ExecutionProvider), Vec<E>> {
    let mut errors = Vec::new();
    while let Some(provider) = plan.next_provider() {
        match attempt(provider) {
            Ok(value) => return Ok((value, provider)),
            Err(error) => errors.push(error),
        }
    }

    plan.retain_last_provider_for_retry();
    Err(errors)
}

pub(super) struct ProviderAttempt {
    providers: Vec<ExecutionProviderDispatch>,
    disable_intra_op_spinning: bool,
    disable_cpu_fallback: bool,
    coreml_cache_dir: Option<PathBuf>,
    execution_provider: ExecutionProvider,
}

impl ProviderAttempt {
    fn cpu_only() -> Self {
        Self {
            providers: vec![CPU::default().with_arena_allocator(true).build()],
            disable_intra_op_spinning: false,
            disable_cpu_fallback: false,
            coreml_cache_dir: None,
            execution_provider: ExecutionProvider::Cpu,
        }
    }

    pub(super) fn coreml_cache_dir(&self) -> Option<&Path> {
        self.coreml_cache_dir.as_deref()
    }

    pub(super) fn execution_provider(&self) -> ExecutionProvider {
        self.execution_provider
    }
}

pub(super) fn provider_attempt(
    provider: ExecutionProvider,
    _model_path: &str,
    _model_namespace: &str,
    _gpu_options: Option<&GpuOptions>,
) -> ProviderAttempt {
    let _disable_cpu_fallback = _gpu_options.is_some_and(|options| {
        provider == ExecutionProvider::WebGpu
            || (provider == ExecutionProvider::CoreMl && !options.subgraphs)
    });
    match provider {
        ExecutionProvider::Cpu => ProviderAttempt::cpu_only(),
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        ExecutionProvider::CoreMl => {
            let (coreml_provider, coreml_cache_dir) =
                coreml_provider(_model_path, _model_namespace, _gpu_options);
            ProviderAttempt {
                providers: if _gpu_options.is_some() {
                    vec![coreml_provider]
                } else {
                    vec![
                        coreml_provider,
                        CPU::default().with_arena_allocator(true).build(),
                    ]
                },
                disable_intra_op_spinning: false,
                disable_cpu_fallback: _disable_cpu_fallback,
                coreml_cache_dir,
                execution_provider: ExecutionProvider::CoreMl,
            }
        }
        #[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
        ExecutionProvider::WebGpu => ProviderAttempt {
            providers: if let Some(options) = _gpu_options {
                vec![ocr_webgpu_provider(options.prefer_nhwc)]
            } else {
                webgpu_attempt_providers()
            },
            disable_intra_op_spinning: true,
            disable_cpu_fallback: _disable_cpu_fallback,
            coreml_cache_dir: None,
            execution_provider: ExecutionProvider::WebGpu,
        },
        #[cfg(target_os = "android")]
        ExecutionProvider::Xnnpack => xnnpack_attempt(),
        // PHOTOVAULT PATCH: desktop GPU execution providers.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        ExecutionProvider::Cuda | ExecutionProvider::Rocm | ExecutionProvider::MiGraphX => {
            desktop_gpu_attempt(provider)
        }
        _ => unreachable!("provider is not available on this platform"),
    }
}

fn environment() -> MlResult<Environment> {
    static ENVIRONMENT: Mutex<Option<Environment>> = Mutex::new(None);
    let mut current = ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(environment) = current.as_ref() {
        return Ok(environment.clone());
    }
    let environment = ort::init().build()?;
    *current = Some(environment.clone());
    Ok(environment)
}

pub(super) fn build_session(model_path: &str, attempt: ProviderAttempt) -> MlResult<Session> {
    let environment = environment()?;
    let mut builder = Session::builder(&environment)?
        .with_auto_device(AutoDevicePolicy::PreferCPU)?
        .with_optimization_level(GraphOptimizationLevel::All)?
        .with_intra_threads(1)?
        .with_inter_threads(1)?;

    if attempt.disable_cpu_fallback {
        builder = builder.with_disable_cpu_fallback()?;
    }

    if attempt.disable_intra_op_spinning {
        builder = builder.with_intra_op_spinning(false)?;
    }
    builder = builder.with_execution_providers(attempt.providers)?;

    let session = builder
        .commit_from_file(model_path)
        .map_err(|error| session_load_error(model_path, error))?;
    Ok(session)
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
fn platform_default_providers(
    model_path: &str,
    validation: AccelerationValidation,
) -> Vec<ExecutionProvider> {
    let mut providers = Vec::new();
    if accelerated_provider_allowed(model_path, "CoreML", validation) {
        providers.push(ExecutionProvider::CoreMl);
    }
    providers.push(ExecutionProvider::Cpu);
    providers
}

#[cfg(target_os = "android")]
fn platform_default_providers(
    model_path: &str,
    validation: AccelerationValidation,
) -> Vec<ExecutionProvider> {
    let mut providers = Vec::new();
    if webgpu::attempt_permitted(model_path)
        && accelerated_provider_allowed(model_path, "WebGPU", validation)
    {
        providers.push(ExecutionProvider::WebGpu);
    }
    providers.push(ExecutionProvider::Xnnpack);
    providers.push(ExecutionProvider::Cpu);
    providers
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn platform_default_providers(
    model_path: &str,
    validation: AccelerationValidation,
) -> Vec<ExecutionProvider> {
    // PHOTOVAULT PATCH: honour the desktop execution provider preference.
    if desktop_execution_provider() != DesktopExecutionProvider::PlatformDefault {
        return desktop_preferred_providers(desktop_execution_provider(), model_path, validation);
    }
    let mut providers = Vec::new();
    if webgpu::attempt_permitted(model_path)
        && accelerated_provider_allowed(model_path, "WebGPU", validation)
    {
        providers.push(ExecutionProvider::WebGpu);
    }
    providers.push(ExecutionProvider::Cpu);
    providers
}

// PHOTOVAULT PATCH: provider order for an explicit desktop preference. GPU
// providers stay subject to the golden-entry requirement, so a model without
// goldens runs on CPU exactly as upstream would.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn desktop_preferred_providers(
    preference: DesktopExecutionProvider,
    model_path: &str,
    validation: AccelerationValidation,
) -> Vec<ExecutionProvider> {
    use DesktopExecutionProvider as P;
    let wanted: &[ExecutionProvider] = match preference {
        P::PlatformDefault | P::Cpu => &[],
        P::Auto => &[
            ExecutionProvider::Cuda,
            ExecutionProvider::MiGraphX,
            ExecutionProvider::Rocm,
            ExecutionProvider::WebGpu,
        ],
        P::Cuda => &[ExecutionProvider::Cuda],
        P::Rocm => &[ExecutionProvider::Rocm, ExecutionProvider::MiGraphX],
        P::MiGraphX => &[ExecutionProvider::MiGraphX],
        P::WebGpu => &[ExecutionProvider::WebGpu],
    };
    let mut providers = Vec::new();
    for &provider in wanted {
        let permitted = match provider {
            // The WebGPU crash-canary quarantine still applies.
            ExecutionProvider::WebGpu => webgpu::model_not_quarantined(model_path),
            _ => true,
        };
        if permitted
            && accelerated_provider_allowed(model_path, provider_label(provider), validation)
        {
            providers.push(provider);
        }
    }
    providers.push(ExecutionProvider::Cpu);
    providers
}

// PHOTOVAULT PATCH: human-readable provider names for logs.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn provider_label(provider: ExecutionProvider) -> &'static str {
    match provider {
        ExecutionProvider::CoreMl => "CoreML",
        ExecutionProvider::WebGpu => "WebGPU",
        ExecutionProvider::Xnnpack => "XNNPACK",
        ExecutionProvider::Cpu => "CPU",
        ExecutionProvider::Cuda => "CUDA",
        ExecutionProvider::Rocm => "ROCm",
        ExecutionProvider::MiGraphX => "MIGraphX",
    }
}

// PHOTOVAULT PATCH: desktop GPU attempt. The GPU provider must register
// (`error_on_failure`), so an ONNX Runtime build without it fails this attempt
// and the plan moves on; CPU stays registered for operators the GPU provider
// does not cover, like Ente's WebGPU attempt.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn desktop_gpu_attempt(provider: ExecutionProvider) -> ProviderAttempt {
    let gpu = match provider {
        ExecutionProvider::Cuda => CUDA::default().build().error_on_failure(),
        ExecutionProvider::Rocm => ROCm::default().build().error_on_failure(),
        _ => MIGraphX::default().build().error_on_failure(),
    };
    ProviderAttempt {
        providers: vec![gpu, CPU::default().with_arena_allocator(true).build()],
        disable_intra_op_spinning: true,
        disable_cpu_fallback: false,
        coreml_cache_dir: None,
        execution_provider: provider,
    }
}

#[cfg(not(any(
    target_os = "ios",
    target_os = "android",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
)))]
fn platform_default_providers(
    _model_path: &str,
    _validation: AccelerationValidation,
) -> Vec<ExecutionProvider> {
    vec![ExecutionProvider::Cpu]
}

// Missing goldens fail closed, usually indicating they were not regenerated
// after an update.
#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
))]
fn accelerated_provider_allowed(
    model_path: &str,
    provider_label: &str,
    validation: AccelerationValidation,
) -> bool {
    if validation == AccelerationValidation::Unvalidated
        || golden_test::lookup(model_path).is_some()
    {
        return true;
    }
    log::error!(
        "no golden self-test entry for '{}'; {provider_label} disabled for this model",
        model_file_label(model_path)
    );
    false
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
fn coreml_provider(
    model_path: &str,
    model_namespace: &str,
    options: Option<&GpuOptions>,
) -> (ExecutionProviderDispatch, Option<PathBuf>) {
    let mut provider = CoreML::default()
        .with_model_format(ModelFormat::MLProgram)
        .with_compute_units(ComputeUnits::All)
        .with_specialization_strategy(SpecializationStrategy::Default);

    if let Some(options) = options {
        provider = provider
            .with_compute_units(ComputeUnits::CPUAndGPU)
            .with_low_precision_accumulation_on_gpu(false)
            .with_static_input_shapes(true)
            .with_subgraphs(options.subgraphs);
    }

    let mut prepared_cache_dir = None;
    if ENABLE_PERSISTENT_COREML_CACHE {
        match coreml_cache::prepare_directory(model_path, model_namespace) {
            Ok(cache_dir) => {
                provider = provider.with_model_cache_dir(cache_dir.to_string_lossy());
                prepared_cache_dir = Some(cache_dir);
            }
            Err(error) => {
                log::warn!(
                    "failed to prepare persistent CoreML cache for '{}'; continuing without it: {error}",
                    model_file_label(model_path)
                );
            }
        }
    } else {
        coreml_cache::remove(model_path);
    }

    (provider.build().error_on_failure(), prepared_cache_dir)
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
fn webgpu_provider() -> ExecutionProviderDispatch {
    let provider = WebGPU::default().with_preferred_layout(PreferredLayout::NCHW);
    #[cfg(any(target_os = "android", target_os = "linux"))]
    let provider = provider.with_dawn_backend_type(DawnBackendType::Vulkan);
    #[cfg(target_os = "windows")]
    let provider = provider.with_dawn_backend_type(DawnBackendType::D3D12);
    provider.build().error_on_failure()
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
fn ocr_webgpu_provider(prefer_nhwc: bool) -> ExecutionProviderDispatch {
    use ort::ep::ArbitrarilyConfigurableExecutionProvider;

    let backend = if cfg!(target_os = "windows") {
        "D3D12"
    } else {
        "Vulkan"
    };
    WebGPU::default()
        .with_arbitrary_config("preferredLayout", if prefer_nhwc { "NHWC" } else { "NCHW" })
        .with_arbitrary_config("dawnBackendType", backend)
        .with_arbitrary_config("enableGraphCapture", "0")
        .build()
        .error_on_failure()
}

#[cfg(target_os = "android")]
fn webgpu_attempt_providers() -> Vec<ExecutionProviderDispatch> {
    vec![
        webgpu_provider(),
        xnnpack_provider().fail_silently(),
        CPU::default().with_arena_allocator(true).build(),
    ]
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn webgpu_attempt_providers() -> Vec<ExecutionProviderDispatch> {
    vec![
        webgpu_provider(),
        CPU::default().with_arena_allocator(true).build(),
    ]
}

#[cfg(target_os = "android")]
fn xnnpack_provider() -> ExecutionProviderDispatch {
    #[expect(clippy::expect_used, reason = "The fixed thread count is nonzero")]
    XNNPACK::default()
        .with_intra_op_num_threads(NonZeroUsize::new(4).expect("four is non-zero"))
        .build()
        .error_on_failure()
}

#[cfg(target_os = "android")]
fn xnnpack_attempt() -> ProviderAttempt {
    ProviderAttempt {
        providers: vec![
            xnnpack_provider(),
            CPU::default().with_arena_allocator(true).build(),
        ],
        disable_intra_op_spinning: true,
        disable_cpu_fallback: false,
        coreml_cache_dir: None,
        execution_provider: ExecutionProvider::Xnnpack,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccelerationValidation, ExecutionMode, ExecutionProvider, GpuOptions, ProviderPlan,
        provider_attempt, run_provider_plan,
    };

    #[test]
    fn ocr_options_preserve_default_and_cpu_fallback_configuration() {
        let providers = [
            ExecutionProvider::Cpu,
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            ExecutionProvider::CoreMl,
            #[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
            ExecutionProvider::WebGpu,
            #[cfg(target_os = "android")]
            ExecutionProvider::Xnnpack,
        ];
        for provider in providers {
            let default = provider_attempt(provider, "missing-model.onnx", "test", None);
            assert!(!default.disable_cpu_fallback);
            assert!(
                default
                    .providers
                    .iter()
                    .any(|dispatch| dispatch.name() == "CPUExecutionProvider")
            );
            for subgraphs in [false, true] {
                let options = GpuOptions {
                    subgraphs,
                    #[cfg(any(target_os = "android", target_os = "linux", target_os = "windows"))]
                    prefer_nhwc: false,
                };
                let ocr = provider_attempt(provider, "missing-model.onnx", "test", Some(&options));
                let (strict, provider_count) = match provider {
                    ExecutionProvider::CoreMl => (!subgraphs, 1),
                    ExecutionProvider::WebGpu => (true, 1),
                    ExecutionProvider::Cpu => (false, 1),
                    ExecutionProvider::Xnnpack => (false, 2),
                    // PHOTOVAULT PATCH: desktop GPU providers are not in this list.
                    ExecutionProvider::Cuda
                    | ExecutionProvider::Rocm
                    | ExecutionProvider::MiGraphX => (false, 2),
                };
                assert_eq!(ocr.disable_cpu_fallback, strict);
                assert_eq!(ocr.providers.len(), provider_count);
                assert_eq!(
                    ocr.disable_intra_op_spinning,
                    default.disable_intra_op_spinning
                );
            }
        }
    }

    #[test]
    fn gpu_preferred_falls_back_directly_to_cpu() {
        let mut plan = ProviderPlan::new(
            ExecutionMode::GpuPreferred,
            "model.onnx",
            AccelerationValidation::Unvalidated,
        );
        let mut attempted = Vec::new();
        let selected = run_provider_plan(&mut plan, |provider| {
            attempted.push(provider);
            if provider == ExecutionProvider::Cpu {
                Ok(provider)
            } else {
                Err(())
            }
        })
        .unwrap();
        assert_eq!(selected, (ExecutionProvider::Cpu, ExecutionProvider::Cpu));
        assert!(!attempted.contains(&ExecutionProvider::Xnnpack));
        assert!(!plan.has_fallback());
    }

    #[test]
    fn construction_falls_through_and_selects_the_successful_provider() {
        let mut plan = accelerated_provider_plan();
        let mut attempted = Vec::new();

        let result = run_provider_plan(&mut plan, |provider| {
            attempted.push(provider);
            match provider {
                ExecutionProvider::Xnnpack => Ok("session"),
                _ => Err(provider),
            }
        });

        assert_eq!(result, Ok(("session", ExecutionProvider::Xnnpack)));
        assert_eq!(
            attempted,
            [ExecutionProvider::WebGpu, ExecutionProvider::Xnnpack]
        );
        assert!(plan.has_fallback());
    }

    #[test]
    fn retry_resumes_strictly_after_the_provider_that_was_selected() {
        let mut plan = accelerated_provider_plan();
        run_provider_plan(&mut plan, |provider| match provider {
            ExecutionProvider::Xnnpack => Ok(()),
            _ => Err(()),
        })
        .unwrap();

        let mut attempted = Vec::new();
        let result = run_provider_plan(&mut plan, |provider| {
            attempted.push(provider);
            Ok::<_, ()>("fallback session")
        });

        assert_eq!(result, Ok(("fallback session", ExecutionProvider::Cpu)));
        assert_eq!(attempted, [ExecutionProvider::Cpu]);
        assert!(!plan.has_fallback());
    }

    #[test]
    fn provider_attempts_are_bounded_and_preserve_each_failure() {
        let mut plan = accelerated_provider_plan();
        let mut attempted = Vec::new();

        let errors = run_provider_plan::<(), _>(&mut plan, |provider| {
            attempted.push(provider);
            Err(provider)
        })
        .unwrap_err();

        assert_eq!(
            attempted,
            [
                ExecutionProvider::WebGpu,
                ExecutionProvider::Xnnpack,
                ExecutionProvider::Cpu,
            ]
        );
        assert_eq!(errors, attempted);
    }

    #[test]
    fn exhausted_cpu_is_retained_as_the_only_construction_retry() {
        let mut plan = ProviderPlan::new(
            ExecutionMode::CpuOnly,
            "model.onnx",
            AccelerationValidation::GoldenRequired,
        );
        let mut attempts = 0;

        let first = run_provider_plan::<(), _>(&mut plan, |provider| {
            attempts += 1;
            Err(provider)
        });

        assert_eq!(first, Err(vec![ExecutionProvider::Cpu]));
        assert_eq!(attempts, 1);
        assert!(plan.has_fallback());

        let retry = run_provider_plan(&mut plan, |provider| {
            attempts += 1;
            Ok::<_, ExecutionProvider>(provider)
        });

        assert_eq!(retry, Ok((ExecutionProvider::Cpu, ExecutionProvider::Cpu)));
        assert_eq!(attempts, 2);
        assert!(!plan.has_fallback());
    }

    fn accelerated_provider_plan() -> ProviderPlan {
        ProviderPlan::from_providers(vec![
            ExecutionProvider::WebGpu,
            ExecutionProvider::Xnnpack,
            ExecutionProvider::Cpu,
        ])
    }
}
