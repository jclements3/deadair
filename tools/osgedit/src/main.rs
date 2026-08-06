//! osgedit — camera-path editor/renderer for the OSG (CSG) conference models.
//!
//! Edit `campath.json` (keyframes: time, eye, look, fov, label), then:
//!   osgedit --keyframes            list the keyframes
//!   osgedit --still 6.5            render one frame at t=6.5 s to a PNG
//!   osgedit --stills               render a PNG at every keyframe
//!   osgedit --preview              fast draft movie (small, low spp)
//!   osgedit                        full-quality 1024x1024 animated WebP
//!
//! Anti-shimmer: 16x stratified supersampling with a frame-invariant jitter
//! pattern, gamma-correct filtering, distance-scaled epsilon, distance fog.

mod campath;
mod render;
mod scene;
mod sdf;

use campath::PathSpec;
use std::path::Path;
use std::time::Instant;
use webp_animation::{Encoder, EncoderOptions, EncodingConfig, EncodingType, LossyEncodingConfig};

struct Args {
    path: String,
    out: Option<String>,
    models: String,
    preview: bool,
    still: Option<f32>,
    stills: bool,
    keyframes: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        path: "campath.json".into(),
        out: None,
        models: "models".into(),
        preview: false,
        still: None,
        stills: false,
        keyframes: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--path" => a.path = it.next().expect("--path needs a file"),
            "--out" => a.out = Some(it.next().expect("--out needs a file")),
            "--models" => a.models = it.next().expect("--models needs a dir"),
            "--preview" => a.preview = true,
            "--still" => {
                a.still = Some(
                    it.next()
                        .expect("--still needs a time in seconds")
                        .parse()
                        .expect("--still time must be a number"),
                )
            }
            "--stills" => a.stills = true,
            "--keyframes" => a.keyframes = true,
            "--help" | "-h" => {
                eprintln!(
                    "usage: osgedit [--path campath.json] [--models models] [--out FILE]\n\
                     \x20              [--preview] [--still SECONDS] [--stills] [--keyframes]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other} (try --help)");
                std::process::exit(2);
            }
        }
    }
    a
}

fn save_png(name: &str, rgba: &[u8], size: u32) {
    image::save_buffer(name, rgba, size, size, image::ColorType::Rgba8).unwrap();
    eprintln!("wrote {name}");
}

fn main() {
    let args = parse_args();
    let mut spec = PathSpec::load(&args.path);

    if args.keyframes {
        println!("{:>6}  {:<12} {:>24} {:>24}  fov", "t (s)", "label", "eye", "look");
        for k in &spec.keys {
            println!(
                "{:>6.2}  {:<12} {:>24} {:>24}  {}",
                k.t,
                k.label,
                format!("{:.1},{:.1},{:.1}", k.eye[0], k.eye[1], k.eye[2]),
                format!("{:.1},{:.1},{:.1}", k.look[0], k.look[1], k.look[2]),
                k.fov_deg.map(|f| format!("{f:.0}")).unwrap_or_else(|| "-".into()),
            );
        }
        return;
    }

    let t0 = Instant::now();
    let world = scene::build(Path::new(&args.models));
    eprintln!("scene: {} objects ({:.2}s)", world.objects.len(), t0.elapsed().as_secs_f32());

    if args.preview {
        spec.size = 360;
        spec.supersample = 2;
        spec.fps = spec.fps.min(10.0);
        spec.quality = 78.0;
    }
    let size = spec.size as usize;
    let ss = spec.supersample as usize;

    // single still
    if let Some(t) = args.still {
        let cam = spec.sample(spec.keys[0].t + t);
        let rgba = render::render_frame(&world, &cam, size, ss);
        let name = args.out.clone().unwrap_or_else(|| format!("still_{t:.2}s.png"));
        save_png(&name, &rgba, spec.size);
        return;
    }

    // one still per keyframe (composition check while editing the path)
    if args.stills {
        for (i, k) in spec.keys.clone().iter().enumerate() {
            let cam = spec.sample(k.t);
            let rgba = render::render_frame(&world, &cam, size, ss);
            let label = if k.label.is_empty() { format!("key{i}") } else { k.label.clone() };
            save_png(&format!("still_{i:02}_{label}.png"), &rgba, spec.size);
        }
        return;
    }

    // the movie
    let dur = spec.duration();
    let frames = (dur * spec.fps).round() as usize + 1;
    eprintln!(
        "rendering {frames} frames, {size}x{size}, {}x{} samples/pixel, {:.1}s @ {} fps",
        ss, ss, dur, spec.fps
    );

    let cfg = EncodingConfig {
        encoding_type: EncodingType::Lossy(LossyEncodingConfig::default()),
        quality: spec.quality,
        method: 4,
    };
    let opts = EncoderOptions { encoding_config: Some(cfg), ..Default::default() };
    let mut enc = Encoder::new_with_options((spec.size, spec.size), opts).unwrap();

    let start = Instant::now();
    for f in 0..frames {
        let t = spec.keys[0].t + f as f32 / spec.fps;
        let cam = spec.sample(t);
        let rgba = render::render_frame(&world, &cam, size, ss);
        let ts = (f as f64 * 1000.0 / spec.fps as f64) as i32;
        enc.add_frame(&rgba, ts).unwrap();
        let el = start.elapsed().as_secs_f32();
        eprintln!(
            "frame {:>3}/{frames}  t={t:>5.2}s  ({:.1}s elapsed, ~{:.0}s left)",
            f + 1,
            el,
            el / (f + 1) as f32 * (frames - f - 1) as f32
        );
    }
    // hold the last frame briefly before the loop restarts
    let final_ts = ((frames as f64 + spec.fps as f64 * 0.5) * 1000.0 / spec.fps as f64) as i32;
    let data = enc.finalize(final_ts).unwrap();
    let out = args.out.unwrap_or_else(|| "osgshow.webp".into());
    std::fs::write(&out, &data).unwrap();
    eprintln!("wrote {out} ({} bytes, {:.1}s total)", data.len(), start.elapsed().as_secs_f32());
}
