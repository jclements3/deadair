//! DeadAir — first-person night pest-control business sim.
//!
//! Window layout (per design direction): a square **1024×1024 first-person
//! view** top-left, the **controls column in the right remainder**, and the
//! **status strip below** the view. Camp screens replace the view between
//! nights.
//!
//! `deadair --shot out.png [--optic thermal] [--t 0.5]` renders one frame of
//! the real Home Farm zone headless and exits (verification without a
//! window).

use deadair::hunt;

use da_core::{Forecast, Rng};
use da_econ::{Business, OpticModel, PnLStatement};
use da_render::{
    draw::Camera,
    renderer::{OpticMode, OpticSettings, Renderer},
    ThermalPalette,
};
use glam::Vec3;
use hunt::{Mounted, NightHunt};

const VIEW: u32 = 1024;
const ZONE_DIR: &str = "assets/zones";

fn zone_path(file: &str) -> String {
    // Run from repo root or apps/deadair.
    let a = format!("{ZONE_DIR}/{file}");
    if std::path::Path::new(&a).exists() {
        a
    } else {
        format!("../../{a}")
    }
}

/// Which screen the player is on.
enum Screen {
    Camp {
        statement: Option<PnLStatement>,
    },
    Night(Box<NightHunt>),
}

struct App {
    business: Business,
    screen: Screen,
    forecast: Forecast,
    mounted: Mounted,
    renderer: Option<Renderer>,
    view_tex: Option<egui::TextureId>,
    optic_mode: OpticMode,
    palette: ThermalPalette,
    scoped: bool,
    yaw: f32,
    pitch: f32,
    frame: u32,
    hud_flash: Option<(String, f64)>,
    rng: Rng,
}

fn save_path() -> std::path::PathBuf {
    std::env::var_os("DEADAIR_SAVE")
        .map(Into::into)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
            std::path::Path::new(&home).join(".deadair-save.ron")
        })
}

impl App {
    fn new() -> Self {
        let business = std::fs::read_to_string(save_path())
            .ok()
            .and_then(|text| da_econ::save::load_from_ron(&text).ok())
            .unwrap_or_else(Business::new);
        let mut rng = Rng::new(0xDEAD_A112 ^ business.night as u64);
        let forecast = roll_forecast(&mut rng);
        Self {
            business,
            screen: Screen::Camp { statement: None },
            forecast,
            mounted: Mounted::Headlamp,
            renderer: None,
            view_tex: None,
            optic_mode: OpticMode::Eye,
            palette: ThermalPalette::WhiteHot,
            scoped: false,
            yaw: 0.0,
            pitch: 0.0,
            frame: 0,
            hud_flash: None,
            rng,
        }
    }

    fn owned_optics(&self) -> Vec<Mounted> {
        let mut v = vec![Mounted::Headlamp];
        if self.business.owns(da_econ::ItemKind::Optic(OpticModel::NvBasic)) {
            v.push(Mounted::NvBasic);
        }
        if self.business.owns(da_econ::ItemKind::Optic(OpticModel::NvPro)) {
            v.push(Mounted::NvPro);
        }
        if self.business.owns(da_econ::ItemKind::Optic(OpticModel::ThermalMk1)) {
            v.push(Mounted::Thermal(1));
        }
        if self.business.owns(da_econ::ItemKind::Optic(OpticModel::ThermalMk2)) {
            v.push(Mounted::Thermal(2));
        }
        if self.business.owns(da_econ::ItemKind::Optic(OpticModel::ThermalMk3)) {
            v.push(Mounted::Thermal(3));
        }
        v
    }

    fn cash_str(&self) -> String {
        da_econ::fmt_dollars(self.business.cash_cents)
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
    }
}

impl App {
    fn persist(&self) {
        if let Ok(text) = da_econ::save::save_to_ron(&self.business) {
            let _ = std::fs::write(save_path(), text);
        }
    }
}

fn roll_forecast(rng: &mut Rng) -> Forecast {
    Forecast::ALL[rng.below(Forecast::ALL.len() as u64) as usize]
}

