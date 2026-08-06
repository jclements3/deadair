//! Visitor traversals in the OSG `NodeVisitor` spirit: a generic
//! depth-first walk that user visitors plug into, plus the provided
//! [`UpdateVisitor`] (world-matrix refresh) and [`CullVisitor`] (draw-list
//! production honoring `Switch` masks, `Lod` ranges, and an optional
//! frustum).

use da_core::NodeId;
use glam::{Mat4, Vec3, Vec4};

use crate::node::{LodRange, Node, NodeKind};
use crate::scene::Scene;
use crate::state::StateSet;

/// A read-only scene traversal callback with enter/leave hooks per node
/// kind.
///
/// The per-kind hooks (`enter_group`, `enter_transform`, ...) default to
/// the generic [`Visitor::enter`] / [`Visitor::leave`], so a visitor may
/// override only the generic pair, only specific kinds, or both. An
/// `enter_*` hook returning `false` prunes the traversal: children are
/// skipped (the matching `leave_*` still runs).
#[allow(unused_variables)]
pub trait Visitor {
    /// Generic enter hook; return `false` to skip the node's children.
    fn enter(&mut self, scene: &Scene, node: &Node) -> bool {
        true
    }

    /// Generic leave hook.
    fn leave(&mut self, scene: &Scene, node: &Node) {}

    /// Enter a `Group` node.
    fn enter_group(&mut self, scene: &Scene, node: &Node) -> bool {
        self.enter(scene, node)
    }
    /// Leave a `Group` node.
    fn leave_group(&mut self, scene: &Scene, node: &Node) {
        self.leave(scene, node)
    }

    /// Enter a `Transform` node.
    fn enter_transform(&mut self, scene: &Scene, node: &Node) -> bool {
        self.enter(scene, node)
    }
    /// Leave a `Transform` node.
    fn leave_transform(&mut self, scene: &Scene, node: &Node) {
        self.leave(scene, node)
    }

    /// Enter a `Geode` leaf.
    fn enter_geode(&mut self, scene: &Scene, node: &Node) -> bool {
        self.enter(scene, node)
    }
    /// Leave a `Geode` leaf.
    fn leave_geode(&mut self, scene: &Scene, node: &Node) {
        self.leave(scene, node)
    }

    /// Enter a `Switch` node.
    fn enter_switch(&mut self, scene: &Scene, node: &Node) -> bool {
        self.enter(scene, node)
    }
    /// Leave a `Switch` node.
    fn leave_switch(&mut self, scene: &Scene, node: &Node) {
        self.leave(scene, node)
    }

    /// Enter a `Lod` node.
    fn enter_lod(&mut self, scene: &Scene, node: &Node) -> bool {
        self.enter(scene, node)
    }
    /// Leave a `Lod` node.
    fn leave_lod(&mut self, scene: &Scene, node: &Node) {
        self.leave(scene, node)
    }
}

/// Depth-first traversal from `root`: preorder `enter_*`, children in
/// order, postorder `leave_*`. Visits *all* children regardless of
/// `Switch`/`Lod` settings (structural traversal); visitors that care can
/// inspect [`Node::kind`] or return `false` from `enter_*` to prune.
pub fn visit_depth_first<V: Visitor + ?Sized>(scene: &Scene, root: NodeId, visitor: &mut V) {
    let Some(node) = scene.node(root) else {
        return;
    };
    let descend = match node.kind() {
        NodeKind::Group => visitor.enter_group(scene, node),
        NodeKind::Transform(_) => visitor.enter_transform(scene, node),
        NodeKind::Geode(_) => visitor.enter_geode(scene, node),
        NodeKind::Switch { .. } => visitor.enter_switch(scene, node),
        NodeKind::Lod { .. } => visitor.enter_lod(scene, node),
    };
    if descend {
        for &child in node.children() {
            visit_depth_first(scene, child, visitor);
        }
    }
    match node.kind() {
        NodeKind::Group => visitor.leave_group(scene, node),
        NodeKind::Transform(_) => visitor.leave_transform(scene, node),
        NodeKind::Geode(_) => visitor.leave_geode(scene, node),
        NodeKind::Switch { .. } => visitor.leave_switch(scene, node),
        NodeKind::Lod { .. } => visitor.leave_lod(scene, node),
    }
}

/// Top-down world-matrix refresh with dirty-flag awareness: valid cache
/// entries are O(1) reads; only dirtied subtrees are recomputed.
#[derive(Debug, Default)]
pub struct UpdateVisitor {
    /// Nodes visited by the last run.
    pub visited: usize,
}

impl Visitor for UpdateVisitor {
    fn enter(&mut self, scene: &Scene, node: &Node) -> bool {
        // Forces (and caches) the world matrix; parents are visited first,
        // so this multiplies against an already-fresh parent matrix.
        let _ = scene.world_matrix(node.id());
        self.visited += 1;
        true
    }
}

