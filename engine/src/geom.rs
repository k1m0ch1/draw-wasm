//! Pure geometry: bounds, hit-testing, arrow routing, pencil smoothing.

use proto::{Element, Kind};
use std::collections::HashMap;

pub type P = (f64, f64);
pub type Els = HashMap<String, Element>;

pub fn bindable(k: Kind) -> bool {
    matches!(k, Kind::Rect | Kind::Ellipse | Kind::Diamond | Kind::Image | Kind::Text | Kind::Html)
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

/// `ah` bits above the arrowheads (bit0 end, bit1 start): layout hint for mermaid edges, dashed, thick.
pub const AH_HINT_H: u8 = 4;
pub const AH_HINT_V: u8 = 8;
pub const AH_DASH: u8 = 16;
pub const AH_HEADS: u8 = 3;

pub type D = (f64, f64);
/// distance an arrow runs straight out of a node before it may turn
const STUB: f64 = 18.0;
/// clearance kept to unrelated nodes
const MARGIN: f64 = 14.0;
const GAP: f64 = 3.0;
const BEND: i64 = 40 * 8;
pub const WIN: f64 = 200.0;

/// Things an arrow must not run through (free text is not one: labels sit next to lines).
pub fn obstacle(k: Kind) -> bool {
    matches!(k, Kind::Rect | Kind::Ellipse | Kind::Diamond | Kind::Image | Kind::Html)
}

pub struct Ends {
    pub s: P,
    pub sd: Option<D>,
    pub e: P,
    pub ed: Option<D>,
}

fn sgn(v: f64) -> f64 {
    if v < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn side_pt(e: &Element, d: D) -> P {
    let c = center(e);
    (c.0 + d.0 * (e.w / 2.0 + GAP), c.1 + d.1 * (e.h / 2.0 + GAP))
}

/// Side of `a` that faces `t` (outward normal).
fn side_toward(a: &Element, t: P) -> D {
    let c = center(a);
    let (dx, dy) = (t.0 - c.0, t.1 - c.1);
    if (dx / (a.w / 2.0 + 1.0)).abs() >= (dy / (a.h / 2.0 + 1.0)).abs() {
        (sgn(dx), 0.0)
    } else {
        (0.0, sgn(dy))
    }
}

/// Anchor sides for two connected nodes: leave from the facing side, enter on the facing side.
fn sides(a: &Element, b: &Element, hint: u8) -> (D, D) {
    let (ac, bc) = (center(a), center(b));
    let gx = (b.x - (a.x + a.w)).max(a.x - (b.x + b.w));
    let gy = (b.y - (a.y + a.h)).max(a.y - (b.y + b.h));
    let horiz = if gx > 0.0 && gy > 0.0 {
        if hint & AH_HINT_H != 0 {
            true
        } else if hint & AH_HINT_V != 0 {
            false
        } else {
            gx >= gy
        }
    } else if gx > 0.0 {
        true
    } else if gy > 0.0 {
        false
    } else {
        (bc.0 - ac.0).abs() >= (bc.1 - ac.1).abs()
    };
    if horiz {
        let s = sgn(bc.0 - ac.0);
        ((s, 0.0), (-s, 0.0))
    } else {
        let s = sgn(bc.1 - ac.1);
        ((0.0, s), (0.0, -s))
    }
}

/// Where an arrow starts and ends, and which node sides it is attached to.
pub fn ends(els: &Els, a: &Element) -> Ends {
    if a.pts.len() < 4 {
        return Ends { s: (a.x, a.y), sd: None, e: (a.x, a.y), ed: None };
    }
    let (s0, e0) = ((a.pts[0], a.pts[1]), (a.pts[2], a.pts[3]));
    let sh = live_bound(els, &a.sb);
    let eh = live_bound(els, &a.eb);
    match (sh, eh) {
        (Some(x), Some(y)) => {
            let (d1, d2) = sides(x, y, a.ah);
            Ends { s: side_pt(x, d1), sd: Some(d1), e: side_pt(y, d2), ed: Some(d2) }
        }
        (Some(x), None) => {
            let d1 = side_toward(x, e0);
            Ends { s: side_pt(x, d1), sd: Some(d1), e: e0, ed: None }
        }
        (None, Some(y)) => {
            let d2 = side_toward(y, s0);
            Ends { s: s0, sd: None, e: side_pt(y, d2), ed: Some(d2) }
        }
        (None, None) => Ends { s: s0, sd: None, e: e0, ed: None },
    }
}

/// Effective arrow end points (bound ends sit on the facing side of their node).
pub fn arrow_ends(els: &Els, a: &Element) -> (P, P) {
    let n = ends(els, a);
    (n.s, n.e)
}

fn axis_dir(from: P, to: P) -> D {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if dx.abs() >= dy.abs() {
        (sgn(dx), 0.0)
    } else {
        (0.0, sgn(dy))
    }
}

/// Orthogonal route of an arrow as a polyline (first point = start, last = end).
/// `full` = search around other nodes; otherwise a cheap elbow (used while dragging, throttled).
pub fn route(els: &Els, a: &Element, full: bool) -> Vec<P> {
    let n = ends(els, a);
    if n.sd.is_none() && n.ed.is_none() {
        return vec![n.s, n.e];
    }
    let sd = n.sd.unwrap_or_else(|| axis_dir(n.s, n.e));
    let ed = n.ed.unwrap_or_else(|| axis_dir(n.e, n.s));
    let ss = if n.sd.is_some() { STUB } else { 0.0 };
    let es = if n.ed.is_some() { STUB } else { 0.0 };
    let s1 = (n.s.0 + sd.0 * ss, n.s.1 + sd.1 * ss);
    let e1 = (n.e.0 + ed.0 * es, n.e.1 + ed.1 * es);
    let found = if full { search(els, s1, sd, e1, ed) } else { None };
    let mid = found.unwrap_or_else(|| elbow(s1, sd, e1, ed));
    let mut pts = Vec::with_capacity(mid.len() + 2);
    pts.push(n.s);
    pts.extend(mid);
    pts.push(n.e);
    tidy(pts)
}

fn elbow(s1: P, sd: D, e1: P, ed: D) -> Vec<P> {
    if sd.0 != 0.0 {
        if ed.0 != 0.0 {
            let mx = (s1.0 + e1.0) / 2.0;
            vec![s1, (mx, s1.1), (mx, e1.1), e1]
        } else {
            vec![s1, (e1.0, s1.1), e1]
        }
    } else if ed.1 != 0.0 {
        let my = (s1.1 + e1.1) / 2.0;
        vec![s1, (s1.0, my), (e1.0, my), e1]
    } else {
        vec![s1, (s1.0, e1.1), e1]
    }
}

/// Drop duplicate and collinear points.
fn tidy(pts: Vec<P>) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len());
    for p in pts {
        if let Some(l) = out.last() {
            if (l.0 - p.0).abs() < 0.01 && (l.1 - p.1).abs() < 0.01 {
                continue;
            }
        }
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b.0 - a.0) * (p.1 - b.1) - (b.1 - a.1) * (p.0 - b.0);
            if cross.abs() < 0.01 {
                out.pop();
            } else {
                break;
            }
        }
        out.push(p);
    }
    out
}

