//! Nim-style modeling language: lex → parse → evaluate into a `Solid`.
//!
//! Principles carried over from Fornjot: code-first, mechanical-CAD focus, and
//! *reliability over features* — every construct either works or returns a
//! clear, actionable error (see `eval`).

mod eval;
mod lexer;
mod parser;
mod sdf;
pub mod sdf_analysis;
pub mod volumetric;

use crate::csg::Solid;
use parser::{Arg, Expr, Parser, Stmt};

/// A line in the build-step outline shown next to the editor.
#[derive(Clone, Debug)]
pub struct StepLine {
    pub depth: usize,
    pub text: String,
}

/// A successfully compiled model.
#[derive(Debug)]
pub struct Compiled {
    pub solid: Solid,
    pub steps: Vec<StepLine>,
    /// Volumetric fields (`fire`/`fog`/`cloud`) the polygon backend dropped.
    /// They are density, not surface: the BSP kernel has no representation
    /// for them, so they are stripped here and reported rather than silently
    /// ignored. Non-empty means the caller should warn.
    pub skipped_volumetrics: Vec<String>,
}

/// The example loaded into the editor on first run.
pub const EXAMPLE: &str = "\
# A flanged, bolted pipe — reads top-to-bottom as build steps.
let bore   = cylinder(r = 8,  h = 42)
let wall   = cylinder(r = 12, h = 40)
let pipe   = wall - bore              # difference: drill the bore

let flange = cylinder(r = 22, h = 6).move(0, 0, -17)
let bolt   = cylinder(r = 2,  h = 8).move(16, 0, -17)

# union the flange on, then cut one bolt hole
model pipe + flange - bolt
";

/// Compile source into a renderable solid plus its build-step outline.
pub fn compile(src: &str) -> Result<Compiled, String> {
    let toks = lexer::lex(src)?;
    let stmts = Parser::new(toks).parse_program()?;

    // Build the outline from the `model` expression, if present.
    let mut steps = Vec::new();
    if let Some(Stmt::Model(expr)) = stmts.iter().find(|s| matches!(s, Stmt::Model(_))) {
        describe(expr, 0, &mut steps);
    }

    let (solid, skipped_volumetrics) = eval::run_program(&stmts)?;
    Ok(Compiled { solid, steps, skipped_volumetrics })
}

/// Compile source into an **analytic SDF tree** (osgedit `Node` JSON) — the
/// triangle-free path. Same language, same programs; primitives stay exact
/// mathematical surfaces and `seg` is ignored.
///
/// Volumetrics are split out of the tree (see [`compile_sdf_scene`]); this
/// entry point returns the solid tree alone, which is what the distance-field
/// consumers (`sdf_analysis`, silhouette parity) want.
pub fn compile_sdf(src: &str) -> Result<serde_json::Value, String> {
    Ok(compile_sdf_scene(src)?.0)
}

/// Compile source into the full analytic scene: the solid SDF tree plus the
/// `fire`/`fog`/`cloud` fields, already placed in the renderer's Y-up world.
///
/// The two are siblings, never nested: a volumetric is composited over the
/// solid image in a second pass, so it must not appear as an SDF operand.
pub fn compile_sdf_scene(
    src: &str,
) -> Result<(serde_json::Value, Vec<serde_json::Value>), String> {
    let toks = lexer::lex(src)?;
    let stmts = Parser::new(toks).parse_program()?;
    sdf::run_program(&stmts)
}

/// The exact model-file object `vimtool --sdf` writes: the solid tree under
/// `"csg"`, and -- only when the script has any -- the density fields under a
/// sibling `"volumetrics"` array. Keeping the key absent for volumetric-free
/// scripts is deliberate: every existing export stays byte-for-byte what it
/// was.
pub fn sdf_model_json(id: &str, src: &str) -> Result<serde_json::Value, String> {
    let (csg, vols) = compile_sdf_scene(src)?;
    let mut file = serde_json::Map::new();
    file.insert("id".into(), serde_json::Value::String(id.to_string()));
    file.insert("csg".into(), csg);
    if !vols.is_empty() {
        file.insert("volumetrics".into(), serde_json::Value::Array(vols));
    }
    Ok(serde_json::Value::Object(file))
}

