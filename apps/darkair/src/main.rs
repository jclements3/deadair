//! DarkAir — first-person night pest-control business sim.
//!
//! Window layout (per design direction): a square **1024×1024 first-person
//! view** top-left, the **controls column in the right remainder**, and the
//! **status strip below** the view. Camp screens replace the view between
//! nights.
//!
//! `darkair --shot out.png [--optic thermal] [--t 0.5]` renders one frame of
//! the real Home Farm zone headless and exits (verification without a
//! window).

use darkair::hunt;

use da_core::{Forecast, Rng};
use da_econ::{
    Accessory, Business, Contract, ContractBoard, ItemKind, License, OpticModel, PnLStatement,
    RifleModel,
};
use darkair::aim;
use darkair::camp3d;
use darkair::camp::{self, CampaignState, ZoneCatalog};
use darkair::range::RangeState;
use darkair::tutorial::Tutorial;
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

fn camp_source_path() -> String {
    let a = "assets/camp.zone.ron";
    if std::path::Path::new(a).exists() {
        a.to_string()
    } else {
        format!("../../{a}")
    }
}

fn zone_path(file: &str) -> String {
    // Run from repo root or apps/darkair.
    let a = format!("{ZONE_DIR}/{file}");
    if std::path::Path::new(&a).exists() {
        a
    } else {
        format!("../../{a}")
    }
}

/// Play the game, or edit the world it's built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Play,
    Edit,
}

/// In-panel zone source editor state (Edit mode).
struct ZoneEdit {
    path: String,
    text: String,
    status: String,
}

/// Which screen the player is on.
enum Screen {
    /// Calibration/test range — the default view for now: latency and
    /// shimmer are measured here before the night hunts depend on them.
    Range(Box<RangeState>),
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
    /// Pointer captured (locked + hidden) for mouse-look. Esc releases.
    captured: bool,
    /// ADS blend 0..1, eased over ~200 ms (spec).
    ads: f32,
    /// Hold-breath state (Shift damps sway, then overshoots).
    breath: aim::Breath,
    /// Wall-clock accumulator driving the sway Lissajous.
    sway_t: f32,
    /// Time since the last simulated sensor frame (refresh-rate hold).
    optic_accum: f32,
    /// Play the game vs edit the parametric world source.
    mode: AppMode,
    /// Edit-mode buffer for the selected zone's RON source.
    zone_edit: Option<ZoneEdit>,
    /// The camp as a walkable world (rebuilt on return from a night).
    camp_world: Option<Box<camp3d::CampWorld>>,
    /// Target locked with left-click, if still alive.
    selected: Option<da_core::EntityId>,
    /// `--demo`: the scripted showcase drives the viewport instead of the
    /// player. Esc ends it; Space skips a segment.
    demo: Option<darkair::demo::DemoDirector>,
}

fn save_path() -> std::path::PathBuf {
    std::env::var_os("DARKAIR_SAVE")
        .map(Into::into)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
            std::path::Path::new(&home).join(".darkair-save.ron")
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
            screen: Screen::Range(Box::new(RangeState::new())),
            forecast,
            mounted: Mounted::Headlamp,
            renderer: None,
            view_tex: None,
            // The range boots in white-hot thermal — the reference look.
            optic_mode: OpticMode::Thermal,
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
            captured: false,
            ads: 0.0,
            breath: aim::Breath::default(),
            sway_t: 0.0,
            optic_accum: 0.0,
            mode: AppMode::Play,
            zone_edit: None,
            camp_world: None,
            selected: None,
            demo: if std::env::args().any(|a| a == "--demo") {
                match darkair::demo::DemoDirector::new(&zones_dir(), &camp_source_path()) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        eprintln!("--demo: {e}");
                        None
                    }
                }
            } else {
                None
            },
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

/// The mounted device's native sensor square-side and refresh rate — the
/// numbers that make a Mk I *look and feel* different from a Mk III.
/// The unaided eye has no sensor (None) and no refresh cap.
fn sensor_for(mounted: Mounted) -> (Option<u32>, f32) {
    let spec = mounted.optic_model().spec();
    let res = spec.resolution.map(|(_, h)| h);
    let hz = if spec.refresh_hz == 0 {
        f32::INFINITY
    } else {
        spec.refresh_hz as f32
    };
    (res, hz)
}

fn optic_mode_for(mounted: Mounted) -> OpticMode {
    match mounted {
        Mounted::Headlamp => OpticMode::Eye,
        Mounted::NvBasic | Mounted::NvPro => OpticMode::Nv,
        Mounted::Thermal(_) => OpticMode::Thermal,
    }
}


