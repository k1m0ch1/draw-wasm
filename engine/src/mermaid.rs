//! Mermaid flowchart subset: parser and layered layout. Pure Rust, no I/O.
//!
//! Supported: `flowchart|graph` + TD/TB/LR/RL/BT; nodes `[rect]`, `(rounded)`, `{diamond}`, `((circle))`;
//! edges `---`, `-->`, `-.->`, `-.-`, `==>`, `===`, with `|label|` or `-- label -->`; chains (`A --> B --> C`);
//! one statement per line; `%%` comments; a surrounding markdown fence is stripped.
//! Also: `([stadium])` nodes, `subgraph id [Label]` ... `end` (nested, own `direction`), edges to and from a subgraph id.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Rect,
    Round,
    Diamond,
    Circle,
    Stadium,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Td,
    Bt,
    Lr,
    Rl,
}

#[derive(Debug)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub shape: Shape,
    /// innermost subgraph holding the node
    pub sub: Option<usize>,
}

#[derive(Debug)]
pub struct Sub {
    pub id: String,
    pub label: String,
    pub dir: Option<Dir>,
    pub parent: Option<usize>,
}

/// An edge end: a node or a subgraph (container).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    N(usize),
    S(usize),
}

#[derive(Debug)]
pub struct Edge {
    pub a: End,
    pub b: End,
    pub label: String,
    pub head: bool,
    pub dash: bool,
    pub thick: bool,
}

#[derive(Debug)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub subs: Vec<Sub>,
    pub edges: Vec<Edge>,
}

struct Ctx {
    g: Graph,
    map: HashMap<String, usize>,
    smap: HashMap<String, usize>,
    stack: Vec<usize>,
}

pub const MAX_SUBS: usize = 40;
pub const MAX_NODES: usize = 150;
pub const MAX_EDGES: usize = 300;
pub const FS: f64 = 16.0;
pub const LABEL_FS: f64 = 14.0;

// ------------------------------------------------------------------ parsing

/// Pull the diagram out of the text: tolerate a BOM, CRLF, prose around a ``` fence, trailing whitespace.
fn body_lines(src: &str) -> Vec<(usize, String)> {
    let src = src.trim_start_matches('\u{feff}');
    let all: Vec<(usize, &str)> = src.lines().enumerate().map(|(i, l)| (i + 1, l)).collect();
    let open = all.iter().position(|(_, l)| l.trim_start().starts_with("```"));
    let (from, to) = match open {
        Some(o) => {
            let close = all.iter().skip(o + 1).position(|(_, l)| l.trim_start().starts_with("```")).map(|c| c + o + 1);
            (o + 1, close.unwrap_or(all.len()))
        }
        None => (0, all.len()),
    };
    all[from..to].iter().map(|(n, l)| (*n, l.trim_end().to_string())).collect()
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    let t = if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') { &t[1..t.len() - 1] } else { t };
    let t = t.replace("<br/>", "\n").replace("<br />", "\n").replace("<br>", "\n");
    t.trim().to_string()
}

