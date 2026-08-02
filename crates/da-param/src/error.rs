//! Errors produced by zone parsing and expansion.

use std::fmt;

use da_graph::GraphError;

/// Everything that can go wrong while loading or expanding a zone source.
#[derive(Debug)]
pub enum ParamError {
    /// A zone file could not be read from disk.
    Io {
        /// Path of the file (or directory) involved.
        path: String,
        /// Underlying I/O error message.
        message: String,
    },
    /// A zone source failed to deserialize from RON.
    Parse {
        /// Path of the source (`"<str>"` for in-memory text).
        path: String,
        /// Underlying RON error message.
        message: String,
    },
    /// A scene-graph operation failed during expansion.
    Graph(GraphError),
    /// A spawn table / friendly record referenced a `Feature("...")` name
    /// that no feature in the zone produced.
    UnresolvedFeature {
        /// Zone name.
        zone: String,
        /// The dangling feature name.
        reference: String,
    },
    /// A hazard's `along: "..."` referenced a feature (e.g. a creek) that
    /// does not exist or carries no path.
    UnresolvedAlong {
        /// Zone name.
        zone: String,
        /// The dangling `along` name.
        reference: String,
    },
    /// A hazard record carried none of the field combinations that define a
    /// volume (`along`, `from`+`to`, or `pos`).
    MalformedHazard {
        /// Zone name.
        zone: String,
        /// Index of the record in the zone's `hazards` list.
        index: usize,
    },
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamError::Io { path, message } => write!(f, "io error on {path}: {message}"),
            ParamError::Parse { path, message } => write!(f, "parse error in {path}: {message}"),
            ParamError::Graph(e) => write!(f, "scene graph error: {e}"),
            ParamError::UnresolvedFeature { zone, reference } => {
                write!(f, "zone {zone:?}: spawn reference Feature({reference:?}) matches no feature")
            }
            ParamError::UnresolvedAlong { zone, reference } => {
                write!(f, "zone {zone:?}: hazard along {reference:?} matches no path feature")
            }
            ParamError::MalformedHazard { zone, index } => {
                write!(f, "zone {zone:?}: hazard #{index} defines no volume (need along, from+to, or pos)")
            }
        }
    }
}

impl std::error::Error for ParamError {}

impl From<GraphError> for ParamError {
    fn from(e: GraphError) -> Self {
        ParamError::Graph(e)
    }
}
