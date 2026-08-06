# vali / loki `.vim` CSG modeling language — a primer for DeadAir

> A hand-off reference for a Claude Code session working in **DeadAir / darkair**.
> It teaches vali's code-first CSG editor ("osgedit" in loki/vali parlance): the
> Nim-flavored `.vim` modeling language and **every** primitive, sketch, and
> operation the DSL exposes. A final section maps these to darkair's
> `*.zone.ron` parametric world and `tools/osgedit` SDF engine.
>
> **Every builtin name and signature below is quoted from vali source**
> (`src/dsl/eval.rs`, `src/csg/*.rs`) with `file:line` citations. Nothing here is
> invented — if it is not in `eval.rs`, it is not a builtin. Source of truth
> lives in the sibling repo at `/home/james.clements/projects/vali`.

---

## 1. What a `.vim` file is

A `.vim` file is a short **code-first CAD script** in a small Nim-flavored
modeling language. The vali app's **Model** sub-tool (its "osgedit"-style
code-first CSG editor) evaluates the script on every edit and renders the
resulting solid in the shared 3D viewport. Scripts are saved with a `.vim`
extension (the "Save .vim" button writes `model.vim`), and load via the
`VALI_MODEL_VIM=<path>` env var.

Pipeline (`src/dsl/mod.rs:43` `compile()`):

```
source text  →  lex  →  parse  →  evaluate  →  Solid  (+ build-step outline)
              lexer.rs  parser.rs  eval.rs      csg/ kernel
```

The evaluator produces a `Solid` — a soup of convex, consistently-wound
polygons — on the **pure-Rust BSP boolean kernel** in `src/csg/` (`bsp.rs`).
That kernel is the reliability spine: every boolean is `solid ∩/∪/∖ solid`
resolved by a BSP tree, and the whole thing is verified against analytic volumes
in the test suite. The design principle carried through the DSL
(`src/dsl/mod.rs:1`) is **reliability over features**: every construct either
works or returns a clear, actionable error string. There is no "silently broken
mesh" outcome.