impl App {
    /// Grab or release the pointer for mouse-look (spec: Locked + hidden;
    /// Confined is winit's own fallback path on platforms without Locked).
    fn set_captured(&mut self, ctx: &egui::Context, captured: bool) {
        if self.captured == captured {
            return;
        }
        self.captured = captured;
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(if captured {
            // Confined, not Locked: X11 has no Locked (winit errors, egui
            // only logs), and raw look doesn't need it — Confined merely
            // keeps the hidden cursor inside the window on every platform.
            egui::CursorGrab::Confined
        } else {
            egui::CursorGrab::None
        }));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!captured));
    }

    /// Raw look deltas this frame: `Event::MouseMoved` is winit's
    /// `DeviceEvent::MouseMotion` (unaccelerated, unclamped), which egui
    /// forwards while the cursor is locked. Summed per frame, applied
    /// per-event magnitude — never scaled by delta-time (mouse input is
    /// displacement, not velocity).
    fn raw_look_delta(ctx: &egui::Context) -> egui::Vec2 {
        let (raw, cursor) = ctx.input(|i| {
            let raw = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseMoved(d) => Some(*d),
                    _ => None,
                })
                .fold(egui::Vec2::ZERO, |a, d| a + d);
            (raw, i.pointer.delta())
        });
        // Belt and braces: a compositor with no raw-motion support delivers
        // only CursorMoved. Use its delta when raw is silent so look still
        // works (the cursor can stall at a window edge — the raw path never
        // has that problem, which is why it's primary).
        let d = if raw != egui::Vec2::ZERO { raw } else { cursor };
        if std::env::var_os("DARKAIR_INPUT_DEBUG").is_some() && d != egui::Vec2::ZERO {
            eprintln!("look raw=({:.1},{:.1}) cursor=({:.1},{:.1})", raw.x, raw.y, cursor.x, cursor.y);
        }
        d
    }

    /// Begin the night in `self.selected_zone`: travel burns clock, the
    /// camp world is dropped for a fresh rebuild on return.
    fn start_selected_night(&mut self) {
        if self.business.best_rifle_tier() == 0 {
            self.hud_flash =
                Some(("You need a rifle. The multi-pump .22 is $200.".into(), 0.0));
            return;
        }
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
                let frac =
                    z.travel_fraction(h.clock.night_hours, camp::has_bicycle(&self.business));
                h.clock.seek(frac);
                if frac > 0.0 {
                    h.log.push(format!(
                        "Travelled to {} — {:.0}% of the night gone.",
                        z.name,
                        frac * 100.0
                    ));
                }
                self.optic_mode = optic_mode_for(self.mounted);
                self.camp_world = None;
                self.screen = Screen::Night(Box::new(h));
            }
            Err(e) => self.hud_flash = Some((e, 0.0)),
        }
    }

    /// Status line + mode switch + the active mode's content. This is the
    /// whole non-viewport UI, hosted right (landscape) or below (portrait).
    fn panel_region(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("D A R K A I R")
                    .size(20.0)
                    .strong()
                    .color(egui::Color32::from_rgb(112, 230, 140)),
            );
            ui.label(
                egui::RichText::new("night contracting")
                    .size(11.0)
                    .italics()
                    .color(egui::Color32::from_rgb(110, 125, 112)),
            );
        });
        ui.add_space(2.0);
        self.status_ui(ui);
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, AppMode::Play, "▶ Play");
            ui.selectable_value(&mut self.mode, AppMode::Edit, "✎ Edit");
            ui.separator();
            if ui.button("⛶ Fullscreen").on_hover_text("F11 or Alt+Enter").clicked() {
                self.fullscreen = !self.fullscreen;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            if ui.button("🗖 Maximize").on_hover_text("F10 — WSLg's title bar doesn't forward double-click").clicked() {
                let is_max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
            }
        });
        ui.separator();
        match self.mode {
            AppMode::Play => {
                egui::ScrollArea::vertical()
                    .id_salt("panel-play")
                    .show(ui, |ui| self.play_panel(ui));
            }
            AppMode::Edit => {
                egui::ScrollArea::vertical()
                    .id_salt("panel-edit")
                    .show(ui, |ui| self.edit_panel(ui));
            }
        }
    }

    /// Second column for the portrait layout: the field log at night, the
    /// forecast briefing at camp.
    fn panel_aux(&mut self, ui: &mut egui::Ui) {
        match &self.screen {
            Screen::Range(_) => {
                ui.heading("What to watch");
                ui.label(
                    "• Checkerboards at 10/25/50/75/100 m: crawling or \
                     boiling edges while zooming = shimmer.\n\
                     • Picket fence at 30 m: thin members flicker first.\n\
                     • Rabbits: hops should track 1:1 with your mouse — any \
                     rubber-banding is latency.\n\
                     • Click-flash: film screen + mouse together; count \
                     frames from button-down to the white square.",
                );
            }
            Screen::Night(h) => {
                ui.heading("Field log");
                egui::ScrollArea::vertical().id_salt("aux-log").show(ui, |ui| {
                    for line in h.log.iter().rev().take(20) {
                        ui.label(line);
                    }
                });
            }
            Screen::Camp { .. } => {
                ui.heading("Campaign");
                let cleared = self
                    .catalog
                    .zones
                    .iter()
                    .filter(|z| {
                        self.business.contracts().iter().any(|c| {
                            c.zone == z.name
                                && c.status == da_econ::ContractStatus::Completed
                        })
                    })
                    .count();
                ui.label(format!(
                    "Zones cleared: {cleared}/{} — clear all six with your \
                     reputation intact to win.",
                    self.catalog.zones.len()
                ));
                ui.separator();
                ui.heading("Active contracts");
                let active = self.business.contracts();
                if active.is_empty() {
                    ui.label("None. Take work off the board.");
                }
                for c in active {
                    ui.label(format!(
                        "{} — {:?} {}/{} · {} nights left",
                        c.zone, c.species, c.progress, c.quota, c.deadline_nights
                    ));
                }
            }
        }
    }

    /// The persistent status readout (was the bottom strip).
    fn status_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.captured {
                ui.colored_label(egui::Color32::LIGHT_GREEN, "🖱 MOUSE: GAME LOOK");
                ui.label("Tab or Esc frees the cursor");
            } else {
                ui.colored_label(egui::Color32::YELLOW, "🖱 MOUSE: FREE");
                ui.label("Tab or click the view to take aim");
            }
        });
        ui.separator();
        match &self.screen {
                    Screen::Range(r) => {
                        let (avg, p95, max) = r.stats.summary_ms();
                        ui.monospace(format!(
                            "RANGE | {:>5.1} fps | avg {avg:.1} ms | p95 {p95:.1} | worst {max:.1} | mag {:.1}x",
                            if avg > 0.0 { 1000.0 / avg } else { 0.0 },
                            self.mag,
                        ));
                    }
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
                            self.business.night,
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
    }

    /// Play mode: loadout at night, store/mounting at camp.
    fn play_panel(&mut self, ui: &mut egui::Ui) {
        match &mut self.screen {
            Screen::Range(r) => {
                ui.heading("Calibration range");
                ui.label("Latency + shimmer testing. This is the default view for now.");
                ui.separator();
                ui.add(
                    egui::Slider::new(&mut r.rabbit_speed, 0.0..=15.0)
                        .text("rabbit speed m/s"),
                );
                ui.add(
                    egui::Slider::new(&mut r.rabbit_count, 1..=darkair::range::MAX_RABBITS)
                        .text("rabbits (stress)"),
                );
                ui.checkbox(&mut r.auto_zoom, "auto zoom sweep (2x–14.5x)");
                ui.checkbox(&mut r.paused, "freeze scene");
                ui.checkbox(&mut r.sway_enabled, "reticle sway");
                ui.separator();
                ui.label("Optic pipeline:");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.optic_mode, OpticMode::Eye, "Eye");
                    ui.selectable_value(&mut self.optic_mode, OpticMode::Nv, "NV");
                    ui.selectable_value(&mut self.optic_mode, OpticMode::Thermal, "Thermal");
                });
                if self.optic_mode == OpticMode::Thermal {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.palette, ThermalPalette::WhiteHot, "White-hot");
                        ui.selectable_value(&mut self.palette, ThermalPalette::BlackHot, "Black-hot");
                        ui.selectable_value(&mut self.palette, ThermalPalette::ColorblindSafe, "CB-safe");
                    });
                }
                ui.separator();
                // Frame-time sparkline: 240 frames, 16.6 ms line marked.
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width().min(360.0), 56.0),
                    egui::Sense::hover(),
                );
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 3.0, egui::Color32::from_gray(18));
                let n = r.stats.recent().count().max(1);
                let scale_ms = 40.0; // full height = 40 ms
                let y16 = rect.bottom() - (16.6 / scale_ms) * rect.height();
                p.line_segment(
                    [egui::pos2(rect.left(), y16), egui::pos2(rect.right(), y16)],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 90, 40)),
                );
                let pts: Vec<egui::Pos2> = r
                    .stats
                    .recent()
                    .enumerate()
                    .map(|(i, dt)| {
                        let x = rect.left()
                            + rect.width() * (i as f32 / (n.max(2) - 1) as f32);
                        let y = rect.bottom()
                            - ((dt * 1000.0) / scale_ms).clamp(0.0, 1.0) * rect.height();
                        egui::pos2(x, y)
                    })
                    .collect();
                if pts.len() >= 2 {
                    p.add(egui::Shape::line(
                        pts,
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 255, 150)),
                    ));
                }
                ui.separator();
                if ui.button("⛺ Enter camp (start the game)").clicked() {
                    self.screen = Screen::Camp { statement: None };
                }
            }
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
                        "Click the view to take the field (Esc/Tab frees the mouse).\n\
                         WASD move · mouse looks — the wide view is your walk-around\n\
                         LEFT-CLICK near an animal LOCKS it and glasses up the scope\n\
                         LEFT-CLICK again FIRES · Q lowers the rifle · wheel = zoom\n\
                         hold RIGHT MOUSE to glass without locking\n\
                         SHIFT holds breath · hold off with the mil scale",
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
                Screen::Camp { statement } => {
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
                    ui.separator();
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
        }
    }

    /// Edit mode: the zone's parametric RON source, editable in place.
    /// Text is ground truth — re-expansion rebuilds the world from it.
    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        if matches!(self.screen, Screen::Night(_)) {
            ui.label("Finish (or abandon) the night to edit zones — the world \
                      can't be rebuilt under your feet.");
            return;
        }
        let zone_name = self.selected_zone.clone();
        let Some(entry) = self.catalog.find(&zone_name).cloned() else {
            ui.label("Select a zone in the travel list first (Play mode).");
            return;
        };
        let path = zone_path(&entry.file);
        let stale = self.zone_edit.as_ref().map(|z| z.path != path).unwrap_or(true);
        if stale {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    self.zone_edit = Some(ZoneEdit {
                        path: path.clone(),
                        text,
                        status: String::new(),
                    });
                }
                Err(e) => {
                    ui.colored_label(egui::Color32::RED, format!("read {path}: {e}"));
                    return;
                }
            }
        }
        let Some(edit) = self.zone_edit.as_mut() else { return };
        ui.label(format!("{} — source is ground truth", entry.name));
        ui.small(&edit.path);
        egui::ScrollArea::vertical()
            .id_salt("zone-src")
            .max_height(ui.available_height() - 70.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut edit.text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(24),
                );
            });
        ui.horizontal(|ui| {
            if ui.button("Check & re-expand").clicked() {
                edit.status = match da_param::parse_zone_str(&edit.text)
                    .and_then(|src| da_param::expand_zone(&src).map(|x| (src, x)))
                {
                    Ok((src, x)) => format!(
                        "OK: \"{}\" expands to {} nodes, {} spawn points",
                        src.name,
                        x.scene.len(),
                        x.spawn_points.len()
                    ),
                    Err(e) => format!("ERROR: {e}"),
                };
            }
            if ui.button("Save").clicked() {
                edit.status = match std::fs::write(&edit.path, &edit.text) {
                    Ok(()) => {
                        // The world changed: rebuild catalog + board from text.
                        match ZoneCatalog::load(&zones_dir()) {
                            Ok(cat) => {
                                self.catalog = cat;
                                self.board = camp::generate_board(
                                    &self.catalog,
                                    self.business.night as u64 * 31 + 7,
                                    self.forecast,
                                );
                                "Saved. Catalog and contract board rebuilt.".into()
                            }
                            Err(e) => format!("Saved, but catalog reload failed: {e}"),
                        }
                    }
                    Err(e) => format!("write failed: {e}"),
                };
            }
        });
        if !edit.status.is_empty() {
            let col = if edit.status.starts_with("ERROR") || edit.status.contains("failed")
            {
                egui::Color32::RED
            } else {
                egui::Color32::LIGHT_GREEN
            };
            ui.colored_label(col, &edit.status);
        }
    }
}


