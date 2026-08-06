//! `strike_demo` — headless film exporter for the GRIPITT strike/CSG reel.
//! Renders the shared scene core (`gripitt_scene.rs`) to a 1024^2 mp4; an
//! optional second pass pads to 1600x1024 and burns a bullet panel + captions
//! (useful for a standalone marketing cut — the interactive booth app is
//! `bin/gripitt.rs`). Touches no shared darkair file.
//!
//! ```text
//! cargo run --release -p darkair --bin strike_demo -- [out.mp4] [fps]   # panel cut
//! cargo run -p darkair --bin strike_demo -- --clip out.mp4 [fps]        # pure 1024^2
//! cargo run -p darkair --bin strike_demo -- --still <t> out.png
//! ```

#[path = "../gripitt_scene.rs"]
mod scene;
use scene::*;

use da_render::{Gpu, Renderer};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// Prefer the system ffmpeg (libx264, real `-crf`) over an Anaconda shim.
fn ffbin() -> &'static str {
    if std::path::Path::new("/usr/bin/ffmpeg").exists() {
        "/usr/bin/ffmpeg"
    } else {
        "ffmpeg"
    }
}

fn render_still(gpu: &Gpu, renderer: &mut Renderer, t: f32, path: &str) {
    let segs = script();
    let (idx, tl) = locate(&segs, t.min(total_dur(&segs) - 0.01));
    let (list, cam, settings) = build(segs[idx].kind, tl, segs[idx].dur, (t * 30.0) as u32);
    for _ in 0..12 {
        renderer.render(gpu, &list, &cam, &settings, 0.1);
    }
    let rgba = renderer.read_rgba(gpu);
    image::save_buffer(path, &rgba, VIEW, VIEW, image::ColorType::Rgba8).expect("png");
    println!("wrote {path} (t={t:.1}s)");
}

/// Render every segment to a piped ffmpeg encoder at 1024^2. Returns the frame
/// count and segment start times (for the caption/panel pass).
fn render_scene(gpu: &Gpu, renderer: &mut Renderer, out: &str, fps: u32) -> u64 {
    let segs = script();
    let dt = 1.0 / fps as f32;
    let mut ff = Command::new(ffbin())
        .args([
            "-y", "-loglevel", "error",
            "-f", "rawvideo", "-pix_fmt", "rgba",
            "-s", &format!("{VIEW}x{VIEW}"),
            "-r", &fps.to_string(),
            "-i", "-",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
            out,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .expect("ffmpeg (installed?)");
    let mut pipe = ff.stdin.take().expect("ffmpeg stdin");
    let mut frame = 0u64;
    let total = total_dur(&segs);
    let mut done = 0.0f32;
    for s in &segs {
        let mut tl = 0.0f32;
        while tl < s.dur {
            let (list, cam, settings) = build(s.kind, tl, s.dur, frame as u32);
            renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, dt);
            let rgba = renderer.read_rgba_on(&gpu.device, &gpu.queue);
            pipe.write_all(&rgba).expect("pipe frame");
            tl += dt;
            frame += 1;
        }
        done += s.dur;
        eprintln!("strike: {done:.0}s / {total:.0}s");
    }
    drop(pipe);
    assert!(ff.wait().expect("ffmpeg wait").success(), "scene encode failed");
    frame
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gpu = Gpu::new_headless().expect("headless GPU");
    let mut renderer = Renderer::new(&gpu, VIEW, VIEW);

    if let Some(i) = args.iter().position(|a| a == "--still") {
        let t: f32 = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
        let path = args.get(i + 2).cloned().unwrap_or_else(|| "strike_still.png".into());
        render_still(&gpu, &mut renderer, t, &path);
        return;
    }

    // dump every frame as a PNG (for the booth app's pre-rendered playback)
    if let Some(i) = args.iter().position(|a| a == "--frames") {
        let dir = args.get(i + 1).cloned().unwrap_or_else(|| "gripitt_frames".into());
        let fps: u32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(10);
        std::fs::create_dir_all(&dir).expect("mkdir frames");
        let segs = script();
        let dt = 1.0 / fps as f32;
        let mut idx = 0u32;
        for s in &segs {
            let mut tl = 0.0f32;
            while tl < s.dur {
                let (list, cam, settings) = build(s.kind, tl, s.dur, idx);
                renderer.render_on(&gpu.device, &gpu.queue, &list, &cam, &settings, dt);
                let rgba = renderer.read_rgba_on(&gpu.device, &gpu.queue);
                image::save_buffer(
                    format!("{dir}/f{idx:05}.png"),
                    &rgba,
                    VIEW,
                    VIEW,
                    image::ColorType::Rgba8,
                )
                .expect("png");
                tl += dt;
                idx += 1;
            }
        }
        std::fs::write(format!("{dir}/fps.txt"), fps.to_string()).ok();
        println!("wrote {idx} frames to {dir} at {fps}fps");
        return;
    }

    // pure-1024^2 clip (for index.html's sample box), no panel
    if let Some(i) = args.iter().position(|a| a == "--clip") {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "gripitt_clip.mp4".into());
        let fps: u32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(24);
        let n = render_scene(&gpu, &mut renderer, &out, fps);
        println!("wrote {out} ({n} frames, 1024x1024)");
        return;
    }

    // default: scene + baked bullet panel + captions (standalone cut)
    let out = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "gripitt_strike.mp4".into());
    let fps: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(24);
    let tmp = format!("{out}.scene.mp4");
    render_scene(&gpu, &mut renderer, &tmp, fps);
    encode_panel(&tmp, &out);
}

