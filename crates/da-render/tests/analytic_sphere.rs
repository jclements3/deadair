//! Is a sphere a sphere?
//!
//! The GPU path used to render `Shape::Sphere` as `unit_sphere(14, 20)` -- a
//! polyhedron. Its silhouette was a 20-gon and no amount of normal smoothing
//! changed that, because the silhouette is decided by the geometry, not the
//! shading. These tests hold the analytic replacement honest by measuring the
//! rendered silhouette against a circle, at a zoom where a 20-gon could not
//! hide.
//!
//! Method: render a single sphere dead-centre, threshold the coverage mask,
//! and for every boundary pixel measure its distance from the centroid. A
//! true circle gives a constant radius; a polyhedron's radius oscillates
//! between its inradius and circumradius once per facet.

use da_render::{Camera, DrawItem, DrawList, Gpu, OpticMode, OpticSettings, Renderer, Shape};
use glam::{Mat4, Vec3};

const W: u32 = 512;
const H: u32 = 512;

fn sphere_scene(radius: f32) -> DrawList {
    DrawList {
        items: vec![DrawItem {
            shape: Shape::Sphere { radius },
            world: Mat4::IDENTITY,
            albedo: [0.85, 0.85, 0.85],
            emissive: 0.0,
            temp_f: 120.0,
            glass: false,
            coat_f: 0.0,
        }],
        ambient_f: 60.0,
        sky_temp_f: 18.0,
        moonlight: 1.0,
        heat_decals: vec![],
        eyeshine: vec![],
    }
}

/// Camera looking at the origin from `dist`, with a FOV tight enough that the
/// sphere fills most of the frame -- the zoom where faceting shows.
fn close_cam(dist: f32, fov_y_deg: f32) -> Camera {
    Camera {
        eye: Vec3::new(0.0, 0.0, dist),
        look: Vec3::ZERO,
        up: Vec3::Y,
        fov_y_deg,
        aspect: W as f32 / H as f32,
    }
}

fn render(list: &DrawList, cam: &Camera) -> Option<Vec<u8>> {
    // Headless GPU is not available in every environment (no lavapipe in a
    // minimal container). Skip rather than fail: a missing GPU is not a
    // regression in sphere geometry.
    let gpu = Gpu::new_headless().ok()?;
    let mut r = Renderer::new(&gpu, W, H);
    let settings = OpticSettings {
        mode: OpticMode::Thermal,
        scope_mask: false,
        ..Default::default()
    };
    for _ in 0..3 {
        r.render(&gpu, list, cam, &settings, 0.1);
    }
    Some(r.read_rgba(&gpu))
}

/// Coverage mask: the sphere is hot against cold sky, so luminance separates
/// them cleanly in the thermal view.
fn mask(rgba: &[u8]) -> Vec<bool> {
    let lo = rgba.chunks(4).map(|p| p[0] as u32).min().unwrap_or(0);
    let hi = rgba.chunks(4).map(|p| p[0] as u32).max().unwrap_or(255);
    let mid = (lo + hi) / 2;
    rgba.chunks(4).map(|p| p[0] as u32 > mid).collect()
}

/// Boundary pixels and their radii about the coverage centroid.
fn edge_radii(m: &[bool]) -> Vec<f64> {
    let at = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
            return false;
        }
        m[(y as usize) * W as usize + x as usize]
    };
    let (mut sx, mut sy, mut n) = (0f64, 0f64, 0f64);
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if at(x, y) {
                sx += x as f64;
                sy += y as f64;
                n += 1.0;
            }
        }
    }
    assert!(n > 1000.0, "sphere covered only {n} px -- framing is wrong");
    let (cx, cy) = (sx / n, sy / n);

    let mut radii = Vec::new();
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if !at(x, y) {
                continue;
            }
            // 4-neighbour boundary test
            if at(x - 1, y) && at(x + 1, y) && at(x, y - 1) && at(x, y + 1) {
                continue;
            }
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            radii.push((dx * dx + dy * dy).sqrt());
        }
    }
    radii
}

