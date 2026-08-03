//! The renderer-facing draw contract.
//!
//! da-render deliberately does NOT walk the scene graph. A cull traversal
//! (da-graph `CullVisitor`) — or the editor, or a test — produces a flat
//! `DrawList`, and every optic pipeline consumes that one truth (SDD §1:
//! optics never get privileged data, they filter it).

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// Geometry primitive shapes the GPU pass instances. Mirrors da-graph
/// drawables; meshes are registered once and referenced by id.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Unit cube scaled by half-extents.
    Box { half: Vec3 },
    /// Y-axis cylinder.
    Cylinder { radius: f32, height: f32 },
    Sphere { radius: f32 },
    /// Registered triangle mesh.
    Mesh { id: u32 },
    /// Ground plane patch (y = 0), extent in meters.
    GroundPatch { half: f32 },
}

/// Everything an optic pass needs to know about one drawable instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawItem {
    pub shape: Shape,
    pub world: Mat4,
    /// Base surface color (naked eye / NV geometry pass).
    pub albedo: [f32; 3],
    /// Emissive light sources bloom in NV (streetlights, windows, IR beam).
    pub emissive: f32,
    /// Current display temperature in °F from the thermal sim. The thermal
    /// pass sees ONLY this — geometry with no ΔT vanishes on its own.
    pub temp_f: f32,
    /// Glass renders transparent to eye/NV but opaque (surface temp) in
    /// thermal (FR-O4).
    pub glass: bool,
}

/// One frame's worth of world, as every pipeline sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawList {
    pub items: Vec<DrawItem>,
    /// Ambient air temperature °F (thermal AGC floor, sky rendering).
    pub ambient_f: f32,
    /// Sky reads as this in thermal — uniform cold, well below ambient.
    pub sky_temp_f: f32,
    /// Moon-phase × cloud illumination factor, 0..1 (naked eye + NV gain).
    pub moonlight: f32,
    /// Live residual-heat decals (bedding, tracks, barrel) — thermal only.
    pub heat_decals: Vec<HeatDecal>,
    /// IR retro-reflective eyeshine points — night-vision only.
    ///
    /// Caller policy (SDD §7.3, `assets/reference/optics-look.md`): emit one
    /// entry per living animal that is *facing the shooter* and unoccluded.
    /// Zombies must never be emitted — dead retinas do not retro-reflect,
    /// and that absence is a deliberate NV-side identification tell,
    /// complementing thermal's "no ΔT" tell.
    #[serde(default)]
    pub eyeshine: Vec<EyeShine>,
}

/// A decaying warm spot on the ground (SDD §2.3), thermal pass only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HeatDecal {
    pub pos: Vec3,
    pub radius_m: f32,
    /// Degrees F above ambient right now.
    pub delta_f: f32,
}

/// A pair of retro-reflecting eyes lit by the optic's IR illuminator.
///
/// Rendered only by the NV pipeline: thermal has no eyeshine channel at all
/// (retro-reflection is light, not heat) and the naked eye sees nothing
/// because there is no visible-spectrum beam to bounce back.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EyeShine {
    /// World position of the eye pair (roughly the animal's head).
    pub pos: Vec3,
    /// Return strength 0..1 — species tapetum quality × beam alignment ×
    /// falloff. 1.0 is a hog staring straight down the barrel at 30 m.
    pub strength: f32,
}

/// Camera for a frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Camera {
    pub eye: Vec3,
    pub look: Vec3,
    pub up: Vec3,
    pub fov_y_deg: f32,
    pub aspect: f32,
}

impl Camera {
    pub fn view_proj(&self) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye, self.look, self.up);
        let proj = Mat4::perspective_rh(self.fov_y_deg.to_radians(), self.aspect, 0.1, 600.0);
        proj * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_projects_forward_point_inside_clip() {
        let cam = Camera {
            eye: Vec3::new(0.0, 1.6, 0.0),
            look: Vec3::new(0.0, 1.6, -10.0),
            up: Vec3::Y,
            fov_y_deg: 60.0,
            aspect: 16.0 / 9.0,
        };
        let clip = cam.view_proj() * Vec3::new(0.0, 1.6, -10.0).extend(1.0);
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 0.01 && ndc.y.abs() < 0.01);
        assert!(ndc.z > 0.0 && ndc.z < 1.0);
    }

    #[test]
    fn draw_list_ron_round_trip() {
        let dl = DrawList {
            items: vec![DrawItem {
                shape: Shape::Box { half: Vec3::splat(0.5) },
                world: Mat4::IDENTITY,
                albedo: [0.5, 0.4, 0.3],
                emissive: 0.0,
                temp_f: 55.0,
                glass: false,
            }],
            ambient_f: 50.0,
            sky_temp_f: 10.0,
            moonlight: 0.4,
            heat_decals: vec![],
            eyeshine: vec![EyeShine {
                pos: Vec3::new(1.0, 0.5, -8.0),
                strength: 0.8,
            }],
        };
        let s = ron::to_string(&dl).expect("serialize");
        let back: DrawList = ron::from_str(&s).expect("deserialize");
        assert_eq!(back.items.len(), 1);
        assert_eq!(back.items[0].temp_f, 55.0);
        assert_eq!(back.eyeshine.len(), 1);
    }
}