/// The one line that prevents "why is it inverted": what you're looking
/// through, painted on the view itself.
fn instrument_label(mode: OpticMode, palette: ThermalPalette, sensor: Option<u32>) -> String {
    match mode {
        OpticMode::Eye => "EYE — unaided".to_string(),
        OpticMode::Nv => format!(
            "NV{} — animals read DARK, eyeshine bright",
            sensor.map(|s| format!(" {s}p")).unwrap_or_default()
        ),
        OpticMode::Thermal => {
            let pal = match palette {
                ThermalPalette::WhiteHot => "WHITE-HOT — warm = bright",
                ThermalPalette::BlackHot => "BLACK-HOT — warm = dark (inverted on purpose)",
                ThermalPalette::ColorblindSafe => "CB-SAFE — warm = bright",
            };
            format!(
                "THERMAL{} · {}",
                sensor.map(|s| format!(" {s}")).unwrap_or_default(),
                pal
            )
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint(); // real-time game
        self.frame = self.frame.wrapping_add(1);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);

        // A pinned spawn position (DARKAIR_POS) starts windowed on the
        // chosen monitor and fullscreens once the window has landed there.
        if self.frame == 3 && std::env::var("DARKAIR_POS").is_ok() {
            self.fullscreen = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        // Fullscreen is the default; F11 toggles it, Esc drops out of it
        // (a windowed Esc quits, so there's always a way out).
        let (toggle_fs, toggle_max, escape) = ctx.input(|i| {
            (
                // F11, or the classic Alt+Enter.
                i.key_pressed(egui::Key::F11)
                    || (i.modifiers.alt && i.key_pressed(egui::Key::Enter)),
                // F10 = OS maximize — WSLg's title bar doesn't forward
                // double-click-to-maximize to X11 apps, so give it a key.
                i.key_pressed(egui::Key::F10),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if toggle_max {
            let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
        }
        // Tab: explicit mouse-mode toggle — game-look vs free cursor.
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            let now = !self.captured;
            self.set_captured(ctx, now);
        }
        if toggle_fs {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        } else if escape {
            if self.captured {
                // First Esc = pause: give the pointer back for the panel.
                self.set_captured(ctx, false);
            } else if self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // ---- The panel: everything that isn't the viewport ---------------
        // Landscape: viewport left, panel fills the right remainder.
        // Portrait: viewport top, panel fills the bottom remainder.
        let screen_rect = ctx.screen_rect();
        let landscape = screen_rect.width() >= screen_rect.height();
        if landscape {
            // The panel owns ALL pixels right of the viewport; when that
            // remainder is wide (ultrawide monitors), split it in two.
            let w = (screen_rect.width() - VIEW as f32 - 16.0).max(240.0);
            egui::SidePanel::right("panel")
                .resizable(false)
                .exact_width(w)
                .show(ctx, |ui| {
                    if w > 700.0 {
                        ui.columns(2, |cols| {
                            self.panel_region(&mut cols[0]);
                            self.panel_aux(&mut cols[1]);
                        });
                    } else {
                        self.panel_region(ui);
                    }
                });
        } else {
            let hpx = (screen_rect.height() - VIEW as f32 - 8.0).clamp(160.0, 900.0);
            egui::TopBottomPanel::bottom("panel")
                .resizable(false)
                .exact_height(hpx)
                .show(ctx, |ui| {
                    // Wide-and-short region: split into two columns so the
                    // pixels under the viewport actually get used.
                    ui.columns(2, |cols| {
                        self.panel_region(&mut cols[0]);
                        self.panel_aux(&mut cols[1]);
                    });
                });
        }

        // ---- Central: the 1024×1024 first-person view --------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            // `--demo`: the director owns the viewport until it finishes
            // (or Esc). Space skips a segment. The panels stay live so a
            // conference audience sees the real shell around the reel.
            if let Some(mut director) = self.demo.take() {
                if ui.input(|i| i.key_pressed(egui::Key::Space)) {
                    director.skip();
                }
                let rs = frame.wgpu_render_state().expect("wgpu backend");
                if self.renderer.is_none() {
                    let rd = Renderer::new_on(&rs.device, VIEW, VIEW);
                    let id = rs.renderer.write().register_native_texture(
                        &rs.device,
                        &rd.output_view(),
                        wgpu::FilterMode::Nearest,
                    );
                    self.renderer = Some(rd);
                    self.view_tex = Some(id);
                }
                match director.advance(dt) {
                    Some(f) => {
                        director.refresh_pending_hunts(&zones_dir());
                        let renderer = self.renderer.as_mut().expect("set above");
                        renderer.render_on(
                            &rs.device, &rs.queue, &f.list, &f.cam, &f.settings, dt,
                        );
                        let avail = ui.available_size();
                        let side = (VIEW as f32).min(avail.x).min(avail.y);
                        let resp = ui.add(egui::Image::new((
                            self.view_tex.expect("registered"),
                            egui::vec2(side, side),
                        )));
                        let rect = resp.rect;
                        ui.painter().rect_stroke(
                            rect.expand(2.0),
                            0.0,
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                        );
                        // Captions over the lower third, promo-style.
                        for (row, cap) in f.captions.iter().enumerate() {
                            let pos = egui::pos2(
                                rect.center().x,
                                rect.bottom() - 120.0 + row as f32 * 34.0,
                            );
                            let galley_pos = pos;
                            ui.painter().text(
                                galley_pos,
                                egui::Align2::CENTER_CENTER,
                                cap,
                                egui::FontId::monospace(22.0),
                                egui::Color32::from_rgb(204, 255, 204),
                            );
                        }
                        if f.mag > 1.2 {
                            ui.painter().text(
                                rect.left_top() + egui::vec2(16.0, 16.0),
                                egui::Align2::LEFT_TOP,
                                format!("{:.1}x", f.mag),
                                egui::FontId::monospace(18.0),
                                egui::Color32::LIGHT_GRAY,
                            );
                        }
                        self.demo = Some(director);
                    }
                    None => {
                        // Reel over: fall back to the normal app next frame.
                        self.demo = None;
                    }
                }
                return;
            }
            match &mut self.screen {
                Screen::Range(r) => {
                    // Same input grammar as the hunt, no economy attached.
                    let (rmb_down, lmb_pressed, shift_down, scroll_y) = ui.input(|i| {
                        (
                            i.pointer.secondary_down(),
                            i.pointer.button_pressed(egui::PointerButton::Primary),
                            i.modifiers.shift,
                            i.raw_scroll_delta.y,
                        )
                    });
                    let ads_target = if rmb_down && self.captured { 1.0 } else { 0.0 };
                    self.ads += (ads_target - self.ads) * (dt / 0.2).clamp(0.0, 1.0);
                    self.scoped = self.ads > 0.5;
                    if r.auto_zoom {
                        self.mag = r.sweep_mag();
                    } else if self.captured && self.ads > 0.3 && scroll_y.abs() > 0.0 {
                        self.mag = (self.mag * (1.0 + scroll_y * 0.0015)).clamp(2.0, 14.5);
                    }
                    self.sway_t += dt;
                    self.breath.update(dt, shift_down && self.scoped);
                    let sway_amp = if r.sway_enabled {
                        aim::SWAY_BASE_RAD
                            * self.ads
                            * self.breath.sway_factor(shift_down && self.scoped)
                    } else {
                        0.0
                    };
                    let sway = aim::sway_offset(self.sway_t, 11, sway_amp);
                    let fov = {
                        let hip = 60.0;
                        let scoped_fov = aim::fov_for_mag(self.mag.max(2.0));
                        hip + (scoped_fov - hip) * self.ads
                    };
                    if self.captured {
                        let sens = 0.0022 * aim::ads_sensitivity_scale(fov, 1.0);
                        let d = Self::raw_look_delta(ui.ctx());
                        self.yaw = (self.yaw + d.x * sens).rem_euclid(std::f32::consts::TAU);
                        self.pitch = (self.pitch - d.y * sens).clamp(
                            -(std::f32::consts::FRAC_PI_2 - 0.001),
                            std::f32::consts::FRAC_PI_2 - 0.001,
                        );
                    }
                    let (eyaw, epitch) =
                        (self.yaw + sway.x, (self.pitch + sway.y).clamp(-1.5, 1.5));
                    let fwd = Vec3::new(
                        eyaw.sin() * epitch.cos(),
                        epitch.sin(),
                        -eyaw.cos() * epitch.cos(),
                    );

                    r.tick(dt);

                    let rs = frame.wgpu_render_state().expect("wgpu backend");
                    if self.renderer.is_none() {
                        let rd = Renderer::new_on(&rs.device, VIEW, VIEW);
                        let id = rs.renderer.write().register_native_texture(
                            &rs.device,
                            &rd.output_view(),
                            wgpu::FilterMode::Nearest,
                        );
                        self.renderer = Some(rd);
                        self.view_tex = Some(id);
                    }
                    let renderer = self.renderer.as_mut().expect("set above");
                    let eye = Vec3::new(0.0, 1.6, 8.0);
                    let cam = Camera {
                        eye,
                        look: eye + fwd,
                        up: Vec3::Y,
                        fov_y_deg: fov,
                        aspect: 1.0,
                    };
                    let settings = OpticSettings {
                        // Calibration view: the selected pipeline applies
                        // whether or not you're glassed — you're here to
                        // LOOK at it. (The hunt gates optics honestly.)
                        mode: self.optic_mode,
                        palette: self.palette,
                        // Stellar-class sim: a tube scope's view is ALWAYS
                        // the round eyepiece — that's the reference video.
                        scope_mask: self.scoped || self.optic_mode == OpticMode::Thermal,
                        frame: self.frame,
                        seed: 11,
                        sensor_res: match (self.optic_mode, self.palette) {
                            (OpticMode::Thermal, _) => Some(288),
                            (OpticMode::Nv, _) => Some(720),
                            _ => None,
                        },
                        ..Default::default()
                    };
                    let list = r.draw_list();
                    renderer.render_on(&rs.device, &rs.queue, &list, &cam, &settings, dt);

                    let avail = ui.available_size();
                    let side = (VIEW as f32).min(avail.x).min(avail.y);
                    let pad = if ui.ctx().screen_rect().width()
                        >= ui.ctx().screen_rect().height()
                    {
                        0.0
                    } else {
                        ((avail.x - side) * 0.5).max(0.0)
                    };
                    let resp = ui
                        .horizontal(|ui| {
                            ui.add_space(pad);
                            ui.add(
                                egui::Image::new((
                                    self.view_tex.expect("registered"),
                                    egui::vec2(side, side),
                                ))
                                .sense(egui::Sense::click_and_drag()),
                            )
                        })
                        .inner;
                    ui.painter().rect_stroke(
                        resp.rect.expand(2.0),
                        0.0,
                        egui::Stroke::new(2.0, egui::Color32::WHITE),
                    );

                    if lmb_pressed {
                        if !self.captured && resp.hovered() {
                            self.captured = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CursorGrab(
                                egui::CursorGrab::Confined,
                            ));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                        } else if self.captured {
                            // Click-flash latency marker + counter tick.
                            r.flash_frames = 3;
                            r.shots += 1;
                        }
                    }
                    if resp.dragged_by(egui::PointerButton::Middle) {
                        let d = resp.drag_delta();
                        let sens = 0.004 * (fov / 60.0);
                        self.yaw += d.x * sens;
                        self.pitch = (self.pitch - d.y * sens).clamp(-1.4, 1.4);
                    }

                    // Reticle + flash overlay.
                    let rect = resp.rect;
                    let painter = ui.painter_at(rect);
                    let c = rect.center();
                    let ppm = aim::px_per_mil(side, fov);
                    let ret = egui::Color32::from_rgba_unmultiplied(255, 60, 60, 200);
                    for (a, b) in [
                        ((-side * 0.5, 0.0), (-8.0, 0.0)),
                        ((8.0, 0.0), (side * 0.5, 0.0)),
                        ((0.0, -side * 0.5), (0.0, -8.0)),
                        ((0.0, 8.0), (0.0, side * 0.5)),
                    ] {
                        painter.line_segment(
                            [c + egui::vec2(a.0, a.1), c + egui::vec2(b.0, b.1)],
                            egui::Stroke::new(1.0, ret),
                        );
                    }
                    if ppm > 4.0 {
                        let max_mil = (side * 0.45 / ppm) as i32;
                        for m in 1..=max_mil {
                            let off = m as f32 * ppm;
                            let len = if m % 5 == 0 { 7.0 } else { 3.5 };
                            for (dx, dy) in
                                [(off, 0.0), (-off, 0.0), (0.0, off), (0.0, -off)]
                            {
                                let (ex, ey) =
                                    if dy == 0.0 { (0.0, len) } else { (len, 0.0) };
                                painter.line_segment(
                                    [
                                        c + egui::vec2(dx - ex, dy - ey),
                                        c + egui::vec2(dx + ex, dy + ey),
                                    ],
                                    egui::Stroke::new(1.0, ret),
                                );
                            }
                        }
                    }
                    painter.text(
                        rect.right_bottom() + egui::vec2(-10.0, -10.0),
                        egui::Align2::RIGHT_BOTTOM,
                        instrument_label(settings.mode, self.palette, settings.sensor_res),
                        egui::FontId::monospace(13.0),
                        egui::Color32::from_rgb(255, 210, 90),
                    );
                    if settings.mode == OpticMode::Thermal {
                        // Stellar-class HUD, laid out like the reference
                        // video: counter box top-center, info box top-right
                        // (zero profile / clock / live drop). Layout homage
                        // only — no real maker's marks.
                        let box_bg = egui::Color32::from_rgba_unmultiplied(10, 10, 10, 210);
                        let fg = egui::Color32::from_gray(235);
                        // Kill-counter box.
                        let cbox = egui::Rect::from_center_size(
                            egui::pos2(c.x, rect.top() + 30.0),
                            egui::vec2(84.0, 34.0),
                        );
                        painter.rect_filled(cbox, 3.0, box_bg);
                        painter.rect_stroke(
                            cbox,
                            3.0,
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 220, 130)),
                        );
                        painter.text(
                            cbox.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("{:03}", r.shots),
                            egui::FontId::monospace(22.0),
                            fg,
                        );
                        // Info box: zero profile, session clock, live drop.
                        let sol = aim::solve(
                            Vec3::new(0.0, 1.6, 8.0),
                            fwd,
                            {
                                // Range to the ground under the reticle.
                                if fwd.y < -1e-3 { (1.6 / -fwd.y).min(300.0) } else { 120.0 }
                            },
                            250.0,
                            1.0,
                            Vec3::ZERO,
                        );
                        let mins = (r.session_t() / 60.0) as u32;
                        let secs = (r.session_t() % 60.0) as u32;
                        let info = format!(
                            "A1-40m\n{mins:02}:{secs:02}\nDrop: {:.1}cm",
                            sol.drop_m * 100.0
                        );
                        let ibox = egui::Rect::from_min_size(
                            egui::pos2(rect.right() - 150.0, rect.top() + 12.0),
                            egui::vec2(138.0, 66.0),
                        );
                        painter.rect_filled(ibox, 3.0, box_bg);
                        painter.text(
                            ibox.left_top() + egui::vec2(8.0, 6.0),
                            egui::Align2::LEFT_TOP,
                            info,
                            egui::FontId::monospace(14.0),
                            fg,
                        );
                    }
                    if r.flash_frames > 0 {
                        r.flash_frames -= 1;
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                rect.left_top() + egui::vec2(8.0, 8.0),
                                egui::vec2(90.0, 90.0),
                            ),
                            0.0,
                            egui::Color32::WHITE,
                        );
                    }
                    painter.text(
                        rect.left_top() + egui::vec2(10.0, 106.0),
                        egui::Align2::LEFT_TOP,
                        format!("{:.1}x", self.mag),
                        egui::FontId::monospace(14.0),
                        egui::Color32::LIGHT_GRAY,
                    );
                }
                Screen::Camp { .. } => {
                    // The camp is a place, not a form (FPS rule: the
                    // viewport is always your perspective).
                    if self.camp_world.is_none() {
                        match camp3d::CampWorld::new(
                            &camp_source_path(),
                            &self.business,
                            &self.catalog,
                        ) {
                            Ok(w) => self.camp_world = Some(Box::new(w)),
                            Err(e) => {
                                ui.colored_label(egui::Color32::RED, format!("camp: {e}"));
                                return;
                            }
                        }
                    }

                    let (yaw, pitch) = (self.yaw, self.pitch);
                    let move_dir = ui.input(|i| {
                        let mut d = Vec3::ZERO;
                        let flat = Vec3::new(yaw.sin(), 0.0, -yaw.cos()).normalize_or_zero();
                        let right = flat.cross(Vec3::Y);
                        if i.key_down(egui::Key::W) { d += flat; }
                        if i.key_down(egui::Key::S) { d -= flat; }
                        if i.key_down(egui::Key::A) { d -= right; }
                        if i.key_down(egui::Key::D) { d += right; }
                        d.normalize_or_zero() * 3.0
                    });
                    // Mouse-look while captured (same feel as the field).
                    if self.captured {
                        let d = Self::raw_look_delta(ui.ctx());
                        let sens = 0.0022;
                        self.yaw = (self.yaw + d.x * sens).rem_euclid(std::f32::consts::TAU);
                        self.pitch = (self.pitch - d.y * sens).clamp(
                            -(std::f32::consts::FRAC_PI_2 - 0.001),
                            std::f32::consts::FRAC_PI_2 - 0.001,
                        );
                    }
                    let fwd = {
                        let (yaw, pitch) = (self.yaw, self.pitch);
                        Vec3::new(
                            yaw.sin() * pitch.cos(),
                            pitch.sin(),
                            -yaw.cos() * pitch.cos(),
                        )
                    };

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

                    let mut pending: Option<camp3d::CampAction> = None;
                    let mut gaze_label: Option<(Vec3, String, String, bool)> = None;
                    {
                        // Restock each frame so panel purchases appear on
                        // the rack immediately (it's a few dozen items).
                        let biz = &self.business;
                        let cat = &self.catalog;
                        let world = self.camp_world.as_mut().expect("created above");
                        world.restock(biz, cat);
                        world.walk(move_dir, dt);
                        let cam = Camera {
                            eye: world.eye(),
                            look: world.eye() + fwd,
                            up: Vec3::Y,
                            fov_y_deg: 60.0,
                            aspect: 1.0,
                        };
                        let list = world.draw_list(&self.business);
                        let renderer = self.renderer.as_mut().expect("set above");
                        let settings = OpticSettings {
                            mode: OpticMode::Eye,
                            scope_mask: false,
                            frame: self.frame,
                            eye_exposure: 2.2, // home is lit; the dark starts at the fence
                            ..Default::default()
                        };
                        renderer.render_on(&rs.device, &rs.queue, &list, &cam, &settings, dt);

                        let avail = ui.available_size();
                        let side = (VIEW as f32).min(avail.x).min(avail.y);
                        let resp = ui.add(
                            egui::Image::new((
                                self.view_tex.expect("registered"),
                                egui::vec2(side, side),
                            ))
                            .sense(egui::Sense::click_and_drag()),
                        );
                        ui.painter().rect_stroke(
                            resp.rect.expand(2.0),
                            0.0,
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                        );
                        if resp.dragged_by(egui::PointerButton::Middle) {
                            let d = resp.drag_delta();
                            self.yaw += d.x * 0.004;
                            self.pitch = (self.pitch - d.y * 0.004).clamp(-1.4, 1.4);
                        }
                        if !self.captured
                            && resp.hovered()
                            && ui.input(|i| {
                                i.pointer.button_pressed(egui::PointerButton::Primary)
                            })
                        {
                            self.captured = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CursorGrab(
                                egui::CursorGrab::Confined,
                            ));
                            ui.ctx().send_viewport_cmd(
                                egui::ViewportCommand::CursorVisible(false),
                            );
                        }
                        if let Some(idx) = world.gaze_item(fwd) {
                            let it = &world.items[idx];
                            gaze_label = Some((
                                it.pos,
                                it.name.clone(),
                                it.detail.clone(),
                                it.enabled,
                            ));
                            let (lmb, e_key) = ui.input(|i| {
                                (
                                    i.pointer.button_pressed(egui::PointerButton::Primary),
                                    i.key_pressed(egui::Key::E),
                                )
                            });
                            if (self.captured && (lmb || e_key))
                                || resp.clicked_by(egui::PointerButton::Secondary)
                            {
                                pending = Some(it.action.clone());
                            }
                        }
                        // Center dot + gaze label, drawn over the world.
                        let rect = resp.rect;
                        let painter = ui.painter_at(rect);
                        let c = rect.center();
                        painter.circle_stroke(c, 3.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
                        if let Some((pos, name, detail, enabled)) = &gaze_label {
                            let clip = cam.view_proj() * pos.extend(1.0);
                            if clip.w > 0.0 {
                                let ndc = clip / clip.w;
                                let px = c + egui::vec2(ndc.x * side * 0.5, -ndc.y * side * 0.5);
                                let col = if *enabled {
                                    egui::Color32::LIGHT_GREEN
                                } else {
                                    egui::Color32::LIGHT_GRAY
                                };
                                painter.circle_stroke(px, 8.0, egui::Stroke::new(1.5, col));
                                painter.text(
                                    px + egui::vec2(12.0, -18.0),
                                    egui::Align2::LEFT_TOP,
                                    name,
                                    egui::FontId::proportional(15.0),
                                    col,
                                );
                                painter.text(
                                    px + egui::vec2(12.0, 0.0),
                                    egui::Align2::LEFT_TOP,
                                    detail,
                                    egui::FontId::monospace(12.0),
                                    egui::Color32::LIGHT_GRAY,
                                );
                            }
                        }
                    }

                    // Commit the gaze action with the world borrow released.
                    if let Some(action) = pending {
                        let mut flash: Option<String> = None;
                        let mut depart: Option<String> = None;
                        match action {
                            camp3d::CampAction::BuyRifle(m) => {
                                if let Err(e) = self.business.buy_rifle(m) {
                                    flash = Some(format!("{e}"));
                                }
                            }
                            camp3d::CampAction::BuyOptic(m) => {
                                let r = if m.price_outright_cents().is_none() {
                                    self.business.upgrade_optic(m)
                                } else {
                                    self.business.buy_optic(m)
                                };
                                if let Err(e) = r {
                                    flash = Some(format!("{e}"));
                                }
                            }
                            camp3d::CampAction::MountOptic(m) => {
                                self.mounted = m;
                                flash = Some(format!("Mounted {m:?} for tonight."));
                            }
                            camp3d::CampAction::BuyAccessory(a) => {
                                if let Err(e) = self.business.buy_accessory(a) {
                                    flash = Some(format!("{e}"));
                                }
                            }
                            camp3d::CampAction::Depart(zone) => depart = Some(zone),
                        }
                        if let Some(msg) = flash {
                            self.hud_flash = Some((msg, 0.0));
                        }
                        if let (Some(world), true) =
                            (self.camp_world.as_mut(), depart.is_none())
                        {
                            world.restock(&self.business, &self.catalog);
                            self.persist();
                        }
                        if let Some(zone) = depart {
                            self.selected_zone = zone;
                            self.start_selected_night();
                        }
                    }
                }
                Screen::Night(h) => {
                    let (yaw, pitch) = (self.yaw, self.pitch);
                    let fwd = Vec3::new(
                        yaw.sin() * pitch.cos(),
                        pitch.sin(),
                        -yaw.cos() * pitch.cos(),
                    );
                    // FPS input (spec): raw-delta mouse-look while the
                    // pointer is captured; RMB holds ADS; LMB fires;
                    // Shift holds breath; wheel = magnification in ADS.
                    let (move_dir, scroll_y, rmb_down, lmb_pressed, shift_down, q_pressed) =
                        ui.input(|i| {
                            let mut d = Vec3::ZERO;
                            let flat =
                                Vec3::new(yaw.sin(), 0.0, -yaw.cos()).normalize_or_zero();
                            let right = flat.cross(Vec3::Y);
                            if i.key_down(egui::Key::W) { d += flat; }
                            if i.key_down(egui::Key::S) { d -= flat; }
                            if i.key_down(egui::Key::A) { d -= right; }
                            if i.key_down(egui::Key::D) { d += right; }
                            (
                                d.normalize_or_zero() * 4.0,
                                i.raw_scroll_delta.y,
                                i.pointer.secondary_down(),
                                i.pointer.button_pressed(egui::PointerButton::Primary),
                                i.modifiers.shift,
                                i.key_pressed(egui::Key::Q),
                            )
                        });

                    // Q lowers the rifle; a dead or hidden target lowers it
                    // for you. The lock is what holds the scope up.
                    if q_pressed {
                        self.selected = None;
                    }
                    if let Some(id) = self.selected {
                        let live = h
                            .sim
                            .animals
                            .iter()
                            .any(|a| a.id == id && a.alive && a.is_targetable());
                        if !live {
                            self.selected = None;
                        }
                    }

                    // The scope raises when a target is locked (the
                    // select-to-scope flow) or while RMB is held (manual
                    // check without committing to a lock).
                    let ads_target = if self.captured && (self.selected.is_some() || rmb_down)
                    {
                        1.0
                    } else {
                        0.0
                    };
                    self.ads += (ads_target - self.ads) * (dt / 0.2).clamp(0.0, 1.0);
                    self.scoped = self.ads > 0.5;

                    // Sway: the reticle never rests. Amplitude rises with
                    // ADS (you see it through the optic), falls while the
                    // breath is held — and overshoots when the hold runs out.
                    self.sway_t += dt;
                    self.breath.update(dt, shift_down && self.scoped);
                    let sway_amp = aim::SWAY_BASE_RAD
                        * self.ads
                        * self.breath.sway_factor(shift_down && self.scoped);
                    let sway = aim::sway_offset(self.sway_t, 11, sway_amp);

                    let fov = {
                        let hip = 60.0;
                        let scoped_fov = aim::fov_for_mag(self.mag.max(2.0));
                        hip + (scoped_fov - hip) * self.ads
                    };
                    // Raw look, scaled by FOV ratio while scoped (spec).
                    if self.captured {
                        let sens = 0.0022 * aim::ads_sensitivity_scale(fov, 1.0);
                        let d = Self::raw_look_delta(ui.ctx());
                        self.yaw += d.x * sens;
                        self.pitch = (self.pitch - d.y * sens)
                            .clamp(-(std::f32::consts::FRAC_PI_2 - 0.001),
                                   std::f32::consts::FRAC_PI_2 - 0.001);
                        self.yaw = self.yaw.rem_euclid(std::f32::consts::TAU);
                    }
                    // The effective sight axis carries the sway.
                    let (eyaw, epitch) = (self.yaw + sway.x, (self.pitch + sway.y)
                        .clamp(-1.5, 1.5));
                    let fwd = Vec3::new(
                        eyaw.sin() * epitch.cos(),
                        epitch.sin(),
                        -eyaw.cos() * epitch.cos(),
                    );

                    let goggles = self
                        .business
                        .owns(da_econ::ItemKind::Accessory(da_econ::Accessory::NvGoggles));
                    h.tick(dt, move_dir, self.scoped || goggles);
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
                        fov_y_deg: fov,
                        aspect: 1.0,
                    };
                    let mods = h.forecast.mods();
                    let (sensor_res, sensor_hz) = sensor_for(h.mounted);
                    // Head-mounted goggles own the walk-around channel:
                    // wide unmagnified NV on your face while the rifle
                    // carries its own optic. The real two-device loadout.
                    let walk_mode = if goggles { OpticMode::Nv } else { OpticMode::Eye };
                    let settings = OpticSettings {
                        mode: if self.scoped { self.optic_mode } else { walk_mode },
                        palette: self.palette,
                        scope_mask: self.scoped,
                        frame: self.frame,
                        seed: 11,
                        nv_gain: 1.0 / mods.nv_visibility.max(0.3),
                        nv_visibility: mods.nv_visibility,
                        // The unaided eye at night is a poor instrument:
                        // walk-view exposure sits low so the dark is dark
                        // (goggles or a scope are what fix that, at retail).
                        eye_exposure: mods.eye_visibility * if self.scoped { 1.0 } else { 0.45 },
                        sensor_res: if self.scoped {
                            sensor_res
                        } else if goggles {
                            Some(720) // PVS-14-class tube through our square view
                        } else {
                            None
                        },
                    };
                    let list = h.draw_list();
                    renderer.render_on(&rs.device, &rs.queue, &list, &cam, &settings, dt);

                    let avail = ui.available_size();
                    let side = (VIEW as f32).min(avail.x).min(avail.y);
                    let pad = if ui.ctx().screen_rect().width()
                        >= ui.ctx().screen_rect().height()
                    {
                        0.0
                    } else {
                        ((avail.x - side) * 0.5).max(0.0)
                    };
                    let resp = ui
                        .horizontal(|ui| {
                            ui.add_space(pad);
                            ui.add(
                                egui::Image::new((
                                    self.view_tex.expect("registered"),
                                    egui::vec2(side, side),
                                ))
                                .sense(egui::Sense::click_and_drag()),
                            )
                        })
                        .inner;

                    // Click the view to capture; then LMB is the whole
                    // hunting verb: no lock -> lock the animal near your
                    // gaze and glass up; locked -> fire.
                    if lmb_pressed {
                        if !self.captured && resp.hovered() {
                            self.captured = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CursorGrab(
                                egui::CursorGrab::Confined,
                            ));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                        } else if self.captured {
                            if self.selected.is_none() {
                                let candidates: Vec<(usize, Vec3)> = h
                                    .sim
                                    .animals
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, a)| a.alive && a.is_targetable())
                                    .map(|(i, a)| {
                                        let head = h
                                            .head_of(a.id)
                                            .unwrap_or(a.pos + Vec3::Y * 0.3);
                                        (i, head)
                                    })
                                    .collect();
                                if let Some(i) = aim::pick_nearest_axis(
                                    h.sim.player.pos,
                                    fwd,
                                    &candidates,
                                    120.0,
                                ) {
                                    let a = &h.sim.animals[i];
                                    self.selected = Some(a.id);
                                    // Auto-fit magnification to the range:
                                    // far target, tighter glass. Wheel
                                    // fine-tunes from there.
                                    let head =
                                        h.head_of(a.id).unwrap_or(a.pos + Vec3::Y * 0.3);
                                    let range = head.distance(h.sim.player.pos);
                                    self.mag = (range / 9.0).clamp(2.0, 14.5);
                                }
                            } else if self.scoped {
                                if let Some(msg) = h.fire_axis(fwd, &self.business) {
                                    self.hud_flash = Some((msg, 0.0));
                                }
                                if let Some(id) = self.selected {
                                    if !h.sim.animals.iter().any(|a| a.id == id && a.alive)
                                    {
                                        self.selected = None;
                                    }
                                }
                            }
                        }
                    }
                    // Wheel: magnification while glassed.
                    if self.captured && self.ads > 0.3 && scroll_y.abs() > 0.0 {
                        self.mag = (self.mag * (1.0 + scroll_y * 0.0015)).clamp(2.0, 14.5);
                    }
                    // Middle-drag still pans (fallback for uncaptured play).
                    if resp.dragged_by(egui::PointerButton::Middle) {
                        let d = resp.drag_delta();
                        let sens = 0.004 * (fov / 60.0);
                        self.yaw += d.x * sens;
                        self.pitch = (self.pitch - d.y * sens).clamp(-1.4, 1.4);
                    }

                    // ---- Reticle overlay: crosshair + mil scale axes ----
                    let rect = resp.rect;
                    let painter = ui.painter_at(rect);
                    let c = rect.center();
                    let ppm = aim::px_per_mil(side, fov);
                    let ret = egui::Color32::from_rgba_unmultiplied(255, 60, 60, 200);
                    let ret_dim = egui::Color32::from_rgba_unmultiplied(255, 60, 60, 110);
                    let stroke = egui::Stroke::new(1.0, ret);
                    if !self.scoped {
                        // Rifle down: just the gaze dot. The reticle lives
                        // in the scope, not on your face.
                        painter.circle_stroke(
                            c,
                            3.0,
                            egui::Stroke::new(1.0, egui::Color32::from_gray(200)),
                        );
                    } else {
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
                    }
                    // Mil ticks on both axes (FFP: spacing follows zoom).
                    if self.scoped && ppm > 4.0 {
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
                    painter.text(
                        rect.right_bottom() + egui::vec2(-10.0, -28.0),
                        egui::Align2::RIGHT_BOTTOM,
                        instrument_label(settings.mode, self.palette, settings.sensor_res),
                        egui::FontId::monospace(13.0),
                        egui::Color32::from_rgb(255, 210, 90),
                    );
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
                    if self.scoped {
                        painter.text(
                            rect.left_top() + egui::vec2(10.0, 10.0),
                            egui::Align2::LEFT_TOP,
                            format!("{:.1}x", self.mag),
                            egui::FontId::monospace(14.0),
                            egui::Color32::LIGHT_GRAY,
                        );
                    }

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

fn camp_shot(path: &str) {
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let catalog = ZoneCatalog::load(&zones_dir()).expect("catalog");
    let world = camp3d::CampWorld::new(&camp_source_path(), &Business::new(), &catalog)
        .expect("camp");
    // Stand at the yard's south edge looking at the cabin + rack.
    let eye = Vec3::new(27.0, 1.6, 22.0);
    let cam = Camera {
        eye,
        look: Vec3::new(26.0, 1.3, 33.0),
        up: Vec3::Y,
        fov_y_deg: 55.0,
        aspect: 1.0,
    };
    let list = world.draw_list(&Business::new());
    let settings = OpticSettings {
        mode: OpticMode::Eye,
        eye_exposure: 2.2,
        ..Default::default()
    };
    for _ in 0..10 {
        renderer.render(&gpu, &list, &cam, &settings, 0.1);
    }
    let rgba = renderer.read_rgba(&gpu);
    image::save_buffer(path, &rgba, VIEW, VIEW, image::ColorType::Rgba8).expect("png");
    println!("wrote {path}");
}

/// The night-ops shell theme: near-black green field, phosphor accent —
/// the panel should feel like the gear, not like a default toolkit.
fn apply_theme(ctx: &egui::Context) {
    use egui::{Color32, Rounding, Stroke};
    let mut v = egui::Visuals::dark();
    let bg = Color32::from_rgb(10, 14, 11);
    let panel = Color32::from_rgb(15, 20, 16);
    let raised = Color32::from_rgb(24, 32, 26);
    let hover = Color32::from_rgb(34, 47, 36);
    let active = Color32::from_rgb(42, 60, 44);
    let phosphor = Color32::from_rgb(112, 230, 140);
    let text = Color32::from_rgb(188, 202, 190);

    v.window_fill = bg;
    v.panel_fill = panel;
    v.extreme_bg_color = Color32::from_rgb(6, 9, 7);
    v.faint_bg_color = raised;
    v.override_text_color = Some(text);
    v.hyperlink_color = phosphor;
    v.selection.bg_fill = Color32::from_rgb(26, 66, 36);
    v.selection.stroke = Stroke::new(1.0, phosphor);
    v.widgets.noninteractive.bg_fill = panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(30, 40, 32));
    v.widgets.inactive.bg_fill = raised;
    v.widgets.inactive.weak_bg_fill = raised;
    v.widgets.hovered.bg_fill = hover;
    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, phosphor);
    v.widgets.active.bg_fill = active;
    v.widgets.active.weak_bg_fill = active;
    v.widgets.active.bg_stroke = Stroke::new(1.2, phosphor);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = Rounding::same(4.0);
    }
    v.window_rounding = Rounding::same(6.0);
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles.insert(TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    style.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
    ctx.set_style(style);
}

