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

RESOLVED (see `range_vs_footage.png`): the light-ground inversion had two
real causes, both fixed. (1) The temp G-buffer cleared to 0 °F, so every
sky pixel fed a phantom cold mass into the device-true AGC histogram and
stretched the window until the frosted dirt mapped near-white — it now
clears to the sky temperature, which is also what a real core reads off
clear night sky. (2) The calibration range had an empty horizon; a scrub
bank now closes its far end like the footage's tree line, anchoring the
window's hot end. With both in, a field-framed view (no sky) renders white
rabbits on a dark mottled field; tilting sky into frame legitimately
lightens the ground, matching the footage's own panning AGC behavior.
Verify headlessly with `deadair --shot-range out.png 6.0 -2.5`.

RESOLVED (see `coat_texture_compare.png`): coat-interior texture. Animal
parts now carry `coat_f` mottle — object-space noise (pattern rides the
body through the gait), streak-anisotropic along the hair lay, amplitude
~70 % of the species' insulation depth on the trunk, near-zero on faces
and bare ears, exactly zero on zombies (a uniform surface is the second
thermal tell). Quad trunks are ellipsoids now, not cylinders — a lying
cylinder reads as a rectangle side-on at close range. At distance the
sensor blur collapses it all back to the same flat blob, which is the
detection-vs-ID axis working as intended.

Known gaps, deliberate for now: grazing-angle ground noise streaks
horizontally beyond the tuft ring (theirs is 3D grass structure across the
whole field), and the clip's finest hair-streak frequency is beyond a
288-class sensor — compare at the 480 tier.

## Isolated rabbit clips (`../../videos/clips/`)

Magnified cuts from the source video, for verification against the live rig:

- `rabbit_covey.mp4` — the opening group: feeding creep, alert sit-ups,
  several animals in frame (the witness-freeze mechanic's source).
- `rabbit_feeding_40m.mp4` — the LRF'd 40 m rabbit: rump-dome graze with
  the ear spike up, then the sit-up (the rig's graze/sit postures).
- `rabbit_scatter_slowmo.mp4` — half-speed flight: bodies fully stretched
  into long low streaks, ears flat back. The bound rig's airborne phase
  should extend toward this read (currently fixed-length — known gap).

## Isolated rat & boar clips (`../../videos/clips/`)

- `rat_close_nv.mp4`, `rat_creep_eyeshine.mp4` — the NV rat read: a dark
  hunched ball creeping low, led by a single brilliant eyeshine dot. The
  eye outshines the body by an order of magnitude — for rats in NV, the
  eyeshine IS the detection event (our eyeshine channel, validated for the
  smallest quarry).
- `boar_blackhot_close.mp4` — the part-level heat map on a close hog in
  black-hot: head, ears, and legs read hottest (darkest), the coat is
  mottled mid-tone, tail curl visible. This is the target for per-part rig
  temperatures (currently flat 101 °F — known gap).
- `boar_longrange.mp4` — the same species collapsed to a smudge at range:
  the detection-vs-ID axis in one clip.
- `boar_nv_eyeshine.mp4` — hog pair in NV: dark bodies, bright posts, twin
  eyeshine dots leading each animal.
