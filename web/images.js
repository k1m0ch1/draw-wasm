// Image cache (hash -> decoded bitmap). Looks in IndexedDB first, then the server.
import { idb } from './db.js';

export class Images {
  constructor() { this.map = new Map(); }
  get(hash) { return this.map.get(hash); }
  set(hash, bmp) { this.map.set(hash, { state: 'ok', img: bmp }); }
  async blob(hash) {
    let b = await idb.get('images', hash).catch(() => null);
    if (b) return b;
    const r = await fetch(`/api/img/${hash}`);
    if (!r.ok) throw new Error('image not found');
    b = await r.blob();
    return b;
  }
  load(hash, done) {
    if (this.map.has(hash)) return;
    const entry = { state: 'loading', img: null };
    this.map.set(hash, entry);
    (async () => {
      try {
        const b = await this.blob(hash);
        entry.img = await createImageBitmap(b);
        entry.state = 'ok';
        done();
      } catch {
        entry.state = 'err';
        setTimeout(() => { if (this.map.get(hash) === entry) this.map.delete(hash); }, 4000);
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

/** Downscale (max edge ~2000px) and re-encode to WebP/JPEG when that is smaller. */
export async function prepareImage(file) {
  const bmp = await createImageBitmap(file);
  let { width: w, height: h } = bmp;
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
  while (blob.size > MAX_BYTES && q > 0.25) {
    scale *= 0.8;
    const out = await encode(scale, q);
    if (!out) break;
    blob = out;
    q -= 0.1;
  }
  if (blob.size > MAX_BYTES) throw new Error('Image is too large');
  const final = blob === file ? bmp : await createImageBitmap(blob);
  const hash = await sha256(blob);
  return { blob, bmp: final, w: final.width, h: final.height, hash };
}
