//! The declarative zone source model (`*.zone.ron`).
//!
//! These types mirror `assets/zones/FORMAT.md` one-to-one. Text is ground
//! truth: a [`ZoneSource`] plus its `seed` is the *complete* description of
//! a zone; the scene graph produced by [`crate::expand_zone`] is a build
//! artifact and never edited by hand.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

/// A 2D extent or size in meters, `(x, z)`.
pub type P2 = (f32, f32);

/// A 3D point in meters, `(x, y, z)` — y is up, y = 0 is ground level.
pub type P3 = (f32, f32, f32);

/// Top-level zone description, deserialized from a `*.zone.ron` file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneSource {
    /// Human-readable zone name (also used by `connections.to`).
    pub name: String,
    /// Master seed; all placement jitter in expansion derives from it.
    pub seed: u64,
    /// Playable extent in meters, `(x, z)`.
    pub size_m: P2,
    /// Default ground biome — sets the ground plane's thermal profile.
    pub ambient_biome: Biome,
    /// Parametric generators, expanded in listed order.
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Pest spawn tables (what the player is contracted to shoot).
    #[serde(default)]
    pub spawn_tables: Vec<SpawnTable>,
    /// Friendly animals — never-shoot dilemmas (dogs, cats, livestock).
    #[serde(default)]
    pub friendlies: Vec<FriendlyRecord>,
    /// Footing / traversal hazards.
    #[serde(default)]
    pub hazards: Vec<HazardRecord>,
    /// Zombie spawn weighting, `0..=1` (town/cemetery zones run high).
    #[serde(default)]
    pub zombie_weight: f32,
    /// Walking connections to neighboring zones.
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// Seed data for the contract generator.
    #[serde(default)]
    pub contracts_hint: Vec<ContractHint>,
    /// Resolved `.vim` prop source *text*, keyed by each `VimProp`'s `src`
    /// path. Never written in the RON file: [`crate::load_zone_file`] /
    /// [`crate::load_all_zones`] fill it (via
    /// [`crate::resolve_vim_sources`]) so [`crate::expand_zone`] stays a
    /// pure, I/O-free function of the [`ZoneSource`] value.
    #[serde(skip)]
    pub vim_sources: BTreeMap<String, String>,
}

/// Ground biome — selects the ground plane's material and thermal profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Biome {
    /// Mowed/pasture grass: cools fast, frosts first on clear nights.
    Grass,
    /// Packed gravel yard: rock-like, holds daytime heat.
    Gravel,
    /// Wet creek-bottom mud: rock-like mass, low sky view under canopy.
    Mud,
    /// Paved street/lot: rock-like, big daytime solar store.
    Asphalt,
}

/// Roof material for buildings that expose the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum RoofKind {
    /// Thin metal roofing — full sky exposure, reads *below* ambient on
    /// clear nights (SDD §7A).
    Metal,
    /// Asphalt shingle — slower to cool than metal, still sky-facing.
    Shingle,
}

/// Thermal/material preset a `VimProp` mesh carries — each maps onto one of
/// the canned material StateSets (same presets the built-in generators use).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum PropThermal {
    /// General sheet metal (silo barrel, tank, mast) — the default.
    #[default]
    Metal,
    /// Thin sky-facing metal roofing: reads below ambient on clear nights.
    MetalRoof,
    /// Weathered dry lumber.
    Wood,
    /// Poured concrete / stone.
    Concrete,
    /// Masonry/wood building wall.
    BuildingWall,
    /// LWIR-opaque glass (SDD §7).
    Glass,
}

/// Tree species used by tree generators (drives silhouette + canopy height).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum TreeKind {
    /// Broad round canopy, mid height.
    Oak,
    /// Tall, narrow, high canopy.
    Pine,
    /// Big creek-bottom hardwood.
    Sycamore,
    /// Short orchard tree — climbable possum height.
    Apple,
    /// Street/yard tree, mid height.
    Maple,
}