/// Headless performance bench: render the calibration range across optic
/// pipelines, rabbit counts, and magnifications; print a timing table.
fn bench() {
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    println!("adapter: {}", gpu.adapter.get_info().name);
    println!("{:<8} {:>7} {:>6} {:>9} {:>9} {:>9}", "optic", "rabbits", "mag", "avg ms", "p95 ms", "max ms");
    for (mode, name) in [
        (OpticMode::Eye, "eye"),
        (OpticMode::Nv, "nv"),
        (OpticMode::Thermal, "thermal"),
    ] {
        for rabbits in [6usize, 50, 200] {
            for mag in [2.0f32, 14.5] {
                let mut r = RangeState::new();
                r.rabbit_count = rabbits;
                r.rabbit_speed = 6.0;
                let eye = Vec3::new(0.0, 1.6, 8.0);
                let cam = Camera {
                    eye,
                    look: eye + Vec3::new(0.0, -0.05, -1.0),
                    up: Vec3::Y,
                    fov_y_deg: aim::fov_for_mag(mag),
                    aspect: 1.0,
                };
                let settings = OpticSettings {
                    mode,
                    scope_mask: true,
                    // Bench what the game ships: each pipeline at its
                    // device's sensor resolution (Mk II thermal, NV basic).
                    sensor_res: match mode {
                        OpticMode::Thermal => Some(288),
                        OpticMode::Nv => Some(720),
                        OpticMode::Eye => None,
                    },
                    ..Default::default()
                };
                let mut dts = Vec::new();
                // Warm up, then measure with the scene animating.
                for i in 0..45 {
                    r.tick(1.0 / 60.0);
                    let list = r.draw_list();
                    let t0 = std::time::Instant::now();
                    renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, 1.0 / 60.0);
                    gpu.device.poll(wgpu::Maintain::Wait);
                    if i >= 5 {
                        dts.push(t0.elapsed().as_secs_f32());
                    }
                }
                dts.sort_by(|a, b| a.total_cmp(b));
                let avg = dts.iter().sum::<f32>() / dts.len() as f32 * 1000.0;
                let p95 = dts[(dts.len() as f32 * 0.95) as usize] * 1000.0;
                let max = dts.last().copied().unwrap_or(0.0) * 1000.0;
                println!(
                    "{name:<8} {rabbits:>7} {mag:>6.1} {avg:>9.2} {p95:>9.2} {max:>9.2}"
                );
            }
        }
    }
}

