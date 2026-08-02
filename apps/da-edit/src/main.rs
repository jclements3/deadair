//! da-edit — the DeadAir scene/animation editor (SDD §10).
//!
//! Blender-spirit direct manipulation (outliner, viewport with orbit
//! camera and a night-`t` scrubber, inspector) over OpenSCAD-spirit
//! scenes-as-code (the zone RON source panel; text is ground truth).
//!
//! All pure logic lives in the `da_edit` library crate (`convert`,
//! `preview`); this binary is the egui shell.

mod app;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DeadAir Editor")
            .with_inner_size([1440.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native(
        "da-edit",
        options,
        Box::new(|_cc| Ok(Box::new(app::EditorApp::new()))),
    )
}
