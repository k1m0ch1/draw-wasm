// PLAN item 52: client memory. Works against old and new builds (new ones also report __draw.memory()).
//   BASE=https://example.com [IDLE=60] node tests/memory.js
// Everything happens in a LOCAL document: nothing is written to the server.
const L = require('./lib');
const { ok, sleep, makeGif, BASE } = L;
const IDLE = Number(process.env.IDLE || 60) * 1000;
const LIMIT = 100 * 1024 * 1024;
const mb = (n) => (n / 1048576).toFixed(1) + ' MB';

async function metrics(p, cdp) {
  const m = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map((x) => [x.name, x.value]));
  const mem = await p.evaluate(() => {
    const D = window.__draw;
    if (D.memory) return D.memory();
    let px = 0;
    for (const e of D.images.map.values()) if (e.img) px += e.img.width * e.img.height;
    return { wasm: 0, heap: 0, bitmapBytes: px * 4, bitmaps: D.images.map.size, gifBytes: 0, total: 0, old: true };
  });
  return { jsHeap: m.JSHeapUsedSize, ...mem };
}

/** Add generated images through the file chooser path, each at its own place in the world. */
async function addImages(p, n, w, h, startIdx = 0) {
  await p.evaluate(async ([n, w, h, startIdx]) => {
    for (let i = 0; i < n; i++) {
      const c = document.createElement('canvas'); c.width = w; c.height = h;
      const x = c.getContext('2d');
      const g = x.createLinearGradient(0, 0, w, h);
      g.addColorStop(0, `hsl(${((startIdx + i) * 37) % 360},70%,55%)`); g.addColorStop(1, `hsl(${((startIdx + i) * 91) % 360},70%,35%)`);
      x.fillStyle = g; x.fillRect(0, 0, w, h);
      for (let k = 0; k < 400; k++) { x.fillStyle = `hsla(${(k * 53) % 360},80%,60%,0.6)`; x.fillRect((k * 97) % w, (k * 61) % h, 40, 40); }
      const blob = await new Promise((r) => c.toBlob(r, 'image/png'));
      const f = new File([blob], `t${startIdx + i}.png`, { type: 'image/png' });
      const dt = new DataTransfer(); dt.items.add(f);
      const inp = document.getElementById('filepick');
      window.__draw.mouse = [200 + ((startIdx + i) % 8) * 520, 150 + Math.floor((startIdx + i) / 8) * 420];
      inp.files = dt.files;
      inp.dispatchEvent(new Event('change', { bubbles: true }));
      await new Promise((r) => setTimeout(r, 150));
    }
  }, [n, w, h, startIdx]);
}

(async () => {
  const browser = await L.launch();
  // ---------------- representative document: 60 elements, 2 images, 1 animated GIF
  {
    const [ctx, p] = await L.open(browser, BASE + '/');
    const cdp = await ctx.newCDPSession(p);
    await cdp.send('Performance.enable');
    const before = await metrics(p, cdp);
    // 60 shapes through the real engine
    await p.evaluate(() => { const b = window.__draw.board; for (let i = 0; i < 60; i++) { const k = ['rect', 'ellipse', 'diamond'][i % 3]; b.set_tool(k); const x = 100 + (i % 10) * 110, y = 100 + Math.floor(i / 10) * 90; b.pointer_down(x, y, false); b.pointer_move(x + 80, y + 60); b.pointer_up(x + 80, y + 60); } });
    const n = await p.evaluate(() => window.__draw.board.live_count());
    ok(n >= 50, `representative doc has ${n} elements`);
    await addImages(p, 2, 1600, 1000, 100);
    const gif = makeGif(120, Array.from({ length: 40 }, (_, i) => i % 4), 8);
    await p.evaluate(async (b64) => {
      const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const dt = new DataTransfer(); dt.items.add(new File([bin], 'a.gif', { type: 'image/gif' }));
      const inp = document.getElementById('filepick'); window.__draw.mouse = [300, 600]; inp.files = dt.files; inp.dispatchEvent(new Event('change', { bubbles: true }));
    }, gif.toString('base64'));
    await sleep(3000);
    console.log(`gif bytes ${gif.length}`);
    await sleep(IDLE);
    const m = await metrics(p, cdp);
    console.log('representative after idle', JSON.stringify(m));
    console.log(`representative: before ${mb(before.jsHeap + before.wasm)}  after ${mb(m.jsHeap + m.wasm)} (JS heap ${mb(m.jsHeap)}, wasm ${mb(m.wasm)}, decoded bitmaps ${mb(m.bitmapBytes)}, gif frames ${mb(m.gifBytes)})`);
    if (!m.old) ok(m.jsHeap + m.wasm < LIMIT, `representative doc: JS heap + wasm = ${mb(m.jsHeap + m.wasm)} < 100 MB`);
    else console.log('(old build: no wasm figure)');
    ok(m.jsHeap < LIMIT, `representative doc: JS heap ${mb(m.jsHeap)} < 100 MB`);
    await ctx.close();
  }
  // ---------------- stress: 40 images, scroll through and come back
  {
    const [ctx, p] = await L.open(browser, BASE + '/');
    const cdp = await ctx.newCDPSession(p);
    await cdp.send('Performance.enable');
    for (let i = 0; i < 40; i += 8) await addImages(p, 8, 1600, 1000, i);
    await sleep(2000);
    await p.evaluate(() => { const b = window.__draw.board; b.set_zoom(0, 0, 1); });
    let peak = 0, peakBitmaps = 0;
    const sample = async (tag) => { const m = await metrics(p, cdp); peak = Math.max(peak, m.jsHeap + (m.wasm || 0), 0); peakBitmaps = Math.max(peakBitmaps, m.bitmapBytes); return m; };
    // scroll right/down through the grid, then back
    const pan = (dx, dy) => p.evaluate(([dx, dy]) => window.__draw.board.pan(dx, dy), [dx, dy]);
    for (let r = 0; r < 2; r++) { for (let i = 0; i < 8; i++) { await pan(-500, 0); await sleep(250); await sample(); } await pan(4000, -420); await sleep(250); await sample(); }
    for (let i = 0; i < 6; i++) { await pan(0, 420); await sleep(250); await sample(); }
    await pan(0, 0);
    await sleep(3000);
    const m = await sample();
    console.log('stress end', JSON.stringify(m), 'peak (heap+wasm)', mb(peak), 'peak decoded bitmaps', mb(peakBitmaps));
    ok(m.jsHeap + (m.wasm || 0) < LIMIT, `stress (40 images): JS heap + wasm ${mb(m.jsHeap + (m.wasm || 0))} < 100 MB`);
    if (!m.old) ok(peakBitmaps <= 16e6 * 4 * 1.6, `stress: decoded bitmaps stay near the 64 MB pixel budget (peak ${mb(peakBitmaps)})`);
    else console.log(`(old build) decoded bitmaps now ${mb(m.bitmapBytes)}, peak ${mb(peakBitmaps)}`);
    await ctx.close();
  }
  await browser.close();
  process.exit(L.state.fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
