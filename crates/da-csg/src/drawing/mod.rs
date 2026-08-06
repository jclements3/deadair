//! ISO 128 orthographic 2D technical-drawing generator.
//!
//! Projects a 3D `Solid` (a soup of convex, outward-wound polygons) down to
//! clean 2D engineering views. Instead of drawing every tessellation triangle,
//! it extracts *feature* edges (dihedral angle > ~15°) and *silhouette* edges
//! (adjacent faces straddling the view direction), then runs hidden-line
//! removal against a coarse depth buffer. Output is styled per ISO 128 line
//! conventions and emitted as SVG.
//!
//! ## Projection
//! * Front — look along −Y, project (X, Z), depth toward viewer = +Y.
//! * Top   — look along −Z, project (X, Y), depth toward viewer = +Z.
//! * Right — look along −X, project (Y, Z), depth toward viewer = +X.
//! * Iso   — standard isometric (view dir (1,1,1)).
//!
//! 2D image space is Y-down (SVG-native): screen = (u, −v).
//!
//! ## Hidden-line removal (HLR)
//! We rasterize all projected faces into a coarse depth buffer keyed by
//! "distance toward the viewer" (larger = nearer). Each candidate edge is
//! sampled along its length; a sample is *Visible* when its depth is within an
//! epsilon of (or in front of) the nearest rasterized face at that pixel, and
//! *Hidden* otherwise. Consecutive same-class samples are merged into
//! sub-segments. This is approximate (grid resolution + sampling density bound
//! the accuracy) but robust and cheap.

