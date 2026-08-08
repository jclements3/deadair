//! Evaluate the AST into a `Solid`.
//!
//! Values are numbers, strings, 2D sketches, or solids. Builtins accept
//! positional or named arguments with sensible defaults. Anything unsupported
//! (fillet, chamfer, unknown names, type mismatches) fails with a clear,
//! actionable message rather than producing a broken result.

use super::parser::{Arg, Expr, Stmt};
use super::volumetric::{check_positive, operator_error, Vol};
use crate::csg::sketch::Sketch;
use crate::csg::Solid;
use std::collections::HashMap;

#[derive(Clone)]
pub enum Value {
    Num(f64),
    Str(String),
    Sketch(Sketch),
    Solid(Solid),
    /// One or more volumetric fields, optionally riding along with the solid
    /// they were `+`-ed onto. The BSP kernel has no density representation,
    /// so this backend keeps the solid and reports the names it dropped --
    /// a volumetric must never reach the kernel.
    Vol { solid: Option<Solid>, names: Vec<String> },
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Sketch(_) => "sketch",
            Value::Solid(_) => "solid",
            Value::Vol { .. } => "volumetric",
        }
    }

    /// The volumetric names this value carries (empty for a pure solid).
    fn vol_names(&self) -> &[String] {
        match self {
            Value::Vol { names, .. } => names,
            _ => &[],
        }
    }
}

type Env = HashMap<String, Value>;

/// Default part names the kernel's constructors assign (`Solid::single`
/// callers plus the bevel cutter). A `let` binding renames these to the bound
/// name — see [`run_program`] — so authored `.vim` scripts carry meaningful
/// part tags ("barrel", "dome", ...) instead of primitive names. Parts an
/// earlier `let` already named keep their name through booleans/patterns.
const DEFAULT_PART_NAMES: &[&str] = &[
    "cube", "cylinder", "cone", "frustum", "sphere", "torus", "wedge", "pyramid", "extrude",
    "revolve", "lathe", "loft", "sweep", "helix", "bevel",
];

/// Run a program: evaluate `let` bindings in order and return the `model` solid.
///
/// darkair adaptation (additive vs. vali): a `let` that binds a **solid**
/// stamps the bound name onto every part still carrying a kernel default name
/// ([`DEFAULT_PART_NAMES`]), so part tags flow from the script's `let`
/// vocabulary. Deterministic: a pure function of the source like everything
/// else here.
pub fn run_program(stmts: &[Stmt]) -> Result<(Solid, Vec<String>), String> {
    let mut env: Env = HashMap::new();
    let mut result: Option<(Solid, Vec<String>)> = None;
    for stmt in stmts {
        match stmt {
            Stmt::Let(name, expr) => {
                let mut v = eval(expr, &env)?;
                let named = match &mut v {
                    Value::Solid(s) => Some(s),
                    Value::Vol { solid: Some(s), .. } => Some(s),
                    _ => None,
                };
                if let Some(s) = named {
                    for part in &mut s.parts {
                        if DEFAULT_PART_NAMES.contains(&part.name.as_str()) {
                            part.name = name.clone();
                        }
                    }
                }
                env.insert(name.clone(), v);
            }
            Stmt::Model(expr) => match eval(expr, &env)? {
                Value::Solid(s) => result = Some((s, Vec::new())),
                // The strip: keep the solid, hand the names back so the
                // caller can warn. Exit status stays success.
                Value::Vol { solid: Some(s), names } => result = Some((s, names)),
                Value::Vol { solid: None, names } => {
                    return Err(format!(
                        "`model` has no solid geometry -- {} {} volumetric{}, which \
                         the polygon backend cannot mesh. Add the surfaces the field \
                         sits on (terrain, a fire ring, a building) to the model.",
                        names.join(", "),
                        if names.len() == 1 { "is a" } else { "are" },
                        if names.len() == 1 { "" } else { "s" }
                    ))
                }
                other => {
                    return Err(format!(
                        "`model` expects a solid, but got a {}",
                        other.kind()
                    ))
                }
            },
        }
    }
    result.ok_or_else(|| "no `model ...` statement — nothing to render".into())
}

