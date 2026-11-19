// PLAN items 49-51 in a real browser: queue UI, in-place promotion, client-side image/GIF caps.
//   BASE=https://example.com node tests/capacity-ui.js
const L = require('./lib');
const { ok, sleep, makeGif, makePng, BASE } = L;
const ROOM = 'zz-test-capui' + L.rnd();
const fill = (n) => Array.from({ length: n }, (_, i) => {
  const ws = new WebSocket(BASE.replace(/^http/, 'ws') + `/ws/${ROOM}/${fill.nano}`);
  ws.onopen = () => ws.send(JSON.stringify({ t: 'hello', cid: 'cfill' + i + L.rnd(), name: 'F' + i }));
  return ws;
});
const chip = (p) => p.evaluate(() => document.getElementById('rolechip').textContent);
const fileTo = (p, buf, name, type) => p.evaluate(async ([b64, name, type]) => {
  const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const dt = new DataTransfer(); dt.items.add(new File([bin], name, { type }));
  const inp = document.getElementById('filepick'); inp.files = dt.files; inp.dispatchEvent(new Event('change', { bubbles: true }));
}, [buf.toString('base64'), name, type]);
(async () => {
  const browser = await L.launch();
  const [ctxO, O, url] = await L.createCloudRoom(browser, 'Owner', ROOM);
  fill.nano = url.split('/').pop();
  ok(/Editing/.test(await chip(O)), `owner sees "${await chip(O)}"`);
  const socks = fill(15);
  await sleep(2500);
  const [ctxV, V] = await L.open(browser, url, 'Viewer');
  await V.waitForFunction(() => window.__draw.wsOpen && window.__draw.role, null, { timeout: 15000 });
  await sleep(500);
  ok(/^Viewing - queue position 1$/.test(await chip(V)), `viewer sees "${await chip(V)}"`);
  const n0 = await L.els(V);
  await V.evaluate(() => { const b = window.__draw.board; b.set_tool('rect'); b.pointer_down(300, 300, false); b.pointer_move(380, 360); b.pointer_up(380, 360); });
  await sleep(1500);
  ok((await L.els(O)).length === n0.length, 'a viewer drawing locally changes nothing for the others');
  ok(await V.evaluate(() => window.__draw.edit === false), 'viewer client is read-only');
  socks[0].close();
  await V.waitForFunction(() => window.__draw.edit === true, null, { timeout: 15000 });
  ok(/^Editing \(\d+\/15\)$/.test(await chip(V)), `promoted in place, chip says "${await chip(V)}"`);
  const toast = await V.evaluate(() => document.getElementById('toast').textContent);
  ok(/Editing/.test(toast), `toast says "${toast}"`);
  ok(await V.evaluate(() => location.href) === url, 'no reload happened');
  // client-side caps
  await fileTo(V, Buffer.concat([makeGif(60, [0, 1]), Buffer.alloc(3.2 * 1024 * 1024)]), 'big.gif', 'image/gif');
  await sleep(1500);
  const t2 = await V.evaluate(() => document.getElementById('toast').textContent);
  ok(/GIF/.test(t2) && /3 MB/.test(t2), `oversized GIF refused with a message: "${t2}"`);
  const before = (await L.els(V)).length;
  await sleep(300);
  ok((await L.els(V)).length === before, 'no still image was added for the refused GIF');
  const bigSize = await V.evaluate(async () => { // incompressible noise: a PNG of several MB
    const c = document.createElement('canvas'); c.width = 1600; c.height = 1200;
    const x = c.getContext('2d'); const d = x.createImageData(1600, 1200); crypto.getRandomValues(d.data.subarray(0, 65536));
    for (let i = 0; i < d.data.length; i += 65536) crypto.getRandomValues(d.data.subarray(i, Math.min(i + 65536, d.data.length)));
    for (let i = 3; i < d.data.length; i += 4) d.data[i] = 255;
    x.putImageData(d, 0, 0);
    const blob = await new Promise((r) => c.toBlob(r, 'image/png'));
    const inp = document.getElementById('filepick'); const dt = new DataTransfer(); dt.items.add(new File([blob], 'photo.png', { type: 'image/png' }));
    inp.files = dt.files; inp.dispatchEvent(new Event('change', { bubbles: true }));
    return blob.size;
  });
  ok(bigSize > 2 * 1024 * 1024, `test image is ${(bigSize / 1048576).toFixed(1)} MB`);
  await V.waitForFunction(() => /downscaled|Could not/.test(document.getElementById('toast').textContent) && !/GIF/.test(document.getElementById('toast').textContent), null, { timeout: 30000 }).catch(() => {});
  await sleep(500);
  ok((await L.els(V)).filter((e) => e.k === 'image').length === 1, 'the big image was added as an element');
  const t3 = await V.evaluate(() => document.getElementById('toast').textContent);
  ok(/downscaled to fit 2 MB/.test(t3), `big image downscaled with a note: "${t3}"`);
  await sleep(4000);
  const st = await V.evaluate(() => window.__draw.images.up.size && [...window.__draw.images.up.values()].map((u) => u.state));
  ok(st && st.every((s) => s === 'ok'), `downscaled image uploaded (${st})`);
  socks.forEach((s) => { try { s.close(); } catch {} });
  await fetch(BASE + `/api/rooms/${ROOM}`, { method: 'DELETE', headers: { 'x-owner-token': await O.evaluate(() => window.__draw.token) } }).catch(() => {});
  await browser.close();
  process.exit(L.state.fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
