//! Evaluate the AST into an **analytic SDF tree** (osgedit `Node` JSON)
//! instead of a polygonized `Solid`.
//!
//! This is the triangle-free path: primitives stay mathematical surfaces
//! (a sphere is `|p| - r`, exact at any zoom), booleans are min/max over
//! distance fields, and `seg` arguments are accepted but *ignored* — an
//! analytic surface has no tessellation resolution.
//!
//! Coordinate handling: the tree is built in the script's native Z-up frame;
//! round primitives (osgedit's cylinder/cone/torus run along +Y) are wrapped
//! in a +90° X rotation so they run along +Z as the manual promises, and the
//! whole model is wrapped in a −90° X rotation at the root so a script's
//! `z = 0` ground lands on osgedit's `y = 0` ground.
//!
//! Supported: all analytic primitives (`cube box cylinder cone frustum sphere
//! torus wedge pyramid tube`), booleans (`+ - *`), `move/translate`, `scale`,
//! `rotatex/y/z`, `rotate("ax", deg)`, `arrayx/y/z`, `polar`, `mirror`,
//! `reflect`. Arrays and polar patterns expand to explicit unions — exact,
//! and cheap at prop scale. `fillet`/`chamfer` are supported when the receiver
//! is a box (optionally under `move`/`rotate` wrappers, which commute exactly
//! with beveling): fillet maps to the rounded-box distance (`round` on the box
//!
//! Sketch-based solids: `revolve`/`lathe` of any sketch become the analytic
//! `lathe` prim — the profile polyline's exact 2D SDF spun about the axis, so
//! line-segment profiles are exact and curved ones (`bezier`, `limacon`,
//! `rose`, ...) are sampled at their authoring density yet stay *infinitely
//! round* in the revolve direction (the direction that shows). `extrude`
//! becomes the analytic `extrude` prim (2D polygon SDF ∩ z-slab; `twist=`
//! twists the domain). The revolve `seg` argument is accepted and ignored —
//! roundness has no tessellation — while sketch `seg` args still set the
//! profile's polyline sampling. `loft`, `sweep`, `helix`, partial (`deg` <
//! 360) revolves, and `fillet`/`chamfer` have no closed-form SDF here and
//! fail with a clear message rather than approximating.

use super::parser::{Arg, Expr, Stmt};
use super::volumetric::{check_positive, operator_error, Mat4, Vol};
use crate::csg::sketch::Sketch;
use serde_json::{json, Value as Json};
use std::collections::HashMap;

#[derive(Clone)]
enum Value {
    Num(f64),
    Str(String),
    Sketch(Sketch),
    Node(Json),
    /// One or more volumetric fields, optionally riding with the solid they
    /// were `+`-ed onto. They are kept strictly *beside* the tree -- a
    /// density field is never an SDF operand.
    Vol { solid: Option<Json>, vols: Vec<Vol> },
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Sketch(_) => "sketch",
            Value::Node(_) => "solid",
            Value::Vol { .. } => "volumetric",
        }
    }

    fn vols(&self) -> &[Vol] {
        match self {
            Value::Vol { vols, .. } => vols,
            _ => &[],
        }
    }
}

fn vol_names(vols: &[Vol]) -> Vec<String> {
    vols.iter().map(Vol::describe).collect()
}

type Env = HashMap<String, Value>;