/// Headless shimmer probe. Two measurements on the checkerboard crop:
/// (1) determinism — the same time+mag rendered twice must be byte-equal
/// (eye/thermal; NV grain is animated by design); (2) zoom stability —
/// mean abs pixel diff between adjacent magnification steps. Large spikes
/// between near-identical mags are aliasing crawl, i.e. shimmer.
fn shimmer() {
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let r = RangeState::new(); // static: rabbits ignored, boards matter
    let eye = Vec3::new(0.0, 1.6, 8.0);
    let list = r.draw_list();

    let render_at = |renderer: &mut Renderer, mode: OpticMode, mag: f32| -> Vec<u8> {
        let cam = Camera {
            eye,
            look: eye + Vec3::new(0.0, -0.02, -1.0),
            up: Vec3::Y,
            fov_y_deg: aim::fov_for_mag(mag),
            aspect: 1.0,
        };
        let settings = OpticSettings {
            mode,
            scope_mask: false,
            frame: 0, // frozen grain frame: determinism check covers NV too
            ..Default::default()
        };
        // Deterministic protocol: reset the thermal AGC and give it a full
        // settle from the same start every time — otherwise the window is
        // still creeping toward its asymptote and the comparison measures
        // convergence, not rendering.
        renderer.agc = da_render::Agc::new();
        for _ in 0..40 {
            renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, 0.1);
        }
        renderer.read_rgba_on(&gpu.device, &gpu.queue)
    };
    // Center crop covering the 25 m board at most magnifications.
    let crop = |img: &[u8]| -> Vec<u8> {
        let (w, c0, c1) = (VIEW as usize, VIEW as usize / 4, 3 * VIEW as usize / 4);
        let mut out = Vec::new();
        for y in c0..c1 {
            for x in c0..c1 {
                let i = (y * w + x) * 4;
                out.extend_from_slice(&img[i..i + 3]);
            }
        }
        out
    };

    for (mode, name) in [
        (OpticMode::Eye, "eye"),
        (OpticMode::Nv, "nv"),
        (OpticMode::Thermal, "thermal"),
    ] {
        let a = render_at(&mut renderer, mode, 8.0);
        let b = render_at(&mut renderer, mode, 8.0);
        println!(
            "{name}: re-render determinism at 8.0x: {}",
            if a == b { "BYTE-IDENTICAL" } else { "MISMATCH" }
        );
        let mut prev: Option<Vec<u8>> = None;
        let mut worst = 0.0f32;
        let mut worst_mag = 0.0f32;
        let mut mag = 2.0f32;
        while mag <= 14.5 {
            let img = crop(&render_at(&mut renderer, mode, mag));
            if let Some(p) = &prev {
                let diff: f32 = img
                    .iter()
                    .zip(p)
                    .map(|(x, y)| (*x as f32 - *y as f32).abs())
                    .sum::<f32>()
                    / img.len() as f32;
                if diff > worst {
                    worst = diff;
                    worst_mag = mag;
                }
            }
            prev = Some(img);
            mag += 0.25;
        }
        println!(
            "{name}: worst adjacent-mag diff {worst:.2}/255 at {worst_mag:.2}x              (crawl proxy; lower is calmer)"
        );
    }
}

