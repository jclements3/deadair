//! Ground vegetation: deterministic grass tufts.
//!
//! The reference footage's ground is not a surface, it's a *structure* —
//! grass clumps that occlude, cast micro-shadows, and give thermal its
//! high-frequency texture. Flat-plane noise can't fake that at grazing
//! angles (it streaks). These are real little cylinders, placed by a
//! world-grid hash so the same field grows the same grass every night,
//! only within a radius of the viewer so the instance count stays sane.

use da_render::draw::{DrawItem, Shape};
use glam::{Mat4, Vec3};

/// Grid pitch between potential tufts, meters.
const CELL_M: f32 = 1.6;
/// Fraction of cells that grow a tuft.
const PRESENCE: f32 = 0.45;

fn hash2(ix: i32, iz: i32, salt: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x8DA6_B343)
        ^ (iz as u32).wrapping_mul(0xD816_3841)
        ^ salt.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Grass tufts within `radius_m` of `center`, deterministic in world space.
/// `ambient_f` sets their thermal read: vegetation holds a little warmth
/// over bare dirt, which is exactly the video's ground texture.
pub fn tufts_around(center: Vec3, radius_m: f32, ambient_f: f32) -> Vec<DrawItem> {
    let mut out = Vec::new();
    let r_cells = (radius_m / CELL_M).ceil() as i32;
    let cx = (center.x / CELL_M).floor() as i32;
    let cz = (center.z / CELL_M).floor() as i32;
    for iz in (cz - r_cells)..=(cz + r_cells) {
        for ix in (cx - r_cells)..=(cx + r_cells) {
            if hash2(ix, iz, 1) > PRESENCE {
                continue;
            }
            // Jitter inside the cell so no grid reads through.
            let jx = hash2(ix, iz, 2) * CELL_M;
            let jz = hash2(ix, iz, 3) * CELL_M;
            let pos = Vec3::new(ix as f32 * CELL_M + jx, 0.0, iz as f32 * CELL_M + jz);
            if pos.distance(center) > radius_m {
                continue;
            }
            let r = 0.05 + hash2(ix, iz, 5) * 0.06;
            let tall = 1.2 + hash2(ix, iz, 4) * 0.8;
            let shade = 0.7 + hash2(ix, iz, 6) * 0.6;
            out.push(DrawItem {
                // A squashed-upright sphere reads as an organic clump from
                // every angle; a cylinder side-on is a tombstone.
                shape: Shape::Sphere { radius: r },
                world: Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, tall, 1.0),
                    glam::Quat::IDENTITY,
                    pos + Vec3::Y * (r * tall * 0.5),
                ),
                albedo: [0.10 * shade, 0.16 * shade, 0.08 * shade],
                emissive: 0.0,
                // Night grass runs COLD: low thermal mass, full sky
                // exposure — it frosts before the dirt does. That's why
                // the footage's clumps read dark against the ground.
                temp_f: ambient_f - 6.0 - hash2(ix, iz, 7) * 5.0,
                glass: false,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tufts_are_deterministic_and_bounded() {
        let a = tufts_around(Vec3::new(40.0, 0.0, 60.0), 30.0, 50.0);
        let b = tufts_around(Vec3::new(40.0, 0.0, 60.0), 30.0, 50.0);
        assert_eq!(a.len(), b.len());
        assert!(!a.is_empty());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.world, y.world, "same field, same grass");
        }
        // Bounded instance count: the llvmpipe budget survives.
        assert!(a.len() < 900, "tuft count sane: {}", a.len());
        for t in &a {
            assert!(
                t.world.w_axis.truncate().distance(Vec3::new(40.0, 0.0, 60.0)) <= 30.5
            );
        }
    }

    #[test]
    fn moving_the_viewer_grows_the_same_world_grass() {
        // A tuft near the overlap of two query circles must appear in both.
        let a = tufts_around(Vec3::new(0.0, 0.0, 0.0), 30.0, 50.0);
        let b = tufts_around(Vec3::new(6.0, 0.0, 0.0), 30.0, 50.0);
        let in_both = a
            .iter()
            .filter(|t| {
                let p = t.world.w_axis.truncate();
                p.distance(Vec3::new(6.0, 0.0, 0.0)) < 24.0
                    && b.iter().any(|u| u.world == t.world)
            })
            .count();
        assert!(in_both > 10, "world-anchored grass: {in_both} shared tufts");
    }
}
