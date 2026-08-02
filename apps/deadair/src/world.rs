//! World state for a night hunt: placeholder scene until the parametric
//! zone loader lands, plus the camera-facing DrawList assembly.

use da_render::draw::{DrawItem, DrawList, Shape};
use glam::{Mat4, Vec3};

/// A temporary hand-built patch of Home Farm: ground, barn-ish boxes, a
/// couple of warm bodies, one ambient-temperature stranger.
pub fn placeholder_scene(ambient_f: f32) -> DrawList {
    let mut items = vec![DrawItem {
        shape: Shape::GroundPatch { half: 120.0 },
        world: Mat4::IDENTITY,
        albedo: [0.22, 0.28, 0.18],
        emissive: 0.0,
        temp_f: ambient_f - 3.0,
        glass: false,
    }];

    // Barn: body + metal roof (roof reads colder than ambient on clear nights).
    items.push(DrawItem {
        shape: Shape::Box {
            half: Vec3::new(6.0, 3.0, 4.0),
        },
        world: Mat4::from_translation(Vec3::new(20.0, 3.0, -35.0)),
        albedo: [0.45, 0.2, 0.15],
        emissive: 0.0,
        temp_f: ambient_f + 4.0,
        glass: false,
    });
    items.push(DrawItem {
        shape: Shape::Box {
            half: Vec3::new(6.5, 0.4, 4.5),
        },
        world: Mat4::from_translation(Vec3::new(20.0, 6.4, -35.0)),
        albedo: [0.5, 0.5, 0.55],
        emissive: 0.0,
        temp_f: ambient_f - 8.0, // radiative cooling, SDD §2.1
        glass: false,
    });

    // Fence posts marching along x.
    for i in 0..14 {
        items.push(DrawItem {
            shape: Shape::Cylinder {
                radius: 0.08,
                height: 1.2,
            },
            world: Mat4::from_translation(Vec3::new(-20.0 + i as f32 * 3.0, 0.0, -20.0)),
            albedo: [0.3, 0.25, 0.2],
            emissive: 0.0,
            temp_f: ambient_f - 1.0,
            glass: false,
        });
    }

    // Trees: trunk + canopy blob.
    for (x, z) in [(-15.0, -50.0), (-5.0, -55.0), (8.0, -52.0)] {
        items.push(DrawItem {
            shape: Shape::Cylinder {
                radius: 0.3,
                height: 4.0,
            },
            world: Mat4::from_translation(Vec3::new(x, 0.0, z)),
            albedo: [0.28, 0.22, 0.16],
            emissive: 0.0,
            temp_f: ambient_f + 1.0, // trees hold heat, SDD §2.1
            glass: false,
        });
        items.push(DrawItem {
            shape: Shape::Sphere { radius: 2.6 },
            world: Mat4::from_translation(Vec3::new(x, 5.5, z)),
            albedo: [0.15, 0.3, 0.12],
            emissive: 0.0,
            temp_f: ambient_f + 1.5,
            glass: false,
        });
    }

    // Warm pests: two rats near the barn, a possum by the trees.
    for (x, z, r) in [(16.0, -28.0, 0.18), (24.5, -30.0, 0.18), (-5.0, -48.0, 0.3)] {
        items.push(DrawItem {
            shape: Shape::Sphere { radius: r },
            world: Mat4::from_translation(Vec3::new(x, r, z)),
            albedo: [0.25, 0.22, 0.2],
            emissive: 0.0,
            temp_f: 101.0,
            glass: false,
        });
    }

    // One ambient-temperature figure standing in the open. Invisible to
    // thermal; pale in NV. You know what it is.
    let zx = -8.0;
    let zz = -30.0;
    items.push(DrawItem {
        shape: Shape::Box {
            half: Vec3::new(0.35, 0.85, 0.25),
        },
        world: Mat4::from_translation(Vec3::new(zx, 0.85, zz)),
        albedo: [0.62, 0.6, 0.55],
        emissive: 0.0,
        temp_f: ambient_f,
        glass: false,
    });
    items.push(DrawItem {
        shape: Shape::Sphere { radius: 0.16 },
        world: Mat4::from_translation(Vec3::new(zx, 1.9, zz)),
        albedo: [0.66, 0.62, 0.55],
        emissive: 0.0,
        temp_f: ambient_f,
        glass: false,
    });

    DrawList {
        items,
        ambient_f,
        sky_temp_f: ambient_f - 45.0,
        moonlight: 0.45,
        heat_decals: vec![],
    }
}
