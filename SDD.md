# DarkAir — Software Design Document (SDD)

**Version:** 1.0
**Date:** August 2, 2026
**Companion document:** SRS.md v1.0

---

## 1. Architecture Overview

DarkAir uses a component-based entity architecture with a central simulation clock driving three coupled systems: the **thermal simulation**, the **optics renderer**, and the **AI/economy layer**. The defining design decision is that *the world is simulated once, in physical terms, and each optic is a lossy view of that single truth.* Optics never get privileged data — they filter it.

```
+------------------------------------------------------+
|                     Game Loop                        |
|  +-----------+  +-----------+  +------------------+  |
|  |  World    |  |  Entity   |  |   Night Clock    |  |
|  |  State    |  |  Manager  |  |  (dusk -> dawn)  |  |
|  +-----+-----+  +-----+-----+  +--------+---------+  |
|        |              |                 |            |
|  +-----v--------------v-----------------v---------+  |
|  |           Thermal Simulation System            |  |
|  |   (per-object temperature, weather, residue)   |  |
|  +-----+------------------------------------------+  |
|        |                                             |
|  +-----v-----------+   +------------------------+    |
|  | Optics Renderer |   |  AI System             |    |
|  |  eye | NV | IR  |   |  pests, friendlies,    |    |
|  +-----+-----------+   |  zombies, noise events |    |
|        |               +-----------+------------+    |
|  +-----v-----+   +---------------+ |                 |
|  |   HUD     |   | Economy/      |<+                 |
|  |           |   | Contracts     |                   |
|  +-----------+   +---------------+                   |
+------------------------------------------------------+
```

---

## 2. Thermal Simulation System

### 2.1 Object thermal model
Every renderable entity carries a `ThermalProfile`:

```
ThermalProfile {
  baseTemp: float        // internal/metabolic temp (pests ~101F, zombies = ambient)
  thermalMass: float     // resistance to cooling (rock high, grass low)
  skyExposure: float     // 0-1; drives radiative cooling below ambient (metal roofs)
  wetness: float         // rain accumulation; pulls temp toward ambient fast
  residualHeat: HeatEvent[]  // bedding spots, tracks, fired barrel
}
```

Per simulation tick (coarse — 1 Hz is sufficient):

```
displayTemp = lerp(displayTemp, targetTemp(ambient, profile), dt / thermalMass)
targetTemp -= skyExposure * radiativeCoefficient   // clear-sky nights only
```

### 2.2 Night contrast curve
Ambient temperature and object convergence follow a scripted curve per night:
- **Dusk (t=0):** stored solar heat maximizes object-vs-object contrast.
- **Mid-night:** exponential decay of stored heat.
- **Crossover (t≈0.85):** scene variance minimized; thermal detection ranges cut by 50–70%.
- Weather modifies the curve: rain collapses it early; clear sky deepens radiative cooling.

This single curve is the game's difficulty dial — no artificial scaling needed.

### 2.3 Residual heat events
`HeatEvent {position, intensity, decayRate}` spawned by: animal resting >30s, animal footfalls on cold ground (brief), rifle discharge (barrel, 90s), pellet impact (5s). Rendered as decaying warm decals in thermal only. These double as a tracking mechanic.

---

## 3. World and Zones

Hand-built zones (not procedural, v1): **Home Farm** (tutorial), **Grain Co-op**, **Creek Bottom**, **Orchard**, **Town Edge**, **Main Street**. Each zone declares:
- Habitat spawn tables (rats→feed sheds, possums→tree canopy nodes, raccoons→patrol routes)
- Hazard placement (wire runs, holes, creek crossings, glass storefronts in town — thermal-opaque)
- Friendly population (farm dog with patrol route, cats in town, livestock pens)
- Zombie density weighting (low on farms, higher near town/cemetery)

Zones connect via a hub-path map; the bicycle upgrade halves travel time (travel consumes night clock, not real-time walking).

---

## 4. Optics Renderer

Three render pipelines over one scene graph:

| Channel | Pipeline | Key rules |
|---|---|---|
| Naked eye | Low-exposure PBR pass | Visibility = f(moon phase, cloud); hazards faintly visible; free |
| NV | Desaturated amplified pass + gain noise + bloom on light sources | Sees everything geometry-wise; fog/rain add scatter; battery drain 1x |
| Thermal | displayTemp → palette LUT (white-hot / black-hot / colorblind-safe) | Geometry with no ΔT vanishes; sky = floor value; glass = opaque; zombies render at ambient (i.e., invisible unless silhouetted against sky); battery drain 2.5x, 3.5x in cold |

