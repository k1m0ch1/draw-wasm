//! The JS-side drawing surface. A thin object implemented in web/gfx.js that wraps a Canvas2D
//! context, so the engine does not need web-sys at all.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    pub type Gfx;
    /// reset the transform and clear the whole canvas (device pixels)
    #[wasm_bindgen(method)]
    pub fn clear(this: &Gfx, w: f64, h: f64);
    /// set the world->device transform: scale, translate x/y (device pixels)
    #[wasm_bindgen(method)]
    pub fn view(this: &Gfx, scale: f64, tx: f64, ty: f64);
    #[wasm_bindgen(method)]
    pub fn begin(this: &Gfx);
    #[wasm_bindgen(method)]
    pub fn mv(this: &Gfx, x: f64, y: f64);
    #[wasm_bindgen(method)]
    pub fn ln(this: &Gfx, x: f64, y: f64);
    #[wasm_bindgen(method)]
    pub fn quad(this: &Gfx, cx: f64, cy: f64, x: f64, y: f64);
    #[wasm_bindgen(method)]
    pub fn ell(this: &Gfx, cx: f64, cy: f64, rx: f64, ry: f64);
    #[wasm_bindgen(method)]
    pub fn rrect(this: &Gfx, x: f64, y: f64, w: f64, h: f64, r: f64);
    #[wasm_bindgen(method)]
    pub fn close(this: &Gfx);
    #[wasm_bindgen(method)]
    pub fn fill(this: &Gfx, colour: &str);
    #[wasm_bindgen(method)]
    pub fn stroke(this: &Gfx, colour: &str, width: f64, dash: f64);
    /// single line of text, vertically centred on y. align: "left" | "center"
    #[wasm_bindgen(method)]
    pub fn text(this: &Gfx, s: &str, x: f64, y: f64, size: f64, colour: &str, align: &str);
    #[wasm_bindgen(method)]
    pub fn measure(this: &Gfx, s: &str, size: f64) -> f64;
    #[wasm_bindgen(method)]
    pub fn image(this: &Gfx, hash: &str, x: f64, y: f64, w: f64, h: f64);
}
