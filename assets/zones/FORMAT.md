# Zone source format (`*.zone.ron`)

A zone is a deterministic function of this file: same source + same `seed`
→ byte-identical scene graph (da-param guarantees it). Text is ground truth
(OpenSCAD spirit); the graph is a build artifact.

Notes for authors:
- Float fields must be written as floats (`12.0`, not `12`); integer fields
  (`bays`, `count`, `floors`, `base_count`, `walk_min`, `quota`) as integers.
- Points are `(x, y, z)` meters, y up, ground at `y: 0.0`.
- Optional record fields (`pen:`, `along:`, `elevated:`, ...) are written
  bare — the loader enables RON `implicit_some` — and may be omitted.

```ron
ZoneSource(
    name: "Home Farm",
    seed: 1001,                    // all placement jitter derives from this
    size_m: (240.0, 180.0),        // playable extent, meters (x, z)
    ambient_biome: Grass,          // ground thermal profile: Grass | Gravel | Mud | Asphalt
    features: [                    // parametric generators, expanded in order
        Barn(pos: (40.0, 0.0, 30.0), width_m: 12.0, bays: 3, roof: Metal),
        FeedShed(pos: (52.0, 0.0, 26.0)),
        FenceLine(from: (10.0, 0.0, 10.0), to: (90.0, 0.0, 10.0), post_gap_m: 3.0),
        TreeRow(from: (0.0, 0.0, 60.0), to: (80.0, 0.0, 64.0), count: 9, kind: Oak),
        CropRows(pos: (20.0, 0.0, 80.0), rows: 12, len_m: 40.0, gap_m: 2.5),
        Creek(path: [(0.0, 0.0, 120.0), (120.0, 0.0, 140.0)], width_m: 4.0),
        House(pos: (100.0, 0.0, 20.0), floors: 1),
        // every generator attaches thermal profiles automatically:
        // Metal roof → high sky_exposure (reads below ambient on clear nights)
    ],
    spawn_tables: [
        (species: Rat,     nodes: [Feature("Barn"), Feature("FeedShed")], base_count: 8),
        // elevated: true puts the spawn points at canopy height (orchard possums)
        (species: Possum,  nodes: [Feature("TreeRow")], base_count: 3, elevated: true),
        (species: Raccoon, patrol: [(30.0,0.0,50.0),(70.0,0.0,70.0),(90.0,0.0,40.0)], base_count: 2),
    ],
    friendlies: [
        // exactly one of patrol / pen / wander_near per record
        (species: Dog, patrol: [(95.0,0.0,25.0),(60.0,0.0,30.0),(95.0,0.0,25.0)]),
        (species: Cow, pen: (pos: (10.0, 0.0, 30.0), size: (20.0, 15.0)), count: 4),
        (species: Cat, wander_near: [Feature("FeedShed")], count: 2),
    ],
    hazards: [
        // volume = along: "<path feature name>" | from+to segment | pos+radius_m sphere
        (kind: Wire,      from: (10.0, 0.0, 10.0), to: (90.0, 0.0, 10.0)),   // along fence
        (kind: Hole,      pos: (55.0, 0.0, 55.0), radius_m: 1.0),
        (kind: CreekBank, along: "Creek"),
    ],
    zombie_weight: 0.2,            // spawn weighting, 0..1 (town/cemetery high)
    connections: [ (to: "Grain Co-op", walk_min: 25), ],
    contracts_hint: [ (species: Rat, quota: 10) ],   // seed data for contract gen
)
```

Feature generators (spawn tables reference them by variant name):
`Barn` `FeedShed` `House` `Shed` `Silo` `LoadingDock` `FenceLine` `TreeRow`
`TreeGrid` `CropRows` `Creek` `BeaverDam` `Deadfall` `BurrowField`
`DumpsterRow` `Storefront` `TownHall` `Cemetery` `AlleyRow` `Park`
`StreetlightRow` `RadioMast` `VimProp`. Tree kinds: `Oak` `Pine` `Sycamore`
`Apple` `Maple`. Roofs: `Metal` `Shingle`. Hazard kinds: `Wire` `Hole`
`CreekBank` `Water` `Limb`. Species: `Rat` `Rabbit` `Possum` `Raccoon`
`Beaver` `Groundhog` `JuvenileFeralHog` `Dog` `Cat` `Cow` `Sheep`.

