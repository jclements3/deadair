//! Scene graph format — inspired by OpenSCAD's CSG tree and Blender's object
//! hierarchy.
//!
//! Scenes are stored as JSON documents where every node carries an optional
//! transform and zero or more child nodes, enabling composable, parametric
//! scene definitions.  The `NodeKind` enum provides the available primitives
//! and entity spawn points.
//!
//! Example (abbreviated):
//! ```json
//! {
//!   "name": "Abandoned Farm",
//!   "ambient_temp_c": 5.0,
//!   "nodes": [
//!     { "id": "ground",       "kind": { "type": "terrain", "size": [100.0,100.0], "elevation": 0.0 } },
//!     { "id": "barn",         "kind": { "type": "box", "size": [12.0,8.0,5.0] }, "translate": {"x":45,"y":40,"z":0} },
//!     { "id": "zombie_0",     "kind": { "type": "zombie", "ambient_offset_c": 0.8 }, "translate": {"x":50,"y":50,"z":0} },
//!     { "id": "hunter_spawn", "kind": { "type": "hunter_spawn" } }
//!   ]
//! }
//! ```

use serde::{Deserialize, Serialize};
use crate::vec::Vec3;

/// The primitive or entity type represented by a scene node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    /// Flat ground plane.
    Terrain {
        /// [width, depth] in metres.
        size: [f32; 2],
        #[serde(default)]
        elevation: f32,
    },
    /// Axis-aligned solid box (building, wall, crate …).
    Box {
        /// [width, depth, height] in metres.
        size: [f32; 3],
    },
    /// Vertical cylinder (tree trunk, pillar …).
    Cylinder {
        radius_m: f32,
        height_m: f32,
    },
    /// A zombie entity.
    /// Its surface temperature = scene `ambient_temp_c` + `ambient_offset_c`.
    Zombie {
        /// Small decomposition heat above ambient (typically 0.5–1.5 °C).
        #[serde(default)]
        ambient_offset_c: f32,
    },
    /// The hunter's starting position.
    HunterSpawn,
    /// A point-light / heat source (campfire, vehicle …).
    Light {
        /// Colour temperature in Kelvin (for IR emission modelling).
        colour_temp_k: f32,
        /// Radiated power in Watts.
        power_w: f32,
    },
}

/// A node in the scene graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    /// Optional human-readable identifier (used by the editor for selection).
    #[serde(default)]
    pub id: Option<String>,

    /// What this node represents.
    pub kind: NodeKind,

    /// Translation relative to the parent node (or world origin for root nodes).
    #[serde(default)]
    pub translate: Option<Vec3>,

    /// Y-axis rotation in degrees applied after translation.
    #[serde(default)]
    pub rotate_y_deg: Option<f32>,

    /// Child nodes (OpenSCAD-style nesting; transforms compose).
    #[serde(default)]
    pub children: Vec<SceneNode>,
}

impl SceneNode {
    /// World-space position of this node given the accumulated `parent_offset`.
    pub fn world_position(&self, parent_offset: Vec3) -> Vec3 {
        let t = self.translate.unwrap_or(Vec3::zero());
        parent_offset + t
    }
}

/// The top-level scene document — the unit saved and loaded by the editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    /// Ambient background temperature in °C (affects zombie ΔT).
    pub ambient_temp_c: f32,
    /// Root-level nodes in the scene graph.
    pub nodes: Vec<SceneNode>,
}

impl Scene {
    pub fn new(name: impl Into<String>, ambient_temp_c: f32) -> Self {
        Self { name: name.into(), ambient_temp_c, nodes: Vec::new() }
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// The built-in "Abandoned Farm" starter scene used by the demo and tests.
    pub fn abandoned_farm() -> Self {
        let mut s = Self::new("Abandoned Farm", 5.0);

        s.nodes.push(SceneNode {
            id: Some("ground".into()),
            kind: NodeKind::Terrain { size: [100.0, 100.0], elevation: 0.0 },
            translate: None,
            rotate_y_deg: None,
            children: vec![],
        });

        // Barn
        s.nodes.push(SceneNode {
            id: Some("barn".into()),
            kind: NodeKind::Box { size: [12.0, 8.0, 5.0] },
            translate: Some(Vec3::new(45.0, 40.0, 0.0)),
            rotate_y_deg: None,
            children: vec![],
        });

        // A few trees (cylinders)
        for (i, (x, y)) in [(20.0f32, 30.0), (25.0, 35.0), (15.0, 55.0)].iter().enumerate() {
            s.nodes.push(SceneNode {
                id: Some(format!("tree_{i}")),
                kind: NodeKind::Cylinder { radius_m: 0.3, height_m: 6.0 },
                translate: Some(Vec3::new(*x, *y, 0.0)),
                rotate_y_deg: None,
                children: vec![],
            });
        }

        // Three zombies at different positions (all cold — equilibrated to ambient)
        for (i, (x, y)) in [(50.0f32, 50.0), (70.0, 30.0), (30.0, 65.0)].iter().enumerate() {
            s.nodes.push(SceneNode {
                id: Some(format!("zombie_{i}")),
                kind: NodeKind::Zombie { ambient_offset_c: 0.1 },
                translate: Some(Vec3::new(*x, *y, 0.0)),
                rotate_y_deg: None,
                children: vec![],
            });
        }

        // Hunter spawn (south-west corner)
        s.nodes.push(SceneNode {
            id: Some("hunter_spawn".into()),
            kind: NodeKind::HunterSpawn,
            translate: Some(Vec3::new(5.0, 5.0, 0.0)),
            rotate_y_deg: None,
            children: vec![],
        });

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_json_round_trip() {
        let original = Scene::abandoned_farm();
        let json = original.to_json().expect("serialise");
        let restored: Scene = Scene::from_json(&json).expect("deserialise");
        assert_eq!(original.name, restored.name);
        assert_eq!(original.nodes.len(), restored.nodes.len());
        assert!((original.ambient_temp_c - restored.ambient_temp_c).abs() < 0.001);
    }

    #[test]
    fn abandoned_farm_has_three_zombies() {
        let s = Scene::abandoned_farm();
        let zombies = s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Zombie { .. })).count();
        assert_eq!(zombies, 3);
    }

    #[test]
    fn abandoned_farm_has_hunter_spawn() {
        let s = Scene::abandoned_farm();
        assert!(s.nodes.iter().any(|n| matches!(n.kind, NodeKind::HunterSpawn)));
    }
}