impl UpdateVisitor {
    /// Refreshes every world matrix and the root bound of `scene`.
    pub fn run(scene: &Scene) -> Self {
        let mut v = UpdateVisitor::default();
        visit_depth_first(scene, scene.root(), &mut v);
        let _ = scene.world_bound(scene.root());
        v
    }
}

/// One entry of the flat draw list produced by [`CullVisitor`].
#[derive(Debug, Clone, PartialEq)]
pub struct RenderLeaf {
    /// The `Geode` node the drawable belongs to.
    pub node: NodeId,
    /// Index of the drawable within the geode.
    pub drawable: usize,
    /// World matrix of the geode.
    pub world: Mat4,
    /// Fully-merged effective state at the geode.
    pub state: StateSet,
}

/// Culling traversal: walks the graph from the root honoring `Switch`
/// masks and `Lod` view-distance ranges, optionally rejects subtrees whose
/// world bound is outside a frustum, and emits a flat [`RenderLeaf`] list.
///
/// Effective state is accumulated incrementally during the walk (one
/// override-merge per node carrying a `StateSet`), so per-leaf state does
/// not re-walk the node path.
#[derive(Debug, Clone)]
pub struct CullVisitor {
    /// Eye position in world space (drives `Lod` selection).
    pub eye: Vec3,
    /// Optional frustum planes `(a, b, c, d)`; a point `p` is inside a
    /// plane when `dot(plane.xyz, p) + plane.w >= 0`. A sphere entirely
    /// outside any plane is culled with its subtree.
    pub frustum: Option<Vec<Vec4>>,
}

impl CullVisitor {
    /// A cull traversal from `eye` with no frustum (distance/LOD and
    /// switch logic only).
    pub fn new(eye: Vec3) -> Self {
        Self { eye, frustum: None }
    }

    /// Builder: attach frustum planes (see [`CullVisitor::frustum`] for the
    /// plane convention; [`plane`] builds one from a point and an inward
    /// normal).
    #[must_use]
    pub fn with_frustum(mut self, planes: Vec<Vec4>) -> Self {
        self.frustum = Some(planes);
        self
    }

    /// Produces the draw list for the whole scene.
    pub fn cull(&self, scene: &Scene) -> Vec<RenderLeaf> {
        self.cull_from(scene, scene.root())
    }

    /// Produces the draw list for the subtree rooted at `root`. State
    /// inherited from `root`'s ancestors is honored.
    pub fn cull_from(&self, scene: &Scene, root: NodeId) -> Vec<RenderLeaf> {
        let inherited = match scene.node(root).and_then(Node::parent) {
            Some(p) => scene.effective_state(p),
            None => StateSet::default(),
        };
        let mut out = Vec::new();
        self.walk(scene, root, &inherited, &mut out);
        out
    }

    fn sphere_outside_frustum(&self, scene: &Scene, id: NodeId) -> bool {
        let Some(planes) = &self.frustum else {
            return false;
        };
        let b = scene.world_bound(id);
        if b.is_empty() {
            return false;
        }
        planes
            .iter()
            .any(|p| p.truncate().dot(b.center) + p.w < -b.radius)
    }

    fn walk(&self, scene: &Scene, id: NodeId, inherited: &StateSet, out: &mut Vec<RenderLeaf>) {
        let Some(node) = scene.node(id) else {
            return;
        };
        // Accumulate state without cloning when the node adds nothing.
        let merged_storage;
        let state: &StateSet = match node.state() {
            Some(s) => {
                merged_storage = inherited.merged_with(s);
                &merged_storage
            }
            None => inherited,
        };
        if self.sphere_outside_frustum(scene, id) {
            return;
        }
        match node.kind() {
            NodeKind::Geode(g) => {
                let world = scene.world_matrix(id);
                for drawable in 0..g.drawables.len() {
                    out.push(RenderLeaf {
                        node: id,
                        drawable,
                        world,
                        state: state.clone(),
                    });
                }
            }
            NodeKind::Switch { mask } => {
                for (i, &child) in node.children().iter().enumerate() {
                    if mask.get(i).copied().unwrap_or(true) {
                        self.walk(scene, child, state, out);
                    }
                }
            }
            NodeKind::Lod { ranges } => {
                let bound = scene.world_bound(id);
                let center = if bound.is_empty() {
                    scene.world_matrix(id).transform_point3(Vec3::ZERO)
                } else {
                    bound.center
                };
                let distance = self.eye.distance(center);
                for (i, &child) in node.children().iter().enumerate() {
                    let range = ranges.get(i).copied().unwrap_or(LodRange::ALL);
                    if range.contains(distance) {
                        self.walk(scene, child, state, out);
                    }
                }
            }
            NodeKind::Group | NodeKind::Transform(_) => {
                for &child in node.children() {
                    self.walk(scene, child, state, out);
                }
            }
        }
    }
}

/// Builds a frustum plane from a point on the plane and its inward-facing
/// normal, in the `(a, b, c, d)` form [`CullVisitor`] expects.
pub fn plane(point: Vec3, inward_normal: Vec3) -> Vec4 {
    let n = inward_normal.normalize_or_zero();
    n.extend(-n.dot(point))
}
