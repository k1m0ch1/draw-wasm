//! Wire/data types shared by the WASM engine and the server, so the format cannot drift.
//!
//! The document is a map `id -> Element`. Every change (add, update, delete, z-order) is a
//! *put* of a whole element carrying `(v, by)`. The element with the greater `(v, by)` wins
//! (last-writer-wins, ties broken by writer id), which makes merging commutative, associative
//! and idempotent. Deletion is a put with `del = true` (a tombstone).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Text elements may hold a whole mermaid diagram or HTML page (the HTML object itself is capped at MAX_HTML_BYTES).
pub const MAX_TEXT_BYTES: usize = 131_072;
/// Source of an HTML object (rendered in a sandboxed iframe without scripts).
pub const MAX_HTML_BYTES: usize = 65_536;
pub const MAX_PTS: usize = 40_000;
pub const MAX_COORD: f64 = 1.0e7;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Rect,
    Ellipse,
    Diamond,
    Arrow,
    Pencil,
    Text,
    Image,
    Html,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero_f(v: &f64) -> bool {
    *v == 0.0
}
fn is_zero_u(v: &u8) -> bool {
    *v == 0
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Element {
    pub id: String,
    /// version, incremented by the writer on every change
    pub v: u32,
    /// last writer (client id)
    pub by: String,
    pub k: Kind,
    #[serde(default, skip_serializing_if = "is_false")]
    pub del: bool,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
    /// stacking order (fractional index)
    #[serde(default)]
    pub z: f64,
    /// stroke colour (text colour for text)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub st: String,
    /// fill colour, empty = none
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fl: String,
    /// stroke width
    #[serde(default, skip_serializing_if = "is_zero_f")]
    pub sw: f64,
    /// arrowheads: bit0 = end, bit1 = start
    #[serde(default, skip_serializing_if = "is_zero_u")]
    pub ah: u8,
    /// font size
    #[serde(default, skip_serializing_if = "is_zero_f")]
    pub fs: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tx: String,
    /// pencil: points relative to (x,y); arrow: 4 absolute numbers x1,y1,x2,y2
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pts: Vec<f64>,
    /// arrow start / end binding (element id)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sb: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub eb: String,
    /// image: sha256 (hex) of the stored bytes
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub img: String,
    /// html object: the markup (rendered in an iframe sandbox without allow-scripts / allow-same-origin)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub src: String,
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_colour(s: &str) -> bool {
    s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"#(),.% ".contains(&b))
}

fn fin(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORD
}

impl Element {
    /// Structural validation used by the server for every incoming element.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_id(&self.id) {
            return Err("bad id");
        }
        if !valid_id(&self.by) {
            return Err("bad writer");
        }
        if self.v >= u32::MAX - 1 {
            return Err("bad version");
        }
        if ![self.x, self.y, self.w, self.h, self.z].iter().all(|v| fin(*v)) {
            return Err("bad number");
        }
        if !(self.sw.is_finite() && (0.0..=64.0).contains(&self.sw)) {
            return Err("bad stroke");
        }
        if !(self.fs.is_finite() && (0.0..=512.0).contains(&self.fs)) {
            return Err("bad font size");
        }
        if !valid_colour(&self.st) || !valid_colour(&self.fl) {
            return Err("bad colour");
        }
        if self.tx.len() > MAX_TEXT_BYTES {
            return Err("text too long");
        }
        if self.pts.len() > MAX_PTS || !self.pts.iter().all(|v| fin(*v)) {
            return Err("bad points");
        }
        if self.k == Kind::Arrow && !self.del && self.pts.len() != 4 {
            return Err("bad arrow");
        }
        for b in [&self.sb, &self.eb] {
            if !b.is_empty() && !valid_id(b) {
                return Err("bad binding");
            }
        }
        if !self.img.is_empty()
            && !(self.img.len() == 64 && self.img.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            return Err("bad image ref");
        }
        if self.k == Kind::Image && !self.del && self.img.is_empty() {
            return Err("image without ref");
        }
        if self.src.len() > MAX_HTML_BYTES {
            return Err("html too large");
        }
        if !self.src.is_empty() && self.k != Kind::Html {
            return Err("bad html payload");
        }
        if self.k == Kind::Html && !self.del && self.src.trim().is_empty() {
            return Err("html without source");
        }
        Ok(())
    }
}

/// Does `new` beat `old`? Strictly greater `(v, by)`; equal means "already have it".
pub fn wins(new: &Element, old: &Element) -> bool {
    (new.v, new.by.as_str()) > (old.v, old.by.as_str())
}

#[derive(Default, Clone, Debug)]
pub struct Doc {
    pub els: HashMap<String, Element>,
}

impl Doc {
    /// Apply one put with last-writer-wins. Returns true when the document changed.
    pub fn apply(&mut self, e: &Element) -> bool {
        if let Some(old) = self.els.get(&e.id) {
            if !wins(e, old) {
                return false;
            }
        }
        let mut n = e.clone();
        if n.del {
            n.pts.clear();
            n.tx.clear();
            n.src.clear();
        }
        self.els.insert(n.id.clone(), n);
        true
    }
    pub fn live_count(&self) -> usize {
        self.els.values().filter(|e| !e.del).count()
    }
}

mod wire;
pub use wire::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn el(id: &str, v: u32, by: &str) -> Element {
        Element { id: id.into(), v, by: by.into(), ..Default::default() }
    }

    #[test]
    fn html_validation() {
        let mut e = el("h1", 1, "a");
        e.k = Kind::Html;
        assert!(e.validate().is_err());
        e.src = "<p>hi</p>".into();
        assert!(e.validate().is_ok());
        e.src = "x".repeat(MAX_HTML_BYTES + 1);
        assert!(e.validate().is_err());
        let mut r = el("r1", 1, "a");
        r.src = "<p>".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn lww_is_order_independent() {
        let a = el("e1", 4, "a");
        let b = el("e1", 4, "b");
        let mut d1 = Doc::default();
        d1.apply(&a);
        d1.apply(&b);
        let mut d2 = Doc::default();
        d2.apply(&b);
        d2.apply(&a);
        assert_eq!(d1.els["e1"].by, "b");
        assert_eq!(d2.els["e1"].by, "b");
        assert!(!d1.apply(&b));
    }

    #[test]
    fn rooms() {
        assert!(valid_room("my-room").is_ok());
        assert!(valid_room("api").is_err());
        assert!(valid_room("-ab").is_err());
        assert!(valid_room("AB1").is_err());
        assert!(valid_room("../x").is_err());
        assert!(valid_room("caf\u{e9}1").is_err());
    }
}
