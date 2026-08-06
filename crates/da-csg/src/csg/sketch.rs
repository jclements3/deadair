//! 2D sketches and the operations that lift them into 3D: extrude and revolve.
//!
//! A `Sketch` is a closed 2D polygon. `extrude` treats it as an XY cross-section
//! swept along Z. `revolve` treats each point as `(radius, height)` and spins it
//! around the Z axis — the body-of-revolution operation (vase, missile hull…).

use super::bsp::{v3, Polygon, Vertex, V3};
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub struct Sketch {
    /// Closed loop of (x, y) points, no repeated closing point.
    pub points: Vec<(f64, f64)>,
}

impl Sketch {
    pub fn polygon(points: Vec<(f64, f64)>) -> Sketch {
        Sketch { points }
    }
    pub fn rect(w: f64, h: f64) -> Sketch {
        let (x, y) = (w / 2.0, h / 2.0);
        Sketch::polygon(vec![(-x, -y), (x, -y), (x, y), (-x, y)])
    }
    pub fn circle(r: f64, seg: usize) -> Sketch {
        let pts = (0..seg)
            .map(|i| {
                let t = i as f64 / seg as f64 * 2.0 * PI;
                (r * t.cos(), r * t.sin())
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Rounded rectangle `w × h` with corner radius `r` (`seg` steps per corner
    /// quarter-arc). Area = `w·h − (4−π)·r²`.
    pub fn roundrect(w: f64, h: f64, r: f64, seg: usize) -> Sketch {
        let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
        let (x, y) = (w / 2.0 - r, h / 2.0 - r);
        let seg = seg.max(1);
        let mut pts = Vec::new();
        // Four corner centers + starting angles, traced CCW.
        for (cx, cy, a0) in [(x, -y, -PI / 2.0), (x, y, 0.0), (-x, y, PI / 2.0), (-x, -y, PI)] {
            for i in 0..=seg {
                let a = a0 + i as f64 / seg as f64 * PI / 2.0;
                pts.push((cx + r * a.cos(), cy + r * a.sin()));
            }
        }
        Sketch::polygon(pts)
    }

    /// Slot / stadium: two semicircles of radius `r` whose centers are `l` apart
    /// on the X axis, joined by straight sides. Area = `π·r² + 2·r·l`.
    pub fn slot(l: f64, r: f64, seg: usize) -> Sketch {
        let seg = seg.max(2);
        let hl = l.abs() / 2.0;
        let mut pts = Vec::new();
        for i in 0..=seg {
            let a = -PI / 2.0 + i as f64 / seg as f64 * PI; // right cap, -90..90
            pts.push((hl + r * a.cos(), r * a.sin()));
        }
        for i in 0..=seg {
            let a = PI / 2.0 + i as f64 / seg as f64 * PI; // left cap, 90..270
            pts.push((-hl + r * a.cos(), r * a.sin()));
        }
        Sketch::polygon(pts)
    }

    /// Regular convex `n`-gon of circumradius `r`, first vertex pointing +Y.
    pub fn ngon(n: usize, r: f64) -> Sketch {
        let n = n.max(3);
        let pts = (0..n)
            .map(|i| {
                let a = PI / 2.0 + i as f64 / n as f64 * 2.0 * PI;
                (r * a.cos(), r * a.sin())
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Limaçon section `r = b·(c + cos θ)` as a closed XY loop (ports core.py
    /// `_curve`, the polar family behind the harp soundbox shells). `c > 1` gives
    /// a dimpled convex loop, `c = 1` the cardioid cusp, `0 < c < 1` a curve with
    /// an inner loop (self-intersecting — fine as a drawing, not as a solid).
    pub fn limacon(c: f64, b: f64, seg: usize) -> Sketch {
        let seg = seg.max(3);
        let pts = (0..seg)
            .map(|i| {
                let th = i as f64 / seg as f64 * 2.0 * PI;
                let r = b * (c + th.cos());
                (-r * th.sin(), r * th.cos())
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Fluted "rose" column section: a near-circular outline carrying `flutes`
    /// lobes of relative amplitude `amp`, normalized so the petal tips reach
    /// radius `2·b` (ports loft_param.py `rose_rings`; its defaults are 12 flutes
    /// at amp 0.4). Pair with a `circle` bore for the hollow pillar.
    pub fn rose(b: f64, flutes: f64, amp: f64, seg: usize) -> Sketch {
        let seg = seg.max(3);
        let denom = 3.0 + amp.abs();
        let pts = (0..seg)
            .map(|i| {
                let th = i as f64 / seg as f64 * 2.0 * PI;
                let ro = 2.0 * b * (3.0 + amp * (flutes * th).sin()) / denom;
                (ro * th.cos(), ro * th.sin())
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Parse an SVG path `d` string into a closed 2D loop (ports core.py
    /// `_flatten`). Supports M/L/H/V/C and Z in both absolute and relative form;
    /// cubic `C` segments are sampled into `steps` points. Coordinates are read
    /// as `(x, y)`. Unsupported commands (arcs, quadratics) return an error.
    pub fn from_svg_path(d: &str, steps: usize) -> Result<Sketch, String> {
        let steps = steps.max(2);
        let toks = svg_tokens(d);
        let mut pts: Vec<(f64, f64)> = Vec::new();
        let mut cur = (0.0, 0.0);
        let mut start = (0.0, 0.0);
        let mut cmd = ' ';
        let mut i = 0;
        // Pull the next `k` numbers; error if a command letter interrupts them.
        let num = |i: &mut usize| -> Result<f64, String> {
            match toks.get(*i) {
                Some(SvgTok::Num(n)) => {
                    *i += 1;
                    Ok(*n)
                }
                _ => Err("svgpath: ran out of coordinates for a command".into()),
            }
        };
        while i < toks.len() {
            match toks[i] {
                SvgTok::Cmd(c) => {
                    cmd = c;
                    i += 1;
                }
                SvgTok::Num(_) => {} // implicit repeat of the previous command
            }
            let rel = cmd.is_ascii_lowercase();
            match cmd.to_ascii_uppercase() {
                'M' => {
                    let (x, y) = (num(&mut i)?, num(&mut i)?);
                    cur = if rel { (cur.0 + x, cur.1 + y) } else { (x, y) };
                    start = cur;
                    pts.push(cur);
                    cmd = if rel { 'l' } else { 'L' }; // extra pairs are implicit L
                }
                'L' => {
                    let (x, y) = (num(&mut i)?, num(&mut i)?);
                    cur = if rel { (cur.0 + x, cur.1 + y) } else { (x, y) };
                    pts.push(cur);
                }
                'H' => {
                    let x = num(&mut i)?;
                    cur = if rel { (cur.0 + x, cur.1) } else { (x, cur.1) };
                    pts.push(cur);
                }
                'V' => {
                    let y = num(&mut i)?;
                    cur = if rel { (cur.0, cur.1 + y) } else { (cur.0, y) };
                    pts.push(cur);
                }
                'C' => {
                    let mut p = [0.0f64; 6];
                    for v in p.iter_mut() {
                        *v = num(&mut i)?;
                    }
                    let (c1, c2, end) = if rel {
                        (
                            (cur.0 + p[0], cur.1 + p[1]),
                            (cur.0 + p[2], cur.1 + p[3]),
                            (cur.0 + p[4], cur.1 + p[5]),
                        )
                    } else {
                        ((p[0], p[1]), (p[2], p[3]), (p[4], p[5]))
                    };
                    for s in 1..=steps {
                        let u = s as f64 / steps as f64;
                        let m = 1.0 - u;
                        let bx = m * m * m * cur.0 + 3.0 * m * m * u * c1.0 + 3.0 * m * u * u * c2.0 + u * u * u * end.0;
                        let by = m * m * m * cur.1 + 3.0 * m * m * u * c1.1 + 3.0 * m * u * u * c2.1 + u * u * u * end.1;
                        pts.push((bx, by));
                    }
                    cur = end;
                }
                'Z' => {
                    cur = start;
                }
                other => return Err(format!("svgpath: unsupported command '{other}' (M/L/H/V/C/Z only)")),
            }
        }
        // Drop a duplicate closing point if the path returned to its start.
        if pts.len() > 1 {
            let (a, b) = (pts[0], *pts.last().unwrap());
            if (a.0 - b.0).hypot(a.1 - b.1) < 1e-9 {
                pts.pop();
            }
        }
        if pts.len() < 3 {
            return Err("svgpath: need at least 3 points".into());
        }
        Ok(Sketch::polygon(pts))
    }

    /// Regular `n`-pointed star as a closed 2n-vertex loop. Vertex `k` sits at
    /// angle `π/2 + k·π/n` (first tip straight up), alternating between the outer
    /// tip radius `r_out` (even `k`) and the inner valley radius `r_in` (odd `k`).
    /// Its area is the analytic `n·r_out·r_in·sin(π/n)` (2n triangles). `seg` is
    /// accepted for palette symmetry but a star has exactly `2n` vertices.
    pub fn star(n: usize, r_out: f64, r_in: f64, _seg: usize) -> Sketch {
        let n = n.max(2);
        let pts = (0..2 * n)
            .map(|k| {
                let th = PI / 2.0 + k as f64 * PI / n as f64;
                let r = if k % 2 == 0 { r_out } else { r_in };
                (r * th.cos(), r * th.sin())
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Superellipse (Lamé curve) `|x/a|^n + |y/b|^n = 1`, sampled parametrically
    /// as `x = a·sign(cosθ)·|cosθ|^(2/n)`, `y = b·sign(sinθ)·|sinθ|^(2/n)` over
    /// `[0, 2π)`. `n = 2` is a plain ellipse (area `π·a·b`); `n > 2` squares the
    /// corners toward a rounded rectangle; `n < 2` pinches it into an astroid.
    pub fn superellipse(a: f64, b: f64, n: f64, seg: usize) -> Sketch {
        let seg = seg.max(3);
        let p = 2.0 / n;
        let pts = (0..seg)
            .map(|i| {
                let th = i as f64 / seg as f64 * 2.0 * PI;
                let (c, s) = (th.cos(), th.sin());
                (a * c.signum() * c.abs().powf(p), b * s.signum() * s.abs().powf(p))
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// The classic mathematical rose `r = a·cos(k·θ)` sampled over `[0, 2π)` as
    /// `(r cosθ, r sinθ)`. Odd `k` draws `k` petals, even `k` draws `2k` petals;
    /// `r` swings negative so successive petals pass back through the origin. The
    /// loop is self-touching at the centre — fine as a sketch, and `extrude`
    /// still yields a positive-volume flower. (Contrast `rose`, which is a
    /// strictly convex fluted ring; this is the true multi-petal curve.)
    pub fn petals(a: f64, k: f64, seg: usize) -> Sketch {
        let seg = seg.max(3);
        let pts = (0..seg)
            .map(|i| {
                let th = i as f64 / seg as f64 * 2.0 * PI;
                let r = a * (k * th).cos();
                (r * th.cos(), r * th.sin())
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Hypotrochoid (the Spirograph curve): a point at distance `d` from the
    /// centre of a small circle of radius `r` rolling inside a fixed circle of
    /// radius `rr`. Traced as `x = (rr−r)cosθ + d·cos((rr−r)/r·θ)`,
    /// `y = (rr−r)sinθ − d·sin((rr−r)/r·θ)`. It closes after
    /// `round(r)/gcd(round(rr),round(r))` full turns, so we sweep exactly that
    /// many turns (falling back to 8 if the integer geometry is degenerate) with
    /// `seg` samples per turn.
    pub fn hypotrochoid(rr: f64, r: f64, d: f64, seg: usize) -> Sketch {
        let seg = seg.max(3);
        fn gcd(mut a: u64, mut b: u64) -> u64 {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        }
        let (ri, rri) = (r.abs().round() as u64, rr.abs().round() as u64);
        let g = gcd(rri, ri);
        let turns = if g == 0 { 8 } else { (ri / g).max(1) } as usize;
        let npts = (seg * turns).max(3);
        let k = (rr - r) / r;
        let pts = (0..npts)
            .map(|i| {
                let th = i as f64 / npts as f64 * 2.0 * PI * turns as f64;
                let x = (rr - r) * th.cos() + d * (k * th).cos();
                let y = (rr - r) * th.sin() - d * (k * th).sin();
                (x, y)
            })
            .collect();
        Sketch::polygon(pts)
    }

    /// Build a profile from a cubic-bezier spline — the lathe/turning silhouette.
    /// `nodes` is `[start, c1, c2, end, c1, c2, end, ...]` (`2 + 6*N` coords, i.e.
    /// a start anchor followed by N cubic segments as (control1, control2, end)
    /// triples). Each segment is sampled into `steps` points. For a lathe the
    /// points are read as `(r, z)` and the silhouette should begin and end on the
    /// axis (r=0) so `revolve` closes it into a solid body.
    pub fn bezier(nodes: &[(f64, f64)], steps: usize) -> Sketch {
        let steps = steps.max(2);
        let mut pts: Vec<(f64, f64)> = Vec::new();
        if nodes.is_empty() {
            return Sketch::polygon(pts);
        }
        pts.push(nodes[0]);
        let mut i = 1;
        while i + 2 < nodes.len() {
            let p0 = *pts.last().unwrap();
            let (c1, c2, p1) = (nodes[i], nodes[i + 1], nodes[i + 2]);
            for s in 1..=steps {
                let u = s as f64 / steps as f64;
                let m = 1.0 - u;
                let bx = m * m * m * p0.0 + 3.0 * m * m * u * c1.0 + 3.0 * m * u * u * c2.0 + u * u * u * p1.0;
                let by = m * m * m * p0.1 + 3.0 * m * m * u * c1.1 + 3.0 * m * u * u * c2.1 + u * u * u * p1.1;
                pts.push((bx, by));
            }
            i += 3;
        }
        Sketch { points: pts }
    }

    /// Shift every point by (dx, dy) — e.g. to offset a revolve profile.
    pub fn translate2d(mut self, dx: f64, dy: f64) -> Sketch {
        for p in &mut self.points {
            p.0 += dx;
            p.1 += dy;
        }
        self
    }

    /// Signed area (CCW positive).
    fn signed_area(&self) -> f64 {
        let p = &self.points;
        let n = p.len();
        let mut a = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            a += p[i].0 * p[j].1 - p[j].0 * p[i].1;
        }
        a / 2.0
    }

    /// Points guaranteed CCW.
    fn ccw(&self) -> Vec<(f64, f64)> {
        if self.signed_area() < 0.0 {
            self.points.iter().rev().cloned().collect()
        } else {
            self.points.clone()
        }
    }
}

/// Ear-clipping triangulation of a simple CCW polygon → index triples.
fn triangulate(poly: &[(f64, f64)]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut indices: Vec<usize> = (0..n).collect();
    let mut tris = Vec::new();

    let area2 = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let in_tri = |p: (f64, f64), a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        let d1 = area2(p, a, b);
        let d2 = area2(p, b, c);
        let d3 = area2(p, c, a);
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    };

    let mut guard = 0;
    while indices.len() > 3 && guard < 10_000 {
        guard += 1;
        let m = indices.len();
        let mut clipped = false;
        for k in 0..m {
            let i0 = indices[(k + m - 1) % m];
            let i1 = indices[k];
            let i2 = indices[(k + 1) % m];
            let a = poly[i0];
            let b = poly[i1];
            let c = poly[i2];
            if area2(a, b, c) <= 0.0 {
                continue; // reflex or degenerate
            }
            // No other vertex inside candidate ear.
            let mut ok = true;
            for &idx in &indices {
                if idx == i0 || idx == i1 || idx == i2 {
                    continue;
                }
                if in_tri(poly[idx], a, b, c) {
                    ok = false;
                    break;
                }
            }
            if ok {
                tris.push([i0, i1, i2]);
                indices.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break; // fall through with whatever remains (degenerate input)
        }
    }
    if indices.len() == 3 {
        tris.push([indices[0], indices[1], indices[2]]);
    }
    tris
}

/// A token from an SVG path `d` string: a command letter or a number.
enum SvgTok {
    Cmd(char),
    Num(f64),
}

/// Tokenize an SVG path `d` string into commands and numbers. Handles the SVG
/// number quirks: commas/whitespace separators, and a sign or second '.' that
/// begins a new number without a separator (e.g. `10-5` -> 10, -5; `.5.5` -> .5, .5).
fn svg_tokens(d: &str) -> Vec<SvgTok> {
    let b = d.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() {
            toks.push(SvgTok::Cmd(c));
            i += 1;
        } else if c == ',' || c.is_whitespace() {
            i += 1;
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() {
            let start = i;
            let (mut dot, mut exp) = (false, false);
            if c == '-' || c == '+' {
                i += 1;
            }
            while i < b.len() {
                let ch = b[i] as char;
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == '.' && !dot && !exp {
                    dot = true;
                    i += 1;
                } else if (ch == 'e' || ch == 'E') && !exp {
                    exp = true;
                    i += 1;
                    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(n) = d[start..i].parse::<f64>() {
                toks.push(SvgTok::Num(n));
            }
            if i == start {
                i += 1; // never stall
            }
        } else {
            i += 1;
        }
    }
    toks
}

/// Extrude a sketch along Z into a prism of height `h`, centered on Z=0.
/// Extrude a sketch along Z into a prism of height `h` while rotating each level
/// by up to `deg` degrees (a twisted prism / column). `steps` levels are skinned
/// with the loft machinery and the two ends are capped watertight.
pub fn extrude_twist(sketch: &Sketch, h: f64, deg: f64, steps: usize) -> Vec<Polygon> {
    let pts = ensure_ccw(&sketch.points);
    if pts.len() < 3 {
        return Vec::new();
    }
    let steps = steps.max(1);
    let ring_at = |t: f64| -> Vec<V3> {
        let z = -h / 2.0 + t * h;
        let a = (deg * t).to_radians();
        let (s, c) = (a.sin(), a.cos());
        pts.iter().map(|&(x, y)| v3(x * c - y * s, x * s + y * c, z)).collect()
    };
    let rings: Vec<Vec<V3>> = (0..=steps).map(|k| ring_at(k as f64 / steps as f64)).collect();
    let mut polys = skin_rings(&rings);
    polys.extend(cap_ring(&pts, &rings[0], v3(0.0, 0.0, -1.0), true));
    polys.extend(cap_ring(&pts, rings.last().unwrap(), v3(0.0, 0.0, 1.0), false));
    polys
}

pub fn extrude(sketch: &Sketch, h: f64) -> Vec<Polygon> {
    let pts = sketch.ccw();
    let n = pts.len();
    let zb = -h / 2.0;
    let zt = h / 2.0;
    let mut polys = Vec::new();

    // Side walls.
    for i in 0..n {
        let j = (i + 1) % n;
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[j];
        let b0 = v3(x0, y0, zb);
        let b1 = v3(x1, y1, zb);
        let t1 = v3(x1, y1, zt);
        let t0 = v3(x0, y0, zt);
        // Outward normal of this wall.
        let nrm = v3(y1 - y0, x0 - x1, 0.0).normalized();
        polys.push(Polygon::new(vec![
            Vertex::new(b0, nrm),
            Vertex::new(b1, nrm),
            Vertex::new(t1, nrm),
            Vertex::new(t0, nrm),
        ]));
    }

    // Caps.
    let tris = triangulate(&pts);
    let nb = v3(0.0, 0.0, -1.0);
    let nt = v3(0.0, 0.0, 1.0);
    for t in &tris {
        let p = |k: usize, z: f64, nrm: V3| Vertex::new(v3(pts[k].0, pts[k].1, z), nrm);
        // Bottom (wind for -Z outward), top (+Z outward).
        polys.push(Polygon::new(vec![
            p(t[0], zb, nb),
            p(t[2], zb, nb),
            p(t[1], zb, nb),
        ]));
        polys.push(Polygon::new(vec![
            p(t[0], zt, nt),
            p(t[1], zt, nt),
            p(t[2], zt, nt),
        ]));
    }
    polys
}

// --- shared skinning helpers (used by both loft and sweep) ----------------

/// Signed area of a raw (x, y) loop; CCW positive.
fn signed_area_pts(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        a += p[i].0 * p[j].1 - p[j].0 * p[i].1;
    }
    a / 2.0
}

/// Return the loop wound CCW (so caps triangulate and skin normals face out).
fn ensure_ccw(p: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if signed_area_pts(p) < 0.0 {
        p.iter().rev().cloned().collect()
    } else {
        p.to_vec()
    }
}

/// Resample a closed loop to exactly `n` points spaced equally by ARC LENGTH
/// around the loop (ports core.py `_resample`). Orientation is preserved.
fn resample_loop(pts: &[(f64, f64)], n: usize) -> Vec<(f64, f64)> {
    let m = pts.len();
    if m < 2 || n == 0 {
        return pts.to_vec();
    }
    // Cumulative arc length at the start of each segment, plus the total.
    let mut cum = vec![0.0; m];
    let mut total = 0.0;
    for i in 0..m {
        cum[i] = total;
        let j = (i + 1) % m;
        let (dx, dy) = (pts[j].0 - pts[i].0, pts[j].1 - pts[i].1);
        total += (dx * dx + dy * dy).sqrt();
    }
    if total < 1e-12 {
        return vec![pts[0]; n]; // fully degenerate loop
    }
    let mut out = Vec::with_capacity(n);
    let mut seg = 0usize;
    for k in 0..n {
        let s = k as f64 / n as f64 * total;
        // Advance to the segment containing arc position `s`.
        while seg + 1 < m && cum[seg + 1] <= s {
            seg += 1;
        }
        let j = (seg + 1) % m;
        let seg_len = if j == 0 { total - cum[seg] } else { cum[j] - cum[seg] };
        let t = if seg_len > 1e-12 { (s - cum[seg]) / seg_len } else { 0.0 };
        out.push((
            pts[seg].0 + (pts[j].0 - pts[seg].0) * t,
            pts[seg].1 + (pts[j].1 - pts[seg].1) * t,
        ));
    }
    out
}

/// Rotate a loop so index 0 is the canonical "apex" vertex — the one maximizing
/// `y*1e4 - |x|` (ports core.py `_roll`). Aligning every section apex-to-apex
/// makes index `j` correspond across sections and removes gross twist.
fn roll_canonical(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n == 0 {
        return Vec::new();
    }
    let mut best = 0usize;
    let mut best_key = f64::NEG_INFINITY;
    for (i, &(x, y)) in pts.iter().enumerate() {
        let key = y * 1e4 - x.abs();
        if key > best_key {
            best_key = key;
            best = i;
        }
    }
    (0..n).map(|i| pts[(i + best) % n]).collect()
}

/// Cyclically shift a loop left by `off` vertices.
fn rotate_loop(pts: &[(f64, f64)], off: usize) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n == 0 {
        return Vec::new();
    }
    (0..n).map(|i| pts[(i + off) % n]).collect()
}

/// Total index-wise correspondence distance between two equal-length loops.
fn corr_dist(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(p, q)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt())
        .sum()
}

/// Dynamic-time-warp correspondence (ports convoy.py `dtw_map`): a full DP cost
/// matrix on Euclidean point distance, backtracked to a warp path, then forced
/// monotone via a running max. Returns, for each index of `a`, the matched
/// (non-decreasing) index of `b`.
fn dtw_map(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<usize> {
    let (na, nb) = (a.len(), b.len());
    if na == 0 || nb == 0 {
        return vec![0; na];
    }
    let cost = |i: usize, j: usize| ((a[i].0 - b[j].0).powi(2) + (a[i].1 - b[j].1).powi(2)).sqrt();
    let inf = f64::INFINITY;
    let mut d = vec![vec![inf; nb + 1]; na + 1];
    d[0][0] = 0.0;
    for i in 1..=na {
        for j in 1..=nb {
            let c = cost(i - 1, j - 1);
            d[i][j] = c + d[i - 1][j].min(d[i][j - 1]).min(d[i - 1][j - 1]);
        }
    }
    // Backtrack the minimal-cost path.
    let (mut i, mut j) = (na, nb);
    let mut map = vec![0usize; na];
    while i > 0 && j > 0 {
        map[i - 1] = j - 1;
        let diag = d[i - 1][j - 1];
        let up = d[i - 1][j];
        let left = d[i][j - 1];
        if diag <= up && diag <= left {
            i -= 1;
            j -= 1;
        } else if up <= left {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    // Force a monotone non-decreasing mapping.
    for k in 1..na {
        if map[k] < map[k - 1] {
            map[k] = map[k - 1];
        }
    }
    map
}

/// Twist-align `cur` to `prev` (both equal-length, apex-rolled) using the DTW
/// correspondence: the mapping's mean index offset selects a single cyclic
/// rotation of `cur`. A rotation is a bijection, so every ring stays a full-N
/// permutation and the skinned tube stays watertight; the rotation is kept only
/// if it actually reduces the correspondence distance (else identity stitch).
fn dtw_align(prev: &[(f64, f64)], cur: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = cur.len();
    if n == 0 || prev.len() != n {
        return cur.to_vec();
    }
    let map = dtw_map(prev, cur);
    let mean: f64 = map
        .iter()
        .enumerate()
        .map(|(i, &m)| m as f64 - i as f64)
        .sum::<f64>()
        / n as f64;
    let off = ((mean.round() as isize).rem_euclid(n as isize)) as usize;
    if off == 0 {
        return cur.to_vec();
    }
    let rotated = rotate_loop(cur, off);
    if corr_dist(prev, &rotated) < corr_dist(prev, cur) {
        rotated
    } else {
        cur.to_vec()
    }
}

/// Triangle with an outward normal from its winding (skips a degenerate one).
fn tri_poly(a: V3, b: V3, c: V3) -> Option<Polygon> {
    let nrm = b.sub(a).cross(c.sub(a));
    if nrm.len() < 1e-12 {
        return None;
    }
    let nrm = nrm.normalized();
    Some(Polygon::new(vec![
        Vertex::new(a, nrm),
        Vertex::new(b, nrm),
        Vertex::new(c, nrm),
    ]))
}

/// Skin consecutive equal-length rings of 3D points into an outward-wound tube
/// (ports core.py `skin_tube`). Between ring k and k+1, vertex j spans the quad
/// `[a[j], a[j+1], b[j+1], b[j]]`, split into two triangles so every face is
/// exactly planar (robust through the BSP booleans).
fn skin_rings(rings: &[Vec<V3>]) -> Vec<Polygon> {
    let mut polys = Vec::new();
    for k in 0..rings.len().saturating_sub(1) {
        let (a, b) = (&rings[k], &rings[k + 1]);
        let n = a.len().min(b.len());
        for j in 0..n {
            let j1 = (j + 1) % n;
            if let Some(p) = tri_poly(a[j], a[j1], b[j1]) {
                polys.push(p);
            }
            if let Some(p) = tri_poly(a[j], b[j1], b[j]) {
                polys.push(p);
            }
        }
    }
    polys
}

/// Cap a ring by triangulating its (CCW) 2D section loop and emitting the tris
/// at the ring's 3D positions (ports core.py `cap_annulus`, simple case). `flip`
/// reverses the winding for the trailing-facing cap so the normal points out.
fn cap_ring(loop2d: &[(f64, f64)], ring3d: &[V3], normal: V3, flip: bool) -> Vec<Polygon> {
    let tris = triangulate(loop2d);
    let mut polys = Vec::new();
    for t in &tris {
        let (i, j, k) = if flip { (t[0], t[2], t[1]) } else { (t[0], t[1], t[2]) };
        polys.push(Polygon::new(vec![
            Vertex::new(ring3d[i], normal),
            Vertex::new(ring3d[j], normal),
            Vertex::new(ring3d[k], normal),
        ]));
    }
    polys
}

/// Loft (skin) a solid through 2+ cross-sections stacked along Z, centered on
/// Z=0. Sections are resampled to a common vertex count by arc length, rolled
/// apex-to-apex, and DTW twist-aligned before skinning; `closed_caps` seals the
/// first and last rings into a watertight solid. Ports the harp "Clements 49"
/// loft pipeline (core.py `_resample`/`_roll`/`skin_tube`/`cap_annulus`).
pub fn loft(sections: &[Sketch], h: f64, closed_caps: bool) -> Vec<Polygon> {
    let n_sec = sections.len();
    if n_sec < 2 {
        return Vec::new();
    }
    let big_n = sections
        .iter()
        .map(|s| s.points.len())
        .max()
        .unwrap_or(3)
        .max(3);
    // Resample + roll each section to a common count and canonical start vertex.
    let mut loops2d: Vec<Vec<(f64, f64)>> = sections
        .iter()
        .map(|s| roll_canonical(&resample_loop(&ensure_ccw(&s.points), big_n)))
        .collect();
    // DTW twist-align each subsequent loop to its (finalized) predecessor.
    for k in 1..n_sec {
        let prev = loops2d[k - 1].clone();
        loops2d[k] = dtw_align(&prev, &loops2d[k]);
    }
    // Place each section in 3D at an evenly spaced, centered height.
    let z_at = |k: usize| -h / 2.0 + k as f64 / (n_sec as f64 - 1.0) * h;
    let rings: Vec<Vec<V3>> = loops2d
        .iter()
        .enumerate()
        .map(|(k, lp)| lp.iter().map(|&(x, y)| v3(x, y, z_at(k))).collect())
        .collect();
    let mut polys = skin_rings(&rings);
    if closed_caps {
        let last = n_sec - 1;
        polys.extend(cap_ring(&loops2d[0], &rings[0], v3(0.0, 0.0, -1.0), true));
        polys.extend(cap_ring(&loops2d[last], &rings[last], v3(0.0, 0.0, 1.0), false));
    }
    polys
}

/// Sweep a cross-section along a planar spine using a rotation-minimizing frame
/// (Wang et al. 2008 double-reflection) for minimal twist. `path` points are
/// read as `(x, z)` with `y = 0` (a spine in the XZ plane — enough for the harp
/// arms). `closed_caps` seals the two ends into a watertight solid.
pub fn sweep(section: &Sketch, path: &[(f64, f64)], closed_caps: bool) -> Vec<Polygon> {
    // Lift the planar (x, z) spine into 3-D and delegate to the general sweep.
    let p: Vec<V3> = path.iter().map(|&(x, z)| v3(x, 0.0, z)).collect();
    sweep_path(section, &p, closed_caps)
}

/// Sweep a cross-section along an arbitrary 3-D spine with a rotation-minimizing
/// frame (Wang et al. 2008 double-reflection) for minimal twist — enables
/// helical / twisting paths, not just planar ones. `closed_caps` seals the ends.
pub fn sweep_path(section: &Sketch, p: &[V3], closed_caps: bool) -> Vec<Polygon> {
    let m = p.len();
    if m < 2 || section.points.len() < 3 {
        return Vec::new();
    }
    // Unit tangents via central differences (one-sided at the ends).
    let tangent = |i: usize| -> V3 {
        let raw = if i == 0 {
            p[1].sub(p[0])
        } else if i == m - 1 {
            p[m - 1].sub(p[m - 2])
        } else {
            p[i + 1].sub(p[i - 1])
        };
        let t = raw.normalized();
        if t.len() < 0.5 {
            v3(0.0, 0.0, 1.0) // degenerate spacing → arbitrary but stable
        } else {
            t
        }
    };
    let t: Vec<V3> = (0..m).map(tangent).collect();
    // Initial reference vector r_0 ⟂ t_0 (project world +Y, else +X).
    let perp = |t0: V3| -> V3 {
        let y = v3(0.0, 1.0, 0.0);
        let r = y.sub(t0.mul(y.dot(t0)));
        if r.len() > 1e-6 {
            r.normalized()
        } else {
            let x = v3(1.0, 0.0, 0.0);
            x.sub(t0.mul(x.dot(t0))).normalized()
        }
    };
    let mut r = vec![v3(0.0, 0.0, 0.0); m];
    r[0] = perp(t[0]);
    // Propagate the frame with the double-reflection method.
    for i in 0..m - 1 {
        let v1 = p[i + 1].sub(p[i]);
        let c1 = v1.dot(v1);
        let (r_l, t_l) = if c1 > 1e-12 {
            (
                r[i].sub(v1.mul(2.0 / c1 * v1.dot(r[i]))),
                t[i].sub(v1.mul(2.0 / c1 * v1.dot(t[i]))),
            )
        } else {
            (r[i], t[i])
        };
        let v2 = t[i + 1].sub(t_l);
        let c2 = v2.dot(v2);
        let r_next = if c2 > 1e-12 {
            r_l.sub(v2.mul(2.0 / c2 * v2.dot(r_l)))
        } else {
            r_l
        };
        r[i + 1] = r_next.normalized();
    }
    // Section basis (r_i, s_i) at each station; s_i = t_i × r_i.
    let sect = ensure_ccw(&section.points);
    let rings: Vec<Vec<V3>> = (0..m)
        .map(|i| {
            let s = t[i].cross(r[i]);
            sect.iter()
                .map(|&(u, v)| p[i].add(r[i].mul(u)).add(s.mul(v)))
                .collect()
        })
        .collect();
    let mut polys = skin_rings(&rings);
    if closed_caps {
        polys.extend(cap_ring(&sect, &rings[0], t[0].negated(), true));
        polys.extend(cap_ring(&sect, &rings[m - 1], t[m - 1], false));
    }
    polys
}

/// Revolve a sketch's `(radius, height)` profile around the Z axis.
/// `seg` angular segments; full 360°. A closed profile yields a closed surface.
pub fn revolve(sketch: &Sketch, seg: usize) -> Vec<Polygon> {
    let profile = &sketch.points; // interpreted as (r, z)
    let n = profile.len();
    let seg = seg.max(3);
    let mut polys = Vec::new();

    let at = |r: f64, z: f64, ai: usize| {
        let a = ai as f64 / seg as f64 * 2.0 * PI;
        v3(r * a.cos(), r * a.sin(), z)
    };

    for i in 0..n {
        let j = (i + 1) % n;
        let (r0, z0) = profile[i];
        let (r1, z1) = profile[j];
        for s in 0..seg {
            let s1 = (s + 1) % seg;
            let p00 = at(r0, z0, s);
            let p01 = at(r0, z0, s1);
            let p10 = at(r1, z1, s);
            let p11 = at(r1, z1, s1);
            // Approximate outward normal from the quad.
            let build = |verts: Vec<V3>| {
                let nrm = verts[1]
                    .sub(verts[0])
                    .cross(verts[2].sub(verts[0]))
                    .normalized();
                Polygon::new(verts.into_iter().map(|p| Vertex::new(p, nrm)).collect())
            };
            if r0 < 1e-9 && r1 < 1e-9 {
                continue; // segment lies on the axis
            } else if r0 < 1e-9 {
                polys.push(build(vec![p10, p11, p01]));
            } else if r1 < 1e-9 {
                polys.push(build(vec![p00, p01, p11]));
            } else {
                polys.push(build(vec![p00, p01, p11, p10]));
            }
        }
    }
    polys
}

/// Partial revolve of an `(r, z)` profile through `deg` degrees about Z, with
/// flat end caps at angle 0 and angle `deg` so the result stays watertight
/// (pipe elbows, sectors). `deg >= 360` delegates to the full [`revolve`].
pub fn revolve_arc(sketch: &Sketch, seg: usize, deg: f64) -> Vec<Polygon> {
    if deg >= 359.999 {
        return revolve(sketch, seg);
    }
    let profile = &sketch.points;
    let n = profile.len();
    let seg = seg.max(2);
    let arc = deg.clamp(0.05, 359.999).to_radians();
    let mut polys = Vec::new();
    let at = |r: f64, z: f64, ai: usize| {
        let a = ai as f64 / seg as f64 * arc;
        v3(r * a.cos(), r * a.sin(), z)
    };
    let build = |verts: Vec<V3>| {
        let nrm = verts[1].sub(verts[0]).cross(verts[2].sub(verts[0])).normalized();
        Polygon::new(verts.into_iter().map(|p| Vertex::new(p, nrm)).collect())
    };
    // Swept side surface (no wrap-around).
    for i in 0..n {
        let j = (i + 1) % n;
        let (r0, z0) = profile[i];
        let (r1, z1) = profile[j];
        for s in 0..seg {
            let s1 = s + 1;
            let p00 = at(r0, z0, s);
            let p01 = at(r0, z0, s1);
            let p10 = at(r1, z1, s);
            let p11 = at(r1, z1, s1);
            if r0 < 1e-9 && r1 < 1e-9 {
                continue;
            } else if r0 < 1e-9 {
                polys.push(build(vec![p10, p11, p01]));
            } else if r1 < 1e-9 {
                polys.push(build(vec![p00, p01, p11]));
            } else {
                polys.push(build(vec![p00, p01, p11, p10]));
            }
        }
    }
    // Flat end caps: the profile region placed at angle 0 and angle `arc`,
    // each wound so its normal points away from the swept material.
    let place = |r: f64, z: f64, a: f64| v3(r * a.cos(), r * a.sin(), z);
    let tris = triangulate(profile);
    let mut cap = |a: f64, outward: V3| {
        for tri in &tris {
            let vs: Vec<V3> = tri.iter().map(|&k| place(profile[k].0, profile[k].1, a)).collect();
            let nrm = vs[1].sub(vs[0]).cross(vs[2].sub(vs[0]));
            if nrm.len() < 1e-12 {
                continue;
            }
            let flip = nrm.normalized().dot(outward) < 0.0;
            let ordered = if flip {
                vec![vs[0], vs[2], vs[1]]
            } else {
                vec![vs[0], vs[1], vs[2]]
            };
            polys.push(build(ordered));
        }
    };
    cap(0.0, v3(0.0, -1.0, 0.0)); // start face looks toward -angle (−Y)
    cap(arc, v3(-arc.sin(), arc.cos(), 0.0)); // end face looks toward +tangent
    polys
}