pub type R = (f64, f64, f64, f64); // l, t, r, b

/// Index of the grid line nearest to `x`.
fn idx(v: &[f64], x: f64) -> usize {
    let i = v.partition_point(|a| *a < x).min(v.len() - 1);
    if i > 0 && (x - v[i - 1]).abs() <= (v[i] - x).abs() {
        i - 1
    } else {
        i
    }
}

/// Hash of everything that decides where an arrow may run: the geometry of all obstacles.
pub fn geo_hash(els: &Els) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x100000001b3);
    };
    let mut acc: u64 = 0;
    for e in els.values().filter(|e| !e.del && obstacle(e.k)) {
        // order independent: sum of per-element hashes
        let mut x: u64 = 0x9e3779b97f4a7c15;
        for v in [e.x, e.y, e.w, e.h] {
            x = (x ^ v.to_bits()).wrapping_mul(0x100000001b3).rotate_left(13);
        }
        acc = acc.wrapping_add(x);
    }
    mix(acc);
    h
}

/// Cache key of one arrow: its own inputs plus the obstacle geometry hash.
pub fn route_key(els: &Els, a: &Element, geo: u64) -> u64 {
    let mut h = geo;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x100000001b3);
    };
    for v in &a.pts {
        mix(v.to_bits());
    }
    mix(a.ah as u64);
    for id in [&a.sb, &a.eb] {
        match els.get(id.as_str()) {
            Some(n) => {
                for v in [n.x, n.y, n.w, n.h] {
                    mix(v.to_bits());
                }
                mix(n.del as u64);
            }
            None => mix(1),
        }
    }
    h
}

