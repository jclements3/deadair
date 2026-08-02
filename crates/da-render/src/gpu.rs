//! Device bring-up with the adapter fallback validated for WSL2:
//! prefer a real GPU when its device creation succeeds, fall back through
//! every adapter (llvmpipe Vulkan works headless), and downgrade limits for
//! compute-less GL adapters.

/// A live wgpu context.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Try every adapter until one yields a device. Headless-safe.
    pub fn new_headless() -> Result<Self, String> {
        Self::pick(wgpu::Instance::default(), None)
    }

    /// Like [`Gpu::new_headless`], but only considers adapters compatible
    /// with `surface` (window presentation).
    pub fn for_surface(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
    ) -> Result<Self, String> {
        Self::pick(instance, Some(surface))
    }

    fn pick(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Self, String> {
        let mut errors = Vec::new();
        for adapter in instance.enumerate_adapters(wgpu::Backends::all()) {
            if let Some(s) = surface {
                if !adapter.is_surface_supported(s) {
                    continue;
                }
            }
            let info = adapter.get_info();
            let limits = if adapter.limits().max_compute_workgroups_per_dimension == 0 {
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
            } else {
                wgpu::Limits::default()
            };
            match pollster::block_on(adapter.request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("da-render device"),
                    required_limits: limits,
                    ..Default::default()
                },
                None,
            )) {
                Ok((device, queue)) => {
                    return Ok(Self {
                        instance,
                        adapter,
                        device,
                        queue,
                    });
                }
                Err(e) => errors.push(format!("{} ({:?}): {e}", info.name, info.backend)),
            }
        }
        Err(format!("no usable GPU adapter; tried: {}", errors.join("; ")))
    }
}
