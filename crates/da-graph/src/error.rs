//! Error type for scene-graph operations.

use da_core::NodeId;

/// Errors produced by [`crate::Scene`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// The referenced node does not exist in the scene.
    NoSuchNode(NodeId),
    /// The node exists but is not a `Transform`.
    NotATransform(NodeId),
    /// The node exists but is not a `Geode`.
    NotAGeode(NodeId),
    /// The node exists but is not a `Switch`.
    NotASwitch(NodeId),
    /// The node exists but is not a `Lod`.
    NotALod(NodeId),
    /// A `Geode` is a leaf and cannot have children.
    GeodeIsLeaf(NodeId),
    /// A child index was out of range for the node's child list.
    ChildIndexOutOfRange {
        /// The node whose children were indexed.
        node: NodeId,
        /// The offending index.
        index: usize,
    },
    /// A RON (de)serialization failure, with the underlying message.
    Ron(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::NoSuchNode(id) => write!(f, "no such node: {id:?}"),
            GraphError::NotATransform(id) => write!(f, "node {id:?} is not a Transform"),
            GraphError::NotAGeode(id) => write!(f, "node {id:?} is not a Geode"),
            GraphError::NotASwitch(id) => write!(f, "node {id:?} is not a Switch"),
            GraphError::NotALod(id) => write!(f, "node {id:?} is not a Lod"),
            GraphError::GeodeIsLeaf(id) => {
                write!(f, "node {id:?} is a Geode leaf and cannot have children")
            }
            GraphError::ChildIndexOutOfRange { node, index } => {
                write!(f, "child index {index} out of range for node {node:?}")
            }
            GraphError::Ron(msg) => write!(f, "RON error: {msg}"),
        }
    }
}

impl std::error::Error for GraphError {}