/// Measure p95 frame time (ms) for `n` rabbits at `mag`. Fails fast when
/// the first frames already blow the budget wide (2×), so the search stays
/// quick on weak machines.
fn measure_p95_ms(
    gpu: &da_render::Gpu,
    renderer: &mut Renderer,
    n: usize,
    mag: f32,
    budget_ms: f32,
) -> f32 {
    let mut r = RangeState::new();
    r.ensure_lanes(n);
    r.rabbit_count = n;
    r.rabbit_speed = 6.0;
    let eye = Vec3::new(0.0, 1.6, 8.0);
    let cam = Camera {
        eye,
        look: eye + Vec3::new(0.0, -0.05, -1.0),
        up: Vec3::Y,
        fov_y_deg: aim::fov_for_mag(mag),
        aspect: 1.0,
    };
    let settings = OpticSettings {
        mode: OpticMode::Thermal, // the hunt's workhorse pipeline
        scope_mask: true,
        ..Default::default()
    };
    let mut dts: Vec<f32> = Vec::new();
    for i in 0..28 {
        r.tick(1.0 / 60.0);
        let list = r.draw_list();
        let t0 = std::time::Instant::now();
        renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, 1.0 / 60.0);
        gpu.device.poll(wgpu::Maintain::Wait);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        if i >= 4 {
            dts.push(ms);
        }
        if i == 8 && dts.iter().sum::<f32>() / dts.len() as f32 > budget_ms * 2.0 {
            return f32::MAX; // hopeless — don't burn the wall clock
        }
    }
    dts.sort_by(|a, b| a.total_cmp(b));
    dts[(dts.len() as f32 * 0.95) as usize]
}