fn optic_mode_for(mounted: Mounted) -> OpticMode {
    match mounted {
        Mounted::Headlamp => OpticMode::Eye,
        Mounted::NvBasic | Mounted::NvPro => OpticMode::Nv,
        Mounted::Thermal(_) => OpticMode::Thermal,
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint(); // real-time game
        self.frame = self.frame.wrapping_add(1);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);

        // ---- Right controls column -------------------------------------
        egui::SidePanel::right("controls")
            .resizable(false)
            .exact_width(
                (ctx.screen_rect().width() - VIEW as f32 - 24.0).clamp(220.0, 420.0),
            )
            .show(ctx, |ui| match &mut self.screen {
                Screen::Night(h) => {
                    ui.heading("Loadout");
                    ui.label(format!("Rifle: tier {}", self.business.best_rifle_tier()));
                    ui.separator();
                    ui.label("Mounted optic (swap at camp only):");
                    ui.label(format!("  {:?}", h.mounted));
                    if matches!(h.mounted, Mounted::Thermal(_)) {
                        ui.horizontal(|ui| {
                            ui.label("Palette:");
                            ui.selectable_value(&mut self.palette, ThermalPalette::WhiteHot, "White-hot");
                            ui.selectable_value(&mut self.palette, ThermalPalette::BlackHot, "Black-hot");
                            ui.selectable_value(&mut self.palette, ThermalPalette::ColorblindSafe, "CB-safe");
                        });
                    }
                    ui.separator();
                    if let da_sim::PowerPlant::MultiPump { pumps, max_pumps, .. } = h.sim.rifle.plant {
                        ui.label(format!("Pump: {pumps}/{max_pumps}"));
                        if ui.button("Pump (hold W to keep walking is fine)").clicked() {
                            h.sim.pump(1.5);
                        }
                    }
                    ui.separator();
                    ui.label("Controls: click view to aim (drag = look),\nWASD move, hold Right-drag = scope,\nclick = fire (while scoped).");
                    ui.separator();
                    if ui.button("⏹ Return to camp (end night)").clicked() {
                        h.over = true;
                    }
                    ui.separator();
                    ui.heading("Field log");
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for line in h.log.iter().rev().take(14) {
                            ui.label(line);
                        }
                    });
                }
                Screen::Camp { .. } => {
                    ui.heading("Camp");
                    ui.label(format!("Night {}", self.business.night + 1));
                    ui.label(format!("Cash: {}", da_econ::fmt_dollars(self.business.cash_cents)));
                    ui.separator();
                    ui.heading("Forecast");
                    ui.label(format!("{:?}", self.forecast));
                    ui.label(self.forecast.blurb());
                    ui.separator();
                    ui.heading("Mount optic");
                    for m in self.owned_optics() {
                        ui.radio_value(&mut self.mounted, m, format!("{m:?}"));
                    }
                    ui.separator();
                    ui.heading("Store");
                    let mut buy = |ui: &mut egui::Ui, label: &str, model: OpticModel, biz: &mut Business| {
                        if biz.owns(da_econ::ItemKind::Optic(model)) {
                            ui.label(format!("✓ {label}"));
                        } else if ui.button(label).clicked() {
                            match biz.buy_optic(model) {
                                Ok(()) => {}
                                Err(e) => { self.hud_flash = Some((format!("{e:?}"), 0.0)); }
                            }
                        }
                    };
                    buy(ui, "NV Basic — $220", OpticModel::NvBasic, &mut self.business);
                    buy(ui, "NV Pro — $480", OpticModel::NvPro, &mut self.business);
                    buy(ui, "Thermal Mk I — $550", OpticModel::ThermalMk1, &mut self.business);
                }
            });

        // ---- Bottom status strip ----------------------------------------
        egui::TopBottomPanel::bottom("status").exact_height(48.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                match &self.screen {
                    Screen::Night(h) => {
                        ui.monospace(h.hud_line(&self.cash_str()));
                        if let Some((msg, _)) = &self.hud_flash {
                            ui.separator();
                            ui.colored_label(egui::Color32::YELLOW, msg);
                        }
                    }
                    Screen::Camp { .. } => {
                        ui.monospace(format!(
                            "CAMP | {} | night {} | forecast {:?}",
                            self.cash_str(),
                            self.business.night + 1,
                            self.forecast
                        ));
                        if self.business.is_bankrupt() {
                            ui.colored_label(egui::Color32::RED, "BANKRUPT — campaign over");
                        }
                    }
                }
            });
        });

        // ---- Central: the 1024×1024 first-person view --------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            match &mut self.screen {
                Screen::Camp { statement } => {
                    ui.heading("DeadAir — camp");
                    if let Some(st) = statement {
                        ui.monospace(st.to_string());
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        if ui.add_sized([220.0, 48.0], egui::Button::new("🌙 Start night (Home Farm)")).clicked() {
                            let seed = self.rng.next_u64();
                            match NightHunt::new(
                                &zone_path("home_farm.zone.ron"),
                                self.forecast,
                                &self.business,
                                seed,
                                self.mounted,
                            ) {
                                Ok(h) => {
                                    self.optic_mode = optic_mode_for(self.mounted);
                                    self.screen = Screen::Night(Box::new(h));
                                }
                                Err(e) => self.hud_flash = Some((e, 0.0)),
                            }
                        }
                        if ui.button("Skip night ($15 camp fee)").clicked() {
                            let st = self.business.skip_night();
                            self.forecast = roll_forecast(&mut self.rng);
                            self.screen = Screen::Camp { statement: Some(st) };
                            self.persist();
                        }
                        if self.business.is_bankrupt()
                            && ui.button("💀 New campaign").clicked()
                        {
                            self.business = Business::new();
                            self.screen = Screen::Camp { statement: None };
                            self.persist();
                        }
                    });
                }
                Screen::Night(h) => {
                    let (yaw, pitch) = (self.yaw, self.pitch);
                    let fwd = Vec3::new(
                        yaw.sin() * pitch.cos(),
                        pitch.sin(),
                        -yaw.cos() * pitch.cos(),
                    );
                    // Advance simulation from input.
                    let (move_dir, fire_clicked) = ui.input(|i| {
                        let mut d = Vec3::ZERO;
                        let flat = Vec3::new(yaw.sin(), 0.0, -yaw.cos()).normalize_or_zero();
                        let right = flat.cross(Vec3::Y);
                        if i.key_down(egui::Key::W) { d += flat; }
                        if i.key_down(egui::Key::S) { d -= flat; }
                        if i.key_down(egui::Key::A) { d -= right; }
                        if i.key_down(egui::Key::D) { d += right; }
                        (d.normalize_or_zero() * 4.0, i.pointer.primary_clicked())
                    });
                    h.tick(dt, move_dir, self.scoped);

                    // Lazy renderer + texture registration on eframe's device.
                    let rs = frame.wgpu_render_state().expect("wgpu backend");
                    if self.renderer.is_none() {
                        let r = Renderer::new_on(&rs.device, VIEW, VIEW);
                        let id = rs.renderer.write().register_native_texture(
                            &rs.device,
                            &r.output_view(),
                            wgpu::FilterMode::Nearest,
                        );
                        self.renderer = Some(r);
                        self.view_tex = Some(id);
                    }
                    let renderer = self.renderer.as_mut().expect("set above");

                    let cam = Camera {
                        eye: h.sim.player.pos,
                        look: h.sim.player.pos + fwd,
                        up: Vec3::Y,
                        fov_y_deg: if self.scoped { 16.0 } else { 60.0 },
                        aspect: 1.0,
                    };
                    let mods = h.forecast.mods();
                    let settings = OpticSettings {
                        mode: if self.scoped { self.optic_mode } else { OpticMode::Eye },
                        palette: self.palette,
                        scope_mask: self.scoped,
                        frame: self.frame,
                        seed: 11,
                        nv_gain: 1.0 / mods.nv_visibility.max(0.3),
                        nv_visibility: mods.nv_visibility,
                        eye_exposure: mods.eye_visibility,
                    };
                    let list = h.draw_list();
                    renderer.render_on(&rs.device, &rs.queue, &list, &cam, &settings, dt);

                    // The square view, top-left of the central region.
                    let avail = ui.available_size();
                    let side = (VIEW as f32).min(avail.x).min(avail.y);
                    let resp = ui.add(
                        egui::Image::new((self.view_tex.expect("registered"), egui::vec2(side, side)))
                            .sense(egui::Sense::click_and_drag()),
                    );
                    // Mouse-look: any drag on the view.
                    if resp.dragged() {
                        let d = resp.drag_delta();
                        self.yaw += d.x * 0.004;
                        self.pitch = (self.pitch - d.y * 0.004).clamp(-1.4, 1.4);
                    }
                    self.scoped = resp.hovered()
                        && ui.input(|i| i.pointer.secondary_down());
                    if fire_clicked && resp.hovered() && self.scoped {
                        if let Some(msg) = h.fire(fwd, &self.business) {
                            self.hud_flash = Some((msg, 0.0));
                        }
                    }

                    // Night over → settle at camp.
                    if h.over {
                        let statement = self.business.settle_night(&h.ledger);
                        self.forecast = roll_forecast(&mut self.rng);
                        self.screen = Screen::Camp { statement: Some(statement) };
                        self.persist();
                    }
                }
            }
        });
    }
}

