//! Scene-graph nodes: the [`Node`] arena record and its [`NodeKind`]
//! payload (`Group`, `Transform`, `Geode`, `Switch`, `Lod`).

use std::cell::Cell;

use da_core::NodeId;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::bounds::BoundingSphere;
use crate::drawable::Drawable;
use crate::state::StateSet;

/// Local TRS for a `Transform` node, with a cached local matrix.
///
/// Fields are private so mutation goes through
/// [`crate::Scene::set_transform`], which invalidates the cached local
/// matrix and dirties the subtree's world matrices and bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
    /// Cached local matrix; rebuilt lazily after TRS changes or deserialize.
    #[serde(skip)]
    cache: Cell<Option<Mat4>>,
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    /// The identity transform.
    pub fn identity() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            cache: Cell::new(None),
        }
    }

    /// Creates a transform from translation, rotation, and scale.
    pub fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
            cache: Cell::new(None),
        }
    }

    /// Local translation.
    pub fn translation(&self) -> Vec3 {
        self.translation
    }

    /// Local rotation.
    pub fn rotation(&self) -> Quat {
        self.rotation
    }

    /// Local scale.
    pub fn scale(&self) -> Vec3 {
        self.scale
    }

    /// The local matrix (scale, then rotation, then translation), cached.
    pub fn matrix(&self) -> Mat4 {
        if let Some(m) = self.cache.get() {
            return m;
        }
        let m = Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation);
        self.cache.set(Some(m));
        m
    }

    pub(crate) fn set_trs(&mut self, translation: Vec3, rotation: Quat, scale: Vec3) {
        self.translation = translation;
        self.rotation = rotation;
        self.scale = scale;
        self.cache.set(None);
    }
}

impl PartialEq for Transform {
    fn eq(&self, other: &Self) -> bool {
        self.translation == other.translation
            && self.rotation == other.rotation
            && self.scale == other.scale
    }
}

/// View-distance range for one `Lod` child: the child is drawn when the
/// eye distance `d` satisfies `min <= d < max`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LodRange {
    /// Inclusive minimum eye distance.
    pub min: f32,
    /// Exclusive maximum eye distance.
    pub max: f32,
}

impl LodRange {
    /// Range that is active at every distance.
    pub const ALL: Self = Self {
        min: 0.0,
        max: f32::INFINITY,
    };

    /// Creates a `[min, max)` range.
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// True if `distance` falls inside `[min, max)`.
    pub fn contains(&self, distance: f32) -> bool {
        distance >= self.min && distance < self.max
    }
}

impl Default for LodRange {
    fn default() -> Self {
        Self::ALL
    }
}

/// Leaf payload: a list of drawables.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Geode {
    /// The drawables rendered at this leaf.
    pub drawables: Vec<Drawable>,
}

/// The typed payload of a node — the OSG node-kind vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Plain group with ordered children.
    Group,
    /// Group that applies a local TRS to its subtree.
    Transform(Transform),
    /// Leaf holding drawables; cannot have children.
    Geode(Geode),
    /// Group with a per-child on/off mask (parallel to the child list).
    Switch {
        /// `mask[i]` gates `children[i]`; missing entries default to on.
        mask: Vec<bool>,
    },
    /// Group whose children are alternate representations selected by eye
    /// distance (ranges parallel to the child list).
    Lod {
        /// `ranges[i]` selects `children[i]`; missing entries default to
        /// [`LodRange::ALL`].
        ranges: Vec<LodRange>,
    },
}

/// One arena slot in a [`crate::Scene`]: identity, hierarchy links,
/// optional name and [`StateSet`], the [`NodeKind`] payload, and cached
/// world matrix / world bound (never serialized; rebuilt on demand).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub(crate) id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<StateSet>,
    pub(crate) kind: NodeKind,

    // --- caches (world space), maintained via interior mutability so
    // read-only traversals can fill them lazily ---
    #[serde(skip)]
    pub(crate) world_cache: Cell<Mat4>,
    #[serde(skip)]
    pub(crate) world_valid: Cell<bool>,
    #[serde(skip)]
    pub(crate) bound_cache: Cell<BoundingSphere>,
    #[serde(skip)]
    pub(crate) bound_valid: Cell<bool>,
}

impl Node {
    pub(crate) fn new(id: NodeId, parent: Option<NodeId>, kind: NodeKind) -> Self {
        Self {
            id,
            name: None,
            parent,
            children: Vec::new(),
            state: None,
            kind,
            world_cache: Cell::new(Mat4::IDENTITY),
            world_valid: Cell::new(false),
            bound_cache: Cell::new(BoundingSphere::EMPTY),
            bound_valid: Cell::new(false),
        }
    }

    /// This node's id.
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Optional human-readable name.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Parent id (`None` for the root).
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Ordered child ids.
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// The `StateSet` attached to this node, if any.
    pub fn state(&self) -> Option<&StateSet> {
        self.state.as_ref()
    }

    /// The typed payload.
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// The node's local matrix: the TRS matrix for `Transform` nodes,
    /// identity for every other kind.
    pub fn local_matrix(&self) -> Mat4 {
        match &self.kind {
            NodeKind::Transform(t) => t.matrix(),
            _ => Mat4::IDENTITY,
        }
    }
}
