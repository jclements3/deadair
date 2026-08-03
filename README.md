# DeadAir

A first-person night pest-control business simulation — silent air rifles,
physically honest night optics, warm targets, cold zombies, and a real
profit-and-loss game underneath it — plus the Rust tooling that builds it.

The player runs a contract exterminator business on an air-rifle ladder
(multi-pump .22 → regulated PCP .25) with swappable optics (naked eye, night
vision, thermal). **The world is simulated once, in physical terms; each optic
is a lossy view of that single truth.** Objects cool through the night, weather
changes contrast and profitability, pests glow warm, protected animals cost
money and reputation — and zombies are ambient-temperature: invisible to
thermal, killable only by headshot. That invisibility is not a renderer special
case; it falls out of giving them an ambient-coupled thermal profile.

**Reference docs:** [SRS.md](SRS.md) (requirements) · [SDD.md](SDD.md) (design)

## Layout

| Path | Contents |
|---|---|
| `crates/da-core` | Ids, night clock (dusk→dawn `t`), weather/forecast tables, deterministic RNG, units |
| `crates/da-graph` | OSG-style retained scene graph: typed nodes, visitor traversals, state inheritance |
| `crates/da-thermal` | Per-object thermal simulation: night contrast curve, crossover, residual heat |
| `crates/da-param` | Parametric scenes-as-code: RON zone source → deterministic scene graph |
| `crates/da-sim` | Ballistics, hit resolution, noise events, AI, hazards — headless gameplay |
| `crates/da-econ` | Contracts, licenses, store ladders, nightly P&L, bankruptcy, RON saves |
| `crates/da-render` | wgpu renderer: one geometry pass, three optic pipelines |
| `apps/deadair` | The game |
| `apps/da-edit` | Scene & animation editor (egui) |
| `tools/osgedit` | Legacy CSG camera-path renderer (conference-demo thumbnails) |

Dependencies flow one way: `da-core` ← everything; `da-graph` ← `da-param`;
the renderer consumes a flat `DrawList` and never walks the graph, so the
game, the editor, and the tests all feed it the same way.

## Build

Requires Rust 1.86+ (this machine: `PATH=/snap/bin:$PATH cargo …`).

```
cargo test --workspace     # every simulation crate is headless-testable
cargo run -p deadair       # the game
cargo run -p da-edit       # the editor

# Headless verification — renders the real Home Farm zone, no window:
cargo run -p deadair -- --shot out.png --optic thermal --t 0.3
```

## Authoring

Zones are **text, not scenes**: `assets/zones/*.zone.ron` declares features
parametrically (`Barn(pos, width_m, bays, roof: Metal)`), and expansion is
deterministic — same source plus seed yields a byte-identical graph, with
thermal profiles attached automatically (a metal roof gets high sky exposure,
so it reads *below* ambient on clear nights without anyone authoring that).
See [`assets/zones/FORMAT.md`](assets/zones/FORMAT.md). The editor's source
panel edits that text and re-expands; graph edits are session-only previews,
because the text is ground truth.

## Optics fidelity

The three pipelines are calibrated against real scope footage — see
[`assets/reference/optics-look.md`](assets/reference/optics-look.md), which
distills white-hot and black-hot thermal, digital NV with IR eyeshine, and
smart-scope HUD conventions into concrete acceptance criteria. Headless golden
tests assert the behavior that matters: a pest blazes in thermal while an
ambient-temperature zombie is indistinguishable from the ground, the same
zombie is plainly visible in NV, NV amplifies over the naked eye, and the
auto-gain window keeps pre-dawn crossover looking flat instead of
noise-stretched.