fn dedup(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v.dedup_by(|a, b| (*a - *b).abs() < 3.0);
    v
}

/// Make sure the exact stub coordinates are grid lines (a nearby line within the merge tolerance is replaced).
fn pin(v: &mut Vec<f64>, exact: &[f64]) {
    for &x in exact {
        v.retain(|a| (*a - x).abs() >= 3.0 || *a == x);
        if !v.contains(&x) {
            v.push(x);
        }
    }
    v.sort_by(|a, b| cmpf(*a, *b));
}

fn cmpf(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Shortest orthogonal path (length + bends) between two stub tips that keeps `MARGIN` to every node.
fn search(els: &Els, s1: P, sd: D, e1: P, ed: D) -> Option<Vec<P>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let win = (s1.0.min(e1.0) - WIN, s1.1.min(e1.1) - WIN, s1.0.max(e1.0) + WIN, s1.1.max(e1.1) + WIN);
    let mid = ((s1.0 + e1.0) / 2.0, (s1.1 + e1.1) / 2.0);
    let inside = |r: &R, p: P| p.0 > r.0 && p.0 < r.2 && p.1 > r.1 && p.1 < r.3;
    let mut obs: Vec<R> = els
        .values()
        .filter(|e| !e.del && obstacle(e.k))
        .map(|e| (e.x - MARGIN, e.y - MARGIN, e.x + e.w + MARGIN, e.y + e.h + MARGIN))
        .filter(|r| r.0 < win.2 && r.2 > win.0 && r.1 < win.3 && r.3 > win.1)
        .filter(|r| !inside(r, s1) && !inside(r, e1))
        .collect();
    let key = |r: &R| ((r.0 + r.2) / 2.0 - mid.0).powi(2) + ((r.1 + r.3) / 2.0 - mid.1).powi(2);
    obs.sort_by(|a, b| cmpf(key(a), key(b)).then(cmpf(a.0, b.0)).then(cmpf(a.1, b.1)));
    obs.truncate(32);
    // grid lines: endpoints, obstacle sides, and the middle of every corridor between them
    let mut xs = vec![s1.0, e1.0, mid.0];
    let mut ys = vec![s1.1, e1.1, mid.1];
    for r in &obs {
        xs.push(r.0);
        xs.push(r.2);
        ys.push(r.1);
        ys.push(r.3);
    }
    let corridors = |v: &[f64]| -> Vec<f64> {
        let d = dedup(v.to_vec());
        d.windows(2).filter(|w| w[1] - w[0] > 2.0 * MARGIN).map(|w| (w[0] + w[1]) / 2.0).collect()
    };
    let (cx, cy) = (corridors(&xs), corridors(&ys));
    xs.extend(cx);
    ys.extend(cy);
    let mut xs = dedup(xs);
    let mut ys = dedup(ys);
    pin(&mut xs, &[s1.0, e1.0]);
    pin(&mut ys, &[s1.1, e1.1]);
    let (nx, ny) = (xs.len(), ys.len());
    if nx < 2 || ny < 2 {
        return None;
    }
    let mut nb = vec![false; nx * ny];
    let mut hb = vec![false; (nx - 1) * ny]; // segment (i,j)-(i+1,j)
    let mut vb = vec![false; nx * (ny - 1)]; // segment (i,j)-(i,j+1)
    for r in &obs {
        let (il, ir, jt, jb) = (idx(&xs, r.0), idx(&xs, r.2), idx(&ys, r.1), idx(&ys, r.3));
        for j in jt + 1..jb {
            for i in il + 1..ir {
                nb[j * nx + i] = true;
            }
            for i in il..ir {
                hb[j * (nx - 1) + i] = true;
            }
        }
        for i in il + 1..ir {
            for j in jt..jb {
                vb[i * (ny - 1) + j] = true;
            }
        }
    }
    const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let dir_of = |d: D| -> usize {
        if d.0 > 0.0 {
            0
        } else if d.0 < 0.0 {
            1
        } else if d.1 > 0.0 {
            2
        } else {
            3
        }
    };
    let (is, js) = (idx(&xs, s1.0), idx(&ys, s1.1));
    let (ie, je) = (idx(&xs, e1.0), idx(&ys, e1.1));
    let d0 = dir_of(sd);
    let d_in = dir_of((-ed.0, -ed.1)); // direction of the last move into the end stub
    let st = |i: usize, j: usize, d: usize| (j * nx + i) * 4 + d;
    let mut dist = vec![i64::MAX; nx * ny * 4];
    let mut prev = vec![u32::MAX; nx * ny * 4];
    let mut heap = BinaryHeap::new();
    dist[st(is, js, d0)] = 0;
    heap.push(Reverse((0i64, st(is, js, d0) as u32)));
    let mut best: Option<(i64, usize)> = None;
    while let Some(Reverse((c, u))) = heap.pop() {
        let u = u as usize;
        if c > dist[u] {
            continue;
        }
        if let Some((b, _)) = best {
            if c >= b {
                break;
            }
        }
        let d = u % 4;
        let cell = u / 4;
        let (i, j) = (cell % nx, cell / nx);
        if i == ie && j == je && d != (d_in ^ 1) {
            let t = c + if d != d_in { BEND } else { 0 };
            if best.map_or(true, |(b, _)| t < b) {
                best = Some((t, u));
            }
        }
        for (nd, (dx, dy)) in DIRS.iter().enumerate() {
            if nd == (d ^ 1) {
                continue; // no U-turns
            }
            let (ni, nj) = (i as i32 + dx, j as i32 + dy);
            if ni < 0 || nj < 0 || ni >= nx as i32 || nj >= ny as i32 {
                continue;
            }
            let (ni, nj) = (ni as usize, nj as usize);
            if nb[nj * nx + ni] {
                continue;
            }
            let (blocked, len) = match nd {
                0 => (hb[j * (nx - 1) + i], xs[i + 1] - xs[i]),
                1 => (hb[j * (nx - 1) + ni], xs[i] - xs[ni]),
                2 => (vb[i * (ny - 1) + j], ys[j + 1] - ys[j]),
                _ => (vb[i * (ny - 1) + nj], ys[j] - ys[nj]),
            };
            if blocked {
                continue;
            }
            let v = st(ni, nj, nd);
            let cost = c + (len * 8.0) as i64 + if nd != d { BEND } else { 0 };
            if cost < dist[v] {
                dist[v] = cost;
                prev[v] = u as u32;
                heap.push(Reverse((cost, v as u32)));
            }
        }
    }
    let (_, mut u) = best?;
    let mut out = vec![];
    loop {
        let cell = u / 4;
        out.push((xs[cell % nx], ys[cell / nx]));
        if prev[u] == u32::MAX {
            break;
        }
        u = prev[u] as usize;
    }
    out.reverse();
    // snap the end points to the exact stub tips
    if let Some(f) = out.first_mut() {
        *f = s1;
    }
    if let Some(l) = out.last_mut() {
        *l = e1;
    }
    Some(out)
}

