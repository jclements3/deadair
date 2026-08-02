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

## HUD notes for the game (FR-U2)

- Thermal HUD: white-on-black boxed readouts, kill-counter box top-center
  (diegetic — the real device counts), drop indicator in cm at current zero.
- NV HUD: red/amber elements only (preserve dark adaptation), corner
  timestamp aesthetic.
