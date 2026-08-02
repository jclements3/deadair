# deadair

A **silent night-hunting business simulator** written in Rust.

## Features

| Pillar | Detail |
|--------|--------|
| **Thermal-honest optics** | Detection is modelled using real camera physics: NETD (Noise Equivalent Temperature Difference), field-of-view, Beer–Lambert atmospheric attenuation.  A budget 80 mK unit detects cold zombies only ~38 % of the time; a military-grade 25 mK core pushes that to 99 %. |
| **Cold zombies** | Dead bodies equilibrate to ambient temperature.  Zombie ΔT ≈ 0.1 °C vs a living human's ΔT ≈ 28 °C.  Thermal imaging is genuinely fallible against them. |
| **Real P&L** | Equipment is depreciated across hunts (straight-line).  Variable costs (ammo, permits) and bounty revenue are tracked per-hunt and presented as a formatted Profit & Loss statement. |
| **Scene editor** | An OpenSCAD / Blender-spirit scene editor backed by JSON.  Objects are parametric nodes (terrain, box, cylinder, zombie, hunter-spawn, light) arranged in a hierarchical scene graph.  Add, remove, move and persist scenes interactively. |

## Quick start

```
cargo run                        # run the built-in demo
cargo run -- hunt                # simulate a hunt on the default scene
cargo run -- hunt my_scene.json  # hunt on a custom scene file
cargo run -- editor              # interactive scene editor
```

## Architecture

```
src/
  vec.rs      Vec2 / Vec3 value types
  entity.rs   Entity definition and temperature model
  thermal.rs  ThermalOptics — NETD, FOV, SNR, detection probability
  economy.rs  Equipment catalogue, LineItem, ProfitLossLedger
  scene.rs    OpenSCAD-spirit scene graph (SceneNode, NodeKind, Scene)
  world.rs    Runtime World derived from a Scene
  hunt.rs     HuntSimulation — step-through turn loop
  editor.rs   SceneEditor REPL — add / remove / move / map / save / load
  main.rs     CLI (demo | hunt | editor)
```

## Example P&L output

```
╔══════════════════════════════════════════════════╗
║               HUNT  P&L  STATEMENT               ║
╠══════════════════════════════════════════════════╣
║                     REVENUE                      ║
║  Zombie eradication bounty ×4           1000.00 ║
║  Total Revenue                          1000.00 ║
╠══════════════════════════════════════════════════╣
║                     EXPENSES                     ║
║  .308 FMJ ×6                          (    7.20)║
║  Night-hunt permit                    (   75.00)║
║  Thermal scope (depreciation)         (    6.00)║
║  Rifle (depreciation)                 (    0.90)║
║  Total Expenses                       (   89.10)║
╠══════════════════════════════════════════════════╣
║  NET PROFIT                              910.90 ║
╚══════════════════════════════════════════════════╝
```

## Scene editor commands

```
list / ls                         List all nodes
map                               Render ASCII top-down map
add zombie <x> <y>                Place a cold zombie
add box <x> <y> <z> <w> <d> <h>  Place an obstacle box
move <id> <x> <y> <z>            Reposition a node
remove / rm <id>                  Delete a node
save <path.json>                  Persist scene to disk
load <path.json>                  Load scene from disk
```

## Running tests

```
cargo test
```
