//! Headless golden tests for the three optic pipelines (SDD §4).
//!
//! Runs on the llvmpipe Vulkan adapter under WSL2 — no display needed.
//! Renders a minimal truth: ground, a warm pest sphere, an ambient-temp
//! zombie box, a glass pane — then asserts each optic filters it correctly.
//! PNGs land in target/optics-test/ for eyeballing against
//! assets/reference/optics-look.md.

use da_render::{
    draw::{Camera, DrawItem, DrawList, Shape},
    gpu::Gpu,
    renderer::{OpticMode, OpticSettings, Renderer},
    ThermalPalette,
};
use glam::{Mat4, Vec3};

const W: u32 = 320;
const H: u32 = 240;

/// One shared GPU for the whole test process. llvmpipe's Vulkan teardown is
/// fragile (segfaults on device drop); a static is never dropped, so we
/// sidestep it — and tests stop racing each other for instances.
fn shared_gpu() -> Option<&'static Gpu> {
    use std::sync::OnceLock;
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| match Gpu::new_headless() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("SKIP: no GPU adapter available: {e}");
            None
        }
    })
    .as_ref()
}

fn scene() -> DrawList {
    let ambient = 48.0;
    DrawList {
        items: vec![
            // Ground: slightly below ambient (radiative night).
            DrawItem {
                shape: Shape::GroundPatch { half: 60.0 },
                world: Mat4::IDENTITY,
                albedo: [0.25, 0.3, 0.2],
                emissive: 0.0,
                temp_f: ambient - 3.0,
                glass: false,
            },
            // Pest: warm sphere at screen left.
            DrawItem {
                shape: Shape::Sphere { radius: 0.5 },
                world: Mat4::from_translation(Vec3::new(-3.0, 0.5, -12.0)),
                albedo: [0.35, 0.3, 0.25],
                emissive: 0.0,
                temp_f: 101.0,
                glass: false,
            },
            // Zombie: ambient-temperature box at screen right. Same temp as
            // air; pallid skin reflects near-IR strongly (bright in NV).
            DrawItem {
                shape: Shape::Box {
                    half: Vec3::new(0.4, 0.9, 0.3),
                },
                world: Mat4::from_translation(Vec3::new(3.0, 0.9, -12.0)),
                albedo: [0.68, 0.66, 0.62],
                emissive: 0.0,
                temp_f: ambient,
                glass: false,
            },
        ],
        ambient_f: ambient,
        sky_temp_f: 5.0,
        moonlight: 0.5,
        heat_decals: vec![],
    }
}

fn cam() -> Camera {
    Camera {
        eye: Vec3::new(0.0, 1.6, 0.0),
        look: Vec3::new(0.0, 0.8, -12.0),
        up: Vec3::Y,
        fov_y_deg: 50.0,
        aspect: W as f32 / H as f32,
    }
}

/// Project a world point to pixel coordinates.
fn to_px(cam: &Camera, p: Vec3) -> (u32, u32) {
    let clip = cam.view_proj() * p.extend(1.0);
    let ndc = clip / clip.w;
    (
        ((ndc.x * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - ndc.y * 0.5) * H as f32) as u32,
    )
}

fn px(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

fn lum(c: [u8; 3]) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}

fn save(frame: &[u8], name: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("optics-test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    image::save_buffer(
        dir.join(name),
        frame,
        W,
        H,
        image::ColorType::Rgba8,
    )
    .expect("png save");
}