/// One parametric generator invocation. Each variant expands to a named
/// subgraph whose root node carries the variant's name (so spawn tables can
/// reference it with `Feature("Barn")` etc.).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Feature {
    /// Gabled livestock/equipment barn.
    Barn {
        /// Position of the barn center at ground level.
        pos: P3,
        /// Barn width (across the gable), meters.
        width_m: f32,
        /// Number of structural bays along the length (4 m per bay).
        bays: u32,
        /// Roof material.
        roof: RoofKind,
    },
    /// Small open-front feed shed with a trough — a rat magnet.
    FeedShed {
        /// Position of the shed center at ground level.
        pos: P3,
    },
    /// Farm/town house.
    House {
        /// Position of the house center at ground level.
        pos: P3,
        /// Number of floors (2.8 m per floor).
        floors: u32,
    },
    /// Garden/tool shed.
    Shed {
        /// Position of the shed center at ground level.
        pos: P3,
    },
    /// Grain silo: metal barrel plus dome cap.
    Silo {
        /// Position of the silo axis at ground level.
        pos: P3,
        /// Barrel radius, meters.
        radius_m: f32,
        /// Barrel height, meters.
        height_m: f32,
    },
    /// Raised concrete loading dock with spill line underneath.
    LoadingDock {
        /// Position of the dock center at ground level.
        pos: P3,
        /// Dock length, meters.
        len_m: f32,
    },
    /// Wire-and-post fence run: posts every `post_gap_m` plus two rails.
    FenceLine {
        /// Start of the run at ground level.
        from: P3,
        /// End of the run at ground level.
        to: P3,
        /// Spacing between posts, meters.
        post_gap_m: f32,
    },
    /// A line of `count` trees between two points, with placement jitter.
    TreeRow {
        /// Start of the row at ground level.
        from: P3,
        /// End of the row at ground level.
        to: P3,
        /// Number of trees.
        count: u32,
        /// Tree species.
        kind: TreeKind,
    },
    /// Orchard-style regular grid of trees (rows × cols), with jitter.
    TreeGrid {
        /// Grid origin (first tree) at ground level.
        pos: P3,
        /// Number of rows (along z).
        rows: u32,
        /// Number of columns (along x).
        cols: u32,
        /// Spacing between trees, meters.
        gap_m: f32,
        /// Tree species.
        kind: TreeKind,
    },
    /// Parallel planted crop rows.
    CropRows {
        /// Origin of the first row at ground level.
        pos: P3,
        /// Number of rows (spaced along z).
        rows: u32,
        /// Row length (along x), meters.
        len_m: f32,
        /// Spacing between rows, meters.
        gap_m: f32,
    },
    /// A creek: polyline of water quads with banks on both sides.
    Creek {
        /// Centerline vertices at water level.
        path: Vec<P3>,
        /// Water surface width, meters.
        width_m: f32,
    },
    /// Beaver dam: log tangle plus mud mound (License D target habitat).
    BeaverDam {
        /// Position of the dam center at ground level.
        pos: P3,
    },
    /// Jumble of fallen trunks — footing hazard and possum cover.
    Deadfall {
        /// Center of the jumble at ground level.
        pos: P3,
        /// Scatter radius, meters.
        radius_m: f32,
    },
    /// Groundhog burrow field: `count` dirt mounds inside a radius.
    BurrowField {
        /// Center of the field at ground level.
        pos: P3,
        /// Scatter radius, meters.
        radius_m: f32,
        /// Number of burrow mounds.
        count: u32,
    },
    /// Row of metal dumpsters (raccoon/rat food source).
    DumpsterRow {
        /// Position of the first dumpster at ground level.
        pos: P3,
        /// Number of dumpsters.
        count: u32,
    },
    /// Commercial storefront; `glass: true` gives it a thermal-opaque
    /// glass front pane (SDD §7).
    Storefront {
        /// Position of the building center at ground level.
        pos: P3,
        /// True for a full glass front pane.
        glass: bool,
    },
    /// Municipal town hall: columns, steps, cupola.
    TownHall {
        /// Position of the building center at ground level.
        pos: P3,
    },
    /// Cemetery: lawn patch with a regular headstone grid.
    Cemetery {
        /// Corner of the cemetery at ground level.
        pos: P3,
        /// Extent in meters, `(x, z)`.
        size: P2,
    },
    /// Back-alley strip: pavement, wall segments, clutter.
    AlleyRow {
        /// Start of the alley at ground level.
        pos: P3,
        /// Alley length (along x), meters.
        len_m: f32,
    },
    /// Town park: grass patch with scattered trees.
    Park {
        /// Corner of the park at ground level.
        pos: P3,
        /// Extent in meters, `(x, z)`.
        size: P2,
    },
    /// Row of streetlights with emissive heads (NV bloom / lit lanes).
    StreetlightRow {
        /// Start of the row at ground level.
        from: P3,
        /// End of the row at ground level.
        to: P3,
        /// Spacing between poles, meters.
        gap_m: f32,
    },
    /// Radio mast with crossarms and a red beacon.
    RadioMast {
        /// Position of the mast base at ground level.
        pos: P3,
        /// Mast height, meters.
        height_m: f32,
    },
    /// A `.vim`-authored CSG prop (vali DSL, compiled by da-csg) placed as
    /// one triangle-mesh part. See `VALI_LOKI_OSG_DSL_PRIMER.md` for the
    /// modeling language.
    VimProp {
        /// Path of the `.vim` script, relative to the assets directory
        /// (e.g. `"props/fluted_silo.vim"`). The loader inlines the script
        /// text into [`ZoneSource::vim_sources`] under this key.
        src: String,
        /// Position of the prop's local origin at ground level.
        pos: P3,
        /// Rotation about +Y, degrees.
        #[serde(default)]
        yaw_deg: f32,
        /// Uniform scale factor applied to the meshed solid.
        #[serde(default = "one_f32")]
        scale: f32,
        /// Thermal/material preset for the whole prop.
        #[serde(default)]
        thermal: PropThermal,
        /// Subgraph root name; spawn tables can reference it with
        /// `Feature("<name>")`. Defaults to `"VimProp"`.
        #[serde(default)]
        name: Option<String>,
    },
}

