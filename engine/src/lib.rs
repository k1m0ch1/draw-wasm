//! draw-wasm engine: scene model, hit-testing, transforms, arrow binding, pencil smoothing,
//! undo/redo, Canvas2D rendering (through the small `Gfx` surface) and serialization.

mod geom;
mod gfx;
mod mermaid;

use geom::*;
use gfx::Gfx;
use proto::{Element, Kind};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Math)]
    fn random() -> f64;
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance)]
    fn now() -> f64;
}

const ACCENT: &str = "#4f6bed";
const HANDLE: [(f64, f64); 8] = [
    (0.0, 0.0),
    (0.5, 0.0),
    (1.0, 0.0),
    (1.0, 0.5),
    (1.0, 1.0),
    (0.5, 1.0),
    (0.0, 1.0),
    (0.0, 0.5),
];
const CURSORS: [&str; 8] = [
    "nwse-resize",
    "ns-resize",
    "nesw-resize",
    "ew-resize",
    "nwse-resize",
    "ns-resize",
    "nesw-resize",
    "ew-resize",
];

type BB = (f64, f64, f64, f64);

#[derive(Clone)]
struct Change {
    id: String,
    before: Option<Element>,
    after: Option<Element>,
}

enum G {
    None,
    Move { start: P, orig: Vec<Element>, moved: bool },
    Resize { h: usize, orig: Vec<Element>, ob: BB },
    Endpoint { id: String, end: usize },
    Marquee { start: P, cur: P, add: bool },
    Create { start: P, cur: P },
    Pencil { pts: Vec<f64> },
    Arrow { start: P, sb: String, cur: P, eb: String },
}

#[wasm_bindgen]
pub struct Board {
    cid: String,
    els: Els,
    order: Vec<String>,
    order_dirty: bool,
    outbox: BTreeSet<String>,
    sel: Vec<String>,
    tool: String,
    // view
    zoom: f64,
    px: f64,
    py: f64,
    // style defaults
    st: String,
    fl: String,
    sw: f64,
    ah: u8,
    fs: f64,
    // gesture
    g: G,
    tx: Option<HashMap<String, Option<Element>>>,
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
    clip: Vec<Element>,
    clip_src: String,
    // flags
    need_render: bool,
    doc_rev: u32,
    sel_rev: u32,
    cursor: String,
    // text editing
    pending: Option<Element>,
    edit_req: String,
    editing: String,
    /// read-only: local mutations are refused (the server enforces this too)
    ro: bool,
    /// arrow route cache: id -> (input key, time computed, polyline)
    rc: RefCell<HashMap<String, (u64, f64, Vec<P>)>>,
    /// obstacle geometry hash, valid for the document revision stored with it
    geo: std::cell::Cell<(u32, u64)>,
}

fn rid() -> String {
    const A: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..12).map(|_| A[(random() * 36.0) as usize % 36] as char).collect()
}

fn same(a: &Element, b: &Element) -> bool {
    let (mut x, mut y) = (a.clone(), b.clone());
    x.v = 0;
    y.v = 0;
    x.by.clear();
    y.by.clear();
    x == y
}

