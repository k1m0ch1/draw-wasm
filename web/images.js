// Image cache (hash -> decoded bitmap) and background uploads.
// Looks in IndexedDB first, then the server. Loads and uploads retry with backoff.
import { idb } from './db.js';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const jitter = (ms) => ms * (0.8 + Math.random() * 0.4);

/** Animated GIF as decoded frames with their own delays (ImageDecoder). */
class FrameAnim {
  constructor(frames) {
    this.frames = frames;
    this.total = frames.reduce((a, f) => a + f.dur, 0) || 1;
    this.t0 = performance.now();
  }
  frameAt(now) {
    let t = (now - this.t0) % this.total;
    let start = now - t;
    for (const f of this.frames) {
      if (t < f.dur) return { src: f.bmp, until: start + f.dur };
      t -= f.dur;
      start += f.dur;
    }
    return { src: this.frames[0].bmp, until: now + 50 };
  }
}
/** Fallback: an off-screen <img> animates the GIF itself; the canvas redraws it ~20 times a second. */
class ImgAnim {
  constructor(img) { this.img = img; }
  frameAt(now) { return { src: this.img, until: now + 50 }; }
}
const isGifBlob = async (b) => {
  const h = new Uint8Array(await b.slice(0, 6).arrayBuffer());
  return h.length >= 6 && String.fromCharCode(...h.slice(0, 4)) === 'GIF8';
};
/** The animation of a GIF blob, or null when it is not an animated GIF. */
async function makeAnim(blob) {
  if (!(await isGifBlob(blob))) return null;
  if (typeof ImageDecoder !== 'undefined') {
    try {
      const dec = new ImageDecoder({ data: await blob.arrayBuffer(), type: 'image/gif' });
      await dec.tracks.ready;
      const n = dec.tracks.selectedTrack.frameCount;
      if (n <= 1) { dec.close(); return null; }
      const frames = [];
      let px = 0;
      for (let i = 0; i < n; i++) {
        const { image } = await dec.decode({ frameIndex: i });
        px += image.displayWidth * image.displayHeight * 4;
        if (px > 256e6) { image.close(); throw new Error('too big for frame cache'); }
        const bmp = await createImageBitmap(image);
        frames.push({ bmp, dur: Math.max(20, (image.duration || 100000) / 1000) });
        image.close();
      }
      dec.close();
      return new FrameAnim(frames);
    } catch { /* fall through to the <img> approach */ }
  }
  const img = new Image();
  img.decoding = 'async';
  img.src = URL.createObjectURL(blob);
  document.getElementById('animhold')?.append(img);
  await img.decode();
  return new ImgAnim(img);
}

