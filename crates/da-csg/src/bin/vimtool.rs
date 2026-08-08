//! vimtool — compile a `.vim` CSG script and emit drawings or meshes.
//!
//!   vimtool input.vim --svg out.svg [--iso] [--section [OFFSET]] [--section-view front|top|right]
//!   vimtool input.vim --stl out.stl [--ascii]
//!   vimtool input.vim --obj out.obj
//!   vimtool input.vim --volume [SAMPLES]
//!
//! `--svg` renders the ISO 128 first-angle multiview (Front/Top/Right) with
//! hidden-line removal; `--iso` appends an isometric pane and `--section` a
//! hatched cross-section pane (cut plane perpendicular to the section view's
//! depth axis at OFFSET, default 0). `--stl` writes binary STL (`--ascii` for
//! ASCII STL); `--obj` writes Wavefront OBJ. `--volume` prints a BRL-CAD-
//! style analysis report on stdout — AABB plus the deterministic R2-sampled
//! volume of the analytic SDF (95% CI), with the BSP mesh volume as
//! reference when the mesh backend also compiles (backlog B1). Output flags
//! may be combined.
//! Meshes are in the `.vim` script's native Z-up metres. On a compile error the
//! DSL's message goes to stderr and the exit code is nonzero.

use da_csg::drawing;
use da_csg::export;

struct Args {
    input: String,
    svg: Option<String>,
    stl: Option<String>,
    obj: Option<String>,
    sdf: Option<String>,
    volume: bool,
    volume_samples: u64,
    iso: bool,
    section: bool,
    section_at: f64,
    section_view: drawing::ViewDir,
    ascii: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: vimtool input.vim [--svg FILE [--iso] [--section [OFFSET]]\n\
         \x20                          [--section-view front|top|right]]\n\
         \x20                         [--stl FILE [--ascii]] [--obj FILE] [--sdf FILE]\n\
         \x20                         [--volume [SAMPLES]]"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut a = Args {
        input: String::new(),
        svg: None,
        stl: None,
        obj: None,
        sdf: None,
        volume: false,
        volume_samples: 200_000,
        iso: false,
        section: false,
        section_at: 0.0,
        section_view: drawing::ViewDir::Front,
        ascii: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--svg" => {
                i += 1;
                a.svg = Some(argv.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--svg needs a file");
                    usage()
                }));
            }
            "--stl" => {
                i += 1;
                a.stl = Some(argv.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--stl needs a file");
                    usage()
                }));
            }
            "--obj" => {
                i += 1;
                a.obj = Some(argv.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--obj needs a file");
                    usage()
                }));
            }
            "--sdf" => {
                i += 1;
                a.sdf = Some(argv.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--sdf needs a file");
                    usage()
                }));
            }
            "--volume" => {
                a.volume = true;
                // Optional positional SAMPLES (an integer) right after --volume.
                if let Some(next) = argv.get(i + 1) {
                    if let Ok(v) = next.parse::<u64>() {
                        if v == 0 {
                            eprintln!("--volume needs at least 1 sample");
                            usage()
                        }
                        a.volume_samples = v;
                        i += 1;
                    }
                }
            }
            "--iso" => a.iso = true,
            "--section" => {
                a.section = true;
                // Optional positional OFFSET (a number) right after --section.
                if let Some(next) = argv.get(i + 1) {
                    if let Ok(v) = next.parse::<f64>() {
                        a.section_at = v;
                        i += 1;
                    }
                }
            }
            "--section-view" => {
                i += 1;
                a.section_view = match argv.get(i).map(String::as_str) {
                    Some("front") => drawing::ViewDir::Front,
                    Some("top") => drawing::ViewDir::Top,
                    Some("right") => drawing::ViewDir::Right,
                    other => {
                        eprintln!(
                            "--section-view must be front|top|right, got {}",
                            other.unwrap_or("nothing")
                        );
                        usage()
                    }
                };
            }
            "--ascii" => a.ascii = true,
            "--help" | "-h" => usage(),
            other if other.starts_with('-') => {
                eprintln!("unknown argument: {other} (try --help)");
                std::process::exit(2);
            }
            other => {
                if !a.input.is_empty() {
                    eprintln!("more than one input file: {} and {other}", a.input);
                    usage()
                }
                a.input = other.to_string();
            }
        }
        i += 1;
    }
    if a.input.is_empty() {
        eprintln!("no input .vim file given");
        usage()
    }
    if a.svg.is_none() && a.stl.is_none() && a.obj.is_none() && a.sdf.is_none() && !a.volume {
        eprintln!("no output requested (--svg / --stl / --obj / --sdf / --volume)");
        usage()
    }
    a
}