/// Adaptive machine calibration: converge on the largest rabbit count whose
/// p95 frame time fits the budget, per magnification. The headline rating
/// is the worst case across mags at the 30 fps budget — "this laptop is an
/// N-rabbit machine" — comparable across hosts because the scene, lanes,
/// and protocol are fully deterministic.
fn calibrate() {
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let adapter = gpu.adapter.get_info().name.clone();
    println!("DARKAIR MACHINE CALIBRATION");
    println!("host: {cpus} cpus | adapter: {adapter} | view {VIEW}x{VIEW}\n");

    const MAX_N: usize = 1024;
    let mags = [2.0f32, 8.0, 14.5];
    let budgets = [(30.0f32, 1000.0 / 30.0), (60.0, 1000.0 / 60.0)];
    let mut headline: usize = MAX_N;

    for (fps, budget_ms) in budgets {
        println!("budget {fps:.0} fps (p95 ≤ {budget_ms:.1} ms):");
        for mag in mags {
            // Binary search the pass/fail boundary.
            let passes = |renderer: &mut Renderer, n: usize| {
                measure_p95_ms(&gpu, renderer, n, mag, budget_ms) <= budget_ms
            };
            let mut result = 0;
            if passes(&mut renderer, 1) {
                let (mut lo, mut hi) = (1usize, MAX_N);
                // Grow first: find a failing ceiling quickly.
                let mut probe = 8;
                while probe < MAX_N && passes(&mut renderer, probe) {
                    lo = probe;
                    probe *= 4;
                }
                hi = probe.min(MAX_N);
                if hi >= MAX_N && passes(&mut renderer, MAX_N) {
                    lo = MAX_N;
                    hi = MAX_N;
                }
                while hi - lo > 1 {
                    let mid = (lo + hi) / 2;
                    if passes(&mut renderer, mid) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                result = lo;
            }
            let shown = if result >= MAX_N {
                format!("{MAX_N}+")
            } else {
                format!("{result}")
            };
            println!("  mag {mag:>4.1}x: {shown} rabbits");
            if fps == 30.0 {
                headline = headline.min(result);
            }
        }
    }
    println!("\nRATING: this is a {headline}-rabbit machine");
    println!("(worst case across magnifications at sustained 30 fps, thermal pipeline)");

    // Persist for future auto-tuning (zone density, default stress dial).
    let card = format!(
        "(\n    cpus: {cpus},\n    adapter: {:?},\n    rabbit_rating: {headline},\n)\n",
        adapter
    );
    let path = std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".darkair-calibration.ron"))
        .unwrap_or_else(|| ".darkair-calibration.ron".into());
    if std::fs::write(&path, card).is_ok() {
        println!("card written to {}", path.display());
    }
}

/// Render one rabbit posture at ~40 m through white-hot thermal for the
/// footage comparison sheet (`--shot-rabbit out.png graze|sit|hop`).
fn rabbit_shot(path: &str, posture: &str) {
    beast_shot(path, "Rabbit", posture, None, false)
}

fn rabbit_shot_at(path: &str, posture: &str, sensor: Option<u32>) {
    beast_shot(path, "Rabbit", posture, sensor, false)
}

/// Render any species' rig at ~40 m through thermal (white-hot, or
/// black-hot for A/B against the ATN-style clips).
fn beast_shot(path: &str, species_name: &str, posture: &str, sensor: Option<u32>, black_hot: bool) {
    use darkair::fauna::{self, FaunaPose};
    use glam::Mat4;
    let species = darkair::convert::sim_species(species_name)
        .unwrap_or(da_sim::Species::Rabbit);
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let ambient = 48.0;
    let (speed, frozen, phase) = match posture {
        "sit" => (0.0, true, 0.0),
        "hop" => (1.0, false, 0.30),
        _ => (0.1, false, 0.25),
    };
    let pose = FaunaPose {
        pos: Vec3::new(0.0, 0.0, -40.0),
        heading: std::f32::consts::PI, // facing left, like the footage
        speed_norm: speed,
        gait_phase: phase,
        frozen,
    };
    let mut items: Vec<da_render::draw::DrawItem> = vec![da_render::draw::DrawItem {
        shape: da_render::draw::Shape::GroundPatch { half: 120.0 },
        world: Mat4::from_translation(Vec3::new(0.0, -0.02, -40.0)),
        albedo: [0.22, 0.26, 0.18],
        emissive: 0.0,
        temp_f: ambient - 4.0,
        glass: false,
        coat_f: 0.0,
    }];
    // Grass structure, as in the footage.
    items.extend(darkair::flora::tufts_around(Vec3::new(0.0, 0.0, -40.0), 30.0, ambient));

    // Background tree line, as in the footage: warm canopy occupies the
    // AGC window's upper half, which is what makes the dirt read dark.
    for i in 0..6 {
        let x = -18.0 + i as f32 * 7.0;
        items.push(da_render::draw::DrawItem {
            shape: da_render::draw::Shape::Cylinder { radius: 0.35, height: 4.0 },
            world: Mat4::from_translation(Vec3::new(x, 0.0, -62.0)),
            albedo: [0.3, 0.24, 0.18],
            emissive: 0.0,
            temp_f: ambient + 3.0,
            glass: false,
            coat_f: 0.0,
        });
        items.push(da_render::draw::DrawItem {
            shape: da_render::draw::Shape::Sphere { radius: 3.4 },
            world: Mat4::from_translation(Vec3::new(x, 6.2, -62.0)),
            albedo: [0.16, 0.28, 0.14],
            emissive: 0.0,
            temp_f: ambient + 5.0,
            glass: false,
            coat_f: 0.0,
        });
    }
    for part in fauna::build(species, &pose) {
        items.push(da_render::draw::DrawItem {
            shape: part.shape,
            world: part.world,
            albedo: part.albedo,
            emissive: 0.0,
            temp_f: 101.0 + part.temp_bias,
            glass: false,
            coat_f: part.coat_f,
        });
    }
    let list = da_render::draw::DrawList {
        items,
        ambient_f: ambient,
        sky_temp_f: ambient - 45.0,
        moonlight: 0.4,
        heat_decals: vec![],
        eyeshine: vec![],
    };
    let eye = Vec3::new(0.0, 1.2, 0.0);
    let cam = Camera {
        eye,
        look: Vec3::new(0.0, 0.15, -40.0),
        up: Vec3::Y,
        fov_y_deg: aim::fov_for_mag(12.0),
        aspect: 1.0,
    };
    let settings = OpticSettings {
        mode: OpticMode::Thermal,
        palette: if black_hot {
            da_render::ThermalPalette::BlackHot
        } else {
            da_render::ThermalPalette::WhiteHot
        },
        scope_mask: false,
        sensor_res: sensor,
        ..Default::default()
    };
    renderer.agc = da_render::Agc::new();
    for _ in 0..40 {
        renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, 0.1);
    }
    let rgba = renderer.read_rgba_on(&gpu.device, &gpu.queue);
    image::save_buffer(path, &rgba, VIEW, VIEW, image::ColorType::Rgba8).expect("png");
    println!("wrote {path}");
}

