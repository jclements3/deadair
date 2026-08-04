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
`StreetlightRow` `RadioMast`. Tree kinds: `Oak` `Pine` `Sycamore` `Apple`
`Maple`. Roofs: `Metal` `Shingle`. Hazard kinds: `Wire` `Hole` `CreekBank`
`Water` `Limb`. Species: `Rat` `Rabbit` `Possum` `Raccoon` `Beaver`
`Groundhog` `JuvenileFeralHog` `Dog` `Cat` `Cow` `Sheep`.

Zones (SDD §3): Home Farm (tutorial), Grain Co-op, Creek Bottom, Orchard,
Town Edge, Main Street.