fn headless_shot(path: &str, optic: OpticMode, night_t: f32) {
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let business = Business::new();
    let mounted = match optic {
        OpticMode::Eye => Mounted::Headlamp,
        OpticMode::Nv => Mounted::NvBasic,
        OpticMode::Thermal => Mounted::Thermal(1),
    };
    let mut h = NightHunt::new(
        &zone_path("home_farm.zone.ron"),
        Forecast::Clear,
        &business,
        42,
        mounted,
    )
    .expect("hunt");
    h.clock.seek(night_t);
    // A few sim seconds so thermal + AI settle.
    for _ in 0..30 {
        h.tick(0.5, Vec3::ZERO, false);
    }
    let cam = Camera {
        eye: h.sim.player.pos + Vec3::new(0.0, 0.4, 0.0),
        look: Vec3::new(60.0, 1.0, 40.0),
        up: Vec3::Y,
        fov_y_deg: 45.0,
        aspect: 1.0,
    };
    let settings = OpticSettings {
        mode: optic,
        scope_mask: true,
        ..Default::default()
    };
    let list = h.draw_list();
    for _ in 0..30 {
        renderer.render(&gpu, &list, &cam, &settings, 0.1);
    }
    let rgba = renderer.read_rgba(&gpu);
    image::save_buffer(path, &rgba, VIEW, VIEW, image::ColorType::Rgba8).expect("png");
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
        let t = args
            .iter()
            .position(|a| a == "--t")
            .and_then(|j| args.get(j + 1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.2);
        headless_shot(path, optic, t);
        return;
    }

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DeadAir")
            .with_inner_size([VIEW as f32 + 320.0, VIEW as f32 + 96.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("DeadAir", native, Box::new(|_| Ok(Box::new(App::new()))))
        .expect("eframe run");
}
