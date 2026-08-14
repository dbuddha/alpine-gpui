use std::{error::Error, fmt};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::native as platform;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
use crate::unsupported as platform;

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) const VERTEX_ENTRY_POINT: &str = "alpine_quad_vertex";
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) const FRAGMENT_ENTRY_POINT: &str = "alpine_quad_fragment";

/// An initialized Alpine-owned Direct Metal backend.
///
/// Native objects remain private and are released through deterministic Rust
/// ownership when this value is dropped.
#[must_use]
pub struct MetalBackend {
    _native: platform::NativeBackend,
    capabilities: MetalCapabilities,
}

impl MetalBackend {
    /// Creates the default Direct Metal device, queue, offline library, and
    /// fixed BGRA8 render pipeline.
    ///
    /// # Errors
    ///
    /// Returns a stage-classified [`InitializationError`] without panicking or
    /// terminating the process.
    pub fn new() -> Result<Self, InitializationError> {
        platform::new_backend().map(Self::from_platform_parts)
    }

    fn from_platform_parts(
        (native, capabilities): (platform::NativeBackend, MetalCapabilities),
    ) -> Self {
        Self {
            _native: native,
            capabilities,
        }
    }

    /// Returns the capabilities captured during initialization.
    #[must_use]
    pub fn capabilities(&self) -> &MetalCapabilities {
        &self.capabilities
    }
}

/// Capabilities observed from the selected physical Metal device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetalCapabilities {
    name: String,
    registry_id: u64,
    properties: u8,
}

impl MetalCapabilities {
    const METAL3: u8 = 1 << 0;
    const UNIFIED_MEMORY: u8 = 1 << 1;
    const LOW_POWER: u8 = 1 << 2;
    const REMOVABLE: u8 = 1 << 3;

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn new(name: String, registry_id: u64) -> Self {
        Self {
            name,
            registry_id,
            properties: 0,
        }
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) const fn with_metal3(mut self, supported: bool) -> Self {
        self.set_property(Self::METAL3, supported);
        self
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) const fn with_unified_memory(mut self, unified: bool) -> Self {
        self.set_property(Self::UNIFIED_MEMORY, unified);
        self
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) const fn with_low_power(mut self, low_power: bool) -> Self {
        self.set_property(Self::LOW_POWER, low_power);
        self
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) const fn with_removable(mut self, removable: bool) -> Self {
        self.set_property(Self::REMOVABLE, removable);
        self
    }

    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    const fn set_property(&mut self, property: u8, enabled: bool) {
        if enabled {
            self.properties |= property;
        } else {
            self.properties &= !property;
        }
    }

    const fn has_property(&self, property: u8) -> bool {
        self.properties & property != 0
    }

    /// Returns the device name reported by Metal.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the process-independent I/O Registry identifier.
    #[must_use]
    pub const fn registry_id(&self) -> u64 {
        self.registry_id
    }

    /// Returns whether the device supports the Metal 3 family baseline.
    #[must_use]
    pub const fn supports_metal3(&self) -> bool {
        self.has_property(Self::METAL3)
    }

    /// Returns whether CPU and GPU share physical memory.
    #[must_use]
    pub const fn has_unified_memory(&self) -> bool {
        self.has_property(Self::UNIFIED_MEMORY)
    }

    /// Returns whether Metal classifies the device as low power.
    #[must_use]
    pub const fn is_low_power(&self) -> bool {
        self.has_property(Self::LOW_POWER)
    }

    /// Returns whether Metal classifies the device as removable.
    #[must_use]
    pub const fn is_removable(&self) -> bool {
        self.has_property(Self::REMOVABLE)
    }
}

/// Native failure details copied into Alpine-owned memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeFailure {
    domain: String,
    code: i64,
    description: String,
}

impl NativeFailure {
    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn new(domain: String, code: i64, description: String) -> Self {
        Self {
            domain,
            code,
            description,
        }
    }

    /// Returns the native error domain.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Returns the native error code.
    #[must_use]
    pub const fn code(&self) -> i64 {
        self.code
    }

