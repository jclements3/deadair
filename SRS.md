# DarkAir — Software Requirements Specification (SRS)

**Version:** 1.0
**Date:** August 2, 2026
**Status:** Draft for review

---

## 1. Introduction

### 1.1 Purpose
This document specifies the functional and non-functional requirements for **DarkAir**, a first-person night-hunting game in which the player earns money as a contract pest exterminator using a .22 caliber PCP air rifle and swappable night optics, while avoiding hazards, protected animals, and wandering zombies.

### 1.2 Project Goal Statement
The goal of this project is twofold and inseparable:

1. **Ship the game.** Deliver DarkAir as a playable, complete night-hunting business simulation — silent air rifles, physically honest optics, warm targets, cold zombies, and a real profit-and-loss game underneath it.

2. **Build the tools that build it.** Develop a scene and animation editor in **Rust** on an OpenSceneGraph-style architecture — a retained scene graph with typed nodes, traversals, and state inheritance — that fuses two authoring philosophies:
   - **In the spirit of Blender:** direct-manipulation 3D viewport editing, keyframe/timeline animation, an outliner over the live scene graph, and a property-driven workflow for artists.
   - **In the spirit of OpenSCAD:** scenes and assets as *code* — declarative, parametric, diffable, version-controllable model definitions where a barn, a fence line, or a raccoon patrol route is a function of its parameters, and regeneration is deterministic.

   Every DarkAir zone, entity, thermal profile, and animation is authored in this editor, so the tool is proven by the game and the game is reproducible from text. The entire stack — engine, editor, simulation — is written in Rust for memory safety, performance, and a single-language codebase from the thermal solver to the viewport.

**Success means:** a stranger can play DarkAir start to bankruptcy-or-riches, and a developer can rebuild any scene in it from parametric source files using the editor alone.

### 1.3 Scope
DarkAir simulates realistic night-optics physics (light amplification vs. thermal imaging), PCP air rifle resource management, and a contract-based bounty economy. The core fantasy: every shot is silent, every target is warm — except the ones that aren't.

### 1.4 Definitions
| Term | Definition |
|---|---|
| PCP | Pre-charged pneumatic air rifle; fires pellets using stored compressed air |
| NV | Night vision (image intensification); amplifies ambient light |
| Thermal | Long-wave infrared imaging; displays temperature differences only |
| Crossover | Pre-dawn period when scene temperatures converge and thermal contrast collapses |
| Friendly | Protected animal (dog, cat, cow, sheep) that must not be shot |
| Zombie | Ambient-temperature hostile; invisible to thermal, killable only by headshot |
| Camp | Home base where the player refills air, swaps optics, recharges batteries, and buys upgrades |

---

## 2. Overall Description

### 2.1 Game Concept
The player is a night-shift pest contractor clearing rats, raccoons, and possums from farms and a small town. Payment is per confirmed kill. Shooting protected animals costs money and reputation; lost reputation cancels contracts. Zombies roam randomly — they are cold, silent, and invisible to the thermal scope the player otherwise depends on.

### 2.2 Core Gameplay Loop
1. Start at camp: choose optic, set rifle power, check air/pellets/battery.
2. Travel to contract zone on foot (or bike, once purchased).
3. Detect, identify, and eliminate pests before dawn.
4. Avoid hazards, friendlies, and zombies.
5. Return to camp to refill/recharge/swap equipment.
6. Collect bounties, spend on upgrades, accept new contracts.

### 2.3 Player Perspective
First-person. The default view is the unaided eye; raising the rifle switches to the scope view of the currently mounted optic.

---

## 3. Functional Requirements

### 3.1 Weapon System
- **FR-W1:** The rifle SHALL be a .22 PCP with a finite air reservoir measured in fill percentage.
- **FR-W2:** Each shot SHALL consume air proportional to the selected power setting.
- **FR-W3:** The player SHALL be able to select power at camp: LOW (more shots, short lethal range — rats), MEDIUM, HIGH (fewer shots, full lethal range — raccoons).
- **FR-W4:** Pellets SHALL be a separate consumable from air.
- **FR-W5:** Pellet trajectory SHALL exhibit drop with distance; higher power flattens trajectory.
- **FR-W6:** Air refill SHALL be possible only at camp.
- **FR-W7:** An unmoderated shot SHALL generate a noise event that alerts animals (flee) and zombies (investigate) within a radius. A purchasable moderator SHALL reduce that radius by at least 70%.
- **FR-W8:** A recently fired barrel SHALL display as warm in thermal view for a limited duration.