fn one_f32() -> f32 {
    1.0
}

impl Feature {
    /// The subgraph root name this feature expands under — exactly the
    /// variant name, matching `Feature("...")` spawn references.
    pub fn root_name(&self) -> &'static str {
        match self {
            Feature::Barn { .. } => "Barn",
            Feature::FeedShed { .. } => "FeedShed",
            Feature::House { .. } => "House",
            Feature::Shed { .. } => "Shed",
            Feature::Silo { .. } => "Silo",
            Feature::LoadingDock { .. } => "LoadingDock",
            Feature::FenceLine { .. } => "FenceLine",
            Feature::TreeRow { .. } => "TreeRow",
            Feature::TreeGrid { .. } => "TreeGrid",
            Feature::CropRows { .. } => "CropRows",
            Feature::Creek { .. } => "Creek",
            Feature::BeaverDam { .. } => "BeaverDam",
            Feature::Deadfall { .. } => "Deadfall",
            Feature::BurrowField { .. } => "BurrowField",
            Feature::DumpsterRow { .. } => "DumpsterRow",
            Feature::Storefront { .. } => "Storefront",
            Feature::TownHall { .. } => "TownHall",
            Feature::Cemetery { .. } => "Cemetery",
            Feature::AlleyRow { .. } => "AlleyRow",
            Feature::Park { .. } => "Park",
            Feature::StreetlightRow { .. } => "StreetlightRow",
            Feature::RadioMast { .. } => "RadioMast",
            Feature::VimProp { .. } => "VimProp",
        }
    }

    /// The subgraph root name this feature actually expands under: the
    /// variant name, except a `VimProp` with an explicit `name:` uses it.
    pub fn instance_name(&self) -> &str {
        match self {
            Feature::VimProp {
                name: Some(name), ..
            } => name.as_str(),
            other => other.root_name(),
        }
    }
}

/// Animal species — pest targets and friendlies share one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Species {
    /// License A pest: barn/feed/dumpster rat.
    Rat,
    /// License A pest: field-margin rabbit — grazes crop rows and tree lines.
    Rabbit,
    /// License A/B pest: tree-line and canopy possum.
    Possum,
    /// License B pest: trash-route raccoon.
    Raccoon,
    /// License D target: dam-building beaver.
    Beaver,
    /// License D target: burrowing groundhog.
    Groundhog,
    /// License D target: park-rooting juvenile feral hog.
    JuvenileFeralHog,
    /// Friendly: farm dog on patrol — the thermal-blob trap.
    Dog,
    /// Friendly: barn cat, wanders exactly where the rats are.
    Cat,
    /// Friendly: penned cow.
    Cow,
    /// Friendly: penned sheep.
    Sheep,
}

