//! The cylinder is a cylinder, not a 20-sided prism.
//!
//! `mesh::unit_cylinder(20)` is a prism: seen end-on its silhouette is a
//! 20-gon whose radius swings between facet centre and vertex, and seen from
//! the side its wall shows flat strips. The analytic pipeline keeps that
//! prism as proxy geometry only and solves the true capped cylinder per
//! pixel, so neither tell survives.
//!
//! The end-on view is the sharp test: a 20-gon's inscribed radius is
//! cos(pi/20) = 0.9877 of its circumscribed radius, a 1.2% swing that dwarfs
//! the ~1 px boundary band these measurements bottom out at.

use da_render::{Camera, DrawItem, DrawList, Gpu, OpticMode, OpticSettings, Renderer, Shape};
use glam::{Mat4, Vec3};

const W: u32 = 512;
const H: u32 = 512;

fn cylinder_scene(radius: f32, height: f32, world: Mat4) -> DrawList {
    DrawList {
        items: vec![DrawItem {
            shape: Shape::Cylinder { radius, height },
            world,
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

fn cam(eye: Vec3, look: Vec3, fov_y_deg: f32) -> Camera {
    Camera { eye, look, up: Vec3::Y, fov_y_deg, aspect: W as f32 / H as f32 }
}

fn render(list: &DrawList, c: &Camera) -> Option<Vec<u8>> {
    // No headless GPU in a minimal container: skip rather than fail, exactly
    // as the sphere test does. A missing GPU is not a geometry regression.
    let gpu = Gpu::new_headless().ok()?;
    let mut r = Renderer::new(&gpu, W, H);
    let settings =
        OpticSettings { mode: OpticMode::Thermal, scope_mask: false, ..Default::default() };
    for _ in 0..3 {
        r.render(&gpu, list, c, &settings, 0.1);
    }
    Some(r.read_rgba(&gpu))
}

fn mask(rgba: &[u8]) -> Vec<bool> {
    let lo = rgba.chunks(4).map(|p| p[0] as u32).min().unwrap_or(0);
    let hi = rgba.chunks(4).map(|p| p[0] as u32).max().unwrap_or(255);
    let mid = (lo + hi) / 2;
    rgba.chunks(4).map(|p| p[0] as u32 > mid).collect()
}

fn at(m: &[bool], x: i32, y: i32) -> bool {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
        return false;
    }
    m[(y as usize) * W as usize + x as usize]
}

fn centroid(m: &[bool]) -> (f64, f64, f64) {
    let (mut sx, mut sy, mut n) = (0f64, 0f64, 0f64);
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if at(m, x, y) {
                sx += x as f64;
                sy += y as f64;
                n += 1.0;
            }
        }
    }
    (sx / n.max(1.0), sy / n.max(1.0), n)
}

fn edge_radii(m: &[bool]) -> Vec<f64> {
    let (cx, cy, n) = centroid(m);
    assert!(n > 1000.0, "cylinder covered only {n} px -- framing is wrong");
    // If the shape reaches the border the "silhouette" is partly the frame,
    // and the radius spread measures the viewport instead of the geometry.
    let touches = (0..W as i32).any(|x| at(m, x, 0) || at(m, x, H as i32 - 1))
        || (0..H as i32).any(|y| at(m, 0, y) || at(m, W as i32 - 1, y));
    assert!(!touches, "silhouette touches the frame border -- widen the FOV");
    let mut radii = Vec::new();
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if !at(m, x, y) {
                continue;
            }
            if at(m, x - 1, y) && at(m, x + 1, y) && at(m, x, y - 1) && at(m, x, y + 1) {
                continue;
            }
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            radii.push((dx * dx + dy * dy).sqrt());
        }
    }
    radii
}

/// End-on: the cap disc's boundary must be a circle. This is where a 20-gon
/// is most obvious.
#[test]
fn cap_silhouette_is_circular_end_on() {
    // Cylinder along Y, camera above looking straight down the axis.
    let list = cylinder_scene(1.0, 3.0, Mat4::IDENTITY);
    let Some(rgba) = render(&list, &cam(Vec3::new(0.0, 12.0, 0.0001), Vec3::new(0.0, 3.0, 0.0), 14.0))
    else {
        eprintln!("no headless GPU; skipping");
        return;
    };
    let radii = edge_radii(&mask(&rgba));
    let n = radii.len() as f64;
    let mean = radii.iter().sum::<f64>() / n;
    let sd = (radii.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n).sqrt();
    let rel = sd / mean;
    eprintln!("cap: edge {} | mean r {:.2} px | sd {:.3} px | relative {:.5}", radii.len(), mean, sd, rel);
    assert!(
        rel < 0.012,
        "cap silhouette radius varies by {:.3}% -- that is a polygon, not a circle",
        100.0 * rel
    );
}