## `VimProp` — `.vim`-authored CSG props

`VimProp` places a solid modeled in the vali `.vim` CSG language (compiled
by the `da-csg` crate) as a single triangle-mesh part. The language — a
small Nim-flavored script of primitives, 2D sketches (bezier lathes, fluted
`rose` sections, twisted extrudes, ...), and exact BSP booleans — is
documented in `VALI_LOKI_OSG_DSL_PRIMER.md` at the repo root. Prop scripts
live in `assets/props/*.vim` and are ground truth exactly like zone text:
same script + same zone source → byte-identical scene graph.

```ron
VimProp(
    src: "props/fluted_silo.vim",  // relative to the assets dir (zones/..)
    pos: (96.0, 0.0, 40.0),        // local origin at ground level
    yaw_deg: 15.0,                 // optional, default 0.0 — rotation about +Y
    scale: 1.0,                    // optional, default 1.0 — uniform scale
    thermal: Metal,                // optional, default Metal (see below)
    name: "FlutedSilo",            // optional subgraph root name, default
                                   // "VimProp"; Feature("FlutedSilo") works
)
```

`thermal` selects the material/thermal preset for the whole prop: `Metal`
(sheet metal, the default) `MetalRoof` (thin sky-facing metal — reads below
ambient on clear nights) `Wood` `Concrete` `BuildingWall` `Glass`
(LWIR-opaque).

Authoring notes: `.vim` is **Z-up**, meters; model the prop with its base
at `z = 0` — da-csg's Y-up conversion then stands it on the ground. Keep
tessellation sane (`seg <= 64`). A script that fails to compile fails zone
expansion with the DSL's error message and the script path — errors are
never swallowed. The `.vim` text is inlined into the `ZoneSource` by the
file loader (`load_zone_file` / `load_all_zones`), so `expand_zone` itself
stays pure; if you build a source with `parse_zone_str`, call
`da_param::resolve_vim_sources(&mut src, assets_dir)` before expanding.

## Builtin `.vim` templates (`assets/props/builtin/`)

The shaped built-in generators no longer hard-code Rust primitives: their
geometry lives in `.vim` templates under `assets/props/builtin/` —
`silo.vim`, `streetlight.vim`, `radio_mast.vim`, `dumpster.vim`, and the
`gravestone_{a,b,c}.vim` cemetery variants. Same authoring rules as
`VimProp` scripts (Z-up, meters, base at `z = 0`, `seg <= 48`), same
determinism contract. Placement/layout (row spacing, counts, seeded
jitter) stays in the Rust generators; only object geometry is text.

- **Loading**: templates are baked into da-param at compile time via
  `include_str!` (`crates/da-param/src/vim.rs`), NOT routed through the
  `VimProp` resolver — `expand_zone` stays a pure, I/O-free function of
  the source value even for programmatically-built sources. Editing a
  template rebuilds da-param; the text stays ground truth.
- **Parameters**: zone RON fields bind onto a template's *constant*
  numeric `let` lines by name — `Silo(radius_m:, height_m:)` rewrites
  `let radius = ...` / `let height = ...` via
  `da_param::vim_with_params`; `RadioMast(height_m:)` binds `let height`.
  Derived lines (`let hz = height / 2`) re-fold from the new values. A
  generator binding a name the template lacks is a hard error.
- **Parts → materials**: `let` bindings name the solid's parts (part tags
  flow from them), and each part expands to its own `Shape::Mesh` node
  with its own material/thermal state. The mapping (e.g. `dome` →
  `SiloDome`/thin sky-facing metal, `head` → `StreetlightHead`/emissive
  lamp glass) is documented in each template's header comment and
  enforced in `crates/da-param/src/generate.rs`.

Each distinct template is compiled once per zone expansion (cache keyed
by final source text) and the renderer dedupes identical meshes by
content hash, so a 27-post streetlight row costs one BSP compile.

Zones (SDD §3): Home Farm (tutorial), Grain Co-op, Creek Bottom, Orchard,
Town Edge, Main Street.
