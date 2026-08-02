//! Interactive scene editor.
//!
//! Inspired by the OpenSCAD workflow (text-driven, parametric, composable) and
//! Blender's object-manipulation model (select → transform → confirm).
//!
//! Objects are identified by their string `id` field.  The editor wraps a
//! [`Scene`] document and exposes add / remove / move / list / render / save /
//! load operations.  The built-in ASCII map gives instant spatial feedback.

use std::io::{self, BufRead, Write};
use crate::{
    scene::{NodeKind, Scene, SceneNode},
    vec::Vec3,
};

/// Wraps a [`Scene`] and provides editing operations.
pub struct SceneEditor {
    pub scene: Scene,
    /// True if there are unsaved changes.
    pub dirty: bool,
}

impl SceneEditor {
    pub fn new(scene: Scene) -> Self { Self { scene, dirty: false } }

    pub fn with_default_scene() -> Self { Self::new(Scene::abandoned_farm()) }

    // ── Object manipulation ─────────────────────────────────────────────────

    /// Add a zombie at (`x`, `y`).  Returns the node's index.
    pub fn add_zombie(&mut self, x: f32, y: f32, ambient_offset_c: f32) -> usize {
        let idx = self.scene.nodes.len();
        self.scene.nodes.push(SceneNode {
            id: Some(format!("zombie_{idx}")),
            kind: NodeKind::Zombie { ambient_offset_c },
            translate: Some(Vec3::new(x, y, 0.0)),
            rotate_y_deg: None,
            children: vec![],
        });
        self.dirty = true;
        idx
    }

    /// Add an axis-aligned box obstacle.  Returns the node's index.
    pub fn add_box(
        &mut self,
        x: f32, y: f32, z: f32,
        width: f32, depth: f32, height: f32,
    ) -> usize {
        let idx = self.scene.nodes.len();
        self.scene.nodes.push(SceneNode {
            id: Some(format!("box_{idx}")),
            kind: NodeKind::Box { size: [width, depth, height] },
            translate: Some(Vec3::new(x, y, z)),
            rotate_y_deg: None,
            children: vec![],
        });
        self.dirty = true;
        idx
    }