    /// Returns a copied native error description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl fmt::Display for NativeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} error {}: {}",
            self.domain, self.code, self.description
        )
    }
}

impl Error for NativeFailure {}

/// Stage at which native Metal initialization stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitializationStage {
    /// Target-platform validation.
    Platform,
    /// Default-device discovery.
    Device,
    /// Device capability inspection.
    Capabilities,
    /// Command-queue creation.
    CommandQueue,
    /// Offline library loading.
    Library,
    /// Vertex-function lookup.
    VertexFunction,
    /// Fragment-function lookup.
    FragmentFunction,
    /// Render-pipeline creation.
    RenderPipeline,
}

/// Structured failure from Direct Metal initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitializationError {
    /// The binary is not running on Apple Silicon macOS.
    UnsupportedPlatform {
        /// Compile-time processor architecture.
        architecture: &'static str,
        /// Compile-time operating system.
        operating_system: &'static str,
    },
    /// Metal returned no system-default device.
    DeviceUnavailable,
    /// Device properties could not be copied across the native boundary.
    CapabilityQueryFailed(NativeFailure),
    /// The selected device does not satisfy Alpine's baseline.
    UnsupportedDevice {
        /// Metal-reported device name.
        device_name: String,
        /// Failed Alpine baseline condition.
        reason: &'static str,
    },
    /// Metal returned no command queue.
    CommandQueueUnavailable,
    /// The embedded offline library could not be loaded.
    LibraryLoadFailed(NativeFailure),
    /// A required entry point is absent from the offline library.
    MissingFunction {
        /// Lookup stage associated with the function.
        stage: InitializationStage,
        /// Required static entry-point name.
        name: &'static str,
    },
    /// Metal rejected the fixed render-pipeline descriptor.
    PipelineCreationFailed(NativeFailure),
}

impl InitializationError {
    /// Returns the failed initialization stage.
    #[must_use]
    pub const fn stage(&self) -> InitializationStage {
        match self {
            Self::UnsupportedPlatform { .. } => InitializationStage::Platform,
            Self::DeviceUnavailable => InitializationStage::Device,
            Self::CapabilityQueryFailed(_) | Self::UnsupportedDevice { .. } => {
                InitializationStage::Capabilities
            }
            Self::CommandQueueUnavailable => InitializationStage::CommandQueue,
            Self::LibraryLoadFailed(_) => InitializationStage::Library,
            Self::MissingFunction { stage, .. } => *stage,
            Self::PipelineCreationFailed(_) => InitializationStage::RenderPipeline,
        }
    }
}

impl fmt::Display for InitializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform {
                architecture,
                operating_system,
            } => write!(
                formatter,
                "Direct Metal requires Apple Silicon macOS, found {architecture}-{operating_system}"
            ),
            Self::DeviceUnavailable => formatter.write_str("Metal returned no default device"),
            Self::CapabilityQueryFailed(failure) => {
                write!(formatter, "Metal capability query failed: {failure}")
            }
            Self::UnsupportedDevice {
                device_name,
                reason,
            } => write!(
                formatter,
                "Metal device {device_name} is unsupported: {reason}"
            ),
            Self::CommandQueueUnavailable => {
                formatter.write_str("Metal command-queue creation failed")
            }
            Self::LibraryLoadFailed(failure) => {
                write!(formatter, "offline Metal library load failed: {failure}")
            }
            Self::MissingFunction { name, .. } => {
                write!(formatter, "offline Metal library is missing {name}")
            }
            Self::PipelineCreationFailed(failure) => {
                write!(
                    formatter,
                    "Metal render-pipeline creation failed: {failure}"
                )
            }
        }
    }
}

