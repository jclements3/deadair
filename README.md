# DeadAir

A first-person night pest-control business simulation — silent air rifles,
physically honest night optics, warm targets, cold zombies, and a real
profit-and-loss game underneath it — plus the Rust tooling that builds it.

The player runs a contract exterminator business on an air-rifle ladder
(multi-pump .22 → regulated PCP .25) with swappable optics (naked eye, night
vision, thermal). The world is simulated once, in physical terms; each optic
is a lossy view of that single truth. Objects cool through the night, weather
changes contrast and profitability, pests glow warm, protected animals cost
money and reputation — and zombies are ambient-temperature: invisible to
thermal, killable only by headshot.

**Reference docs:** [SRS.md](SRS.md) (requirements) · [SDD.md](SDD.md) (design)

## Layout

| Path | Contents |
|---|---|
| `crates/da-core` | Ids, night clock, weather/forecast tables, deterministic RNG, units |
| `crates/da-graph` | OSG-style retained scene graph: typed nodes, visitor traversals, state inheritance |
| `crates/da-thermal` | Per-object thermal simulation: night contrast curve, residual heat, weather coupling |
| `crates/da-param` | Parametric scenes-as-code (OpenSCAD spirit): RON source → deterministic subgraphs |
| `crates/da-sim` | Ballistics, hit resolution, noise events, AI (pests / friendlies / zombies), hazards |
| `crates/da-econ` | Contracts, licenses, store ladders, nightly P&L, bankruptcy, RON saves |
| `apps/deadair` | The game (wgpu + winit) |
| `apps/da-edit` | Scene & animation editor (egui): Blender-spirit viewport + OpenSCAD-spirit source |
| `tools/osgedit` | Legacy CSG camera-path renderer (conference-demo thumbnails) |

## Build

Requires Rust 1.85+.

```
cargo test --workspace     # all simulation crates are headless-testable
cargo run -p deadair       # the game
cargo run -p da-edit       # the editor
```
