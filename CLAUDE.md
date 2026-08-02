# DeadAir — project notes for Claude Code

## Build
- **Rust 1.85 is behind `/snap/bin`** — the default `/usr/bin/rustc` is 1.75 and too old.
  Always: `PATH=/snap/bin:$PATH cargo <cmd>`.
- `cargo test --workspace` must stay green; all simulation crates are headless-testable.
- wgpu under WSL2: headless tests use the llvmpipe Vulkan adapter (works, verified).
  The Intel Iris Xe adapter surfaces via GL/D3D12 and rejects compute-capable limits —
  request `Limits::downlevel_webgl2_defaults()` when falling back to it.

## Ground rules
- SRS.md and SDD.md are the requirements/design ground truth. Cite FR-xx / SDD § in
  commit messages when implementing a requirement.
- Parametric zone sources in `assets/zones/*.zone.ron` are ground truth for world
  content (OpenSCAD spirit): same source + seed → identical scene graph. Never edit
  generated output; edit the source.
- Determinism is load-bearing: all randomness flows from `da_core::Rng` seeds. No
  `std::time`/`rand` in simulation crates.
- Optic pipeline look targets live in `assets/reference/optics-look.md` (distilled
  from real thermal/NV scope footage in `videos/`, which is gitignored).
- `*:Zone.Identifier` files are WSL junk created when the user copies files in from
  Windows — delete on sight (gitignored).

## Crate map
da-core (clock/weather/rng/ids) ← da-graph (OSG-style scene graph) ← da-param
(parametric expansion) ; da-thermal (night thermal sim) ; da-sim (ballistics/AI/
hazards) ; da-econ (contracts/P&L/saves) ; da-render (wgpu, three optic passes) ;
apps/deadair (game) ; apps/da-edit (editor) ; tools/osgedit (legacy CSG renderer).
