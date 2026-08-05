//! da-graph — DarkAir's retained scene graph, in the OpenSceneGraph
//! spirit (SDD §10).
//!
//! - **Nodes** live in an arena ([`Scene`]) addressed by [`NodeId`]:
//!   [`NodeKind::Group`], [`NodeKind::Transform`] (TRS with cached local
//!   matrix), [`NodeKind::Geode`] (leaf of [`Drawable`]s),
//!   [`NodeKind::Switch`] (per-child mask), [`NodeKind::Lod`] (per-child
//!   view-distance ranges).
//! - **State inheritance**: a [`StateSet`] (material + optional
//!   [`ThermalAttach`]) may be attached to any node; effective state is the
//!   override-merge down the path ([`Scene::effective_state`]).
//! - **Traversals**: the [`Visitor`] trait with [`visit_depth_first`],
//!   plus [`UpdateVisitor`] (world matrices, dirty-flag aware) and
//!   [`CullVisitor`] (Switch/Lod/frustum-aware flat [`RenderLeaf`] list).
//! - **Bounds**: cached per-node world [`BoundingSphere`]s, dirtied by
//!   transform edits.
//! - **Serialization**: stable, human-diffable RON via [`Scene::to_ron`] /
//!   [`Scene::from_ron`]; caches are rebuilt on load.

#![warn(missing_docs)]

pub mod bounds;
pub mod drawable;
pub mod error;
pub mod node;
pub mod scene;
pub mod state;
pub mod visitor;

pub use bounds::BoundingSphere;
pub use drawable::{Drawable, Shape};
pub use error::GraphError;
pub use node::{Geode, LodRange, Node, NodeKind, Transform};
pub use scene::Scene;
pub use state::{StateSet, ThermalAttach};
pub use visitor::{plane, visit_depth_first, CullVisitor, RenderLeaf, UpdateVisitor, Visitor};

// Re-export the id types the graph is addressed with.
pub use da_core::{IdGen, NodeId};

/// Convenient glob-import surface: `use da_graph::prelude::*;`.
pub mod prelude {
    pub use crate::bounds::BoundingSphere;
    pub use crate::drawable::{Drawable, Shape};
    pub use crate::error::GraphError;
    pub use crate::node::{Geode, LodRange, Node, NodeKind, Transform};
    pub use crate::scene::Scene;
    pub use crate::state::{StateSet, ThermalAttach};
    pub use crate::visitor::{
        plane, visit_depth_first, CullVisitor, RenderLeaf, UpdateVisitor, Visitor,
    };
    pub use da_core::{NodeId, TempF};
}