use crate::csg::bsp::{v3, V3};
use crate::csg::Solid;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum ViewDir {
    Front,
    Top,
    Right,
    Iso,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LineKind {
    Visible,
    Hidden,
    Center,
    Section,
    /// Reserved for auto-generated dimension lines (styled but not yet emitted).
    #[allow(dead_code)]
    Dimension,
}

#[derive(Clone, Copy)]
pub struct Seg2 {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub kind: LineKind,
}

pub struct View {
    pub name: String,
    pub segments: Vec<Seg2>,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

pub struct Drawing {
    pub views: Vec<View>,
}

/// Project `solid` to a single 2D view with feature/silhouette edge extraction
/// and hidden-line removal.
pub fn project(solid: &Solid, view: ViewDir) -> View {
    let (right, up, w) = basis(view);
    let faces = collect_faces(solid);
    let edges = build_edges(&faces);

    // Screen-space projection: (u, -v) with Y-down; depth = distance toward viewer.
    let proj = move |p: V3| -> (f32, f32, f32) {
        let u = p.dot(right);
        let v = p.dot(up);
        let d = p.dot(w);
        (u as f32, (-v) as f32, d as f32)
    };

    // Which edges to draw: feature (dihedral > 15°), boundary, or silhouette.
    let cos_t = (15.0_f64).to_radians().cos();
    // Each drawn edge carries an `outline` flag: silhouette + open-boundary edges
    // lie exactly on the object's frontmost contour, so they should read as solid
    // visible outline rather than dashing out from z-fighting with the coplanar
    // surface behind them. Feature (crease) edges keep the strict depth test.
    let mut drawn: Vec<(V3, V3, bool)> = Vec::new();
    for e in &edges {
        let (draw, outline) = if e.faces.len() == 1 {
            (true, true) // open boundary edge — always a real outline
        } else {
            let mut feature = false;
            let mut silhouette = false;
            for a in 0..e.faces.len() {
                for b in (a + 1)..e.faces.len() {
                    let na = faces[e.faces[a]].normal;
                    let nb = faces[e.faces[b]].normal;
                    if na.dot(nb) < cos_t {
                        feature = true;
                    }
                    if na.dot(w) * nb.dot(w) < 0.0 {
                        silhouette = true;
                    }
                }
            }
            (feature || silhouette, silhouette)
        };
        if draw {
            drawn.push((e.a, e.b, outline));
        }
    }

    // Depth buffer over all faces for HLR.
    let depth = DepthBuffer::build(&faces, &proj);

    // Classify each edge into visible/hidden sub-segments.
    let mut segments: Vec<Seg2> = Vec::new();
    for (a, b, outline) in drawn {
        // Nudge outline edges a few depth-epsilons toward the camera so a grazing
        // silhouette wins against its own surface instead of dashing in and out.
        let bias = if outline { depth.eps * 3.0 } else { 0.0 };
        classify_edge(a, b, &proj, &depth, bias, &mut segments);
    }

    let (min, max) = bounds_of_segments(&segments);
    View {
        name: view_name(view).to_string(),
        segments,
        min,
        max,
    }
}

/// Front, Top and Right views (first-angle set). Layout offsets are applied by
/// `to_svg`; the raw views are returned here in model coordinates.
pub fn multiview(solid: &Solid) -> Drawing {
    Drawing {
        views: vec![
            project(solid, ViewDir::Front),
            project(solid, ViewDir::Top),
            project(solid, ViewDir::Right),
        ],
    }
}

/// Standard isometric pictorial (view direction (1,1,1)) with the same
/// hidden-line removal as the orthographic views — an optional extra pane.
pub fn isometric(solid: &Solid) -> View {
    project(solid, ViewDir::Iso)
}

/// Cut `solid` with an axis-aligned plane (perpendicular to the view's depth
/// axis) at `plane_offset`, keep the near half, project it, then hatch the cut
/// cross-section.
pub fn section(solid: &Solid, view: ViewDir, plane_offset: f64) -> View {
    // Iso has no axis-aligned depth axis to cut along cleanly — just project.
    if view == ViewDir::Iso {
        let mut v = project(solid, ViewDir::Iso);
        v.name = "Section - Iso".to_string();
        return v;
    }

    let (right, up, w) = basis(view);
    let (lo, hi) = solid_bounds(solid);
    let size = hi.sub(lo);
    let extent = size.x.max(size.y).max(size.z).max(1.0);
    let big = extent * 3.0 + 10.0;
    let mid = lo.add(hi).mul(0.5);

    // A large axis-aligned box covering the *far* half (depth < offset), which
    // we subtract to keep the near half. w is one of ±X/Y/Z for these views.
    let (bx, by, bz, cx, cy, cz) = match view {
        ViewDir::Front => (big, big, big, mid.x, plane_offset - big / 2.0, mid.z), // w = +Y
        ViewDir::Top => (big, big, big, mid.x, mid.y, plane_offset - big / 2.0),   // w = +Z
        ViewDir::Right => (big, big, big, plane_offset - big / 2.0, mid.y, mid.z), // w = +X
        ViewDir::Iso => unreachable!(),
    };
    let cutter = Solid::cube(bx, by, bz).translate(cx, cy, cz);
    let cut = solid.clone().difference(cutter);

    // Project the cut solid through the normal pipeline.
    let proj = move |p: V3| -> (f32, f32, f32) {
        let u = p.dot(right);
        let v = p.dot(up);
        let d = p.dot(w);
        (u as f32, (-v) as f32, d as f32)
    };

    let mut v = project(&cut, view);
    v.name = format!("Section - {}", view_name(view));

    // Identify the cross-section faces: polygons lying on the cut plane.
    let tol = extent * 1e-4 + 1e-5;
    let mut section_tris: Vec<([f32; 2], [f32; 2], [f32; 2])> = Vec::new();
    for poly in &cut.polys {
        let on_plane = poly
            .vertices
            .iter()
            .all(|vx| (vx.pos.dot(w) - plane_offset).abs() < tol);
        if !on_plane || poly.vertices.len() < 3 {
            continue;
        }
        // Fan-triangulate in screen space.
        let p0 = proj(poly.vertices[0].pos);
        for i in 1..poly.vertices.len() - 1 {
            let p1 = proj(poly.vertices[i].pos);
            let p2 = proj(poly.vertices[i + 1].pos);
            section_tris.push(([p0.0, p0.1], [p1.0, p1.1], [p2.0, p2.1]));
        }
    }

    // 45° hatch lines clipped to the cross-section triangles.
    if !section_tris.is_empty() {
        let (smin, smax) = bounds_of_segments(&v.segments);
        let diag =
            ((smax[0] - smin[0]).powi(2) + (smax[1] - smin[1]).powi(2)).sqrt() as f64;
        let step = (diag / 14.0).max(extent as f64 * 0.05).max(1e-3) as f32;
        for (a, b, c) in &section_tris {
            hatch_triangle(*a, *b, *c, step, &mut v.segments);
        }
    }

    let (min, max) = bounds_of_segments(&v.segments);
    v.min = min;
    v.max = max;
    v
}

/// Emit one `<svg>` containing every view laid out side by side, each in its own
/// `<g transform>` with a title and a light background.
pub fn to_svg(drawing: &Drawing) -> String {
    const S: f32 = 30.0; // px per model unit
    const PAD: f32 = 24.0;
    const TITLE_H: f32 = 22.0;

    // Per-view pixel sizes.
    let mut sizes: Vec<(f32, f32)> = Vec::new();
    for view in &drawing.views {
        let w = ((view.max[0] - view.min[0]) * S).max(1.0);
        let h = ((view.max[1] - view.min[1]) * S).max(1.0);
        sizes.push((w, h));
    }

    let total_w: f32 = sizes.iter().map(|(w, _)| w + 2.0 * PAD).sum::<f32>() + PAD;
    let max_h: f32 = sizes.iter().map(|(_, h)| *h).fold(0.0, f32::max);
    let total_h = max_h + 2.0 * PAD + TITLE_H + PAD;

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" \
         viewBox=\"0 0 {:.0} {:.0}\">\n",
        total_w.max(1.0),
        total_h.max(1.0),
        total_w.max(1.0),
        total_h.max(1.0)
    ));
    s.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{:.0}\" height=\"{:.0}\" fill=\"#f6f7f9\"/>\n",
        total_w.max(1.0),
        total_h.max(1.0)
    ));

    let mut ox = PAD;
    for (view, (w, _h)) in drawing.views.iter().zip(sizes.iter()) {
        let gx = ox;
        let gy = PAD + TITLE_H;
        s.push_str(&format!("  <g transform=\"translate({:.2},{:.2})\">\n", gx, gy));
        s.push_str(&format!(
            "    <text x=\"0\" y=\"-6\" font-family=\"sans-serif\" font-size=\"13\" \
             fill=\"#222\">{}</text>\n",
            xml_escape(&view.name)
        ));

        for seg in &view.segments {
            let ax = (seg.a[0] - view.min[0]) * S;
            let ay = (seg.a[1] - view.min[1]) * S;
            let bx = (seg.b[0] - view.min[0]) * S;
            let by = (seg.b[1] - view.min[1]) * S;
            s.push_str(&format!(
                "    <line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" {}/>\n",
                ax,
                ay,
                bx,
                by,
                style_for(seg.kind)
            ));
        }
        s.push_str("  </g>\n");
        ox += w + 2.0 * PAD;
    }

    s.push_str("</svg>\n");
    s
}

