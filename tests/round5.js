// Space-pan, image paste, paste placement, deploy handshake and offline edits, with REAL keyboard/clipboard input.
// BASE=https://example.com SHOTS=dir node tests/round5.js
const { chromium } = require(process.env.PLAYWRIGHT || 'playwright');
const zlib = require('zlib');
const BASE = process.env.BASE;
const SHOTS = process.env.SHOTS || '.';
let fails = 0;
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sum = (p) => p.evaluate(() => JSON.parse(window.__draw.board.summary()));
const ROOM = 'zz-test-' + Math.random().toString(36).slice(2, 8);
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
async function ctxOpen(browser, url, name) {
  const ctx = await browser.newContext({ viewport: { width: 1200, height: 760 } });
  await ctx.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: BASE });
  const p = await ctx.newPage();
  p.on('pageerror', (e) => console.log('PAGEERROR', e.message));
  await p.goto(url);
  await p.waitForFunction(() => window.__draw && window.__draw.board);
  if (name) { await p.waitForSelector('#name-input', { timeout: 10000 }); await p.fill('#name-input', name); await p.click('#name-go'); }
  return [ctx, p];
}
const setClip = (p, buf) => p.evaluate(async (b64) => {
  const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  await navigator.clipboard.write([new ClipboardItem({ 'image/png': new Blob([bin], { type: 'image/png' }) })]);
}, buf.toString('base64'));
const view = (p) => p.evaluate(() => [window.__draw.board.pan_x(), window.__draw.board.pan_y()]);
const state = (p) => p.evaluate(() => ({ z: window.__draw.board.zoom(), t: window.__draw.board.tool(), n: window.__draw.board.live_count() }));
const closeModal = (p) => p.evaluate(() => { const m = document.getElementById('modal'); m.classList.add('hidden'); m.replaceChildren(); });
async function spaceDrag(p) {
  await p.mouse.move(600, 400);
  const v0 = await view(p);
  await p.keyboard.down('Space');
  await p.mouse.down();
  for (let i = 1; i <= 8; i++) await p.mouse.move(600 + i * 10, 400 + i * 5);
  await p.mouse.up();
  await p.keyboard.up('Space');
  const v1 = await view(p);
  return [v1[0] - v0[0], v1[1] - v0[1]];
}
const allUploaded = () => JSON.parse(window.__draw.board.images_used()).every((h) => !window.__draw.images.status(h));
(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'chrome' });
  const [ctxA, A] = await ctxOpen(browser, BASE + '/');
  await A.click('#toolbar [data-tool="rect"]');
  await A.mouse.move(300, 200); await A.mouse.down(); await A.mouse.move(400, 260); await A.mouse.up();
  await A.click('#btn-cloud'); await A.fill('#cloud-name', 'Olive'); await A.fill('#cloud-room', ROOM); await A.click('#cloud-go');
  await A.waitForURL(new RegExp('/' + ROOM + '/[A-Za-z0-9_-]{10}$'), { timeout: 15000 });
  const url = A.url();
  await A.waitForFunction(() => window.__draw.wsOpen);
  const [ctxB, B] = await ctxOpen(browser, url, 'Bea B');
  await B.waitForFunction(() => window.__draw.wsOpen);

  // ---- U3: space pan for the joiner, with real key events, after clicking each control
  let d = await spaceDrag(B);
  ok(Math.abs(d[0] - 80) < 1 && Math.abs(d[1] - 40) < 1, `fresh page: Space+drag pans (${d})`);
  for (const sel of ['#btn-zin', '#btn-zout', '#zlabel', '#btn-fit', '#btn-undo', '#btn-redo', '#toolbar [data-tool="rect"]', '#toolbar [data-tool="pencil"]', '#toolbar [data-tool="select"]', '#btn-save', '#btn-share', '#btn-name', '.sw']) {
    await B.locator(sel).first().click({ force: true, timeout: 5000 });
    if (sel === '#btn-share' || sel === '#btn-name') await closeModal(B);
    const act = await B.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.tagName));
    const s0 = await state(B);
    d = await spaceDrag(B);
    const s1 = await state(B);
    ok(Math.abs(d[0] - 80) < 1 && s0.z === s1.z && s0.t === s1.t && s0.n === s1.n, `after clicking ${sel} (focus ${act}): pans, no side effect (${d})`);
  }
  await B.click('#btn-zin'); await B.keyboard.down('Space'); await B.mouse.move(600, 400); await B.mouse.down(); await B.mouse.move(640, 420); await B.mouse.up(); await B.keyboard.up('Space');
  const vv = await view(B);
  await B.mouse.move(500, 300); await B.mouse.down(); await B.keyboard.down('Space'); await B.mouse.move(540, 330); await B.mouse.move(560, 340); await B.mouse.up(); await B.keyboard.up('Space');
  const vw = await view(B);
  ok(Math.abs(vw[0] - vv[0] - 60) < 1 && Math.abs(vw[1] - vv[1] - 40) < 1, `Space pressed with the mouse already down pans (${vw[0] - vv[0]},${vw[1] - vv[1]})`);
  await B.evaluate(() => document.getElementById('btn-zin').focus());
  const z0 = await B.evaluate(() => window.__draw.board.zoom());
  await B.keyboard.press('Space');
  ok((await B.evaluate(() => window.__draw.board.zoom())) === z0, 'Space on a focused button does not activate it');
  await B.click('#btn-name'); await B.fill('#name-input', 'Bea'); await B.keyboard.type(' B');
  ok((await B.inputValue('#name-input')) === 'Bea B', 'Space types a space in a text field');
  await closeModal(B);
  const m0 = await view(B);
  await B.mouse.move(600, 400); await B.mouse.down({ button: 'middle' });
  for (let i = 1; i <= 6; i++) await B.mouse.move(600 - i * 10, 400 - i * 10);
  await B.mouse.up({ button: 'middle' });
  const m1 = await view(B);
  ok(Math.abs(m1[0] - m0[0] + 60) < 1 && Math.abs(m1[1] - m0[1] + 60) < 1, `middle-mouse drag pans (${m1[0] - m0[0]},${m1[1] - m0[1]})`);
  const w0 = await view(B); const wz0 = await B.evaluate(() => window.__draw.board.zoom());
  await B.mouse.move(600, 400); await B.mouse.wheel(15, 12); await sleep(30);
  const w1 = await view(B);
  ok(w1[0] < w0[0] && w1[1] < w0[1] && (await B.evaluate(() => window.__draw.board.zoom())) === wz0, `two-finger scroll pans (${w1[0] - w0[0]},${w1[1] - w0[1]})`);
  await sleep(300);
  await B.mouse.wheel(0, -200); await sleep(50);
  ok((await B.evaluate(() => window.__draw.board.zoom())) !== wz0, 'mouse wheel step zooms');

  // ---- U6: old client + build handshake
  const wsUrl = BASE.replace(/^http/, 'ws') + '/ws' + new URL(url).pathname;
  const stale = await B.evaluate((u) => new Promise((res) => {
    const out = []; const ws = new WebSocket(u);
    ws.onopen = () => ws.send(JSON.stringify({ t: 'hello', cid: 'cstale' + Math.random().toString(36).slice(2, 9), name: '', token: '', seq: 0 }));
    ws.onmessage = (e) => out.push(JSON.parse(e.data).t);
    ws.onclose = (e) => res({ out, code: e.code });
    setTimeout(() => { res({ out, code: 'open' }); ws.close(); }, 2500);
  }), wsUrl);
  ok(stale.out.includes('snap') && stale.out.includes('reload') && stale.code === 'open', `old client is admitted and told to reload: ${stale.out.join(',')} close=${stale.code}`);
  const [ctxS, S] = await ctxOpen(browser, url, 'Stan S');
  await S.waitForFunction(() => window.__draw.wsOpen);
  await S.evaluate(() => { window.__draw.build = 'oldbuild00'; window.__draw.ws.close(); });
  await S.waitForFunction(() => !document.getElementById('banner').classList.contains('hidden'), null, { timeout: 8000 });
  ok(/new version/i.test(await S.textContent('#banner')), 'banner: ' + (await S.textContent('#banner')));
  await S.screenshot({ path: SHOTS + '/90-banner.png' });
  await S.waitForFunction(() => window.__draw && window.__draw.board && window.__draw.wsOpen && window.__draw.build !== 'oldbuild00', null, { timeout: 15000 });
  ok(true, 'page auto-reloaded onto the new build and reconnected');
  const [ctxS2, S2] = await ctxOpen(browser, url, 'Stu S');
  await S2.waitForFunction(() => window.__draw.wsOpen);
  await ctxS2.setOffline(true);
  await S2.evaluate(() => { window.__draw.build = 'oldbuild01'; window.__draw.ws.close(); });
  await S2.click('#toolbar [data-tool="ellipse"]');
  await S2.mouse.move(700, 300); await S2.mouse.down(); await S2.mouse.move(780, 360); await S2.mouse.up();
  const n0 = (await sum(S2)).length;
  await ctxS2.setOffline(false);
  await S2.evaluate(() => { window.__draw.wsTry = 0; });
  await S2.evaluate(() => window.dispatchEvent(new Event('online')));
  await S2.waitForFunction(() => /reload/i.test(document.getElementById('banner').textContent) && !document.getElementById('banner').classList.contains('hidden'), null, { timeout: 15000 });
  await sleep(2500);
  ok(S2.url() === url && (await S2.evaluate(() => window.__draw.build)) === 'oldbuild01', 'no auto reload while unsent edits exist; banner offers a reload');
  await S2.click('#banner-action');
  await S2.waitForFunction(() => window.__draw && window.__draw.board && window.__draw.wsOpen && window.__draw.build !== 'oldbuild01', null, { timeout: 15000 });
  await sleep(1500);
  ok((await sum(S2)).length === n0 && (await sum(A)).length === n0, `unsent edit kept across the reload and delivered (${n0} elements on both)`);

  // ---- offline: backoff, message, edits kept and sent after reconnect
  await ctxB.setOffline(true);
  await B.evaluate(() => window.__draw.ws.close());
  await B.click('#toolbar [data-tool="diamond"]');
  await B.mouse.move(800, 500); await B.mouse.down(); await B.mouse.move(880, 580); await B.mouse.up();
  await sleep(7000);
  const off = await B.evaluate(() => ({ banner: document.getElementById('banner').textContent, status: document.getElementById('status').textContent, tries: window.__draw.wsTry }));
  ok(/not being shared/i.test(off.banner) && /Offline/.test(off.status), `offline UI: "${off.banner}" / "${off.status}" (tries ${off.tries})`);
  await B.screenshot({ path: SHOTS + '/91-offline.png' });
  await ctxB.setOffline(false);
  await B.click('#banner-action');
  await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 20000 });
  await sleep(1200);
  ok((await sum(A)).some((e) => e.k === 'Diamond'), 'edit made while offline reached the other participant after reconnect');
  ok(await B.evaluate(() => document.getElementById('banner').classList.contains('hidden')), 'banner hidden after reconnect');

  // ---- U4 / U5: OS clipboard image as a joiner
  await B.click('#toolbar [data-tool="select"]');
  await B.evaluate(() => window.__draw.board.set_zoom(0, 0, 1));
  await setClip(B, makePng(300, 200, 1));
  await B.mouse.move(400, 300);
  await B.keyboard.press('Control+v'); await sleep(600);
  let imgs = (await sum(B)).filter((e) => e.k === 'Image');
  ok(imgs.length === 1, 'OS-clipboard image appears at once for a joiner');
  const sc0 = await B.evaluate(() => window.__draw.board.element_screen(JSON.parse(window.__draw.board.summary()).find((x) => x.k === 'Image').id));
  console.log('element_screen', JSON.stringify(sc0));
  await B.screenshot({ path: SHOTS + '/92a-paste-first.png' });
  await B.keyboard.press('Control+v'); await sleep(500); await B.keyboard.press('Control+v'); await sleep(800);
  imgs = (await sum(B)).filter((e) => e.k === 'Image');
  const pos = imgs.map((e) => `${Math.round(e.x)},${Math.round(e.y)}`);
  ok(imgs.length === 3 && new Set(pos).size === 3, `3 pastes at 3 distinct positions: ${pos.join(' | ')}`);
  const selNow = await B.evaluate(() => JSON.parse(window.__draw.board.selection()));
  ok(selNow.length === 1, 'the newest paste is selected');
  const sel0 = (await sum(B)).find((e) => e.id === selNow[0]);
  const cen = await B.evaluate((id) => window.__draw.board.element_screen(id), selNow[0]);
  const cx = cen[0] + cen[2] / 2, cy = cen[1] + cen[3] / 2;
  await B.mouse.move(cx, cy); await B.mouse.down();
  for (let i = 1; i <= 8; i++) await B.mouse.move(cx + i * 15, cy + i * 10);
  await B.mouse.up(); await sleep(800);
  const sumB = await sum(B), sumA = await sum(A);
  const mB = sumB.find((e) => e.id === sel0.id), mA = sumA.find((e) => e.id === sel0.id);
  ok(Math.abs(mB.x - sel0.x - 120) < 2 && Math.abs(mB.y - sel0.y - 80) < 2, `dragging moved exactly the selection (${(mB.x - sel0.x).toFixed(0)},${(mB.y - sel0.y).toFixed(0)})`);
  ok(mA && mA.x === mB.x && mA.y === mB.y, 'peer sees the image at the new position');
  ok(sumB.filter((e) => e.k === 'Image' && e.id !== sel0.id).every((e) => `${e.x},${e.y}` !== `${mB.x},${mB.y}`), 'no copy left underneath');
  await B.waitForFunction(allUploaded, null, { timeout: 30000 });
  const hash = await B.evaluate(() => JSON.parse(window.__draw.board.images_used())[0]);
  ok((await B.evaluate((h) => fetch('/api/img/' + h).then((r) => r.status), hash)) === 200, 'pasted image bytes are on the server');
  ok(await A.evaluate((h) => new Promise((res) => { const t0 = Date.now(); const f = () => { window.__draw.board.invalidate(); const e = window.__draw.images.get(h); if (e && e.state === 'ok') res(true); else if (Date.now() - t0 > 15000) res(false); else setTimeout(f, 200); }; f(); }), hash), "the owner decoded the joiner's image");
  await B.screenshot({ path: SHOTS + '/92-paste-B.png' });
  await A.screenshot({ path: SHOTS + '/93-paste-A.png' });

  // copy/paste of an image element: same doc, then another doc in a fresh browser
  const cnt0 = (await sum(B)).filter((e) => e.k === 'Image').length;
  await B.keyboard.press('Control+c'); await sleep(200);
  const clipTxt = await B.evaluate(() => navigator.clipboard.readText());
  ok(clipTxt.startsWith('draw-wasm:clip:v1:'), 'Ctrl+C put the element on the system clipboard');
  await B.mouse.move(900, 200); await B.keyboard.press('Control+v'); await sleep(500);
  ok((await sum(B)).filter((e) => e.k === 'Image').length === cnt0 + 1, 'copied image element pastes in the same doc');
  const tok = await A.evaluate((room) => new Promise((res) => { const rq = indexedDB.open('draw-wasm'); rq.onsuccess = () => { const g = rq.result.transaction('meta').objectStore('meta').get('tok:' + room); g.onsuccess = () => res(g.result); g.onerror = () => res(null); }; rq.onerror = () => res(null); }), ROOM);
  const d2 = await A.evaluate(async ({ room, tok }) => (await fetch(`/api/rooms/${room}/docs`, { method: 'POST', headers: { 'content-type': 'application/json', 'x-owner-token': tok }, body: JSON.stringify({ title: 'second', els: [] }) })).json(), { room: ROOM, tok });
  ok(d2.nano && d2.nano.length === 10, 'second doc created: ' + d2.nano);
  const [ctxE, E] = await ctxOpen(browser, `${BASE}/${ROOM}/${d2.nano}`, 'Eve E');
  await E.waitForFunction(() => window.__draw.wsOpen);
  await E.evaluate((t) => navigator.clipboard.writeText(t), clipTxt);
  await E.mouse.move(500, 350); await E.keyboard.press('Control+v');
  await E.waitForFunction(() => JSON.parse(window.__draw.board.summary()).some((e) => e.k === 'Image'), null, { timeout: 5000 });
  const h2 = await E.evaluate(() => JSON.parse(window.__draw.board.images_used())[0]);
  ok(await E.evaluate((h) => new Promise((res) => { const t0 = Date.now(); const f = () => { window.__draw.board.invalidate(); const e = window.__draw.images.get(h); if (e && e.state === 'ok') res(true); else if (Date.now() - t0 > 20000) res(false); else setTimeout(f, 250); }; f(); }), h2), 'image copied from another doc renders in a fresh browser (bytes only on the server)');
  await E.waitForFunction(allUploaded, null, { timeout: 30000 });
  ok(true, 'and its upload to this doc completed');
  await E.screenshot({ path: SHOTS + '/94-paste-other-doc.png' });
  await setClip(E, makePng(120, 80, 7));
  const nE = (await sum(E)).filter((e) => e.k === 'Image').length;
  await E.mouse.move(300, 500); await E.keyboard.press('Control+v'); await sleep(700);
  ok((await sum(E)).filter((e) => e.k === 'Image').length === nE + 1, 'an image copied afterwards still pastes as an image');

  // oversized image: downscaled, not rejected
  const big = makePng(2600, 1800, 5);
  console.log('oversized png bytes', big.length);
  await setClip(E, big);
  const nBefore = (await sum(E)).filter((e) => e.k === 'Image').length;
  await E.mouse.move(600, 600); await E.keyboard.press('Control+v');
  await E.waitForFunction((n) => JSON.parse(window.__draw.board.summary()).filter((e) => e.k === 'Image').length === n + 1, nBefore, { timeout: 60000 });
  await E.waitForFunction(allUploaded, null, { timeout: 60000 });
  ok(true, 'oversized image appears (downscaled in the browser) and uploads');

  // transient failure -> retry; permanent failure -> visible on the element
  await ctxE.route('**/images', (route) => route.fulfill({ status: 503, contentType: 'application/json', body: '{"error":"busy"}' }));
  await setClip(E, makePng(90, 60, 11));
  await E.mouse.move(250, 250); await E.keyboard.press('Control+v'); await sleep(4000);
  const stt = await E.evaluate(() => JSON.parse(window.__draw.board.images_used()).map((h) => (window.__draw.images.status(h) || { state: 'ok' }).state));
  ok(stt.some((s) => s === 'uploading' || s === 'retrying'), 'failing upload keeps the element and shows its state: ' + stt.join(','));
  await E.screenshot({ path: SHOTS + '/95-upload-retrying.png' });
  await ctxE.unroute('**/images');
  await E.waitForFunction(allUploaded, null, { timeout: 60000 });
  ok(true, 'upload succeeded by itself once the failure cleared');
  await ctxE.route('**/images', (route) => route.fulfill({ status: 413, contentType: 'application/json', body: '{"error":"image limit for this document reached"}' }));
  await setClip(E, makePng(70, 50, 13));
  await E.mouse.move(350, 250); await E.keyboard.press('Control+v'); await sleep(2500);
  const fs = await E.evaluate(() => JSON.parse(window.__draw.board.images_used()).map((h) => window.__draw.images.status(h)).filter(Boolean));
  ok(fs.some((s) => s.state === 'failed' && /limit/.test(s.msg)), 'permanent failure is shown on the element: ' + JSON.stringify(fs));
  await E.screenshot({ path: SHOTS + '/96-upload-failed.png' });
  await ctxE.unroute('**/images');

  console.log('ROOM=' + ROOM + ' URL=' + url);
  console.log(fails ? `FAILED ${fails}` : 'ALL PASS');
  await browser.close();
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