### 3.2 Optics System
- **FR-O1:** Exactly one optic SHALL be mounted at a time; swapping SHALL occur only at camp.
- **FR-O2:** **Naked eye** SHALL render the scene based on moon phase and cloud cover; no battery cost.
- **FR-O3:** **Night vision** SHALL render a monochrome amplified-light scene showing terrain, vegetation, hazards, sky, moon, and all warm or cold bodies; degraded by fog and rain; consumes battery.
- **FR-O4:** **Thermal** SHALL render temperature contrast only:
  - Warm bodies (pests, friendlies, player's dog-sized blobs) visible at all light levels.
  - Terrain hazards (holes, wire, limbs, water edges) SHALL NOT be reliably visible.
  - Zombies SHALL NOT be visible (ambient temperature).
  - Sky SHALL render as uniform cold; glass SHALL be opaque.
  - Consumes battery at a higher rate than NV; drain increases in cold weather.
- **FR-O5:** Battery SHALL be a shared consumable rechargeable only at camp.

### 3.3 Thermal World Simulation
- **FR-T1:** Every object SHALL carry thermal properties (thermal mass, emissivity proxy) causing its displayed temperature to evolve across the night.
- **FR-T2:** Scene contrast SHALL follow a curve: high contrast at dusk → progressively flatter → minimum contrast at pre-dawn crossover.
- **FR-T3:** Rain SHALL cool exposed surfaces toward uniformity; light fog SHALL minimally affect thermal but significantly degrade NV and naked eye.
- **FR-T4:** Residual heat SHALL be simulated: animal bedding spots, fresh tracks, fired barrel, spent pellets.

### 3.4 Targets and AI
- **FR-A1:** Pest types and base bounties: rat (low), possum (medium), raccoon (high).
- **FR-A2:** Pests SHALL have habitat-driven spawn logic: rats near barns/feed, possums in and near trees (including elevated positions), raccoons wide-ranging and evasive.
- **FR-A3:** A headshot SHALL kill any pest instantly. A body hit SHALL wound: the animal flees, no bounty is paid, and nearby animals are alerted.
- **FR-A4:** Raccoons SHALL exhibit learned avoidance after witnessing a kill in their group.
- **FR-A5:** Friendlies (dog, cat, cow, sheep) SHALL share silhouette/thermal-blob ambiguity with pests at distance, requiring positive identification.
- **FR-A6:** Hitting a friendly SHALL deduct money and reputation; each farm/town contract SHALL have a reputation threshold below which the contract is cancelled.
- **FR-A7:** Zombies SHALL spawn randomly, move slowly, deal contact damage, and die only to a headshot. Zombies pay no bounty (design decision: pure hazard, incentivizing avoidance).
- **FR-A8:** A shot SHALL be forbidden when a friendly is in the line of fire behind the target (no safe backstop); such hits count as friendly hits.

### 3.5 Hazards and Health
- **FR-H1:** The player SHALL have a health pool reduced by trips/falls (holes, wire, limbs, creek banks), zombie contact, and drowning-adjacent water hazards.
- **FR-H2:** Trip hazards SHALL be visible in NV and (moon-dependent) naked eye, but not in thermal.
- **FR-H3:** Health SHALL recover only at camp.
- **FR-H4:** Health reaching zero SHALL end the night with a monetary penalty (medical costs) and loss of unclaimed bounties.

### 3.6 Economy and Progression
- **FR-E1:** Bounties SHALL pay per confirmed kill, collected on return to camp.
- **FR-E2:** Contracts SHALL define zone, quota, deadline (nights), and reputation requirements.
- **FR-E3:** Purchasable upgrades SHALL include: moderator, larger air reservoir, higher-capacity battery, improved thermal (better resolution/sensitivity, wider crossover tolerance), improved NV, scope magnification, pellet variants (trajectory/energy tradeoffs), bicycle (travel speed).
- **FR-E4:** Campaign completion SHALL require clearing all contract zones while maintaining reputation above zero.

### 3.7 Business Simulation
- **FR-B1:** The player SHALL begin with a fixed starting investment (cash) used to buy the initial rifle, optic, and consumables; remaining cash is working capital.
- **FR-B2:** Rifles SHALL form an upgrade ladder with distinct operating characteristics:
  - **Tier 1 — Multi-pump .22:** cheap; no air reservoir; requires 5–8 pump strokes between shots (time cost, movement noise, stamina drain); low power ceiling.
  - **Tier 2 — Unregulated PCP .22:** shot capacity per fill; velocity varies across the fill curve (accuracy penalty at low fill); camp refills required.
  - **Tier 3 — Regulated PCP .22:** consistent velocity across the fill; more usable shots; supports moderator and power tuning.
  - **Tier 4 — Premium PCP .25:** highest energy and range; unlocks the largest pest class.
- **FR-B3:** Contract access SHALL be gated by **licenses/commissions** purchased with cash and unlocked by reputation:
  - License A (starter): rats only.
  - License B: + possums.
  - License C: + raccoons; requires Tier 2+ rifle.
  - License D (municipal commission): + premium targets (beaver, groundhog, feral hog juveniles) at top bounties; requires Tier 4 rifle and >80 town reputation.
- **FR-B4:** The game SHALL charge recurring operating costs per night: pellets consumed, battery wear, camp fees, and equipment maintenance; the end-of-night screen SHALL present a profit/loss statement (revenue − costs).
- **FR-B5:** Efficiency upgrades SHALL reduce cost-per-kill or time-per-kill (moderator, bike, larger tank, better optics, pellet match to rifle).
- **FR-B6:** Bankruptcy (cash below zero with no sellable assets) SHALL be a fail state; equipment SHALL be sellable at depreciated value.

### 3.8 Weather and Profitability
- **FR-WX1:** Each night SHALL have a forecast (visible at camp before committing): clear, overcast, fog, rain, cold snap, heat wave.
- **FR-WX2:** Weather SHALL modify detection and operations:
  - Clear/cold: best thermal contrast; fastest battery drain; deepest pre-dawn crossover.
  - Overcast: dim naked eye/NV; average thermal.
  - Fog: thermal mildly reduced; NV heavily reduced; pest activity high.
  - Rain: thermal contrast collapses; pests shelter (low spawn rates); trip hazards worsen.
  - Heat wave: warm ambient narrows pest-vs-background contrast all night.
- **FR-WX3:** Weather SHALL modify economics: pest activity multipliers change expected revenue per hour; the player MAY skip a night (paying camp fees only) — choosing when *not* to hunt SHALL be a valid profitable strategy.
- **FR-WX4:** Certain contracts SHALL carry weather bonuses (e.g., rat surge in the grain co-op just before a storm).

### 3.9 Time and Session Structure
- **FR-S1:** Each session SHALL run from dusk to dawn on an accelerated clock.
- **FR-S2:** Dawn SHALL end the hunt; the player must return to camp before first light or forfeit a travel-time penalty.
- **FR-S3:** Difficulty SHALL naturally scale within a night via the thermal contrast curve (FR-T2).

### 3.10 HUD Requirements
- **FR-U1:** The HUD SHALL persistently display: air fill % (or pump state for Tier 1), shots remaining, pellet count, battery %, active optic, health, clock (time until dawn), current contract quota progress, and cash.
- **FR-U5:** The end-of-night screen SHALL display a P&L: bounties earned, penalties, operating costs, net profit, and running business balance.
- **FR-U6:** The camp forecast panel SHALL show tomorrow night's weather and its expected activity/contrast modifiers before the player commits.
- **FR-U2:** Optic-dependent HUD styling SHALL match the active device (e.g., thermal reticle and palette vs. NV green).
- **FR-U3:** At camp, a loadout screen SHALL allow optic swap, power setting, refills, and purchases.
- **FR-U4:** A contract board at camp SHALL list available/active contracts and reputation per client.

---

## 4. Non-Functional Requirements

- **NFR-1 (Performance):** Stable frame rate on mid-range hardware; thermal simulation LOD may reduce update frequency for distant objects.
- **NFR-2 (Usability):** A first-night tutorial contract SHALL teach optics switching, power tradeoff, and the friendly-fire rule.
- **NFR-3 (Accessibility):** Thermal palettes SHALL include white-hot, black-hot, and a colorblind-safe option; subtitles for audio cues; remappable controls.
- **NFR-4 (Audio):** Positional audio SHALL be a first-class detection channel (rustling, zombie moans) since thermal hides zombies and darkness hides everything else.
- **NFR-5 (Tone):** Violence is stylized, not gory; the game is dark-comedy rural horror, suitable for a Teen rating target.
- **NFR-6 (Persistence):** Game state (money, reputation, upgrades, contract progress) SHALL persist between sessions.

---

## 5. Constraints and Assumptions

- Single-player only (v1).
- Air rifles only (multi-pump and PCP ladder, .22/.25); no firearms, keeping the silent-hunting identity intact.
- Zombies are ambient-temperature "dead" type by lore; no fever-hot infected variants in v1.
- No in-game purchases; economy is fully internal.

## 6. Open Items

| # | Item | Status |
|---|---|---|
| 1 | Zombie bounty (currently: none — pure hazard) | Decided, revisit after playtest |
| 2 | Difficulty modes vs. single tuned curve | Open |
| 3 | Procedural vs. hand-built zones | See SDD §3 |