/// The headline test. At this zoom a 20-slice polyhedron has a radius that
/// swings by ~1.2% of its radius between facet centre and vertex; an
/// analytic sphere's boundary radius is constant to within the width of the
/// boundary band itself.
#[test]
fn silhouette_is_circular_at_high_zoom() {
    let list = sphere_scene(1.0);
    // 6 deg vertical FOV at 12 units: the sphere spans most of the frame.
    let Some(rgba) = render(&list, &close_cam(12.0, 10.0)) else {
        eprintln!("no headless GPU; skipping");
        return;
    };
    let radii = edge_radii(&mask(&rgba));
    let n = radii.len() as f64;
    let mean = radii.iter().sum::<f64>() / n;
    let var = radii.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
    let sd = var.sqrt();
    let rel = sd / mean;
    eprintln!(
        "edge pixels {} | mean radius {:.2} px | sd {:.3} px | relative {:.5}",
        radii.len(),
        mean,
        sd,
        rel
    );
    // A boundary band is ~1 px thick, so sd cannot go to zero; but a facetted
    // silhouette at this zoom would sit far above this bound.
    assert!(
        rel < 0.010,
        "silhouette radius varies by {:.3}% of radius -- that is faceting, not a circle",
        100.0 * rel
    );
}

/// Roundness must not depend on how close the camera is. A tessellated
/// sphere gets visibly worse as it fills more of the frame, because the
/// facet count is fixed; an analytic one does not change at all.
#[test]
fn roundness_holds_as_the_sphere_fills_the_frame() {
    let list = sphere_scene(1.0);
    let mut worst = 0.0f64;
    for (dist, fov) in [(40.0f32, 30.0f32), (12.0, 10.0), (6.0, 20.0)] {
        let Some(rgba) = render(&list, &close_cam(dist, fov)) else {
            eprintln!("no headless GPU; skipping");
            return;
        };
        let radii = edge_radii(&mask(&rgba));
        let n = radii.len() as f64;
        let mean = radii.iter().sum::<f64>() / n;
        let sd = (radii.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n).sqrt();
        let rel = sd / mean;
        eprintln!("dist {dist:>5} fov {fov:>5} -> mean r {mean:7.2} px, relative sd {rel:.5}");
        worst = worst.max(rel);
    }
    assert!(
        worst < 0.012,
        "roundness degrades with zoom (worst {:.3}%) -- the silhouette is tessellated",
        100.0 * worst
    );
}

/// Spheres must still occlude and be occluded correctly. The analytic pass
/// writes its own frag_depth; if that were wrong, a sphere behind a box
/// would punch through it.
#[test]
fn analytic_sphere_depth_sorts_against_a_box() {
    let mut list = sphere_scene(1.0);
    // Box between the camera (at +z) and the sphere at the origin.
    list.items.push(DrawItem {
        shape: Shape::Box { half: Vec3::new(3.0, 3.0, 0.2) },
        world: Mat4::from_translation(Vec3::new(0.0, 0.0, 4.0)),
        albedo: [0.2, 0.2, 0.2],
        temp_f: 20.0,
        emissive: 0.0,
        glass: false,
        coat_f: 0.0,
    });
    let Some(occluded) = render(&list, &close_cam(12.0, 10.0)) else {
        eprintln!("no headless GPU; skipping");
        return;
    };
    // Compare against the same scene WITHOUT the occluder. Asserting an
    // absolute brightness would pass trivially (an all-black frame satisfies
    // "not bright"); the meaningful claim is that interposing a box
    // CHANGES the pixels the sphere occupied.
    let bare = render(&sphere_scene(1.0), &close_cam(12.0, 10.0)).expect("second render");

    let sphere_px = mask(&bare);
    let mut differing = 0usize;
    let mut total = 0usize;
    for (i, &covered) in sphere_px.iter().enumerate() {
        if !covered {
            continue;
        }
        total += 1;
        if bare[i * 4..i * 4 + 3] != occluded[i * 4..i * 4 + 3] {
            differing += 1;
        }
    }
    assert!(total > 1000, "no sphere coverage to test against");
    let frac = differing as f64 / total as f64;
    eprintln!("sphere pixels {total}, changed by occluder {differing} ({:.1}%)", 100.0 * frac);
    assert!(
        frac > 0.95,
        "occluder changed only {:.1}% of the sphere's pixels -- the analytic \
         sphere is ignoring depth and drawing through solid geometry",
        100.0 * frac
    );
}
