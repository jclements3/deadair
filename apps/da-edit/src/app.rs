//! The editor shell: egui panels around an offscreen da-render viewport.
//!
//! Viewport strategy: a *separate headless* [`da_render::Gpu`] renders the
//! scene at a fixed 640×360, the RGBA result is read back with
//! `Renderer::read_rgba`, and uploaded as an egui texture each frame. This
//! decouples the editor completely from eframe's own wgpu device and is
//! fast enough on llvmpipe at editor resolution.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use da_core::{Forecast, NodeId};
use da_edit::convert;
use da_edit::preview::PreviewEnv;
use da_graph::{CullVisitor, NodeKind, Scene};
use da_param::{expand_zone, parse_zone_str, ZoneExpansion};
use da_render::{Camera, Gpu, OpticMode, OpticSettings, Renderer, ThermalPalette};
use glam::Vec3;

const VIEW_W: u32 = 640;
const VIEW_H: u32 = 360;

// ----------------------------------------------------------------------
// Orbit camera
// ----------------------------------------------------------------------

struct Orbit {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl Orbit {
    fn eye(&self) -> Vec3 {
        self.target
            + self.dist
                * Vec3::new(
                    self.yaw.cos() * self.pitch.cos(),
                    self.pitch.sin(),
                    self.yaw.sin() * self.pitch.cos(),
                )
    }

    fn camera(&self) -> Camera {
        Camera {
            eye: self.eye(),
            look: self.target,
            up: Vec3::Y,
            fov_y_deg: 55.0,
            aspect: VIEW_W as f32 / VIEW_H as f32,
        }
    }

    /// Camera-space right and up axes (for panning).
    fn basis(&self) -> (Vec3, Vec3) {
        let fwd = (self.target - self.eye()).normalize_or_zero();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd);
        (right, up)
    }
}

// ----------------------------------------------------------------------
// App
// ----------------------------------------------------------------------

pub struct EditorApp {
    // Zone / source loop (text is ground truth).
    zone_path: PathBuf,
    zone_name: String,
    source_text: String,
    expansion: Option<ZoneExpansion>,
    available_zones: Vec<PathBuf>,
    error: Option<String>,
    status: Option<String>,

    // Selection / inspector.
    selected: Option<NodeId>,
    name_edit: String,
    name_edit_node: Option<NodeId>,

    // Preview environment.
    night_t: f32,
    forecast: Forecast,
    optic: OpticMode,
    palette: ThermalPalette,

    // Offscreen viewport.
    gpu: Option<Gpu>,
    renderer: Option<Renderer>,
    gpu_error: Option<String>,
    viewport_tex: Option<egui::TextureHandle>,
    orbit: Orbit,
    frame: u32,
    last_frame: Instant,

    show_source: bool,
}

fn zones_dir() -> PathBuf {
    let local = PathBuf::from("assets/zones");
    if local.is_dir() {
        return local;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/zones")
}

fn list_zone_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".zone.ron"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn kind_label(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Group => "Group".to_owned(),
        NodeKind::Transform(_) => "Transform".to_owned(),
        NodeKind::Geode(g) => format!("Geode({})", g.drawables.len()),
        NodeKind::Switch { .. } => "Switch".to_owned(),
        NodeKind::Lod { .. } => "Lod".to_owned(),
    }
}