// ---------------------------------------------------------------------------
// ISO 128 styling
// ---------------------------------------------------------------------------

fn style_for(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Visible => "stroke=\"#111111\" stroke-width=\"0.7\" fill=\"none\"",
        LineKind::Hidden => {
            "stroke=\"#333333\" stroke-width=\"0.35\" stroke-dasharray=\"4 2\" fill=\"none\""
        }
        LineKind::Center => {
            "stroke=\"#8a1f1f\" stroke-width=\"0.35\" stroke-dasharray=\"12 3 2 3\" fill=\"none\""
        }
        LineKind::Section => "stroke=\"#556070\" stroke-width=\"0.35\" fill=\"none\"",
        LineKind::Dimension => "stroke=\"#1560bd\" stroke-width=\"0.35\" fill=\"none\"",
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Projection bases
// ---------------------------------------------------------------------------

/// Returns (right, up, toward-viewer) unit axes for a view.
fn basis(view: ViewDir) -> (V3, V3, V3) {
    match view {
        ViewDir::Front => (v3(1.0, 0.0, 0.0), v3(0.0, 0.0, 1.0), v3(0.0, 1.0, 0.0)),
        ViewDir::Top => (v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0)),
        ViewDir::Right => (v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0), v3(1.0, 0.0, 0.0)),
        ViewDir::Iso => {
            let w = v3(1.0, 1.0, 1.0).normalized();
            let up_world = v3(0.0, 0.0, 1.0);
            let right = up_world.cross(w).normalized();
            let up = w.cross(right).normalized();
            (right, up, w)
        }
    }
}