fn eval(expr: &Expr, env: &Env) -> Result<Value, String> {
    match expr {
        Expr::Num(n) => Ok(Value::Num(*n)),
        Expr::Str(s) => Ok(Value::Str(s.clone())),
        Expr::Var(name) => env
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown name `{name}`")),
        Expr::Neg(e) => match eval(e, env)? {
            Value::Num(n) => Ok(Value::Num(-n)),
            other => Err(format!("cannot negate a {}", other.kind())),
        },
        Expr::Bin(op, a, b) => {
            let av = eval(a, env)?;
            let bv = eval(b, env)?;
            eval_bin(*op, av, bv)
        }
        Expr::Call(name, args) => {
            let ea = EvalArgs::new(args, env)?;
            call_builtin(name, &ea)
        }
        Expr::Method(recv, method, args) => {
            let rv = eval(recv, env)?;
            let ea = EvalArgs::new(args, env)?;
            call_method(rv, method, &ea)
        }
    }
}

fn eval_bin(op: char, a: Value, b: Value) -> Result<Value, String> {
    // Volumetrics are not CSG operands: `+` adds them to the scene beside the
    // solid, everything else is an error naming the operator and the field.
    if !a.vol_names().is_empty() || !b.vol_names().is_empty() {
        let mut names: Vec<String> = a.vol_names().to_vec();
        names.extend_from_slice(b.vol_names());
        if op != '+' {
            return Err(operator_error(op, &names));
        }
        let mut solid: Option<Solid> = None;
        for v in [a, b] {
            let s = match v {
                Value::Solid(s) => Some(s),
                Value::Vol { solid, .. } => solid,
                other => {
                    return Err(format!(
                        "operator `+` doesn't apply to a volumetric and a {}",
                        other.kind()
                    ))
                }
            };
            solid = match (solid, s) {
                (Some(x), Some(y)) => Some(x.union(y)),
                (Some(x), None) => Some(x),
                (None, y) => y,
            };
        }
        return Ok(Value::Vol { solid, names });
    }
    match (op, a, b) {
        ('+', Value::Solid(x), Value::Solid(y)) => Ok(Value::Solid(x.union(y))),
        ('-', Value::Solid(x), Value::Solid(y)) => Ok(Value::Solid(x.difference(y))),
        ('*', Value::Solid(x), Value::Solid(y)) => Ok(Value::Solid(x.intersection(y))),
        ('+', Value::Num(x), Value::Num(y)) => Ok(Value::Num(x + y)),
        ('-', Value::Num(x), Value::Num(y)) => Ok(Value::Num(x - y)),
        ('*', Value::Num(x), Value::Num(y)) => Ok(Value::Num(x * y)),
        ('/', Value::Num(x), Value::Num(y)) => Ok(Value::Num(x / y)),
        (op, x, y) => Err(format!(
            "operator `{op}` doesn't apply to {} and {}",
            x.kind(),
            y.kind()
        )),
    }
}

/// Evaluated call arguments: positional values plus named values.
struct EvalArgs {
    pos: Vec<Value>,
    named: HashMap<String, Value>,
}

impl EvalArgs {
    fn new(args: &[Arg], env: &Env) -> Result<EvalArgs, String> {
        let mut pos = Vec::new();
        let mut named = HashMap::new();
        for a in args {
            let v = eval(&a.value, env)?;
            match &a.name {
                Some(n) => {
                    named.insert(n.clone(), v);
                }
                None => pos.push(v),
            }
        }
        Ok(EvalArgs { pos, named })
    }

    /// Resolve a numeric parameter by name-or-position, with an optional default.
    fn num(&self, idx: usize, name: &str, default: Option<f64>) -> Result<f64, String> {
        let v = self
            .named
            .get(name)
            .or_else(|| self.pos.get(idx))
            .cloned();
        match v {
            Some(Value::Num(n)) => Ok(n),
            Some(other) => Err(format!("argument `{name}` must be a number, got {}", other.kind())),
            None => default.ok_or_else(|| format!("missing argument `{name}`")),
        }
    }