impl Error for InitializationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CapabilityQueryFailed(failure)
            | Self::LibraryLoadFailed(failure)
            | Self::PipelineCreationFailed(failure) => Some(failure),
            Self::UnsupportedPlatform { .. }
            | Self::DeviceUnavailable
            | Self::UnsupportedDevice { .. }
            | Self::CommandQueueUnavailable
            | Self::MissingFunction { .. } => None,
        }
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) trait InitializationDriver {
    type Device;
    type Queue;
    type Library;
    type Function;
    type Pipeline;

    fn create_device(&self) -> Option<Self::Device>;
    fn capabilities(&self, device: &Self::Device) -> Result<MetalCapabilities, NativeFailure>;
    fn create_queue(&self, device: &Self::Device) -> Option<Self::Queue>;
    fn load_library(&self, device: &Self::Device) -> Result<Self::Library, NativeFailure>;
    fn find_function(&self, library: &Self::Library, name: &'static str) -> Option<Self::Function>;
    fn create_pipeline(
        &self,
        device: &Self::Device,
        vertex: &Self::Function,
        fragment: &Self::Function,
    ) -> Result<Self::Pipeline, NativeFailure>;
}

#[allow(
    dead_code,
    reason = "native handles are intentionally retained until backend teardown"
)]
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) struct Initialized<D: InitializationDriver> {
    pub(crate) capabilities: MetalCapabilities,
    pub(crate) device: D::Device,
    pub(crate) queue: D::Queue,
    pub(crate) library: D::Library,
    pub(crate) pipeline: D::Pipeline,
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn initialize<D: InitializationDriver>(
    driver: &D,
) -> Result<Initialized<D>, InitializationError> {
    initialize_with_capability_validation(driver, require_supported_device)
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
pub(crate) fn initialize_for_native_validation<D: InitializationDriver>(
    driver: &D,
) -> Result<Initialized<D>, InitializationError> {
    initialize_with_capability_validation(driver, |_| Ok(()))
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn initialize_with_capability_validation<D, V>(
    driver: &D,
    validate_capabilities: V,
) -> Result<Initialized<D>, InitializationError>
where
    D: InitializationDriver,
    V: FnOnce(&MetalCapabilities) -> Result<(), InitializationError>,
{
    let device = driver
        .create_device()
        .ok_or(InitializationError::DeviceUnavailable)?;
    let capabilities = driver
        .capabilities(&device)
        .map_err(InitializationError::CapabilityQueryFailed)?;
    validate_capabilities(&capabilities)?;
    let queue = driver
        .create_queue(&device)
        .ok_or(InitializationError::CommandQueueUnavailable)?;
    let library = driver
        .load_library(&device)
        .map_err(InitializationError::LibraryLoadFailed)?;
    let vertex = driver.find_function(&library, VERTEX_ENTRY_POINT).ok_or(
        InitializationError::MissingFunction {
            stage: InitializationStage::VertexFunction,
            name: VERTEX_ENTRY_POINT,
        },
    )?;
    let fragment = driver.find_function(&library, FRAGMENT_ENTRY_POINT).ok_or(
        InitializationError::MissingFunction {
            stage: InitializationStage::FragmentFunction,
            name: FRAGMENT_ENTRY_POINT,
        },
    )?;
    let pipeline = driver
        .create_pipeline(&device, &vertex, &fragment)
        .map_err(InitializationError::PipelineCreationFailed)?;

    Ok(Initialized {
        capabilities,
        device,
        queue,
        library,
        pipeline,
    })
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn require_supported_device(capabilities: &MetalCapabilities) -> Result<(), InitializationError> {
    if !capabilities.supports_metal3() {
        return Err(InitializationError::UnsupportedDevice {
            device_name: capabilities.name().to_owned(),
            reason: "Metal 3 family support is required",
        });
    }
    if !capabilities.has_unified_memory() {
        return Err(InitializationError::UnsupportedDevice {
            device_name: capabilities.name().to_owned(),
            reason: "unified memory is required",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        error::Error as _,
        rc::Rc,
    };

    use super::{
        FRAGMENT_ENTRY_POINT, InitializationDriver, InitializationError, InitializationStage,
        MetalCapabilities, NativeFailure, VERTEX_ENTRY_POINT, initialize,
    };

    const HANDLE_KINDS: usize = 6;
    const DEVICE: usize = 0;
    const QUEUE: usize = 1;
    const LIBRARY: usize = 2;
    const VERTEX: usize = 3;
    const FRAGMENT: usize = 4;
    const PIPELINE: usize = 5;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FailurePoint {
        None,
        Device,
        Capabilities,
        UnsupportedFamily,
        DiscreteMemory,
        Queue,
        Library,
        Vertex,
        Fragment,
        Pipeline,
    }

    struct Handle {
        kind: usize,
        dropped: Rc<RefCell<[usize; HANDLE_KINDS]>>,
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            self.dropped.borrow_mut()[self.kind] += 1;
        }
    }

    struct MockDriver {
        failure: FailurePoint,
        created: Cell<[usize; HANDLE_KINDS]>,
        dropped: Rc<RefCell<[usize; HANDLE_KINDS]>>,
    }

    impl MockDriver {
        fn new(failure: FailurePoint) -> Self {
            Self {
                failure,
                created: Cell::new([0; HANDLE_KINDS]),
                dropped: Rc::new(RefCell::new([0; HANDLE_KINDS])),
            }
        }

        fn handle(&self, kind: usize) -> Handle {
            let mut created = self.created.get();
            created[kind] += 1;
            self.created.set(created);
            Handle {
                kind,
                dropped: Rc::clone(&self.dropped),
            }
        }

        fn assert_balanced(&self) {
            assert_eq!(self.created.get(), *self.dropped.borrow());
        }
    }

    impl InitializationDriver for MockDriver {
        type Device = Handle;
        type Function = Handle;
        type Library = Handle;
        type Pipeline = Handle;
        type Queue = Handle;

        fn create_device(&self) -> Option<Self::Device> {
            (self.failure != FailurePoint::Device).then(|| self.handle(DEVICE))
        }

        fn capabilities(&self, _device: &Self::Device) -> Result<MetalCapabilities, NativeFailure> {
            if self.failure == FailurePoint::Capabilities {
                return Err(failure());
            }
            Ok(MetalCapabilities::new("Mock GPU".to_owned(), 42)
                .with_metal3(self.failure != FailurePoint::UnsupportedFamily)
                .with_unified_memory(self.failure != FailurePoint::DiscreteMemory)
                .with_low_power(true)
                .with_removable(false))
        }

        fn create_queue(&self, _device: &Self::Device) -> Option<Self::Queue> {
            (self.failure != FailurePoint::Queue).then(|| self.handle(QUEUE))
        }

        fn load_library(&self, _device: &Self::Device) -> Result<Self::Library, NativeFailure> {
            if self.failure == FailurePoint::Library {
                Err(failure())
            } else {
                Ok(self.handle(LIBRARY))
            }
        }

        fn find_function(
            &self,
            _library: &Self::Library,
            name: &'static str,
        ) -> Option<Self::Function> {
            if (name == VERTEX_ENTRY_POINT && self.failure == FailurePoint::Vertex)
                || (name == FRAGMENT_ENTRY_POINT && self.failure == FailurePoint::Fragment)
            {
                None
            } else if name == VERTEX_ENTRY_POINT {
                Some(self.handle(VERTEX))
            } else {
                Some(self.handle(FRAGMENT))
            }
        }

        fn create_pipeline(
            &self,
            _device: &Self::Device,
            _vertex: &Self::Function,
            _fragment: &Self::Function,
        ) -> Result<Self::Pipeline, NativeFailure> {
            if self.failure == FailurePoint::Pipeline {
                Err(failure())
            } else {
                Ok(self.handle(PIPELINE))
            }
        }
    }

    fn failure() -> NativeFailure {
        NativeFailure::new("MockDomain".to_owned(), 17, "injected".to_owned())
    }

    #[test]
    fn initializes_supported_device_and_releases_every_handle_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let driver = MockDriver::new(FailurePoint::None);
        let initialized = initialize(&driver)?;

        assert_eq!(initialized.capabilities.name(), "Mock GPU");
        assert_eq!(initialized.capabilities.registry_id(), 42);
        assert!(initialized.capabilities.supports_metal3());
        assert!(initialized.capabilities.has_unified_memory());
        assert!(initialized.capabilities.is_low_power());
        assert!(!initialized.capabilities.is_removable());
        assert_eq!(*driver.dropped.borrow(), [0, 0, 0, 1, 1, 0]);

        drop(initialized);
        driver.assert_balanced();
        Ok(())
    }

    #[test]
    fn classifies_every_failure_and_releases_partial_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (FailurePoint::Device, InitializationStage::Device),
            (
                FailurePoint::Capabilities,
                InitializationStage::Capabilities,
            ),
            (
                FailurePoint::UnsupportedFamily,
                InitializationStage::Capabilities,
            ),
            (
                FailurePoint::DiscreteMemory,
                InitializationStage::Capabilities,
            ),
            (FailurePoint::Queue, InitializationStage::CommandQueue),
            (FailurePoint::Library, InitializationStage::Library),
            (FailurePoint::Vertex, InitializationStage::VertexFunction),
            (
                FailurePoint::Fragment,
                InitializationStage::FragmentFunction,
            ),
            (FailurePoint::Pipeline, InitializationStage::RenderPipeline),
        ];

        for (failure_point, expected_stage) in cases {
            let driver = MockDriver::new(failure_point);
            let error = initialize(&driver)
                .err()
                .ok_or("failure injection unexpectedly initialized")?;
            assert_eq!(error.stage(), expected_stage);
            drop(error);
            driver.assert_balanced();
        }
        Ok(())
    }

    #[test]
    fn error_contract_preserves_native_details_and_stage() {
        let native = failure();
        assert_eq!(native.domain(), "MockDomain");
        assert_eq!(native.code(), 17);
        assert_eq!(native.description(), "injected");
        assert_eq!(native.to_string(), "MockDomain error 17: injected");

        let missing = InitializationError::MissingFunction {
            stage: InitializationStage::VertexFunction,
            name: VERTEX_ENTRY_POINT,
        };
        assert_eq!(missing.stage(), InitializationStage::VertexFunction);
        assert!(missing.to_string().contains(VERTEX_ENTRY_POINT));
        assert!(missing.source().is_none());

        let library = InitializationError::LibraryLoadFailed(native);
        assert_eq!(library.stage(), InitializationStage::Library);
        assert_eq!(
            library.to_string(),
            "offline Metal library load failed: MockDomain error 17: injected"
        );
        assert!(library.source().is_some());

        let errors = [
            (
                InitializationError::UnsupportedPlatform {
                    architecture: "fixture-arch",
                    operating_system: "fixture-os",
                },
                "Direct Metal requires Apple Silicon macOS, found fixture-arch-fixture-os",
                false,
            ),
            (
                InitializationError::DeviceUnavailable,
                "Metal returned no default device",
                false,
            ),
            (
                InitializationError::CapabilityQueryFailed(failure()),
                "Metal capability query failed: MockDomain error 17: injected",
                true,
            ),
            (
                InitializationError::UnsupportedDevice {
                    device_name: "Fixture GPU".to_owned(),
                    reason: "fixture reason",
                },
                "Metal device Fixture GPU is unsupported: fixture reason",
                false,
            ),
            (
                InitializationError::CommandQueueUnavailable,
                "Metal command-queue creation failed",
                false,
            ),
            (
                InitializationError::MissingFunction {
                    stage: InitializationStage::FragmentFunction,
                    name: FRAGMENT_ENTRY_POINT,
                },
                "offline Metal library is missing alpine_quad_fragment",
                false,
            ),
            (
                InitializationError::PipelineCreationFailed(failure()),
                "Metal render-pipeline creation failed: MockDomain error 17: injected",
                true,
            ),
        ];

        for (error, expected, has_source) in errors {
            assert_eq!(error.to_string(), expected);
            assert_eq!(error.source().is_some(), has_source);
        }
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn assembles_the_safe_owner_from_validated_platform_parts() {
        let capabilities = MetalCapabilities::new("Fixture GPU".to_owned(), 73)
            .with_metal3(true)
            .with_unified_memory(true)
            .with_low_power(true)
            .with_removable(false);
        let backend = super::MetalBackend::from_platform_parts((
            crate::unsupported::NativeBackend,
            capabilities.clone(),
        ));

        assert_eq!(backend.capabilities(), &capabilities);
    }
}
