# Zone source format (`*.zone.ron`)

A zone is a deterministic function of this file: same source + same `seed`
→ byte-identical scene graph (da-param guarantees it). Text is ground truth
(OpenSCAD spirit); the graph is a build artifact.

```ron
ZoneSource(
    name: "Home Farm",
    seed: 1001,                    // all placement jitter derives from this
    size_m: (240.0, 180.0),        // playable extent, meters (x, z)
    ambient_biome: Grass,          // default ground thermal profile
    features: [                    // parametric generators, expanded in order
        Barn(pos: (40, 0, 30), width_m: 12, bays: 3, roof: Metal),
        FeedShed(pos: (52, 0, 26)),
        FenceLine(from: (10, 0, 10), to: (90, 0, 10), post_gap_m: 3.0),
        TreeRow(from: (0, 0, 60), to: (80, 0, 64), count: 9, kind: Oak),
        CropRows(pos: (20, 0, 80), rows: 12, len_m: 40, gap_m: 2.5),
        Creek(path: [(0, 0, 120), (120, 0, 140)], width_m: 4),
        House(pos: (100, 0, 20), floors: 1),
        // every generator attaches thermal profiles automatically:
        // Metal roof → high sky_exposure (reads below ambient on clear nights)
    ],
    spawn_tables: [
        (species: Rat,     nodes: [Feature("Barn"), Feature("FeedShed")], base_count: 8),
        (species: Possum,  nodes: [Feature("TreeRow")],                   base_count: 3),
        (species: Raccoon, patrol: [(30,0,50),(70,0,70),(90,0,40)],       base_count: 2),
    ],
    friendlies: [
        (species: Dog, patrol: [(95,0,25),(60,0,30),(95,0,25)]),
        (species: Cow, pen: (pos: (10, 0, 30), size: (20, 15)), count: 4),
    ],
    hazards: [
        (kind: Wire,      from: (10, 0, 10), to: (90, 0, 10)),   // along fence
        (kind: Hole,      pos: (55, 0, 55), radius_m: 1.0),
        (kind: CreekBank, along: "Creek"),
    ],
    zombie_weight: 0.2,            // spawn weighting, 0..1 (town/cemetery high)
    connections: [ (to: "Grain Co-op", walk_min: 25), ],
    contracts_hint: [ (species: Rat, quota: 10) ],   // seed data for contract gen
)
```

Zones (SDD §3): Home Farm (tutorial), Grain Co-op, Creek Bottom, Orchard,
Town Edge, Main Street.