Conventions (match darkair's own):

- **Units are metres.** Missile geometry, part dimensions — all SI.
- **Z-up.** The Edit viewport is Z-up (ENU / Blender convention). Round
  primitives run along **+Z**; every primitive is **centered at the origin**
  (`src/csg/primitives.rs:4`). You position parts with `.move` / `.rotate`.
  (Note the axis difference vs. darkair's osgedit SDF engine, which is **Y-up** —
  see §10.)
- **Source may use Unicode** freely in comments; keep it clean.

---

## 2. Language basics

### Statements

A program is a sequence of newline-separated statements. There are exactly two
(`src/dsl/parser.rs:86`, `parse_stmt`):

```vim
let name = expr        " a named binding (a dimension, a sketch, or a solid)
model expr             " THE solid to render — required, or you get an error
```

`run_program` (`src/dsl/eval.rs:35`) evaluates `let` bindings in order into an
environment, then renders the `model` expression. **A `model` statement is
mandatory** — without one you get `"no `model ...` statement — nothing to
render"`. If multiple `model` lines exist the last one wins, but the build-step
outline is taken from the first (`src/dsl/mod.rs:49`). `model` must evaluate to a
**solid**, not a number or sketch (`eval.rs:44`).

### Values / types

Four value kinds (`src/dsl/eval.rs:13`): **Num** (`f64`), **Str**, **Sketch**
(2D closed loop), **Solid** (3D). Builtins are typed: passing a sketch where a
number is expected yields e.g. `` argument `h` must be a number, got sketch ``.

### `let` — parametric dimensions & derived relationships

`let` binds a name to any value. Bind **numbers** as parameters, then derive new
dimensions with arithmetic — this is how you make a part parametric:

```vim
let bore = 8            " a driving dimension
let wall = 12
let gap  = wall - bore  " derived: 4  (pure arithmetic, folded to a number)
let pipe = cylinder(wall, 40) - cylinder(bore, 40)   " a derived SOLID
```

Arithmetic operators on numbers: `+ - * /` (`eval.rs:92`). Precedence
(`parser.rs:126`): `+ -` bind at 10, `* /` at 20; use parentheses to group.
Unary minus is supported (`-bore`). The same operators are **overloaded on
solids** — see §6.

> The editor's Properties/parameters panel can fold pure-arithmetic `let`
> relationships (`eval_num_expr`, `numeric_refs` in `mod.rs:62`) so tweaking
> `bore` updates everything derived from it. A `let` whose value is a call or
> method chain is a *part* binding, not a numeric relationship.

### Comments

Two comment styles (`src/dsl/lexer.rs`):

- `#` to end of line — anywhere.
- **VimL-style `"`** at the **start of a statement** runs to end of line
  (`lexer.rs:38`). A `"` **inside** an expression (e.g. `rotate("z", 90)`) is a
  **string literal**, not a comment (`lexer.rs:77`).

### Calls: parens optional (Nim command style)

Two equivalent call syntaxes (`parser.rs:206`):

```vim
cylinder(r = 8, h = 42, seg = 64)   " paren call, named args
cylinder 8, 42, 64                  " command style (parens omitted)
```

Arguments are **positional or named** (`name = value`), resolved by
name-or-position with defaults (`eval.rs:127`, `EvalArgs::num`). Command-style
arguments are *atoms* (numbers/vars/parens/calls), not full operator
expressions, so `cylinder 8, 40 + cube 2` parses as `(cylinder 8,40) + (cube 2)`;
put arithmetic inside parens. Command style deliberately excludes a leading `-`
so `a - b` stays subtraction (`parser.rs:191`).

### Method chains

Transforms and bevels are **methods** on a solid, chained with `.`
(`parser.rs:154`):

```vim
cylinder(2, 8).move(16, 0, 0).rotatez(45).fillet(0.5, 8)
```

`cube 3 .move 0,0,0` attaches `.move` to the *call*, not to the number `3`
(`parser.rs:197`).

### Minimal working example

```vim
" a drilled block — the smallest useful .vim
let block = box(20, 20, 10)
let hole  = cylinder(r = 4, h = 12)
model block - hole
```

---

## 3. Primitives

All solids, all **centered at the origin**, round shapes along **+Z**
(`primitives.rs:4`). Signatures and defaults are exactly as decoded in
`call_builtin` (`src/dsl/eval.rs:156`). `seg` = angular tessellation segments
(minimum 3, `eval.rs:141`).

| Call | Signature (defaults) | Makes | eval.rs |
|---|---|---|---|
| `cube(s)` | `s` | Cube `s×s×s` (one arg drives all three) | `:158` |
| `box(w, d, h)` | `w, d, h` | Axis-aligned block `w×d×h` (x,y,z) | `:163` |
| `cylinder(r, h, seg)` | `seg=48` | Right circular cylinder, axis +Z | `:168` |
| `cone(r, h, seg)` | `seg=48` | Cone: base radius `r` at −h/2 → apex at +h/2 | `:173` |
| `frustum(r1, r2, h, seg)` | `seg=48` | Truncated cone: `r1` at −h/2, `r2` at +h/2 | `:178` |
| `sphere(r, seg)` | `seg=32` | UV sphere | `:184` |
| `torus(R, r, seg)` | `seg=40` | Ring, center-circle radius `R`, tube radius `r`, in XY plane. Minor segs = `seg·3/4` | `:185` |
| `wedge(w, d, h)` | `w, d, h` | Right triangular prism (XZ right-triangle, extruded along y) | `:191` |
| `pyramid(w, d, h)` | `w, d, h` | Rectangular pyramid: `w×d` base at −h/2, apex at +h/2 | `:196` |
| `tube(R, r, h, seg)` | `seg=48` | Hollow cylinder (pipe) = `cylinder(R,h) − cylinder(r,h)`. **Errors if `r ≥ R`** | `:218` |

`box`/`cube` map to `Solid::cube(w,d,h)` (`csg/mod.rs:47`); `cylinder`/`cone`
defer to `frustum` (`primitives.rs:133`).

```vim
model box(30, 20, 10)
model cylinder(r = 6, h = 40, seg = 96)
model frustum(r1 = 10, r2 = 4, h = 20)   " nozzle-ish taper
model tube(R = 12, r = 8, h = 40)        " pipe wall
model torus(R = 20, r = 5)
```

---

## 4. 2D sketches

A `Sketch` (`src/csg/sketch.rs:11`) is a **closed 2D loop** of `(x, y)` points
(no repeated closing point). Sketches are inert until lifted into 3D by
`extrude` / `revolve` / `lathe` / `loft` / `sweep` (§5). A sketch has one method:
`.move(dx, dy)` / `.translate2d(dx, dy)` (`eval.rs:516`) — e.g. to offset a
revolve profile off-axis.

### Basic sketches

| Call | Signature (defaults) | Shape | eval.rs / sketch.rs |
|---|---|---|---|
| `circle(r, seg)` | `seg=48` | Circle | `eval:203` |
| `rect(w, h)` | `w, h` | Rectangle centered at origin | `eval:204` |
| `ngon(n, r)` | `n, r` | Regular convex n-gon, circumradius `r`, first vertex at +Y | `eval:205` / `sketch:69` |
| `roundrect(w, h, r, seg)` | `seg=8` | Rounded rectangle, corner radius `r`, `seg` steps/quarter-arc. Area `w·h − (4−π)r²` | `eval:206` / `sketch:36` |
| `slot(l, r, seg)` | `seg=24` | Slot / stadium: two semicircles radius `r`, centers `l` apart on X. Area `πr² + 2rl` | `eval:212` / `sketch:53` |
| `polygon(x0,y0, x1,y1, …)` | ≥3 (x,y) pairs | Arbitrary closed loop from an even, ≥6-number list | `eval:271` / `sketch:17` |
| `svgpath("d", steps)` | `steps=24` | Import an SVG path d-string. **M/L/H/V/C/Z only**, absolute or relative; cubics sampled to `steps`. Arcs/quadratics error | `eval:283` / `sketch:117` |

> `roundrect` and `slot` are **recent additions** (commit `5dc398a`), verified in
> `mod.rs:275` (`roundrect_and_slot_areas`).

### `bezier` — the lathe/turning silhouette

```
bezier(r0,z0,  c1r,c1z, c2r,c2z, r1,z1,  …,  steps = 16)
```

A start anchor followed by N cubic segments as `(control1, control2, end)`
triples — i.e. **`2 + 6·N` coordinates** (else a clear arity error, `eval.rs:310`
/ tested at `mod.rs:245`). Read as **`(r, z)`**: feed to `lathe`/`revolve` to turn
a body of revolution. The silhouette should begin and end on the axis (`r = 0`)
so the revolve closes into a watertight solid (`sketch.rs:292`). `steps=` sets
per-segment sampling.

```vim
" an ogive-ish missile silhouette (r,z), nose and tail on axis
let sil = bezier(0,10, 0.3,10, 1.6,9, 1.6,8,  1.6,8, 1.6,3, 1.6,2,  1.6,2, 1.0,1, 0.2,0)
model lathe(sil, 48)
```

### Math sections (polar / parametric curves)

Ordinary sketches produced by closed-form curves — feed any of them to
`extrude` / `revolve` / booleans.

| Call | Signature (defaults) | Curve | eval.rs / sketch.rs |
|---|---|---|---|
| `limacon(c, b, seg)` | `seg=128` | Limaçon `r = b·(c + cosθ)`. `c>1` dimpled convex, `c=1` cardioid cusp | `eval:232` / `sketch:84` |
| `cardioid(b, seg)` | `seg=128` | Cardioid = `limacon(1, b)` | `eval:237` |
| `rose(b, flutes, amp, seg)` | `flutes=12, amp=0.4, seg=160` | Fluted near-circular ring; petal tips reach radius `2b` (a *convex* fluted column, not the multi-petal curve) | `eval:238` / `sketch:100` |
| `star(n, r_out, r_in, seg)` | `seg` ignored | Regular n-pointed star, `2n` vertices; tip radius `r_out`, valley `r_in`. Area `n·r_out·r_in·sin(π/n)` | `eval:248` / `sketch:214` |
| `superellipse(a, b, n, seg)` | `n=2, seg=128` | Lamé curve `|x/a|ⁿ+|y/b|ⁿ=1`. `n=2` ellipse (area `πab`); `n>2` → rounded rect; `n<2` → astroid | `eval:254` / `sketch:230` |
| `petals(a, k, seg)` | `seg=256` | True rose `r = a·cos(kθ)`. Odd `k` → `k` petals, even `k` → `2k`. Self-touches at origin (fine as a sketch) | `eval:260` / `sketch:249` |
| `hypotrochoid(rr, r, d, seg)` | `seg=120` | Spirograph curve; auto-closes after `round(r)/gcd(round(rr),round(r))` turns | `eval:265` / `sketch:268` |

```vim
model extrude(star(5, 10, 4), 3)          " 5-point star prism
model extrude(superellipse(10, 6, 4), 2)  " squircle slab
model extrude(petals(10, 5), 2)           " 5-petal flower
model extrude(rose(10), 4)                 " 12-flute column ring
```

---

## 5. Turning sketches into solids

| Call | Signature (defaults) | Operation | eval.rs |
|---|---|---|---|
| `extrude(shape, h)` | — | Straight prism, height `h` centered on Z=0 (XY cross-section along Z) | `:328` |
| `extrude(shape, h, twist=deg)` | — | **Twisted** prism: each level rotates progressively up to `deg°` (spiral column). Auto-picks levels ≈ `deg/6` | `:328` |
| `revolve(shape, seg)` | `seg=64` | Spin an `(r, z)` profile 360° about Z | `:344` |
| `revolve(shape, seg, deg)` | — | **Partial** revolve `deg<360°` with **flat end caps** (pipe elbow / sector), stays watertight | `:344` |
| `lathe(shape, seg)` | `seg=64` | Like `revolve` but **auto-orients outward** (reverses profile if it wound inward). The missile-authoring name | `:352` |
| `loft(a, b, …, h=H)` | `h=10` | Skin a watertight hull through **2+** sketch sections stacked along Z. Errors if <2 sketches | `:356` |
| `sweep(shape, path)` | — | Ride the `shape` cross-section along `path` (a sketch read as `(x, z)`; a `bezier` is a natural spine) | `:383` |
| `helix(shape, r, pitch, turns, seg)` | `turns=1, seg=48` | Sweep a section along a helical spine of radius `r`, rising `pitch`/turn | `:399` |

**`extrude` / twisted extrude** — `twist=` triggers `Solid::extrude_twist`
(`sketch.rs:476`); a straight extrude otherwise. Twist is a **recent addition**
(commit `66c9e7c`), verified in `mod.rs:287`. `extrude` reads the sketch as an
**XY cross-section** swept along **Z**.

**`revolve` / partial revolve** — reads the sketch as `(radius, height)` and
spins about **Z** (`sketch.rs:897`). A `deg < 360` argument (positional 3rd, or
`deg=`) routes to `Solid::revolve_arc` (`sketch.rs:943`), which adds two flat end
caps so a sector/elbow is watertight. **Recent addition** (commit `d9ed076`),
verified in `mod.rs:301`.

**`loft`** — resamples every section to a common vertex count by arc length,
rolls them apex-to-apex, and **DTW twist-aligns** each section to its predecessor
(dynamic-time-warp correspondence, `sketch.rs:643` `dtw_map` / `:689`
`dtw_align`) before skinning, then caps the ends. `loft(circle(3), circle(3),
h=10)` is a cylinder; `loft(circle(2), circle(4), h=9)` a frustum; morph a
limaçon into a circle for an organic transition. Height is `h=` (named) or a
trailing positional number (default 10).

**`sweep` / `helix`** — use a **rotation-minimizing frame** (Wang et al. 2008
double-reflection, `sketch.rs:821` `sweep_path`) so the section rides the spine
with minimal twist, then caps the ends watertight. `sweep`'s `path` sketch points
are read as `(x, z)` in the XZ plane; `helix` builds the spine internally. A
dense helix booleaned into another solid is valid but heavy.

```vim
model extrude(ngon(6, 10), 8, twist = 180)                 " twisted hex column
model revolve(polygon(4,-1, 6,-1, 6,1, 4,1), 200, 180)     " half a rectangular torus
model loft(circle(2, 128), circle(3, 128), circle(2, 128), h = 10)   " barrel
model sweep(circle(2, 128), bezier(0,-5, 0,-5, 0,5, 0,5))  " straight tube
model helix(circle(1), 6, 8, 2)                            " 2-turn spring
```

---

## 6. Booleans

Booleans are the arithmetic operators **overloaded on solids** (`eval.rs:87`,
`eval_bin`). Same precedence as numeric arithmetic (`* /` bind tighter than
`+ -`).

| Operator | Op | Method behind it |
|---|---|---|
| `a + b` | **Union** | `Solid::union` (`csg/mod.rs:126`) |
| `a - b` | **Difference** (cut `b` out of `a`) | `Solid::difference` (`:130`) |
| `a * b` | **Intersection** | `Solid::intersection` (`:134`) |

There is **no** `union(...)` / `subtract(...)` function form in the DSL — booleans
are only the infix operators (contrast darkair's op-tree node names in §10). The
build-step outline renders them as `∪ union` / `∖ difference (cut)` / `∩
intersection` (`mod.rs:123`).

Each boolean concatenates the two operands' **part tables** (`mod.rs:141`): every
polygon keeps a `part` tag, so a drilled bore's wall stays a distinguishable part
from the block it was cut from — this is what vali's thermal viewer colors
per-part. Booleans stay watertight and outward-wound by construction.

```vim
let body = cylinder(12, 40)
let bore = cylinder(8, 42)          " slightly longer so it cuts clean through
let slot = box(4, 30, 4)
model (body - bore) - slot          " grouped with parens
```

---

## 7. Transforms

Methods on a **solid** (`eval.rs:422`, `call_method`). Angles are **degrees**;
translations/scales in **metres**. Rotation is Rodrigues about a unit axis
(`csg/mod.rs:195`).

| Method | Signature (defaults) | Effect | eval.rs |
|---|---|---|---|
| `.move(x, y, z)` / `.translate(x, y, z)` | each `0` | Translate | `:425` |
| `.scale(x, y, z)` | `y=x, z=x` | Scale (one arg = uniform) | `:430` |
| `.rotatex(deg)` / `.rotatey(deg)` / `.rotatez(deg)` | `deg` | Rotate about that axis | `:437` |
| `.rotate("axis", deg)` | axis ∈ `"x"/"y"/"z"` | Rotate about a named axis | `:440` |
| `.arrayx(n, dx)` / `.arrayy(n, dy)` / `.arrayz(n, dz)` | — | Linear array: union `n` copies each offset by the step | `:456` |
| `.polar(n, "axis")` | axis `"z"` | Circular array: `n` copies over a full turn about the axis | `:474` |
| `.mirror("axis")` | axis string | **Symmetrize**: union self with its reflection (Blender Mirror modifier) | `:489` |
| `.reflect("axis")` | axis string | Bare reflection (re-winds to stay watertight) | `:489` |

Sketches only support `.move` / `.translate2d` (`eval.rs:516`); the pattern /
rotate / bevel methods are solid-only. `.scale` adjusts normals by the
inverse-transpose so shading stays correct (`csg/mod.rs:186`).

```vim
let bolt = cylinder(2, 8).move(16, 0, 0)
model bolt.polar(6)                    " 6 bolts around Z
model box(2,2,2).move(6,0,0).mirror("x")   " symmetric pair
model cube(2).arrayx(3, 10)            " 3 cubes in a row
```

---

## 8. Bevels — fillet & chamfer

Methods on a solid (`eval.rs:507`), implemented by cutting half-space planes and
letting the BSP core resolve them (`src/csg/ops.rs`), so results stay watertight.

| Method | Signature (defaults) | Effect | eval.rs |
|---|---|---|---|
| `.chamfer(d)` | `d` | Flat cut of distance `d` off sharp convex edges | `:512` |
| `.fillet(r, seg)` | `seg=8` | Rounded edge of radius `r`, `seg` flat facets per arc | `:507` |

**Conservative limits (deliberate, `ops.rs:26`):** bevels apply **only to sharp
convex edges** of prismatic solids (cube, box, wedge, pyramid, extrusions). They
**skip**:

- **Concave** edges — beveling them would *add* material, which a cut-based
  approach cannot do.
- **Near-flat / smoothly-curved** edges (sphere facets, cylinder walls) — below
  the ~1.1° dihedral threshold (`MIN_ANGLE`, `ops.rs:44`); a sphere passes
  through unchanged.
- **Non-manifold** edges (not shared by exactly two faces).

If the radius/distance would consume a whole edge, the op **returns an error**
rather than emit degenerate geometry (`ops.rs:99`). Fillet corners are faceted,
not perfectly spherical. Net effect: a bevel either produces valid watertight
geometry or a clear error — never a broken mesh.

```vim
model box(20, 12, 6).fillet(1.5, 8)   " rounded box edges
model cube(10).chamfer(1.2)           " chamfered cube
```

---

## 9. Complete, copy-pasteable `.vim` examples

### 9a. The canonical example (`EXAMPLE`, `src/dsl/mod.rs:29`)

A flanged, bolted pipe — reads top-to-bottom as build steps.

```vim
# A flanged, bolted pipe — reads top-to-bottom as build steps.
let bore   = cylinder(r = 8,  h = 42)
let wall   = cylinder(r = 12, h = 40)
let pipe   = wall - bore              # difference: drill the bore

let flange = cylinder(r = 22, h = 6).move(0, 0, -17)
let bolt   = cylinder(r = 2,  h = 8).move(16, 0, -17)

# union the flange on, then cut one bolt hole
model pipe + flange - bolt
```

### 9b. Parametric part — `let` dimensions + boolean + bevel

A mounting bracket driven entirely by named dimensions; change `plate_w` or
`hole_r` and the whole part follows.

```vim
" ---- driving dimensions (metres) ----
let plate_w = 40
let plate_d = 24
let plate_t = 4
let hole_r  = 3
let edge    = plate_d / 2 - 6      " derived: hole offset from centre

" ---- build ----
let plate = box(plate_w, plate_d, plate_t).fillet(2, 8)
let hole  = cylinder(r = hole_r, h = plate_t + 2)   " overshoot so it cuts clean
let holeL = hole.move(-plate_w / 2 + 6,  edge, 0)
let holeR = hole.move( plate_w / 2 - 6, -edge, 0)

model plate - holeL - holeR
```

### 9c. Math-section sketch + twisted extrude + partial revolve

Combines a fluted twisted column with a capped 90° pipe elbow.

```vim
" a twisted hex column (progressive 120-degree twist over its height)
let column = extrude(ngon(6, 8), 30, twist = 120)

" a 90-degree pipe elbow: a 2x2 square tube profile revolved a quarter turn,
" with flat end caps (partial revolve keeps it watertight)
let elbow = revolve(polygon(9,-1, 11,-1, 11,1, 9,1), 96, 90).move(0, 0, 20)

model column + elbow
```

### 9d. Lathe a body of revolution from a bezier silhouette

```vim
" ogive nose -> straight body -> boat-tail, nose & tail on the axis (r=0)
let sil = bezier(0,20,  0.4,20, 3.0,19, 3.2,17,   3.2,17, 3.2,6, 3.2,4,   3.2,4, 2.0,1, 0.3,0)
model lathe(sil, 64)
```

> Every function used in 9a–9d exists in `eval.rs`: `cylinder`, `box`, `ngon`,
> `polygon`, `bezier`, `extrude(…twist=)`, `revolve(…,deg)`, `lathe`, `fillet`,
> `.move`, and the `+`/`-` boolean operators. Shipped `.vim` samples live in
> vali's `assets/lathes/` (all volume-tested in `mod.rs:219`).

---

## 10. Mapping to darkair osgedit / `.zone.ron`

darkair has two related "CSG" surfaces. vali's `.vim` DSL maps cleanly onto both,
but note two structural differences up front:

- **Kernel:** vali is a **BSP polygon / B-rep** kernel (exact watertight
  booleans, analytic volumes). darkair's `tools/osgedit` is an **SDF raymarcher**
  (`tools/osgedit/src/sdf.rs`) — booleans are min/max on signed distances, which
  also unlocks **smooth** blends vali does not have.
- **Up axis:** vali/`.vim` is **Z-up** (round prims along +Z); darkair osgedit
  is **Y-up, inside < 0** (`sdf.rs:2`), and `.zone.ron` points are `(x, y, z)`
  with **y up, ground at y:0** (`FORMAT.md:8`). Swap Y↔Z and negate handedness
  when porting a profile.

### 10a. vali `.vim` builtin → darkair osgedit SDF node

darkair's op-tree is `Prim` + `Node` enums in `tools/osgedit/src/sdf.rs` (models
authored as JSON in `tools/osgedit/models/*.json`, e.g. `bracket.json`). Rough
correspondence:

| vali `.vim` | darkair osgedit (`sdf.rs`) | Notes |
|---|---|---|
| `sphere(r)` | `Prim::Sphere { r }` (`:218`) | direct |
| `box(w,d,h)` / `cube` | `Prim::Box { half, round }` (`:221`) | osgedit box takes **half-extents** + a free `round` (rounded box); vali has no rounded-box primitive (use `.fillet`) |
| `cylinder(r,h)` | `Prim::Cylinder { r, h }` (`:226`) | axis differs (Z vs Y) |
| `frustum(r1,r2,h)` / `cone` | `Prim::Cone { r1, r2, h }` (`:231`) | vali `cone` = `frustum(r,0,h)` |
| `torus(R,r)` | `Prim::Torus { major, minor }` (`:235`) | direct |
| (no direct prim) | `Prim::Capsule { a, b, r }` (`:239`), `Prim::Plane { n, offset }` (`:244`) | vali has no capsule/half-space primitive; `bracket.json` uses `plane` to slice via `intersect` |
| `a + b` | `Node::Union` (`:335`) | |
| `a - b` | `Node::Subtract` (`:336`) | |
| `a * b` | `Node::Intersect` (`:337`) | |
| *(none — BSP is exact)* | `Node::SmoothUnion/Subtract/Intersect { k }` (`:337`) | SDF-only blends via `smin`/`smax` (`:349`) |
| `.move(x,y,z)` | `Node::Translate { v }` (`:341`) | |
| `.rotate("z",deg)` | `Node::Rotate { q }` (`:342`) | osgedit stores a **quaternion**; vali takes axis+degrees |
| `.scale(x,y,z)` | `Node::Scale { s }` (`:343`) | |
| `.polar(n)` | `Node::RadialRepeat { n }` (`:344`) | radial repeat about the up axis |
| `.arrayx/y/z(n,d)` | `Node::GridRepeat { cell, count }` (`:345`) | grid handles all axes at once |
| `.mirror("x")` | `Node::Mirror { axis }` (`:346`) | |
| *(none — use `tube`)* | `Node::Shell { thickness }` (`:340`) | SDF hollowing; vali makes shells via boolean difference |

**Takeaways for an osgedit implementer:** vali's `extrude`, `revolve`/`lathe`,
`loft`, `sweep`, `helix`, `bezier`, and the math sections (`limacon`, `rose`,
`star`, `superellipse`, `petals`, `hypotrochoid`) have **no SDF equivalent** in
the current `sdf.rs` — they are B-rep sketch-to-solid operations. Adding them to
osgedit would mean either (a) a new lofted-SDF path, or (b) porting vali's
polygon kernel. Conversely, osgedit's `SmoothUnion`/`Shell`/`Capsule` have no
`.vim` equivalent. If you want vali-authored props inside darkair's SDF world,
the clean bridge is meshing vali's `Solid` (it exports STL/OBJ) rather than
re-deriving the distance field.

### 10b. `.zone.ron` — the real "OpenSCAD spirit" analog

The closest philosophical match to `.vim` is **not** the low-level osgedit
op-tree but darkair's **`assets/zones/*.zone.ron`** parametric zone sources
(`crates/da-param`). Both are **code-first, deterministic, source-is-truth**:

| Aspect | vali `.vim` | darkair `.zone.ron` |
|---|---|---|
| Ground truth | the script text | the RON text (`CLAUDE.md`: "Never edit generated output; edit the source") |
| Determinism | pure function of source | same source **+ seed** → byte-identical scene graph (`FORMAT.md:3`) |
| Parametrics | `let` dimensions + derived arithmetic | typed feature generators with fields, e.g. `Barn(pos, width_m, bays, roof: Metal)` |
| Output | a `Solid` (BSP polygons) | a `da-graph` OSG-style scene graph |
| Editor | vali Model panel re-evaluates on edit | `apps/da-edit` source panel re-expands on edit; graph edits are session-only previews |

Where `.vim` composes **primitives + booleans** into one part, a `.zone.ron`
composes **feature generators** (`Barn` `FenceLine` `TreeRow` `CropRows` `Creek`
`Silo` `RadioMast` …, `FORMAT.md:55`) into a world, with thermal profiles
attached automatically. An implementer wanting richer per-feature geometry
(a parametric barn silhouette, a fluted silo, a lofted grain bin) would author it
in the **vali `.vim`** style and expand it inside a `da-param` feature generator —
that keeps the "same source + seed → identical scene graph" contract while
borrowing vali's sketch-to-solid vocabulary. Determinism is load-bearing in
darkair (`CLAUDE.md`: all randomness flows from `da_core::Rng` seeds), so any
ported operation must stay a pure function of its inputs — which vali's DSL
already is.

---

## Appendix — one-line cheat sheet

```
" statements
let name = expr                 model expr            # (# or leading " = comment)

" primitives (centered, +Z, metres)
cube(s)  box(w,d,h)  cylinder(r,h,seg=48)  cone(r,h,seg=48)  frustum(r1,r2,h,seg=48)
sphere(r,seg=32)  torus(R,r,seg=40)  wedge(w,d,h)  pyramid(w,d,h)  tube(R,r,h,seg=48)

" sketches
circle(r,seg=48)  rect(w,h)  ngon(n,r)  roundrect(w,h,r,seg=8)  slot(l,r,seg=24)
polygon(x0,y0,x1,y1,...)  svgpath("d",steps=24)  bezier(r0,z0,...,steps=16)   " (r,z)
limacon(c,b,seg=128)  cardioid(b,seg=128)  rose(b,flutes=12,amp=0.4,seg=160)
star(n,r_out,r_in)  superellipse(a,b,n=2,seg=128)  petals(a,k,seg=256)  hypotrochoid(rr,r,d,seg=120)

" sketch -> solid
extrude(shape,h[,twist=deg])   revolve(shape,seg=64[,deg])   lathe(shape,seg=64)
loft(a,b,...,h=10)   sweep(shape,path)   helix(shape,r,pitch,turns=1,seg=48)

" booleans / transforms / bevels
a + b  (union)   a - b  (difference)   a * b  (intersection)
.move(x,y,z) .scale(x[,y,z]) .rotatex/y/z(deg) .rotate("z",deg)
.arrayx/y/z(n,d) .polar(n[,"z"]) .mirror("x") .reflect("x") .fillet(r,seg=8) .chamfer(d)
```
