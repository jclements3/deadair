//! da-edit — pure logic for the DeadAir scene editor (SDD §10).
//!
//! The editor binary (`main.rs` / `app.rs`) is a thin egui shell over the
//! modules here:
//!
//! - [`convert`] — turns da-graph cull output ([`da_graph::RenderLeaf`])
//!   into the renderer's flat [`da_render::DrawList`].
//! - [`preview`] — the editor's thermal-preview environment: ambient,
//!   sky, moonlight, and an approximate per-object display temperature at
//!   any night-`t` under any forecast.
//!
//! Text is ground truth: zones are edited as `*.zone.ron` source and
//! compiled to graphs with [`da_param::expand_zone`]; in-editor graph
//! edits are session-only overrides on the build artifact.

pub mod convert;
pub mod preview;

#[cfg(test)]
mod source_loop_tests {
    //! The OpenSCAD loop: text → parse → expand, and re-expand after a
    //! text edit.

    use std::path::PathBuf;

    use da_graph::Scene;
    use da_param::{expand_zone, parse_zone_str};

    fn home_farm_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/zones/home_farm.zone.ron")
    }

    fn node_names(scene: &Scene) -> Vec<String> {
        scene
            .nodes()
            .map(|n| n.name().unwrap_or("").to_owned())
            .collect()
    }

    #[test]
    fn home_farm_parses_and_expands() {
        let text = std::fs::read_to_string(home_farm_path()).expect("read home_farm");
        let src = parse_zone_str(&text).expect("parse");
        let exp = expand_zone(&src).expect("expand");
        assert_eq!(src.name, "Home Farm");
        assert!(exp.scene.len() > 1, "expanded scene has nodes");
        assert!(!exp.spawn_points.is_empty(), "home farm has spawns");
    }

    #[test]
    fn seed_edit_keeps_node_names_and_count() {
        let text = std::fs::read_to_string(home_farm_path()).expect("read home_farm");
        let exp_a = expand_zone(&parse_zone_str(&text).expect("parse a")).expect("expand a");

        let edited = text.replace("seed: 1001", "seed: 4242");
        assert_ne!(edited, text, "seed edit must change the source text");
        let exp_b = expand_zone(&parse_zone_str(&edited).expect("parse b")).expect("expand b");

        // Structure comes from the source alone: a seed change moves
        // jitter but never node counts or names (da-param contract).
        assert_eq!(exp_a.scene.len(), exp_b.scene.len());
        assert_eq!(node_names(&exp_a.scene), node_names(&exp_b.scene));
    }
}
