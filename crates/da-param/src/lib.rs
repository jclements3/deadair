//! da-param — DeadAir's parametric zone layer (SDD §10, OpenSCAD spirit).
//!
//! Zones are authored as declarative RON text (`assets/zones/*.zone.ron`,
//! spec in `assets/zones/FORMAT.md`) and compiled into [`da_graph::Scene`]
//! graphs by [`expand_zone`]. **Text is ground truth; the graph is a build
//! artifact**:
//!
//! - Same source + same seed → byte-identical `Scene::to_ron` output and
//!   identical spawn data, every time.
//! - Changing only the seed moves placement jitter but never changes the
//!   node count or node names — structure comes from the source alone.
//!
//! Every generator attaches materials *and* thermal state automatically
//! (metal roofs read below ambient on clear nights, water holds its heat,
//! storefront glass is LWIR-opaque), so the thermal optic works against
//! expanded zones with no hand-tuning.
//!
//! Typical use:
//!
//! ```no_run
//! let src = da_param::load_zone_file("assets/zones/home_farm.zone.ron")?;
//! let zone = da_param::expand_zone(&src)?;
//! println!("{} nodes, {} spawns", zone.scene.len(), zone.spawn_points.len());
//! # Ok::<(), da_param::ParamError>(())
//! ```

#![warn(missing_docs)]

pub mod error;
pub mod expand;
pub mod loader;
pub mod source;

mod generate;
mod material;

pub use error::ParamError;
pub use expand::{
    expand_zone, FriendlyBehavior, FriendlySetup, HazardVolume, SpawnPoint, Volume, ZoneExpansion,
};
pub use loader::{load_all_zones, load_zone_file, parse_zone_str};
pub use source::{
    Biome, Connection, ContractHint, Feature, FriendlyRecord, HazardKind, HazardRecord, PenSpec,
    RoofKind, Species, SpawnRef, SpawnTable, TreeKind, ZoneSource, P2, P3,
};