/// Rough text size used to place edge labels (no font metrics in this layer).
pub fn label_size(text: &str, fs: f64) -> (f64, f64) {
    let lines: Vec<&str> = text.split('\n').collect();
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * fs * 0.56;
    (w + 10.0, lines.len() as f64 * fs * 1.3 + 4.0)
}

/// Where the centre of an arrow label goes: beside the longest stretches of the route (never on the line),
/// clear of every node and of the labels in `taken` (l, t, r, b).
pub fn label_pos(els: &Els, route: &[P], text: &str, fs: f64, taken: &[R]) -> P {
    if route.len() < 2 {
        return route.first().copied().unwrap_or((0.0, 0.0));
    }
    let (tw, th) = label_size(text, fs);
    let mut segs: Vec<(f64, P, bool)> = route
        .windows(2)
        .map(|w| {
            let horiz = (w[1].1 - w[0].1).abs() < 0.01;
            ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs(), ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0), horiz)
        })
        .collect();
    segs.sort_by(|a, b| cmpf(b.0, a.0));
    let clear = |c: P| {
        let (l, t, r, b) = (c.0 - tw / 2.0, c.1 - th / 2.0, c.0 + tw / 2.0, c.1 + th / 2.0);
        !els.values().any(|e| !e.del && obstacle(e.k) && e.x < r && e.x + e.w > l && e.y < b && e.y + e.h > t)
            && !taken.iter().any(|q| q.0 < r && q.2 > l && q.1 < b && q.3 > t)
    };
    for (_, c, horiz) in &segs {
        let cands = if *horiz {
            [(c.0, c.1 - th / 2.0 - 3.0), (c.0, c.1 + th / 2.0 + 3.0)]
        } else {
            [(c.0 + tw / 2.0 + 6.0, c.1), (c.0 - tw / 2.0 - 6.0, c.1)]
        };
        for q in cands {
            if clear(q) {
                return q;
            }
        }
    }
    let (_, c, horiz) = segs[0];
    if horiz {
        (c.0, c.1 - th / 2.0 - 3.0)
    } else {
        (c.0 + tw / 2.0 + 6.0, c.1)
    }
}