    fn seg(&self, idx: usize, default: usize) -> Result<usize, String> {
        Ok(self.num(idx, "seg", Some(default as f64))?.max(3.0) as usize)
    }

    /// `seed` picks which noise field a volumetric samples. Default 0.
    fn seed_arg(&self, idx: usize) -> Result<u64, String> {
        Ok(self.num(idx, "seed", Some(0.0))?.max(0.0).min(u32::MAX as f64) as u64)
    }

    /// `phase` advances the animation; a frame sweep is a phase sweep, which
    /// is why it is an argument and not a clock read. Default 0.
    fn phase_arg(&self, idx: usize) -> Result<f64, String> {
        let p = self.num(idx, "phase", Some(0.0))?;
        if !p.is_finite() {
            return Err("argument `phase` must be a finite number".into());
        }
        Ok(p)
    }

    fn sketch(&self, idx: usize, name: &str) -> Result<Sketch, String> {
        match self.named.get(name).or_else(|| self.pos.get(idx)) {
            Some(Value::Sketch(s)) => Ok(s.clone()),
            Some(other) => Err(format!("argument `{name}` must be a sketch, got {}", other.kind())),
            None => Err(format!("missing sketch argument `{name}`")),
        }
    }
}

fn call_builtin(name: &str, ea: &EvalArgs) -> Result<Value, String> {
    let solid = |s: Solid| Ok(Value::Solid(s));
    let sketch = |s: Sketch| Ok(Value::Sketch(s));
    match name {
        // Primitives (all centered at origin, round shapes along +Z).
        "cube" => solid(Solid::cube(
            ea.num(0, "s", None)?,
            ea.num(0, "s", None)?,
            ea.num(0, "s", None)?,
        )),
        "box" => solid(Solid::cube(
            ea.num(0, "w", None)?,
            ea.num(1, "d", None)?,
            ea.num(2, "h", None)?,
        )),
        "cylinder" => solid(Solid::cylinder(
            ea.num(0, "r", None)?,
            ea.num(1, "h", None)?,
            ea.seg(2, 48)?,
        )),
        "cone" => solid(Solid::cone(
            ea.num(0, "r", None)?,
            ea.num(1, "h", None)?,
            ea.seg(2, 48)?,
        )),
        "frustum" => solid(Solid::frustum(
            ea.num(0, "r1", None)?,
            ea.num(1, "r2", None)?,
            ea.num(2, "h", None)?,
            ea.seg(3, 48)?,
        )),
        "sphere" => solid(Solid::sphere(ea.num(0, "r", None)?, ea.seg(1, 32)?)),
        "torus" => solid(Solid::torus(
            ea.num(0, "R", None)?,
            ea.num(1, "r", None)?,
            ea.seg(2, 40)?,
            (ea.seg(2, 40)? * 3 / 4).max(3),
        )),
        "wedge" => solid(Solid::wedge(
            ea.num(0, "w", None)?,
            ea.num(1, "d", None)?,
            ea.num(2, "h", None)?,
        )),
        "pyramid" => solid(Solid::pyramid(
            ea.num(0, "w", None)?,
            ea.num(1, "d", None)?,
            ea.num(2, "h", None)?,
        )),

        // Sketches.
        "circle" => sketch(Sketch::circle(ea.num(0, "r", None)?, ea.seg(1, 48)?)),
        "rect" => sketch(Sketch::rect(ea.num(0, "w", None)?, ea.num(1, "h", None)?)),
        "ngon" => sketch(Sketch::ngon(ea.num(0, "n", None)? as usize, ea.num(1, "r", None)?)),
        "roundrect" => sketch(Sketch::roundrect(
            ea.num(0, "w", None)?,
            ea.num(1, "h", None)?,
            ea.num(2, "r", None)?,
            ea.seg(3, 8)?,
        )),
        "slot" => sketch(Sketch::slot(
            ea.num(0, "l", None)?,
            ea.num(1, "r", None)?,
            ea.seg(2, 24)?,
        )),
        // Hollow cylinder (pipe): tube(R, r, h) = cylinder(R,h) - cylinder(r,h).
        "tube" => {
            let ro = ea.num(0, "R", None)?;
            let ri = ea.num(1, "r", None)?;
            let h = ea.num(2, "h", None)?;
            let seg = ea.seg(3, 48)?;
            if ri >= ro {
                return Err("tube: inner radius r must be smaller than outer R".into());
            }
            solid(Solid::cylinder(ro, h, seg).difference(Solid::cylinder(ri, h * 1.001, seg)))
        }

        // Scripted math sections (polar curves). Feed to extrude/revolve/lathe
        // like any other sketch. `limacon(c, b)`: r = b(c+cos t). `cardioid(b)` =
        // limacon at c=1. `rose(b, flutes=12, amp=0.4)`: fluted near-circle.
        "limacon" => sketch(Sketch::limacon(
            ea.num(0, "c", None)?,
            ea.num(1, "b", None)?,
            ea.seg(2, 128)?,
        )),
        "cardioid" => sketch(Sketch::limacon(1.0, ea.num(0, "b", None)?, ea.seg(1, 128)?)),
        "rose" => sketch(Sketch::rose(
            ea.num(0, "b", None)?,
            ea.num(1, "flutes", Some(12.0))?,
            ea.num(2, "amp", Some(0.4))?,
            ea.seg(3, 160)?,
        )),
        // More math families. `star(n, r_out, r_in)`: n-pointed star (area
        // n·r_out·r_in·sin(π/n)). `superellipse(a, b, n)`: Lamé curve, n=2 is an
        // ellipse. `petals(a, k)`: true rose r=a·cos(kθ) (odd k → k petals, even
        // k → 2k). `hypotrochoid(rr, r, d)`: spirograph curve.
        "star" => sketch(Sketch::star(
            ea.num(0, "n", None)? as usize,
            ea.num(1, "r_out", None)?,
            ea.num(2, "r_in", None)?,
            ea.seg(3, 0)?,
        )),
        "superellipse" => sketch(Sketch::superellipse(
            ea.num(0, "a", None)?,
            ea.num(1, "b", None)?,
            ea.num(2, "n", Some(2.0))?,
            ea.seg(3, 128)?,
        )),
        "petals" => sketch(Sketch::petals(
            ea.num(0, "a", None)?,
            ea.num(1, "k", None)?,
            ea.seg(2, 256)?,
        )),
        "hypotrochoid" => sketch(Sketch::hypotrochoid(
            ea.num(0, "rr", None)?,
            ea.num(1, "r", None)?,
            ea.num(2, "d", None)?,
            ea.seg(3, 120)?,
        )),
        "polygon" => {
            if ea.pos.len() < 6 || ea.pos.len() % 2 != 0 {
                return Err("polygon needs an even list of ≥3 (x, y) coords".into());
            }
            let mut pts = Vec::new();
            let mut it = ea.pos.iter();
            while let (Some(Value::Num(x)), Some(Value::Num(y))) = (it.next(), it.next()) {
                pts.push((*x, *y));
            }
            sketch(Sketch::polygon(pts))
        }
        // Import an SVG path d-string as a sketch: svgpath("M0,0 L10,0 ...", steps=24).
        "svgpath" => {
            let d = match ea.named.get("d").or_else(|| ea.pos.first()) {
                Some(Value::Str(s)) => s.clone(),
                _ => {
                    return Err(
                        "svgpath needs a path string, e.g. svgpath(\"M0,0 L10,0 L10,10 Z\")".into(),
                    )
                }
            };
            let steps = match ea.named.get("steps").or_else(|| ea.pos.get(1)) {
                Some(Value::Num(n)) => (*n as usize).max(2),
                _ => 24,
            };
            sketch(Sketch::from_svg_path(&d, steps)?)
        }
        // Cubic-bezier lathe silhouette: `bezier(r0,z0, c1r,c1z, c2r,c2z, r1,z1, ...)`
        // — a start anchor then (control1, control2, end) triples. `steps=` sets
        // the per-segment sampling. Read as (r, z); feed to `lathe`/`revolve`.
        "bezier" => {
            let nums: Vec<f64> = ea
                .pos
                .iter()
                .map(|v| match v {
                    Value::Num(n) => Ok(*n),
                    other => Err(format!("bezier coords must be numbers, got {}", other.kind())),
                })
                .collect::<Result<_, _>>()?;
            if nums.len() < 8 || nums.len() % 2 != 0 || (nums.len() / 2 - 1) % 3 != 0 {
                return Err(
                    "bezier needs a start anchor + N cubic segments: 2 + 6*N coords \
                     (r0,z0, then c1,c2,end triples)"
                        .into(),
                );
            }
            let nodes: Vec<(f64, f64)> = nums.chunks(2).map(|c| (c[0], c[1])).collect();
            let steps = match ea.named.get("steps") {
                Some(Value::Num(n)) => (*n as usize).max(2),
                _ => 16,
            };
            sketch(Sketch::bezier(&nodes, steps))
        }

        // Operations lifting sketches to solids.
        // extrude(shape, h[, twist=deg]) -- straight prism, or a twisted one if
        // `twist` degrees is given.
        "extrude" => {
            let shape = ea.sketch(0, "shape")?;
            let h = ea.num(1, "h", None)?;
            match ea.named.get("twist") {
                Some(Value::Num(d)) if d.abs() > 1e-6 => {
                    let steps = (d.abs() / 6.0).ceil().max(8.0) as usize;
                    solid(Solid::extrude_twist(&shape, h, *d, steps))
                }
                _ => solid(Solid::extrude(&shape, h)),
            }
        }
        // `revolve` spins an (r,z) profile; `lathe` is the missile-authoring name
        // that additionally auto-orients the hull outward (bezier silhouette can be
        // wound either way).
        // revolve(shape, seg[, deg]) -- full 360 by default; a `deg` < 360 makes
        // a capped partial revolve (pipe elbow / sector).
        "revolve" => {
            let shape = ea.sketch(0, "shape")?;
            let seg = ea.seg(1, 64)?;
            match ea.named.get("deg").or_else(|| ea.pos.get(2)) {
                Some(Value::Num(d)) if *d < 360.0 => solid(Solid::revolve_arc(&shape, seg, *d)),
                _ => solid(Solid::revolve(&shape, seg)),
            }
        }
        "lathe" => solid(Solid::lathe(&ea.sketch(0, "shape")?, ea.seg(1, 64)?)),
        // `loft(a, b, c, ..., h=H)`: skin through every positional sketch section,
        // stacked along Z over height `h` (named, or a trailing positional number,
        // default 10). Caps on → watertight.
        "loft" => {
            let sections: Vec<Sketch> = ea
                .pos
                .iter()
                .filter_map(|v| match v {
                    Value::Sketch(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            if sections.len() < 2 {
                return Err("loft needs at least 2 sketches".into());
            }
            let h = match ea.named.get("h") {
                Some(Value::Num(n)) => *n,
                Some(other) => {
                    return Err(format!("argument `h` must be a number, got {}", other.kind()))
                }
                None => match ea.pos.last() {
                    Some(Value::Num(n)) => *n,
                    _ => 10.0,
                },
            };
            solid(Solid::loft(&sections, h, true))
        }
        // `sweep(shape, path)`: sweep the `shape` cross-section (XY) along `path`,
        // a sketch whose points are read as `(x, z)` — a natural spine is a
        // `bezier(...)`. Rotation-minimizing frame → minimal twist; caps on.
        "sweep" => {
            let section = ea.sketch(0, "shape")?;
            let path = match ea.named.get("path").or_else(|| ea.pos.get(1)) {
                Some(Value::Sketch(s)) => s.clone(),
                Some(other) => {
                    return Err(format!("argument `path` must be a sketch, got {}", other.kind()))
                }
                None => return Err("missing sketch argument `path`".into()),
            };
            if path.points.len() < 2 {
                return Err("sweep needs a path of at least 2 points".into());
            }
            solid(Solid::sweep(&section, &path.points, true))
        }
        // helix(shape, r, pitch, turns[, seg]) -- sweep a section along a helical
        // spine of radius `r`, rising `pitch` per turn for `turns` turns.
        "helix" => {
            use crate::csg::bsp::v3;
            let section = ea.sketch(0, "shape")?;
            let r = ea.num(1, "r", None)?;
            let pitch = ea.num(2, "pitch", None)?;
            let turns = ea.num(3, "turns", Some(1.0))?.abs().max(1e-3);
            let seg = ea.seg(4, 48)?;
            let n = ((seg as f64 * turns).ceil() as usize).max(2);
            let two_pi = std::f64::consts::PI * 2.0;
            let path: Vec<_> = (0..=n)
                .map(|i| {
                    let f = i as f64 / n as f64; // 0..1 over the whole helix
                    let a = two_pi * turns * f;
                    v3(r * a.cos(), r * a.sin(), pitch * turns * f)
                })
                .collect();
            solid(Solid::sweep_path(&section, &path, true))
        }

        // Volumetric fields. The polygon backend has no density
        // representation, so it only records that they were here; the
        // analytic backend (`dsl/sdf.rs`) exports them for real.
        "fire" => {
            let (r, h) = (ea.num(0, "r", None)?, ea.num(1, "h", None)?);
            check_positive("fire", &[("r", r), ("h", h)])?;
            let v = Vol::fire(r, h, ea.seed_arg(2)?, ea.phase_arg(3)?);
            Ok(Value::Vol { solid: None, names: vec![v.describe()] })
        }
        "fog" => {
            let (w, d, h) = (
                ea.num(0, "w", None)?,
                ea.num(1, "d", None)?,
                ea.num(2, "h", None)?,
            );
            check_positive("fog", &[("w", w), ("d", d), ("h", h)])?;
            let v = Vol::fog(w, d, h, ea.seed_arg(3)?, ea.phase_arg(4)?);
            Ok(Value::Vol { solid: None, names: vec![v.describe()] })
        }
        "cloud" => {
            let r = ea.num(0, "r", None)?;
            check_positive("cloud", &[("r", r)])?;
            let v = Vol::cloud(r, ea.seed_arg(1)?, ea.phase_arg(2)?);
            Ok(Value::Vol { solid: None, names: vec![v.describe()] })
        }

        other => Err(format!("unknown function `{other}`")),
    }
}

/// Placement methods on a volumetric group: the transform lands on whatever
/// solid rides with it; the field itself is placed by the analytic backend.
/// Anything else (bevels, patterns) has no meaning for a density field here.
const VOL_METHODS: &[&str] =
    &["move", "translate", "scale", "rotate", "rotatex", "rotatey", "rotatez"];

fn call_method(recv: Value, method: &str, ea: &EvalArgs) -> Result<Value, String> {
    use crate::csg::bsp::v3;
    if let Value::Vol { solid, names } = recv {
        if !VOL_METHODS.contains(&method) {
            return Err(format!(
                "`.{method}` is not a method on a volumetric ({}) -- only \
                 {} place a density field",
                names.join(", "),
                VOL_METHODS.join("/")
            ));
        }
        let moved = match solid {
            Some(s) => match call_method(Value::Solid(s), method, ea)? {
                Value::Solid(s) => Some(s),
                _ => None,
            },
            None => None,
        };
        return Ok(Value::Vol { solid: moved, names });
    }
    match (recv, method) {
        (Value::Solid(s), "move") | (Value::Solid(s), "translate") => Ok(Value::Solid(s.translate(
            ea.num(0, "x", Some(0.0))?,
            ea.num(1, "y", Some(0.0))?,
            ea.num(2, "z", Some(0.0))?,
        ))),
        (Value::Solid(s), "scale") => {
            let sx = ea.num(0, "x", None)?;
            // One arg → uniform scale.
            let sy = ea.num(1, "y", Some(sx))?;
            let sz = ea.num(2, "z", Some(sx))?;
            Ok(Value::Solid(s.scale(sx, sy, sz)))
        }
        (Value::Solid(s), "rotatex") => Ok(Value::Solid(s.rotate(v3(1.0, 0.0, 0.0), ea.num(0, "deg", None)?))),
        (Value::Solid(s), "rotatey") => Ok(Value::Solid(s.rotate(v3(0.0, 1.0, 0.0), ea.num(0, "deg", None)?))),
        (Value::Solid(s), "rotatez") => Ok(Value::Solid(s.rotate(v3(0.0, 0.0, 1.0), ea.num(0, "deg", None)?))),
        (Value::Solid(s), "rotate") => {
            // rotate("z", 90) — axis is a string, then degrees.
            let axis = match ea.named.get("axis").or_else(|| ea.pos.first()) {
                Some(Value::Str(a)) => a.clone(),
                _ => return Err("rotate expects an axis string, e.g. rotate(\"z\", 90)".into()),
            };
            let deg = ea.num(1, "deg", None)?;
            let ax = match axis.as_str() {
                "x" => v3(1.0, 0.0, 0.0),
                "y" => v3(0.0, 1.0, 0.0),
                "z" => v3(0.0, 0.0, 1.0),
                other => return Err(format!("unknown axis \"{other}\" (use \"x\", \"y\", or \"z\")")),
            };
            Ok(Value::Solid(s.rotate(ax, deg)))
        }
        // Patterns: linear array along an axis, circular array, mirror.
        (Value::Solid(s), "arrayx") => Ok(Value::Solid(s.array(
            ea.num(0, "n", None)? as usize,
            ea.num(1, "dx", None)?,
            0.0,
            0.0,
        ))),
        (Value::Solid(s), "arrayy") => Ok(Value::Solid(s.array(
            ea.num(0, "n", None)? as usize,
            0.0,
            ea.num(1, "dy", None)?,
            0.0,
        ))),
        (Value::Solid(s), "arrayz") => Ok(Value::Solid(s.array(
            ea.num(0, "n", None)? as usize,
            0.0,
            0.0,
            ea.num(1, "dz", None)?,
        ))),
        (Value::Solid(s), "polar") => {
            // polar(n) or polar(n, "z") — n copies a full turn about the axis.
            let n = ea.num(0, "n", None)? as usize;
            let axis = match ea.named.get("axis").or_else(|| ea.pos.get(1)) {
                Some(Value::Str(a)) => a.clone(),
                _ => "z".to_string(),
            };
            let ax = match axis.as_str() {
                "x" => v3(1.0, 0.0, 0.0),
                "y" => v3(0.0, 1.0, 0.0),
                "z" => v3(0.0, 0.0, 1.0),
                other => return Err(format!("unknown axis \"{other}\" (use \"x\", \"y\", or \"z\")")),
            };
            Ok(Value::Solid(s.polar(n, ax)))
        }
        (Value::Solid(s), "mirror") | (Value::Solid(s), "reflect") => {
            // mirror("x") symmetrizes; reflect("x") is the bare reflection.
            let axis = match ea.named.get("axis").or_else(|| ea.pos.first()) {
                Some(Value::Str(a)) => a.clone(),
                _ => return Err("mirror expects an axis string, e.g. mirror(\"x\")".into()),
            };
            let (nx, ny, nz) = match axis.as_str() {
                "x" => (true, false, false),
                "y" => (false, true, false),
                "z" => (false, false, true),
                other => return Err(format!("unknown axis \"{other}\" (use \"x\", \"y\", or \"z\")")),
            };
            Ok(Value::Solid(if method == "mirror" {
                s.mirror(nx, ny, nz)
            } else {
                s.reflect(nx, ny, nz)
            }))
        }
        (Value::Solid(s), "fillet") => {
            let r = ea.num(0, "r", None)?;
            let seg = ea.seg(1, 8)?;
            Ok(Value::Solid(s.fillet(r, seg)?))
        }
        (Value::Solid(s), "chamfer") => {
            let d = ea.num(0, "d", None)?;
            Ok(Value::Solid(s.chamfer(d)?))
        }
        (Value::Sketch(s), "move") | (Value::Sketch(s), "translate2d") => Ok(Value::Sketch(
            s.translate2d(ea.num(0, "x", Some(0.0))?, ea.num(1, "y", Some(0.0))?),
        )),
        (recv, m) => Err(format!("`{m}` is not a method on a {}", recv.kind())),
    }
}