impl EditorApp {
    pub fn new() -> Self {
        let available_zones = list_zone_files(&zones_dir());
        let (gpu, renderer, gpu_error) = match Gpu::new_headless() {
            Ok(g) => {
                let r = Renderer::new(&g, VIEW_W, VIEW_H);
                (Some(g), Some(r), None)
            }
            Err(e) => (None, None, Some(e)),
        };
        let mut app = Self {
            zone_path: PathBuf::new(),
            zone_name: "(no zone)".to_owned(),
            source_text: String::new(),
            expansion: None,
            available_zones,
            error: None,
            status: None,
            selected: None,
            name_edit: String::new(),
            name_edit_node: None,
            night_t: 0.35,
            forecast: Forecast::Clear,
            optic: OpticMode::Thermal,
            palette: ThermalPalette::WhiteHot,
            gpu,
            renderer,
            gpu_error,
            viewport_tex: None,
            orbit: Orbit {
                target: Vec3::ZERO,
                yaw: -1.9,
                pitch: 0.65,
                dist: 180.0,
            },
            frame: 0,
            last_frame: Instant::now(),
            show_source: true,
        };
        let default = app
            .available_zones
            .iter()
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("home_farm"))
            })
            .or_else(|| app.available_zones.first())
            .cloned();
        if let Some(path) = default {
            app.load_zone(path);
        }
        app
    }

    // ------------------------------------------------------------------
    // Zone / source loop
    // ------------------------------------------------------------------

    fn load_zone(&mut self, path: PathBuf) {
        match fs::read_to_string(&path) {
            Ok(text) => {
                self.source_text = text;
                self.zone_path = path;
                self.reexpand();
                if self.error.is_none() {
                    self.frame_scene();
                }
            }
            Err(e) => self.error = Some(format!("read {}: {e}", path.display())),
        }
    }

    /// Parse + expand `source_text`; on success the graph is replaced (and
    /// the selection cleared), on failure the old graph stays and the
    /// error is shown inline.
    fn reexpand(&mut self) {
        match parse_zone_str(&self.source_text)
            .and_then(|src| expand_zone(&src).map(|exp| (src.name.clone(), exp)))
        {
            Ok((name, exp)) => {
                self.zone_name = name;
                self.expansion = Some(exp);
                self.selected = None;
                self.name_edit_node = None;
                self.error = None;
                self.status = Some(format!("expanded \"{}\"", self.zone_name));
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn save_source(&mut self) {
        match fs::write(&self.zone_path, &self.source_text) {
            Ok(()) => {
                self.status = Some(format!("saved {}", self.zone_path.display()));
                self.error = None;
            }
            Err(e) => self.error = Some(format!("save {}: {e}", self.zone_path.display())),
        }
    }

    /// Point the orbit camera at the expanded scene's bounds.
    fn frame_scene(&mut self) {
        let Some(exp) = &self.expansion else { return };
        let bound = exp.scene.world_bound(exp.scene.root());
        if !bound.is_empty() {
            self.orbit.target = bound.center;
            self.orbit.dist = (bound.radius * 1.7).clamp(10.0, 900.0);
        }
    }

    // ------------------------------------------------------------------
    // Offscreen viewport render
    // ------------------------------------------------------------------

    fn render_viewport(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().clamp(0.0, 0.1);
        self.last_frame = now;
        self.frame = self.frame.wrapping_add(1);

        let Some(exp) = &self.expansion else { return };
        let Some(gpu) = self.gpu.as_ref() else { return };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        let cam = self.orbit.camera();
        let leaves = CullVisitor::new(cam.eye).cull(&exp.scene);
        let env = PreviewEnv::new(self.night_t, self.forecast);
        let list = convert::build_draw_list(&exp.scene, &leaves, &env);
        let settings = OpticSettings {
            mode: self.optic,
            palette: self.palette,
            scope_mask: false,
            frame: self.frame,
            seed: 1,
            nv_gain: 1.0,
            nv_visibility: self.forecast.mods().nv_visibility,
            eye_exposure: 1.0,
        };
        renderer.render(gpu, &list, &cam, &settings, dt);
        let bytes = renderer.read_rgba(gpu);
        let image =
            egui::ColorImage::from_rgba_unmultiplied([VIEW_W as usize, VIEW_H as usize], &bytes);
        match &mut self.viewport_tex {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.viewport_tex =
                    Some(ctx.load_texture("da-edit viewport", image, egui::TextureOptions::LINEAR))
            }
        }
    }

    // ------------------------------------------------------------------
    // Panels
    // ------------------------------------------------------------------

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let zones = self.available_zones.clone();
                    ui.menu_button("Open Zone", |ui| {
                        for path in &zones {
                            let label = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("?")
                                .to_owned();
                            if ui.button(label).clicked() {
                                self.load_zone(path.clone());
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Save source").clicked() {
                        self.save_source();
                        ui.close_menu();
                    }
                    if ui.button("Re-expand").clicked() {
                        self.reexpand();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_source, "Source panel");
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(&self.zone_name);
                if let Some(exp) = &self.expansion {
                    ui.separator();
                    ui.label(format!("{} nodes", exp.scene.len()));
                    ui.separator();
                    ui.label(format!("{} spawn points", exp.spawn_points.len()));
                    if let Some(sel) = self.selected {
                        let path = exp
                            .scene
                            .path(sel)
                            .iter()
                            .map(|&id| {
                                exp.scene
                                    .node(id)
                                    .and_then(|n| n.name())
                                    .unwrap_or("·")
                                    .to_owned()
                            })
                            .collect::<Vec<_>>()
                            .join(" / ");
                        ui.separator();
                        ui.label(path);
                    }
                }
                if let Some(status) = &self.status {
                    ui.separator();
                    ui.weak(status);
                }
            });
        });
    }

    fn source_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("source")
            .resizable(true)
            .default_height(220.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let arrow = if self.show_source { "v" } else { ">" };
                    if ui.button(format!("{arrow} Source")).clicked() {
                        self.show_source = !self.show_source;
                    }
                    if ui.button("Re-expand").clicked() {
                        self.reexpand();
                    }
                    if ui.button("Save source").clicked() {
                        self.save_source();
                    }
                    ui.weak(self.zone_path.display().to_string());
                    if let Some(err) = &self.error {
                        ui.colored_label(egui::Color32::from_rgb(230, 90, 90), err);
                    }
                });
                if self.show_source {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.source_text)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(10),
                            );
                        });
                }
            });
    }

    fn outliner(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("outliner")
            .resizable(true)
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Outliner");
                let mut clicked = None;
                match &self.expansion {
                    Some(exp) => {
                        ui.label(format!("{} nodes", exp.scene.len()));
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                outliner_rows(
                                    ui,
                                    &exp.scene,
                                    exp.scene.root(),
                                    0,
                                    self.selected,
                                    &mut clicked,
                                );
                            });
                    }
                    None => {
                        ui.label("no zone loaded");
                    }
                }
                if clicked.is_some() {
                    self.selected = clicked;
                }
            });
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(290.0)
            .show(ctx, |ui| {
                ui.heading("Inspector");
                let env = PreviewEnv::new(self.night_t, self.forecast);
                let Some(sel) = self.selected else {
                    ui.label("nothing selected");
                    return;
                };
                let Some(exp) = self.expansion.as_mut() else {
                    ui.label("no zone loaded");
                    return;
                };
                let scene = &mut exp.scene;
                if scene.node(sel).is_none() {
                    self.selected = None;
                    return;
                }

                // Name (writes back through Scene::set_name).
                if self.name_edit_node != Some(sel) {
                    self.name_edit = scene
                        .node(sel)
                        .and_then(|n| n.name().map(str::to_owned))
                        .unwrap_or_default();
                    self.name_edit_node = Some(sel);
                }
                ui.horizontal(|ui| {
                    ui.label("name");
                    if ui.text_edit_singleline(&mut self.name_edit).changed() {
                        let new = if self.name_edit.trim().is_empty() {
                            None
                        } else {
                            Some(self.name_edit.clone())
                        };
                        let _ = scene.set_name(sel, new);
                    }
                });
                let kind = kind_label(scene.node(sel).expect("checked above").kind());
                ui.label(format!("kind: {kind}"));

                // Local translation (live write-back via set_translation).
                if let Some(mut t) = scene.transform(sel).map(|t| t.translation()) {
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label("translation");
                        changed |= ui
                            .add(egui::DragValue::new(&mut t.x).speed(0.1).prefix("x "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut t.y).speed(0.1).prefix("y "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut t.z).speed(0.1).prefix("z "))
                            .changed();
                    });
                    if changed {
                        let _ = scene.set_translation(sel, t);
                    }
                }
                ui.label(
                    egui::RichText::new(
                        "edits are not saved to source (source is ground truth)",
                    )
                    .small()
                    .weak(),
                );

                // Effective (inherited) state.
                ui.separator();
                ui.strong("effective state");
                let st = scene.effective_state(sel);
                if let Some(c) = st.base_color {
                    ui.horizontal(|ui| {
                        ui.label("base color");
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
                        let col = egui::Color32::from_rgba_unmultiplied(
                            (c.x.clamp(0.0, 1.0) * 255.0) as u8,
                            (c.y.clamp(0.0, 1.0) * 255.0) as u8,
                            (c.z.clamp(0.0, 1.0) * 255.0) as u8,
                            (c.w.clamp(0.0, 1.0) * 255.0) as u8,
                        );
                        ui.painter().rect_filled(rect, 2.0, col);
                        ui.monospace(format!("({:.2}, {:.2}, {:.2}, {:.2})", c.x, c.y, c.z, c.w));
                    });
                }
                if let Some(e) = st.emissive {
                    ui.label(format!("emissive: ({:.2}, {:.2}, {:.2})", e.x, e.y, e.z));
                }
                if let Some(m) = st.metallic {
                    ui.label(format!("metallic: {m:.2}"));
                }
                if let Some(r) = st.roughness {
                    ui.label(format!("roughness: {r:.2}"));
                }
                if let Some(g) = st.glass {
                    ui.label(format!("glass: {g}"));
                }
                match st.thermal {
                    Some(th) => {
                        ui.label(format!(
                            "thermal: base {:.1} °F, mass {:.0}, sky {:.2}",
                            th.base_temp.0, th.thermal_mass, th.sky_exposure
                        ));
                        ui.label(format!(
                            "preview temp @ t={:.2}: {:.1} °F (ambient {:.1})",
                            env.t,
                            env.display_temp_f(Some(&th)),
                            env.ambient_f
                        ));
                    }
                    None => {
                        ui.label(format!(
                            "thermal: none (reads ambient {:.1} °F)",
                            env.ambient_f
                        ));
                    }
                }
            });
    }

    fn viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.optic_bar(ui);
            ui.separator();

            if let Some(err) = &self.gpu_error {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 90, 90),
                    format!("viewport GPU unavailable: {err}"),
                );
                return;
            }
            let Some(tex) = &self.viewport_tex else {
                ui.label("no frame yet");
                return;
            };

            let avail = ui.available_size();
            let aspect = VIEW_W as f32 / VIEW_H as f32;
            let w = avail.x.min(avail.y * aspect).max(64.0);
            let size = egui::vec2(w, w / aspect);
            let resp = ui.add(
                egui::Image::new(tex)
                    .fit_to_exact_size(size)
                    .sense(egui::Sense::drag()),
            );

            // Blender-spirit camera: drag orbits, middle/shift-drag pans,
            // scroll zooms.
            let delta = resp.drag_delta();
            let shift = ui.input(|i| i.modifiers.shift);
            let panning = resp.dragged_by(egui::PointerButton::Middle)
                || (shift && resp.dragged_by(egui::PointerButton::Primary));
            if panning {
                let (right, up) = self.orbit.basis();
                let k = self.orbit.dist * 0.0016;
                self.orbit.target -= right * delta.x * k;
                self.orbit.target += up * delta.y * k;
            } else if resp.dragged_by(egui::PointerButton::Primary) {
                self.orbit.yaw += delta.x * 0.008;
                self.orbit.pitch = (self.orbit.pitch + delta.y * 0.008).clamp(-1.45, 1.45);
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.raw_scroll_delta.y);
                if scroll != 0.0 {
                    self.orbit.dist =
                        (self.orbit.dist * (0.9985f32).powf(scroll)).clamp(2.0, 2000.0);
                }
            }
            ui.weak("drag: orbit · shift/middle-drag: pan · scroll: zoom");
        });
    }

    fn optic_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("optic");
            egui::ComboBox::from_id_salt("optic")
                .selected_text(format!("{:?}", self.optic))
                .show_ui(ui, |ui| {
                    for m in [OpticMode::Eye, OpticMode::Nv, OpticMode::Thermal] {
                        ui.selectable_value(&mut self.optic, m, format!("{m:?}"));
                    }
                });
            ui.label("palette");
            egui::ComboBox::from_id_salt("palette")
                .selected_text(format!("{:?}", self.palette))
                .show_ui(ui, |ui| {
                    for p in [
                        ThermalPalette::WhiteHot,
                        ThermalPalette::BlackHot,
                        ThermalPalette::ColorblindSafe,
                    ] {
                        ui.selectable_value(&mut self.palette, p, format!("{p:?}"));
                    }
                });
            ui.label("forecast");
            egui::ComboBox::from_id_salt("forecast")
                .selected_text(format!("{:?}", self.forecast))
                .show_ui(ui, |ui| {
                    for f in Forecast::ALL {
                        ui.selectable_value(&mut self.forecast, f, format!("{f:?}"));
                    }
                });
            ui.separator();
            // The Blender-spirit night scrubber: 0 = dusk, 1 = dawn.
            ui.label("dusk");
            ui.add(
                egui::Slider::new(&mut self.night_t, 0.0..=1.0)
                    .show_value(true)
                    .fixed_decimals(2),
            );
            ui.label("dawn");
            ui.weak(format!(
                "ambient {:.1} °F",
                PreviewEnv::new(self.night_t, self.forecast).ambient_f
            ));
        });
    }
}