impl fmt::Display for Species {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Species::Rat => "Rat",
            Species::Rabbit => "Rabbit",
            Species::Possum => "Possum",
            Species::Raccoon => "Raccoon",
            Species::Beaver => "Beaver",
            Species::Groundhog => "Groundhog",
            Species::JuvenileFeralHog => "JuvenileFeralHog",
            Species::Dog => "Dog",
            Species::Cat => "Cat",
            Species::Cow => "Cow",
            Species::Sheep => "Sheep",
        };
        f.write_str(s)
    }
}

/// A reference from a spawn table (or `wander_near`) to expanded geometry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum SpawnRef {
    /// References every expanded feature whose subgraph root carries this
    /// name (e.g. `Feature("Barn")` hits all barns in the zone).
    Feature(String),
}

/// One pest spawn table row: where a species appears and how many.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnTable {
    /// The pest species.
    pub species: Species,
    /// Feature references the spawns cluster around (habitat anchors).
    #[serde(default)]
    pub nodes: Vec<SpawnRef>,
    /// Alternative to `nodes`: a fixed patrol polyline the species ranges
    /// along (raccoon trash routes).
    #[serde(default)]
    pub patrol: Vec<P3>,
    /// Baseline number of individuals (weather/contract systems scale it).
    pub base_count: u32,
    /// True to place spawn points at canopy height (orchard possums).
    #[serde(default)]
    pub elevated: bool,
}

/// A rectangular livestock pen, expanded to real fence geometry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PenSpec {
    /// Corner of the pen at ground level.
    pub pos: P3,
    /// Pen extent in meters, `(x, z)`.
    pub size: P2,
}

/// One friendly-animal record. Exactly one of `patrol`, `pen`, or
/// `wander_near` should be given; `pen` wins, then `patrol`, then
/// `wander_near`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FriendlyRecord {
    /// The friendly species (never a legal target).
    pub species: Species,
    /// Patrol polyline (dogs).
    #[serde(default)]
    pub patrol: Vec<P3>,
    /// Fenced pen; expansion adds the fence to the scene and places
    /// `count` static positions inside (livestock).
    #[serde(default)]
    pub pen: Option<PenSpec>,
    /// Feature references the animal loiters near (cats at feed sheds).
    #[serde(default)]
    pub wander_near: Vec<SpawnRef>,
    /// Number of individuals.
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

/// Footing / traversal hazard kinds (SRS hazard taxonomy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum HazardKind {
    /// Fence wire at shin height along a segment.
    Wire,
    /// Hole / burrow / rooted-up ground.
    Hole,
    /// Slick creek bank strip alongside a water path.
    CreekBank,
    /// Open water along a path.
    Water,
    /// Fallen limbs in an area.
    Limb,
}

/// One hazard record. The volume is defined by whichever fields are
/// present: `along` (path of the named feature), `from`+`to` (segment), or
/// `pos` (+ `radius_m`, sphere).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HazardRecord {
    /// What kind of hazard this is.
    pub kind: HazardKind,
    /// Segment start (used with `to`).
    #[serde(default)]
    pub from: Option<P3>,
    /// Segment end (used with `from`).
    #[serde(default)]
    pub to: Option<P3>,
    /// Sphere center (used with `radius_m`).
    #[serde(default)]
    pub pos: Option<P3>,
    /// Sphere radius in meters (defaults to 1.0 when only `pos` is given).
    #[serde(default)]
    pub radius_m: Option<f32>,
    /// Name of a path-bearing feature ("Creek") this hazard runs along.
    #[serde(default)]
    pub along: Option<String>,
}

/// A walking connection to a neighboring zone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// Destination zone name.
    pub to: String,
    /// Walk time in minutes.
    pub walk_min: u32,
}

/// Seed data for the contract generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractHint {
    /// Contracted species.
    pub species: Species,
    /// Suggested kill quota.
    pub quota: u32,
}