/// Side-on: the wall's left and right edges must be straight and parallel.
/// A prism seen side-on has a silhouette that steps as facets rotate past
/// the limb; measure the per-row half-width and require it to be constant.
#[test]
fn side_silhouette_edges_are_straight() {
    let list = cylinder_scene(1.0, 4.0, Mat4::IDENTITY);
    let Some(rgba) = render(&list, &cam(Vec3::new(0.0, 2.0, 7.0), Vec3::new(0.0, 2.0, 0.0), 26.0))
    else {
        eprintln!("no headless GPU; skipping");
        return;
    };
    let m = mask(&rgba);
    let mut widths = Vec::new();
    for y in 0..H as i32 {
        let row: Vec<i32> = (0..W as i32).filter(|&x| at(&m, x, y)).collect();
        // only rows fully inside the body, away from the cap ellipses
        if row.len() > 40 {
            widths.push((row[row.len() - 1] - row[0]) as f64);
        }
    }
    assert!(widths.len() > 50, "only {} usable rows -- framing is wrong", widths.len());
    let n = widths.len() as f64;
    let mean = widths.iter().sum::<f64>() / n;
    let sd = (widths.iter().map(|w| (w - mean).powi(2)).sum::<f64>() / n).sqrt();
    let rel = sd / mean;
    eprintln!("side: rows {} | mean width {:.2} px | sd {:.3} px | relative {:.5}", widths.len(), mean, sd, rel);
    assert!(rel < 0.01, "wall width varies by {:.3}% down the body -- facets", 100.0 * rel);
}

/// Roundness must not degrade as the cylinder fills the frame. A prism gets
/// visibly worse with zoom because its facet count is fixed.
#[test]
fn cap_roundness_holds_across_zoom() {
    let list = cylinder_scene(1.0, 3.0, Mat4::IDENTITY);
    let mut worst = 0.0f64;
    for (dist, fov) in [(24.0f32, 6.0f32), (9.0, 14.0), (5.5, 24.0)] {
        let Some(rgba) = render(
            &list,
            &cam(Vec3::new(0.0, dist + 3.0, 0.0001), Vec3::new(0.0, 3.0, 0.0), fov),
        ) else {
            eprintln!("no headless GPU; skipping");
            return;
        };
        let radii = edge_radii(&mask(&rgba));
        let n = radii.len() as f64;
        let mean = radii.iter().sum::<f64>() / n;
        let sd = (radii.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n).sqrt();
        eprintln!("dist {dist:5} fov {fov:5} -> mean r {:7.2} px, relative sd {:.5}", mean, sd / mean);
        worst = worst.max(sd / mean);
    }
    assert!(worst < 0.012, "worst relative sd {:.5} across zoom", worst);
}

/// A rotated cylinder must stay correct: the solve works in world space off
/// the instance transform, so an off-axis body is the same problem. Also
/// guards the axis/height extraction from the model matrix.
#[test]
fn rotated_cylinder_still_solves() {
    let world = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_3)
        * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0));
    let list = cylinder_scene(0.8, 4.0, world);
    let Some(rgba) = render(&list, &cam(Vec3::new(0.0, 1.5, 9.0), Vec3::new(0.0, 1.5, 0.0), 30.0))
    else {
        eprintln!("no headless GPU; skipping");
        return;
    };
    let m = mask(&rgba);
    let (_, _, n) = centroid(&m);
    eprintln!("rotated cylinder covers {n} px");
    assert!(n > 2000.0, "rotated cylinder nearly vanished ({n} px) -- axis extraction is wrong");
}

/// Depth must come from the true hit, not the proxy prism, or a cylinder
/// would sort wrongly against neighbouring geometry.
#[test]
fn cylinder_depth_sorts_against_a_box() {
    let mut list = cylinder_scene(1.2, 3.0, Mat4::IDENTITY);
    let c = cam(Vec3::new(0.0, 1.5, 8.0), Vec3::new(0.0, 1.5, 0.0), 30.0);
    let Some(before) = render(&list, &c) else {
        eprintln!("no headless GPU; skipping");
        return;
    };
    // A cold slab in front of the cylinder must occlude it.
    list.items.push(DrawItem {
        shape: Shape::Box { half: Vec3::new(3.0, 3.0, 0.1) },
        world: Mat4::from_translation(Vec3::new(0.0, 1.5, 3.0)),
        albedo: [0.1, 0.1, 0.1],
        emissive: 0.0,
        temp_f: 20.0,
        glass: false,
        coat_f: 0.0,
    });
    let Some(after) = render(&list, &c) else { return };
    let (m0, m1) = (mask(&before), mask(&after));
    let cyl_px = m0.iter().filter(|&&b| b).count();
    let changed = m0.iter().zip(&m1).filter(|(&a, &b)| a && !b).count();
    let frac = changed as f64 / cyl_px.max(1) as f64;
    eprintln!("cylinder pixels {cyl_px}, changed by occluder {changed} ({:.1}%)", 100.0 * frac);
    assert!(frac > 0.95, "occluder only changed {:.1}% -- depth is coming from the proxy", 100.0 * frac);
}
