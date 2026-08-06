# DarkAir — project notes for Claude Code

## Build
- **Rust stable (1.97+) is behind `/snap/bin`** (snap rustup) — the default `/usr/bin/rustc`
  is 1.75 and too old. Always: `PATH=/snap/bin:$PATH cargo <cmd>`.
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
(parametric expansion; shaped builtin generators — Silo, StreetlightRow,
RadioMast, DumpsterRow, Cemetery — take their geometry from editable
`assets/props/builtin/*.vim` templates, see FORMAT.md) ; da-csg (vendored vali
BSP CSG kernel + `.vim` modeling DSL — compiles `assets/props/**/*.vim` into
Y-up per-part meshes for da-param; also ISO 128 drawing/SVG + STL/OBJ export
via the `vimtool` CLI; language primer in VALI_LOKI_OSG_DSL_PRIMER.md) ;
da-thermal (night
thermal sim) ; da-sim (ballistics/AI/hazards) ; da-econ (contracts/P&L/saves) ;
da-render (wgpu, three optic passes) ; apps/darkair (game) ; apps/da-edit
(editor) ; tools/osgedit (legacy CSG renderer).