fn norm(a: P, b: P) -> BB {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

#[wasm_bindgen]
impl Board {
    #[wasm_bindgen(constructor)]
    pub fn new(cid: &str) -> Board {
        Board {
            cid: cid.to_string(),
            els: HashMap::new(),
            order: vec![],
            order_dirty: true,
            outbox: BTreeSet::new(),
            sel: vec![],
            tool: "select".into(),
            zoom: 1.0,
            px: 0.0,
            py: 0.0,
            st: "#1f2937".into(),
            fl: "#dbeafe".into(),
            sw: 2.0,
            ah: 1,
            fs: 18.0,
            g: G::None,
            tx: None,
            undo: vec![],
            redo: vec![],
            clip: vec![],
            clip_src: String::new(),
            need_render: true,
            doc_rev: 0,
            sel_rev: 0,
            cursor: "default".into(),
            pending: None,
            edit_req: String::new(),
            editing: String::new(),
            ro: false,
            rc: RefCell::new(HashMap::new()),
            geo: std::cell::Cell::new((u32::MAX, 0)),
        }
    }
    pub fn set_readonly(&mut self, ro: bool) {
        self.ro = ro;
        if ro {
            self.cancel();
        }
        self.need_render = true;
    }
    pub fn readonly(&self) -> bool {
        self.ro
    }

    // ------------------------------------------------------------ flags
    pub fn take_render(&mut self) -> bool {
        std::mem::replace(&mut self.need_render, false)
    }
    pub fn invalidate(&mut self) {
        self.need_render = true;
    }
    /// bumps on every change of the document (local or remote)
    pub fn doc_rev(&self) -> u32 {
        self.doc_rev
    }
    pub fn sel_rev(&self) -> u32 {
        self.sel_rev
    }
    pub fn cursor(&self) -> String {
        self.cursor.clone()
    }
    pub fn tool(&self) -> String {
        self.tool.clone()
    }
    pub fn set_tool(&mut self, t: &str) {
        self.cancel();
        self.tool = t.to_string();
        if t != "select" {
            self.set_sel(vec![]);
        }
        self.cursor = self.tool_cursor();
        self.need_render = true;
    }
    fn tool_cursor(&self) -> String {
        match self.tool.as_str() {
            "select" => "default",
            "text" => "text",
            _ => "crosshair",
        }
        .into()
    }
    pub fn zoom(&self) -> f64 {
        self.zoom
    }
    pub fn pan_x(&self) -> f64 {
        self.px
    }
    pub fn pan_y(&self) -> f64 {
        self.py
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn touch(&mut self) {
        self.order_dirty = true;
        self.need_render = true;
        self.doc_rev = self.doc_rev.wrapping_add(1);
    }
    fn set_sel(&mut self, s: Vec<String>) {
        if s != self.sel {
            self.sel = s;
            self.sel_rev = self.sel_rev.wrapping_add(1);
        }
        self.need_render = true;
    }

    // ------------------------------------------------------------ view
    pub fn w2s(&self, x: f64, y: f64) -> Vec<f64> {
        vec![x * self.zoom + self.px, y * self.zoom + self.py]
    }
    fn s2w(&self, x: f64, y: f64) -> P {
        ((x - self.px) / self.zoom, (y - self.py) / self.zoom)
    }
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.px += dx;
        self.py += dy;
        self.need_render = true;
    }
    pub fn zoom_at(&mut self, sx: f64, sy: f64, factor: f64) {
        let z = (self.zoom * factor).clamp(0.1, 4.0);
        let k = z / self.zoom;
        self.px = sx - (sx - self.px) * k;
        self.py = sy - (sy - self.py) * k;
        self.zoom = z;
        self.need_render = true;
    }
    /// Set an exact zoom level (clamped to 10%..400%) keeping the screen point (sx, sy) fixed.
    pub fn set_zoom(&mut self, sx: f64, sy: f64, z: f64) {
        let z = z.clamp(0.1, 4.0);
        let k = z / self.zoom;
        self.px = sx - (sx - self.px) * k;
        self.py = sy - (sy - self.py) * k;
        self.zoom = z;
        self.need_render = true;
    }
    pub fn fit(&mut self, vw: f64, vh: f64) {
        let mut b: Option<BB> = None;
        for e in self.els.values().filter(|e| !e.del) {
            let eb = self.el_bb(e);
            b = Some(b.map_or(eb, |u| union(u, eb)));
        }
        match b {
            None => {
                self.zoom = 1.0;
                self.px = 0.0;
                self.py = 0.0;
            }
            Some((x, y, w, h)) => {
                let pad = 60.0;
                let z = ((vw - pad * 2.0) / w.max(1.0)).min((vh - pad * 2.0) / h.max(1.0));
                self.zoom = z.clamp(0.1, 1.5);
                self.px = vw / 2.0 - (x + w / 2.0) * self.zoom;
                self.py = vh / 2.0 - (y + h / 2.0) * self.zoom;
            }
        }
        self.need_render = true;
    }

    // ------------------------------------------------------------ order / queries
    fn reorder(&mut self) {
        if !self.order_dirty {
            return;
        }
        let mut v: Vec<&Element> = self.els.values().filter(|e| !e.del).collect();
        v.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal).then(a.id.cmp(&b.id)));
        self.order = v.into_iter().map(|e| e.id.clone()).collect();
        self.order_dirty = false;
    }
    fn top_z(&self) -> f64 {
        self.els.values().filter(|e| !e.del).map(|e| e.z).fold(0.0, f64::max) + 1.0
    }
    fn bottom_z(&self) -> f64 {
        self.els.values().filter(|e| !e.del).map(|e| e.z).fold(0.0, f64::min) - 1.0
    }
    fn hit_at(&mut self, p: P, bindable_only: bool, exclude: &str) -> Option<String> {
        self.reorder();
        let tol = 5.0 / self.zoom;
        for id in self.order.iter().rev() {
            let e = &self.els[id];
            if id == exclude || (bindable_only && !bindable(e.k)) {
                continue;
            }
            let h = if e.k == Kind::Arrow {
                let b = self.arrow_reach(e);
                p.0 >= b.0 && p.0 <= b.0 + b.2 && p.1 >= b.1 && p.1 <= b.1 + b.3 && dist_poly(p, &self.route_of(e)) <= tol + e.sw / 2.0
            } else {
                hit(&self.els, e, p, tol)
            };
            if h {
                return Some(id.clone());
            }
        }
        None
    }
    /// Cheap box that contains the whole route of an arrow (its ends plus the detour allowance).
    fn arrow_reach(&self, e: &Element) -> BB {
        let (s, t) = arrow_ends(&self.els, e);
        let b = norm(s, t);
        (b.0 - WIN - 20.0, b.1 - WIN - 20.0, b.2 + 2.0 * (WIN + 20.0), b.3 + 2.0 * (WIN + 20.0))
    }
    /// Bounding box including the whole route for arrows.
    fn el_bb(&self, e: &Element) -> BB {
        if e.k == Kind::Arrow {
            let r = self.route_of(e);
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for q in &r {
                x0 = x0.min(q.0);
                y0 = y0.min(q.1);
                x1 = x1.max(q.0);
                y1 = y1.max(q.1);
            }
            if x0 > x1 {
                return bbox(e);
            }
            (x0, y0, x1 - x0, y1 - y0)
        } else {
            bbox(e)
        }
    }
    fn sel_bbox(&self) -> Option<BB> {
        let mut b: Option<BB> = None;
        for id in &self.sel {
            if let Some(e) = self.els.get(id).filter(|e| !e.del) {
                let eb = self.el_bb(e);
                b = Some(b.map_or(eb, |u| union(u, eb)));
            }
        }
        b
    }
    fn single_arrow(&self) -> Option<&Element> {
        if self.sel.len() == 1 {
            self.els.get(&self.sel[0]).filter(|e| e.k == Kind::Arrow && !e.del)
        } else {
            None
        }
    }
    fn handle_at(&self, p: P) -> Option<usize> {
        let b = self.sel_bbox()?;
        if self.single_arrow().is_some() {
            return None;
        }
        let r = 8.0 / self.zoom;
        for (i, h) in HANDLE.iter().enumerate() {
            let (hx, hy) = (b.0 + b.2 * h.0, b.1 + b.3 * h.1);
            if (p.0 - hx).abs() <= r && (p.1 - hy).abs() <= r {
                return Some(i);
            }
        }
        None
    }
    fn endpoint_at(&self, p: P) -> Option<(String, usize)> {
        let a = self.single_arrow()?;
        let (s, e) = arrow_ends(&self.els, a);
        let r = 9.0 / self.zoom;
        let d = |q: P| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
        if d(e) <= r {
            Some((a.id.clone(), 1))
        } else if d(s) <= r {
            Some((a.id.clone(), 0))
        } else {
            None
        }
    }

    // ------------------------------------------------------------ mutation core
    fn begin_tx(&mut self) {
        if self.tx.is_none() {
            self.tx = Some(HashMap::new());
        }
    }
    fn end_tx(&mut self) {
        if let Some(tx) = self.tx.take() {
            let mut ch = vec![];
            for (id, before) in tx {
                let after = self.els.get(&id).cloned();
                let diff = match (&before, &after) {
                    (Some(b), Some(a)) => !same(a, b),
                    (None, None) => false,
                    _ => true,
                };
                if diff {
                    ch.push(Change { id, before, after });
                }
            }
            if !ch.is_empty() {
                self.undo.push(ch);
                if self.undo.len() > 100 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
        }
    }
    /// Write an element (bumps version, sets writer, queues for sync, records history).
    fn put(&mut self, mut e: Element) {
        if self.ro {
            return;
        }
        let id = e.id.clone();
        let old = self.els.get(&id).cloned();
        if let Some(tx) = self.tx.as_mut() {
            tx.entry(id.clone()).or_insert_with(|| old.clone());
        }
        e.v = old.as_ref().map_or(0, |o| o.v) + 1;
        e.by = self.cid.clone();
        if e.del {
            e.pts.clear();
            e.tx.clear();
        }
        self.els.insert(id.clone(), e);
        self.outbox.insert(id);
        self.touch();
    }
    fn remove_ids(&mut self, ids: &[String]) {
        let set: BTreeSet<&String> = ids.iter().collect();
        // detach arrows bound to removed shapes (freeze their current position)
        let arrows: Vec<Element> = self
            .els
            .values()
            .filter(|a| a.k == Kind::Arrow && !a.del && !set.contains(&a.id) && (set.contains(&a.sb) || set.contains(&a.eb)))
            .cloned()
            .collect();
        for mut a in arrows {
            let (s, e) = arrow_ends(&self.els, &a);
            a.pts = vec![s.0, s.1, e.0, e.1];
            a.sb.clear();
            a.eb.clear();
            self.put(a);
        }
        for id in ids {
            if let Some(mut e) = self.els.get(id).cloned() {
                if !e.del {
                    e.del = true;
                    self.put(e);
                }
            }
        }
    }
    fn new_el(&self, k: Kind) -> Element {
        Element {
            id: rid(),
            k,
            z: self.top_z(),
            st: self.st.clone(),
            fl: if matches!(k, Kind::Rect | Kind::Ellipse | Kind::Diamond) { self.fl.clone() } else { String::new() },
            sw: self.sw,
            ah: if k == Kind::Arrow { self.ah } else { 0 },
            fs: if matches!(k, Kind::Text | Kind::Rect | Kind::Ellipse | Kind::Diamond) { self.fs } else { 0.0 },
            ..Default::default()
        }
    }

    // ------------------------------------------------------------ pointer
    pub fn pointer_down(&mut self, sx: f64, sy: f64, shift: bool) {
        let p = self.s2w(sx, sy);
        self.cancel();
        if self.ro && self.tool != "select" {
            return;
        }
        match self.tool.as_str() {
            "select" => self.down_select(p, shift),
            "rect" | "ellipse" | "diamond" => {
                self.set_sel(vec![]);
                self.g = G::Create { start: p, cur: p };
            }
            "arrow" => {
                let sb = self.hit_at(p, true, "").unwrap_or_default();
                self.g = G::Arrow { start: p, sb, cur: p, eb: String::new() };
            }
            "pencil" => {
                self.set_sel(vec![]);
                self.g = G::Pencil { pts: vec![p.0, p.1] };
            }
            "text" => {
                self.start_text(p);
            }
            _ => {}
        }
        self.need_render = true;
    }

    fn down_select(&mut self, p: P, shift: bool) {
        if let Some((id, end)) = self.endpoint_at(p) {
            self.begin_tx();
            self.g = G::Endpoint { id, end };
            return;
        }
        if let Some(h) = self.handle_at(p) {
            if let Some(ob) = self.sel_bbox() {
                let orig: Vec<Element> = self.sel.iter().filter_map(|i| self.els.get(i).cloned()).collect();
                self.begin_tx();
                self.g = G::Resize { h, orig, ob };
                return;
            }
        }
        match self.hit_at(p, false, "") {
            Some(id) => {
                if shift {
                    let mut s = self.sel.clone();
                    if let Some(i) = s.iter().position(|x| *x == id) {
                        s.remove(i);
                    } else {
                        s.push(id);
                    }
                    self.set_sel(s);
                } else if !self.sel.contains(&id) {
                    self.set_sel(vec![id]);
                }
                let orig: Vec<Element> = self.sel.iter().filter_map(|i| self.els.get(i).cloned()).collect();
                if !orig.is_empty() {
                    self.begin_tx();
                    self.g = G::Move { start: p, orig, moved: false };
                }
            }
            None => {
                if !shift {
                    self.set_sel(vec![]);
                }
                self.g = G::Marquee { start: p, cur: p, add: shift };
            }
        }
    }

    pub fn pointer_move(&mut self, sx: f64, sy: f64) {
        let p = self.s2w(sx, sy);
        let zoom = self.zoom;
        match std::mem::replace(&mut self.g, G::None) {
            G::None => {
                self.hover(p);
            }
            G::Move { start, orig, mut moved } => {
                let (dx, dy) = (p.0 - start.0, p.1 - start.1);
                if !moved && (dx * dx + dy * dy).sqrt() * zoom < 3.0 {
                    self.g = G::Move { start, orig, moved };
                    return;
                }
                moved = true;
                let ids: BTreeSet<String> = orig.iter().map(|e| e.id.clone()).collect();
                for o in &orig {
                    let mut e = o.clone();
                    if e.k == Kind::Arrow {
                        for i in 0..2 {
                            if e.pts.len() >= 4 {
                                e.pts[i * 2] += dx;
                                e.pts[i * 2 + 1] += dy;
                            }
                        }
                        if !ids.contains(&e.sb) {
                            // a bound end detaches from a shape that is not moving with it
                            if !e.sb.is_empty() {
                                let (s, _) = arrow_ends(&self.els, o);
                                e.pts[0] = s.0 + dx;
                                e.pts[1] = s.1 + dy;
                            }
                            e.sb.clear();
                        }
                        if !ids.contains(&e.eb) {
                            if !e.eb.is_empty() {
                                let (_, t) = arrow_ends(&self.els, o);
                                e.pts[2] = t.0 + dx;
                                e.pts[3] = t.1 + dy;
                            }
                            e.eb.clear();
                        }
                    } else {
                        e.x += dx;
                        e.y += dy;
                    }
                    self.put(e);
                }
                self.g = G::Move { start, orig, moved };
            }
            G::Resize { h, orig, ob } => {
                self.do_resize(h, &orig, ob, p);
                self.g = G::Resize { h, orig, ob };
            }
            G::Endpoint { id, end } => {
                let eb = self.hit_at(p, true, "").unwrap_or_default();
                if let Some(mut a) = self.els.get(&id).cloned() {
                    // freeze the other end if it is bound, so it stays where it is
                    let (s, e) = arrow_ends(&self.els, &a);
                    a.pts = vec![s.0, s.1, e.0, e.1];
                    a.pts[end * 2] = p.0;
                    a.pts[end * 2 + 1] = p.1;
                    if end == 0 {
                        a.sb = eb;
                    } else {
                        a.eb = eb;
                    }
                    // an arrow bound at both ends to the same shape makes no sense
                    if !a.sb.is_empty() && a.sb == a.eb {
                        if end == 0 { a.sb.clear() } else { a.eb.clear() }
                    }
                    self.put(a);
                }
                self.g = G::Endpoint { id, end };
            }
            G::Marquee { start, add, .. } => {
                self.g = G::Marquee { start, cur: p, add };
                self.need_render = true;
            }
            G::Create { start, .. } => {
                self.g = G::Create { start, cur: p };
                self.need_render = true;
            }
            G::Pencil { mut pts } => {
                let n = pts.len();
                let (lx, ly) = (pts[n - 2], pts[n - 1]);
                if ((p.0 - lx).powi(2) + (p.1 - ly).powi(2)).sqrt() * zoom >= 1.5 && n < 20000 {
                    pts.push(p.0);
                    pts.push(p.1);
                    self.need_render = true;
                }
                self.g = G::Pencil { pts };
            }
            G::Arrow { start, sb, .. } => {
                let eb = self.hit_at(p, true, "").unwrap_or_default();
                self.g = G::Arrow { start, sb, cur: p, eb };
                self.need_render = true;
            }
        }
    }

    fn hover(&mut self, p: P) {
        let c = if self.tool != "select" {
            self.tool_cursor()
        } else if self.endpoint_at(p).is_some() {
            "crosshair".into()
        } else if let Some(h) = self.handle_at(p) {
            CURSORS[h].into()
        } else if self.hit_at(p, false, "").is_some() {
            "move".into()
        } else {
            "default".into()
        };
        self.cursor = c;
    }

    fn do_resize(&mut self, h: usize, orig: &[Element], ob: BB, p: P) {
        let (hx, hy) = HANDLE[h];
        let (mut x0, mut y0, mut x1, mut y1) = (ob.0, ob.1, ob.0 + ob.2, ob.1 + ob.3);
        if hx == 0.0 {
            x0 = p.0.min(x1 - 4.0);
        } else if hx == 1.0 {
            x1 = p.0.max(x0 + 4.0);
        }
        if hy == 0.0 {
            y0 = p.1.min(y1 - 4.0);
        } else if hy == 1.0 {
            y1 = p.1.max(y0 + 4.0);
        }
        // images keep their aspect ratio when dragged by a corner
        if orig.len() == 1 && orig[0].k == Kind::Image && hx != 0.5 && hy != 0.5 && ob.2 > 0.0 {
            let ratio = ob.3 / ob.2;
            let nw = x1 - x0;
            let nh = nw * ratio;
            if hy == 0.0 {
                y0 = y1 - nh;
            } else {
                y1 = y0 + nh;
            }
        }
        let sx = (x1 - x0) / ob.2.max(1e-6);
        let sy = (y1 - y0) / ob.3.max(1e-6);
        for o in orig {
            let mut e = o.clone();
            if e.k == Kind::Arrow {
                let (s, t) = arrow_ends(&self.els, o);
                let m = |q: P| (x0 + (q.0 - ob.0) * sx, y0 + (q.1 - ob.1) * sy);
                let (s2, t2) = (m(s), m(t));
                e.pts = vec![s2.0, s2.1, t2.0, t2.1];
            } else {
                e.x = x0 + (o.x - ob.0) * sx;
                e.y = y0 + (o.y - ob.1) * sy;
                e.w = (o.w * sx).max(1.0);
                e.h = (o.h * sy).max(1.0);
                if e.k == Kind::Pencil {
                    e.pts = o.pts.chunks(2).flat_map(|c| [c[0] * sx, c[1] * sy]).collect();
                }
                if e.k == Kind::Text {
                    e.fs = (o.fs * sy).clamp(6.0, 400.0);
                }
            }
            self.put(e);
        }
    }

    pub fn pointer_up(&mut self, sx: f64, sy: f64) {
        let p = self.s2w(sx, sy);
        match std::mem::replace(&mut self.g, G::None) {
            G::None => {}
            G::Move { .. } | G::Resize { .. } | G::Endpoint { .. } => self.end_tx(),
            G::Marquee { start, add, .. } => {
                let r = norm(start, p);
                self.reorder();
                let mut s: Vec<String> = if add { self.sel.clone() } else { vec![] };
                for id in &self.order {
                    let e = &self.els[id];
                    let eb = self.el_bb(e);
                    let hit_r = eb.0 <= r.0 + r.2 && eb.0 + eb.2 >= r.0 && eb.1 <= r.1 + r.3 && eb.1 + eb.3 >= r.1;
                    if hit_r && !s.contains(id) {
                        s.push(id.clone());
                    }
                }
                self.set_sel(s);
            }
            G::Create { start, .. } => {
                let k = match self.tool.as_str() {
                    "ellipse" => Kind::Ellipse,
                    "diamond" => Kind::Diamond,
                    _ => Kind::Rect,
                };
                let mut e = self.new_el(k);
                let b = norm(start, p);
                if b.2 * self.zoom < 6.0 && b.3 * self.zoom < 6.0 {
                    e.x = start.0 - 70.0;
                    e.y = start.1 - 45.0;
                    e.w = 140.0;
                    e.h = 90.0;
                } else {
                    e.x = b.0;
                    e.y = b.1;
                    e.w = b.2.max(8.0);
                    e.h = b.3.max(8.0);
                }
                let id = e.id.clone();
                self.begin_tx();
                self.put(e);
                self.end_tx();
                self.set_sel(vec![id]);
                self.tool = "select".into();
            }
            G::Pencil { mut pts } => {
                pts.push(p.0);
                pts.push(p.1);
                let eps = 0.8 / self.zoom;
                let sp = simplify(&pts, eps);
                let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for c in sp.chunks(2) {
                    minx = minx.min(c[0]);
                    maxx = maxx.max(c[0]);
                    miny = miny.min(c[1]);
                    maxy = maxy.max(c[1]);
                }
                let mut e = self.new_el(Kind::Pencil);
                e.x = minx;
                e.y = miny;
                e.w = (maxx - minx).max(1.0);
                e.h = (maxy - miny).max(1.0);
                e.pts = sp.chunks(2).flat_map(|c| [c[0] - minx, c[1] - miny]).collect();
                self.begin_tx();
                self.put(e);
                self.end_tx();
            }
            G::Arrow { start, sb, eb, .. } => {
                let dist = ((p.0 - start.0).powi(2) + (p.1 - start.1).powi(2)).sqrt() * self.zoom;
                if dist >= 10.0 {
                    let mut e = self.new_el(Kind::Arrow);
                    e.pts = vec![start.0, start.1, p.0, p.1];
                    e.sb = sb;
                    e.eb = eb;
                    if e.sb == e.eb {
                        e.sb.clear();
                        e.eb.clear();
                    }
                    let id = e.id.clone();
                    self.begin_tx();
                    self.put(e);
                    self.end_tx();
                    self.set_sel(vec![id]);
                    self.tool = "select".into();
                }
            }
        }
        self.need_render = true;
    }

    /// Abort the current gesture and roll back anything it changed (used when a pinch starts).
    pub fn cancel(&mut self) {
        let was = !matches!(self.g, G::None);
        self.g = G::None;
        if let Some(tx) = self.tx.take() {
            for (id, before) in tx {
                self.restore(&id, before);
            }
        }
        if was {
            self.need_render = true;
        }
    }

    pub fn double_click(&mut self, sx: f64, sy: f64) {
        let p = self.s2w(sx, sy);
        self.cancel();
        if self.tool != "select" || self.ro {
            return;
        }
        match self.hit_at(p, false, "") {
            Some(id) => {
                let k = self.els[&id].k;
                if matches!(k, Kind::Rect | Kind::Ellipse | Kind::Diamond | Kind::Text | Kind::Arrow) {
                    self.set_sel(vec![id.clone()]);
                    self.edit_req = id;
                }
            }
            None => self.start_text(p),
        }
    }

    fn start_text(&mut self, p: P) {
        let mut e = self.new_el(Kind::Text);
        e.id = "pending".into();
        e.x = p.0;
        e.y = p.1 - self.fs * 0.65;
        e.w = 10.0;
        e.h = self.fs * 1.3;
        self.pending = Some(e);
        self.edit_req = "pending".into();
        self.tool = "select".into();
    }

    // ------------------------------------------------------------ text editing
    /// id of an element the user wants to edit ("" = none); clears the request
    pub fn take_edit_request(&mut self) -> String {
        std::mem::take(&mut self.edit_req)
    }
    /// JSON of what the overlay editor needs: {id,k,x,y,w,h,fs,tx,st}
    pub fn edit_info(&self, id: &str) -> String {
        let e = if id == "pending" { self.pending.as_ref() } else { self.els.get(id) };
        match e {
            Some(e) => {
                let mut e = e.clone();
                if e.k == Kind::Arrow {
                    // the editor sits on the middle of the arrow: expose its bounding box
                    let (s, t) = arrow_ends(&self.els, &e);
                    let b = norm(s, t);
                    e.x = b.0;
                    e.y = b.1;
                    e.w = b.2;
                    e.h = b.3;
                }
                serde_json::to_string(&e).unwrap_or_default()
            }
            None => String::new(),
        }
    }
    /// Commit the text editor. w/h are the measured text size (used for free-standing text).
    pub fn commit_text(&mut self, id: &str, text: &str, w: f64, h: f64) {
        let text = clip_bytes(text, proto::MAX_TEXT_BYTES);
        if id == "pending" {
            if let Some(mut e) = self.pending.take() {
                if text.trim().is_empty() {
                    self.need_render = true;
                    return;
                }
                e.id = rid();
                e.tx = text;
                e.w = w.max(10.0);
                e.h = h.max(self.fs);
                let nid = e.id.clone();
                self.begin_tx();
                self.put(e);
                self.end_tx();
                self.set_sel(vec![nid]);
            }
            return;
        }
        if let Some(mut e) = self.els.get(id).cloned() {
            if e.del || e.tx == text {
                return;
            }
            self.begin_tx();
            if e.k == Kind::Text {
                if text.trim().is_empty() {
                    self.remove_ids(&[id.to_string()]);
                    self.end_tx();
                    self.set_sel(vec![]);
                    return;
                }
                e.w = w.max(10.0);
                e.h = h.max(e.fs);
            }
            e.tx = text;
            self.put(e);
            self.end_tx();
        }
    }
    /// the element whose text is currently being edited in the overlay (not drawn on the canvas)
    pub fn set_editing(&mut self, id: &str) {
        self.editing = id.to_string();
        self.need_render = true;
    }
    pub fn cancel_text(&mut self) {
        self.pending = None;
        self.need_render = true;
    }

    // ------------------------------------------------------------ commands
    pub fn select_all(&mut self) {
        self.reorder();
        let s = self.order.clone();
        self.set_sel(s);
    }
    pub fn deselect(&mut self) {
        self.set_sel(vec![]);
    }
    pub fn delete_selected(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        let ids = self.sel.clone();
        self.begin_tx();
        self.remove_ids(&ids);
        self.end_tx();
        self.set_sel(vec![]);
    }
    pub fn nudge(&mut self, dx: f64, dy: f64) {
        if self.sel.is_empty() {
            return;
        }
        self.begin_tx();
        let ids: BTreeSet<String> = self.sel.iter().cloned().collect();
        for id in self.sel.clone() {
            if let Some(mut e) = self.els.get(&id).cloned() {
                if e.k == Kind::Arrow {
                    let (s, t) = arrow_ends(&self.els, &e);
                    e.pts = vec![s.0 + dx, s.1 + dy, t.0 + dx, t.1 + dy];
                    if !ids.contains(&e.sb) {
                        e.sb.clear();
                    }
                    if !ids.contains(&e.eb) {
                        e.eb.clear();
                    }
                } else {
                    e.x += dx;
                    e.y += dy;
                }
                self.put(e);
            }
        }
        self.end_tx();
    }
    fn copies(&mut self, src: Vec<Element>, dx: f64, dy: f64) -> Vec<String> {
        let map: HashMap<String, String> = src.iter().map(|e| (e.id.clone(), rid())).collect();
        let mut z = self.top_z();
        let mut out = vec![];
        self.begin_tx();
        let mut src = src;
        src.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal));
        for mut e in src {
            e.id = map[&e.id].clone();
            e.z = z;
            z += 1.0;
            e.v = 0;
            if e.k == Kind::Arrow {
                if e.pts.len() >= 4 {
                    for i in 0..4 {
                        e.pts[i] += if i % 2 == 0 { dx } else { dy };
                    }
                }
                e.sb = map.get(&e.sb).cloned().unwrap_or_default();
                e.eb = map.get(&e.eb).cloned().unwrap_or_default();
            } else {
                e.x += dx;
                e.y += dy;
            }
            out.push(e.id.clone());
            self.put(e);
        }
        self.end_tx();
        out
    }
    pub fn duplicate(&mut self) {
        let src: Vec<Element> = self.sel.iter().filter_map(|i| self.els.get(i).cloned()).collect();
        if src.is_empty() {
            return;
        }
        // arrows bound to a shape that is not duplicated: freeze into plain arrows
        let src: Vec<Element> = src
            .into_iter()
            .map(|mut e| {
                if e.k == Kind::Arrow {
                    let (s, t) = arrow_ends(&self.els, &e);
                    e.pts = vec![s.0, s.1, t.0, t.1];
                }
                e
            })
            .collect();
        let ids = self.paste_at(src, None);
        self.set_sel(ids);
    }
    fn clip_of_selection(&self) -> Vec<Element> {
        self.sel
            .iter()
            .filter_map(|i| self.els.get(i).cloned())
            .filter(|e| !e.del)
            .map(|mut e| {
                if e.k == Kind::Arrow {
                    let (s, t) = arrow_ends(&self.els, &e);
                    e.pts = vec![s.0, s.1, t.0, t.1];
                }
                e
            })
            .collect()
    }
    /// Copy the selection; returns it as JSON for the system clipboard ("" when nothing is selected).
    pub fn copy_json(&mut self) -> String {
        let c = self.clip_of_selection();
        if c.is_empty() {
            return String::new();
        }
        let j = serde_json::to_string(&c).unwrap_or_default();
        self.clip = c;
        self.clip_src = j.clone();
        j
    }
    /// Paste elements from a JSON payload (from the system clipboard). When (sx, sy) is a finite
    /// screen point the group is centred there, else it is offset by 24px. A paste never lands
    /// exactly on an existing element of the same kind, so repeated pastes fan out.
    /// Returns the number of elements pasted.
    pub fn paste_json(&mut self, json: &str, sx: f64, sy: f64) -> u32 {
        let v: Vec<Element> = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return 0,
        };
        let v: Vec<Element> = v.into_iter().filter(|e| !e.del && e.validate().is_ok()).take(2000).collect();
        if v.is_empty() {
            return 0;
        }
        let n = v.len() as u32;
        self.clip_src = json.to_string();
        self.clip = v.clone();
        let at = if sx.is_finite() && sy.is_finite() { Some((sx, sy)) } else { None };
        let ids = self.paste_at(v, at);
        self.set_sel(ids);
        n
    }
    fn paste_at(&mut self, src: Vec<Element>, at: Option<(f64, f64)>) -> Vec<String> {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for e in &src {
            let b = if e.k == Kind::Arrow && e.pts.len() >= 4 {
                (e.pts[0].min(e.pts[2]), e.pts[1].min(e.pts[3]), e.pts[0].max(e.pts[2]), e.pts[1].max(e.pts[3]))
            } else {
                (e.x, e.y, e.x + e.w, e.y + e.h)
            };
            x0 = x0.min(b.0);
            y0 = y0.min(b.1);
            x1 = x1.max(b.2);
            y1 = y1.max(b.3);
        }
        let (mut dx, mut dy) = match at {
            Some((sx, sy)) => {
                let c = self.s2w(sx, sy);
                (c.0 - (x0 + x1) / 2.0, c.1 - (y0 + y1) / 2.0)
            }
            None => (24.0, 24.0),
        };
        let first = src.iter().min_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal)).cloned();
        if let Some(f) = first {
            let anchor = |e: &Element| if e.k == Kind::Arrow && e.pts.len() >= 2 { (e.pts[0], e.pts[1]) } else { (e.x, e.y) };
            let a = anchor(&f);
            let mut guard = 0;
            while guard < 500
                && self.els.values().any(|o| {
                    let p = anchor(o);
                    !o.del && o.k == f.k && (p.0 - (a.0 + dx)).abs() < 1.0 && (p.1 - (a.1 + dy)).abs() < 1.0
                })
            {
                dx += 24.0;
                dy += 24.0;
                guard += 1;
            }
        }
        self.copies(src, dx, dy)
    }
    /// Add a free-standing text label at screen position (sx, sy) with measured size (w, h).
    pub fn add_text(&mut self, text: &str, w: f64, h: f64, sx: f64, sy: f64) {
        let text = clip_bytes(text, proto::MAX_TEXT_BYTES);
        if text.trim().is_empty() {
            return;
        }
        let c = self.s2w(sx, sy);
        let mut e = self.new_el(Kind::Text);
        e.x = c.0;
        e.y = c.1;
        e.w = w.max(10.0);
        e.h = h.max(self.fs);
        e.tx = text;
        let id = e.id.clone();
        self.begin_tx();
        self.put(e);
        self.end_tx();
        self.set_sel(vec![id]);
    }
    pub fn default_fs(&self) -> f64 {
        self.fs
    }
    pub fn paste(&mut self) {
        if self.clip.is_empty() {
            return;
        }
        let ids = self.paste_at(self.clip.clone(), None);
        self.set_sel(ids);
    }
    pub fn z_front(&mut self) {
        self.zmove(true, true);
    }
    pub fn z_back(&mut self) {
        self.zmove(false, true);
    }
    pub fn z_up(&mut self) {
        self.zmove(true, false);
    }
    pub fn z_down(&mut self) {
        self.zmove(false, false);
    }
    fn zmove(&mut self, up: bool, all: bool) {
        if self.sel.is_empty() {
            return;
        }
        self.reorder();
        self.begin_tx();
        let ids = self.sel.clone();
        if all {
            let mut z = if up { self.top_z() } else { self.bottom_z() };
            let mut sorted: Vec<String> = self.order.iter().filter(|i| ids.contains(i)).cloned().collect();
            if !up {
                sorted.reverse();
            }
            for id in sorted {
                if let Some(mut e) = self.els.get(&id).cloned() {
                    e.z = z;
                    z += if up { 1.0 } else { -1.0 };
                    self.put(e);
                }
            }
        } else {
            let order = self.order.clone();
            let mut seq: Vec<String> = order.clone();
            let idxs: Vec<usize> = seq.iter().enumerate().filter(|(_, i)| ids.contains(i)).map(|(n, _)| n).collect();
            let iter: Vec<usize> = if up { idxs.iter().rev().cloned().collect() } else { idxs.clone() };
            for n in iter {
                let m = if up { n + 1 } else { n.wrapping_sub(1) };
                if m < seq.len() && !ids.contains(&seq[m]) {
                    seq.swap(n, m);
                }
            }
            // assign fresh sequential z to elements whose position changed
            for (n, id) in seq.iter().enumerate() {
                let z = n as f64;
                if let Some(e) = self.els.get(id) {
                    if order[n] != *id {
                        let mut e = e.clone();
                        e.z = z + 0.5 * 0.0;
                        self.put(e);
                    }
                }
            }
        }
        self.end_tx();
    }

    /// Apply a style to the selection (when any) and make it the default for new shapes.
    /// Empty string / negative number = leave unchanged.
    pub fn set_style(&mut self, st: &str, fl: &str, sw: f64, ah: i32, fs: f64) {
        let fill_given = fl != "~";
        if !st.is_empty() {
            self.st = st.to_string();
        }
        if fill_given {
            self.fl = if fl == "none" { String::new() } else { fl.to_string() };
        }
        if sw >= 0.0 {
            self.sw = sw;
        }
        if ah >= 0 {
            self.ah = ah as u8;
        }
        if fs > 0.0 {
            self.fs = fs;
        }
        if self.sel.is_empty() {
            return;
        }
        self.begin_tx();
        for id in self.sel.clone() {
            if let Some(mut e) = self.els.get(&id).cloned() {
                let mut ch = false;
                if !st.is_empty() && e.st != st {
                    e.st = st.to_string();
                    ch = true;
                }
                if fill_given && matches!(e.k, Kind::Rect | Kind::Ellipse | Kind::Diamond) {
                    let nf = if fl == "none" { String::new() } else { fl.to_string() };
                    if e.fl != nf {
                        e.fl = nf;
                        ch = true;
                    }
                }
                if sw >= 0.0 && !matches!(e.k, Kind::Text | Kind::Image) && e.sw != sw {
                    e.sw = sw;
                    ch = true;
                }
                if ah >= 0 && e.k == Kind::Arrow && e.ah & AH_HEADS != ah as u8 & AH_HEADS {
                    e.ah = (e.ah & !AH_HEADS) | (ah as u8 & AH_HEADS);
                    ch = true;
                }
                if fs > 0.0 && matches!(e.k, Kind::Rect | Kind::Ellipse | Kind::Diamond | Kind::Text) && e.fs != fs {
                    if e.k == Kind::Text {
                        let r = fs / e.fs.max(1.0);
                        e.w *= r;
                        e.h *= r;
                    }
                    e.fs = fs;
                    ch = true;
                }
                if ch {
                    self.put(e);
                }
            }
        }
        self.end_tx();
    }
    /// JSON describing the selection / defaults for the toolbar
    pub fn style_info(&self) -> String {
        let es: Vec<&Element> = self.sel.iter().filter_map(|i| self.els.get(i)).collect();
        let first = es.first();
        let (st, fl, sw, ah, fs) = match first {
            Some(e) => (e.st.clone(), e.fl.clone(), e.sw, e.ah & AH_HEADS, e.fs),
            None => (self.st.clone(), self.fl.clone(), self.sw, self.ah, self.fs),
        };
        let kinds: BTreeSet<String> = es.iter().map(|e| format!("{:?}", e.k).to_lowercase()).collect();
        format!(
            "{{\"n\":{},\"st\":{},\"fl\":{},\"sw\":{},\"ah\":{},\"fs\":{},\"kinds\":{}}}",
            es.len(),
            serde_json::to_string(&st).unwrap_or_default(),
            serde_json::to_string(&fl).unwrap_or_default(),
            sw,
            ah,
            fs,
            serde_json::to_string(&kinds).unwrap_or_default()
        )
    }

    pub fn add_image(&mut self, hash: &str, w: f64, h: f64, sx: f64, sy: f64) -> String {
        let c = self.s2w(sx, sy);
        let (mut w, mut h) = (w.max(1.0), h.max(1.0));
        let m = 480.0f64.min(480.0 / self.zoom.max(0.3));
        let k = (m / w).min(m / h).min(1.0);
        w *= k;
        h *= k;
        let (mut x, mut y) = (c.0 - w / 2.0, c.1 - h / 2.0);
        // repeated adds of the same picture fan out instead of stacking exactly
        let mut guard = 0;
        while guard < 500
            && self.els.values().any(|o| !o.del && o.k == Kind::Image && o.img == hash && (o.x - x).abs() < 1.0 && (o.y - y).abs() < 1.0)
        {
            x += 24.0;
            y += 24.0;
            guard += 1;
        }
        let mut e = self.new_el(Kind::Image);
        e.x = x;
        e.y = y;
        e.w = w;
        e.h = h;
        e.img = hash.to_string();
        e.st = String::new();
        e.sw = 0.0;
        let id = e.id.clone();
        self.begin_tx();
        self.put(e);
        self.end_tx();
        self.tool = "select".into();
        self.set_sel(vec![id.clone()]);
        id
    }

    // ------------------------------------------------------------ history
    fn restore(&mut self, id: &str, state: Option<Element>) {
        let cur = self.els.get(id).cloned();
        let mut e = match (state, &cur) {
            (Some(s), _) => s,
            (None, Some(c)) => {
                let mut c = c.clone();
                c.del = true;
                c
            }
            (None, None) => return,
        };
        e.v = cur.as_ref().map_or(0, |c| c.v) + 1;
        e.by = self.cid.clone();
        if e.del {
            e.pts.clear();
            e.tx.clear();
        }
        self.els.insert(id.to_string(), e);
        self.outbox.insert(id.to_string());
        self.touch();
    }
    /// Undo the last change. Returns what was undone ("add", "move", ...), or "" when there was nothing to undo.
    pub fn undo(&mut self) -> String {
        self.cancel();
        if self.ro {
            return String::new();
        }
        if let Some(ch) = self.undo.pop() {
            for c in &ch {
                self.restore(&c.id, c.before.clone());
            }
            let label = describe(&ch);
            self.redo.push(ch);
            self.fix_sel();
            return label;
        }
        String::new()
    }
    /// Redo the last undone change. Returns what was redone, or "" when there was nothing to redo.
    pub fn redo(&mut self) -> String {
        self.cancel();
        if self.ro {
            return String::new();
        }
        if let Some(ch) = self.redo.pop() {
            for c in &ch {
                self.restore(&c.id, c.after.clone());
            }
            let label = describe(&ch);
            self.undo.push(ch);
            self.fix_sel();
            return label;
        }
        String::new()
    }
    fn fix_sel(&mut self) {
        let s: Vec<String> = self.sel.iter().filter(|i| self.els.get(*i).map_or(false, |e| !e.del)).cloned().collect();
        self.set_sel(s);
    }

    // ------------------------------------------------------------ sync / serialization
    /// JSON array of the elements changed locally since the last call
    pub fn take_ops(&mut self) -> String {
        let ids = std::mem::take(&mut self.outbox);
        let v: Vec<&Element> = ids.iter().filter_map(|i| self.els.get(i)).collect();
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }
    pub fn has_ops(&self) -> bool {
        !self.outbox.is_empty()
    }
    /// Merge remote elements (last-writer-wins). Returns true if anything changed.
    pub fn apply_remote(&mut self, json: &str) -> bool {
        let v: Vec<Element> = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let mut ch = false;
        for e in v {
            if e.validate().is_err() {
                continue;
            }
            if let Some(old) = self.els.get(&e.id) {
                if !proto::wins(&e, old) {
                    continue;
                }
            }
            self.els.insert(e.id.clone(), e);
            ch = true;
        }
        if ch {
            self.touch();
            self.fix_sel();
        }
        ch
    }
    /// Merge a full snapshot, and queue everything where we hold a newer state than the server.
    pub fn merge_snapshot(&mut self, json: &str) {
        let v: Vec<Element> = serde_json::from_str(json).unwrap_or_default();
        let theirs: HashMap<String, Element> = v.iter().cloned().map(|e| (e.id.clone(), e)).collect();
        self.apply_remote(json);
        for (id, mine) in &self.els {
            match theirs.get(id) {
                Some(t) if !proto::wins(mine, t) => {}
                _ => {
                    self.outbox.insert(id.clone());
                }
            }
        }
        self.touch();
    }
    /// Replace the whole document (loading from IndexedDB / an uploaded file).
    pub fn load(&mut self, json: &str) {
        let v: Vec<Element> = serde_json::from_str(json).unwrap_or_default();
        self.els.clear();
        for e in v {
            if e.validate().is_ok() {
                self.els.insert(e.id.clone(), e);
            }
        }
        self.undo.clear();
        self.redo.clear();
        self.sel.clear();
        self.sel_rev = self.sel_rev.wrapping_add(1);
        self.outbox.clear();
        self.touch();
    }
    /// All elements including tombstones (for cloud upload / sync)
    pub fn export_all(&self) -> String {
        let v: Vec<&Element> = self.els.values().collect();
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }
    /// Live elements only (for local storage)
    pub fn export_live(&self) -> String {
        let v: Vec<&Element> = self.els.values().filter(|e| !e.del).collect();
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }
    /// Rough bytes held by the document model (elements + history), for the memory readout in tests.
    pub fn model_bytes(&self) -> f64 {
        let el = |e: &Element| 160 + e.id.len() + e.by.len() + e.tx.len() + e.st.len() + e.fl.len() + e.img.len() + e.pts.len() * 8;
        let a: usize = self.els.values().map(el).sum();
        let h: usize = self
            .undo
            .iter()
            .chain(self.redo.iter())
            .flatten()
            .map(|c| c.before.as_ref().map_or(0, el) + c.after.as_ref().map_or(0, el))
            .sum();
        (a + h) as f64
    }
    // ------------------------------------------------------------ conversions (text -> html / mermaid)
    /// The single selected text element: (id, source). Err = message for the user.
    fn selected_text(&self) -> Result<(String, String), String> {
        if self.ro {
            return Err("This document is read-only".into());
        }
        let one = if self.sel.len() == 1 { self.els.get(&self.sel[0]).filter(|e| !e.del) } else { None };
        match one {
            Some(e) if e.k == Kind::Text && !e.tx.trim().is_empty() => Ok((e.id.clone(), e.tx.clone())),
            Some(e) if e.k == Kind::Text => Err("The selected text is empty".into()),
            _ => Err("Select one text element first (paste or type the code as text)".into()),
        }
    }

    /// Turn the selected text element into an HTML object. Returns "ok:<id>" or "err:<message>"; on error nothing changed.
    pub fn text_to_html(&mut self) -> String {
        let (id, tx) = match self.selected_text() {
            Ok(v) => v,
            Err(m) => return format!("err:{}", m),
        };
        let src = strip_fence(&tx, &["html", "htm", ""]);
        if src.trim().is_empty() {
            return "err:The selected text has no HTML".into();
        }
        if src.len() > proto::MAX_HTML_BYTES {
            return format!("err:HTML is too large ({} KB, the limit is {} KB)", src.len() / 1024 + 1, proto::MAX_HTML_BYTES / 1024);
        }
        let old = self.els[&id].clone();
        let mut e = self.new_el(Kind::Html);
        e.x = old.x;
        e.y = old.y;
        e.w = 520.0;
        e.h = 360.0;
        e.src = src;
        e.z = old.z;
        let nid = e.id.clone();
        self.begin_tx();
        self.remove_ids(&[id]);
        self.put(e);
        self.end_tx();
        self.set_sel(vec![nid.clone()]);
        format!("ok:{}", nid)
    }

    /// Live HTML objects as JSON [[id,x,y,w,h,v],...] in world coordinates, for the shell's iframe layer.
    pub fn html_list(&self) -> String {
        let v: Vec<(&str, f64, f64, f64, f64, u32)> =
            self.els.values().filter(|e| e.k == Kind::Html && !e.del).map(|e| (e.id.as_str(), e.x, e.y, e.w, e.h, e.v)).collect();
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }
    /// The routed polyline of an arrow as [x0,y0,x1,y1,...] (empty when it is not an arrow). For tests and tooling.
    pub fn arrow_route(&self, id: &str) -> Vec<f64> {
        match self.els.get(id).filter(|e| e.k == Kind::Arrow && !e.del) {
            Some(e) => self.route_of(e).iter().flat_map(|p| [p.0, p.1]).collect(),
            None => vec![],
        }
    }
    pub fn html_src(&self, id: &str) -> String {
        self.els.get(id).filter(|e| e.k == Kind::Html && !e.del).map(|e| e.src.clone()).unwrap_or_default()
    }

    /// Turn the selected mermaid text into real elements. Returns "ok:<nodes>:<edges>" or "err:<message>".
    /// Everything (create the flow + remove the text) is one undo step; on error nothing changed.
    pub fn text_to_mermaid(&mut self) -> String {
        let (id, tx) = match self.selected_text() {
            Ok(v) => v,
            Err(m) => return format!("err:{}", m),
        };
        let g = match mermaid::parse(&tx) {
            Ok(g) => g,
            Err(m) => return format!("err:{}", m),
        };
        let lay = mermaid::layout(&g);
        let old = self.els[&id].clone();
        let (ox, oy) = (old.x, old.y);
        let horiz = matches!(g.dir, mermaid::Dir::Lr | mermaid::Dir::Rl);
        let mut z = self.top_z();
        let mut made: Vec<Element> = vec![];
        let mut ids: Vec<String> = vec![];
        for (i, n) in g.nodes.iter().enumerate() {
            let (x, y, w, h) = lay.pos[i];
            let (k, fl) = match n.shape {
                mermaid::Shape::Rect => (Kind::Rect, "#dbeafe"),
                mermaid::Shape::Round => (Kind::Rect, "#d3f9d8"),
                mermaid::Shape::Diamond => (Kind::Diamond, "#fff3bf"),
                mermaid::Shape::Circle => (Kind::Ellipse, "#f3d9fa"),
            };
            let mut e = self.new_el(k);
            e.x = ox + x;
            e.y = oy + y;
            e.w = w;
            e.h = h;
            e.fl = fl.into();
            e.fs = mermaid::FS;
            e.tx = mermaid::wrap_label(&n.label, 24);
            e.z = z;
            z += 1.0;
            ids.push(e.id.clone());
            made.push(e);
        }
        let mut edges: Vec<(Element, String)> = vec![];
        for ed in g.edges.iter().filter(|e| e.a != e.b) {
            let mut a = self.new_el(Kind::Arrow);
            a.sb = ids[ed.a].clone();
            a.eb = ids[ed.b].clone();
            let (ca, cb) = (center(&made[ed.a]), center(&made[ed.b]));
            a.pts = vec![ca.0, ca.1, cb.0, cb.1];
            a.ah = (if ed.head { 1 } else { 0 }) | (if horiz { AH_HINT_H } else { AH_HINT_V }) | (if ed.dash { AH_DASH } else { 0 });
            a.sw = if ed.thick { 4.0 } else { 2.0 };
            a.z = z;
            z += 1.0;
            edges.push((a, ed.label.clone()));
        }
        // route every edge on the finished layout, then put its label beside the route
        let mut tmp: Els = made.iter().map(|e| (e.id.clone(), e.clone())).collect();
        for (a, _) in &edges {
            tmp.insert(a.id.clone(), a.clone());
        }
        let mut taken: Vec<R> = vec![];
        let mut labels: Vec<Element> = vec![];
        for (a, lab) in &edges {
            if lab.is_empty() {
                continue;
            }
            let r = route(&tmp, a, true);
            let c = label_pos(&tmp, &r, lab, mermaid::LABEL_FS, &taken);
            let (w, h) = label_size(lab, mermaid::LABEL_FS);
            taken.push((c.0 - w / 2.0, c.1 - h / 2.0, c.0 + w / 2.0, c.1 + h / 2.0));
            let mut t = self.new_el(Kind::Text);
            t.fs = mermaid::LABEL_FS;
            t.tx = lab.clone();
            t.st = "#495057".into();
            t.w = w - 10.0;
            t.h = h - 4.0;
            t.x = c.0 - t.w / 2.0;
            t.y = c.1 - t.h / 2.0;
            t.z = z;
            z += 1.0;
            labels.push(t);
        }
        self.begin_tx();
        self.remove_ids(&[id]);
        let mut sel = vec![];
        for e in made.into_iter().chain(edges.into_iter().map(|x| x.0)).chain(labels) {
            sel.push(e.id.clone());
            self.put(e);
        }
        self.end_tx();
        self.set_sel(sel);
        format!("ok:{}:{}", g.nodes.len(), g.edges.len())
    }

    pub fn live_count(&self) -> usize {
        self.els.values().filter(|e| !e.del).count()
    }
    /// JSON array of image hashes referenced by live elements
    pub fn images_used(&self) -> String {
        let s: BTreeSet<&String> = self.els.values().filter(|e| !e.del && e.k == Kind::Image).map(|e| &e.img).collect();
        serde_json::to_string(&s).unwrap_or_else(|_| "[]".into())
    }
    /// Screen position of an element's centre (for tests / tooling): [x, y, w, h] in CSS px
    pub fn element_screen(&self, id: &str) -> Vec<f64> {
        match self.els.get(id) {
            Some(e) => {
                let (x, y, w, h) = self.el_bb(e);
                vec![x * self.zoom + self.px, y * self.zoom + self.py, w * self.zoom, h * self.zoom]
            }
            None => vec![],
        }
    }
    /// Compact summary for tests and debugging: JSON array of live elements {id,k,x,y,w,h,tx,sb,eb}
    pub fn summary(&mut self) -> String {
        self.reorder();
        let mut out = String::from("[");
        for (n, id) in self.order.iter().enumerate() {
            let e = &self.els[id];
            let (s, t) = if e.k == Kind::Arrow { arrow_ends(&self.els, e) } else { ((0.0, 0.0), (0.0, 0.0)) };
            if n > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"id\":{},\"k\":\"{:?}\",\"x\":{:.1},\"y\":{:.1},\"w\":{:.1},\"h\":{:.1},\"tx\":{},\"sb\":{},\"eb\":{},\"v\":{},\"ax\":[{:.1},{:.1},{:.1},{:.1}],\"np\":{}}}",
                serde_json::to_string(&e.id).unwrap_or_default(),
                e.k, e.x, e.y, e.w, e.h,
                serde_json::to_string(&e.tx).unwrap_or_default(),
                serde_json::to_string(&e.sb).unwrap_or_default(),
                serde_json::to_string(&e.eb).unwrap_or_default(),
                e.v, s.0, s.1, t.0, t.1, e.pts.len() / 2
            ));
        }
        out.push(']');
        out
    }
    pub fn selection(&self) -> String {
        serde_json::to_string(&self.sel).unwrap_or_default()
    }
    pub fn select_ids(&mut self, json: &str) {
        let v: Vec<String> = serde_json::from_str(json).unwrap_or_default();
        self.set_sel(v);
    }
    /// Deterministic content hash of the live document (for convergence checks)
    pub fn digest(&self) -> String {
        let mut v: Vec<&Element> = self.els.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        let s = serde_json::to_string(&v).unwrap_or_default();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{:016x}:{}", h, v.len())
    }

    // ------------------------------------------------------------ rendering
    pub fn render(&mut self, g: &Gfx, w: f64, h: f64, dpr: f64) {
        self.reorder();
        g.clear(w * dpr, h * dpr);
        g.view(self.zoom * dpr, self.px * dpr, self.py * dpr);
        // borrowed, not cloned: this runs every frame
        let hovered_bind: Vec<&String> = match &self.g {
            G::Arrow { sb, eb, .. } => vec![sb, eb],
            G::Endpoint { id, .. } => self.els.get(id).map(|a| vec![&a.sb, &a.eb]).unwrap_or_default(),
            _ => vec![],
        };
        // visible world rect for culling
        let vis: BB = (-self.px / self.zoom, -self.py / self.zoom, w / self.zoom, h / self.zoom);
        for id in &self.order {
            let e = &self.els[id];
            let eb = if e.k == Kind::Arrow { self.arrow_reach(e) } else { bbox(e) };
            let r = (vis.0 - 40.0, vis.1 - 40.0, vis.2 + 80.0, vis.3 + 80.0);
            if !(eb.0 <= r.0 + r.2 && eb.0 + eb.2 >= r.0 && eb.1 <= r.1 + r.3 && eb.1 + eb.3 >= r.1) {
                continue;
            }
            self.draw_el(g, e);
        }
        if let Some(p) = &self.pending {
            let _ = p;
        }
        // in-progress gestures
        let z = self.zoom;
        match &self.g {
            G::Create { start, cur } => {
                let b = norm(*start, *cur);
                g.begin();
                match self.tool.as_str() {
                    "ellipse" => g.ell(b.0 + b.2 / 2.0, b.1 + b.3 / 2.0, b.2 / 2.0, b.3 / 2.0),
                    "diamond" => diamond_path(g, b),
                    _ => g.rrect(b.0, b.1, b.2, b.3, 12.0f64.min(b.2 / 4.0).min(b.3 / 4.0)),
                }
                g.fill("rgba(79,107,237,0.08)");
                g.stroke(ACCENT, 1.5 / z, 0.0);
            }
            G::Pencil { pts } => {
                g.begin();
                smooth_path(g, pts, 0.0, 0.0);
                g.stroke(&self.st, self.sw, 0.0);
            }
            G::Arrow { start, cur, sb, eb } => {
                let mut tmp = Element { k: Kind::Arrow, pts: vec![start.0, start.1, cur.0, cur.1], ..Default::default() };
                if sb != eb {
                    tmp.sb = sb.clone();
                    tmp.eb = eb.clone();
                }
                let r = route(&self.els, &tmp, false);
                g.begin();
                rounded_poly(g, &r, 8.0);
                g.stroke(&self.st, self.sw, 0.0);
                if r.len() >= 2 && self.ah & 1 != 0 {
                    let (a, b) = (r[r.len() - 2], r[r.len() - 1]);
                    arrow_head(g, a, b, self.sw, &self.st, true);
                }
            }
            G::Marquee { start, cur, .. } => {
                let b = norm(*start, *cur);
                g.begin();
                g.rrect(b.0, b.1, b.2, b.3, 0.0);
                g.fill("rgba(79,107,237,0.08)");
                g.stroke(ACCENT, 1.0 / z, 0.0);
            }
            _ => {}
        }
        for id in hovered_bind.iter().filter(|s| !s.is_empty()) {
            if let Some(e) = self.els.get(*id) {
                g.begin();
                g.rrect(e.x - 5.0 / z, e.y - 5.0 / z, e.w + 10.0 / z, e.h + 10.0 / z, 8.0 / z);
                g.stroke(ACCENT, 2.0 / z, 0.0);
            }
        }
        // selection
        if self.tool == "select" {
            let single = self.single_arrow();
            for id in &self.sel {
                if let Some(e) = self.els.get(id) {
                    if e.k == Kind::Arrow {
                        continue;
                    }
                    let b = bbox(e);
                    g.begin();
                    g.rrect(b.0 - 2.0 / z, b.1 - 2.0 / z, b.2 + 4.0 / z, b.3 + 4.0 / z, 2.0 / z);
                    g.stroke(ACCENT, 1.5 / z, 0.0);
                }
            }
            if let Some(a) = single {
                let (s, t) = arrow_ends(&self.els, a);
                for q in [s, t] {
                    g.begin();
                    g.ell(q.0, q.1, 5.0 / z, 5.0 / z);
                    g.fill("#ffffff");
                    g.stroke(ACCENT, 1.5 / z, 0.0);
                }
            } else if let Some(b) = self.sel_bbox() {
                if self.sel.len() > 1 {
                    g.begin();
                    g.rrect(b.0 - 4.0 / z, b.1 - 4.0 / z, b.2 + 8.0 / z, b.3 + 8.0 / z, 2.0 / z);
                    g.stroke(ACCENT, 1.0 / z, 4.0 / z);
                }
                let r = 4.5 / z;
                for hnd in HANDLE.iter() {
                    g.begin();
                    g.rrect(b.0 + b.2 * hnd.0 - r, b.1 + b.3 * hnd.1 - r, r * 2.0, r * 2.0, 2.0 / z);
                    g.fill("#ffffff");
                    g.stroke(ACCENT, 1.5 / z, 0.0);
                }
            }
        }
    }

    fn draw_el(&self, g: &Gfx, e: &Element) {
        let sw = if e.sw > 0.0 { e.sw } else { 2.0 };
        match e.k {
            Kind::Rect | Kind::Ellipse | Kind::Diamond => {
                g.begin();
                match e.k {
                    Kind::Rect => g.rrect(e.x, e.y, e.w, e.h, 14.0f64.min(e.w / 3.0).min(e.h / 3.0)),
                    Kind::Ellipse => g.ell(e.x + e.w / 2.0, e.y + e.h / 2.0, e.w / 2.0, e.h / 2.0),
                    _ => diamond_path(g, (e.x, e.y, e.w, e.h)),
                }
                if !e.fl.is_empty() {
                    g.fill(&e.fl);
                }
                g.stroke(&e.st, sw, 0.0);
                if !e.tx.is_empty() && !(self.edit_target(&e.id)) {
                    let fs = if e.fs > 0.0 { e.fs } else { 18.0 };
                    let inner = match e.k {
                        Kind::Rect => e.w - 20.0,
                        _ => e.w * 0.62,
                    };
                    let lines = wrap(g, &e.tx, fs, inner.max(20.0));
                    let lh = fs * 1.3;
                    let y0 = e.y + e.h / 2.0 - lh * lines.len() as f64 / 2.0 + lh / 2.0;
                    for (i, l) in lines.iter().enumerate() {
                        g.text(l, e.x + e.w / 2.0, y0 + i as f64 * lh, fs, &e.st, "center");
                    }
                }
            }
            Kind::Text => {
                if self.edit_target(&e.id) {
                    return;
                }
                let fs = if e.fs > 0.0 { e.fs } else { 18.0 };
                for (i, l) in e.tx.split('\n').enumerate() {
                    g.text(l, e.x, e.y + (i as f64 + 0.5) * fs * 1.3, fs, &e.st, "left");
                }
            }
            Kind::Pencil => {
                g.begin();
                smooth_path(g, &e.pts, e.x, e.y);
                g.stroke(&e.st, sw, 0.0);
            }
            Kind::Arrow => {
                let r = self.route_of(e);
                let n = r.len();
                g.begin();
                rounded_poly(g, &r, 8.0);
                g.stroke(&e.st, sw, if e.ah & AH_DASH != 0 { 7.0 } else { 0.0 });
                if n >= 2 {
                    let (s, t) = (r[0], r[n - 1]);
                    let dir = |a: P, b: P| {
                        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
                        (dx / l * 60.0, dy / l * 60.0)
                    };
                    if e.ah & 1 != 0 {
                        let u = dir(r[n - 2], t);
                        arrow_head(g, (t.0 - u.0, t.1 - u.1), t, sw, &e.st, true);
                    }
                    if e.ah & 2 != 0 {
                        let u = dir(r[1], s);
                        arrow_head(g, (s.0 - u.0, s.1 - u.1), s, sw, &e.st, true);
                    }
                }
                if !e.tx.is_empty() && !self.edit_target(&e.id) {
                    let fs = if e.fs > 0.0 { e.fs } else { 18.0 };
                    let (mx, my) = poly_mid(&r);
                    let lines: Vec<&str> = e.tx.split('\n').collect();
                    let lh = fs * 1.3;
                    let tw = lines.iter().map(|l| g.measure(l, fs)).fold(0.0, f64::max);
                    let th = lh * lines.len() as f64;
                    g.begin();
                    g.rrect(mx - tw / 2.0 - 5.0, my - th / 2.0 - 2.0, tw + 10.0, th + 4.0, 6.0);
                    g.fill("#ffffff");
                    for (i, l) in lines.iter().enumerate() {
                        g.text(l, mx, my - th / 2.0 + lh / 2.0 + i as f64 * lh, fs, &e.st, "center");
                    }
                }
            }
            Kind::Html => {
                // the markup itself is drawn by a sandboxed iframe underneath this transparent canvas
                g.begin();
                g.rrect(e.x, e.y, e.w, e.h, 4.0);
                g.stroke("#d0d5dd", 1.0, 0.0);
            }
            Kind::Image => g.image(&e.img, e.x, e.y, e.w, e.h),
        }
    }
    /// Route of an arrow (cached; while a drag is going on a result up to 33 ms old is reused: <= 30 Hz).
    fn route_of(&self, e: &Element) -> Vec<P> {
        let n = ends(&self.els, e);
        if n.sd.is_none() && n.ed.is_none() {
            return vec![n.s, n.e];
        }
        let mut gk = self.geo.get();
        if gk.0 != self.doc_rev {
            gk = (self.doc_rev, geo_hash(&self.els));
            self.geo.set(gk);
        }
        let key = route_key(&self.els, e, gk.1);
        let t = now();
        let dragging = matches!(self.g, G::Move { .. } | G::Resize { .. } | G::Endpoint { .. });
        let mut c = self.rc.borrow_mut();
        if let Some((k, at, r)) = c.get(&e.id) {
            // while dragging: arrows that do not touch the moving nodes keep their route, the others are refreshed at <= 30 Hz
            let moving = |id: &str| self.tx.as_ref().map_or(false, |m| m.contains_key(id));
            let touched = moving(&e.id) || moving(&e.sb) || moving(&e.eb);
            if *k == key || (dragging && (!touched || t - *at < 33.0)) {
                return r.clone();
            }
        }
        if c.len() > 4096 {
            c.clear();
        }
        let r = route(&self.els, e, true);
        c.insert(e.id.clone(), (key, t, r.clone()));
        r
    }
    fn edit_target(&self, id: &str) -> bool {
        !self.editing.is_empty() && self.editing == id
    }
}