fn main() {
    let a = parse_args();

    let source = match std::fs::read_to_string(&a.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {e}", a.input);
            std::process::exit(1);
        }
    };
    if a.volume {
        // B1: BRL-CAD-style analysis report on the analytic backend —
        // AABB + deterministic R2-sampled volume with a 95% CI, plus the
        // BSP mesh volume as cross-backend reference when it compiles.
        let field = match da_csg::dsl::sdf_analysis::compile_sdf_field(&source) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        let rep = match da_csg::dsl::sdf_analysis::volume_report(&field, a.volume_samples) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        let stem = std::path::Path::new(&a.input)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bsp = da_csg::compile_vim(&source).map(|c| c.solid.volume());
        print!("{}", volume_report_text(&stem, &rep, &bsp));
        if a.sdf.is_none() && a.svg.is_none() && a.stl.is_none() && a.obj.is_none() {
            return;
        }
    }

    if let Some(path) = &a.sdf {
        // Analytic SDF export: triangle-free osgedit model JSON. Independent
        // of the mesh path — a sphere here is the equation of a sphere.
        let stem = std::path::Path::new(&a.input)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match da_csg::sdf_model_json(&stem, &source) {
            Ok(file) => {
                write_out(path, serde_json::to_string_pretty(&file).unwrap().as_bytes());
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        if a.svg.is_none() && a.stl.is_none() && a.obj.is_none() {
            return;
        }
    }

    let compiled = match da_csg::compile_vim(&source) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let solid = &compiled.solid;
    // The polygon kernel meshes surfaces; fire/fog/cloud are density fields
    // with no triangles to give it. Report exactly what was dropped and carry
    // on with the solids -- a drawing of a campfire is still a drawing.
    if !compiled.skipped_volumetrics.is_empty() {
        eprintln!(
            "warning: {} skipped on the polygon path (volumetrics have no mesh): {}",
            compiled.skipped_volumetrics.len(),
            compiled.skipped_volumetrics.join(", ")
        );
        eprintln!("         export with --sdf to keep them (they render as a post-pass)");
    }

    if let Some(path) = &a.svg {
        let mut dwg = drawing::multiview(solid);
        if a.section {
            dwg.views
                .push(drawing::section(solid, a.section_view, a.section_at));
        }
        if a.iso {
            dwg.views.push(drawing::isometric(solid));
        }
        let svg = drawing::to_svg(&dwg);
        write_out(path, svg.as_bytes());
    }
    if let Some(path) = &a.stl {
        if a.ascii {
            write_out(path, export::stl_ascii(solid).as_bytes());
        } else {
            write_out(path, &export::stl_binary(solid));
        }
    }
    if let Some(path) = &a.obj {
        write_out(path, export::obj(solid).as_bytes());
    }
}

fn write_out(path: &str, bytes: &[u8]) {
    if let Err(e) = std::fs::write(path, bytes) {
        eprintln!("cannot write {path}: {e}");
        std::process::exit(1);
    }
    eprintln!("wrote {path} ({} bytes)", bytes.len());
}

/// Render the `--volume` report (B1). Pure text-from-values so the format
/// is unit-testable; `bsp` is the mesh backend's signed volume, or its
/// compile error when the script has no mesh path.
fn volume_report_text(
    stem: &str,
    rep: &da_csg::dsl::sdf_analysis::VolumeReport,
    bsp: &Result<f64, String>,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{stem} — analytic volume report ({} R2 samples, deterministic)",
        rep.samples
    );
    let (mn, mx) = (rep.aabb.min, rep.aabb.max);
    let _ = writeln!(
        out,
        "  aabb (Y-up, m): [{:.3}, {:.3}, {:.3}] .. [{:.3}, {:.3}, {:.3}]",
        mn.x, mn.y, mn.z, mx.x, mx.y, mx.z
    );
    let _ = writeln!(
        out,
        "  sdf volume: {:12.3} m^3  +/- {:.3} (95% CI)",
        rep.volume, rep.ci95
    );
    match bsp {
        Ok(v) => {
            let _ = writeln!(out, "  bsp volume: {v:12.3} m^3  (mesh reference)");
            let delta = (v - rep.volume).abs();
            let pct = if *v != 0.0 { 100.0 * delta / v.abs() } else { 0.0 };
            let _ = writeln!(out, "  delta:      {delta:12.3} m^3  ({pct:.2}% of bsp)");
        }
        Err(e) => {
            let first = e.lines().next().unwrap_or("");
            let _ = writeln!(out, "  bsp volume: unavailable — {first}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::volume_report_text;
    use da_csg::dsl::sdf_analysis::{compile_sdf_field, volume_report};

    #[test]
    fn volume_report_text_carries_the_numbers() {
        let f = compile_sdf_field("model cube(2)").unwrap();
        let rep = volume_report(&f, 10_000).unwrap();
        let text = volume_report_text("cube", &rep, &Ok(7.917));
        assert!(text.contains("cube — analytic volume report (10000 R2 samples"), "{text}");
        assert!(text.contains("aabb (Y-up, m): [-1.000, -1.000, -1.000] .. [1.000, 1.000, 1.000]"), "{text}");
        assert!(text.contains("sdf volume:        8.000 m^3  +/- 0.000"), "{text}");
        assert!(text.contains("bsp volume:        7.917 m^3"), "{text}");
        assert!(text.contains("delta:             0.083 m^3  (1.05% of bsp)"), "{text}");
    }

    #[test]
    fn volume_report_text_survives_a_meshless_script() {
        let f = compile_sdf_field("model sphere(1)").unwrap();
        let rep = volume_report(&f, 1_000).unwrap();
        let text = volume_report_text("s", &rep, &Err("no mesh path\nsecond line".into()));
        assert!(text.contains("bsp volume: unavailable — no mesh path"), "{text}");
        assert!(!text.contains("second line"), "{text}");
    }
}