export class Images {
  constructor() {
    this.auth = async () => ({}); // extra request headers (owner token / edit grant), set by the app
    this.map = new Map(); // hash -> {state: loading|ok|err, img}
    this.up = new Map(); // hash -> {state: uploading|retrying|ok|failed, msg}
    this.onChange = () => {};
    this.running = 0;
    this.waiting = [];
  }
  get(hash) { return this.map.get(hash); }
  set(hash, bmp, blob) {
    const entry = { state: 'ok', img: bmp, anim: null };
    this.map.set(hash, entry);
    if (blob) this.animate(entry, blob);
  }
  animate(entry, blob) {
    makeAnim(blob).then((a) => { if (a) { entry.anim = a; this.onChange(); } }).catch(() => {});
  }
  /** Upload state shown on the element ({state, msg}), or undefined when nothing is pending or wrong. */
  status(hash) {
    const u = this.up.get(hash);
    return u && u.state !== 'ok' ? u : undefined;
  }
  async blob(hash) {
    let b = await idb.get('images', hash).catch(() => null);
    if (b) return b;
    const r = await fetch(`/api/img/${hash}`);
    if (!r.ok) throw new Error('image not found');
    b = await r.blob();
    idb.put('images', hash, b).catch(() => {}); // keep a local copy (best effort)
    return b;
  }
  /** Decode an image for drawing. A missing image is retried: its author may still be uploading it. */
  load(hash, done) {
    if (this.map.has(hash)) return;
    const entry = { state: 'loading', img: null, tries: 0 };
    this.map.set(hash, entry);
    (async () => {
      for (let i = 0; ; i++) {
        try {
          const b = await this.blob(hash);
          entry.img = await createImageBitmap(b);
          entry.state = 'ok';
          done();
          this.animate(entry, b);
          return;
        } catch {
          entry.tries = i + 1;
          if (i >= 12) break;
          await sleep(jitter(Math.min(500 * 2 ** i, 8000)));
        }
      }
      entry.state = 'err';
      done();
      // look again later: once the entry is dropped the next draw starts a fresh attempt
      setTimeout(() => { if (this.map.get(hash) === entry) { this.map.delete(hash); done(); } }, 20000);
    })();
  }
  async slot() {
    if (this.running >= 3) await new Promise((r) => this.waiting.push(r));
    this.running++;
  }
  release() {
    this.running--;
    const n = this.waiting.shift();
    if (n) n();
  }
  /** Make sure the server has the bytes of `hash` for this document (idempotent; retries in the background). */
  upload(hash, url) {
    const cur = this.up.get(hash);
    if (cur && cur.state !== 'failed') return;
    const u = { state: 'uploading', msg: '', tries: 0, missing: 0 };
    this.up.set(hash, u);
    this.onChange();
    (async () => {
      for (;;) {
        await this.slot();
        let permanent = '';
        try {
          let blob = null;
          try { blob = await this.blob(hash); } catch { blob = null; }
          if (!blob) {
            u.missing++;
            if (u.missing >= 8) permanent = 'The image data is not available';
          } else {
            const r = await fetch(url, { method: 'POST', body: blob, headers: await this.auth() });
            if (r.ok) { u.state = 'ok'; this.release(); this.onChange(); return; }
            const j = await r.json().catch(() => ({}));
            if (r.status !== 429 && r.status !== 408 && r.status < 500) permanent = j.error || `Upload failed (${r.status})`;
          }
        } catch { /* network error: retry */ }
        this.release();
        if (permanent) { u.state = 'failed'; u.msg = permanent; this.onChange(); return; }
        u.tries++;
        if (u.tries >= 2 && u.state !== 'retrying') { u.state = 'retrying'; this.onChange(); }
        await sleep(jitter(Math.min(1000 * 2 ** Math.min(u.tries, 5), 30000)));
      }
    })();
  }
}

async function sha256(blob) {
  const buf = await crypto.subtle.digest('SHA-256', await blob.arrayBuffer());
  return [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function toBlob(c, type, q) {
  return new Promise((r) => c.toBlob((b) => r(b), type, q));
}

const OK_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'];
export const MAX_EDGE = 2000;
const MAX_BYTES = 3.9 * 1024 * 1024;

/** Downscale (max edge ~2000px) and re-encode to WebP/JPEG; shrinks further until it fits the server cap. */
export async function prepareImage(file) {
  const bmp = await createImageBitmap(file);
  const w = bmp.width, h = bmp.height;
  const gif = file.type === 'image/gif' || (await isGifBlob(file));
  if (gif && file.size <= MAX_BYTES) {
    // keep the original bytes: re-encoding would drop the animation
    return { blob: file, bmp, w, h, hash: await sha256(file), animated: true };
  }
  let note = gif ? 'This GIF is too large to keep animated: it was added as a still image' : '';
  let scale = Math.min(1, MAX_EDGE / Math.max(w, h));
  let blob = file;
  const type = OK_TYPES.includes(file.type) ? file.type : '';
  const encode = async (s, q) => {
    const c = document.createElement('canvas');
    c.width = Math.max(1, Math.round(w * s));
    c.height = Math.max(1, Math.round(h * s));
    c.getContext('2d').drawImage(bmp, 0, 0, c.width, c.height);
    let out = await toBlob(c, 'image/webp', q);
    if (!out || out.type !== 'image/webp') out = await toBlob(c, 'image/jpeg', q);
    return out;
  };
  if (scale < 1 || !type || file.size > 600 * 1024) {
    const out = await encode(scale, 0.85);
    if (out && (scale < 1 || !type || out.size < file.size)) blob = out;
  }
  let q = 0.8;
  for (let i = 0; blob.size > MAX_BYTES && i < 12; i++) {
    scale *= 0.8;
    const out = await encode(scale, q);
    if (!out) break;
    blob = out;
    q = Math.max(0.4, q - 0.1);
  }
  if (blob.size > MAX_BYTES) throw new Error('Image is too large even after shrinking');
  const final = blob === file ? bmp : await createImageBitmap(blob);
  const hash = await sha256(blob);
  return { blob, bmp: final, w: final.width, h: final.height, hash, note };
}