/// Evaluate a numeric relationship expression (`bore * 2`, `wall - bore`)
/// against a set of named parameter values. Reuses the modeling-language lexer
/// and parser, then folds the pure-arithmetic AST to a number. Returns `None`
/// if the expression references an unknown name, divides by zero, or isn't pure
/// arithmetic (a call, method, or string) -- i.e. it's not a numeric relation.
pub fn eval_num_expr(expr_src: &str, params: &[(String, f64)]) -> Option<f64> {
    let toks = lexer::lex(expr_src).ok()?;
    let expr = Parser::new(toks).parse_single().ok()?;
    eval_num(&expr, params)
}

/// The variable names referenced by a pure-arithmetic expression, in source
/// order without duplicates. Returns `None` if the expression contains a call,
/// method, or string literal -- those are solid/part bindings, not numeric
/// relationships. A bare number yields `Some(vec![])`.
pub fn numeric_refs(expr_src: &str) -> Option<Vec<String>> {
    let toks = lexer::lex(expr_src).ok()?;
    let expr = Parser::new(toks).parse_single().ok()?;
    let mut refs = Vec::new();
    collect_refs(&expr, &mut refs).then_some(refs)
}

/// Fold a pure-arithmetic expression to a number given the known parameters.
fn eval_num(e: &Expr, params: &[(String, f64)]) -> Option<f64> {
    match e {
        Expr::Num(n) => Some(*n),
        Expr::Var(name) => params.iter().find(|(k, _)| k == name).map(|(_, v)| *v),
        Expr::Neg(a) => eval_num(a, params).map(|v| -v),
        Expr::Bin(op, a, b) => {
            let x = eval_num(a, params)?;
            let y = eval_num(b, params)?;
            match op {
                '+' => Some(x + y),
                '-' => Some(x - y),
                '*' => Some(x * y),
                '/' => (y != 0.0).then_some(x / y),
                _ => None,
            }
        }
        Expr::Call(..) | Expr::Method(..) | Expr::Str(_) => None,
    }
}

/// Walk a pure-arithmetic expression, collecting variable names. Returns `false`
/// if it contains a call, method, or string (not a numeric relationship).
fn collect_refs(e: &Expr, out: &mut Vec<String>) -> bool {
    match e {
        Expr::Num(_) => true,
        Expr::Var(name) => {
            if !out.contains(name) {
                out.push(name.clone());
            }
            true
        }
        Expr::Neg(a) => collect_refs(a, out),
        Expr::Bin(_, a, b) => collect_refs(a, out) && collect_refs(b, out),
        Expr::Call(..) | Expr::Method(..) | Expr::Str(_) => false,
    }
}

/// Render the model expression as an indented op tree for the UI.
fn describe(expr: &Expr, depth: usize, out: &mut Vec<StepLine>) {
    let node = |text: String, out: &mut Vec<StepLine>| {
        out.push(StepLine { depth, text });
    };
    match expr {
        Expr::Bin('+', a, b) => {
            node("∪  union".into(), out);
            describe(a, depth + 1, out);
            describe(b, depth + 1, out);
        }
        Expr::Bin('-', a, b) => {
            node("∖  difference (cut)".into(), out);
            describe(a, depth + 1, out);
            describe(b, depth + 1, out);
        }
        Expr::Bin('*', a, b) => {
            node("∩  intersection".into(), out);
            describe(a, depth + 1, out);
            describe(b, depth + 1, out);
        }
        Expr::Bin(op, a, b) => {
            node(format!("{op}"), out);
            describe(a, depth + 1, out);
            describe(b, depth + 1, out);
        }
        Expr::Neg(a) => {
            node("negate".into(), out);
            describe(a, depth + 1, out);
        }
        Expr::Method(recv, m, args) => {
            node(format!(".{m}({})", summarize_args(args)), out);
            describe(recv, depth + 1, out);
        }
        Expr::Call(name, args) => node(format!("{name}({})", summarize_args(args)), out),
        Expr::Var(name) => node(name.clone(), out),
        Expr::Num(n) => node(format!("{n}"), out),
        Expr::Str(s) => node(format!("\"{s}\""), out),
    }
}

