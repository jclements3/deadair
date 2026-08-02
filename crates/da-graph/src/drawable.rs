//! Drawables: the renderable payload of a `Geode` leaf — a primitive shape
//! plus a material slot.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::bounds::BoundingSphere;

/// A primitive shape in the drawable's local frame (centered at the origin).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Axis-aligned box with the given half extents.
    Box {
        /// Half the box size along each local axis.
        half_extents: Vec3,
    },
    /// Cylinder aligned to the local Y axis.
    Cylinder {
        /// Cylinder radius.
        radius: f32,
        /// Full height along Y.
        height: f32,
    },
    /// Sphere.
    Sphere {
        /// Sphere radius.
        radius: f32,
    },
    /// Capsule aligned to the local Y axis.
    Capsule {
        /// Capsule radius (also the cap radius).
        radius: f32,
        /// Height of the *cylindrical segment* (total height is
        /// `height + 2 * radius`).
        height: f32,
    },
    /// Arbitrary triangle mesh (placeholder until real assets exist).
    Mesh {
        /// Vertex positions in local space.
        vertices: Vec<Vec3>,
        /// Triangle list indices into `vertices`.
        indices: Vec<u32>,
    },
}

impl Shape {
    /// Bounding sphere of the shape in its local frame.
    pub fn local_bound(&self) -> BoundingSphere {
        match self {
            Shape::Box { half_extents } => BoundingSphere::new(Vec3::ZERO, half_extents.length()),
            Shape::Cylinder { radius, height } => {
                let half_h = height * 0.5;
                BoundingSphere::new(Vec3::ZERO, (radius * radius + half_h * half_h).sqrt())
            }
            Shape::Sphere { radius } => BoundingSphere::new(Vec3::ZERO, *radius),
            Shape::Capsule { radius, height } => {
                BoundingSphere::new(Vec3::ZERO, height * 0.5 + radius)
            }
            Shape::Mesh { vertices, .. } => BoundingSphere::from_points(vertices.iter().copied()),
        }
    }
}

/// One renderable item inside a `Geode`: a shape and the material slot it
/// binds. Material *parameters* come from the inherited [`crate::StateSet`];
/// the slot selects among multiple materials on multi-part objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drawable {
    /// The primitive shape.
    pub shape: Shape,
    /// Material slot index (0 = default slot).
    #[serde(default)]
    pub material: usize,
}

impl Drawable {
    /// A drawable in the default material slot.
    pub fn new(shape: Shape) -> Self {
        Self { shape, material: 0 }
    }

    /// Builder: assign a material slot.
    #[must_use]
    pub fn with_material(mut self, slot: usize) -> Self {
        self.material = slot;
        self
    }

    /// Local-space bounding sphere of the shape.
    pub fn local_bound(&self) -> BoundingSphere {
        self.shape.local_bound()
    }
}