Scope-up interpolates from world view to optic view with a vignette; reticle and HUD palette follow the active device (SRS FR-U2).

### 4.1 The zombie invisibility rule
Zombies are ordinary entities with `baseTemp = ambient`. Their invisibility in thermal is *emergent from the simulation*, not a special case — which also means a zombie standing against the cold sky IS faintly visible as an occlusion silhouette, and a zombie that has been indoors near a heater reads slightly warm. These edge cases are deliberate discoverable depth.

---

## 5. Weapon System

### 5.1 Power plant models (per rifle tier)
```
MultiPump { pumpCount 0-8, energyPerPump, pumpTimeSec, pumpNoiseRadius }
  // fire consumes all pumps; re-pumping is an interruptible timed action
UnregulatedPCP { fillPct, capacity, velocityCurve(fillPct) }
  // accuracy modifier follows the fill curve: sweet spot mid-fill
RegulatedPCP { fillPct, capacity, regSetpoint }
  // flat velocity while fillPct > regSetpoint; PowerSetting wheel LOW/MED/HIGH
```
Ballistics: simple drag-free parabolic drop scaled by muzzleEnergy — readable, learnable, cheap. Pellet variants adjust the drop constant and lethal-range table; "matched" pellets add a per-tier accuracy bonus. The unregulated velocity curve is the Tier 2 skill mechanic: players learn to top off at camp and track their sweet spot, and the Tier 3 upgrade's flat curve *feels* like the relief it is in real life.

### 5.2 Hit resolution
Raycast → first entity hit → zone check (`head` vs `body` colliders):
- Pest + head → kill, bounty queued
- Pest + body → wounded flee, alert pulse
- Friendly (any zone) → penalty event (money + reputation), alert pulse
- Zombie + head → destroyed; zombie + body → stagger only
- Backstop rule: continue the ray past the target; if it intersects a friendly within lethal range, the shot is flagged forbidden pre-fire (reticle warning) and penalized post-fire

### 5.3 Noise events
`NoiseEvent {position, radius}` on discharge; moderator multiplies radius ×0.3. Pests inside radius flee; zombies inside radius pathfind toward source for 60s. This makes the moderator the most gameplay-relevant purchase, mirroring real PCP culture.

---

## 6. AI Design

- **Rats:** short feeding loops near spawn nodes; flee on noise/light; fast re-spawn (population pressure justifies contracts).
- **Possums:** slow, tree-biased, freeze-when-lit behavior (real possum behavior — free difficulty reduction the player discovers).
- **Raccoons:** patrol routes + group memory; witnessing a group member's death raises the group's flee threshold for the rest of the night.
- **Friendlies:** dog patrols with warm blob deliberately raccoon-sized at >40 yd in thermal; cows/sheep static in pens but positioned to create backstop dilemmas; cats wander town exactly where rats are.
- **Zombies:** slow wander, noise-seeking, contact damage; spawn weighting rises near town. No bounty — their entire design role is to punish thermal-only play and reward NV scouting and audio awareness.

Identification is the intended skill: every AI decision above exists to force the "positive ID before the trigger" discipline.

---

## 7. Business Simulation and Progression

### 7.1 Starting position
The player begins with **$1,200 investment**. Forced first purchases leave working capital thin by design:

```
Tier 1 multi-pump rifle  $200
Basic 3-9x scope         $60
Headlamp (red filter)    $25
Pellet tin (500)         $18
Working capital          ~$897 (buffer for camp fees, first optic)
```

Night vision and thermal are aspirational at start — the first nights are hunted by moonlight and headlamp, which teaches the terrain before optics arrive.

### 7.2 Rifle ladder
Each tier changes *how the game plays*, not just numbers:

| Tier | Platform | Price | Operating model | Gameplay identity |
|---|---|---|---|---|
| 1 | Multi-pump .22 | $200 | 5–8 pumps per shot; no fills needed | Slow, deliberate; pumping takes ~8s and makes movement noise; infinite "air" but terrible rate of fire; rats/possums only |
| 2 | Unregulated PCP .22 | $500 | ~30 shots/fill; velocity curve — accuracy sweet spot mid-fill, penalties at full and low fill | Shot-count management; learn your fill curve; raccoon-capable |
| 3 | Regulated PCP .22 | $950 (or Tier 2 + $300 regulator retrofit) | Flat velocity to reservoir empty; moderator mount; power wheel | The efficient professional tool; cost-per-kill drops sharply |
| 4 | Premium PCP .25 | $2,000 | High energy, long range, large shot count | Unlocks License D targets; end-game margin machine |

The Tier 2→3 retrofit path deliberately costs less than buying Tier 3 outright *only if the player skipped a regulated Tier 2 variant* — buying regulated then re-regulating is the trap the store tooltip warns about. (A lesson imported directly from real PCP buying advice.)

### 7.3 Optics ladder
Like the rifles, each optic tier changes how the night is played. Specs mirror real-world differentiators (resolution, sensitivity, refresh, battery):

| Optic | Price | Features | Limits |
|---|---|---|---|
| Headlamp, red filter | $25 | Free "optic"; short cone of visibility; red light spooks pests less | Announces your position; useless past ~25 yd |
| Digital NV Gen-basic | $350 | Monochrome scene, built-in IR illuminator, sees all terrain/hazards | Grainy past 60 yd; IR beam visible to raccoons (they learn it); battery 1x |
| Digital NV Pro | $550 | Cleaner sensor, longer IR reach (~120 yd), better fog penetration | Battery 1x |
| Thermal 256 ("Mk I") | $950 | 256×192 sensor, 40mK sensitivity, 25 Hz; detection to ~150 yd | Blob-level ID only past 50 yd; heavy crossover penalty; battery 2.5x |
| Thermal 384 ("Mk II") | $1,100 upgrade (from Mk I) / $1,950 outright | 384×288, 25mK, 50 Hz; detection ~300 yd, ID to ~100 yd; raised contrast floor softens crossover | Battery 2.5x |
| Thermal 640 ("Mk III") | $2,100 upgrade | 640×480, <20mK; near-ID-grade imaging; mild crossover immunity | Battery 3x; end-game purchase |

Design rationale: detection vs. identification is the axis. Thermal tiers buy *detection range* and *crossover tolerance*; NV tiers buy *identification confidence* and hazard safety. The sensitivity/resolution spec language (NETD-style mK numbers, sensor resolution, refresh rate) is surfaced in store tooltips so players shop the way real buyers do — and learn that the optic, not the rifle, is the decision-critical purchase.

### 7.4 License/commission gating
```
License A  included    rats                        $8/head
License B  $150        + possums                   $25
License C  $400 + rep>50 (any farm) + Tier 2+      + raccoons $60
License D  $900 + town rep>80 + Tier 4             + groundhog $90, beaver $140,
                                                     juvenile feral hog $200
```
Shooting an unlicensed species pays nothing and costs reputation (poaching). This makes target ID doubly valuable: species determines both legality and revenue.

### 7.5 Operating costs and P&L
Nightly costs: camp fee $15, pellets at cost, battery wear $2/hr of optic use, maintenance accrual per shot (higher for Tier 1 pump linkage). End-of-night screen:

```
NIGHT 7 — GRAIN CO-OP          
Bounties (11 rats, 2 possums)   +$138
Penalty (cat)                   -$150
Operating costs                 -$31
NET                             -$43   Balance: $612
```

Losing money on a bad night must be possible — the friendly-fire penalty only matters if margins are real. Bankruptcy (cash < 0, no sellable assets) ends the campaign.

### 7.6 Efficiency upgrades (cost-per-kill reducers)
Moderator $200 (Tier 3+ mount) · battery pack $150 · larger tank $250 · bicycle $300 · matched pellets $30/tin (accuracy bonus per rifle tier) · scope magnification $120 · laser rangefinder $350 (live range + computed holdover chevron on the reticle scale). (Optic tier upgrades are priced in §7.3.)

### 7.7 Intended arc
Nights 1–3: multi-pump + moonlight, rats only, thin margins, learn the map. Night ~4: NV purchase transforms navigation. Night ~6: Tier 2 PCP + License B. Night ~9: thermal purchase transforms detection — and the first zombie encounter punishes over-trusting it. Mid-game: Tier 3 + moderator, License C, margins open up. End-game: Tier 4 + License D municipal commission, zombie-dense town zones, weather-timing mastery.