/// Render the calibration range (the boot scene) through the Stellar-class
/// thermal at a given magnification/pitch — the headless twin of what the
/// app shows on launch, for A/B against the reference footage.
fn range_shot(path: &str, mag: f32, pitch_deg: f32) {
    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let mut range = darkair::range::RangeState::new();
    for _ in 0..30 {
        range.tick(1.0 / 30.0);
    }
    let list = range.draw_list();
    let eye = Vec3::new(0.0, 1.6, 8.0);
    let pitch = pitch_deg.to_radians();
    let fwd = Vec3::new(0.0, pitch.sin(), -pitch.cos());
    let cam = Camera {
        eye,
        look: eye + fwd * 40.0,
        up: Vec3::Y,
        fov_y_deg: aim::fov_for_mag(mag),
        aspect: 1.0,
    };
    let settings = OpticSettings {
        mode: OpticMode::Thermal,
        scope_mask: true,
        sensor_res: Some(288),
        ..Default::default()
    };
    renderer.agc = da_render::Agc::new();
    for _ in 0..40 {
        renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, 0.1);
    }
    let rgba = renderer.read_rgba_on(&gpu.device, &gpu.queue);
    image::save_buffer(path, &rgba, VIEW, VIEW, image::ColorType::Rgba8).expect("png");
    println!("wrote {path}");
}

/// `--demo-film out.mp4 [fps]`: render the scripted demo headlessly and
/// pipe raw frames straight into ffmpeg; captions are logged with frame
/// times and burned in a drawtext second pass. Deterministic end to end.
fn demo_film(out_path: &str, fps: u32) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let gpu = da_render::Gpu::new_headless().expect("gpu");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);
    let mut director =
        darkair::demo::DemoDirector::new(&zones_dir(), &camp_source_path()).expect("demo script");
    let total = director.total_dur();
    let dt = 1.0 / fps as f32;
    let tmp = format!("{out_path}.nocap.mp4");

    let mut ff = Command::new("ffmpeg")
        .args([
            "-y", "-loglevel", "error",
            "-f", "rawvideo", "-pix_fmt", "rgba",
            "-s", &format!("{VIEW}x{VIEW}"),
            "-r", &fps.to_string(),
            "-i", "-",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
            &tmp,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .expect("ffmpeg (is it installed?)");
    let mut pipe = ff.stdin.take().expect("ffmpeg stdin");

    // Caption intervals: (text, row, start_s, end_s-so-far).
    let mut log: Vec<(String, usize, f32, f32)> = Vec::new();
    let mut open: Vec<(String, usize, f32)> = Vec::new();
    let mut t = 0.0f32;
    let mut frames = 0u64;
    let mut last_pct = 0u32;
    while let Some(frame) = director.advance(dt) {
        director.refresh_pending_hunts(&zones_dir());
        renderer.render_on(&gpu.device, &gpu.queue, &frame.list, &frame.cam, &frame.settings, dt);
        let rgba = renderer.read_rgba_on(&gpu.device, &gpu.queue);
        pipe.write_all(&rgba).expect("pipe frame");
        // Track caption lifetimes for the burn pass.
        for (row, c) in frame.captions.iter().enumerate() {
            if !open.iter().any(|(s, r, _)| s == c && *r == row) {
                open.push((c.clone(), row, t));
            }
        }
        open.retain(|(s, r, start)| {
            let alive = frame
                .captions
                .get(*r)
                .map(|c| c == s)
                .unwrap_or(false);
            if !alive {
                log.push((s.clone(), *r, *start, t));
            }
            alive
        });
        t += dt;
        frames += 1;
        let pct = (t / total * 100.0) as u32;
        if pct >= last_pct + 10 {
            last_pct = pct;
            eprintln!("demo-film: {pct}% ({t:.0}s / {total:.0}s)");
        }
    }
    for (s, r, start) in open.drain(..) {
        log.push((s, r, start, t));
    }
    drop(pipe);
    assert!(ff.wait().expect("ffmpeg wait").success(), "ffmpeg pass 1 failed");

    // Burn captions: one drawtext per interval, enabled by time.
    let font = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ]
    .iter()
    .find(|p| std::path::Path::new(p).exists())
    .copied();
    let Some(font) = font else {
        std::fs::rename(&tmp, out_path).expect("rename");
        println!("wrote {out_path} ({frames} frames, captions skipped: no DejaVu font)");
        return;
    };
    let esc = |s: &str| s.replace('\\', "\\\\").replace(':', "\\:").replace('\'', "");
    let mut filters: Vec<String> = Vec::new();
    for (text, row, start, end) in &log {
        let y = 880 + row * 42;
        filters.push(format!(
            "drawtext=fontfile={font}:text='{}':x=(w-text_w)/2:y={y}:fontsize=30:\
             fontcolor=0xCCFFCC:box=1:boxcolor=0x000000AA:boxborderw=10:\
             enable='between(t,{start:.2},{end:.2})'",
            esc(text)
        ));
    }
    let filter_file = format!("{out_path}.filters");
    std::fs::write(&filter_file, filters.join(",\n")).expect("filter script");
    let ok = Command::new("ffmpeg")
        .args([
            "-y", "-loglevel", "error",
            "-i", &tmp,
            "-filter_complex_script", &filter_file,
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
            out_path,
        ])
        .status()
        .expect("ffmpeg pass 2")
        .success();
    if ok {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&filter_file);
        println!("wrote {out_path} ({frames} frames, {:.0}s at {fps} fps)", t);
    } else {
        std::fs::rename(&tmp, out_path).expect("rename");
        println!("wrote {out_path} WITHOUT captions (drawtext pass failed; filters kept at {filter_file})");
    }
}

fn main() {
    // WSLg: the Wayland compositor bridge has no relative-pointer or
    // pointer-constraints protocol, so mouse-look gets zero raw deltas and
    // Locked grabs fail. XWayland delivers XInput2 raw motion regardless of
    // grab state, so prefer X11 there (respecting an explicit override).
    if std::env::var_os("WINIT_UNIX_BACKEND").is_none()
        && (std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .map(|r| r.to_lowercase().contains("microsoft"))
                .unwrap_or(false))
    {
        std::env::set_var("WINIT_UNIX_BACKEND", "x11");
    }

    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--demo-film") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("darkair_demo.mp4");
        let fps: u32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(30);
        demo_film(path, fps);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--shot-range") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("range.png");
        let mag: f32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let pitch: f32 = args.get(i + 3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        range_shot(path, mag, pitch);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--shot-rabbit") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("rabbit.png");
        let posture = args.get(i + 2).map(String::as_str).unwrap_or("graze");
        let sensor = args.get(i + 3).and_then(|s| s.parse().ok());
        rabbit_shot_at(path, posture, sensor);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--shot-beast") {
        let species = args.get(i + 1).map(String::as_str).unwrap_or("Rabbit");
        let path = args.get(i + 2).map(String::as_str).unwrap_or("beast.png");
        let posture = args.get(i + 3).map(String::as_str).unwrap_or("graze");
        let sensor = args.get(i + 4).and_then(|s| s.parse().ok());
        let black_hot = args.get(i + 5).map(String::as_str) == Some("black");
        beast_shot(path, species, posture, sensor, black_hot);
        return;
    }
    if args.iter().any(|a| a == "--calibrate") {
        calibrate();
        return;
    }
    if args.iter().any(|a| a == "--bench") {
        bench();
        return;
    }
    if args.iter().any(|a| a == "--shimmer") {
        shimmer();
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--shot-camp") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("camp.png");
        camp_shot(path);
        return;
    }
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

    // A flag this build doesn't know must NOT fall through and launch the
    // GUI — a stale binary silently opening the game window reads as "the
    // fix didn't work" when it's really "wrong binary". Whitelist, so
    // flags invented later are rejected by binaries built earlier.
    const KNOWN_FLAGS: &[&str] = &["--windowed", "--demo"];
    if let Some(unknown) = args
        .iter()
        .skip(1)
        .find(|a| a.starts_with("--") && !KNOWN_FLAGS.contains(&a.as_str()))
    {
        eprintln!(
            "error: unrecognized flag `{unknown}` — this binary is older \
             than that feature. Rebuild first:\n  PATH=/snap/bin:$PATH \
             cargo build --release -p darkair"
        );
        std::process::exit(2);
    }

    // Multi-monitor control: fullscreen claims whatever monitor the window
    // spawns on, so window placement decides everything.
    //   - default: the last position is persisted, so Esc -> drag to the
    //     monitor you want -> F11 once, and every later launch lands there;
    //   - DARKAIR_POS="x,y" pins the spawn position explicitly (virtual
    //     desktop coordinates; overrides persistence for that run);
    //   - --windowed starts windowed for easy dragging.
    let windowed = std::env::args().any(|a| a == "--windowed");
    let forced_pos = std::env::var("DARKAIR_POS").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("DarkAir")
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
                        label: Some("darkair"),
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
        "DarkAir",
        native,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
    .expect("eframe run");
}