/// Run a program and return the `model` expression as an osgedit scene: the
/// CSG node plus the volumetric records, both already converted from the
/// script's Z-up frame to osgedit's Y-up world.
pub fn run_program(stmts: &[Stmt]) -> Result<(Json, Vec<Json>), String> {
    let mut env: Env = HashMap::new();
    let mut result: Option<(Option<Json>, Vec<Vol>)> = None;
    for stmt in stmts {
        match stmt {
            Stmt::Let(name, expr) => {
                let v = eval(expr, &env)?;
                env.insert(name.clone(), v);
            }
            Stmt::Model(expr) => match eval(expr, &env)? {
                Value::Node(n) => result = Some((Some(n), Vec::new())),
                Value::Vol { solid, vols } => result = Some((solid, vols)),
                other => {
                    return Err(format!("`model` expects a solid, but got a {}", other.kind()))
                }
            },
        }
    }
    let (model, vols) =
        result.ok_or_else(|| "no `model ...` statement — nothing to render".to_string())?;
    // Z-up (script) -> Y-up (osgedit world): rotate −90° about X.
    let root = rotate_axis(
        [1.0, 0.0, 0.0],
        -90.0,
        // A volumetric-only model still exports: an empty union is the
        // identity of `+`, distance +inf, and the renderer just has nothing
        // to occlude the fields with.
        model.unwrap_or_else(|| nary("union", Vec::new())),
    );
    let root_m = Mat4::rot([1.0, 0.0, 0.0], -90.0);
    let vols = vols
        .into_iter()
        .map(|v| v.transformed(root_m).to_json())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((root, vols))
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
    // `solid + volumetric` is legal and splits the two apart; `-` and `*`
    // against a density field are hard errors, named.
    if !a.vols().is_empty() || !b.vols().is_empty() {
        let mut vols: Vec<Vol> = a.vols().to_vec();
        vols.extend_from_slice(b.vols());
        if op != '+' {
            return Err(operator_error(op, &vol_names(&vols)));
        }
        let mut solid: Option<Json> = None;
        for v in [a, b] {
            let s = match v {
                Value::Node(n) => Some(n),
                Value::Vol { solid, .. } => solid,
                other => {
                    return Err(format!(
                        "operator `+` doesn't apply to a volumetric and a {}",
                        other.kind()
                    ))
                }
            };
            solid = match (solid, s) {
                (Some(x), Some(y)) => Some(boolean("union", x, y)),
                (Some(x), None) => Some(x),
                (None, y) => y,
            };
        }
        return Ok(Value::Vol { solid, vols });
    }
    match (op, a, b) {
        ('+', Value::Node(x), Value::Node(y)) => Ok(Value::Node(boolean("union", x, y))),
        ('-', Value::Node(x), Value::Node(y)) => Ok(Value::Node(boolean("subtract", x, y))),
        ('*', Value::Node(x), Value::Node(y)) => Ok(Value::Node(boolean("intersect", x, y))),
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

// --- argument plumbing (mirrors eval.rs) -----------------------------------

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

    fn num(&self, idx: usize, name: &str, default: Option<f64>) -> Result<f64, String> {
        let v = self.named.get(name).or_else(|| self.pos.get(idx)).cloned();
        match v {
            Some(Value::Num(n)) => Ok(n),
            Some(other) => Err(format!("argument `{name}` must be a number, got {}", other.kind())),
            None => default.ok_or_else(|| format!("missing argument `{name}`")),
        }
    }

    /// Sketch sampling density (`seg`) — still meaningful for sketches, whose
    /// polylines the analytic prims consume; solid `seg` args stay ignored.
    fn seg(&self, idx: usize, default: usize) -> Result<usize, String> {
        Ok(self.num(idx, "seg", Some(default as f64))?.max(3.0) as usize)
    }

    /// `seed` picks which noise field a volumetric samples (default 0).
    fn seed_arg(&self, idx: usize) -> Result<u64, String> {
        Ok(self.num(idx, "seed", Some(0.0))?.max(0.0).min(u32::MAX as f64) as u64)
    }

    /// `phase` advances flame lick / drift (default 0). An animation is a
    /// phase sweep across re-renders, so it stays deterministic per frame.
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

    fn axis_str(&self, idx: usize, default: Option<&str>) -> Result<String, String> {
        match self.pos.get(idx) {
            Some(Value::Str(s)) => Ok(s.clone()),
            Some(other) => Err(format!("expected an axis string, got {}", other.kind())),
            None => default
                .map(|s| s.to_string())
                .ok_or_else(|| "expected an axis string, e.g. \"x\"".into()),
        }
    }
}

// --- JSON node builders -----------------------------------------------------

fn prim(shape: Json) -> Json {
    json!({ "kind": "prim", "prim": shape })
}

fn boolean(kind: &str, a: Json, b: Json) -> Json {
    json!({ "kind": kind, "children": [a, b] })
}

fn nary(kind: &str, children: Vec<Json>) -> Json {
    json!({ "kind": kind, "children": children })
}

fn translate(v: [f64; 3], child: Json) -> Json {
    json!({ "kind": "translate", "v": v, "child": child })
}

fn scale_node(s: [f64; 3], child: Json) -> Json {
    json!({ "kind": "scale", "s": s, "child": child })
}

/// Quaternion [x, y, z, w] for `deg` about `axis` (unit), then a Rotate node.
fn rotate_axis(axis: [f64; 3], deg: f64, child: Json) -> Json {
    let half = deg.to_radians() * 0.5;
    let s = half.sin();
    let q = [axis[0] * s, axis[1] * s, axis[2] * s, half.cos()];
    json!({ "kind": "rotate", "q": q, "child": child })
}

/// Wrap a +Y-axis osgedit primitive so it runs along the script's +Z.
fn y_to_z(child: Json) -> Json {
    rotate_axis([1.0, 0.0, 0.0], 90.0, child)
}

fn boxp(w: f64, d: f64, h: f64) -> Json {
    prim(json!({ "shape": "box", "half": [w / 2.0, d / 2.0, h / 2.0] }))
}

fn cylinder_z(r: f64, h: f64) -> Json {
    y_to_z(prim(json!({ "shape": "cylinder", "r": r, "h": h })))
}

fn plane(n: [f64; 3], point_on_plane: [f64; 3]) -> Json {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let nn = [n[0] / len, n[1] / len, n[2] / len];
    let offset = nn[0] * point_on_plane[0] + nn[1] * point_on_plane[1] + nn[2] * point_on_plane[2];
    prim(json!({ "shape": "plane", "n": nn, "offset": offset }))
}

/// The analytic body-of-revolution prim: the sketch's points read as a closed
/// `(r, z)` profile spun about the script's Z axis. Straight-sided profiles
/// are exact; curved ones carry their authoring sampling density but stay
/// infinitely round in the revolve direction.
fn lathe_prim(shape: &Sketch, what: &str) -> Result<Json, String> {
    if shape.points.len() < 3 {
        return Err(format!("`{what}` needs a profile sketch of at least 3 points"));
    }
    let pts: Vec<[f64; 2]> = shape.points.iter().map(|&(r, z)| [r, z]).collect();
    Ok(prim(json!({ "shape": "lathe", "pts": pts })))
}

/// The analytic prism prim: the sketch as an XY cross-section extruded along
/// the script's Z over `[-h/2, +h/2]`, optionally twisted.
fn extrude_prim(shape: &Sketch, h: f64, twist_deg: f64) -> Result<Json, String> {
    if shape.points.len() < 3 {
        return Err("`extrude` needs a cross-section sketch of at least 3 points".into());
    }
    let pts: Vec<[f64; 2]> = shape.points.iter().map(|&(x, y)| [x, y]).collect();
    let mut j = json!({ "shape": "extrude", "pts": pts, "h": h });
    if twist_deg != 0.0 {
        j["twist_deg"] = json!(twist_deg);
    }
    Ok(prim(j))
}

// --- builtins ---------------------------------------------------------------

fn call_builtin(name: &str, ea: &EvalArgs) -> Result<Value, String> {
    let node = |n: Json| Ok(Value::Node(n));
    let sketch = |s: Sketch| Ok(Value::Sketch(s));
    // Volumetric local frame is Y-up; stand it up in the script's Z-up world.
    let vol = |v: Vol| {
        Ok(Value::Vol {
            solid: None,
            vols: vec![v.transformed(Mat4::rot([1.0, 0.0, 0.0], 90.0))],
        })
    };
    match name {
        "cube" => {
            let s = ea.num(0, "s", None)?;
            node(boxp(s, s, s))
        }
        "box" => node(boxp(
            ea.num(0, "w", None)?,
            ea.num(1, "d", None)?,
            ea.num(2, "h", None)?,
        )),
        // seg args are accepted and ignored: analytic surfaces have no facets.
        "cylinder" => node(cylinder_z(ea.num(0, "r", None)?, ea.num(1, "h", None)?)),
        "cone" => {
            let r = ea.num(0, "r", None)?;
            let h = ea.num(1, "h", None)?;
            node(y_to_z(prim(json!({ "shape": "cone", "r1": r, "r2": 0.0, "h": h }))))
        }
        "frustum" => {
            let r1 = ea.num(0, "r1", None)?;
            let r2 = ea.num(1, "r2", None)?;
            let h = ea.num(2, "h", None)?;
            node(y_to_z(prim(json!({ "shape": "cone", "r1": r1, "r2": r2, "h": h }))))
        }
        "sphere" => node(prim(json!({ "shape": "sphere", "r": ea.num(0, "r", None)? }))),
        "torus" => {
            let rr = ea.num(0, "R", None)?;
            let r = ea.num(1, "r", None)?;
            node(y_to_z(prim(json!({ "shape": "torus", "major": rr, "minor": r }))))
        }
        "wedge" => {
            // Triangle profile in XZ — (-x,-z), (x,-z), (-x,z) — extruded along
            // Y, matching primitives.rs. Box ∩ hypotenuse half-space.
            let w = ea.num(0, "w", None)?;
            let d = ea.num(1, "d", None)?;
            let h = ea.num(2, "h", None)?;
            let hyp = plane([h, 0.0, w], [w / 2.0, 0.0, -h / 2.0]);
            node(nary("intersect", vec![boxp(w, d, h), hyp]))
        }
        "pyramid" => {
            // Base w×d at −h/2, apex at +h/2: box ∩ four apex planes.
            let w = ea.num(0, "w", None)?;
            let d = ea.num(1, "d", None)?;
            let h = ea.num(2, "h", None)?;
            let apex = [0.0, 0.0, h / 2.0];
            let sides = vec![
                boxp(w, d, h),
                plane([2.0 * h, 0.0, w], apex),
                plane([-2.0 * h, 0.0, w], apex),
                plane([0.0, 2.0 * h, d], apex),
                plane([0.0, -2.0 * h, d], apex),
            ];
            node(nary("intersect", sides))
        }
        "tube" => {
            let ro = ea.num(0, "R", None)?;
            let ri = ea.num(1, "r", None)?;
            let h = ea.num(2, "h", None)?;
            if ri >= ro {
                return Err("tube: inner radius r must be smaller than outer R".into());
            }
            node(boolean("subtract", cylinder_z(ro, h), cylinder_z(ri, h * 1.001)))
        }
        // Sketches — same constructors and sampling densities as the mesh
        // path (`eval.rs`), so both backends see the same profile points.
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

        // Sketch → solid: the analytic lathe / extrude prims. The revolve
        // `seg` is accepted and ignored (an analytic surface of revolution is
        // infinitely round); a partial revolve stays mesh-only.
        "revolve" => {
            let shape = ea.sketch(0, "shape")?;
            let _seg = ea.seg(1, 64)?;
            if let Some(Value::Num(d)) = ea.named.get("deg").or_else(|| ea.pos.get(2)) {
                if *d < 360.0 {
                    return Err(
                        "partial `revolve` (deg < 360) has no analytic SDF here — \
                         use the mesh path for this model"
                            .into(),
                    );
                }
            }
            node(lathe_prim(&shape, "revolve")?)
        }
        "lathe" => node(lathe_prim(&ea.sketch(0, "shape")?, "lathe")?),
        "extrude" => {
            let shape = ea.sketch(0, "shape")?;
            let h = ea.num(1, "h", None)?;
            let twist = match ea.named.get("twist") {
                Some(Value::Num(d)) if d.abs() > 1e-6 => *d,
                _ => 0.0,
            };
            node(extrude_prim(&shape, h, twist)?)
        }

        // Volumetric fields: density, not surface. They leave here as a
        // `Value::Vol` and are split out of the tree at `model` time -- see
        // `dsl/volumetric.rs`. The local frame is Y-up, so each one carries
        // the same +90 deg X rotation the round prims use to stand up in the
        // script's Z-up world; the root -90 deg cancels it back to Y-up.
        "fire" => {
            let (r, h) = (ea.num(0, "r", None)?, ea.num(1, "h", None)?);
            check_positive("fire", &[("r", r), ("h", h)])?;
            vol(Vol::fire(r, h, ea.seed_arg(2)?, ea.phase_arg(3)?))
        }
        "fog" => {
            let (w, d, h) = (
                ea.num(0, "w", None)?,
                ea.num(1, "d", None)?,
                ea.num(2, "h", None)?,
            );
            check_positive("fog", &[("w", w), ("d", d), ("h", h)])?;
            vol(Vol::fog(w, d, h, ea.seed_arg(3)?, ea.phase_arg(4)?))
        }
        "cloud" => {
            let r = ea.num(0, "r", None)?;
            check_positive("cloud", &[("r", r)])?;
            vol(Vol::cloud(r, ea.seed_arg(1)?, ea.phase_arg(2)?))
        }

        "loft" | "sweep" | "helix" => Err(format!(
            "`{name}` has no analytic SDF — general skinned/swept surfaces stay \
             mesh-only; use the mesh path for this model"
        )),
        other => Err(format!("unknown function `{other}`")),
    }
}

// --- methods ----------------------------------------------------------------

fn axis_unit(ax: &str) -> Result<[f64; 3], String> {
    match ax {
        "x" => Ok([1.0, 0.0, 0.0]),
        "y" => Ok([0.0, 1.0, 0.0]),
        "z" => Ok([0.0, 0.0, 1.0]),
        _ => Err(format!("unknown axis `{ax}` (expected \"x\", \"y\", or \"z\")")),
    }
}

fn reflect_node(ax: &str, child: Json) -> Result<Json, String> {
    // Exact reflection via a −1 scale on one axis (|det| = 1, so the distance
    // field is preserved exactly by osgedit's Scale node).
    let s = match ax {
        "x" => [-1.0, 1.0, 1.0],
        "y" => [1.0, -1.0, 1.0],
        "z" => [1.0, 1.0, -1.0],
        _ => return Err(format!("unknown axis `{ax}` (expected \"x\", \"y\", or \"z\")")),
    };
    Ok(scale_node(s, child))
}

fn linear_array(child: Json, n: usize, step: [f64; 3]) -> Json {
    let n = n.max(1);
    let copies: Vec<Json> = (0..n)
        .map(|k| {
            let f = k as f64;
            if k == 0 {
                child.clone()
            } else {
                translate([step[0] * f, step[1] * f, step[2] * f], child.clone())
            }
        })
        .collect();
    if copies.len() == 1 {
        copies.into_iter().next().unwrap()
    } else {
        nary("union", copies)
    }
}

/// The affine a placement method applies, or `None` if the method is not a
/// placement (patterns, bevels). Volumetrics accept only placements.
fn placement_matrix(method: &str, ea: &EvalArgs) -> Result<Option<Mat4>, String> {
    Ok(Some(match method {
        "move" | "translate" => Mat4::translate(
            ea.num(0, "x", Some(0.0))?,
            ea.num(1, "y", Some(0.0))?,
            ea.num(2, "z", Some(0.0))?,
        ),
        "scale" => {
            let sx = ea.num(0, "x", None)?;
            Mat4::scale(sx, ea.num(1, "y", Some(sx))?, ea.num(2, "z", Some(sx))?)
        }
        "rotatex" => Mat4::rot([1.0, 0.0, 0.0], ea.num(0, "deg", None)?),
        "rotatey" => Mat4::rot([0.0, 1.0, 0.0], ea.num(0, "deg", None)?),
        "rotatez" => Mat4::rot([0.0, 0.0, 1.0], ea.num(0, "deg", None)?),
        "rotate" => {
            let ax = ea.axis_str(0, None)?;
            Mat4::rot(axis_unit(&ax)?, ea.num(1, "deg", None)?)
        }
        _ => return Ok(None),
    }))
}

fn call_method(recv: Value, method: &str, ea: &EvalArgs) -> Result<Value, String> {
    // Volumetric group: the placement lands on the field's own affine and on
    // whatever solid rides with it, so the two stay registered.
    if let Value::Vol { solid, vols } = recv {
        let Some(m) = placement_matrix(method, ea)? else {
            return Err(format!(
                "`.{method}` is not a method on a volumetric ({}) -- only \
                 move/translate/scale/rotate/rotatex/rotatey/rotatez place a \
                 density field",
                vol_names(&vols).join(", ")
            ));
        };
        let moved_solid = match solid {
            Some(n) => match call_method(Value::Node(n), method, ea)? {
                Value::Node(n) => Some(n),
                _ => None,
            },
            None => None,
        };
        return Ok(Value::Vol {
            solid: moved_solid,
            vols: vols.into_iter().map(|v| v.transformed(m)).collect(),
        });
    }
    // Sketch methods (mirrors eval.rs): 2D translate before lifting to 3D.
    if let Value::Sketch(s) = recv {
        return match method {
            "move" | "translate2d" => Ok(Value::Sketch(
                s.translate2d(ea.num(0, "x", Some(0.0))?, ea.num(1, "y", Some(0.0))?),
            )),
            other => Err(format!("`{other}` is not a method on a sketch")),
        };
    }
    let n = match recv {
        Value::Node(n) => n,
        other => {
            return Err(format!(
                "method `.{method}` applies to solids, not {}",
                other.kind()
            ))
        }
    };
    let node = |j: Json| Ok(Value::Node(j));
    match method {
        "move" | "translate" => node(translate(
            [
                ea.num(0, "x", Some(0.0))?,
                ea.num(1, "y", Some(0.0))?,
                ea.num(2, "z", Some(0.0))?,
            ],
            n,
        )),
        "scale" => {
            let sx = ea.num(0, "x", None)?;
            let sy = ea.num(1, "y", Some(sx))?;
            let sz = ea.num(2, "z", Some(sx))?;
            node(scale_node([sx, sy, sz], n))
        }
        "rotatex" => node(rotate_axis([1.0, 0.0, 0.0], ea.num(0, "deg", None)?, n)),
        "rotatey" => node(rotate_axis([0.0, 1.0, 0.0], ea.num(0, "deg", None)?, n)),
        "rotatez" => node(rotate_axis([0.0, 0.0, 1.0], ea.num(0, "deg", None)?, n)),
        "rotate" => {
            let ax = ea.axis_str(0, None)?;
            let deg = ea.num(1, "deg", None)?;
            node(rotate_axis(axis_unit(&ax)?, deg, n))
        }
        "arrayx" => {
            let cnt = ea.num(0, "n", None)?.max(1.0) as usize;
            let d = ea.num(1, "d", None)?;
            node(linear_array(n, cnt, [d, 0.0, 0.0]))
        }
        "arrayy" => {
            let cnt = ea.num(0, "n", None)?.max(1.0) as usize;
            let d = ea.num(1, "d", None)?;
            node(linear_array(n, cnt, [0.0, d, 0.0]))
        }
        "arrayz" => {
            let cnt = ea.num(0, "n", None)?.max(1.0) as usize;
            let d = ea.num(1, "d", None)?;
            node(linear_array(n, cnt, [0.0, 0.0, d]))
        }
        "polar" => {
            let cnt = ea.num(0, "n", None)?.max(1.0) as usize;
            let ax = ea.axis_str(1, Some("z"))?;
            let axis = axis_unit(&ax)?;
            let copies: Vec<Json> = (0..cnt)
                .map(|k| {
                    if k == 0 {
                        n.clone()
                    } else {
                        rotate_axis(axis, 360.0 * k as f64 / cnt as f64, n.clone())
                    }
                })
                .collect();
            node(if copies.len() == 1 {
                copies.into_iter().next().unwrap()
            } else {
                nary("union", copies)
            })
        }
        "mirror" => {
            let ax = ea.axis_str(0, None)?;
            let refl = reflect_node(&ax, n.clone())?;
            node(boolean("union", n, refl))
        }
        "reflect" => {
            let ax = ea.axis_str(0, None)?;
            node(reflect_node(&ax, n)?)
        }
        "fillet" => {
            // Rounded box: the box prim's `round` field shrinks the half-
            // extents by r internally and adds r back, so the overall
            // dimensions are preserved and every convex edge gets an exact
            // (not faceted) radius-r round. The mesh path's `seg` argument is
            // accepted and ignored — an analytic round has no facet count.
            let r = ea.num(0, "r", None)?;
            if !(r > 0.0) || !r.is_finite() {
                return Err("fillet: radius must be a positive number".into());
            }
            let out = on_box_receiver(&n, "fillet", &|half| {
                let hmin = half[0].min(half[1]).min(half[2]);
                if r >= hmin {
                    return Err(format!(
                        "fillet: radius {r} must be smaller than the box's \
                         smallest half-extent {hmin}"
                    ));
                }
                Ok(prim(json!({ "shape": "box", "half": half, "round": r })))
            })?;
            node(out)
        }
        "chamfer" => {
            // Chamfer = box ∩ 12 edge half-spaces, one 45° bevel plane per
            // convex edge, cut back `d` along each adjacent face — the same
            // planes the mesh path cuts with, so the two backends agree
            // exactly (the corner facets fall out of the intersections).
            let d = ea.num(0, "d", None)?;
            if !(d > 0.0) || !d.is_finite() {
                return Err("chamfer: distance must be a positive number".into());
            }
            let out = on_box_receiver(&n, "chamfer", &|half| {
                let hmin = half[0].min(half[1]).min(half[2]);
                if d >= 2.0 * hmin {
                    return Err(format!(
                        "chamfer: distance {d} too large for this box (would \
                         consume an edge of length {})",
                        2.0 * hmin
                    ));
                }
                let mut children =
                    vec![prim(json!({ "shape": "box", "half": half }))];
                for (i, j) in [(0usize, 1usize), (0, 2), (1, 2)] {
                    for si in [-1.0, 1.0] {
                        for sj in [-1.0, 1.0] {
                            let mut nrm = [0.0; 3];
                            nrm[i] = si;
                            nrm[j] = sj;
                            let mut pt = [0.0; 3];
                            pt[i] = si * (half[i] - d);
                            pt[j] = sj * half[j];
                            children.push(plane(nrm, pt));
                        }
                    }
                }
                Ok(nary("intersect", children))
            })?;
            node(out)
        }
        other => Err(format!("unknown method `.{other}`")),
    }
}

// --- fillet / chamfer receiver plumbing --------------------------------------

/// The half-extents of a bare, un-rounded box prim node, if that's what it is.
fn bare_box_half(n: &Json) -> Option<[f64; 3]> {
    if n.get("kind")?.as_str()? != "prim" {
        return None;
    }
    let p = n.get("prim")?;
    if p.get("shape")?.as_str()? != "box" {
        return None;
    }
    if p.get("round").and_then(Json::as_f64).unwrap_or(0.0) != 0.0 {
        return None;
    }
    let h = n.get("prim")?.get("half")?.as_array()?;
    if h.len() != 3 {
        return None;
    }
    Some([h[0].as_f64()?, h[1].as_f64()?, h[2].as_f64()?])
}

/// Apply `f` (a bevel builder taking the box's half-extents) to a box
/// receiver, peeling and re-applying any `translate`/`rotate` wrappers —
/// rigid motions commute exactly with beveling, so
/// `box.move(...).chamfer(d)` == `box.chamfer(d).move(...)`. Any other
/// receiver (sphere, boolean, scaled/mirrored box, ...) is an error: those
/// bevels have no closed form here.
fn on_box_receiver(
    n: &Json,
    method: &str,
    f: &dyn Fn([f64; 3]) -> Result<Json, String>,
) -> Result<Json, String> {
    if let Some(half) = bare_box_half(n) {
        return f(half);
    }
    match n.get("kind").and_then(Json::as_str) {
        Some("translate") => {
            let inner = on_box_receiver(&n["child"], method, f)?;
            Ok(json!({ "kind": "translate", "v": n["v"], "child": inner }))
        }
        Some("rotate") => {
            let inner = on_box_receiver(&n["child"], method, f)?;
            Ok(json!({ "kind": "rotate", "q": n["q"], "child": inner }))
        }
        _ => Err(format!(
            "`.{method}` in the SDF export needs a box receiver — apply it to \
             a bare `box`/`cube` (moves and rotations in between are fine); \
             this receiver is unsupported, use the mesh path for this model"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::Json;
    use crate::dsl::compile_sdf;

    /// compile_sdf wraps the whole model in the Z-up → Y-up root rotation;
    /// peel it to inspect the model node itself.
    fn unwrap_root(v: &Json) -> &Json {
        assert_eq!(v["kind"], "rotate", "root should be the Z-up→Y-up rotate");
        &v["child"]
    }

    // --- D3: fillet on box receivers -------------------------------------

    #[test]
    fn fillet_on_cube_is_a_rounded_box() {
        let v = compile_sdf("model cube(2).fillet(0.4, 6)").unwrap();
        let n = unwrap_root(&v);
        assert_eq!(n["kind"], "prim");
        assert_eq!(n["prim"]["shape"], "box");
        assert_eq!(n["prim"]["half"], serde_json::json!([1.0, 1.0, 1.0]));
        assert_eq!(n["prim"]["round"], serde_json::json!(0.4));
    }

    #[test]
    fn fillet_commutes_through_move_and_rotate_wrappers() {
        let v = compile_sdf("model box(2, 3, 1).rotatez(30).move(0, 0, 5).fillet(0.2)")
            .unwrap();
        let mut n = unwrap_root(&v);
        assert_eq!(n["kind"], "translate");
        n = &n["child"];
        assert_eq!(n["kind"], "rotate");
        n = &n["child"];
        assert_eq!(n["prim"]["shape"], "box");
        assert_eq!(n["prim"]["round"], serde_json::json!(0.2));
    }

    #[test]
    fn fillet_radius_must_fit_the_box() {
        let e = compile_sdf("model box(2, 3, 1).fillet(0.6)").unwrap_err();
        assert!(e.contains("half-extent"), "message was: {e}");
        let e = compile_sdf("model cube(2).fillet(-0.1)").unwrap_err();
        assert!(e.contains("positive"), "message was: {e}");
    }

    #[test]
    fn fillet_rejects_non_box_receivers() {
        for src in [
            "model sphere(3).fillet(0.2)",
            "model cylinder(1, 4).fillet(0.2)",
            "let a = cube(2) + sphere(1)\nmodel a.fillet(0.2)",
            "model cube(2).scale(1, 2, 1).fillet(0.2)",
        ] {
            let e = compile_sdf(src).unwrap_err();
            assert!(e.contains("box receiver"), "{src}: message was {e}");
        }
    }

    // --- D3: chamfer on box receivers -------------------------------------

    #[test]
    fn chamfer_on_box_is_box_intersect_12_edge_planes() {
        let v = compile_sdf("model box(2, 3, 4).chamfer(0.3)").unwrap();
        let n = unwrap_root(&v);
        assert_eq!(n["kind"], "intersect");
        let ch = n["children"].as_array().unwrap();
        assert_eq!(ch.len(), 13, "box + 12 edge planes");
        assert_eq!(ch[0]["prim"]["shape"], "box");
        for p in &ch[1..] {
            assert_eq!(p["prim"]["shape"], "plane");
        }
        // First plane pair is the (x, y) edge family: half = [1, 1.5, 2], so
        // the bevel plane offset is (hx + hy - d) / sqrt(2), same for all
        // four sign combinations.
        let want = (1.0 + 1.5 - 0.3) / 2f64.sqrt();
        for p in &ch[1..5] {
            let off = p["prim"]["offset"].as_f64().unwrap();
            assert!((off - want).abs() < 1e-12, "offset {off}, want {want}");
        }
    }
    fn chamfer_distance_must_fit_and_receiver_must_be_a_box() {
        let e = compile_sdf("model box(2, 3, 1).chamfer(1.0)").unwrap_err();
        assert!(e.contains("too large"), "message was: {e}");
        let e = compile_sdf("model sphere(2).chamfer(0.1)").unwrap_err();
        assert!(e.contains("box receiver"), "message was: {e}");
    }

    // --- D3 acceptance: dumpster.vim exports through the SDF path ---------

    #[test]
    fn dumpster_vim_exports_and_still_compiles_as_mesh() {
        let src = include_str!("../../../../assets/props/builtin/dumpster.vim");
        // Mesh backend (ground truth) still compiles to a positive volume.
        let solid = crate::compile_vim(src).expect("mesh compile").solid;
        assert!(solid.volume() > 0.0);
        // SDF backend now accepts the `.move(...).chamfer(...)` body: the
        // translate wrapper is peeled, the bevel lands on the bare box.
        let v = compile_sdf(src).expect("dumpster.vim should export --sdf");
        let s = v.to_string();
        assert!(s.contains("\"plane\""), "chamfer should emit plane cuts");
    }


    fn compile(src: &str) -> Result<Json, String> {
        crate::compile_sdf(src)
    }

    /// Count nodes in the emitted tree whose `"shape"` equals `shape`.
    fn count_shapes(j: &Json, shape: &str) -> usize {
        let mut n = 0;
        match j {
            Json::Object(m) => {
                if m.get("shape").and_then(Json::as_str) == Some(shape) {
                    n += 1;
                }
                for v in m.values() {
                    n += count_shapes(v, shape);
                }
            }
            Json::Array(a) => {
                for v in a {
                    n += count_shapes(v, shape);
                }
            }
            _ => {}
        }
        n
    }

    /// Find the first prim object with the given shape tag.
    fn find_shape<'a>(j: &'a Json, shape: &str) -> Option<&'a Json> {
        match j {
            Json::Object(m) => {
                if m.get("shape").and_then(Json::as_str) == Some(shape) {
                    return Some(j);
                }
                m.values().find_map(|v| find_shape(v, shape))
            }
            Json::Array(a) => a.iter().find_map(|v| find_shape(v, shape)),
            _ => None,
        }
    }

    #[test]
    fn lathe_of_bezier_emits_lathe_prim_with_profile_points() {
        let src = "let sil = bezier(0,10, 0.3,10, 1.6,9, 1.6,8, steps=8)\n\
                   model lathe(sil, 48)";
        let j = compile(src).expect("lathe should export");
        let lathe = find_shape(&j, "lathe").expect("a lathe prim in the tree");
        // Same profile sampling as the mesh path: 1 anchor + 8 bezier steps.
        let pts = lathe["pts"].as_array().unwrap();
        assert_eq!(pts.len(), 9);
        // First point is the (r, z) anchor on the axis.
        assert_eq!(pts[0][0].as_f64().unwrap(), 0.0);
        assert_eq!(pts[0][1].as_f64().unwrap(), 10.0);
    }

    #[test]
    fn revolve_full_circle_is_analytic_partial_is_not() {
        let full = "model revolve(polygon(1,0, 3,0, 3,2, 1,2), 32)";
        let j = compile(full).expect("full revolve should export");
        assert_eq!(count_shapes(&j, "lathe"), 1);
        // seg is accepted & ignored; deg=360 exactly is still a full revolve.
        compile("model revolve(polygon(1,0, 3,0, 3,2, 1,2), 32, 360)").unwrap();
        let partial = "model revolve(polygon(1,0, 3,0, 3,2, 1,2), 32, 90)";
        let e = compile(partial).unwrap_err();
        assert!(e.contains("partial"), "unexpected error: {e}");
    }

    #[test]
    fn extrude_emits_prism_with_optional_twist() {
        let j = compile("model extrude(rect(4, 2), 7)").unwrap();
        let ext = find_shape(&j, "extrude").expect("an extrude prim");
        assert_eq!(ext["h"].as_f64().unwrap(), 7.0);
        assert_eq!(ext["pts"].as_array().unwrap().len(), 4);
        assert!(ext.get("twist_deg").is_none(), "no twist key when untwisted");

        let j = compile("model extrude(rect(4, 2), 7, twist=30)").unwrap();
        let ext = find_shape(&j, "extrude").expect("an extrude prim");
        assert_eq!(ext["twist_deg"].as_f64().unwrap(), 30.0);
    }

    // --- volumetrics: split out of the tree, never an SDF operand ---------

    /// Round-trip: each builtin reaches the model JSON with a finite AABB and
    /// the params it was called with.
    #[test]
    fn each_volumetric_builtin_round_trips_with_finite_bounds() {
        for (src, kind, want) in [
            ("model cube(1) + fire(1, 2)", "fire", vec![("r", 1.0), ("h", 2.0)]),
            (
                "model cube(1) + fog(4, 6, 1.5)",
                "fog",
                vec![("w", 4.0), ("d", 6.0), ("h", 1.5)],
            ),
            ("model cube(1) + cloud(3)", "cloud", vec![("r", 3.0)]),
        ] {
            let (_, vols) = crate::compile_sdf_scene(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert_eq!(vols.len(), 1, "{src}");
            let v = &vols[0];
            assert_eq!(v["kind"], kind, "{src}");
            for (k, expect) in want {
                assert_eq!(v["params"][k].as_f64().unwrap(), expect, "{src}: param {k}");
            }
            assert_eq!(v["params"]["seed"].as_u64().unwrap(), 0);
            assert_eq!(v["params"]["phase"].as_f64().unwrap(), 0.0);
            assert_eq!(v["xform"].as_array().unwrap().len(), 16, "{src}");
            for key in ["min", "max"] {
                let b = v["bounds"][key].as_array().unwrap();
                assert_eq!(b.len(), 3);
                for c in b {
                    let c = c.as_f64().expect("bounds component is a number");
                    assert!(c.is_finite() && c.abs() < 1.0e5, "{src}: bound {c} not finite");
                }
            }
            // a finite, non-degenerate box
            let (mn, mx) = (&v["bounds"]["min"], &v["bounds"]["max"]);
            for i in 0..3 {
                assert!(
                    mx[i].as_f64().unwrap() > mn[i].as_f64().unwrap(),
                    "{src}: bounds are degenerate on axis {i}"
                );
            }
        }
    }

    /// `solid + volumetric` splits into exactly one solid tree and one
    /// volumetrics entry -- the volumetric must not appear in the tree.
    #[test]
    fn union_splits_the_volumetric_out_of_the_tree() {
        let (csg, vols) = crate::compile_sdf_scene("model cube(1) + fog(4, 4, 1)").unwrap();
        assert_eq!(vols.len(), 1);
        assert_eq!(vols[0]["kind"], "fog");
        // the tree is just the cube under the Z-up -> Y-up root rotation
        let n = unwrap_root(&csg);
        assert_eq!(n["kind"], "prim");
        assert_eq!(n["prim"]["shape"], "box");
        let text = csg.to_string();
        for word in ["fog", "fire", "cloud", "volumetric"] {
            assert!(!text.contains(word), "the tree leaked `{word}`: {text}");
        }
    }

    /// `-` and `*` against a volumetric are hard errors that name both the
    /// operator and the field.
    #[test]
    fn difference_and_intersection_with_a_volumetric_are_errors() {
        for (src, op) in [
            ("model fire(1, 2) - cube(1)", '-'),
            ("model cube(1) - fire(1, 2)", '-'),
            ("model cube(1) * cloud(2)", '*'),
            ("model fog(2, 2, 1) * cube(1)", '*'),
        ] {
            let e = crate::compile_sdf_scene(src).unwrap_err();
            assert!(e.contains(&format!("`{op}`")), "{src}: message was {e}");
            assert!(
                e.contains("fire(") || e.contains("cloud(") || e.contains("fog("),
                "{src}: message does not name the volumetric: {e}"
            );
            // the mesh backend must reject it identically
            let e = crate::compile_vim(src).unwrap_err();
            assert!(e.contains(&format!("`{op}`")), "{src} (mesh): message was {e}");
        }
    }

    /// The placement chain composes into the field's own affine, and the
    /// declared bounds move with it.
    #[test]
    fn placement_chain_moves_the_field_and_its_bounds() {
        let (_, v) =
            crate::compile_sdf_scene("model cube(1) + cloud(2).move(10, 0, 4)").unwrap();
        let b = &v[0]["bounds"];
        // script (10, 0, 4) -> world (10, 4, 0) (Z-up to Y-up)
        let cx = 0.5 * (b["min"][0].as_f64().unwrap() + b["max"][0].as_f64().unwrap());
        let cy = 0.5 * (b["min"][1].as_f64().unwrap() + b["max"][1].as_f64().unwrap());
        let cz = 0.5 * (b["min"][2].as_f64().unwrap() + b["max"][2].as_f64().unwrap());
        assert!((cx - 10.0).abs() < 1e-6, "{cx}");
        assert!((cy - 4.0).abs() < 1e-6, "{cy}");
        assert!(cz.abs() < 1e-6, "{cz}");
        // rotations are rejected only for non-placement methods
        let e = crate::compile_sdf_scene("model cube(1) + cloud(2).fillet(0.1)").unwrap_err();
        assert!(e.contains("not a method on a volumetric"), "{e}");
    }

    /// A volumetric-only model still exports: the tree is an empty union.
    #[test]
    fn a_volumetric_only_model_exports_an_empty_tree() {
        let (csg, vols) = crate::compile_sdf_scene("model fire(1, 2)").unwrap();
        assert_eq!(vols.len(), 1);
        let n = unwrap_root(&csg);
        assert_eq!(n["kind"], "union");
        assert_eq!(n["children"].as_array().unwrap().len(), 0);
        // ...but the polygon path has nothing to mesh and says so
        let e = crate::compile_vim("model fire(1, 2)").unwrap_err();
        assert!(e.contains("no solid geometry"), "{e}");
    }

    /// Seed and phase are plain arguments, positional or named.
    #[test]
    fn seed_and_phase_are_arguments_not_state() {
        let (_, v) =
            crate::compile_sdf_scene("model cube(1) + fire(1, 2, seed = 42, phase = 0.75)")
                .unwrap();
        assert_eq!(v[0]["params"]["seed"].as_u64().unwrap(), 42);
        assert_eq!(v[0]["params"]["phase"].as_f64().unwrap(), 0.75);
        // same source twice -> identical bytes (no clock, no RNG)
        let a = serde_json::to_string(&crate::sdf_model_json("x", "model cube(1) + cloud(2, 5, 0.3)").unwrap()).unwrap();
        let b = serde_json::to_string(&crate::sdf_model_json("x", "model cube(1) + cloud(2, 5, 0.3)").unwrap()).unwrap();
        assert_eq!(a, b);
    }

    /// Zero and negative extents are modeling mistakes, caught at compile.
    #[test]
    fn non_positive_extents_are_rejected() {
        for src in [
            "model cube(1) + fire(0, 2)",
            "model cube(1) + fire(1, -2)",
            "model cube(1) + fog(4, 0, 1)",
            "model cube(1) + cloud(0)",
        ] {
            let e = crate::compile_sdf_scene(src).unwrap_err();
            assert!(e.contains("positive"), "{src}: message was {e}");
        }
    }

    #[test]
    fn sketch_move_offsets_the_profile() {
        // Offset a rect profile outward before revolving — a ring.
        let j = compile("model revolve(rect(1, 2).move(5, 0), 32)").unwrap();
        let lathe = find_shape(&j, "lathe").unwrap();
        let pts = lathe["pts"].as_array().unwrap();
        let rs: Vec<f64> = pts.iter().map(|p| p[0].as_f64().unwrap()).collect();
        assert!(rs.iter().all(|r| (4.5..=5.5).contains(r)), "offset radii: {rs:?}");
    }

    #[test]
    fn curved_sketch_families_all_revolve_and_extrude() {
        // A2: every curved sketch family routes through the lathe/extrude prims
        // at its authoring density.
        for src in [
            "model lathe(bezier(0,10, 0.3,10, 1.6,9, 1.6,8, 1.6,8, 1.6,1, 0,1), 64)",
            "model revolve(limacon(2, 8), 64)",
            "model revolve(rose(6), 48)",
            "model revolve(superellipse(9, 6, 4), 48)",
            "model extrude(star(5, 10, 4), 4)",
            "model extrude(petals(8, 5), 3)",
            "model extrude(hypotrochoid(10, 3, 5), 3)",
            "model extrude(cardioid(6), 2)",
            "model extrude(ngon(6, 3), 2)",
            "model extrude(roundrect(6, 4, 1), 2)",
            "model extrude(slot(8, 2), 2)",
            "model extrude(svgpath(\"M0,0 L10,0 L10,10 Z\"), 2)",
        ] {
            let j = compile(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert_eq!(
                count_shapes(&j, "lathe") + count_shapes(&j, "extrude"),
                1,
                "{src}"
            );
        }
    }

    #[test]
    fn shipped_lathe_assets_export_analytically() {
        // The acceptance list: every sketch-based asset that isn't loft/sweep/
        // helix must now pass the analytic export.
        for (name, src) in [
            ("ogive_srbm", include_str!("../../assets/lathes/ogive_srbm.vim")),
            ("blunt_boattail", include_str!("../../assets/lathes/blunt_boattail.vim")),
            ("slender_icbm_rv", include_str!("../../assets/lathes/slender_icbm_rv.vim")),
            ("finned_missile", include_str!("../../assets/lathes/finned_missile.vim")),
            ("math_curves", include_str!("../../assets/lathes/math_curves.vim")),
            ("math_sections", include_str!("../../assets/lathes/math_sections.vim")),
        ] {
            compile(src).unwrap_or_else(|e| panic!("{name}.vim should export: {e}"));
        }
    }
    fn skinned_and_swept_stay_unsupported() {
        // The honest remainder: loft/sweep/helix have no analytic SDF.
        for (name, src) in [
            ("loft_demo", include_str!("../../assets/lathes/loft_demo.vim")),
            ("sweep_demo", include_str!("../../assets/lathes/sweep_demo.vim")),
            ("helix_demo", include_str!("../../assets/lathes/helix_demo.vim")),
            ("harp_arm", include_str!("../../assets/lathes/harp_arm.vim")),
            ("harp_neck", include_str!("../../assets/lathes/harp_neck.vim")),
        ] {
            let e = compile(src).unwrap_err();
            assert!(
                e.contains("no analytic SDF"),
                "{name}.vim should fail with the mesh-only message, got: {e}"
            );
        }
    }

    #[test]
    fn export_is_deterministic() {
        let src = include_str!("../../assets/lathes/ogive_srbm.vim");
        let a = serde_json::to_string(&compile(src).unwrap()).unwrap();
        let b = serde_json::to_string(&compile(src).unwrap()).unwrap();
        assert_eq!(a, b);
    }
}