## 7A. Weather Economics

Forecast is shown at camp before committing to a night. Each type reshapes the expected-value calculation:

| Forecast | Thermal | NV | Pest activity | Best play |
|---|---|---|---|---|
| Clear/cold | Excellent (deep crossover late) | Good | Normal | Hunt early, quit before crossover |
| Overcast | Average | Dim | Normal | Thermal night |
| Fog | Slightly reduced | Poor | **High** | Thermal + audio; premium night for the equipped |
| Rain | Collapsed | Poor | **Low** (sheltering) | Skip night (pay camp fee) or hunt barn interiors |
| Heat wave | Poor all night | Good | High | NV night — the inversion that keeps NV relevant late-game |
| Pre-storm surge | Good | Good | **Very high** (rat surge contracts) | The jackpot night; contracts pay bonuses |

Skipping a bad night costs $15 and a contract-deadline day — making *when not to work* a genuine business decision. Weather also couples to the thermal sim (§2.2): rain collapses the contrast curve early; clear-sky nights deepen radiative cooling (metal roofs read below ambient).

---

## 8. HUD Design

Persistent minimal strip (bottom): `AIR 68% | 14 shots | pellets 43 | BATT 51% | 02:14 to dawn | $312` — the air segment becomes a pump indicator (`PUMPS 6/8`) on Tier 1. Camp adds two panels: the **forecast panel** (tomorrow's weather + modifiers, shown before committing) and the **ledger** (P&L history, cost-per-kill trend — the player's own efficiency made visible).
Health as an edge vignette, not a bar. Contract quota as a corner tally (`RATS 6/10`). Reticle warnings: backstop-friendly flag, out-of-lethal-range dimming. Camp screens: Loadout (optic/power/refill), Store, Contract Board (client reputation shown as handshake icons).

## 9. Audio Design

Audio is the fourth optic. Layered positional cues: rustle (pest movement class-distinguishable), moan/drag (zombie), dog bark (friendly proximity warning), creek noise (hazard). The thermal's inability to see zombies is balanced entirely through audio — headphone play is the intended experience.

## 10. Technology Notes (v1 targets)

**Language: Rust throughout** — engine, editor, thermal simulation, and tooling share one codebase.

Reference stack:
- **Rendering:** `wgpu` (WebGPU) — one renderer serves the game's three optic pipelines and the editor viewport; targets native and web (WASM) from the same code
- **Windowing/input:** `winit`; **editor UI:** `egui` (immediate-mode panels: outliner, timeline, property inspector, code editor)
- **Scene graph:** OSG-style retained graph — `Group`, `Transform`, `Geode`/`Drawable`, `Switch`, `LOD` node types; visitor-pattern traversals for update, cull, and thermal-tick passes; state (materials, thermal profiles) inherited down the graph
- **Math:** `glam`; **serialization:** `serde` (RON for human-editable scene files, binary for runtime)
- **Parametric layer (OpenSCAD spirit):** scenes compile from a declarative Rust DSL / RON description — `barn(width, bays, roof: Metal)` expands deterministically into a subgraph with geometry *and* thermal profiles attached (metal roof gets high `skyExposure` automatically). Text source is the ground truth; the graph is the build artifact.
- **Animation:** keyframe tracks bound to node properties (transform, switch state, thermal params), edited on the Blender-spirit timeline; animal gaits and zombie shamble as authored clips + procedural blend
- **Editor↔game loop:** the editor embeds the live game renderer, so a zone can be play-tested in-place with any optic pipeline active — including previewing a scene's thermal appearance at any night-time `t`, which makes the crossover curve an authorable, visible property

Thermal sim ticks at 1 Hz with per-object LOD via a dedicated traversal; render at frame rate via interpolation. Save data: versioned RON (money, rep, upgrades, contract state).

## 11. Traceability

Every FR in SRS.md maps to a section here: Weapon §5 (FR-W*), Optics §4 (FR-O*), Thermal §2 (FR-T*), AI §6 (FR-A*), Hazards §3+§6 (FR-H*), Economy §7 (FR-E*), Session §2.2+§3 (FR-S*), HUD §8 (FR-U*).