pub fn parse(src: &str) -> Result<Graph, String> {
    let lines = body_lines(src);
    let mut cx = Ctx {
        g: Graph { dir: Dir::Td, nodes: vec![], subs: vec![], edges: vec![] },
        map: HashMap::new(),
        smap: HashMap::new(),
        stack: vec![],
    };
    let mut header = false;
    for (no, raw) in &lines {
        let mut t = raw.trim();
        if t.is_empty() || t.starts_with("%%") {
            continue;
        }
        t = t.trim_end_matches(';').trim();
        if t.is_empty() {
            continue;
        }
        if !header {
            let mut it = t.split_whitespace();
            let kw = it.next().unwrap_or("").to_ascii_lowercase();
            if kw != "flowchart" && kw != "graph" {
                return Err(format!("Line {}: expected 'flowchart TD' (or 'graph LR') first, found '{}'", no, clip(t)));
            }
            cx.g.dir = match it.next() {
                None => Dir::Td,
                Some(d) => dir_of(d).ok_or_else(|| format!("Line {}: unknown direction '{}' (use TD, BT, LR or RL)", no, clip(d)))?,
            };
            if it.next().is_some() {
                return Err(format!("Line {}: put the first statement on its own line", no));
            }
            header = true;
            continue;
        }
        let kw = t.split_whitespace().next().unwrap_or("");
        match kw {
            "style" | "classDef" | "class" | "linkStyle" | "click" => continue, // cosmetic, ignored
            "direction" => {
                let d = t.split_whitespace().nth(1).unwrap_or("");
                let d = dir_of(d).ok_or_else(|| format!("Line {}: unknown direction '{}' (use TB, LR, BT or RL)", no, clip(d)))?;
                if let Some(&j) = cx.stack.last() {
                    cx.g.subs[j].dir = Some(d);
                }
                continue;
            }
            "subgraph" => {
                open_sub(&mut cx, t["subgraph".len()..].trim()).map_err(|m| format!("Line {}: {}", no, m))?;
                continue;
            }
            "end" if t == "end" => {
                if cx.stack.pop().is_none() {
                    return Err(format!("Line {}: 'end' without a matching 'subgraph'", no));
                }
                continue;
            }
            _ => {}
        }
        let chars: Vec<char> = t.chars().collect();
        stmt(&chars, &mut cx).map_err(|m| format!("Line {}: {}", no, m))?;
        if cx.g.nodes.len() > MAX_NODES {
            return Err(format!("Line {}: too many nodes (max {})", no, MAX_NODES));
        }
        if cx.g.edges.len() > MAX_EDGES {
            return Err(format!("Line {}: too many edges (max {})", no, MAX_EDGES));
        }
    }
    if !header {
        return Err("No mermaid code found: start with 'flowchart TD'".into());
    }
    if let Some(&j) = cx.stack.last() {
        return Err(format!("Subgraph '{}' is not closed: add 'end'", cx.g.subs[j].id));
    }
    if cx.g.nodes.is_empty() {
        return Err("The diagram has no nodes".into());
    }
    Ok(cx.g)
}

fn dir_of(d: &str) -> Option<Dir> {
    match d.to_ascii_uppercase().as_str() {
        "TD" | "TB" => Some(Dir::Td),
        "BT" => Some(Dir::Bt),
        "LR" => Some(Dir::Lr),
        "RL" => Some(Dir::Rl),
        _ => None,
    }
}

/// `subgraph NAME`, `subgraph id [Label]`, `subgraph "Title"`
fn open_sub(cx: &mut Ctx, rest: &str) -> Result<(), String> {
    if rest.is_empty() {
        return Err("subgraph needs a name".into());
    }
    let ok_id = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let n = cx.g.subs.len();
    let (id, label) = if let (Some(p), Some(q)) = (rest.find('['), rest.rfind(']')) {
        if q < p || !ok_id(rest[..p].trim()) {
            return Err("bad subgraph name (use: subgraph id [Label])".into());
        }
        (rest[..p].trim().to_string(), unquote(&rest[p + 1..q]))
    } else if ok_id(rest) {
        (rest.to_string(), rest.to_string())
    } else {
        (format!("sg{}", n + 1), unquote(rest))
    };
    if n >= MAX_SUBS {
        return Err(format!("too many subgraphs (max {})", MAX_SUBS));
    }
    if cx.smap.contains_key(&id) || cx.map.contains_key(&id) {
        return Err(format!("'{}' is already used as a node or subgraph id", id));
    }
    cx.smap.insert(id.clone(), n);
    let parent = cx.stack.last().copied();
    cx.g.subs.push(Sub { id, label, dir: None, parent });
    cx.stack.push(n);
    Ok(())
}

fn clip(s: &str) -> String {
    s.chars().take(24).collect()
}

fn skip_ws(c: &[char], i: &mut usize) {
    while *i < c.len() && c[*i].is_whitespace() {
        *i += 1;
    }
}

fn starts(c: &[char], i: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    c.len() >= i + p.len() && c[i..i + p.len()] == p[..]
}

fn find(c: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..c.len()).find(|&i| starts(c, i, pat))
}

fn near(c: &[char], i: usize) -> String {
    c[i.min(c.len())..].iter().take(12).collect()
}

/// Label up to `close` (a quoted label may contain the closing characters).
fn label(c: &[char], i: &mut usize, close: &str) -> Result<String, String> {
    skip_ws(c, i);
    if *i < c.len() && c[*i] == '"' {
        let q = find(c, *i + 1, "\"").ok_or("missing closing quote")?;
        let text: String = c[*i + 1..q].iter().collect();
        *i = q + 1;
        skip_ws(c, i);
        if !starts(c, *i, close) {
            return Err(format!("expected '{}' after the quoted label", close));
        }
        *i += close.chars().count();
        return Ok(unquote(&text));
    }
    let e = find(c, *i, close).ok_or_else(|| format!("missing closing '{}'", close))?;
    let text: String = c[*i..e].iter().collect();
    *i = e + close.chars().count();
    Ok(unquote(&text))
}

