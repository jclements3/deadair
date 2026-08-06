//! Viewport picking and the screen-space translate gizmo.
//!
//! All of the maths lives here so it can be tested without a window:
//!
//! - [`ray_from_screen`] turns a click inside the viewport image into a
//!   world-space ray through the [`Camera`].
//! - [`pick_node`] walks the scene's world bounding spheres and returns the
//!   nearest node the ray hits.
//! - [`project_point`] projects world → viewport pixels (y down, egui
//!   convention), returning `None` for points behind the camera.
//! - [`axis_drag_delta`] converts a pixel drag into a distance along a world
//!   axis, which the editor feeds to [`da_graph::Scene::set_translation`].
//!
//! The gizmo is drawn as a 2D overlay with the egui painter; there is no
//! 3D gizmo geometry in the render pass.

use da_core::NodeId;
use da_graph::{NodeKind, Scene};
use da_render::Camera;
use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};

/// The three translate axes, in draw order, with their handle colours.
pub const AXES: [(Vec3, [u8; 3]); 3] = [
    (Vec3::X, [230, 70, 70]),
    (Vec3::Y, [90, 210, 90]),
    (Vec3::Z, [90, 130, 240]),
];

/// A world-space ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    /// Ray start (the camera eye).
    pub origin: Vec3,
    /// Normalized direction.
    pub dir: Vec3,
}

