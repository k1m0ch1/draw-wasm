// Shared helpers for the browser tests (Playwright + Chrome). Rooms start with zz-test- so they can be purged.
const { chromium } = require(process.env.PLAYWRIGHT || 'playwright');
const zlib = require('zlib');
const BASE = process.env.BASE;
const SHOTS = process.env.SHOTS || '.';
const state = { fails: 0 };
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) state.fails++; return !!c; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rnd = () => Math.random().toString(36).slice(2, 8);

function makePng(w, h, seed) {
  const crcT = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b) => { let c = 0xffffffff; for (const x of b) c = crcT[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([l, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  const raw = Buffer.alloc((w * 3 + 1) * h);
  let s = seed;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const o = y * (w * 3 + 1) + 1 + x * 3; s = (s * 1103515245 + 12345) >>> 0; raw[o] = (s >>> 8) & 255; raw[o + 1] = (s >>> 16) & 255; raw[o + 2] = (s >>> 24) & 255; }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

/** A real animated GIF: `colours` is a list of palette indexes 0..3 (red, blue, green, yellow), one frame each. */
function makeGif(size, colours, delayCs = 10) {
  const pal = [[255, 0, 0], [0, 0, 255], [0, 200, 0], [255, 230, 0]];
  const out = [Buffer.from('GIF89a'), Buffer.from([size & 255, size >> 8, size & 255, size >> 8, 0xf1, 0, 0])];
  out.push(Buffer.from(pal.flat()));
  out.push(Buffer.from([0x21, 0xff, 0x0b, ...Buffer.from('NETSCAPE2.0'), 3, 1, 0, 0, 0])); // loop forever
  for (const c of colours) {
    out.push(Buffer.from([0x21, 0xf9, 4, 0, delayCs & 255, delayCs >> 8, 0, 0]));
    out.push(Buffer.from([0x2c, 0, 0, 0, 0, size & 255, size >> 8, size & 255, size >> 8, 0]));
    // LZW with min code size 2 (3-bit codes); a clear code every 2 literals keeps the width fixed
    const codes = [];
    let n = 0;
    codes.push(4);
    for (let i = 0; i < size * size; i++) { codes.push(c); if (++n % 2 === 0) codes.push(4); }
    codes.push(5);
    const bytes = []; let acc = 0, bits = 0;
    for (const k of codes) { acc |= k << bits; bits += 3; while (bits >= 8) { bytes.push(acc & 255); acc >>= 8; bits -= 8; } }
    if (bits > 0) bytes.push(acc & 255);
    out.push(Buffer.from([2]));
    for (let i = 0; i < bytes.length; i += 255) { const part = bytes.slice(i, i + 255); out.push(Buffer.from([part.length, ...part])); }
    out.push(Buffer.from([0]));
  }
  out.push(Buffer.from([0x3b]));
  return Buffer.concat(out);
}

async function launch() { return chromium.launch({ headless: true, channel: 'chrome' }); }

/**
 * Open a page. `name`: undefined = do not touch a name dialog, '' = press Enter with an empty name,
 * 'x' = type that name. Resolves to [ctx, page].
 */
async function open(browser, url, name, opts = {}) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, ...opts });
  await ctx.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: BASE });
  const p = await ctx.newPage();
  p.on('pageerror', (e) => console.log('PAGEERROR', e.message));
  await p.goto(url);
  await p.waitForFunction(() => window.__draw && window.__draw.board);
  if (name !== undefined) {
    await p.waitForSelector('#name-input', { timeout: 10000 });
    if (name) await p.fill('#name-input', name);
    if (name) await p.click('#name-go'); else await p.press('#name-input', 'Enter');
  }
  return [ctx, p];
}

const summary = (p) => p.evaluate(() => JSON.parse(window.__draw.board.summary()));
const live = (p) => summary(p).then((s) => s);
/** Content signature of the live document (no versions / writers), for cross-client and reload comparison. */
const sig = (p) => p.evaluate(() => JSON.stringify(JSON.parse(window.__draw.board.export_live()).map(({ v, by, ...e }) => e).sort((a, b) => (a.id < b.id ? -1 : 1))));
async function converge(pages, ms = 8000) {
  const t0 = Date.now();
  for (;;) {
    const s = await Promise.all(pages.map(sig));
    if (s.every((x) => x === s[0])) return true;
    if (Date.now() - t0 > ms) return false;
    await sleep(150);
  }
}
async function drag(p, a, b, steps = 8) {
  await p.mouse.move(a[0], a[1]);
  await p.mouse.down();
  for (let i = 1; i <= steps; i++) await p.mouse.move(a[0] + ((b[0] - a[0]) * i) / steps, a[1] + ((b[1] - a[1]) * i) / steps);
  await p.mouse.up();
}
const tool = (p, t) => p.click(`#toolbar [data-tool="${t}"]`);
const scr = (p, id) => p.evaluate((i) => window.__draw.board.element_screen(i), id); // [x,y,w,h]
const els = (p) => p.evaluate(() => JSON.parse(window.__draw.board.export_live()));
const setClip = (p, buf, type = 'image/png') => p.evaluate(async ([b64, t]) => {
  const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  await navigator.clipboard.write([new ClipboardItem({ [t]: new Blob([bin], { type: t }) })]);
}, [buf.toString('base64'), type]);
const idsOf = (a) => new Set(a.map((e) => e.id));
/** Run `fn`, return the live elements that did not exist before. */
async function created(p, fn) {
  const before = idsOf(await els(p));
  await fn();
  await sleep(120);
  return (await els(p)).filter((e) => !before.has(e.id));
}
async function createCloudRoom(browser, name, room) {
  const [ctx, p] = await open(browser, BASE + '/');
  await tool(p, 'rect');
  await drag(p, [300, 200], [400, 260]);
  await p.click('#btn-cloud');
  if (name) await p.fill('#cloud-name', name);
  await p.fill('#cloud-room', room);
  await p.click('#cloud-go');
  await p.waitForURL(new RegExp('/' + room + '/[A-Za-z0-9_-]{10}$'), { timeout: 20000 });
  await p.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 15000 });
  return [ctx, p, p.url()];
}
const FACE = /^[\u{1F600}-\u{1F637}\u{1F641}-\u{1F644}\u{1F910}-\u{1F917}\u{1F920}-\u{1F925}\u{1F927}-\u{1F92F}\u{1F970}-\u{1F976}\u{1F978}-\u{1F97A}]$/u;

module.exports = { chromium, BASE, SHOTS, state, ok, sleep, rnd, makePng, makeGif, launch, open, summary, live, sig, converge, drag, tool, scr, els, setClip, created, createCloudRoom, FACE, idsOf };