fn node(c: &[char], i: &mut usize, cx: &mut Ctx) -> Result<End, String> {
    skip_ws(c, i);
    let s = *i;
    while *i < c.len() && (c[*i].is_ascii_alphanumeric() || c[*i] == '_') {
        *i += 1;
    }
    if *i == s {
        return Err(format!("expected a node id (letters, digits, _) near '{}'", near(c, s)));
    }
    let id: String = c[s..*i].iter().collect();
    let mut shape = None;
    let mut text = None;
    if let Some(&j) = cx.smap.get(&id) {
        if *i >= c.len() || !matches!(c[*i], '[' | '(' | '{') {
            return Ok(End::S(j)); // an edge to or from a subgraph
        }
    }
    if *i < c.len() {
        match c[*i] {
            '[' => {
                if starts(c, *i, "[[") || starts(c, *i, "[(") || starts(c, *i, "[/") || starts(c, *i, "[\\") {
                    return Err("this node shape is not supported (use [..], (..), {..} or ((..)))".into());
                }
                *i += 1;
                text = Some(label(c, i, "]")?);
                shape = Some(Shape::Rect);
            }
            '(' => {
                if starts(c, *i, "((") {
                    *i += 2;
                    text = Some(label(c, i, "))")?);
                    shape = Some(Shape::Circle);
                } else if starts(c, *i, "([") {
                    *i += 2;
                    text = Some(label(c, i, "])")?);
                    shape = Some(Shape::Stadium);
                } else if starts(c, *i, "(((") {
                    return Err("this node shape is not supported (use [..], (..), {..}, ([..]) or ((..)))".into());
                } else {
                    *i += 1;
                    text = Some(label(c, i, ")")?);
                    shape = Some(Shape::Round);
                }
            }
            '{' => {
                if starts(c, *i, "{{") {
                    return Err("this node shape is not supported (use [..], (..), {..} or ((..)))".into());
                }
                *i += 1;
                text = Some(label(c, i, "}")?);
                shape = Some(Shape::Diamond);
            }
            _ => {}
        }
    }
    let here = cx.stack.last().copied();
    let idx = match cx.map.get(&id) {
        Some(&k) => k,
        None => {
            cx.g.nodes.push(Node { id: id.clone(), label: id.clone(), shape: Shape::Rect, sub: here });
            cx.map.insert(id, cx.g.nodes.len() - 1);
            cx.g.nodes.len() - 1
        }
    };
    if cx.g.nodes[idx].sub.is_none() {
        cx.g.nodes[idx].sub = here;
    }
    if let Some(t) = text {
        cx.g.nodes[idx].label = if t.is_empty() { cx.g.nodes[idx].id.clone() } else { t };
    }
    if let Some(sh) = shape {
        cx.g.nodes[idx].shape = sh;
    }
    Ok(End::N(idx))
}

struct Op {
    head: bool,
    dash: bool,
    thick: bool,
    label: String,
}

