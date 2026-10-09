//! GPU device bootstrap (§17 of the detailed design document).
//!
//! One [`GpuContext`] per application: instance, adapter, device and queue. The surface itself is
//! created per window by [`GpuContext::create_surface`], which accepts anything that carries raw
//! window and display handles - the wiring layer hands it a `winit` window without this crate
//! depending on `winit` (principle P1).

use rustui_core::{Result, UiError};
use wgpu::{
    Device, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints, Queue,
    RequestAdapterOptions, Surface,
};

/// Everything needed to draw: instance, adapter, device and queue.
pub struct GpuContext {
    instance: Instance,
    adapter: wgpu::Adapter,
    device: Device,
    queue: Queue,
}

impl std::fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContext")
            .field("adapter", &self.adapter.get_info().name)
            .finish_non_exhaustive()
    }
}

impl GpuContext {
    /// Creates a context with the default instance options.
    pub fn new() -> Result<Self> {
        Self::with_descriptor(InstanceDescriptor::new_without_display_handle())
    }

    /// Creates a context from an explicit instance descriptor.
    ///
    /// The wiring layer uses this to pass a `winit` display handle along, which the GLES backend
    /// needs to present on Wayland (§41).
    pub fn with_descriptor(descriptor: InstanceDescriptor) -> Result<Self> {
        let instance = Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|error| UiError::GpuUnavailable(error.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("rustui-device"),
            required_features: Features::empty(),
            required_limits: Limits::default(),
            experimental_features: Default::default(),
            memory_hints: MemoryHints::default(),
            trace: Default::default(),
        }))
        .map_err(|error| UiError::DeviceCreation(error.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// The wgpu instance.
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// The selected adapter (used to query surface capabilities).
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// The logical device.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The queue used for uploads and submissions.
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Adapter name, for diagnostics.
    pub fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }

    /// Creates a render surface for a window.
    ///
    /// `window` only needs the raw window and display handles, so the caller can pass a
    /// reference counted `winit` window without this crate knowing about `winit`.
    pub fn create_surface(
        &self,
        window: impl wgpu::DisplayAndWindowHandle + 'static,
    ) -> Result<Surface<'static>> {
        self.instance
            .create_surface(window)
            .map_err(|error| UiError::SurfaceCreation(error.to_string()))
    }

    /// Blocks until queued GPU work finished (used after resource creation in tests).
    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}
