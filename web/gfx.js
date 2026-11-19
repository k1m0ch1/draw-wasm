// Canvas2D drawing surface used by the WASM engine (see engine/src/gfx.rs).
export const FONT = 'Inter, "Segoe UI", system-ui, -apple-system, Roboto, "Helvetica Neue", Arial, sans-serif';

export class Gfx {
  constructor(canvas, images) {
    this.c = canvas;
    this.x = canvas.getContext('2d');
    this.images = images; // hash -> {img, state}
    this.scale = 1;
    this.animNext = Infinity; // when an animated image drawn in the last render needs its next frame
    this.onImageReady = () => {};
  }
  clear(w, h) {
    const x = this.x;
    this.animNext = Infinity;
    this.images.frameNo++;
    x.setTransform(1, 0, 0, 1, 0, 0);
    x.clearRect(0, 0, w, h);
    x.lineCap = 'round';
    x.lineJoin = 'round';
  }
  view(s, tx, ty) {
    this.scale = s;
    this.x.setTransform(s, 0, 0, s, tx, ty);
  }
  begin() { this.x.beginPath(); }
  mv(x, y) { this.x.moveTo(x, y); }
  ln(x, y) { this.x.lineTo(x, y); }
  quad(cx, cy, x, y) { this.x.quadraticCurveTo(cx, cy, x, y); }
  ell(cx, cy, rx, ry) {
    this.x.moveTo(cx + Math.max(rx, 0), cy);
    this.x.ellipse(cx, cy, Math.max(rx, 0.01), Math.max(ry, 0.01), 0, 0, Math.PI * 2);
  }
  rrect(x, y, w, h, r) {
    const c = this.x;
    if (r <= 0) { c.rect(x, y, w, h); return; }
    c.moveTo(x + r, y);
    c.arcTo(x + w, y, x + w, y + h, r);
    c.arcTo(x + w, y + h, x, y + h, r);
    c.arcTo(x, y + h, x, y, r);
    c.arcTo(x, y, x + w, y, r);
    c.closePath();
  }
  close() { this.x.closePath(); }
  fill(colour) { this.x.fillStyle = colour; this.x.fill(); }
  stroke(colour, width, dash) {
    const c = this.x;
    c.strokeStyle = colour;
    c.lineWidth = width;
    c.setLineDash(dash > 0 ? [dash, dash] : []);
    c.stroke();
    c.setLineDash([]);
  }
  text(s, x, y, size, colour, align) {
    const c = this.x;
    c.font = `${size}px ${FONT}`;
    c.textBaseline = 'middle';
    c.textAlign = align === 'center' ? 'center' : 'left';
    c.fillStyle = colour;
    c.fillText(s, x, y);
  }
  measure(s, size) {
    this.x.font = `${size}px ${FONT}`;
    return this.x.measureText(s).width;
  }
  badge(text, x, y, bg) {
    const c = this.x, s = this.scale;
    c.save();
    c.font = `${12 / s}px ${FONT}`;
    c.textBaseline = 'middle';
    c.textAlign = 'left';
    const pw = c.measureText(text).width + 12 / s, ph = 20 / s;
    c.fillStyle = bg;
    c.fillRect(x, y, pw, ph);
    c.fillStyle = '#fff';
    c.fillText(text, x + 6 / s, y + ph / 2);
    c.restore();
  }
  image(hash, x, y, w, h) {
    const c = this.x;
    const e = this.images.get(hash);
    if (e && e.state === 'ok') {
      e.last = this.images.frameNo;
      let src = e.img;
      if (e.anim) { // animated GIF: pick the frame for the current time and ask for a redraw when it ends
        const f = e.anim.frameAt(performance.now());
        src = f.src;
        this.animNext = Math.min(this.animNext, f.until);
      }
      try { c.drawImage(src, x, y, w, h); } catch { c.drawImage(e.img, x, y, w, h); }
      const u = this.images.status(hash);
      if (u && u.state === 'failed') {
        c.save();
        c.strokeStyle = '#e03131';
        c.lineWidth = 3 / this.scale;
        c.strokeRect(x, y, w, h);
        c.restore();
        this.badge('Not shared: ' + u.msg, x, y, '#c92a2a');
      } else if (u) {
        this.badge(u.state === 'retrying' ? 'Upload failed, retrying...' : 'Uploading...', x, y, u.state === 'retrying' ? '#e8590c' : '#4f6bed');
      }
      return;
    }
    // placeholder
    c.save();
    c.fillStyle = '#f1f3f5';
    c.fillRect(x, y, w, h);
    c.strokeStyle = '#ced4da';
    c.lineWidth = 1 / this.scale;
    c.strokeRect(x, y, w, h);
    c.restore();
    this.badge(e && e.state === 'err' ? 'Image unavailable, retrying...' : 'Loading image...', x, y, e && e.state === 'err' ? '#c92a2a' : '#868e96');
    if (e) e.last = this.images.frameNo;
    if (!e) this.images.load(hash, () => this.onImageReady());
  }
}