    /// Remove the node with the given id.  Returns `true` if found.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.scene.nodes.len();
        self.scene.nodes.retain(|n| n.id.as_deref() != Some(id));
        let removed = self.scene.nodes.len() < before;
        if removed { self.dirty = true; }
        removed
    }

    /// Reposition a node.  Returns `true` if found.
    pub fn move_node(&mut self, id: &str, x: f32, y: f32, z: f32) -> bool {
        for node in &mut self.scene.nodes {
            if node.id.as_deref() == Some(id) {
                node.translate = Some(Vec3::new(x, y, z));
                self.dirty = true;
                return true;
            }
        }
        false
    }

    // ── Visualisation ───────────────────────────────────────────────────────

    /// Render a 20×20 character ASCII top-down map of the scene.
    ///
    /// Symbol key:  `Z` zombie  `H` hunter spawn  `#` box  `T` tree/cylinder
    ///              `L` light   `.` empty
    pub fn render_map(&self) -> String {
        const COLS: usize = 20;
        const ROWS: usize = 20;
        let mut grid = vec![vec!['.'; COLS]; ROWS];

        // Derive scene bounds from the first Terrain node, or use 100×100.
        let (w, d) = self.scene.nodes.iter().find_map(|n| {
            if let NodeKind::Terrain { size, .. } = &n.kind {
                Some((size[0], size[1]))
            } else {
                None
            }
        }).unwrap_or((100.0, 100.0));

        for node in &self.scene.nodes {
            let pos = node.world_position(Vec3::zero());
            let col = ((pos.x / w) * COLS as f32) as usize;
            let row = ((pos.y / d) * ROWS as f32) as usize;
            let col = col.min(COLS - 1);
            let row = row.min(ROWS - 1);
            let ch = match &node.kind {
                NodeKind::Zombie { .. }   => 'Z',
                NodeKind::HunterSpawn     => 'H',
                NodeKind::Box { .. }      => '#',
                NodeKind::Cylinder { .. } => 'T',
                NodeKind::Light { .. }    => 'L',
                NodeKind::Terrain { .. }  => continue,
            };
            grid[row][col] = ch;
        }

        let mut out = String::new();
        out.push_str("     0         1\n");
        out.push_str("     01234567890123456789\n");
        for (r, row) in grid.iter().enumerate() {
            let row_str: String = row.iter().collect();
            out.push_str(&format!("{r:3}  {row_str}\n"));
        }
        out.push_str("\n  Z zombie  H hunter-spawn  # box  T tree  L light\n");
        out
    }

    /// Format all scene nodes as a table string.
    pub fn list_nodes(&self) -> String {
        if self.scene.nodes.is_empty() {
            return "  (empty scene)\n".into();
        }
        let mut out = format!(
            "  {:<22} {:<18}  {}\n",
            "ID", "POSITION", "KIND"
        );
        out.push_str(&format!("  {}\n", "─".repeat(60)));
        for node in &self.scene.nodes {
            let id = node.id.as_deref().unwrap_or("(anon)");
            let pos = node.translate
                .map(|t| format!("({:.1}, {:.1}, {:.1})", t.x, t.y, t.z))
                .unwrap_or_else(|| "(origin)".into());
            let kind = match &node.kind {
                NodeKind::Zombie { ambient_offset_c } =>
                    format!("Zombie  ΔT +{ambient_offset_c:.1} °C"),
                NodeKind::HunterSpawn => "HunterSpawn".into(),
                NodeKind::Box { size } =>
                    format!("Box  {:.1}×{:.1}×{:.1} m", size[0], size[1], size[2]),
                NodeKind::Cylinder { radius_m, height_m } =>
                    format!("Cylinder  r={radius_m:.1} m  h={height_m:.1} m"),
                NodeKind::Terrain { size, elevation } =>
                    format!("Terrain  {:.0}×{:.0} m  elev={elevation:.1}", size[0], size[1]),
                NodeKind::Light { colour_temp_k, power_w } =>
                    format!("Light  {colour_temp_k:.0} K  {power_w:.0} W"),
            };
            out.push_str(&format!("  {id:<22} {pos:<18}  {kind}\n"));
        }
        out
    }

    // ── Persistence ─────────────────────────────────────────────────────────

    /// Serialize the current scene to a pretty-printed JSON string.
    pub fn save_to_json(&self) -> Result<String, serde_json::Error> {
        self.scene.to_json()
    }

    /// Replace the current scene from a JSON string.
    pub fn load_from_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        self.scene = Scene::from_json(json)?;
        self.dirty = false;
        Ok(())
    }

    // ── Interactive REPL ────────────────────────────────────────────────────

    /// Run an interactive editing session on stdin / stdout.
    pub fn run_interactive(&mut self) {
        print_editor_help();
        let stdin = io::stdin();
        loop {
            print!("deadair-editor> ");
            io::stdout().flush().ok();

            let mut line = String::new();
            if stdin.lock().read_line(&mut line).is_err() || line.is_empty() {
                break;
            }
            let line = line.trim();
            if line.is_empty() { continue; }

            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.as_slice() {
                ["help"] | ["h"] => print_editor_help(),

                ["list"] | ["ls"] => print!("{}", self.list_nodes()),

                ["map"] => print!("{}", self.render_map()),

                ["add", "zombie", x, y] => {
                    match (x.parse::<f32>(), y.parse::<f32>()) {
                        (Ok(x), Ok(y)) => {
                            let i = self.add_zombie(x, y, 0.8);
                            println!("Added zombie at ({x}, {y}) → index {i}");
                        }
                        _ => println!("usage: add zombie <x> <y>"),
                    }
                }

                ["add", "box", x, y, z, w, d, h] => {
                    let nums: Vec<f32> = [x, y, z, w, d, h]
                        .iter().filter_map(|s| s.parse().ok()).collect();
                    if nums.len() == 6 {
                        let i = self.add_box(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]);
                        println!("Added box → index {i}");
                    } else {
                        println!("usage: add box <x> <y> <z> <width> <depth> <height>");
                    }
                }

                ["remove", id] | ["rm", id] => {
                    if self.remove(id) {
                        println!("Removed '{id}'");
                    } else {
                        println!("Node '{id}' not found");
                    }
                }

                ["move", id, x, y, z] => {
                    let nums: Vec<f32> = [x, y, z].iter().filter_map(|s| s.parse().ok()).collect();
                    if nums.len() == 3 {
                        if self.move_node(id, nums[0], nums[1], nums[2]) {
                            println!("Moved '{id}' to ({}, {}, {})", nums[0], nums[1], nums[2]);
                        } else {
                            println!("Node '{id}' not found");
                        }
                    } else {
                        println!("usage: move <id> <x> <y> <z>");
                    }
                }

                ["save", path] => {
                    match self.save_to_json().and_then(|json| {
                        std::fs::write(path, &json).map_err(|e| {
                            serde_json::Error::io(e)
                        })
                    }) {
                        Ok(_) => println!("Saved to '{path}'"),
                        Err(e) => println!("Error: {e}"),
                    }
                }

                ["load", path] => {
                    match std::fs::read_to_string(path) {
                        Ok(json) => match self.load_from_json(&json) {
                            Ok(_) => println!("Loaded '{path}' ({} nodes)", self.scene.nodes.len()),
                            Err(e) => println!("Parse error: {e}"),
                        },
                        Err(e) => println!("Read error: {e}"),
                    }
                }

                ["quit"] | ["q"] | ["exit"] => {
                    if self.dirty {
                        println!("Warning: unsaved changes (use 'save <path>' first)");
                    }
                    break;
                }

                _ => println!("Unknown command '{}'.  Type 'help'.", parts[0]),
            }
        }
    }
}

fn print_editor_help() {
    println!(
        "\
deadair scene editor  (OpenSCAD / Blender spirit, JSON-backed)
══════════════════════════════════════════════════════════════════
  list / ls                         List all nodes
  map                               Render ASCII top-down map
  add zombie <x> <y>                Place a cold zombie
  add box <x> <y> <z> <w> <d> <h>  Place an obstacle box
  move <id> <x> <y> <z>            Reposition a node
  remove / rm <id>                  Delete a node
  save <path.json>                  Persist scene to disk
  load <path.json>                  Load scene from disk
  help / h                          Show this message
  quit / q                          Exit the editor
══════════════════════════════════════════════════════════════════"
    );
}