/// Recursive outliner rows: CollapsingHeaders for interior nodes,
/// selectable labels for leaves. Clicking any row selects its node.
fn outliner_rows(
    ui: &mut egui::Ui,
    scene: &Scene,
    id: NodeId,
    depth: usize,
    selected: Option<NodeId>,
    clicked: &mut Option<NodeId>,
) {
    let Some(node) = scene.node(id) else { return };
    let label = format!(
        "{}  ·  {}",
        node.name().unwrap_or("(unnamed)"),
        kind_label(node.kind())
    );
    let is_selected = selected == Some(id);
    if node.children().is_empty() {
        if ui.selectable_label(is_selected, label).clicked() {
            *clicked = Some(id);
        }
    } else {
        let text = if is_selected {
            egui::RichText::new(label).strong().underline()
        } else {
            egui::RichText::new(label)
        };
        let resp = egui::CollapsingHeader::new(text)
            .id_salt(id)
            .default_open(depth < 1)
            .show(ui, |ui| {
                for &child in node.children() {
                    outliner_rows(ui, scene, child, depth + 1, selected, clicked);
                }
            });
        if resp.header_response.clicked() {
            *clicked = Some(id);
        }
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render_viewport(ctx);

        self.menu_bar(ctx);
        self.status_bar(ctx);
        self.source_panel(ctx);
        self.outliner(ctx);
        self.inspector(ctx);
        self.viewport(ctx);

        // Continuous repaint: the viewport is a live render.
        ctx.request_repaint();
    }
}