fn edge_op(c: &[char], i: &mut usize) -> Result<Op, String> {
    skip_ws(c, i);
    let at = *i;
    let bad = move || format!("expected an edge (-->, ---, -.->, ==>) near '{}'", near(c, at));
    if *i >= c.len() {
        return Err(bad());
    }
    let mut op = Op { head: false, dash: false, thick: false, label: String::new() };
    let ch = c[*i];
    if ch == '=' {
        op.thick = true;
        let s = *i;
        while *i < c.len() && c[*i] == '=' {
            *i += 1;
        }
        let run = *i - s;
        if *i < c.len() && c[*i] == '>' {
            op.head = true;
            *i += 1;
        } else if run == 2 {
            let e = ["==>", "==="].iter().filter_map(|p| find(c, *i, p).map(|k| (k, *p))).min().ok_or_else(bad)?;
            op.label = unquote(&c[*i..e.0].iter().collect::<String>());
            op.head = e.1 == "==>";
            *i = e.0 + 3;
        }
    } else if ch == '-' && *i + 1 < c.len() && c[*i + 1] == '.' {
        op.dash = true;
        *i += 2;
        let inline = *i < c.len() && c[*i].is_whitespace();
        if inline {
            let k = find(c, *i, ".-").ok_or_else(bad)?;
            op.label = unquote(&c[*i..k].iter().collect::<String>());
            *i = k;
        }
        while *i < c.len() && c[*i] == '.' {
            *i += 1;
        }
        if *i >= c.len() || c[*i] != '-' {
            return Err(bad());
        }
        *i += 1;
        if *i < c.len() && c[*i] == '>' {
            op.head = true;
            *i += 1;
        }
    } else if ch == '-' && *i + 1 < c.len() && c[*i + 1] == '-' {
        let s = *i;
        while *i < c.len() && c[*i] == '-' {
            *i += 1;
        }
        let run = *i - s;
        if *i < c.len() && c[*i] == '>' {
            op.head = true;
            *i += 1;
        } else if run == 2 {
            if *i < c.len() && (c[*i] == 'o' || c[*i] == 'x') && (*i + 1 >= c.len() || c[*i + 1].is_whitespace()) {
                return Err("circle/cross edge ends are not supported".into());
            }
            let e = ["-->", "---"].iter().filter_map(|p| find(c, *i, p).map(|k| (k, *p))).min().ok_or_else(bad)?;
            op.label = unquote(&c[*i..e.0].iter().collect::<String>());
            op.head = e.1 == "-->";
            *i = e.0 + 3;
        }
    } else {
        return Err(bad());
    }
    skip_ws(c, i);
    if *i < c.len() && c[*i] == '|' {
        let e = find(c, *i + 1, "|").ok_or("missing closing '|' in the edge label")?;
        op.label = unquote(&c[*i + 1..e].iter().collect::<String>());
        *i = e + 1;
    }
    Ok(op)
}

fn stmt(c: &[char], cx: &mut Ctx) -> Result<(), String> {
    let mut i = 0;
    let mut prev = node(c, &mut i, cx)?;
    loop {
        skip_ws(c, &mut i);
        if i >= c.len() {
            return Ok(());
        }
        if c[i] == '&' {
            return Err("'&' (several nodes in one statement) is not supported".into());
        }
        let op = edge_op(c, &mut i)?;
        let next = node(c, &mut i, cx)?;
        cx.g.edges.push(Edge { a: prev, b: next, label: op.label, head: op.head, dash: op.dash, thick: op.thick });
        prev = next;
    }
}

// ------------------------------------------------------------------ layout

/// Wrap a label to at most `n` characters per line (the engine then draws it as is).
pub fn wrap_label(s: &str, n: usize) -> String {
    let mut out: Vec<String> = vec![];
    for para in s.split('\n') {
        let mut line = String::new();
        for w in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + w.chars().count() > n {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(w);
        }
        out.push(line);
    }
    out.join("\n")
}

/// (w, h) of the element for a node.
pub fn node_size(n: &Node) -> (f64, f64) {
    let text = wrap_label(&n.label, 24);
    let lines: Vec<&str> = text.split('\n').collect();
    let cw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
    let tw = cw * FS * 0.6;
    let th = lines.len() as f64 * FS * 1.3;
    match n.shape {
        Shape::Rect | Shape::Round => ((tw + 44.0).max(110.0), (th + 28.0).max(52.0)),
        Shape::Stadium => ((tw + 64.0).max(130.0), (th + 28.0).max(56.0)),
        Shape::Diamond => {
            let w = (tw / 0.62 + 20.0).max(140.0);
            (w, (th * 1.9 + 24.0).max(w * 0.5).max(84.0))
        }
        Shape::Circle => {
            let d = (tw / 0.62 + 10.0).max(th / 0.62 + 10.0).max(90.0);
            (d, d)
        }
    }
}

pub struct Layout {
    /// top-left (x, y) and size of each node, relative to the top-left of the whole flow
    pub pos: Vec<(f64, f64, f64, f64)>,
    /// same for each subgraph container
    pub subs: Vec<(f64, f64, f64, f64)>,
}

const NODE_SEP: f64 = 52.0;
const LANE_SEP: f64 = 30.0;
const RANK_SEP: f64 = 96.0;
const DUMMY: f64 = 8.0;
/// container padding and the strip reserved for its title
pub const SUB_PAD: f64 = 28.0;
pub const SUB_HEAD: f64 = 40.0;

