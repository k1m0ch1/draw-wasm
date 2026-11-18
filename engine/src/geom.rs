//! Pure geometry: bounds, hit-testing, arrow routing, pencil smoothing.

use proto::{Element, Kind};
use std::collections::HashMap;

pub type P = (f64, f64);
pub type Els = HashMap<String, Element>;

pub fn bindable(k: Kind) -> bool {
    matches!(k, Kind::Rect | Kind::Ellipse | Kind::Diamond | Kind::Image | Kind::Text)
}

pub fn center(e: &Element) -> P {
    (e.x + e.w / 2.0, e.y + e.h / 2.0)
}

/// Bounding box (x, y, w, h) in world coordinates.
pub fn bbox(e: &Element) -> (f64, f64, f64, f64) {
    if e.k == Kind::Arrow && e.pts.len() >= 4 {
        let (x1, y1, x2, y2) = (e.pts[0], e.pts[1], e.pts[2], e.pts[3]);
        let (x, y) = (x1.min(x2), y1.min(y2));
        return (x, y, (x1 - x2).abs(), (y1 - y2).abs());
    }
    (e.x, e.y, e.w, e.h)
}

pub fn union(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    let r = (a.0 + a.2).max(b.0 + b.2);
    let btm = (a.1 + a.3).max(b.1 + b.3);
    (x, y, r - x, btm - y)
}

/// Point on the outline of `e` along the ray from its centre towards `toward`, pushed out by `gap`.
pub fn boundary_point(e: &Element, toward: P, gap: f64) -> P {
    let (cx, cy) = center(e);
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 {
        return (cx, cy);
    }
    let (a, b) = ((e.w / 2.0).max(1e-6), (e.h / 2.0).max(1e-6));
    let t = match e.k {
        Kind::Ellipse => 1.0 / ((dx / a).powi(2) + (dy / b).powi(2)).sqrt(),
        Kind::Diamond => 1.0 / (dx.abs() / a + dy.abs() / b),
        _ => {
            let tx = if dx.abs() < 1e-9 { f64::MAX } else { a / dx.abs() };
            let ty = if dy.abs() < 1e-9 { f64::MAX } else { b / dy.abs() };
            tx.min(ty)
        }
    };
    let t = t.min(len / len.max(1.0) * 1e9);
    let d = (t * len + gap).min(len); // never past the target
    (cx + dx / len * d, cy + dy / len * d)
}

fn live_bound<'a>(els: &'a Els, id: &str) -> Option<&'a Element> {
    if id.is_empty() {
        return None;
    }
    els.get(id).filter(|e| !e.del && bindable(e.k))
}

/// Effective arrow end points (bound ends follow their shapes).
pub fn arrow_ends(els: &Els, a: &Element) -> (P, P) {
    if a.pts.len() < 4 {
        return ((a.x, a.y), (a.x, a.y));
    }
    let (s, e) = ((a.pts[0], a.pts[1]), (a.pts[2], a.pts[3]));
    let sh = live_bound(els, &a.sb);
    let eh = live_bound(els, &a.eb);
    let s_ref = eh.map(center).unwrap_or(e);
    let e_ref = sh.map(center).unwrap_or(s);
    let gap = 3.0;
    let s2 = sh.map(|x| boundary_point(x, s_ref, -gap)).unwrap_or(s);
    let e2 = eh.map(|x| boundary_point(x, e_ref, -gap)).unwrap_or(e);
    (s2, e2)
}

pub fn dist_seg(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-12 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

pub fn hit(els: &Els, e: &Element, p: P, tol: f64) -> bool {
    match e.k {
        Kind::Rect | Kind::Image | Kind::Text => {
            p.0 >= e.x - tol && p.0 <= e.x + e.w + tol && p.1 >= e.y - tol && p.1 <= e.y + e.h + tol
        }
        Kind::Ellipse => {
            let (cx, cy) = center(e);
            let (a, b) = (e.w / 2.0 + tol, e.h / 2.0 + tol);
            ((p.0 - cx) / a).powi(2) + ((p.1 - cy) / b).powi(2) <= 1.0
        }
        Kind::Diamond => {
            let (cx, cy) = center(e);
            let (a, b) = (e.w / 2.0 + tol, e.h / 2.0 + tol);
            (p.0 - cx).abs() / a + (p.1 - cy).abs() / b <= 1.0
        }
        Kind::Pencil => {
            let t = tol + e.sw / 2.0;
            let n = e.pts.len() / 2;
            if n == 1 {
                return ((p.0 - e.x - e.pts[0]).powi(2) + (p.1 - e.y - e.pts[1]).powi(2)).sqrt() <= t;
            }
            (1..n).any(|i| {
                dist_seg(
                    p,
                    (e.x + e.pts[i * 2 - 2], e.y + e.pts[i * 2 - 1]),
                    (e.x + e.pts[i * 2], e.y + e.pts[i * 2 + 1]),
                )
                .le(&t)
            })
        }
        Kind::Arrow => {
            let (s, t) = arrow_ends(els, e);
            dist_seg(p, s, t) <= tol + e.sw / 2.0
        }
    }
}

/// Does the element's bbox intersect the rectangle?
pub fn intersects(e: &Element, els: &Els, r: (f64, f64, f64, f64)) -> bool {
    let b = if e.k == Kind::Arrow {
        let (s, t) = arrow_ends(els, e);
        (s.0.min(t.0), s.1.min(t.1), (s.0 - t.0).abs(), (s.1 - t.1).abs())
    } else {
        bbox(e)
    };
    b.0 <= r.0 + r.2 && b.0 + b.2 >= r.0 && b.1 <= r.1 + r.3 && b.1 + b.3 >= r.1
}

/// Ramer-Douglas-Peucker on a flat [x0,y0,x1,y1,...] list.
pub fn simplify(pts: &[f64], eps: f64) -> Vec<f64> {
    let n = pts.len() / 2;
    if n < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = ((pts[a * 2], pts[a * 2 + 1]), (pts[b * 2], pts[b * 2 + 1]));
        let (mut md, mut mi) = (0.0, 0);
        for i in a + 1..b {
            let d = dist_seg((pts[i * 2], pts[i * 2 + 1]), pa, pb);
            if d > md {
                md = d;
                mi = i;
            }
        }
        if md > eps {
            keep[mi] = true;
            stack.push((a, mi));
            stack.push((mi, b));
        }
    }
    let mut out = Vec::new();
    for i in 0..n {
        if keep[i] {
            out.push(pts[i * 2]);
            out.push(pts[i * 2 + 1]);
        }
    }
    out
}
