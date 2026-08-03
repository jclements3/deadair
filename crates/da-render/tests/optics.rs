//! Headless golden tests for the three optic pipelines (SDD §4).
//!
//! Runs on the llvmpipe Vulkan adapter under WSL2 — no display needed.
//! Renders a minimal truth: ground, a warm pest sphere, an ambient-temp
//! zombie box, a glass pane — then asserts each optic filters it correctly.
//! PNGs land in target/optics-test/ for eyeballing against
//! assets/reference/optics-look.md.

use da_render::{
    draw::{Camera, DrawItem, DrawList, EyeShine, HeatDecal, Shape},
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
        eyeshine: vec![],
    }
}

/// Mean luminance of a small patch — beats NV grain when sampling a region.
fn patch_lum(frame: &[u8], cx: u32, cy: u32, r: u32) -> f32 {
    let mut sum = 0.0;
    let mut n = 0.0f32;
    for y in cy.saturating_sub(r)..(cy + r + 1).min(H) {
        for x in cx.saturating_sub(r)..(cx + r + 1).min(W) {
            sum += lum(px(frame, x, y));
            n += 1.0;
        }
    }
    sum / n.max(1.0)
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

/// SDD §2.3 / FR-T4: residual heat is *temperature*, so the thermal channel
/// must show it and the light-path channels must not know it exists.
#[test]
fn heat_decals_are_thermal_only() {
    let Some(gpu) = shared_gpu() else { return };
    let mut r = Renderer::new(gpu, W, H);
    let camera = cam();
    let bed = Vec3::new(0.0, 0.0, -10.0);
    let plain = scene();
    let mut hot = scene();
    hot.heat_decals.push(HeatDecal {
        pos: bed,
        radius_m: 1.5,
        delta_f: 25.0,
    });

    let bed_px = to_px(&camera, bed);
    let bare_px = to_px(&camera, Vec3::new(-5.0, 0.0, -9.0));

    let mut settings = OpticSettings {
        mode: OpticMode::Thermal,
        palette: ThermalPalette::WhiteHot,
        ..Default::default()
    };
    for _ in 0..30 {
        r.render(&gpu, &hot, &camera, &settings, 0.1);
    }
    let thermal = r.read_rgba(&gpu);
    save(&thermal, "thermal_heat_decal.png");

    let t_bed = lum(px(&thermal, bed_px.0, bed_px.1));
    let t_bare = lum(px(&thermal, bare_px.0, bare_px.1));
    assert!(
        t_bed > t_bare + 40.0,
        "bedding decal must read hotter than the ground around it: \
         bed={t_bed} bare={t_bare}"
    );

    // Same AGC history, no decals: the bed pixel falls back to ground.
    let mut r2 = Renderer::new(gpu, W, H);
    for _ in 0..30 {
        r2.render(&gpu, &plain, &camera, &settings, 0.1);
    }
    let cold = r2.read_rgba(&gpu);
    let c_bed = lum(px(&cold, bed_px.0, bed_px.1));
    assert!(
        t_bed > c_bed + 40.0,
        "decal must warm its own pixel: with={t_bed} without={c_bed}"
    );

    // Light-path channels: byte-identical with and without decals.
    for mode in [OpticMode::Nv, OpticMode::Eye] {
        settings.mode = mode;
        r.render(&gpu, &hot, &camera, &settings, 0.016);
        let with = r.read_rgba(&gpu);
        r.render(&gpu, &plain, &camera, &settings, 0.016);
        let without = r.read_rgba(&gpu);
        assert_eq!(
            with, without,
            "heat decals must be invisible in {mode:?} — they are heat, not light"
        );
    }
    save(&r.read_rgba(&gpu), "nv_no_heat_decal.png");
}

/// The standout detail of the IR-illuminated NV footage: retro-reflecting
/// eyes as brilliant dots. NV-only, and never emitted for zombies.
#[test]
fn eyeshine_is_nv_only() {
    let Some(gpu) = shared_gpu() else { return };
    let mut r = Renderer::new(gpu, W, H);
    let camera = cam();
    let eyes = Vec3::new(-3.0, 0.95, -12.0); // pest's head, facing the shooter
    let plain = scene();
    let mut lit = scene();
    lit.eyeshine.push(EyeShine {
        pos: eyes,
        strength: 1.0,
    });
    let eye_px = to_px(&camera, eyes);

    let mut settings = OpticSettings {
        mode: OpticMode::Nv,
        ..Default::default()
    };
    r.render(&gpu, &lit, &camera, &settings, 0.016);
    let nv = r.read_rgba(&gpu);
    save(&nv, "nv_eyeshine.png");
    let n_eye = lum(px(&nv, eye_px.0, eye_px.1));
    assert!(
        n_eye > 235.0,
        "eyeshine must be near-saturated in NV: {n_eye}"
    );
    // ...and it halos, via the emissive bloom path.
    let halo = patch_lum(&nv, eye_px.0, eye_px.1 + 8, 2);
    r.render(&gpu, &plain, &camera, &settings, 0.016);
    let nv_plain = r.read_rgba(&gpu);
    let halo_plain = patch_lum(&nv_plain, eye_px.0, eye_px.1 + 8, 2);
    assert!(
        halo > halo_plain + 6.0,
        "eyeshine must bloom past its own dot: {halo} vs {halo_plain}"
    );

    // Thermal and naked eye must be untouched: no eyeshine channel at all.
    for mode in [OpticMode::Thermal, OpticMode::Eye] {
        settings.mode = mode;
        let mut a = Renderer::new(gpu, W, H);
        let mut b = Renderer::new(gpu, W, H);
        for _ in 0..20 {
            a.render(&gpu, &lit, &camera, &settings, 0.1);
            b.render(&gpu, &plain, &camera, &settings, 0.1);
        }
        assert_eq!(
            a.read_rgba(&gpu),
            b.read_rgba(&gpu),
            "eyeshine must not exist in {mode:?}"
        );
        if mode == OpticMode::Thermal {
            save(&a.read_rgba(&gpu), "thermal_no_eyeshine.png");
        }
    }
}

/// Emissive sources halo in NV and to the dark-adapted eye — a light through
/// a real objective is never a hard-edged shape.
#[test]
fn emissive_sources_bloom_beyond_their_silhouette() {
    let Some(gpu) = shared_gpu() else { return };
    let camera = cam();
    let lamp = Vec3::new(0.0, 3.0, -14.0);
    let lamp_px = to_px(&camera, lamp);

    let build = |emissive: f32| {
        let mut s = scene();
        s.items.push(DrawItem {
            shape: Shape::Sphere { radius: 0.3 },
            world: Mat4::from_translation(lamp),
            albedo: [0.09, 0.085, 0.07],
            emissive,
            temp_f: 55.0,
            glass: false,
        });
        s
    };
    let lit = build(1.0);
    let dark = build(0.0);

    for (mode, name, margin) in [
        (OpticMode::Nv, "nv_bloom.png", 8.0f32),
        (OpticMode::Eye, "eye_bloom.png", 4.0),
    ] {
        let settings = OpticSettings {
            mode,
            ..Default::default()
        };
        let mut a = Renderer::new(gpu, W, H);
        let mut b = Renderer::new(gpu, W, H);
        a.render(&gpu, &lit, &camera, &settings, 0.016);
        b.render(&gpu, &dark, &camera, &settings, 0.016);
        let on = a.read_rgba(&gpu);
        let off = b.read_rgba(&gpu);
        save(&on, name);

        // The lamp's own pixels are brighter (that is just emissive)...
        assert!(
            lum(px(&on, lamp_px.0, lamp_px.1)) > lum(px(&off, lamp_px.0, lamp_px.1)),
            "{mode:?}: emissive lamp must be brighter than an unlit one"
        );
        // ...and so is a patch well outside its ~6 px silhouette, which only
        // the bloom pass can reach.
        let halo_on = patch_lum(&on, lamp_px.0 + 12, lamp_px.1, 2);
        let halo_off = patch_lum(&off, lamp_px.0 + 12, lamp_px.1, 2);
        assert!(
            halo_on > halo_off + margin,
            "{mode:?}: bloom must extend past the silhouette: {halo_on} vs {halo_off}"
        );
    }
}

/// The AGC windows a coverage-weighted percentile, not min/max: a speck of
/// something very cold must not flatten the whole frame.
#[test]
fn percentile_agc_ignores_a_tiny_cold_speck() {
    let Some(gpu) = shared_gpu() else { return };
    let camera = cam();
    let zombie_px = to_px(&camera, Vec3::new(3.0, 0.9, -12.0));
    let ground_px = to_px(&camera, Vec3::new(0.0, 0.0, -10.0));

    let plain = scene();
    let mut speck = scene();
    speck.items.push(DrawItem {
        shape: Shape::Sphere { radius: 0.06 },
        world: Mat4::from_translation(Vec3::new(1.2, 1.0, -5.0)),
        albedo: [0.2, 0.2, 0.2],
        emissive: 0.0,
        temp_f: -40.0, // a frost-cold scrap, a few pixels across
        glass: false,
    });

    let settings = OpticSettings {
        mode: OpticMode::Thermal,
        palette: ThermalPalette::WhiteHot,
        ..Default::default()
    };
    let contrast = |list: &da_render::DrawList| {
        let mut r = Renderer::new(gpu, W, H);
        for _ in 0..30 {
            r.render(&gpu, list, &camera, &settings, 0.1);
        }
        let f = r.read_rgba(&gpu);
        (
            (lum(px(&f, zombie_px.0, zombie_px.1)) - lum(px(&f, ground_px.0, ground_px.1)))
                .abs(),
            f,
        )
    };
    let (base, _) = contrast(&plain);
    let (with_speck, frame) = contrast(&speck);
    save(&frame, "thermal_cold_speck.png");
    assert!(base > 4.0, "baseline ground contrast must exist: {base}");
    assert!(
        with_speck > base * 0.8,
        "a tiny cold speck must not compress ground contrast: \
         {with_speck} vs {base}"
    );

    // A min/max window would have done exactly that.
    let mut naive = da_render::Agc::new();
    for _ in 0..60 {
        naive.update(-40.0, 101.0, 0.1);
    }
    let mut fair = da_render::Agc::new();
    for _ in 0..60 {
        fair.update(5.0, 101.0, 0.1);
    }
    let squeeze = (naive.normalize(48.0) - naive.normalize(45.0))
        / (fair.normalize(48.0) - fair.normalize(45.0));
    assert!(
        squeeze < 0.75,
        "sanity: min/max really is the compressing one ({squeeze})"
    );
}

