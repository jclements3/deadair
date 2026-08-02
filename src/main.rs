//! deadair — silent night-hunting business sim.
//!
//! Usage:
//!   deadair demo            Run the full demo (default)
//!   deadair hunt [scene]    Simulate a hunt (default: Abandoned Farm)
//!   deadair editor          Interactive scene editor

use deadair::{
    editor::SceneEditor,
    hunt::{HuntConfig, HuntSimulation},
    scene::Scene,
    thermal::ThermalOptics,
    world::World,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("hunt")   => cmd_hunt(args.get(2).map(String::as_str)),
        Some("editor") => cmd_editor(),
        _              => cmd_demo(),
    }
}

// ── demo ─────────────────────────────────────────────────────────────────────

fn cmd_demo() {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  DEADAIR  —  Night-Hunting Business Sim  (Rust)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ── 1. Scene editor ──────────────────────────────────────────────────────
    println!("\n[1/3] Scene editor — loading 'Abandoned Farm'\n");
    let mut editor = SceneEditor::with_default_scene();
    println!("{}", editor.list_nodes());
    println!("{}", editor.render_map());

    // Demonstrate programmatic editing (OpenSCAD / Blender spirit)
    editor.add_zombie(80.0, 80.0, 1.2);
    editor.add_box(60.0, 60.0, 0.0, 4.0, 4.0, 2.0);
    println!("Added extra zombie and crate.  New map:\n");
    println!("{}", editor.render_map());

    // ── 2. Thermal optics explainer ──────────────────────────────────────────
    println!("[2/3] Thermal-honest optics\n");
    explain_thermal(&editor.scene);

    // ── 3. Hunt simulation + P&L ─────────────────────────────────────────────
    println!("[3/3] Running hunt simulation …\n");
    let world = World::from_scene(&editor.scene);
    let zombie_count = world.zombie_count();
    println!("  Scene: {}  |  Zombies: {}  |  Optic: budget 80 mK NETD\n",
             editor.scene.name, zombie_count);

    let config = HuntConfig::default();
    let mut sim = HuntSimulation::new(world, config);
    let mut rng = rand::thread_rng();
    sim.run(&mut rng);

    if sim.log.is_empty() {
        println!("  (no engagements this hunt)");
    } else {
        for entry in &sim.log {
            println!("  {entry}");
        }
    }

    println!();
    let result = sim.finish();
    println!("  Ticks: {}  |  Shots: {}  |  Kills: {}/{}",
             result.ticks_elapsed, result.shots_fired, result.kills, zombie_count);
    println!();
    println!("{}", result.ledger.report());
    println!("  Tip: run 'deadair editor' for the interactive scene editor.");
}

// ── hunt ─────────────────────────────────────────────────────────────────────

fn cmd_hunt(scene_path: Option<&str>) {
    let scene = match scene_path {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(json) => match Scene::from_json(&json) {
                Ok(s) => { println!("Loaded scene: {}", s.name); s }
                Err(e) => { eprintln!("Failed to parse '{}': {e}", path); std::process::exit(1); }
            },
            Err(e) => { eprintln!("Cannot read '{}': {e}", path); std::process::exit(1); }
        },
        None => {
            println!("No scene file given — using built-in 'Abandoned Farm'.");
            Scene::abandoned_farm()
        }
    };

    let world = World::from_scene(&scene);
    let config = HuntConfig::default();
    println!("Zombies in scene: {}", world.zombie_count());

    let mut sim = HuntSimulation::new(world, config);
    let mut rng = rand::thread_rng();
    sim.run(&mut rng);

    for entry in &sim.log {
        println!("{entry}");
    }

    let result = sim.finish();
    println!("\nKills: {} / Shots: {} / Ticks: {}",
             result.kills, result.shots_fired, result.ticks_elapsed);
    println!("{}", result.ledger.report());
}

// ── editor ───────────────────────────────────────────────────────────────────

fn cmd_editor() {
    let mut editor = SceneEditor::with_default_scene();
    editor.run_interactive();
}

// ── thermal explainer ────────────────────────────────────────────────────────

fn explain_thermal(scene: &Scene) {
    use deadair::entity::Entity;
    use deadair::vec::Vec3;

    let optics_budget = ThermalOptics::budget();
    let optics_mil    = ThermalOptics::military_grade();
    let ambient       = scene.ambient_temp_c;

    let observer = Entity::hunter(0, Vec3::zero());
    let human    = Entity::hunter(1, Vec3::new(50.0, 0.0, 0.0));
    // Cold zombie: equilibrated to ambient + tiny decomp offset (0.1 °C)
    let zombie   = Entity::zombie(2, Vec3::new(50.0, 0.0, 0.0), ambient, 0.1);

    println!("  Ambient temp   : {ambient:.1} °C");
    println!("  Human surface  : {:.1} °C  (ΔT = {:.1} °C)", human.temperature_c,
             human.temperature_c - ambient);
    println!("  Zombie surface : {:.1} °C  (ΔT = {:.1} °C  — decomp heat only)",
             zombie.temperature_c, zombie.temperature_c - ambient);
    println!();

    for (label, optics) in [("Budget 80 mK", &optics_budget), ("Mil-grade 25 mK", &optics_mil)] {
        let p_human = optics.detection_probability(&observer, 0.0, &human,  ambient);
        let p_zombie = optics.detection_probability(&observer, 0.0, &zombie, ambient);
        println!("  {label} at 50 m:");
        println!("    P(detect human)  = {:.1} %", p_human  * 100.0);
        println!("    P(detect zombie) = {:.1} %", p_zombie * 100.0);
        println!("    → zombie is {:.1}× harder to spot", p_human / p_zombie.max(1e-6));
        println!();
    }
}
