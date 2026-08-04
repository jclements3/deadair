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
use deadair::aim;
use deadair::camp::{self, CampaignState, ZoneCatalog};
use deadair::tutorial::Tutorial;
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
    /// First-night tutorial (NFR-2). Retired once cleared.
    tutorial: Option<Tutorial>,
    /// Mirrors the viewport's fullscreen state (starts true).
    fullscreen: bool,
    /// Scope magnification, 1.0 (unaided) .. 14.5 (smart-scope max).
    mag: f32,
    /// Target locked with left-click, if still alive.
    selected: Option<da_core::EntityId>,
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
        let business_night = business.night;
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
            // Only night one gets taught.
            tutorial: (business_night == 1).then(Tutorial::new),
            fullscreen: true,
            mag: 1.0,
            selected: None,
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

        // A pinned spawn position (DEADAIR_POS) starts windowed on the
        // chosen monitor and fullscreens once the window has landed there.
        if self.frame == 3 && std::env::var("DEADAIR_POS").is_ok() {
            self.fullscreen = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        // Fullscreen is the default; F11 toggles it, Esc drops out of it
        // (a windowed Esc quits, so there's always a way out).
        let (toggle_fs, escape) = ctx.input(|i| {
            (i.key_pressed(egui::Key::F11), i.key_pressed(egui::Key::Escape))
        });
        if toggle_fs {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        } else if escape {
            if self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

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
                    ui.label(
                        "Controls: WASD move · middle-drag pans the sights\n\
                         scroll wheel zooms (1-14.5x) · left-click locks target\n\
                         RIGHT-CLICK FIRES · hold off with the mil scale",
                    );
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
                    ui.label(format!("Pellets: {}", self.business.pellets));
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
                        // Audio captions (NFR-3): the loudest few, so a deaf
                        // player still gets the channel thermal can't give.
                        for sub in h.subtitles.iter().take(3) {
                            ui.separator();
                            ui.colored_label(
                                egui::Color32::LIGHT_GRAY,
                                format!("🔊 {}", sub.to_line()),
                            );
                        }
                    }
                    Screen::Camp { .. } => {
                        ui.monospace(format!(
                            "CAMP | {} | night {} | forecast {:?}",
                            self.cash_str(),
                            self.business.night + 1,
                            self.forecast
                        ));
                        match camp::campaign_state(&self.business, &self.catalog) {
                            CampaignState::Bankrupt => {
                                ui.colored_label(
                                    egui::Color32::RED,
                                    "BANKRUPT — campaign over",
                                );
                            }
                            CampaignState::Won => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_GREEN,
                                    "EVERY ZONE CLEARED — the business made it",
                                );
                            }
                            CampaignState::Running => {}
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

                    if camp::campaign_state(&self.business, &self.catalog) == CampaignState::Won {
                        ui.group(|ui| {
                            ui.heading("Campaign complete");
                            ui.label(
                                "Every zone cleared with your reputation intact. The \
                                 contracts dry up, the nights get quiet, and the \
                                 business is yours to keep.",
                            );
                            ui.label(format!(
                                "Final balance: {} after {} nights.",
                                da_econ::fmt_dollars(self.business.cash_cents),
                                self.business.night
                            ));
                            if ui.button("Start a new campaign").clicked() {
                                self.business = Business::new();
                                self.screen = Screen::Camp { statement: None };
                                self.advance_to_next_night();
                            }
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
                            let has_rifle = self.business.best_rifle_tier() > 0;
                            let start = ui.add_enabled(
                                has_rifle,
                                egui::Button::new(format!("🌙 Hunt {}", self.selected_zone))
                                    .min_size(egui::vec2(260.0, 44.0)),
                            );
                            if !has_rifle {
                                ui.colored_label(
                                    egui::Color32::YELLOW,
                                    "You need a rifle. The multi-pump .22 is $200.",
                                );
                            }
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
                    // Advance simulation from input. WASD moves; the mouse
                    // scheme is scope-style: LMB selects, middle-drag pans,
                    // wheel zooms, RMB fires (design direction).
                    let (move_dir, scroll_y) = ui.input(|i| {
                        let mut d = Vec3::ZERO;
                        let flat = Vec3::new(yaw.sin(), 0.0, -yaw.cos()).normalize_or_zero();
                        let right = flat.cross(Vec3::Y);
                        if i.key_down(egui::Key::W) { d += flat; }
                        if i.key_down(egui::Key::S) { d -= flat; }
                        if i.key_down(egui::Key::A) { d -= right; }
                        if i.key_down(egui::Key::D) { d += right; }
                        (d.normalize_or_zero() * 4.0, i.raw_scroll_delta.y)
                    });
                    self.scoped = self.mag > 1.5;
                    h.tick(dt, move_dir, self.scoped);
                    if let Some(tut) = &mut self.tutorial {
                        let fired: Vec<_> = h.recent_events().to_vec();
                        let can_fire = h.sim.rifle.plant.can_fire();
                        if let Some(prompt) =
                            tut.update(self.scoped, can_fire, h.clock.t(), &fired)
                        {
                            h.log.push(prompt.to_string());
                        }
                        if tut.is_done() {
                            self.tutorial = None;
                        }
                    }

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

                    let fov = aim::fov_for_mag(self.mag);
                    let cam = Camera {
                        eye: h.sim.player.pos,
                        look: h.sim.player.pos + fwd,
                        up: Vec3::Y,
                        fov_y_deg: fov,
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

                    // Wheel zoom (over the view).
                    if resp.hovered() && scroll_y.abs() > 0.0 {
                        self.mag = (self.mag * (1.0 + scroll_y * 0.0015)).clamp(1.0, 14.5);
                    }
                    // Middle-drag pans — slower when zoomed, like a scope on
                    // sticks.
                    if resp.dragged_by(egui::PointerButton::Middle) {
                        let d = resp.drag_delta();
                        let sens = 0.004 * (fov / 60.0);
                        self.yaw += d.x * sens;
                        self.pitch = (self.pitch - d.y * sens).clamp(-1.4, 1.4);
                    }
                    // LMB: lock the target nearest the sights.
                    if resp.clicked_by(egui::PointerButton::Primary) {
                        let candidates: Vec<(usize, Vec3)> = h
                            .sim
                            .animals
                            .iter()
                            .enumerate()
                            .filter(|(_, a)| a.alive && a.is_targetable())
                            .map(|(i, a)| {
                                let head =
                                    h.head_of(a.id).unwrap_or(a.pos + Vec3::Y * 0.3);
                                (i, head)
                            })
                            .collect();
                        self.selected = aim::pick_nearest_axis(
                            h.sim.player.pos,
                            fwd,
                            &candidates,
                            120.0, // generous lock cone; alignment is the skill
                        )
                        .map(|i| h.sim.animals[i].id);
                    }
                    // RMB: take the shot — drop and wind applied for real.
                    if resp.clicked_by(egui::PointerButton::Secondary) {
                        if let Some(msg) = h.fire_axis(fwd, &self.business) {
                            self.hud_flash = Some((msg, 0.0));
                        }
                        if let Some(id) = self.selected {
                            if !h.sim.animals.iter().any(|a| a.id == id && a.alive) {
                                self.selected = None;
                            }
                        }
                    }

                    // ---- Reticle overlay: crosshair + mil scale axes ----
                    let rect = resp.rect;
                    let painter = ui.painter_at(rect);
                    let c = rect.center();
                    let ppm = aim::px_per_mil(side, fov);
                    let ret = egui::Color32::from_rgba_unmultiplied(255, 60, 60, 200);
                    let ret_dim = egui::Color32::from_rgba_unmultiplied(255, 60, 60, 110);
                    let stroke = egui::Stroke::new(1.0, ret);
                    // Crosshair with an open center.
                    for (a, b) in [
                        ((-side * 0.5, 0.0), (-8.0, 0.0)),
                        ((8.0, 0.0), (side * 0.5, 0.0)),
                        ((0.0, -side * 0.5), (0.0, -8.0)),
                        ((0.0, 8.0), (0.0, side * 0.5)),
                    ] {
                        painter.line_segment(
                            [c + egui::vec2(a.0, a.1), c + egui::vec2(b.0, b.1)],
                            stroke,
                        );
                    }
                    // Mil ticks on both axes (FFP: spacing follows zoom).
                    if ppm > 4.0 {
                        let max_mil = (side * 0.45 / ppm) as i32;
                        for m in 1..=max_mil {
                            let off = m as f32 * ppm;
                            let len = if m % 5 == 0 { 7.0 } else { 3.5 };
                            let col = if m % 5 == 0 { ret } else { ret_dim };
                            let st = egui::Stroke::new(1.0, col);
                            painter.line_segment(
                                [c + egui::vec2(off, -len), c + egui::vec2(off, len)],
                                st,
                            );
                            painter.line_segment(
                                [c + egui::vec2(-off, -len), c + egui::vec2(-off, len)],
                                st,
                            );
                            painter.line_segment(
                                [c + egui::vec2(-len, off), c + egui::vec2(len, off)],
                                st,
                            );
                            painter.line_segment(
                                [c + egui::vec2(-len, -off), c + egui::vec2(len, -off)],
                                st,
                            );
                        }
                    }

                    let has_lrf = self
                        .business
                        .owns(da_econ::ItemKind::Accessory(da_econ::Accessory::Rangefinder));
                    let sol = h.shot_solution(fwd);
                    if has_lrf {
                        // Holdover chevron: where the pellet actually lands.
                        // Put the target under THIS, not the crosshair.
                        let hold = c + egui::vec2(sol.drift_mil * ppm, sol.drop_mil * ppm);
                        painter.circle_stroke(
                            hold,
                            4.0,
                            egui::Stroke::new(1.5, egui::Color32::YELLOW),
                        );
                        painter.line_segment(
                            [hold + egui::vec2(-7.0, 0.0), hold + egui::vec2(7.0, 0.0)],
                            egui::Stroke::new(1.0, egui::Color32::YELLOW),
                        );
                        painter.text(
                            rect.right_top() + egui::vec2(-10.0, 26.0),
                            egui::Align2::RIGHT_TOP,
                            format!(
                                "RNG {:>5.1} m   DROP {:.1} mil   WIND {:+.1} mil",
                                sol.range_m, sol.drop_mil, sol.drift_mil
                            ),
                            egui::FontId::monospace(14.0),
                            egui::Color32::YELLOW,
                        );
                    }
                    // Wind is always felt, even without the LRF.
                    let wind = h.wind_mps;
                    let wind_txt = {
                        let flat = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
                        let right = flat.cross(Vec3::Y);
                        let x = wind.dot(right);
                        let z = wind.dot(flat);
                        let arrow = if x.abs() > z.abs() {
                            if x > 0.0 { "->" } else { "<-" }
                        } else if z > 0.0 {
                            "^"
                        } else {
                            "v"
                        };
                        format!("WIND {:.1} m/s {arrow}", wind.length())
                    };
                    painter.text(
                        rect.left_bottom() + egui::vec2(10.0, -10.0),
                        egui::Align2::LEFT_BOTTOM,
                        wind_txt,
                        egui::FontId::monospace(14.0),
                        egui::Color32::LIGHT_GRAY,
                    );
                    painter.text(
                        rect.left_top() + egui::vec2(10.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        format!("{:.1}x", self.mag),
                        egui::FontId::monospace(14.0),
                        egui::Color32::LIGHT_GRAY,
                    );

                    // Selection brackets around the locked target.
                    if let Some(id) = self.selected {
                        if let Some(a) = h.sim.animals.iter().find(|a| a.id == id && a.alive) {
                            let head = h.head_of(id).unwrap_or(a.pos + Vec3::Y * 0.3);
                            let clip = cam.view_proj() * head.extend(1.0);
                            if clip.w > 0.0 {
                                let ndc = clip / clip.w;
                                let px =
                                    c + egui::vec2(ndc.x * side * 0.5, -ndc.y * side * 0.5);
                                let r = 16.0;
                                let g = egui::Stroke::new(1.5, egui::Color32::LIGHT_GREEN);
                                for (dx, dy) in
                                    [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
                                {
                                    let corner = px + egui::vec2(dx * r, dy * r);
                                    painter.line_segment(
                                        [corner, corner - egui::vec2(dx * 6.0, 0.0)],
                                        g,
                                    );
                                    painter.line_segment(
                                        [corner, corner - egui::vec2(0.0, dy * 6.0)],
                                        g,
                                    );
                                }
                                if has_lrf {
                                    let d = head.distance(h.sim.player.pos);
                                    painter.text(
                                        px + egui::vec2(r + 4.0, -r),
                                        egui::Align2::LEFT_TOP,
                                        format!("{d:.0} m"),
                                        egui::FontId::monospace(12.0),
                                        egui::Color32::LIGHT_GREEN,
                                    );
                                }
                            }
                        } else {
                            self.selected = None;
                        }
                    }

                    // Night over → settle at camp.
                    if h.over {
                        // Unspent pellets come home with you.
                        self.business.pellets = h.pellets;
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
    // Stand near the crop rows and glass the rabbit field.
    let cam = Camera {
        eye: Vec3::new(45.0, 1.8, 110.0),
        look: Vec3::new(30.0, 0.3, 85.0),
        up: Vec3::Y,
        fov_y_deg: 30.0,
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

    // Multi-monitor control: fullscreen claims whatever monitor the window
    // spawns on, so window placement decides everything.
    //   - default: the last position is persisted, so Esc -> drag to the
    //     monitor you want -> F11 once, and every later launch lands there;
    //   - DEADAIR_POS="x,y" pins the spawn position explicitly (virtual
    //     desktop coordinates; overrides persistence for that run);
    //   - --windowed starts windowed for easy dragging.
    let windowed = std::env::args().any(|a| a == "--windowed");
    let forced_pos = std::env::var("DEADAIR_POS").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("DeadAir")
        // Night hunting wants the whole screen. F11 toggles, Esc leaves
        // fullscreen (and only quits from a window).
        .with_fullscreen(!windowed && forced_pos.is_none())
        .with_inner_size([VIEW as f32 + 320.0, VIEW as f32 + 96.0]);
    if let Some(pos) = forced_pos {
        viewport = viewport.with_position(pos);
    }
    let native = eframe::NativeOptions {
        viewport,
        persist_window: forced_pos.is_none(),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            // Under WSL2 the Intel adapter surfaces through GL/D3D12 and
            // loses the device on request; the Vulkan path (including
            // llvmpipe) works. Prefer Vulkan, and ask only for limits a
            // downlevel adapter can actually grant.
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
                        label: Some("deadair"),
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
    eframe::run_native("DeadAir", native, Box::new(|_| Ok(Box::new(App::new()))))
        .expect("eframe run");
}