fn view_name(view: ViewDir) -> &'static str {
    match view {
        ViewDir::Front => "Front",
        ViewDir::Top => "Top",
        ViewDir::Right => "Right",
        ViewDir::Iso => "Iso",
    }
}

// ---------------------------------------------------------------------------
// Faces & edge adjacency
// ---------------------------------------------------------------------------

struct Face {
    normal: V3,
    verts: Vec<V3>,
}

fn face_normal(vs: &[V3]) -> V3 {
    let n = vs.len();
    for i in 1..n - 1 {
        let a = vs[i].sub(vs[0]);
        let b = vs[i + 1].sub(vs[0]);
        let c = a.cross(b);
        if c.len() > 1e-12 {
            return c.normalized();
        }
    }
    v3(0.0, 0.0, 1.0)
}

fn collect_faces(solid: &Solid) -> Vec<Face> {
    solid
        .polys
        .iter()
        .map(|p| {
            let verts: Vec<V3> = p.vertices.iter().map(|v| v.pos).collect();
            Face {
                normal: face_normal(&verts),
                verts,
            }
        })
        .collect()
}

type QKey = (i64, i64, i64);

fn quant(p: V3) -> QKey {
    (
        (p.x * 1e5).round() as i64,
        (p.y * 1e5).round() as i64,
        (p.z * 1e5).round() as i64,
    )
}

struct Edge {
    a: V3,
    b: V3,
    faces: Vec<usize>,
}

fn build_edges(faces: &[Face]) -> Vec<Edge> {
    // darkair: BTreeMap (not HashMap) so edges emerge in a deterministic order —
    // edge order decides SVG <line> element order, and byte-identical output for
    // identical input is load-bearing here.
    let mut map: BTreeMap<(QKey, QKey), Edge> = BTreeMap::new();
    for (fi, f) in faces.iter().enumerate() {
        let n = f.verts.len();
        for i in 0..n {
            let a = f.verts[i];
            let b = f.verts[(i + 1) % n];
            if a.sub(b).len() < 1e-9 {
                continue;
            }
            let (qa, qb) = (quant(a), quant(b));
            let key = if qa <= qb { (qa, qb) } else { (qb, qa) };
            let entry = map.entry(key).or_insert_with(|| Edge {
                a,
                b,
                faces: Vec::new(),
            });
            if !entry.faces.contains(&fi) {
                entry.faces.push(fi);
            }
        }
    }
    map.into_values().collect()
}

// ---------------------------------------------------------------------------
// Depth buffer & hidden-line removal
// ---------------------------------------------------------------------------

struct DepthBuffer {
    w: usize,
    h: usize,
    buf: Vec<f32>,
    minx: f32,
    miny: f32,
    dx: f32,
    dy: f32,
    eps: f32,
}

impl DepthBuffer {
    fn build(faces: &[Face], proj: &impl Fn(V3) -> (f32, f32, f32)) -> DepthBuffer {
        // Screen-space bounds and depth span.
        let mut minx = f32::INFINITY;
        let mut miny = f32::INFINITY;
        let mut maxx = f32::NEG_INFINITY;
        let mut maxy = f32::NEG_INFINITY;
        let mut dmin = f32::INFINITY;
        let mut dmax = f32::NEG_INFINITY;
        for f in faces {
            for v in &f.verts {
                let (x, y, d) = proj(*v);
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
                dmin = dmin.min(d);
                dmax = dmax.max(d);
            }
        }
        if !minx.is_finite() {
            return DepthBuffer {
                w: 1,
                h: 1,
                buf: vec![f32::NEG_INFINITY],
                minx: 0.0,
                miny: 0.0,
                dx: 1.0,
                dy: 1.0,
                eps: 1e-6,
            };
        }
        let du = (maxx - minx).max(1e-6);
        let dv = (maxy - miny).max(1e-6);
        let res = 320.0_f32;
        let m = du.max(dv);
        let w = ((du / m * res).ceil() as usize).max(2);
        let h = ((dv / m * res).ceil() as usize).max(2);
        let eps = (dmax - dmin).max(1e-6) * 0.03;

        let mut db = DepthBuffer {
            w,
            h,
            buf: vec![f32::NEG_INFINITY; w * h],
            minx,
            miny,
            dx: du,
            dy: dv,
            eps,
        };

        // Rasterize each face (fan-triangulated) keeping the nearest depth.
        for f in faces {
            let ps: Vec<(f32, f32, f32)> = f.verts.iter().map(|v| proj(*v)).collect();
            for i in 1..ps.len() - 1 {
                db.raster_tri(ps[0], ps[i], ps[i + 1]);
            }
        }
        db
    }

