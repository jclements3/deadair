//! The retained scene graph: an arena of [`Node`]s addressed by
//! [`NodeId`], with parent/children links, lazy cached world matrices and
//! world bounds, inherited state, and RON serialization.

use std::collections::HashMap;

use da_core::{IdGen, NodeId};
use glam::{Mat4, Quat, Vec3};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::bounds::BoundingSphere;
use crate::drawable::Drawable;
use crate::error::GraphError;
use crate::node::{Geode, LodRange, Node, NodeKind, Transform};
use crate::state::StateSet;

/// A retained scene graph (OSG spirit).
///
/// Nodes live in an arena (`Vec<Node>`) and are addressed by [`NodeId`];
/// hierarchy is expressed with parent/children id links. A `Scene` always
/// has a root `Group` node, created by [`Scene::new`].
///
/// World matrices and world bounding spheres are cached per node and
/// recomputed lazily: mutating a transform dirties its subtree's matrices
/// and bounds plus its ancestors' bounds.
///
/// Serializes to human-diffable RON via [`Scene::to_ron`] /
/// [`Scene::from_ron`]; caches are not serialized.
#[derive(Debug, Deserialize)]
#[serde(from = "SceneData")]
pub struct Scene {
    idgen: IdGen,
    root: NodeId,
    nodes: Vec<Node>,
    #[serde(skip)]
    index: HashMap<NodeId, usize>,
}

/// Serialized form of [`Scene`] (no cache fields, no index — the index is
/// rebuilt on load).
#[derive(Deserialize)]
#[serde(rename = "Scene")]
struct SceneData {
    idgen: IdGen,
    root: NodeId,
    nodes: Vec<Node>,
}

impl From<SceneData> for Scene {
    fn from(data: SceneData) -> Self {
        let index = data
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        Scene {
            idgen: data.idgen,
            root: data.root,
            nodes: data.nodes,
            index,
        }
    }
}

impl Serialize for Scene {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("Scene", 3)?;
        s.serialize_field("idgen", &self.idgen)?;
        s.serialize_field("root", &self.root)?;
        s.serialize_field("nodes", &self.nodes)?;
        s.end()
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// Creates a scene containing a single root `Group` named `"root"`.
    pub fn new() -> Self {
        let mut idgen = IdGen::new();
        let root_id = idgen.node();
        let mut root = Node::new(root_id, None, NodeKind::Group);
        root.name = Some("root".to_owned());
        let mut index = HashMap::new();
        index.insert(root_id, 0);
        Scene {
            idgen,
            root: root_id,
            nodes: vec![root],
            index,
        }
    }

