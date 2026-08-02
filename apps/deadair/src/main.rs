//! DeadAir — first-person night pest-control business sim.
//!
//! Current shell: walk the placeholder farm with WASD + mouse, switch optics
//! with 1/2/3 (naked eye / NV / thermal), hold right mouse to scope,
//! P cycles thermal palette, Esc quits.
//!
//! `deadair --shot out.png [--optic thermal]` renders one frame headless
//! and exits — used for CI-style verification without a window.

mod world;

use da_render::{
    draw::Camera,
    gpu::Gpu,
    renderer::{OpticMode, OpticSettings, Renderer},
    Presenter, ThermalPalette,
};
use glam::Vec3;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const RENDER_W: u32 = 960;
const RENDER_H: u32 = 540;

struct Player {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    keys: [bool; 6], // W A S D up down
}

impl Player {
    fn new() -> Self {
        Self {
            pos: Vec3::new(0.0, 1.6, 0.0),
            yaw: 0.0, // facing -Z
            pitch: 0.0,
            keys: [false; 6],
        }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }

    fn tick(&mut self, dt: f32) {
        let f = self.forward();
        let flat = Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
        let right = flat.cross(Vec3::Y);
        let speed = 4.0; // m/s walk
        let mut v = Vec3::ZERO;
        if self.keys[0] {
            v += flat;
        }
        if self.keys[1] {
            v -= right;
        }
        if self.keys[2] {
            v -= flat;
        }
        if self.keys[3] {
            v += right;
        }
        self.pos += v.normalize_or_zero() * speed * dt;
        self.pos.y = 1.6; // eye height; terrain is flat for now
    }

    fn camera(&self, aspect: f32, scoped: bool) -> Camera {
        Camera {
            eye: self.pos,
            look: self.pos + self.forward(),
            up: Vec3::Y,
            fov_y_deg: if scoped { 18.0 } else { 60.0 },
            aspect,
        }
    }
}

struct App {
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    surface_cfg: Option<wgpu::SurfaceConfiguration>,
    gpu: Option<Gpu>,
    renderer: Option<Renderer>,
    presenter: Option<Presenter>,
    player: Player,
    optic: OpticMode,
    palette: ThermalPalette,
    scoped: bool,
    frame: u32,
    last: std::time::Instant,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            surface: None,
            surface_cfg: None,
            gpu: None,
            renderer: None,
            presenter: None,
            player: Player::new(),
            optic: OpticMode::Eye,
            palette: ThermalPalette::WhiteHot,
            scoped: false,
            frame: 0,
            last: std::time::Instant::now(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("DeadAir")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
                )
                .expect("window"),
        );
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("surface");
        let gpu = Gpu::for_surface(instance, &surface).expect("gpu");
        let size = window.inner_size();
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let cfg = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &cfg);
        self.renderer = Some(Renderer::new(&gpu, RENDER_W, RENDER_H));
        self.presenter = Some(Presenter::new(&gpu, format));
        self.window = Some(window);
        self.surface = Some(surface);
        self.surface_cfg = Some(cfg);
        self.gpu = Some(gpu);
        self.last = std::time::Instant::now();
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            let sens = 0.0022;
            self.player.yaw += delta.0 as f32 * sens;
            self.player.pitch = (self.player.pitch - delta.1 as f32 * sens)
                .clamp(-1.4, 1.4);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(surface), Some(cfg), Some(gpu)) = (
                    self.surface.as_ref(),
                    self.surface_cfg.as_mut(),
                    self.gpu.as_ref(),
                ) {
                    cfg.width = size.width.max(1);
                    cfg.height = size.height.max(1);
                    surface.configure(&gpu.device, cfg);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::KeyW) => self.player.keys[0] = down,
                    PhysicalKey::Code(KeyCode::KeyA) => self.player.keys[1] = down,
                    PhysicalKey::Code(KeyCode::KeyS) => self.player.keys[2] = down,
                    PhysicalKey::Code(KeyCode::KeyD) => self.player.keys[3] = down,
                    PhysicalKey::Code(KeyCode::Digit1) if down => self.optic = OpticMode::Eye,
                    PhysicalKey::Code(KeyCode::Digit2) if down => self.optic = OpticMode::Nv,
                    PhysicalKey::Code(KeyCode::Digit3) if down => {
                        self.optic = OpticMode::Thermal
                    }
                    PhysicalKey::Code(KeyCode::KeyP) if down => {
                        self.palette = match self.palette {
                            ThermalPalette::WhiteHot => ThermalPalette::BlackHot,
                            ThermalPalette::BlackHot => ThermalPalette::ColorblindSafe,
                            ThermalPalette::ColorblindSafe => ThermalPalette::WhiteHot,
                        };
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Right {
                    self.scoped = state == ElementState::Pressed;
                }
            }
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                self.player.tick(dt);
                self.frame = self.frame.wrapping_add(1);

                let (Some(gpu), Some(renderer), Some(presenter), Some(surface), Some(cfg)) = (
                    self.gpu.as_ref(),
                    self.renderer.as_mut(),
                    self.presenter.as_ref(),
                    self.surface.as_ref(),
                    self.surface_cfg.as_ref(),
                ) else {
                    return;
                };

                let list = world::placeholder_scene(48.0);
                let cam = self
                    .player
                    .camera(RENDER_W as f32 / RENDER_H as f32, self.scoped);
                let settings = OpticSettings {
                    mode: if self.scoped { self.optic } else { OpticMode::Eye },
                    palette: self.palette,
                    scope_mask: self.scoped,
                    frame: self.frame,
                    seed: 7,
                    ..Default::default()
                };
                renderer.render(gpu, &list, &cam, &settings, dt);

                match surface.get_current_texture() {
                    Ok(tex) => {
                        let view = tex.texture.create_view(&Default::default());
                        presenter.present(gpu, renderer, &view, cfg.width, cfg.height);
                        tex.present();
                    }
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        surface.configure(&gpu.device, cfg);
                    }
                    Err(e) => eprintln!("surface error: {e:?}"),
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn headless_shot(path: &str, optic: OpticMode) {
    let gpu = Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, RENDER_W, RENDER_H);
    let list = world::placeholder_scene(48.0);
    let player = Player::new();
    let cam = player.camera(RENDER_W as f32 / RENDER_H as f32, true);
    let settings = OpticSettings {
        mode: optic,
        scope_mask: true,
        ..Default::default()
    };
    // Settle AGC, then shoot.
    for _ in 0..30 {
        renderer.render(&gpu, &list, &cam, &settings, 0.1);
    }
    let rgba = renderer.read_rgba(&gpu);
    image::save_buffer(path, &rgba, RENDER_W, RENDER_H, image::ColorType::Rgba8)
        .expect("png save");
    println!("wrote {path}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("shot.png");
        let optic = match args
            .iter()
            .position(|a| a == "--optic")
            .and_then(|j| args.get(j + 1))
            .map(String::as_str)
        {
            Some("nv") => OpticMode::Nv,
            Some("thermal") => OpticMode::Thermal,
            _ => OpticMode::Eye,
        };
        headless_shot(path, optic);
        return;
    }

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run");
}