#[test]
fn three_optics_filter_one_truth() {
    let Some(gpu) = shared_gpu() else { return };
    let mut r = Renderer::new(gpu, W, H);
    let list = scene();
    let camera = cam();

    let pest_px = to_px(&camera, Vec3::new(-3.0, 0.5, -12.0));
    let zombie_px = to_px(&camera, Vec3::new(3.0, 0.9, -12.0));
    let ground_px = to_px(&camera, Vec3::new(0.0, 0.0, -10.0));
    let sky_px = (W / 2, 4u32);

    // Let the AGC settle, then render each optic.
    let mut settings = OpticSettings {
        mode: OpticMode::Thermal,
        palette: ThermalPalette::WhiteHot,
        ..Default::default()
    };
    for _ in 0..30 {
        r.render(&gpu, &list, &camera, &settings, 0.1);
    }
    let thermal = r.read_rgba(&gpu);
    save(&thermal, "thermal.png");

    settings.mode = OpticMode::Nv;
    r.render(&gpu, &list, &camera, &settings, 0.016);
    let nv = r.read_rgba(&gpu);
    save(&nv, "nv.png");

    settings.mode = OpticMode::Eye;
    r.render(&gpu, &list, &camera, &settings, 0.016);
    let eye = r.read_rgba(&gpu);
    save(&eye, "eye.png");

    // THERMAL: pest blazes above ground; zombie indistinguishable from
    // background at the same range (SDD §4.1 — invisibility is emergent).
    let t_pest = lum(px(&thermal, pest_px.0, pest_px.1));
    let t_zombie = lum(px(&thermal, zombie_px.0, zombie_px.1));
    let t_ground = lum(px(&thermal, ground_px.0, ground_px.1));
    let t_sky = lum(px(&thermal, sky_px.0, sky_px.1));
    assert!(
        t_pest > t_ground + 80.0,
        "pest must glow in thermal: pest={t_pest} ground={t_ground}"
    );
    assert!(
        (t_zombie - t_ground).abs() < 25.0,
        "zombie must blend with ground in thermal: zombie={t_zombie} ground={t_ground}"
    );
    assert!(
        t_sky < t_ground,
        "sky must be the cold floor: sky={t_sky} ground={t_ground}"
    );

    // NV: zombie clearly separable from ground (different albedo), because
    // NV sees geometry, not temperature.
    let n_zombie = lum(px(&nv, zombie_px.0, zombie_px.1));
    let n_ground = lum(px(&nv, ground_px.0, ground_px.1));
    assert!(
        (n_zombie - n_ground).abs() > 12.0,
        "zombie must be visible in NV: zombie={n_zombie} ground={n_ground}"
    );

    // NV is brighter than naked eye (that's the point of the tube).
    let n_scene = lum(px(&nv, ground_px.0, ground_px.1));
    let e_scene = lum(px(&eye, ground_px.0, ground_px.1));
    assert!(
        n_scene > e_scene * 1.5,
        "NV must amplify: nv={n_scene} eye={e_scene}"
    );

    // NV grain: same pixel differs between consecutive frames.
    settings.mode = OpticMode::Nv;
    settings.frame = 1;
    r.render(&gpu, &list, &camera, &settings, 0.016);
    let nv2 = r.read_rgba(&gpu);
    let mut diffs = 0u32;
    for y in (0..H).step_by(7) {
        for x in (0..W).step_by(7) {
            if px(&nv, x, y) != px(&nv2, x, y) {
                diffs += 1;
            }
        }
    }
    assert!(diffs > 100, "NV grain must animate between frames: {diffs}");
}

#[test]
fn black_hot_inverts_and_scope_mask_darkens_corners() {
    let Some(gpu) = shared_gpu() else { return };
    let mut r = Renderer::new(gpu, W, H);
    let list = scene();
    let camera = cam();
    let pest_px = to_px(&camera, Vec3::new(-3.0, 0.5, -12.0));

    let mut settings = OpticSettings {
        mode: OpticMode::Thermal,
        palette: ThermalPalette::BlackHot,
        scope_mask: true,
        ..Default::default()
    };
    for _ in 0..30 {
        r.render(&gpu, &list, &camera, &settings, 0.1);
    }
    let bh = r.read_rgba(&gpu);
    save(&bh, "thermal_blackhot_masked.png");

    // Hot pest reads DARK in black-hot (like the hog footage).
    let t_pest = lum(px(&bh, pest_px.0, pest_px.1));
    let t_corner = lum(px(&bh, 2, 2));
    assert!(t_pest < 60.0, "pest must be dark in black-hot: {t_pest}");
    // Circular mask blacks out the corners.
    assert!(t_corner < 8.0, "scope mask must darken corners: {t_corner}");

    // White-hot for comparison: same pixel bright.
    settings.palette = ThermalPalette::WhiteHot;
    r.render(&gpu, &list, &camera, &settings, 0.016);
    let wh = r.read_rgba(&gpu);
    let w_pest = lum(px(&wh, pest_px.0, pest_px.1));
    assert!(
        w_pest > 180.0,
        "pest must be bright in white-hot: {w_pest}"
    );
}
