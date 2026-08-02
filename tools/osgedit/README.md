# osgedit

Camera-path editor/renderer for the OSG (CSG) conference-demo models.
It flies a camera from the **golfer** to the **farm** to the **town** to the
**mountains** and writes a 1024x1024 RGB animated WebP with no shimmering —
for making movie-scene thumbnails that show off OSG modeling and animation.

Adapted from the `zoomrender` app (now in `reference/zoomrender_main.rs`);
the SDF/CSG engine is a port of the vali `csg-showcase` `sdf-core` crate, so
the model JSONs in `models/` (copied from the conference demo) load unchanged.

## Workflow

```
cargo build --release
./target/release/osgedit --keyframes          # list the camera path
./target/release/osgedit --preview --stills   # fast PNG at every keyframe
./target/release/osgedit --preview            # fast draft movie (360px)
./target/release/osgedit --still 6.5          # one full-quality frame at t=6.5s
./target/release/osgedit                      # full 1024x1024 -> osgshow.webp
```

Edit `campath.json`, re-run `--preview --stills` to check composition, then
render. `--out FILE` overrides the output name; `--path FILE` selects another
camera-path file; `--models DIR` selects another model library.

## campath.json

```jsonc
{
  "size": 1024,        // square output, pixels
  "fps": 20,
  "supersample": 4,    // 4x4 = 16 stratified samples/pixel
  "fov_deg": 42,       // default vertical FOV
  "quality": 92,       // WebP lossy quality
  "keys": [
    { "t": 0.0, "eye": [-5.5,1.5,4.5], "look": [0.3,1.1,0.2],
      "fov_deg": 38, "label": "golfer" },
    ...
  ]
}
```

* `t` is seconds; keys must be strictly increasing. Total movie length is the
  last key's `t`.
* `eye`/`look` are interpolated with a non-uniform Catmull-Rom spline (C1
  smooth). Duplicate a key at a later `t` to hold the camera still.
* `fov_deg` per key is optional; it eases smoothly between keys.

## The world

Built in `src/scene.rs` from `models/*.json` (figure, tree, building,
buildings, vehicle, lattice, plane, quad, mountain) plus inline CSG props
(club, ball, flag, silo, crop rows, fence). Layout runs along +X:

| region    | x      | contents                                             |
|-----------|--------|------------------------------------------------------|
| golfer    | 0      | figure + club + ball, tee green, pin flag, trees     |
| farm      | ~60    | barn, silo, tractor, crop rows, fence, orchard       |
| town      | ~140   | building blocks, town hall, radio mast, cars, quad, aircraft |
| mountains | 250+   | six layered mountain instances with snow caps        |

The gallery `hills.json` diorama is loaded but unused — it has its own
plinth and tree, so it doesn't scale up into terrain.

## Anti-shimmer

* 16x (4x4) stratified supersampling per pixel, averaged before gamma —
  the same scheme as zoomrender.
* The jitter pattern is **frame-invariant** (the seed has no frame term), so
  there is no per-frame grain reseed to flicker (the conference-demo
  anti-flicker hard gate).
* Raymarch epsilon scales with distance, soft shadows use a fixed penumbra,
  and distance fog fades far detail smoothly instead of letting it crawl.
* Ground/patchwork variation is smooth low-frequency value noise — static in
  time.
