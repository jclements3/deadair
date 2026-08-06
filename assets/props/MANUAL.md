# Authoring `.vim` objects — the DarkAir prop manual

`.vim` files are small CAD scripts: a Nim-flavored modeling language evaluated
on a pure-Rust CSG kernel (`crates/da-csg`, harvested from the vali project).
Text is ground truth — the same script always produces the identical solid.

Deep language reference with every builtin signature: `VALI_LOKI_OSG_DSL_PRIMER.md`
(repo root). This manual is the working guide.

## Where props live

| path | what |
|---|---|
| `assets/props/*.vim` | zone props, placed with `VimProp(...)` in a `.zone.ron` |
| `assets/props/builtin/*.vim` | templates behind the built-in generators (Silo, StreetlightRow, RadioMast, DumpsterRow, Cemetery) — baked into da-param at compile time, so edits need a rebuild |
| `crates/da-csg/assets/lathes/*.vim` | vali sample scripts — living DSL documentation, not game content |

## Quickstart

The smallest useful prop:

```vim
# feed_trough.vim — a hollowed box on the ground.
let body = box(2.4, 0.8, 0.5).move(0, 0, 0.25)     # base at z = 0
let bowl = box(2.2, 0.6, 0.45).move(0, 0, 0.35)
model body - bowl
```

Place it in a zone:

```ron
VimProp(src: "props/feed_trough.vim", pos: (52.0, 0.0, 28.0), yaw_deg: 10.0, thermal: Metal),
```

Iterate fast, without launching the game:

```bash
# geometry check: ISO 128 multiview drawing (add --iso / --section to taste)
PATH=/snap/bin:$PATH cargo run -q -p da-csg --bin vimtool -- assets/props/feed_trough.vim --svg /tmp/t.svg
# mesh export for any external viewer
... vimtool -- assets/props/feed_trough.vim --stl /tmp/t.stl
# every prop in assets/props/ must compile — this test enforces it
PATH=/snap/bin:$PATH cargo test -p da-param vim_props
```

## Conventions (the four rules)

1. **Meters, Z-up.** Scripts are authored Z-up (vali convention); da-csg
   converts to the game's Y-up automatically. Round primitives run along +Z.
2. **Base at z = 0.** Primitives are *centered* at the origin — `.move` your
   geometry up so the prop stands on the ground (`cylinder(r, h).move(0,0,h/2)`).
3. **Keep `seg` ≤ 48–64.** Booleans are exact BSP; a dense helix cut into a
   dense lathe is valid but slow at zone load.
4. **Name your parts.** A `let` binding a solid stamps that name on its
   polygons as a part tag. Multi-material objects are split per part name and
   mapped to thermal/emissive materials (see "Materials" below).

## The language in one screen

```vim
let name = expr        " binding: a number, sketch, or solid ('#' or leading '"' = comment)
model expr             " REQUIRED: the solid to render

" numbers & arithmetic drive parametric parts
let wall = 12
let bore = wall - 4
let pipe = cylinder(wall, 40) - cylinder(bore, 42)

" calls: parens with named args, or Nim command style
cylinder(r = 8, h = 42, seg = 64)
cylinder 8, 42, 64

" method chains on solids
cube(3).move(16, 0, 0).rotatez(45).fillet(0.5, 8)
```

**Primitives** (centered, +Z): `cube(s)` `box(w,d,h)` `cylinder(r,h)` `cone(r,h)`
`frustum(r1,r2,h)` `sphere(r)` `torus(R,r)` `wedge(w,d,h)` `pyramid(w,d,h)` `tube(R,r,h)`

**Sketches** (2D, inert until lifted): `circle` `rect` `ngon` `roundrect` `slot`
`polygon(x0,y0,…)` `svgpath("d")` `bezier(...)` and the math sections
`limacon` `cardioid` `rose` `star` `superellipse` `petals` `hypotrochoid`

**Sketch → solid**: `extrude(shape, h [, twist=deg])` · `revolve(shape, seg [, deg])`
· `lathe(shape, seg)` · `loft(a, b, …, h=H)` · `sweep(shape, path)` · `helix(shape, r, pitch, turns)`

**Booleans**: `a + b` union · `a - b` difference · `a * b` intersection

**Transforms/finish**: `.move .scale .rotatex/y/z .rotate("z",deg)`
`.arrayx/y/z(n,d) .polar(n) .mirror("x") .reflect("x") .fillet(r,seg) .chamfer(d)`

## Gotchas the kernel will catch (and two it won't)

- `bezier` takes `2 + 6·N` coordinates (anchor + N cubic segments) — wrong
  arity is a clear error. Lathe silhouettes should start and end **on the
  axis** (`r = 0`) or the revolve won't close watertight.
- `fillet`/`chamfer` only touch **sharp convex** edges of prismatic solids;
  spheres/cylinder walls pass through unchanged, and an oversized radius is
  an error, never broken geometry.
- `tube` errors if `r >= R`; cut-through holes should overshoot the wall
  (`h + 2`) so the boolean cuts clean faces.
- Silent gotcha #1: forgetting rule 2 — the prop *renders* but floats or
  sinks. `vimtool --svg` front view shows it instantly.
- Silent gotcha #2: `cube 3 .move 0,0,0` attaches `.move` to the call — but
  command-style args are atoms, so put arithmetic in parens.

## Materials (thermal is the game)

`VimProp` applies ONE thermal preset to the whole prop:
`thermal: Metal | MetalRoof | Wood | Concrete | BuildingWall | Glass` (default Metal).

Builtin templates go further: each **part name** maps to its own material in
`crates/da-param/src/generate.rs` (e.g. `silo.vim`'s `dome` part → metal roof
profile, `streetlight.vim`'s `head` → emissive lamp glass). If you add a part
to a builtin template, add its mapping there too — unmapped parts fall back
to the feature's default.

## Parameterized builtins

Builtin templates expose dimensions as *constant* `let` lines:

```vim
let radius = 2.2      " bound by Rust: vim_with_params(src, [("radius", r)])
let height = 9.0
```

`da_param::vim_with_params` rewrites only `let name = <constant>` lines —
derived lines (`let hz = height / 2`) recompute automatically. Binding a name
that isn't a constant `let` is a hard error, so templates fail loudly, not
silently.

## Troubleshooting

- **"no `model ...` statement"** — every script must end with a `model` line.
- **`VimMissing` in an editor panel** — the zone was parsed without resolving
  prop sources; the game loader does this automatically (`resolve_vim_sources`).
- **Slow zone load** — check `triangle_count()` ambitions: drop `seg`, avoid
  boolean-heavy helixes, and remember each distinct script compiles once per
  expansion (repeats are cheap; the renderer instances by content hash).
- **Determinism** — scripts are pure functions; never depends on anything but
  the text. If two expansions differ, the bug is not in your `.vim`.