    /// The root node's id.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Number of nodes in the scene (including the root).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True if the scene somehow has no nodes (cannot happen via the public
    /// API; present for completeness).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn new_node(&mut self, parent: NodeId, kind: NodeKind) -> Result<NodeId, GraphError> {
        let parent_slot = *self
            .index
            .get(&parent)
            .ok_or(GraphError::NoSuchNode(parent))?;
        if matches!(self.nodes[parent_slot].kind, NodeKind::Geode(_)) {
            return Err(GraphError::GeodeIsLeaf(parent));
        }
        let id = self.idgen.node();
        let node = Node::new(id, Some(parent), kind);
        let slot = self.nodes.len();
        self.nodes.push(node);
        self.index.insert(id, slot);
        {
            let p = &mut self.nodes[parent_slot];
            p.children.push(id);
            match &mut p.kind {
                NodeKind::Switch { mask } => mask.push(true),
                NodeKind::Lod { ranges } => ranges.push(LodRange::ALL),
                _ => {}
            }
        }
        // New geometry may grow ancestor bounds.
        self.invalidate_bounds_upward(parent);
        Ok(id)
    }

    /// Adds a `Group` child under `parent`.
    pub fn add_group(&mut self, parent: NodeId) -> Result<NodeId, GraphError> {
        self.new_node(parent, NodeKind::Group)
    }

    /// Adds an identity `Transform` child under `parent`.
    pub fn add_transform(&mut self, parent: NodeId) -> Result<NodeId, GraphError> {
        self.new_node(parent, NodeKind::Transform(Transform::IDENTITY))
    }

    /// Adds a `Transform` child under `parent` translated by `translation`.
    pub fn add_transform_at(
        &mut self,
        parent: NodeId,
        translation: Vec3,
    ) -> Result<NodeId, GraphError> {
        self.new_node(
            parent,
            NodeKind::Transform(Transform::new(translation, Quat::IDENTITY, Vec3::ONE)),
        )
    }

    /// Adds an empty `Geode` leaf under `parent`.
    pub fn add_geode(&mut self, parent: NodeId) -> Result<NodeId, GraphError> {
        self.new_node(parent, NodeKind::Geode(Geode::default()))
    }

    /// Adds a `Switch` child under `parent` (children default to on).
    pub fn add_switch(&mut self, parent: NodeId) -> Result<NodeId, GraphError> {
        self.new_node(parent, NodeKind::Switch { mask: Vec::new() })
    }

    /// Adds a `Lod` child under `parent` (children default to all-distance
    /// ranges).
    pub fn add_lod(&mut self, parent: NodeId) -> Result<NodeId, GraphError> {
        self.new_node(parent, NodeKind::Lod { ranges: Vec::new() })
    }

    /// Appends a drawable to a `Geode` and returns its index within the
    /// geode.
    pub fn add_drawable(&mut self, geode: NodeId, drawable: Drawable) -> Result<usize, GraphError> {
        let slot = *self.index.get(&geode).ok_or(GraphError::NoSuchNode(geode))?;
        let idx = match &mut self.nodes[slot].kind {
            NodeKind::Geode(g) => {
                g.drawables.push(drawable);
                g.drawables.len() - 1
            }
            _ => return Err(GraphError::NotAGeode(geode)),
        };
        self.nodes[slot].bound_valid.set(false);
        self.invalidate_bounds_upward(geode);
        Ok(idx)
    }

    // ------------------------------------------------------------------
    // Access
    // ------------------------------------------------------------------

    /// The node with the given id, if it exists.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.index.get(&id).map(|&slot| &self.nodes[slot])
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let slot = *self.index.get(&id)?;
        Some(&mut self.nodes[slot])
    }

    /// All nodes in creation order (root first).
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    /// The `Transform` payload of a node, if it is a `Transform`.
    pub fn transform(&self, id: NodeId) -> Option<&Transform> {
        match self.node(id)?.kind() {
            NodeKind::Transform(t) => Some(t),
            _ => None,
        }
    }

    /// The switch mask of a node, if it is a `Switch`.
    pub fn switch_mask(&self, id: NodeId) -> Option<&[bool]> {
        match self.node(id)?.kind() {
            NodeKind::Switch { mask } => Some(mask),
            _ => None,
        }
    }

    /// The LOD ranges of a node, if it is a `Lod`.
    pub fn lod_ranges(&self, id: NodeId) -> Option<&[LodRange]> {
        match self.node(id)?.kind() {
            NodeKind::Lod { ranges } => Some(ranges),
            _ => None,
        }
    }

    // ------------------------------------------------------------------
    // Naming and paths
    // ------------------------------------------------------------------

    /// Sets (or clears) the node's name.
    pub fn set_name(
        &mut self,
        id: NodeId,
        name: impl Into<Option<String>>,
    ) -> Result<(), GraphError> {
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        node.name = name.into();
        Ok(())
    }

    /// Finds the first node named `name` in depth-first preorder from the
    /// root.
    pub fn find_by_name(&self, name: &str) -> Option<NodeId> {
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            if let Some(n) = self.node(id) {
                if n.name.as_deref() == Some(name) {
                    return Some(id);
                }
                for &c in n.children.iter().rev() {
                    stack.push(c);
                }
            }
        }
        None
    }

    /// The node path from the root down to `id` (inclusive). Returns an
    /// empty vector if `id` is unknown.
    pub fn path(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.node(c) else {
                return Vec::new();
            };
            out.push(c);
            cur = n.parent;
        }
        out.reverse();
        out
    }

    // ------------------------------------------------------------------
    // State inheritance
    // ------------------------------------------------------------------

    /// Attaches (or replaces, or clears) the node's `StateSet`.
    pub fn set_state(
        &mut self,
        id: NodeId,
        state: impl Into<Option<StateSet>>,
    ) -> Result<(), GraphError> {
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        node.state = state.into();
        Ok(())
    }

    /// The effective state at `id`: the override-merge of every `StateSet`
    /// on the root→`id` path (nearer ancestors are overridden by deeper
    /// ones; `id`'s own set wins last). Unknown ids yield the empty set.
    ///
    /// This walks the node's path; traversals that visit many nodes (e.g.
    /// [`crate::CullVisitor`]) instead accumulate the merge incrementally,
    /// one cheap merge per node with an attached set.
    pub fn effective_state(&self, id: NodeId) -> StateSet {
        let mut acc = StateSet::default();
        for nid in self.path(id) {
            if let Some(n) = self.node(nid) {
                if let Some(s) = &n.state {
                    acc = acc.merged_with(s);
                }
            }
        }
        acc
    }

    // ------------------------------------------------------------------
    // Transforms, world matrices, dirty propagation
    // ------------------------------------------------------------------

    /// Sets a `Transform` node's local TRS, dirtying the world matrices and
    /// bounds of its subtree and the bounds of its ancestors.
    pub fn set_transform(
        &mut self,
        id: NodeId,
        translation: Vec3,
        rotation: Quat,
        scale: Vec3,
    ) -> Result<(), GraphError> {
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        match &mut node.kind {
            NodeKind::Transform(t) => t.set_trs(translation, rotation, scale),
            _ => return Err(GraphError::NotATransform(id)),
        }
        self.invalidate_world_subtree(id);
        self.invalidate_bounds_upward(id);
        Ok(())
    }

    /// Sets only the translation of a `Transform` node (rotation and scale
    /// preserved).
    pub fn set_translation(&mut self, id: NodeId, translation: Vec3) -> Result<(), GraphError> {
        let (r, s) = {
            let t = self
                .transform(id)
                .ok_or(GraphError::NotATransform(id))?;
            (t.rotation(), t.scale())
        };
        self.set_transform(id, translation, r, s)
    }

    /// The node's world matrix (product of ancestor local matrices),
    /// computed lazily and cached. Unknown ids yield identity.
    pub fn world_matrix(&self, id: NodeId) -> Mat4 {
        let Some(n) = self.node(id) else {
            return Mat4::IDENTITY;
        };
        if n.world_valid.get() {
            return n.world_cache.get();
        }
        let parent_world = match n.parent {
            Some(p) => self.world_matrix(p),
            None => Mat4::IDENTITY,
        };
        let w = parent_world * n.local_matrix();
        n.world_cache.set(w);
        n.world_valid.set(true);
        w
    }

    /// Marks the world matrix and bound caches of `id`'s entire subtree
    /// invalid.
    fn invalidate_world_subtree(&self, id: NodeId) {
        let mut stack = vec![id];
        while let Some(nid) = stack.pop() {
            if let Some(n) = self.node(nid) {
                n.world_valid.set(false);
                n.bound_valid.set(false);
                stack.extend_from_slice(&n.children);
            }
        }
    }

    /// Marks the bound caches of `id` and all its ancestors invalid.
    fn invalidate_bounds_upward(&self, id: NodeId) {
        let mut cur = Some(id);
        while let Some(nid) = cur {
            let Some(n) = self.node(nid) else { break };
            n.bound_valid.set(false);
            cur = n.parent;
        }
    }

    // ------------------------------------------------------------------
    // Bounds
    // ------------------------------------------------------------------

    /// The node's world-space bounding sphere, computed bottom-up (geodes
    /// from their drawables, groups as the merge of child bounds — `Switch`
    /// and `Lod` conservatively include all children), cached, and dirtied
    /// by transform edits. Unknown ids yield the empty sphere.
    pub fn world_bound(&self, id: NodeId) -> BoundingSphere {
        let Some(n) = self.node(id) else {
            return BoundingSphere::EMPTY;
        };
        if n.bound_valid.get() {
            return n.bound_cache.get();
        }
        let bound = match &n.kind {
            NodeKind::Geode(g) => {
                let world = self.world_matrix(id);
                g.drawables
                    .iter()
                    .map(|d| d.local_bound().transformed(&world))
                    .fold(BoundingSphere::EMPTY, |acc, b| acc.merged(&b))
            }
            _ => n
                .children
                .iter()
                .map(|&c| self.world_bound(c))
                .fold(BoundingSphere::EMPTY, |acc, b| acc.merged(&b)),
        };
        n.bound_cache.set(bound);
        n.bound_valid.set(true);
        bound
    }

    // ------------------------------------------------------------------
    // Switch / Lod controls
    // ------------------------------------------------------------------

    /// Turns one child of a `Switch` on or off (by child index).
    pub fn set_switch(&mut self, id: NodeId, child: usize, on: bool) -> Result<(), GraphError> {
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        match &mut node.kind {
            NodeKind::Switch { mask } => match mask.get_mut(child) {
                Some(m) => {
                    *m = on;
                    Ok(())
                }
                None => Err(GraphError::ChildIndexOutOfRange { node: id, index: child }),
            },
            _ => Err(GraphError::NotASwitch(id)),
        }
    }

    /// Turns every child of a `Switch` on or off.
    pub fn set_switch_all(&mut self, id: NodeId, on: bool) -> Result<(), GraphError> {
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        match &mut node.kind {
            NodeKind::Switch { mask } => {
                mask.iter_mut().for_each(|m| *m = on);
                Ok(())
            }
            _ => Err(GraphError::NotASwitch(id)),
        }
    }

    /// Sets the `[min, max)` view-distance range for one child of a `Lod`.
    pub fn set_lod_range(
        &mut self,
        id: NodeId,
        child: usize,
        min: f32,
        max: f32,
    ) -> Result<(), GraphError> {
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        match &mut node.kind {
            NodeKind::Lod { ranges } => match ranges.get_mut(child) {
                Some(r) => {
                    *r = LodRange::new(min, max);
                    Ok(())
                }
                None => Err(GraphError::ChildIndexOutOfRange { node: id, index: child }),
            },
            _ => Err(GraphError::NotALod(id)),
        }
    }

    // ------------------------------------------------------------------
    // RON serialization
    // ------------------------------------------------------------------

    /// Serializes the scene to pretty-printed, human-diffable RON.
    pub fn to_ron(&self) -> Result<String, GraphError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new())
            .map_err(|e| GraphError::Ron(e.to_string()))
    }

    /// Deserializes a scene from RON produced by [`Scene::to_ron`].
    pub fn from_ron(text: &str) -> Result<Scene, GraphError> {
        ron::de::from_str(text).map_err(|e| GraphError::Ron(e.to_string()))
    }
}