fn summarize_args(args: &[Arg]) -> String {
    args.iter()
        .map(|a| {
            let v = summarize_expr(&a.value);
            match &a.name {
                Some(n) => format!("{n}={v}"),
                None => v,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn summarize_expr(e: &Expr) -> String {
    match e {
        Expr::Num(n) => format!("{n}"),
        Expr::Var(v) => v.clone(),
        Expr::Str(s) => format!("\"{s}\""),
        Expr::Call(name, args) => format!("{name}({})", summarize_args(args)),
        Expr::Method(recv, m, _) => format!("{}.{m}(…)", summarize_expr(recv)),
        Expr::Neg(a) => format!("-{}", summarize_expr(a)),
        Expr::Bin(op, a, b) => format!("{} {op} {}", summarize_expr(a), summarize_expr(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn vol(src: &str) -> f64 {
        compile(src).unwrap().solid.volume()
    }

    #[test]
    fn example_compiles() {
        let c = compile(EXAMPLE).expect("example should compile");
        assert!(c.solid.volume() > 0.0);
        assert!(!c.steps.is_empty());
    }

    #[test]
    fn bezier_lathe_makes_a_hull() {
        // An ogive-ish missile silhouette (r,z): nose on axis -> body -> tail on
        // axis. Lathe (revolve) should yield a positive-volume solid.
        let src = "let sil = bezier(0,10,  0.3,10, 1.6,9, 1.6,8,  \
                   1.6,8, 1.6,3, 1.6,2,  1.6,2, 1.0,1, 0.2,0)\n\
                   model lathe(sil, 48)";
        let v = vol(src);
        assert!(v > 0.0, "bezier lathe volume {v}");
        // steps= controls sampling; more steps -> finer, still positive.
        let v2 = compile(
            "model lathe(bezier(0,10, 0.3,10, 1.6,9, 1.6,8, steps=32), 32)",
        )
        .unwrap()
        .solid
        .volume();
        assert!(v2 > 0.0);
    }

    #[test]
    fn shipped_lathe_examples_compile() {
        // Each authored missile silhouette in assets/lathes/ must compile to a
        // positive-volume solid — they are referenced from the docs and loadable
        // via VALI_MODEL_VIM.
        for (name, src) in [
            ("ogive_srbm", include_str!("../../assets/lathes/ogive_srbm.vim")),
            ("slender_icbm_rv", include_str!("../../assets/lathes/slender_icbm_rv.vim")),
            ("blunt_boattail", include_str!("../../assets/lathes/blunt_boattail.vim")),
            ("math_sections", include_str!("../../assets/lathes/math_sections.vim")),
            ("math_curves", include_str!("../../assets/lathes/math_curves.vim")),
            ("loft_demo", include_str!("../../assets/lathes/loft_demo.vim")),
            ("sweep_demo", include_str!("../../assets/lathes/sweep_demo.vim")),
            ("harp_arm", include_str!("../../assets/lathes/harp_arm.vim")),
            ("harp_neck", include_str!("../../assets/lathes/harp_neck.vim")),
            ("helix_demo", include_str!("../../assets/lathes/helix_demo.vim")),
        ] {
            let v = compile(src)
                .unwrap_or_else(|e| panic!("{name}.vim should compile: {e}"))
                .solid
                .volume();
            assert!(v > 0.0, "{name}.vim volume {v}");
        }
    }

    #[test]
    fn bezier_rejects_bad_arity() {
        assert!(compile("model lathe(bezier(0,10, 1,9, 2,8), 32)").is_err()); // 6 coords, not 2+6N
    }

    #[test]
    fn difference_drills_a_hole() {
        let v = vol("let b = box(4, 4, 4)\nlet d = cylinder(r=1, h=6, seg=256)\nmodel b - d");
        assert!((v - (64.0 - PI * 4.0)).abs() < 0.1, "got {v}");
    }

    #[test]
    fn named_and_positional_agree() {
        let a = vol("model cylinder(r=2, h=5, seg=200)");
        let b = vol("model cylinder(2, 5, 200)");
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn limacon_section_extrudes_to_analytic_volume() {
        // Area enclosed by the limaçon r = b(c+cosθ) (simple loop, c ≥ 1) is
        // π·b²·(2c²+1)/2; extruding by h multiplies by h.
        let v = vol("model extrude(limacon(2, 10, 512), 5)");
        let want = PI * 100.0 * (2.0 * 4.0 + 1.0) / 2.0 * 5.0; // 2250π
        assert!((v - want).abs() < 5.0, "limacon extrude vol {v}, want {want}");
        // cardioid(b) is exactly the limaçon at c = 1.
        let a = vol("model extrude(cardioid(10, 512), 3)");
        let b = vol("model extrude(limacon(1, 10, 512), 3)");
        assert!((a - b).abs() < 1e-6, "cardioid should equal limacon c=1");
    }

    #[test]
    fn roundrect_and_slot_areas() {
        // roundrect area = w*h - (4-pi) r^2.
        let rr = vol("model extrude(roundrect(20, 10, 3, 64), 2)");
        let want_rr = (200.0 - (4.0 - PI) * 9.0) * 2.0;
        assert!((rr - want_rr).abs() < 0.5, "roundrect vol {rr}, want {want_rr}");
        // slot area = pi r^2 + 2 r l.
        let sl = vol("model extrude(slot(10, 3, 128), 2)");
        let want_sl = (PI * 9.0 + 2.0 * 3.0 * 10.0) * 2.0;
        assert!((sl - want_sl).abs() < 0.5, "slot vol {sl}, want {want_sl}");
    }

    #[test]
    fn twisted_extrude_preserves_volume() {
        // Twisting rotates each level rigidly, so the volume is ~unchanged from a
        // straight extrude (Cavalieri) apart from the faceting between levels
        // (flat side-quads chord the rotation, under-estimating a few percent).
        let straight = vol("model extrude(ngon(6, 10), 8)");
        let twisted = vol("model extrude(ngon(6, 10), 8, twist=180)");
        assert!(
            (twisted - straight).abs() < straight * 0.04,
            "twist {twisted} vs straight {straight}"
        );
        assert!(twisted > 0.0);
    }

    #[test]
    fn partial_revolve_caps_a_sector() {
        // A 2x2 square profile at r in [4,6] revolved 180deg -> half a rectangular
        // torus. Pappus: full = area*2pi*r_c = 4*2pi*5 = 40pi; half = 20pi.
        let v = vol("model revolve(polygon(4,-1, 6,-1, 6,1, 4,1), 200, 180)");
        assert!((v - 20.0 * PI).abs() < 0.4, "half ring vol {v}, want {}", 20.0 * PI);
        let f = vol("model revolve(polygon(4,-1, 6,-1, 6,1, 4,1), 200)");
        assert!((f - 40.0 * PI).abs() < 0.4, "full ring vol {f}");
    }

    #[test]
    fn ngon_and_tube() {
        // Hexagon (circumradius 10) area = (3*sqrt(3)/2) r^2; extrude by 2.
        let hex = vol("model extrude(ngon(6, 10), 2)");
        let want = 3.0 * 3.0f64.sqrt() / 2.0 * 100.0 * 2.0;
        assert!((hex - want).abs() < 1e-6, "hexagon prism vol {hex}, want {want}");
        // Tube volume = pi(R^2 - r^2) h.
        let t = vol("model tube(5, 3, 10, 256)");
        assert!((t - PI * (25.0 - 9.0) * 10.0).abs() < 0.5, "tube vol {t}");
    }

    #[test]
    fn svgpath_imports_a_profile() {
        // A 10x10 square via absolute commands -> extrude area 100 x h.
        let v = vol("model extrude(svgpath(\"M0,0 L10,0 L10,10 L0,10 Z\"), 5)");
        assert!((v - 500.0).abs() < 1e-6, "svg square vol {v}");
        // Relative (lowercase) commands give the same square.
        let r = vol("model extrude(svgpath(\"m 0 0 l 10 0 l 0 10 l -10 0 z\"), 5)");
        assert!((r - 500.0).abs() < 1e-6, "svg relative square vol {r}");
        // Unsupported commands surface a clear error.
        assert!(compile("model extrude(svgpath(\"M0,0 A5,5 0 0 1 10,10\"), 1)").is_err());
    }

    #[test]
    fn helix_sweeps_a_watertight_spring() {
        // A round section swept along a 2-turn helix -> a positive-volume spring.
        let v = vol("model helix(circle(1), 6, 8, 2)");
        assert!(v > 0.0, "helix volume {v}");
        // Roughly a tube of length = helical arc length; sanity upper/lower bounds.
        let turn_len = ((2.0 * PI * 6.0f64).powi(2) + 8.0f64.powi(2)).sqrt(); // per turn
        let approx = PI * 1.0 * 1.0 * turn_len * 2.0; // area * arc-length, 2 turns
        assert!(v > approx * 0.6 && v < approx * 1.4, "helix vol {v} vs ~{approx}");
    }

    #[test]
    fn array_polar_mirror_patterns() {
        // Linear array of 3 well-separated unit cubes = 3x the volume.
        let v = vol("model cube(2).arrayx(3, 10)");
        assert!((v - 3.0 * 8.0).abs() < 1e-6, "arrayx vol {v}");
        // Polar array of 4 disjoint boxes around Z = 4x the volume.
        let p = vol("model box(2,2,2).move(8,0,0).polar(4)");
        assert!((p - 4.0 * 8.0).abs() < 1e-4, "polar vol {p}");
        // Mirror-symmetrize a box offset in +x: two disjoint copies = 2x volume.
        let m = vol("model box(2,2,2).move(6,0,0).mirror(\"x\")");
        assert!((m - 2.0 * 8.0).abs() < 1e-4, "mirror vol {m}");
        // A bare reflection keeps the volume (still watertight, > 0).
        let r = vol("model box(2,3,4).move(5,0,0).reflect(\"x\")");
        assert!((r - 24.0).abs() < 1e-6, "reflect vol {r}");
    }

    #[test]
    fn rose_section_is_a_fluted_ring() {
        // Fluted near-circle with petal tips at radius 2b; its prism volume must
        // be positive and stay under the tip-circle prism.
        let v = vol("model extrude(rose(10), 4)"); // 12 flutes, amp 0.4, seg 160
        assert!(v > 0.0, "rose extrude vol {v}");
        let tip = PI * 20.0f64.powi(2) * 4.0; // (2b)^2·π·h
        assert!(v < tip, "rose vol {v} should be under tip-circle prism {tip}");
    }

    #[test]
    fn star_section_has_analytic_area() {
        // A regular n-pointed star of tip radius r_out and valley radius r_in has
        // area n·r_out·r_in·sin(π/n) (2n triangles); extruding scales it by h.
        let (n, r_out, r_in, h) = (5.0, 10.0, 4.0, 3.0);
        let v = vol("model extrude(star(5, 10, 4), 3)");
        let area = n * r_out * r_in * (PI / n).sin();
        assert!((v - area * h).abs() < 1e-6, "star extrude vol {v}, want {}", area * h);
    }

    #[test]
    fn superellipse_at_n2_is_an_ellipse() {
        // The superellipse degenerates to an ellipse at n = 2, area π·a·b.
        let (a, b, h) = (10.0, 6.0, 4.0);
        let v = vol("model extrude(superellipse(10, 6, 2, 1024), 4)");
        let want = PI * a * b * h;
        assert!((v - want).abs() < 1.0, "superellipse extrude vol {v}, want {want}");
    }

    #[test]
    fn petals_extrudes_to_positive_volume() {
        // The true rose r = a·cos(kθ) self-touches at the origin, which is fine as
        // a sketch: odd k → k petals, even k → 2k, both extrude to real flowers.
        let five = vol("model extrude(petals(10, 5), 2)"); // 5 petals
        assert!(five > 0.0, "5-petal rose vol {five}");
        let eight = vol("model extrude(petals(10, 4), 2)"); // 8 petals
        assert!(eight > 0.0, "8-petal rose vol {eight}");
    }

    #[test]
    fn hypotrochoid_closes_and_extrudes() {
        // The spirograph curve closes after an integer number of turns; the
        // resulting closed loop must extrude to a positive-volume solid.
        let v = vol("model extrude(hypotrochoid(10, 3, 5), 2)");
        assert!(v > 0.0, "hypotrochoid extrude vol {v}");
    }

    #[test]
    fn command_style_calls_parse() {
        // Parenless, Nim command style, plus a method chain.
        let v = vol("model cube 3 .move 0,0,0");
        assert!((v - 27.0).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn extrude_and_revolve() {
        let prism = vol("model extrude(rect(2, 3), 5)");
        assert!((prism - 30.0).abs() < 1e-6, "prism {prism}");
        let disk = vol("model revolve(circle(1, 128).move(3, 0), 256)");
        let torus = 2.0 * PI * PI * 3.0 * 1.0; // 2π²Rr²
        assert!((disk - torus).abs() < 0.1, "revolved {disk} vs {torus}");
    }

    #[test]
    fn fillet_and_chamfer_reduce_volume() {
        // Beveling a cube's edges removes material: 0 < result < original.
        let filleted = vol("model cube(2).fillet(0.4, 6)");
        assert!(filleted > 0.0 && filleted < 8.0, "fillet vol {filleted}");
        let chamfered = vol("model cube(2).chamfer(0.4)");
        assert!(chamfered > 0.0 && chamfered < 8.0, "chamfer vol {chamfered}");
    }

    // --- volumetrics on the polygon path ---------------------------------

    /// The polygon backend has no density representation. It must keep the
    /// solids, report exactly which fields it dropped, and succeed -- the
    /// caller turns that report into a stderr warning and exits 0.
    #[test]
    fn polygon_path_strips_volumetrics_and_keeps_the_solids() {
        let c = compile(
            "let ground = box(10, 10, 1)\n\
             let flame  = fire(1, 2).move(0, 0, 0.5)\n\
             let haze   = fog(10, 10, 1, seed = 4)\n\
             model ground + flame + haze",
        )
        .expect("volumetrics must not fail the polygon path");
        // the solids survive, unchanged: a 10x10x1 slab
        assert!((c.solid.volume() - 100.0).abs() < 1e-6, "vol {}", c.solid.volume());
        // ...and both fields are named in the report
        assert_eq!(c.skipped_volumetrics.len(), 2, "{:?}", c.skipped_volumetrics);
        assert!(c.skipped_volumetrics[0].starts_with("fire("), "{:?}", c.skipped_volumetrics);
        assert!(c.skipped_volumetrics[1].starts_with("fog("), "{:?}", c.skipped_volumetrics);
        // triangles exist and none of them came from a field
        assert!(!crate::triangles_of(&c.solid).is_empty());
    }

    /// A volumetric-free script reports nothing -- the warning never fires
    /// for the existing prop library.
    #[test]
    fn no_volumetrics_means_no_report() {
        let c = compile("model cube(2)").unwrap();
        assert!(c.skipped_volumetrics.is_empty());
    }

    #[test]
    fn unknown_name_errors() {
        let err = compile("model wibble(3)").unwrap_err();
        assert!(err.contains("wibble"), "message was: {err}");
    }

    fn params(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn eval_num_expr_folds_arithmetic() {
        let p = params(&[("bore", 8.0), ("wall", 12.0)]);
        assert_eq!(eval_num_expr("wall - bore", &p), Some(4.0));
        assert_eq!(eval_num_expr("bore * 2", &p), Some(16.0));
        assert_eq!(eval_num_expr("wall / bore", &p), Some(1.5));
        assert_eq!(eval_num_expr("-bore + wall", &p), Some(4.0));
        assert_eq!(eval_num_expr("(wall + bore) / 2", &p), Some(10.0));
    }

    #[test]
    fn eval_num_expr_none_on_unknown_or_bad() {
        let p = params(&[("bore", 8.0)]);
        // An unknown name has no value.
        assert_eq!(eval_num_expr("bore + gap", &p), None);
        // Division by zero is not a usable dimension.
        assert_eq!(eval_num_expr("bore / 0", &p), None);
        // A call is a solid/part binding, not a numeric relationship.
        assert_eq!(eval_num_expr("cylinder(bore, 40)", &p), None);
    }

    #[test]
    fn numeric_refs_distinguishes_relations_from_parts() {
        assert_eq!(
            numeric_refs("wall - bore"),
            Some(vec!["wall".to_string(), "bore".to_string()])
        );
        // Repeated name appears once, in source order.
        assert_eq!(
            numeric_refs("bore + bore * 2"),
            Some(vec!["bore".to_string()])
        );
        // A bare number references nothing.
        assert_eq!(numeric_refs("42"), Some(vec![]));
        // Calls and method chains are not numeric relationships.
        assert_eq!(numeric_refs("cylinder(8, 40)"), None);
        assert_eq!(numeric_refs("wall.move(0, 0, 5)"), None);
    }

    #[test]
    fn division_flows_through_dsl() {
        // `/` is a first-class numeric operator end-to-end.
        let v = vol("let a = 10\nlet b = a / 2\nmodel cube(b)");
        assert!((v - 125.0).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn loft_cylinder_and_frustum_through_dsl() {
        // Two equal circles skin to a cylinder: π r² h.
        let cyl = vol("model loft(circle(3, 256), circle(3, 256), h=10)");
        let want = PI * 9.0 * 10.0;
        assert!((cyl - want).abs() < want * 0.005, "loft cyl {cyl} vs {want}");
        // Unequal circles skin to a frustum: (π h/3)(r1²+r1r2+r2²).
        let fr = vol("model loft(circle(2, 256), circle(4, 256), h=9)");
        let wf = PI * 9.0 / 3.0 * 28.0;
        assert!((fr - wf).abs() < wf * 0.01, "loft frustum {fr} vs {wf}");
    }

    #[test]
    fn three_section_loft_through_dsl_is_positive() {
        let v = vol("model loft(circle(2, 128), circle(3, 128), circle(2, 128), h=10)");
        assert!(v > 0.0, "3-section loft vol {v}");
    }

    #[test]
    fn loft_needs_two_sketches() {
        let err = compile("model loft(circle(2, 64), h=10)").unwrap_err();
        assert!(err.contains("at least 2 sketches"), "message was: {err}");
    }

    #[test]
    fn sweep_straight_path_is_a_cylinder_through_dsl() {
        // A straight vertical bezier spine of length 10, read as (x, z): the swept
        // circle is a cylinder π r² L.
        let v = vol("model sweep(circle(2, 128), bezier(0,-5, 0,-5, 0,5, 0,5))");
        let want = PI * 4.0 * 10.0;
        assert!((v - want).abs() < want * 0.01, "swept cyl {v} vs {want}");
    }

    #[test]
    fn sweep_bent_path_is_positive() {
        // A curved bezier spine still closes into a watertight (positive) tube.
        let v = vol("model sweep(circle(1, 64), bezier(0,-8, 5,-3, 5,3, 0,8))");
        assert!(v > 0.0, "bent sweep vol {v}");
    }
}