/// Distance from `p` to a polyline.
pub fn dist_poly(p: P, pts: &[P]) -> f64 {
    pts.windows(2).map(|w| dist_seg(p, w[0], w[1])).fold(f64::MAX, f64::min)
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
        Kind::Rect | Kind::Image | Kind::Text | Kind::Html => {
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
            dist_poly(p, &route(els, e, false)) <= tol + e.sw / 2.0
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, x: f64, y: f64) -> Element {
        Element { id: id.into(), k: Kind::Rect, x, y, w: 120.0, h: 60.0, ..Default::default() }
    }
    fn arrow(a: &str, b: &str) -> Element {
        Element { id: format!("{a}{b}"), k: Kind::Arrow, sb: a.into(), eb: b.into(), pts: vec![0.0, 0.0, 1.0, 1.0], ..Default::default() }
    }
    fn world(v: Vec<Element>) -> Els {
        v.into_iter().map(|e| (e.id.clone(), e)).collect()
    }
    fn crosses(r: &[P], e: &Element) -> bool {
        // does any segment pass through the interior of the node?
        r.windows(2).any(|w| {
            let (x0, x1) = (w[0].0.min(w[1].0), w[0].0.max(w[1].0));
            let (y0, y1) = (w[0].1.min(w[1].1), w[0].1.max(w[1].1));
            x0 < e.x + e.w && x1 > e.x && y0 < e.y + e.h && y1 > e.y
        })
    }
    fn orthogonal(r: &[P]) -> bool {
        r.windows(2).all(|w| (w[0].0 - w[1].0).abs() < 0.01 || (w[0].1 - w[1].1).abs() < 0.01)
    }

    #[test]
    fn leaves_from_the_facing_side() {
        let els = world(vec![node("a", 0.0, 0.0), node("b", 300.0, 0.0), arrow("a", "b")]);
        let r = route(&els, &els["ab"], true);
        assert!(orthogonal(&r));
        assert!((r[0].0 - 123.0).abs() < 0.1 && (r[0].1 - 30.0).abs() < 0.1, "{:?}", r);
        assert!((r[r.len() - 1].0 - 297.0).abs() < 0.1);
    }

    #[test]
    fn avoids_a_node_in_the_way() {
        let els = world(vec![node("a", 0.0, 0.0), node("m", 250.0, 0.0), node("b", 500.0, 0.0), arrow("a", "b")]);
        let r = route(&els, &els["ab"], true);
        assert!(orthogonal(&r), "{:?}", r);
        assert!(!crosses(&r, &els["m"]), "{:?}", r);
        assert!(r.len() <= 6, "single corridor, not a weave: {:?}", r);
    }

    #[test]
    fn long_edge_across_rows_stays_in_one_corridor() {
        let mut v = vec![];
        for row in 0..5 {
            for col in 0..3 {
                v.push(node(&format!("n{row}{col}"), col as f64 * 220.0, row as f64 * 140.0));
            }
        }
        v.push(arrow("n00", "n40"));
        let els = world(v);
        let r = route(&els, &els["n00n40"], true);
        assert!(orthogonal(&r), "{:?}", r);
        for e in els.values().filter(|e| e.k == Kind::Rect) {
            assert!(!crosses(&r, e), "crosses {}: {:?}", e.id, r);
        }
        assert!(r.len() <= 6, "{:?}", r);
    }

    #[test]
    fn free_arrows_stay_straight() {
        let a = Element { id: "f".into(), k: Kind::Arrow, pts: vec![0.0, 0.0, 50.0, 80.0], ..Default::default() };
        let els = world(vec![a.clone()]);
        assert_eq!(route(&els, &a, true).len(), 2);
    }
}