    fn to_px(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x - self.minx) / self.dx * (self.w as f32 - 1.0),
            (y - self.miny) / self.dy * (self.h as f32 - 1.0),
        )
    }

    fn raster_tri(&mut self, a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32)) {
        let (ax, ay) = self.to_px(a.0, a.1);
        let (bx, by) = self.to_px(b.0, b.1);
        let (cx, cy) = self.to_px(c.0, c.1);
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area.abs() < 1e-9 {
            return;
        }
        let min_x = ax.min(bx).min(cx).floor().max(0.0) as i32;
        let max_x = ax.max(bx).max(cx).ceil().min(self.w as f32 - 1.0) as i32;
        let min_y = ay.min(by).min(cy).floor().max(0.0) as i32;
        let max_y = ay.max(by).max(cy).ceil().min(self.h as f32 - 1.0) as i32;
        let inv = 1.0 / area;
        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let fx = px as f32;
                let fy = py as f32;
                let w0 = ((bx - fx) * (cy - fy) - (by - fy) * (cx - fx)) * inv;
                let w1 = ((cx - fx) * (ay - fy) - (cy - fy) * (ax - fx)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let depth = w0 * a.2 + w1 * b.2 + w2 * c.2;
                let idx = py as usize * self.w + px as usize;
                if depth > self.buf[idx] {
                    self.buf[idx] = depth;
                }
            }
        }
    }

    fn visible(&self, x: f32, y: f32, d: f32) -> bool {
        let (px, py) = self.to_px(x, y);
        let ix = px.round() as i32;
        let iy = py.round() as i32;
        if ix < 0 || iy < 0 || ix >= self.w as i32 || iy >= self.h as i32 {
            return true; // outside the rasterized area — nothing occludes it
        }
        let nearest = self.buf[iy as usize * self.w + ix as usize];
        if !nearest.is_finite() {
            return true;
        }
        d >= nearest - self.eps
    }
}

/// Sample an edge and emit visible/hidden sub-segments.
fn classify_edge(
    a: V3,
    b: V3,
    proj: &impl Fn(V3) -> (f32, f32, f32),
    depth: &DepthBuffer,
    bias: f32,
    out: &mut Vec<Seg2>,
) {
    // Cull edges that project to (near) a point.
    let pa = proj(a);
    let pb = proj(b);
    if (pa.0 - pb.0).hypot(pa.1 - pb.1) < 1e-6 {
        return;
    }

    const N: usize = 48;
    let mut pts: Vec<([f32; 2], bool)> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let p = a.add(b.sub(a).mul(t));
        let (x, y, d) = proj(p);
        pts.push(([x, y], depth.visible(x, y, d + bias)));
    }

    let mut i = 0;
    while i < pts.len() {
        let vis = pts[i].1;
        let start = pts[i].0;
        let mut j = i;
        while j + 1 < pts.len() && pts[j + 1].1 == vis {
            j += 1;
        }
        let end = pts[j].0;
        if (start[0] - end[0]).hypot(start[1] - end[1]) > 1e-6 {
            out.push(Seg2 {
                a: start,
                b: end,
                kind: if vis { LineKind::Visible } else { LineKind::Hidden },
            });
        }
        i = j + 1;
    }
}

// ---------------------------------------------------------------------------
// Section hatching
// ---------------------------------------------------------------------------

