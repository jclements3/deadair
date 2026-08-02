//! Bounding volumes. The scene graph caches one world-space
//! [`BoundingSphere`] per node, computed bottom-up.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// A sphere used for culling and spatial queries.
///
/// A negative radius denotes the *empty* sphere (bounds nothing). Merging
/// with the empty sphere is the identity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingSphere {
    /// Sphere center.
    pub center: Vec3,
    /// Sphere radius; `< 0` means empty.
    pub radius: f32,
}

impl Default for BoundingSphere {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl BoundingSphere {
    /// The empty sphere (bounds nothing).
    pub const EMPTY: Self = Self {
        center: Vec3::ZERO,
        radius: -1.0,
    };

    /// Creates a sphere from a center and radius.
    pub fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// True if this sphere bounds nothing.
    pub fn is_empty(&self) -> bool {
        self.radius < 0.0
    }

    /// Smallest sphere enclosing both `self` and `other`.
    pub fn merged(&self, other: &BoundingSphere) -> BoundingSphere {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let d = self.center.distance(other.center);
        if d + other.radius <= self.radius {
            return *self;
        }
        if d + self.radius <= other.radius {
            return *other;
        }
        let radius = (d + self.radius + other.radius) * 0.5;
        // Move from self.center toward other.center far enough that the new
        // sphere reaches `radius` past self's far side.
        let center = if d > f32::EPSILON {
            self.center + (other.center - self.center) * ((radius - self.radius) / d)
        } else {
            self.center
        };
        BoundingSphere { center, radius }
    }

    /// True if `p` lies inside (or on) the sphere.
    pub fn contains_point(&self, p: Vec3) -> bool {
        !self.is_empty() && self.center.distance(p) <= self.radius + 1e-4
    }

    /// True if `other` lies entirely inside this sphere (empty spheres are
    /// trivially contained).
    pub fn contains_sphere(&self, other: &BoundingSphere) -> bool {
        if other.is_empty() {
            return true;
        }
        !self.is_empty() && self.center.distance(other.center) + other.radius <= self.radius + 1e-4
    }

    /// The sphere transformed by an affine matrix. The radius is scaled by
    /// the largest axis scale of `m` (conservative for non-uniform scale).
    pub fn transformed(&self, m: &Mat4) -> BoundingSphere {
        if self.is_empty() {
            return *self;
        }
        let sx = m.x_axis.truncate().length();
        let sy = m.y_axis.truncate().length();
        let sz = m.z_axis.truncate().length();
        BoundingSphere {
            center: m.transform_point3(self.center),
            radius: self.radius * sx.max(sy).max(sz),
        }
    }

    /// Smallest reasonable sphere over a point set: centered on the AABB
    /// center with radius to the farthest point. Empty input yields
    /// [`BoundingSphere::EMPTY`].
    pub fn from_points<I>(points: I) -> BoundingSphere
    where
        I: IntoIterator<Item = Vec3> + Clone,
    {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for p in points.clone() {
            any = true;
            min = min.min(p);
            max = max.max(p);
        }
        if !any {
            return BoundingSphere::EMPTY;
        }
        let center = (min + max) * 0.5;
        let mut radius: f32 = 0.0;
        for p in points {
            radius = radius.max(center.distance(p));
        }
        BoundingSphere { center, radius }
    }
}
