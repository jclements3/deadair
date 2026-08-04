# Optic pipeline look references

Distilled from real scope footage supplied 2026-08-02 (`videos/` — two screen
recordings: HIKMICRO thermal rabbit cull; digital NV pest shooting). Frame
grabs kept beside this file. These are the *verification targets* for
da-render's optic pipelines — when a rendered frame is side-by-side plausible
with these, the pipeline passes.

## Thermal (white-hot) — `thermal_ref_wide.png`, `thermal_ref_close.png`

- **Circular scope mask** over black; HUD elements live OUTSIDE the circle
  corners (kill counter top-center, zero profile / clock / drop readout
  top-right in translucent boxes).
- Warm bodies: **saturated white blobs with soft bloom halo**; the hottest
  spot on a target carries a small **red/orange accent** (device "target
  highlight" feature — we can gate this on optic tier Mk II+).
- **Thermal reflection**: warm bodies mirror faintly on damp ground below
  them (close-range frame shows a rabbit's reflection). Cheap win: flipped
  low-alpha blob decal on wet/smooth ground.
- Ground: mottled mid-gray with vegetation clumps darker AND lighter —
  low-frequency patchiness, not uniform. Grass tufts in foreground read
  near-black (cold, low emissive angle).
- Background trees/fog: **washed light gray, low contrast, soft** —
  atmospheric attenuation flattens everything distant toward a uniform
  value. Distance fog in temperature space, not color space.
- Fence posts/wire: dark silhouettes, clearly visible near, lost far —
  consistent with "hazards unreliable in thermal" (FR-O4).
- Fine black **thin-line reticle** (barely visible against ground, clear on
  white targets).
- Motion: whole-scene uniform value shifts when panning across sky vs ground
  (AGC behavior) — a subtle full-frame auto-gain lerp sells realism.

## Digital NV — `nv_ref.png`

- Full-frame (no circular mask on this device), **grainy monochrome**,
  slightly milky blacks; visible sensor noise everywhere, heavier in dark
  areas; soft focus + motion smear when panning.
- **Red illuminated duplex crosshair** (thick outer bars, thin center),
  rendered as an emissive overlay that blooms slightly — NOT a crisp vector.
- All geometry readable: ground texture, tree trunks (dark), fence posts,
  terrain relief. This is the "sees everything, identifies everything close"
  channel per SDD §4.
- Timestamp overlay corner (flavor option for the digital-NV HUD).

## Smart-scope comparison footage (third video) — `thermal_blackhot_ref.png`, `nv_ir_eyeshine_ref.png`

Same device platform (ATN-style smart scope, rounded-rect eyebox mask, not
circular) switching between black-hot thermal and IR-illuminated digital NV
on feral hogs. This is our optic-tier HUD/look reference for higher tiers:

- **Black-hot thermal**: hog reads *dark on light warm ground*. At close
  range (14.5×) there is real **coat texture** inside the silhouette — hot
  skin through thin fur vs cold guard hair — not a flat blob. At distance
  the same animal is a featureless smudge (detection vs identification axis,
  SDD §7.3). Background collapses into soft horizontal gray bands.
- **Digital NV + IR illuminator**: hogs read **dark** (fur absorbs IR) on a
  bright grainy IR-lit ground — the *inverse* of thermal expectations. Fence
  posts bright. And the killer detail: **eyeshine** — animal eyes
  retro-reflect the IR beam as brilliant white dots. Gameplay hooks:
  - Eyeshine is an NV-only detection channel (thermal has none) — balances
    thermal's blob advantage and rewards NV scanning.
  - Eyeshine color/height helps ID species at distance (positive-ID skill).
  - Zombies get **no eyeshine** (dead retinas don't retro-reflect) — a
    second subtle zombie tell for NV users, complementing thermal absence.
  - Raccoons "seeing" the IR beam (SDD §7.3 learned avoidance) now has a
    physical basis worth surfacing in a tooltip.
- **Smart-scope HUD** (higher-tier optics): magnification readout, compass
  strip (NW·N·NE), windage/elevation scales flanking the view, ballistic
  drop marks below center reticle, Bluetooth/battery/recording icons, menu
  bar. Lower tiers should feel analog by contrast (plain reticle, no data).

## HUD notes for the game (FR-U2)

- Thermal HUD: white-on-black boxed readouts, kill-counter box top-center
  (diegetic — the real device counts), drop indicator in cm at current zero.
- NV HUD: red/amber elements only (preserve dark adaptation), corner
  timestamp aesthetic.

## Rabbit ground truth (`rabbit_comparison.png`)

Isolated 40 m crops from the HIKMICRO cull footage (top row) against our
rig through the same pipeline (bottom row: graze / sit-up / bound). What
the footage established, now encoded in the rig and its tests:

- Feeding read = **rump dome tapering to a ground-level head, ear spike
  still up** (the ID tell survives feeding). Grazing motion is an inchworm
  creep; the bound hop is for relocation/flight only.
- Alert sit-up is tall with a V of ears — the pause a hunter shoots on.
- Legs do not resolve at 40 m; the blob + ears carry the silhouette.

Known gaps, deliberate for now: our ground reads lighter than the footage
in sparse scenes (their window's top half is filled by warm vegetation —
composition, not calibration; real zones have that composition), grazing-
angle ground noise streaks horizontally (theirs is 3D grass structure, ours
is a flat field — needs geometric tufts eventually), and real coats show
internal texture where ours saturate flat white (needs per-part temps).