/// Clip a family of 45° lines (constant `x - y`) to a screen-space triangle,
/// emitting `Section` segments.
fn hatch_triangle(a: [f32; 2], b: [f32; 2], c: [f32; 2], step: f32, out: &mut Vec<Seg2>) {
    let f = |p: [f32; 2]| p[0] - p[1];
    let (fa, fb, fc) = (f(a), f(b), f(c));
    let fmin = fa.min(fb).min(fc);
    let fmax = fa.max(fb).max(fc);
    let edges = [(a, b), (b, c), (c, a)];
    let mut k = (fmin / step).ceil() * step;
    while k <= fmax {
        let mut hits: Vec<[f32; 2]> = Vec::new();
        for (p, q) in &edges {
            let (fp, fq) = (f(*p), f(*q));
            if (fp - k) * (fq - k) <= 0.0 && (fp - fq).abs() > 1e-9 {
                let t = (k - fp) / (fq - fp);
                hits.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
            }
        }
        if hits.len() >= 2 {
            out.push(Seg2 {
                a: hits[0],
                b: hits[1],
                kind: LineKind::Section,
            });
        }
        k += step;
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn bounds_of_segments(segs: &[Seg2]) -> ([f32; 2], [f32; 2]) {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for s in segs {
        for p in [s.a, s.b] {
            min[0] = min[0].min(p[0]);
            min[1] = min[1].min(p[1]);
            max[0] = max[0].max(p[0]);
            max[1] = max[1].max(p[1]);
        }
    }
    if !min[0].is_finite() {
        return ([0.0, 0.0], [0.0, 0.0]);
    }
    (min, max)
}

fn solid_bounds(solid: &Solid) -> (V3, V3) {
    let mut lo = v3(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = v3(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &solid.polys {
        for vx in &p.vertices {
            lo = v3(lo.x.min(vx.pos.x), lo.y.min(vx.pos.y), lo.z.min(vx.pos.z));
            hi = v3(hi.x.max(vx.pos.x), hi.y.max(vx.pos.y), hi.z.max(vx.pos.z));
        }
    }
    if !lo.x.is_finite() {
        return (v3(0.0, 0.0, 0.0), v3(0.0, 0.0, 0.0));
    }
    (lo, hi)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csg::Solid;

    #[test]
    fn front_view_of_cube_is_a_square() {
        let cube = Solid::cube(2.0, 2.0, 2.0);
        let view = project(&cube, ViewDir::Front);

        // Bounding box ~ (-1,-1)..(1,1).
        assert!((view.min[0] + 1.0).abs() < 0.05, "min x {}", view.min[0]);
        assert!((view.min[1] + 1.0).abs() < 0.05, "min y {}", view.min[1]);
        assert!((view.max[0] - 1.0).abs() < 0.05, "max x {}", view.max[0]);
        assert!((view.max[1] - 1.0).abs() < 0.05, "max y {}", view.max[1]);

        let visible = view
            .segments
            .iter()
            .filter(|s| s.kind == LineKind::Visible)
            .count();
        assert!(visible >= 4, "expected >=4 visible segments, got {visible}");
    }

    #[test]
    fn stacked_boxes_produce_hidden_lines() {
        // An L-shape / step: a big base with a smaller box on top, offset so a
        // step edge is occluded from some direction.
        let base = Solid::cube(4.0, 4.0, 2.0);
        let top = Solid::cube(2.0, 2.0, 2.0).translate(1.0, 0.0, 2.0);
        let l = base.union(top);

        let mut hidden_total = 0;
        for v in [ViewDir::Front, ViewDir::Top, ViewDir::Right, ViewDir::Iso] {
            hidden_total += project(&l, v)
                .segments
                .iter()
                .filter(|s| s.kind == LineKind::Hidden)
                .count();
        }
        assert!(
            hidden_total >= 1,
            "expected at least one hidden segment across views, got {hidden_total}"
        );
    }

    #[test]
    fn svg_has_hidden_dashes_and_is_nontrivial() {
        let cube = Solid::cube(2.0, 2.0, 2.0);
        let drawing = multiview(&cube);
        let svg = to_svg(&drawing);

        assert!(svg.contains("<svg"), "missing <svg root");
        assert!(
            svg.contains("stroke-dasharray"),
            "expected dashed hidden lines in svg"
        );
        assert!(svg.len() > 500, "svg too short: {} bytes", svg.len());
    }

    #[test]
    fn section_cuts_and_hatches() {
        // Cut a cube through the middle; expect hatch (Section) lines.
        let cube = Solid::cube(4.0, 4.0, 4.0);
        let v = section(&cube, ViewDir::Front, 0.0);
        let hatch = v
            .segments
            .iter()
            .filter(|s| s.kind == LineKind::Section)
            .count();
        assert!(hatch >= 1, "expected section hatch lines, got {hatch}");
    }
}