/// Layered layout of abstract boxes. `edges` = (from, to, extra main-axis room for a label; 0 = no label).
/// Returns the top-left of every box, the whole drawing starting at (0, 0).
fn layered(sizes: &[(f64, f64)], edges: &[(usize, usize, f64)], dir: Dir) -> Vec<(f64, f64)> {
    let n = sizes.len();
    if n == 0 {
        return vec![];
    }
    let horiz = matches!(dir, Dir::Lr | Dir::Rl);
    // (cross, main) extents
    let ext = |i: usize| if horiz { (sizes[i].1, sizes[i].0) } else { (sizes[i].0, sizes[i].1) };

    // ---- break cycles: DFS back edges are reversed for layering only
    let mut out: Vec<Vec<usize>> = vec![vec![]; n];
    for (k, e) in edges.iter().enumerate() {
        if e.0 != e.1 {
            out[e.0].push(k);
        }
    }
    let mut state = vec![0u8; n];
    let mut rev = vec![false; edges.len()];
    fn dfs(u: usize, edges: &[(usize, usize, f64)], out: &[Vec<usize>], state: &mut [u8], rev: &mut [bool]) {
        state[u] = 1;
        for &k in &out[u] {
            let v = edges[k].1;
            if state[v] == 1 {
                rev[k] = true;
            } else if state[v] == 0 {
                dfs(v, edges, out, state, rev);
            }
        }
        state[u] = 2;
    }
    for u in 0..n {
        if state[u] == 0 {
            dfs(u, edges, &out, &mut state, &mut rev);
        }
    }
    // oriented edges (u -> v goes to a deeper layer)
    let oe: Vec<(usize, usize, f64)> = edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.0 != e.1)
        .map(|(k, e)| if rev[k] { (e.1, e.0, e.2) } else { (e.0, e.1, e.2) })
        .collect();

    // ---- longest-path layering
    let mut indeg = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    for &(u, v, _) in &oe {
        adj[u].push(v);
        indeg[v] += 1;
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let u = queue[qi];
        qi += 1;
        for &v in &adj[u] {
            layer[v] = layer[v].max(layer[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    let nl = layer.iter().copied().max().unwrap_or(0) + 1;

    // ---- layered graph with dummy nodes for long edges (they reserve a lane)
    let mut lay_of: Vec<usize> = layer.clone();
    let mut up: Vec<Vec<usize>> = vec![vec![]; n];
    let mut down: Vec<Vec<usize>> = vec![vec![]; n];
    let mut gap_extra = vec![0.0f64; nl];
    for &(u, v, extra) in &oe {
        let mut prev = u;
        for l in layer[u] + 1..layer[v] {
            let d = lay_of.len();
            lay_of.push(l);
            up.push(vec![]);
            down.push(vec![]);
            down[prev].push(d);
            up[d].push(prev);
            prev = d;
        }
        down[prev].push(v);
        up[v].push(prev);
        if extra > 0.0 {
            for l in layer[u]..layer[v] {
                gap_extra[l] = gap_extra[l].max(extra);
            }
        }
    }
    let total = lay_of.len();
    let mut layers: Vec<Vec<usize>> = vec![vec![]; nl];
    for v in 0..total {
        layers[lay_of[v]].push(v);
    }
    // ---- ordering: barycentre sweeps
    let mut pos = vec![0.0f64; total];
    for l in &layers {
        for (i, &v) in l.iter().enumerate() {
            pos[v] = i as f64;
        }
    }
    for it in 0..8 {
        let downward = it % 2 == 0;
        let order: Vec<usize> = if downward { (1..nl).collect() } else { (0..nl.saturating_sub(1)).rev().collect() };
        for l in order {
            let nb = if downward { &up } else { &down };
            let mut keyed: Vec<(f64, usize, usize)> = layers[l]
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let ns = &nb[v];
                    let k = if ns.is_empty() { pos[v] } else { ns.iter().map(|&u| pos[u]).sum::<f64>() / ns.len() as f64 };
                    (k, i, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
            layers[l] = keyed.into_iter().map(|k| k.2).collect();
            for (i, &v) in layers[l].iter().enumerate() {
                pos[v] = i as f64;
            }
        }
    }

    // ---- coordinates along the cross axis
    let cross = |v: usize| if v < n { ext(v).0 } else { DUMMY };
    let sep = |a: usize, b: usize| if a < n && b < n { NODE_SEP } else { LANE_SEP };
    let mut cc = vec![0.0f64; total]; // centre
    for l in &layers {
        let mut x = 0.0;
        for (i, &v) in l.iter().enumerate() {
            if i > 0 {
                x += sep(l[i - 1], v);
            }
            cc[v] = x + cross(v) / 2.0;
            x += cross(v);
        }
    }
    // centre the layers on the widest one
    let width: Vec<f64> = layers.iter().map(|l| l.last().map_or(0.0, |&v| cc[v] + cross(v) / 2.0)).collect();
    let wmax = width.iter().cloned().fold(0.0, f64::max);
    for (l, w) in layers.iter().zip(&width) {
        for &v in l {
            cc[v] += (wmax - w) / 2.0;
        }
    }
    for it in 0..8 {
        let downward = it % 2 == 0;
        let order: Vec<usize> = if downward { (1..nl).collect() } else { (0..nl.saturating_sub(1)).rev().collect() };
        for l in order {
            let nb = if downward { &up } else { &down };
            let mut edge = f64::NEG_INFINITY;
            let mut prev: Option<usize> = None;
            for &v in &layers[l] {
                let ns = &nb[v];
                let want = if ns.is_empty() { cc[v] } else { ns.iter().map(|&u| cc[u]).sum::<f64>() / ns.len() as f64 };
                let min_c = match prev {
                    Some(p) => edge + sep(p, v) + cross(v) / 2.0,
                    None => f64::NEG_INFINITY,
                };
                cc[v] = want.max(min_c);
                edge = cc[v] + cross(v) / 2.0;
                prev = Some(v);
            }
        }
    }
    let minc = (0..n).map(|v| cc[v] - cross(v) / 2.0).fold(f64::INFINITY, f64::min);
    let minc = if minc.is_finite() { minc } else { 0.0 };

    // ---- main axis
    let mut thick = vec![0.0f64; nl];
    for v in 0..n {
        thick[lay_of[v]] = thick[lay_of[v]].max(ext(v).1);
    }
    let mut start = vec![0.0f64; nl];
    let mut m = 0.0;
    for l in 0..nl {
        start[l] = m;
        m += thick[l] + RANK_SEP + gap_extra[l];
    }
    let total_main = (m - RANK_SEP - gap_extra[nl - 1]).max(0.0);

    let mut res = Vec::with_capacity(n);
    for v in 0..n {
        let (cw, mh) = ext(v);
        let c0 = cc[v] - cw / 2.0 - minc;
        let mut m0 = start[lay_of[v]] + (thick[lay_of[v]] - mh) / 2.0;
        if matches!(dir, Dir::Bt | Dir::Rl) {
            m0 = total_main - (m0 + mh);
        }
        res.push(if horiz { (m0, c0) } else { (c0, m0) });
    }
    res
}

#[derive(Clone, Copy, PartialEq)]
enum Item {
    N(usize),
    S(usize),
}

/// Main-axis room a label needs between two ranks.
fn label_room(label: &str, horiz: bool) -> f64 {
    if label.is_empty() {
        0.0
    } else if horiz {
        (crate::geom::label_size(label, LABEL_FS).0 + 24.0).clamp(44.0, 240.0)
    } else {
        44.0
    }
}

/// Lay out the whole graph. Subgraphs are laid out inside-out: every container is sized from its own
/// content (using its own direction), then the parent arranges containers and nodes like ordinary boxes.
pub fn layout(g: &Graph) -> Layout {
    let (nn, ns) = (g.nodes.len(), g.subs.len());
    let nsz: Vec<(f64, f64)> = g.nodes.iter().map(node_size).collect();
    let mut ssz = vec![(0.0f64, 0.0f64); ns];
    // effective direction (a parent always has a lower index than its children)
    let mut eff: Vec<Dir> = Vec::with_capacity(ns);
    for j in 0..ns {
        let up = g.subs[j].parent.map_or(g.dir, |p| eff[p]);
        eff.push(g.subs[j].dir.unwrap_or(up));
    }
    let parent_of = |it: Item| match it {
        Item::N(i) => g.nodes[i].sub,
        Item::S(j) => g.subs[j].parent,
    };
    // the direct child of `grp` that holds `e` (None when `e` is outside `grp`)
    let lift = |e: End, grp: Option<usize>| -> Option<Item> {
        let mut it = match e {
            End::N(i) => Item::N(i),
            End::S(j) => Item::S(j),
        };
        loop {
            let p = parent_of(it);
            if p == grp {
                return Some(it);
            }
            match p {
                Some(q) => it = Item::S(q),
                None => return None,
            }
        }
    };
    // kids[grp] = (item, x, y) relative to the group's top-left; grp = ns is the root
    let mut kids: Vec<Vec<(Item, f64, f64)>> = vec![vec![]; ns + 1];
    for gi in (0..ns).rev().chain(std::iter::once(ns)) {
        // children (higher index) first, the root last
        let grp = if gi == ns { None } else { Some(gi) };
        let dir = grp.map_or(g.dir, |j| eff[j]);
        let horiz = matches!(dir, Dir::Lr | Dir::Rl);
        let mut items: Vec<Item> = (0..nn).filter(|&i| g.nodes[i].sub == grp).map(Item::N).collect();
        items.extend((0..ns).filter(|&j| g.subs[j].parent == grp).map(Item::S));
        let sizes: Vec<(f64, f64)> = items
            .iter()
            .map(|it| match it {
                Item::N(i) => nsz[*i],
                Item::S(j) => ssz[*j],
            })
            .collect();
        let local = |it: Item| items.iter().position(|x| *x == it);
        let mut es: Vec<(usize, usize, f64)> = vec![];
        for e in &g.edges {
            if let (Some(a), Some(b)) = (lift(e.a, grp), lift(e.b, grp)) {
                if let (Some(a), Some(b)) = (local(a), local(b)) {
                    if a == b {
                        continue;
                    }
                    let room = label_room(&e.label, horiz);
                    match es.iter_mut().find(|x| x.0 == a && x.1 == b) {
                        Some(x) => x.2 = x.2.max(room),
                        None => es.push((a, b, room)),
                    }
                }
            }
        }
        let pos = layered(&sizes, &es, dir);
        let (mut cw, mut ch) = (0.0f64, 0.0f64);
        for (p, s) in pos.iter().zip(&sizes) {
            cw = cw.max(p.0 + s.0);
            ch = ch.max(p.1 + s.1);
        }
        match grp {
            None => kids[ns] = items.iter().zip(&pos).map(|(it, p)| (*it, p.0, p.1)).collect(),
            Some(j) => {
                if items.is_empty() {
                    cw = 120.0;
                    ch = 40.0;
                }
                let title = crate::geom::label_size(&g.subs[j].label, FS).0 + 20.0;
                let w = (cw + 2.0 * SUB_PAD).max(title).max(150.0);
                let h = ch + SUB_PAD + SUB_HEAD;
                let dx = (w - cw) / 2.0;
                ssz[j] = (w, h);
                kids[j] = items.iter().zip(&pos).map(|(it, p)| (*it, p.0 + dx, p.1 + SUB_HEAD)).collect();
            }
        }
    }
    // absolute positions, top-down
    let mut npos = vec![(0.0, 0.0, 0.0, 0.0); nn];
    let mut spos = vec![(0.0, 0.0, 0.0, 0.0); ns];
    let mut todo: Vec<(usize, f64, f64)> = vec![(ns, 0.0, 0.0)];
    while let Some((gi, ox, oy)) = todo.pop() {
        for &(it, x, y) in &kids[gi] {
            match it {
                Item::N(i) => npos[i] = (ox + x, oy + y, nsz[i].0, nsz[i].1),
                Item::S(j) => {
                    spos[j] = (ox + x, oy + y, ssz[j].0, ssz[j].1);
                    todo.push((j, ox + x, oy + y));
                }
            }
        }
    }
    Layout { pos: npos, subs: spos }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fence_and_shapes() {
        let g = parse("Here:\n```mermaid\nflowchart LR\n  A[Start] --> B{Ok?}\n  B -->|yes| C((Done))\n  B -- no --> D(Retry)\n```\n").unwrap();
        assert_eq!(g.nodes.len(), 4);
        assert_eq!(g.edges.len(), 3);
        assert_eq!(g.edges[1].label, "yes");
        assert_eq!(g.edges[2].label, "no");
        assert_eq!(g.nodes[1].shape, Shape::Diamond);
        assert_eq!(g.dir, Dir::Lr);
    }

    #[test]
    fn reports_the_line() {
        let e = parse("graph TD\nA --> B\nB ~~> C\n").unwrap_err();
        assert!(e.starts_with("Line 3"), "{}", e);
        assert!(parse("hello").is_err());
        let e = parse("graph TD\nsubgraph X\nA --> B\n").unwrap_err();
        assert!(e.contains("not closed"), "{}", e);
        assert!(parse("graph TD\nA --> B\nend\n").unwrap_err().starts_with("Line 3"));
    }

    #[test]
    fn layers_do_not_overlap() {
        let g = parse("flowchart TD\nA-->B\nA-->C\nB-->D\nC-->D\nA-->D\nD-->A\n").unwrap();
        let l = layout(&g);
        for i in 0..l.pos.len() {
            for j in i + 1..l.pos.len() {
                let (a, b) = (l.pos[i], l.pos[j]);
                assert!(a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1);
            }
        }
    }

    const SAMPLE: &str = include_str!("../../tests/fixtures/mermaid-r12.txt");

    fn apart(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
        a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1
    }

    #[test]
    fn the_real_sample() {
        let g = parse(SAMPLE).unwrap();
        assert_eq!(g.subs.len(), 3);
        assert_eq!(g.nodes.len(), 13);
        assert_eq!(g.edges.len(), 16);
        assert_eq!(g.edges.iter().filter(|e| matches!((e.a, e.b), (End::S(_), End::S(_)))).count(), 2);
        assert!(g.subs.iter().all(|s| s.dir == Some(Dir::Td)));
        assert_eq!(g.nodes.iter().filter(|n| n.shape == Shape::Stadium).count(), 1);
        assert_eq!(g.nodes.iter().filter(|n| n.shape == Shape::Diamond).count(), 1);
        assert!(g.nodes.iter().all(|n| !n.label.contains("<br")));
        assert!(g.nodes.iter().any(|n| n.label.contains('\n')));
        let labels: Vec<&str> = g.edges.iter().map(|e| e.label.as_str()).filter(|l| !l.is_empty()).collect();
        assert_eq!(labels, ["gagal", "gagal", "gagal", "Belum, masih ada issue", "Sudah"]);
        let nsub = |name: &str| g.nodes.iter().filter(|n| n.sub.map_or(false, |s| g.subs[s].id == name)).count();
        assert_eq!((nsub("PLANNER"), nsub("BUILDER"), nsub("EVALUATOR")), (3, 4, 5));
        let l = layout(&g);
        let s = &l.subs;
        assert!(s[0].0 + s[0].2 <= s[1].0 && s[1].0 + s[1].2 <= s[2].0, "{:?}", s);
        for (i, n) in g.nodes.iter().enumerate() {
            if let Some(j) = n.sub {
                let (c, p) = (l.subs[j], l.pos[i]);
                assert!(
                    p.0 >= c.0 + 1.0 && p.1 >= c.1 + SUB_HEAD - 0.1 && p.0 + p.2 <= c.0 + c.2 && p.1 + p.3 <= c.1 + c.3,
                    "{} outside {:?}",
                    n.id,
                    c
                );
            }
        }
        for i in 0..l.pos.len() {
            for j in i + 1..l.pos.len() {
                assert!(apart(l.pos[i], l.pos[j]), "{} overlaps {}", g.nodes[i].id, g.nodes[j].id);
            }
        }
        for i in 0..s.len() {
            for j in i + 1..s.len() {
                assert!(apart(s[i], s[j]));
            }
            for (k, n) in g.nodes.iter().enumerate() {
                if n.sub != Some(i) {
                    assert!(apart(s[i], l.pos[k]), "{} inside container {}", n.id, g.subs[i].id);
                }
            }
        }
        // top-to-bottom inside each container
        for name in ["P", "B", "E"] {
            let ys: Vec<f64> = g.nodes.iter().enumerate().filter(|(_, n)| n.id.starts_with(name) && n.id.len() == 2).map(|(i, _)| l.pos[i].1).collect();
            assert!(ys.windows(2).all(|w| w[0] < w[1]), "{} {:?}", name, ys);
        }
    }

    #[test]
    fn nested_subgraph_and_directions() {
        let g = parse("flowchart TD\nsubgraph outer [Outer one]\ndirection LR\nA --> B\nsubgraph inner\nC --> D\nend\nend\nB --> C\nouter --> Z\n").unwrap();
        assert_eq!(g.subs.len(), 2);
        assert_eq!(g.subs[1].parent, Some(0));
        let l = layout(&g);
        let (o, i) = (l.subs[0], l.subs[1]);
        assert!(i.0 >= o.0 && i.1 >= o.1 && i.0 + i.2 <= o.0 + o.2 && i.1 + i.3 <= o.1 + o.3);
        assert!(l.pos[0].0 < l.pos[1].0); // outer is LR: A left of B
    }
}