/// Short name of what a history entry did (for the undo/redo popup).
fn describe(ch: &[Change]) -> String {
    let live = |e: &Option<Element>| e.as_ref().map_or(false, |e| !e.del);
    if ch.iter().any(|c| !live(&c.after) && live(&c.before)) {
        return "delete".into();
    }
    if ch.iter().any(|c| !live(&c.before) && live(&c.after)) {
        return "add".into();
    }
    for c in ch {
        if let (Some(b), Some(a)) = (&c.before, &c.after) {
            if (a.w - b.w).abs() > 1e-9 || (a.h - b.h).abs() > 1e-9 {
                return "resize".into();
            }
            if (a.x - b.x).abs() > 1e-9 || (a.y - b.y).abs() > 1e-9 || a.pts != b.pts {
                return "move".into();
            }
            if a.tx != b.tx {
                return "text".into();
            }
            if a.st != b.st || a.fl != b.fl || a.sw != b.sw || a.fs != b.fs || a.ah != b.ah {
                return "style".into();
            }
            if (a.z - b.z).abs() > 1e-9 {
                return "reorder".into();
            }
        }
    }
    "edit".into()
}

/// Remove a surrounding markdown fence (```lang ... ```), a BOM and surrounding whitespace.
fn strip_fence(s: &str, langs: &[&str]) -> String {
    let t = s.trim_start_matches('\u{feff}').trim();
    if let Some(rest) = t.strip_prefix("```") {
        let (first, body) = rest.split_once('\n').unwrap_or((rest, ""));
        if langs.contains(&first.trim().to_ascii_lowercase().as_str()) {
            let body = body.trim_end();
            let body = body.strip_suffix("```").unwrap_or(body);
            return body.trim().to_string();
        }
    }
    t.to_string()
}