/// Second pass: pad the square scene into 1600x1024 and burn the right-side
/// bullet panel + lower-third captions with ffmpeg drawtext.
fn encode_panel(scene_mp4: &str, out: &str) {
    let segs = script();
    let font = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ]
    .iter()
    .find(|p| std::path::Path::new(p).exists())
    .copied();
    let Some(font) = font else {
        std::fs::rename(scene_mp4, out).expect("rename");
        println!("wrote {out} (panel skipped: no DejaVu font)");
        return;
    };
    let esc = |s: &str| {
        s.replace('\\', "\\\\").replace(':', "\\:").replace('\'', "").replace('%', "")
    };
    let px = VIEW + 34;
    let mut chain = format!("[0:v]pad={FRAME_W}:{VIEW}:0:0:0x0d1017,");
    chain.push_str(&format!("drawbox=x={VIEW}:y=0:w=4:h={VIEW}:color=0x4ea3ff@0.6:t=fill,"));
    let mut g = 0.0f32;
    let mut parts: Vec<String> = Vec::new();
    for s in &segs {
        let (start, end) = (g, g + s.dur);
        parts.push(format!(
            "drawtext=fontfile={font}:text='{}':x={px}:y=70:fontsize=34:fontcolor=0x4ea3ff:\
             enable='between(t,{start:.2},{end:.2})'",
            esc(s.title)
        ));
        for (bi, b) in s.bullets.iter().enumerate() {
            let y = 150 + bi * 74;
            parts.push(format!(
                "drawtext=fontfile={font}:text='- {}':x={px}:y={y}:fontsize=24:fontcolor=0xD7E2EE:\
                 enable='between(t,{start:.2},{end:.2})'",
                esc(b)
            ));
        }
        for (cs, ce, text) in s.caps {
            let (a, bnd) = (g + cs, g + ce);
            parts.push(format!(
                "drawtext=fontfile={font}:text='{}':x=({VIEW}-text_w)/2:y=930:fontsize=28:\
                 fontcolor=0xCCFFCC:box=1:boxcolor=0x000000AA:boxborderw=10:\
                 enable='between(t,{a:.2},{bnd:.2})'",
                esc(text)
            ));
        }
        g = end;
    }
    chain.push_str(&parts.join(","));
    chain.push_str("[v]");
    let filter_file = format!("{out}.filters");
    std::fs::write(&filter_file, &chain).expect("filter script");
    let ok = Command::new(ffbin())
        .args([
            "-y", "-loglevel", "error",
            "-i", scene_mp4,
            "-filter_complex_script", &filter_file,
            "-map", "[v]",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
            out,
        ])
        .status()
        .expect("ffmpeg pass 2")
        .success();
    if ok {
        let _ = std::fs::remove_file(scene_mp4);
        let _ = std::fs::remove_file(&filter_file);
        println!("wrote {out}");
    } else {
        std::fs::rename(scene_mp4, out).expect("rename");
        println!("wrote {out} WITHOUT panel (drawtext failed; filters at {filter_file})");
    }
}
