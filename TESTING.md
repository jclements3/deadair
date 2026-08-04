# Calibration & performance findings

Measured 2026-08-04 on the WSL2 dev box (llvmpipe software Vulkan — the
CPU-worst-case renderer; any real GPU collapses these numbers). Reproduce
with `deadair --bench` and `deadair --shimmer`; interactive verification
lives in the in-app calibration range (the default view).

## Render timings (1024×1024, release, llvmpipe)

| optic | rabbits | 2.0× avg | 14.5× avg | 200-rabbit p95 |
|---|---|---|---|---|
| eye | 6 | 24.8 ms | 33.2 ms | 81.5 ms |
| nv | 6 | 29.8 ms | 30.5 ms | 71.6 ms |
| thermal | 6 | 15.4 ms | 17.5 ms | 62.8 ms |

Full table: run `--bench`. Reading:

- **Thermal is the cheapest pipeline** (no bloom pass), eye the dearest.
- **llvmpipe cannot hold 60 fps at 1024²** even at the 6-rabbit baseline
  (~25–33 ms ≈ 30–40 fps); the 200-rabbit stress dial runs ~14–22 fps.
  Frame time IS the latency floor: at 30 fps, mouse-to-photon ≥ ~2 frames
  ≈ 60–70 ms. This is a software-rasterizer ceiling, not a scene cost —
  the fix is hardware Vulkan (mesa's `dozen` D3D12 layer, or native), or a
  render-scale option (768² would roughly halve fragment cost).
- Cost scales with covered pixels more than with object count: high zoom is
  often *cheaper* because fewer large surfaces fill the frame.

## Shimmer & determinism (`--shimmer`)

- **Re-render determinism: BYTE-IDENTICAL in all three pipelines** once the
  thermal AGC has settled (~2–4 s after a scene change). A static scene at
  fixed magnification does not boil — any shimmer you see in motion is
  aliasing, not nondeterminism. (First probe run flagged thermal MISMATCH;
  that was the probe measuring AGC convergence, not rendering. The AGC's
  slow creep after scene changes is deliberate device-realistic behavior.)
- **Zoom crawl proxy** (mean |Δpixel| between adjacent 0.25× mag steps,
  center crop over the checkerboards): eye 4.95, nv 14.19, thermal 21.30
  per 255. These are baselines, not pass/fail — magnification legitimately
  changes the image; watch this number for regressions. Thermal reads
  worst because its checkerboard cells alternate ±30 °F, so a one-pixel
  edge shift flips full-contrast cells.
- **No MSAA is the root cause of visible crawl at high zoom.** The
  geometry pass renders 1 sample/pixel; thin members (picket fence,
  checker edges) alias. Recommendation: optional 4× MSAA on the geometry
  pass once running on hardware (on llvmpipe it would quadruple fragment
  cost — do not enable there).

## What the in-app range verifies by eye

Checkerboards at 10/25/50/75/100 m and the 30 m picket fence show crawl
first; rabbits at up to 200 exercise the exact hop the hunts use;
click-flash gives filmed mouse-to-photon counts; the sparkline shows
frame-time spikes against the 16.6 ms line.

## Machine calibration (`--calibrate`)

Adaptive binary search on rabbit count against a p95 frame-time budget, per
magnification, thermal pipeline, fully deterministic scene — the numbers
are comparable across hosts. Headline rating = worst case across mags at
sustained 30 fps. Writes `~/.deadair-calibration.ron` for future
auto-tuning (zone density defaults).

First card — the 8-cpu WSL2 laptop on llvmpipe:

```
budget 30 fps:  2.0x → 131   8.0x → 151   14.5x → 146
budget 60 fps:  ~0 at all mags (the empty range alone exceeds 16.7 ms)
RATING: 131-rabbit machine
```

Reading: this laptop sustains ~130 fully-rigged hopping rabbits in frame at
30 fps, and cannot reach 60 fps at 1024² on a software rasterizer at all —
that line is llvmpipe's floor, not scene cost. Run the same command on the
40-cpu lab box for its card; llvmpipe scales with cores, so expect several
hundred, and any real GPU pushes the 60 fps row from zero into the
hundreds.

## Update: device-true AGC + sensor-resolution benchmarks

The thermal AGC no longer estimates scene coverage — it histograms the
rendered temperature buffer itself (exactly what a real core does), so
occlusion, framing, and zoom are automatically correct. Scene-side
estimation was structurally unable to know that six background canopies
overlap behind a hog.

Re-benched at real device resolutions (thermal = Mk II 288, NV = 720):
thermal 6 rabbits ≈ 8–9 ms (>100 fps), 200 rabbits at 14.5× ≈ 18 ms.
The sensor pass made the hunt's primary pipeline the fastest one — 60 fps
scoped thermal is now real on the 8-cpu llvmpipe laptop. Native-res
thermal (unused by the game) pays a histogram readback and is bench-visible
only.