/// Longest prefix of `s` that fits in `max` bytes (on a char boundary).
fn clip_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut n = max;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    s[..n].to_string()
}

/// Polyline with rounded corners (quadratic bends of radius `r`).
fn rounded_poly(g: &Gfx, p: &[P], r: f64) {
    if p.is_empty() {
        return;
    }
    g.mv(p[0].0, p[0].1);
    for i in 1..p.len().saturating_sub(1) {
        let (a, b, c) = (p[i - 1], p[i], p[i + 1]);
        let l1 = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt().max(1e-6);
        let l2 = ((c.0 - b.0).powi(2) + (c.1 - b.1).powi(2)).sqrt().max(1e-6);
        let rr = r.min(l1 / 2.0).min(l2 / 2.0);
        g.ln(b.0 + (a.0 - b.0) / l1 * rr, b.1 + (a.1 - b.1) / l1 * rr);
        g.quad(b.0, b.1, b.0 + (c.0 - b.0) / l2 * rr, b.1 + (c.1 - b.1) / l2 * rr);
    }
    let l = p[p.len() - 1];
    g.ln(l.0, l.1);
}

/// Point half way along a polyline.
fn poly_mid(p: &[P]) -> P {
    let len: f64 = p.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
    let mut left = len / 2.0;
    for w in p.windows(2) {
        let l = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        if l >= left && l > 0.0 {
            let t = left / l;
            return (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
        }
        left -= l;
    }
    p.first().copied().unwrap_or((0.0, 0.0))
}

fn diamond_path(g: &Gfx, b: BB) {
    g.mv(b.0 + b.2 / 2.0, b.1);
    g.ln(b.0 + b.2, b.1 + b.3 / 2.0);
    g.ln(b.0 + b.2 / 2.0, b.1 + b.3);
    g.ln(b.0, b.1 + b.3 / 2.0);
    g.close();
}

fn arrow_head(g: &Gfx, from: P, to: P, sw: f64, colour: &str, filled: bool) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let l = (dx * dx + dy * dy).sqrt();
    if l < 1.0 {
        return;
    }
    let (ux, uy) = (dx / l, dy / l);
    let size = (10.0 + sw * 2.0).min(l * 0.8);
    let (bx, by) = (to.0 - ux * size, to.1 - uy * size);
    let (nx, ny) = (-uy * size * 0.45, ux * size * 0.45);
    g.begin();
    g.mv(to.0, to.1);
    g.ln(bx + nx, by + ny);
    g.ln(bx - nx, by - ny);
    g.close();
    if filled {
        g.fill(colour);
    }
    g.stroke(colour, sw * 0.6, 0.0);
}

/// Quadratic-midpoint smoothing through the points.
fn smooth_path(g: &Gfx, pts: &[f64], ox: f64, oy: f64) {
    let n = pts.len() / 2;
    if n == 0 {
        return;
    }
    let pt = |i: usize| (pts[i * 2] + ox, pts[i * 2 + 1] + oy);
    let p0 = pt(0);
    g.mv(p0.0, p0.1);
    if n == 1 {
        g.ln(p0.0 + 0.01, p0.1);
        return;
    }
    if n == 2 {
        let p1 = pt(1);
        g.ln(p1.0, p1.1);
        return;
    }
    for i in 1..n - 1 {
        let (a, b) = (pt(i), pt(i + 1));
        g.quad(a.0, a.1, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    }
    let last = pt(n - 1);
    g.ln(last.0, last.1);
}

fn wrap(g: &Gfx, text: &str, fs: f64, maxw: f64) -> Vec<String> {
    let mut out = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { word.to_string() } else { format!("{} {}", line, word) };
            if !line.is_empty() && g.measure(&cand, fs) > maxw {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = cand;
            }
        }
        out.push(line);
    }
    out
}
