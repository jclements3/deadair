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
use da_econ::{
    Accessory, Business, Contract, ContractBoard, ItemKind, License, OpticModel, PnLStatement,
    RifleModel,
};
use deadair::camp::{self, ZoneCatalog};
use da_render::{
    draw::Camera,
    renderer::{OpticMode, OpticSettings, Renderer},
    ThermalPalette,
};
use glam::Vec3;
use hunt::{Mounted, NightHunt};

const VIEW: u32 = 1024;
const ZONE_DIR: &str = "assets/zones";

fn zones_dir() -> String {
    if std::path::Path::new(ZONE_DIR).exists() {
        ZONE_DIR.to_string()
    } else {
        format!("../../{ZONE_DIR}")
    }
}

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
    catalog: ZoneCatalog,
    board: ContractBoard,
    selected_zone: String,
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
        let catalog = ZoneCatalog::load(&zones_dir()).unwrap_or_else(|e| {
            eprintln!("zone catalog: {e}");
            ZoneCatalog { zones: Vec::new() }
        });
        let board = camp::generate_board(&catalog, business.night as u64 * 31 + 7, forecast);
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
            catalog,
            board,
            selected_zone: camp::CAMP_ZONE.to_string(),
        }
    }

    /// Roll a new forecast and re-post the board (called after every night).
    fn advance_to_next_night(&mut self) {
        self.forecast = roll_forecast(&mut self.rng);
        self.board = camp::generate_board(
            &self.catalog,
            self.business.night as u64 * 31 + 7,
            self.forecast,
        );
        self.persist();
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
                    egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Camp");
                    ui.label(format!("Night {}", self.business.night));
                    ui.label(format!("Cash: {}", da_econ::fmt_dollars(self.business.cash_cents)));
                    ui.separator();

                    ui.heading("Forecast");
                    ui.label(egui::RichText::new(format!("{:?}", self.forecast)).strong());
                    ui.label(self.forecast.blurb());
                    let m = self.forecast.mods();
                    ui.monospace(format!(
                        "thermal ×{:.2}  NV ×{:.2}  activity ×{:.2}\nbattery ×{:.2}  hazards ×{:.2}",
                        m.thermal_contrast, m.nv_visibility, m.pest_activity,
                        m.battery_drain, m.hazard_severity
                    ));
                    ui.separator();

                    ui.heading("Mount optic");
                    for opt in self.owned_optics() {
                        ui.radio_value(&mut self.mounted, opt, format!("{opt:?}"));
                    }
                    ui.separator();

                    let mut flash: Option<String> = None;
                    ui.collapsing("Store — rifles", |ui| {
                        for model in [
                            RifleModel::MultiPump,
                            RifleModel::UnregulatedPcp,
                            RifleModel::RegulatedTier2Variant,
                            RifleModel::RegulatedPcp,
                            RifleModel::Premium25,
                        ] {
                            let owned = self.business.owns(ItemKind::Rifle(model));
                            let label = format!(
                                "{} — {}",
                                model.name(),
                                da_econ::fmt_dollars(model.price_cents())
                            );
                            ui.horizontal(|ui| {
                                if owned {
                                    ui.label(format!("✓ {label}"));
                                } else {
                                    let mut btn = ui.button(&label);
                                    if let Some(w) = model.warning() {
                                        btn = btn.on_hover_text(w);
                                    }
                                    if btn.clicked() {
                                        if let Err(e) = self.business.buy_rifle(model) {
                                            flash = Some(format!("{e}"));
                                        }
                                    }
                                }
                            });
                        }
                        if ui
                            .button(format!(
                                "Regulator retrofit — {}",
                                da_econ::fmt_dollars(da_econ::store::REGULATOR_RETROFIT_CENTS)
                            ))
                            .on_hover_text("Turns an unregulated Tier 2 into a Tier 3. Cheaper than buying Tier 3 outright — unless you already bought the regulated Tier 2 variant.")
                            .clicked()
                        {
                            if let Err(e) = self.business.retrofit_regulator() {
                                flash = Some(format!("{e}"));
                            }
                        }
                    });

                    ui.collapsing("Store — optics", |ui| {
                        for model in [
                            OpticModel::Headlamp,
                            OpticModel::NvBasic,
                            OpticModel::NvPro,
                            OpticModel::ThermalMk1,
                            OpticModel::ThermalMk2,
                            OpticModel::ThermalMk3,
                        ] {
                            let owned = self.business.owns(ItemKind::Optic(model));
                            if owned {
                                ui.label(format!("✓ {}", model.name()));
                                continue;
                            }
                            let price = match (model.price_outright_cents(), model.upgrade_from()) {
                                (Some(p), _) => p,
                                (None, Some((_, up))) => up,
                                (None, None) => 0,
                            };
                            let label =
                                format!("{} — {}", model.name(), da_econ::fmt_dollars(price));
                            if ui.button(label).on_hover_text(model.tooltip()).clicked() {
                                let r = if model.price_outright_cents().is_none() {
                                    self.business.upgrade_optic(model)
                                } else {
                                    self.business.buy_optic(model)
                                };
                                if let Err(e) = r {
                                    flash = Some(format!("{e}"));
                                }
                            }
                        }
                    });

                    ui.collapsing("Store — licenses", |ui| {
                        for lic in [License::A, License::B, License::C, License::D] {
                            if self.business.has_license(lic) {
                                ui.label(format!("✓ License {lic:?}"));
                                continue;
                            }
                            let label = format!(
                                "License {lic:?} — {}",
                                da_econ::fmt_dollars(lic.price_cents())
                            );
                            if ui.button(label).on_hover_text(lic.tooltip()).clicked() {
                                if let Err(e) = self.business.buy_license(lic) {
                                    flash = Some(format!("{e}"));
                                }
                            }
                        }
                    });

                    ui.collapsing("Store — kit", |ui| {
                        for acc in [
                            Accessory::Moderator,
                            Accessory::BatteryPack,
                            Accessory::LargerTank,
                            Accessory::Bicycle,
                            Accessory::MatchedPelletTin,
                            Accessory::ScopeMagnification,
                            Accessory::PelletTin,
                        ] {
                            let owned = !acc.is_consumable()
                                && self.business.owns(ItemKind::Accessory(acc));
                            if owned {
                                ui.label(format!("✓ {}", acc.name()));
                                continue;
                            }
                            let label = format!(
                                "{} — {}",
                                acc.name(),
                                da_econ::fmt_dollars(acc.price_cents())
                            );
                            if ui.button(label).clicked() {
                                if let Err(e) = self.business.buy_accessory(acc) {
                                    flash = Some(format!("{e}"));
                                }
                            }
                        }
                    });

                    if let Some(msg) = flash {
                        self.hud_flash = Some((msg, 0.0));
                    } else {
                        self.persist();
                    }
                    });
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
                    ui.heading("DeadAir");
                    if let Some(st) = statement {
                        ui.group(|ui| {
                            ui.monospace(st.to_string());
                        });
                        ui.separator();
                    }

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("Contract board");
                        let visible: Vec<Contract> =
                            self.board.visible(&self.business).into_iter().cloned().collect();
                        if visible.is_empty() {
                            ui.label("No work you're licensed for. Buy a license.");
                        }
                        let mut accept: Option<Contract> = None;
                        egui::Grid::new("board").striped(true).show(ui, |ui| {
                            ui.label(egui::RichText::new("Client").strong());
                            ui.label(egui::RichText::new("Zone").strong());
                            ui.label(egui::RichText::new("Target").strong());
                            ui.label(egui::RichText::new("Quota").strong());
                            ui.label(egui::RichText::new("Deadline").strong());
                            ui.label(egui::RichText::new("Est. value").strong());
                            ui.label("");
                            ui.end_row();
                            for c in &visible {
                                ui.label(&c.client);
                                ui.label(&c.zone);
                                ui.label(format!("{:?}", c.species));
                                ui.label(format!("{}", c.quota));
                                ui.label(format!("{} nights", c.deadline_nights));
                                let ev = da_econ::expected_night_value(self.forecast, c);
                                ui.label(format!("${ev:.0}/night"));
                                if ui.button("Accept").clicked() {
                                    accept = Some(c.clone());
                                }
                                ui.end_row();
                            }
                        });
                        if let Some(c) = accept {
                            let id = c.id;
                            match self.business.accept_contract(c) {
                                Ok(()) => {
                                    self.board.take(id);
                                }
                                Err(e) => self.hud_flash = Some((format!("{e}"), 0.0)),
                            }
                        }

                        let active = self.business.contracts().to_vec();
                        if !active.is_empty() {
                            ui.separator();
                            ui.heading("Active contracts");
                            for c in &active {
                                ui.label(format!(
                                    "{} — {:?} {}/{} · {} nights left · {}",
                                    c.zone, c.species, c.progress, c.quota,
                                    c.deadline_nights, 
                                    match c.status {
                                        da_econ::ContractStatus::Accepted => "active",
                                        da_econ::ContractStatus::Completed => "done",
                                        da_econ::ContractStatus::Cancelled => "CANCELLED",
                                        da_econ::ContractStatus::Failed => "FAILED",
                                        da_econ::ContractStatus::Offered => "offered",
                                    }
                                ));
                            }
                        }

                        ui.separator();
                        ui.heading("Travel");
                        let bike = camp::has_bicycle(&self.business);
                        ui.label(if bike {
                            "Bicycle: travel time halved."
                        } else {
                            "On foot. A bicycle would halve travel time."
                        });
                        for z in &self.catalog.zones {
                            let frac = z.travel_fraction(10.0, bike);
                            let mins = if bike { z.walk_min / 2 } else { z.walk_min };
                            ui.radio_value(
                                &mut self.selected_zone,
                                z.name.clone(),
                                format!(
                                    "{} — {} min travel ({:.0}% of the night)",
                                    z.name, mins, frac * 100.0
                                ),
                            );
                        }

                        ui.separator();
                        ui.horizontal(|ui| {
                            let start = ui.add_sized(
                                [260.0, 44.0],
                                egui::Button::new(format!("🌙 Hunt {}", self.selected_zone)),
                            );
                            if start.clicked() {
                                let Some(z) = self.catalog.find(&self.selected_zone).cloned() else {
                                    self.hud_flash = Some(("unknown zone".into(), 0.0));
                                    return;
                                };
                                let seed = self.rng.next_u64();
                                match NightHunt::new(
                                    &zone_path(&z.file),
                                    self.forecast,
                                    &self.business,
                                    seed,
                                    self.mounted,
                                ) {
                                    Ok(mut h) => {
                                        // Travel burns night clock (SDD §3).
                                        let frac = z.travel_fraction(
                                            h.clock.night_hours,
                                            camp::has_bicycle(&self.business),
                                        );
                                        h.clock.seek(frac);
                                        if frac > 0.0 {
                                            h.log.push(format!(
                                                "Travelled to {} — {:.0}% of the night gone.",
                                                z.name,
                                                frac * 100.0
                                            ));
                                        }
                                        self.optic_mode = optic_mode_for(self.mounted);
                                        self.screen = Screen::Night(Box::new(h));
                                    }
                                    Err(e) => self.hud_flash = Some((e, 0.0)),
                                }
                            }
                            if ui.button("Skip night ($15 camp fee)").clicked() {
                                let st = self.business.skip_night();
                                self.screen = Screen::Camp { statement: Some(st) };
                                self.advance_to_next_night();
                            }
                            if self.business.is_bankrupt()
                                && ui.button("💀 New campaign").clicked()
                            {
                                self.business = Business::new();
                                self.screen = Screen::Camp { statement: None };
                                self.advance_to_next_night();
                            }
                        });
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
                        self.screen = Screen::Camp { statement: Some(statement) };
                        self.advance_to_next_night();
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
