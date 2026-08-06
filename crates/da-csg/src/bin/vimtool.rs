//! vimtool — compile a `.vim` CSG script and emit drawings or meshes.
//!
//!   vimtool input.vim --svg out.svg [--iso] [--section [OFFSET]] [--section-view front|top|right]
//!   vimtool input.vim --stl out.stl [--ascii]
//!   vimtool input.vim --obj out.obj
//!
//! `--svg` renders the ISO 128 first-angle multiview (Front/Top/Right) with
//! hidden-line removal; `--iso` appends an isometric pane and `--section` a
//! hatched cross-section pane (cut plane perpendicular to the section view's
//! depth axis at OFFSET, default 0). `--stl` writes binary STL (`--ascii` for
//! ASCII STL); `--obj` writes Wavefront OBJ. Output flags may be combined.
//! Meshes are in the `.vim` script's native Z-up metres. On a compile error the
//! DSL's message goes to stderr and the exit code is nonzero.

use da_csg::drawing;
use da_csg::export;

struct Args {
    input: String,
    svg: Option<String>,
    stl: Option<String>,
    obj: Option<String>,
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
         \x20                         [--stl FILE [--ascii]] [--obj FILE]"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut a = Args {
        input: String::new(),
        svg: None,
        stl: None,
        obj: None,
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
    if a.svg.is_none() && a.stl.is_none() && a.obj.is_none() {
        eprintln!("no output requested (--svg / --stl / --obj)");
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
    let compiled = match da_csg::compile_vim(&source) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let solid = &compiled.solid;

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