impl Ray {
    /// Point at distance `t` along the ray.
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

/// Convert a pixel position inside a viewport image of `size` pixels into
/// normalized device coordinates (`x` right, `y` up, both `-1..=1`).
pub fn pixel_to_ndc(pos: Vec2, size: Vec2) -> Vec2 {
    let w = if size.x.abs() < 1e-6 { 1.0 } else { size.x };
    let h = if size.y.abs() < 1e-6 { 1.0 } else { size.y };
    Vec2::new(pos.x / w * 2.0 - 1.0, 1.0 - pos.y / h * 2.0)
}

/// Build the world-space ray through pixel `pos` of a `size`-pixel image.
///
/// `pos` is relative to the image's top-left corner, y down.
pub fn ray_from_screen(cam: &Camera, pos: Vec2, size: Vec2) -> Ray {
    let ndc = pixel_to_ndc(pos, size);
    let inv = cam.view_proj().inverse();
    // wgpu clip space is z ∈ [0, 1]: 0 = near, 1 = far.
    let near = inv * glam::Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
    let far = inv * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
    let np = if near.w.abs() > 1e-9 {
        near.xyz() / near.w
    } else {
        cam.eye
    };
    let fp = if far.w.abs() > 1e-9 {
        far.xyz() / far.w
    } else {
        np + (cam.look - cam.eye)
    };
    Ray {
        origin: np,
        dir: (fp - np).normalize_or_zero(),
    }
}

/// Distance along `ray` to the nearest intersection with the sphere, or
/// `None` when the ray misses. A ray starting inside the sphere hits at 0.
pub fn ray_sphere(ray: &Ray, center: Vec3, radius: f32) -> Option<f32> {
    if radius <= 0.0 {
        return None;
    }
    let m = ray.origin - center;
    let b = m.dot(ray.dir);
    let c = m.length_squared() - radius * radius;
    if c > 0.0 && b > 0.0 {
        return None; // pointing away from the sphere
    }
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    Some(t.max(0.0))
}

/// The nearest *drawable* node (a geode with at least one drawable) whose
/// world bounding sphere `ray` hits.
pub fn pick_node(scene: &Scene, ray: &Ray) -> Option<NodeId> {
    let mut best: Option<(f32, NodeId)> = None;
    for node in scene.nodes() {
        let drawable = match node.kind() {
            NodeKind::Geode(g) => !g.drawables.is_empty(),
            _ => false,
        };
        if !drawable {
            continue;
        }
        let bound = scene.world_bound(node.id());
        if bound.is_empty() {
            continue;
        }
        if let Some(t) = ray_sphere(ray, bound.center, bound.radius) {
            let better = match best {
                Some((bt, _)) => t < bt,
                None => true,
            };
            if better {
                best = Some((t, node.id()));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// Pick from a click at pixel `pos` in a `size`-pixel viewport image.
pub fn pick_at(scene: &Scene, cam: &Camera, pos: Vec2, size: Vec2) -> Option<NodeId> {
    pick_node(scene, &ray_from_screen(cam, pos, size))
}

/// Project a world point into viewport pixels (origin top-left, y down).
/// Returns `None` for points at or behind the camera plane.
pub fn project_point(view_proj: &Mat4, p: Vec3, size: Vec2) -> Option<Vec2> {
    let clip = *view_proj * p.extend(1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.xyz() / clip.w;
    Some(Vec2::new(
        (ndc.x * 0.5 + 0.5) * size.x,
        (0.5 - ndc.y * 0.5) * size.y,
    ))
}

/// Screen-space direction (pixels, y down) of a unit world axis at `origin`,
/// scaled by the gizmo's world `length`. `None` when either endpoint is
/// behind the camera.
pub fn axis_screen_dir(
    view_proj: &Mat4,
    origin: Vec3,
    axis: Vec3,
    length: f32,
    size: Vec2,
) -> Option<Vec2> {
    let a = project_point(view_proj, origin, size)?;
    let b = project_point(view_proj, origin + axis.normalize_or_zero() * length, size)?;
    Some(b - a)
}

/// Distance in *world units* along `axis` corresponding to a pixel drag.
///
/// The drag is projected onto the axis' screen-space direction: a drag
/// exactly along the handle moves the node by the full world length the
/// handle represents.
pub fn axis_drag_delta(
    view_proj: &Mat4,
    origin: Vec3,
    axis: Vec3,
    length: f32,
    size: Vec2,
    drag_px: Vec2,
) -> f32 {
    let Some(dir) = axis_screen_dir(view_proj, origin, axis, length, size) else {
        return 0.0;
    };
    let len_sq = dir.length_squared();
    if len_sq < 1e-6 {
        return 0.0; // axis points at the camera: no usable screen direction
    }
    drag_px.dot(dir) / len_sq * length
}

/// Distance in pixels from `p` to the segment `a`→`b`.
pub fn point_segment_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-9 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Index into [`AXES`] of the handle within `tolerance` pixels of `pos`,
/// nearest first — i.e. which axis a click at `pos` grabs.
pub fn nearest_axis(
    view_proj: &Mat4,
    origin: Vec3,
    length: f32,
    size: Vec2,
    pos: Vec2,
    tolerance: f32,
) -> Option<usize> {
    let a = project_point(view_proj, origin, size)?;
    let mut best: Option<(f32, usize)> = None;
    for (i, (axis, _)) in AXES.iter().enumerate() {
        let Some(b) = project_point(view_proj, origin + *axis * length, size) else {
            continue;
        };
        let d = point_segment_dist(pos, a, b);
        if d > tolerance {
            continue;
        }
        let better = match best {
            Some((bd, _)) => d < bd,
            None => true,
        };
        if better {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// A sensible gizmo handle length in world units for a node of the given
/// world bounding radius, viewed from `dist` away.
pub fn handle_length(bound_radius: f32, dist: f32) -> f32 {
    (bound_radius.max(0.5) * 1.6).clamp(0.5, dist * 0.25)
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_graph::{Drawable, Shape};

    const SIZE: Vec2 = Vec2::new(640.0, 360.0);

    fn cam() -> Camera {
        Camera {
            eye: Vec3::new(0.0, 0.0, 20.0),
            look: Vec3::ZERO,
            up: Vec3::Y,
            fov_y_deg: 55.0,
            aspect: SIZE.x / SIZE.y,
        }
    }

    #[test]
    fn centre_pixel_maps_to_ndc_origin() {
        let ndc = pixel_to_ndc(SIZE * 0.5, SIZE);
        assert!(ndc.length() < 1e-6);
        // Top-left is (-1, +1) in NDC, bottom-right (+1, -1).
        assert_eq!(pixel_to_ndc(Vec2::ZERO, SIZE), Vec2::new(-1.0, 1.0));
        assert_eq!(pixel_to_ndc(SIZE, SIZE), Vec2::new(1.0, -1.0));
    }

    #[test]
    fn centre_ray_points_down_the_view_axis() {
        let ray = ray_from_screen(&cam(), SIZE * 0.5, SIZE);
        assert!(ray.dir.abs_diff_eq(Vec3::NEG_Z, 1e-4), "{:?}", ray.dir);
        // The near-plane origin sits just in front of the eye.
        assert!((ray.origin - Vec3::new(0.0, 0.0, 20.0)).length() < 0.2);
    }

    #[test]
    fn ray_sphere_hits_and_misses() {
        let ray = Ray {
            origin: Vec3::new(0.0, 0.0, 10.0),
            dir: Vec3::NEG_Z,
        };
        let t = ray_sphere(&ray, Vec3::ZERO, 2.0).expect("hit");
        assert!((t - 8.0).abs() < 1e-4, "front surface at 8: {t}");
        assert!(ray_sphere(&ray, Vec3::new(5.0, 0.0, 0.0), 2.0).is_none(), "off to the side");
        assert!(ray_sphere(&ray, Vec3::new(0.0, 0.0, 30.0), 2.0).is_none(), "behind the ray");
        // Starting inside the sphere hits immediately.
        assert_eq!(ray_sphere(&ray, Vec3::new(0.0, 0.0, 10.0), 3.0), Some(0.0));
        assert!(ray_sphere(&ray, Vec3::ZERO, 0.0).is_none(), "degenerate sphere");
    }

    fn scene_with_two_spheres() -> (Scene, NodeId, NodeId) {
        let mut scene = Scene::new();
        let root = scene.root();
        let near_xf = scene
            .add_transform_at(root, Vec3::new(0.0, 0.0, 5.0))
            .expect("xf");
        let near = scene.add_geode(near_xf).expect("geode");
        scene
            .add_drawable(near, Drawable::new(Shape::Sphere { radius: 1.0 }))
            .expect("drawable");
        let far_xf = scene
            .add_transform_at(root, Vec3::new(0.0, 0.0, -5.0))
            .expect("xf");
        let far = scene.add_geode(far_xf).expect("geode");
        scene
            .add_drawable(far, Drawable::new(Shape::Sphere { radius: 1.0 }))
            .expect("drawable");
        (scene, near, far)
    }

    #[test]
    fn picking_returns_the_nearest_hit() {
        let (scene, near, _far) = scene_with_two_spheres();
        let hit = pick_at(&scene, &cam(), SIZE * 0.5, SIZE);
        assert_eq!(hit, Some(near), "the sphere closest to the eye wins");
    }

    #[test]
    fn picking_the_far_sphere_when_the_near_one_is_not_under_the_cursor() {
        let (mut scene, near, far) = scene_with_two_spheres();
        // Shove the near sphere far off to the side.
        scene
            .set_translation(scene.node(near).unwrap().parent().unwrap(), Vec3::new(60.0, 0.0, 5.0))
            .expect("move");
        assert_eq!(pick_at(&scene, &cam(), SIZE * 0.5, SIZE), Some(far));
    }

    #[test]
    fn picking_empty_space_misses() {
        let (scene, _, _) = scene_with_two_spheres();
        // Top-left corner of the frame: nothing there.
        assert_eq!(pick_at(&scene, &cam(), Vec2::new(2.0, 2.0), SIZE), None);
    }

    #[test]
    fn non_drawable_nodes_are_not_pickable() {
        let mut scene = Scene::new();
        scene.add_group(scene.root()).expect("group");
        scene.add_geode(scene.root()).expect("empty geode");
        assert_eq!(pick_at(&scene, &cam(), SIZE * 0.5, SIZE), None);
    }

    #[test]
    fn projection_round_trips_a_known_point() {
        let cam = cam();
        let vp = cam.view_proj();
        // The look target lands dead centre.
        let centre = project_point(&vp, Vec3::ZERO, SIZE).expect("in front");
        assert!((centre - SIZE * 0.5).length() < 1e-3, "{centre:?}");

        // A point up and to the right projects up (smaller y) and right.
        let p = Vec3::new(2.0, 3.0, 0.0);
        let s = project_point(&vp, p, SIZE).expect("in front");
        assert!(s.x > SIZE.x * 0.5 && s.y < SIZE.y * 0.5, "{s:?}");

        // Round-trip: the ray through that pixel passes through the point.
        let ray = ray_from_screen(&cam, s, SIZE);
        let t = (p - ray.origin).dot(ray.dir);
        assert!((ray.at(t) - p).length() < 1e-2, "round trip: {:?}", ray.at(t));
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        let vp = cam().view_proj();
        assert!(project_point(&vp, Vec3::new(0.0, 0.0, 40.0), SIZE).is_none());
    }

    #[test]
    fn horizontal_drag_on_the_x_axis_moves_by_the_expected_amount() {
        let cam = cam();
        let vp = cam.view_proj();
        let origin = Vec3::ZERO;
        let length = 4.0;
        // With the camera looking down -Z, world +X is screen +x.
        let dir = axis_screen_dir(&vp, origin, Vec3::X, length, SIZE).expect("visible");
        assert!(dir.x > 0.0 && dir.y.abs() < 1e-3, "{dir:?}");

        // Dragging exactly the handle's pixel length moves a full `length`.
        let d = axis_drag_delta(&vp, origin, Vec3::X, length, SIZE, dir);
        assert!((d - length).abs() < 1e-3, "full-handle drag: {d}");
        // Half the drag, half the motion; the opposite drag reverses it.
        let half = axis_drag_delta(&vp, origin, Vec3::X, length, SIZE, dir * 0.5);
        assert!((half - length * 0.5).abs() < 1e-3, "{half}");
        let back = axis_drag_delta(&vp, origin, Vec3::X, length, SIZE, -dir);
        assert!((back + length).abs() < 1e-3, "{back}");
        // A purely vertical drag does not move the X axis.
        let cross = axis_drag_delta(&vp, origin, Vec3::X, length, SIZE, Vec2::new(0.0, 50.0));
        assert!(cross.abs() < 1e-4, "{cross}");
    }

    #[test]
    fn y_axis_drag_uses_screen_up_as_positive() {
        let vp = cam().view_proj();
        // Screen y is down, so dragging *up* (negative y) must raise the node.
        let d = axis_drag_delta(&vp, Vec3::ZERO, Vec3::Y, 4.0, SIZE, Vec2::new(0.0, -30.0));
        assert!(d > 0.0, "dragging up moves +Y: {d}");
    }

    #[test]
    fn axis_pointing_at_the_camera_yields_no_motion() {
        // Camera on +Z looking at the origin: world Z projects to (almost)
        // nothing on screen at the centre.
        let vp = cam().view_proj();
        let d = axis_drag_delta(&vp, Vec3::ZERO, Vec3::Z, 4.0, SIZE, Vec2::new(50.0, 50.0));
        assert!(d.abs() < 1e-3, "degenerate axis is inert: {d}");
    }

    #[test]
    fn clicking_near_a_handle_grabs_that_axis() {
        let vp = cam().view_proj();
        let origin = Vec3::ZERO;
        let len = 4.0;
        let centre = project_point(&vp, origin, SIZE).expect("visible");
        let x_end = project_point(&vp, Vec3::X * len, SIZE).expect("visible");
        let y_end = project_point(&vp, Vec3::Y * len, SIZE).expect("visible");

        let mid_x = (centre + x_end) * 0.5;
        assert_eq!(nearest_axis(&vp, origin, len, SIZE, mid_x, 8.0), Some(0));
        let mid_y = (centre + y_end) * 0.5;
        assert_eq!(nearest_axis(&vp, origin, len, SIZE, mid_y, 8.0), Some(1));
        // Far from every handle: nothing grabbed.
        assert_eq!(
            nearest_axis(&vp, origin, len, SIZE, Vec2::new(10.0, 10.0), 8.0),
            None
        );
    }

    #[test]
    fn point_segment_distance_handles_ends_and_degenerate_segments() {
        let a = Vec2::ZERO;
        let b = Vec2::new(10.0, 0.0);
        assert!((point_segment_dist(Vec2::new(5.0, 3.0), a, b) - 3.0).abs() < 1e-5);
        assert!((point_segment_dist(Vec2::new(-4.0, 0.0), a, b) - 4.0).abs() < 1e-5);
        assert!((point_segment_dist(Vec2::new(3.0, 4.0), a, a) - 5.0).abs() < 1e-5);
    }

    #[test]
    fn handle_length_stays_in_a_usable_range() {
        assert!(handle_length(0.0, 100.0) >= 0.5);
        assert!(handle_length(1000.0, 20.0) <= 5.0, "clamped by view distance");
    }
}
