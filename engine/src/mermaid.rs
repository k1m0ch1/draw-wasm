//! Mermaid flowchart subset: parser and layered layout. Pure Rust, no I/O.
//!
//! Supported: `flowchart|graph` + TD/TB/LR/RL/BT; nodes `[rect]`, `(rounded)`, `{diamond}`, `((circle))`;
//! edges `---`, `-->`, `-.->`, `-.-`, `==>`, `===`, with `|label|` or `-- label -->`; chains (`A --> B --> C`);
//! one statement per line; `%%` comments; a surrounding markdown fence is stripped.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Rect,
    Round,
    Diamond,
    Circle,
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
}

#[derive(Debug)]
pub struct Edge {
    pub a: usize,
    pub b: usize,
    pub label: String,
    pub head: bool,
    pub dash: bool,
    pub thick: bool,
}

#[derive(Debug)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

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
    let mut g = Graph { dir: Dir::Td, nodes: vec![], edges: vec![] };
    let mut map: HashMap<String, usize> = HashMap::new();
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
            g.dir = match it.next().map(|d| d.to_ascii_uppercase()).as_deref() {
                None | Some("TD") | Some("TB") => Dir::Td,
                Some("BT") => Dir::Bt,
                Some("LR") => Dir::Lr,
                Some("RL") => Dir::Rl,
                Some(d) => return Err(format!("Line {}: unknown direction '{}' (use TD, BT, LR or RL)", no, clip(d))),
            };
            if it.next().is_some() {
                return Err(format!("Line {}: put the first statement on its own line", no));
            }
            header = true;
            continue;
        }
        let kw = t.split_whitespace().next().unwrap_or("");
        match kw {
            "style" | "classDef" | "class" | "linkStyle" | "click" | "direction" => continue, // cosmetic, ignored
            "subgraph" | "end" => return Err(format!("Line {}: subgraph is not supported", no)),
            _ => {}
        }
        let chars: Vec<char> = t.chars().collect();
        stmt(&chars, &mut g, &mut map).map_err(|m| format!("Line {}: {}", no, m))?;
        if g.nodes.len() > MAX_NODES {
            return Err(format!("Line {}: too many nodes (max {})", no, MAX_NODES));
        }
        if g.edges.len() > MAX_EDGES {
            return Err(format!("Line {}: too many edges (max {})", no, MAX_EDGES));
        }
    }
    if !header {
        return Err("No mermaid code found: start with 'flowchart TD'".into());
    }
    if g.nodes.is_empty() {
        return Err("The diagram has no nodes".into());
    }
    Ok(g)
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

fn node(c: &[char], i: &mut usize, g: &mut Graph, map: &mut HashMap<String, usize>) -> Result<usize, String> {
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
                } else if starts(c, *i, "([") || starts(c, *i, "(((") {
                    return Err("this node shape is not supported (use [..], (..), {..} or ((..)))".into());
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
    let idx = match map.get(&id) {
        Some(&k) => k,
        None => {
            g.nodes.push(Node { id: id.clone(), label: id.clone(), shape: Shape::Rect });
            map.insert(id, g.nodes.len() - 1);
            g.nodes.len() - 1
        }
    };
    if let Some(t) = text {
        g.nodes[idx].label = if t.is_empty() { g.nodes[idx].id.clone() } else { t };
    }
    if let Some(sh) = shape {
        g.nodes[idx].shape = sh;
    }
    Ok(idx)
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

fn stmt(c: &[char], g: &mut Graph, map: &mut HashMap<String, usize>) -> Result<(), String> {
    let mut i = 0;
    let mut prev = node(c, &mut i, g, map)?;
    loop {
        skip_ws(c, &mut i);
        if i >= c.len() {
            return Ok(());
        }
        if c[i] == '&' {
            return Err("'&' (several nodes in one statement) is not supported".into());
        }
        let op = edge_op(c, &mut i)?;
        let next = node(c, &mut i, g, map)?;
        g.edges.push(Edge { a: prev, b: next, label: op.label, head: op.head, dash: op.dash, thick: op.thick });
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
}

const NODE_SEP: f64 = 52.0;
const LANE_SEP: f64 = 30.0;
const RANK_SEP: f64 = 96.0;
const LABEL_RANK: f64 = 40.0;
const DUMMY: f64 = 8.0;

pub fn layout(g: &Graph) -> Layout {
    let n = g.nodes.len();
    let horiz = matches!(g.dir, Dir::Lr | Dir::Rl);
    let size: Vec<(f64, f64)> = g.nodes.iter().map(node_size).collect();
    // (cross, main) extents
    let ext = |i: usize| if horiz { (size[i].1, size[i].0) } else { (size[i].0, size[i].1) };

    // ---- break cycles: DFS back edges are reversed for layering only
    let mut out: Vec<Vec<usize>> = vec![vec![]; n];
    for (k, e) in g.edges.iter().enumerate() {
        if e.a != e.b {
            out[e.a].push(k);
        }
    }
    let mut state = vec![0u8; n];
    let mut rev = vec![false; g.edges.len()];
    fn dfs(u: usize, g: &Graph, out: &[Vec<usize>], state: &mut [u8], rev: &mut [bool]) {
        state[u] = 1;
        for &k in &out[u] {
            let v = g.edges[k].b;
            if state[v] == 1 {
                rev[k] = true;
            } else if state[v] == 0 {
                dfs(v, g, out, state, rev);
            }
        }
        state[u] = 2;
    }
    for u in 0..n {
        if state[u] == 0 {
            dfs(u, g, &out, &mut state, &mut rev);
        }
    }
    // oriented edges (u -> v goes to a deeper layer)
    let oe: Vec<(usize, usize, bool)> = g
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.a != e.b)
        .map(|(k, e)| if rev[k] { (e.b, e.a, !e.label.is_empty()) } else { (e.a, e.b, !e.label.is_empty()) })
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
    let mut labelled_gap = vec![false; nl];
    for &(u, v, lab) in &oe {
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
        if lab {
            for l in layer[u]..layer[v] {
                labelled_gap[l] = true;
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
    let renumber = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f64>| {
        for l in layers {
            for (i, &v) in l.iter().enumerate() {
                pos[v] = i as f64;
            }
        }
    };
    renumber(&layers, &mut pos);
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
        m += thick[l] + RANK_SEP + if labelled_gap[l] { LABEL_RANK } else { 0.0 };
    }
    let total_main = (m - RANK_SEP - if labelled_gap[nl - 1] { LABEL_RANK } else { 0.0 }).max(0.0);

    let mut res = Vec::with_capacity(n);
    for v in 0..n {
        let (cw, mh) = ext(v);
        let c0 = cc[v] - cw / 2.0 - minc;
        let mut m0 = start[lay_of[v]] + (thick[lay_of[v]] - mh) / 2.0;
        if matches!(g.dir, Dir::Bt | Dir::Rl) {
            m0 = total_main - (m0 + mh);
        }
        let (x, y) = if horiz { (m0, c0) } else { (c0, m0) };
        res.push((x, y, size[v].0, size[v].1));
    }
    Layout { pos: res }
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
}
