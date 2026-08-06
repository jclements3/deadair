//! `gripitt` — the GRIPITT booth app: a windowed player that shows the CSG
//! strike/scene reel as **1024^2 IR imagery on the left** with the **slide-deck
//! bullet points on the right** (no game controls). Self-running and looping;
//! F fullscreen, Esc/Q quit, Space pause, arrows scrub.
//!
//! The IR frames are **pre-rendered** headlessly (`strike_demo --frames <dir>`)
//! so the app does no wgpu rendering itself — it only draws egui and blits the
//! current frame as a texture. Robust on software GPUs; touches no shared
//! darkair file. Shares the reel script/timing with the renderer via `#[path]`.
//!
//! Frame dir resolves from `$GRIPITT_FRAMES`, else `./gripitt_frames`, else a
//! dir next to the binary. `fps.txt` in that dir sets playback rate.
//!
//! ```text
//! cargo run --release -p darkair --bin strike_demo -- --frames gripitt_frames 12
//! cargo run --release -p darkair --bin gripitt
//! ```

#[path = "../gripitt_scene.rs"]
mod scene;
use scene::{locate, script, total_dur, Seg, PANEL_W, VIEW};

use eframe::egui;
use std::path::PathBuf;
use std::time::Instant;

struct Gripitt {
    dir: PathBuf,
    fps: f32,
    nframes: usize,
    segs: Vec<Seg>,
    total: f32,
    start: Instant,
    paused_at: Option<f32>,
    clock: f32,
    cur: Option<usize>,
    tex: Option<egui::TextureHandle>,
}

fn find_frames_dir() -> PathBuf {
    if let Ok(d) = std::env::var("GRIPITT_FRAMES") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return p;
        }
    }
    let cwd = PathBuf::from("gripitt_frames");
    if cwd.is_dir() {
        return cwd;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("gripitt_frames");
            if p.is_dir() {
                return p;
            }
        }
    }
    cwd
}

impl Gripitt {
    fn new() -> Self {
        let dir = find_frames_dir();
        let fps = std::fs::read_to_string(dir.join("fps.txt"))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(12.0);
        let nframes = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path().extension().map(|x| x == "png").unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        let segs = script();
        let total = total_dur(&segs);
        Self {
            dir,
            fps,
            nframes,
            segs,
            total,
            start: Instant::now(),
            paused_at: None,
            clock: 0.0,
            cur: None,
            tex: None,
        }
    }

    fn load_frame(&self, idx: usize) -> Option<egui::ColorImage> {
        let path = self.dir.join(format!("f{idx:05}.png"));
        let img = image::open(&path).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            img.as_raw(),
        ))
    }
}

const BLUE: egui::Color32 = egui::Color32::from_rgb(0x4e, 0xa3, 0xff);
const BLUE2: egui::Color32 = egui::Color32::from_rgb(0x7e, 0xc7, 0xff);
const INK: egui::Color32 = egui::Color32::from_rgb(0xd7, 0xe2, 0xee);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x8f, 0xa3, 0xba);
const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x10, 0x17);
const VIEW_BG: egui::Color32 = egui::Color32::from_rgb(0x05, 0x07, 0x0b);

impl eframe::App for Gripitt {
    fn update(&mut self, ctx: &egui::Context, _ef: &mut eframe::Frame) {
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Q) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if i.key_pressed(egui::Key::F) {
                let full = i.viewport().fullscreen.unwrap_or(false);
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            if i.key_pressed(egui::Key::Space) {
                self.paused_at = match self.paused_at {
                    Some(_) => {
                        self.start = Instant::now()
                            - std::time::Duration::from_secs_f32(self.clock);
                        None
                    }
                    None => Some(self.clock),
                };
            }
        });

        if let Some(p) = self.paused_at {
            self.clock = p;
        } else {
            self.clock = self.start.elapsed().as_secs_f32() % self.total.max(0.001);
        }
        let (idx, tl) = locate(&self.segs, self.clock);
        let seg = &self.segs[idx];

        // pick + (re)load the current pre-rendered frame
        if self.nframes > 0 {
            let fi = ((self.clock * self.fps) as usize).min(self.nframes - 1);
            if self.cur != Some(fi) {
                if let Some(img) = self.load_frame(fi) {
                    match &mut self.tex {
                        Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                        None => {
                            self.tex =
                                Some(ctx.load_texture("ir", img, egui::TextureOptions::NEAREST))
                        }
                    }
                    self.cur = Some(fi);
                }
            }
        }

        egui::SidePanel::right("bullets")
            .exact_width(PANEL_W as f32)
            .resizable(false)
            .frame(
                egui::Frame::none().fill(PANEL_BG).inner_margin(egui::Margin {
                    left: 34.0,
                    right: 30.0,
                    top: 30.0,
                    bottom: 26.0,
                }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("GEOMETRIC").size(15.0).color(DIM).strong());
                    ui.label(egui::RichText::new("RiPiTT").size(15.0).color(BLUE2).strong());
                });
                ui.add_space(26.0);
                ui.label(egui::RichText::new(seg.title).size(30.0).color(BLUE).strong());
                ui.add_space(22.0);
                for b in seg.bullets {
                    ui.horizontal_top(|ui| {
                        ui.label(egui::RichText::new("▸").size(20.0).color(BLUE));
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(*b).size(20.0).color(INK));
                    });
                    ui.add_space(15.0);
                }
                if let Some((_, _, c)) = seg.caps.iter().find(|(a, b, _)| tl >= *a && tl <= *b) {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(*c).size(16.0).italics().color(BLUE2));
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(egui::RichText::new("VALKYRIE ENTERPRISES").size(12.0).color(DIM));
                    ui.add_space(8.0);
                    let p = self.clock / self.total.max(0.001);
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 4.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                    let mut fill = rect;
                    fill.set_width(rect.width() * p);
                    ui.painter().rect_filled(fill, 2.0, BLUE);
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(VIEW_BG))
            .show(ctx, |ui| {
                if let Some(t) = &self.tex {
                    let avail = ui.available_size();
                    let s = avail.x.min(avail.y);
                    ui.centered_and_justified(|ui| {
                        ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(
                            t.id(),
                            egui::vec2(s, s),
                        )));
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(
                                "no frames — run: strike_demo --frames gripitt_frames 12",
                            )
                            .color(DIM),
                        );
                    });
                }
            });

        if self.paused_at.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(1.0 / self.fps.max(1.0)));
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([(VIEW + PANEL_W) as f32, VIEW as f32])
            .with_min_inner_size([1200.0, 760.0])
            .with_title("Geometric RiPiTT"),
        ..Default::default()
    };
    eframe::run_native(
        "gripitt",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(Gripitt::new()))
        }),
    )
}
