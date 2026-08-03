//! da-edit — the DeadAir scene/animation editor (SDD §10).
//!
//! Blender-spirit direct manipulation (outliner, viewport with orbit
//! camera, gizmos, and a keyframe dope sheet) over OpenSCAD-spirit
//! scenes-as-code (the zone RON source panel; text is ground truth).
//!
//! All pure logic lives in the `da_edit` library crate (`anim`, `convert`,
//! `gizmo`, `preview`); this binary is the egui shell.

mod app;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DeadAir Editor")
            .with_inner_size([1440.0, 900.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            // Same WSL2 trap the game hits: the Intel adapter reports the
            // surface as supported and then loses the device on request,
            // while the Vulkan path works. Pin Vulkan and ask only for
            // limits a downlevel adapter can actually grant.
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew {
                supported_backends: wgpu::Backends::VULKAN,
                power_preference: wgpu::PowerPreference::HighPerformance,
                device_descriptor: std::sync::Arc::new(|adapter: &wgpu::Adapter| {
                    let limits = if adapter.limits().max_compute_workgroups_per_dimension == 0 {
                        wgpu::Limits::downlevel_webgl2_defaults()
                            .using_resolution(adapter.limits())
                    } else {
                        wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
                    };
                    wgpu::DeviceDescriptor {
                        label: Some("da-edit"),
                        required_features: wgpu::Features::empty(),
                        required_limits: limits,
                        memory_hints: Default::default(),
                    }
                }),
            },
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "da-edit",
        options,
        Box::new(|_cc| Ok(Box::new(app::EditorApp::new()))),
    )
}
