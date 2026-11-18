// Cloud + collaboration test. BASE=https://example.com node tests/collab.js
const { chromium } = require(process.env.PLAYWRIGHT || 'playwright');
const BASE = process.env.BASE;
const SHOTS = process.env.SHOTS || '.';
let fails = 0;
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sum = (p) => p.evaluate(() => JSON.parse(window.__draw.board.summary()));
const dig = (p) => p.evaluate(() => window.__draw.board.digest());
const ROOM = 'zz-test-' + Math.random().toString(36).slice(2, 8);
async function drag(page, a, b) {
  await page.mouse.move(a[0], a[1]); await page.mouse.down();
  for (let i = 1; i <= 6; i++) await page.mouse.move(a[0] + ((b[0] - a[0]) * i) / 6, a[1] + ((b[1] - a[1]) * i) / 6);
  await page.mouse.up();
}
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAIAAAACUFjqAAAAEklEQVR4nGP4z8CAFTEMLQkAnEpXASPjPrsAAAAAAElFTkSuQmCC', 'base64');
async function open(browser, url) {
  const ctx = await browser.newContext({ viewport: { width: 1200, height: 760 } });
  const p = await ctx.newPage();
  await p.goto(url);
  await p.waitForFunction(() => window.__draw && window.__draw.board);
  return [ctx, p];
}
(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'chrome' });
  const [ctxA, A] = await open(browser, BASE + '/');
  await A.click('#toolbar [data-tool="rect"]'); await drag(A, [300, 200], [440, 280]);
  // image drop via file chooser
  const [fc] = await Promise.all([A.waitForEvent('filechooser'), A.click('#toolbar [data-tool="image"]')]);
  await fc.setFiles({ name: 'a.png', mimeType: 'image/png', buffer: png });
  await sleep(700);
  ok((await sum(A)).some((e) => e.k === 'Image'), 'image added from file');
  // paste image from clipboard (synthetic paste event with a File)
  await A.evaluate(async (b64) => {
    const bin = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const dt = new DataTransfer(); dt.items.add(new File([bin], 'p.png', { type: 'image/png' }));
    document.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true }));
  }, png.toString('base64'));
  await sleep(700);
  ok((await sum(A)).filter((e) => e.k === 'Image').length === 2, 'image pasted from clipboard');
  // local doc has no share link
  ok(await A.isHidden('#btn-share'), 'local doc has no share button');
  // cloud dialog
  await A.click('#btn-cloud');
  ok((await A.textContent('#cloud-warning')).includes('PUBLIC'), 'cloud dialog warns about public');
  await A.fill('#cloud-room', 'api'); await A.click('#cloud-go'); await sleep(500);
  ok((await A.textContent('#cloud-msg')).toLowerCase().includes('reserved'), 'reserved room rejected');
  await A.fill('#cloud-room', 'Bad_Room!'); await A.click('#cloud-go'); await sleep(500);
  ok((await A.textContent('#cloud-msg')).length > 0, 'invalid room rejected');
  await A.fill('#cloud-room', ROOM); await A.fill('#cloud-title', '<img src=x onerror=alert(1)>');
  await A.click('#cloud-go');
  await A.waitForURL(new RegExp('/' + ROOM + '/[A-Za-z0-9_-]{10}$'), { timeout: 15000 });
  const url = A.url();
  ok(/\/[a-z0-9-]+\/[A-Za-z0-9_-]{10}$/.test(url), 'share url ' + url.replace(BASE, ''));
  await A.waitForFunction(() => window.__draw.wsOpen); await sleep(500);
  ok((await sum(A)).length === 3, 'cloud doc keeps 3 elements in A');
  // fresh context
  const [ctxB, B] = await open(browser, url);
  await B.waitForFunction(() => window.__draw.wsOpen); await sleep(1000);
  ok((await dig(A)) === (await dig(B)), 'fresh browser loads identical doc');
  const imgsOk = await B.evaluate(() => [...window.__draw.images.map.values()].filter((e) => e.state === 'ok').length);
  ok(imgsOk >= 1, `images loaded in fresh browser (${imgsOk})`);
  await B.screenshot({ path: SHOTS + '/cloud-B.png' });
  // live edit A -> B timing
  await A.click('#toolbar [data-tool="ellipse"]');
  const t0 = Date.now(); await drag(A, [600, 400], [740, 480]);
  await B.waitForFunction(() => JSON.parse(window.__draw.board.summary()).some((e) => e.k === 'Ellipse'), null, { timeout: 5000 });
  ok(true, `A->B in ${Date.now() - t0} ms (incl. drag)`);
  // B -> A
  await B.click('#toolbar [data-tool="diamond"]'); await drag(B, [800, 200], [900, 300]);
  await A.waitForFunction(() => JSON.parse(window.__draw.board.summary()).some((e) => e.k === 'Diamond'), null, { timeout: 5000 });
  ok(true, 'B->A diamond');
  // presence
  await B.mouse.move(500, 500); await B.mouse.move(520, 520); await sleep(600);
  const peersA = await A.evaluate(() => document.querySelectorAll('#peers .peer').length);
  const curA = await A.evaluate(() => document.querySelectorAll('#cursors .rcursor').length);
  ok(peersA === 2 && curA >= 1, `presence: ${peersA} peers, ${curA} remote cursor(s)`);
  await A.screenshot({ path: SHOTS + '/cloud-A.png' });
  // late joiner
  const [ctxC, C] = await open(browser, url);
  await C.waitForFunction(() => window.__draw.wsOpen); await sleep(800);
  ok((await dig(C)) === (await dig(A)) && (await sum(C)).length === 5, 'late joiner gets full state');
  // concurrent edits to same element
  const id = (await sum(A)).find((e) => e.k === 'Rect').id;
  await Promise.all([A, B].map((p, i) => p.evaluate(({ id, i }) => {
    const b = window.__draw.board; b.select_ids(JSON.stringify([id]));
    for (let k = 0; k < 5; k++) b.nudge(i ? 7 : -9, i ? 3 : 4);
  }, { id, i })));
  await sleep(1500);
  const [dA, dB, dC] = [await dig(A), await dig(B), await dig(C)];
  ok(dA === dB && dB === dC, 'concurrent edits converge on 3 clients ' + dA);
  // forced disconnect: close ws on B, edit on both while offline
  await B.evaluate(() => window.__draw.ws.close());
  await A.click('#toolbar [data-tool="rect"]'); await drag(A, [200, 500], [300, 560]);
  await B.click('#toolbar [data-tool="rect"]'); await drag(B, [200, 620], [300, 700]);
  await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 20000 });
  await sleep(2000);
  const [e1, e2, e3] = [await dig(A), await dig(B), await dig(C)];
  const ids = (await sum(A)).map((e) => e.id);
  ok(e1 === e2 && e2 === e3 && new Set(ids).size === ids.length && ids.length === 7, `reconnect converges, no loss/dup (${ids.length} elements)`);
  // offline/online via context
  await ctxB.setOffline(true); await sleep(300);
  await B.evaluate(() => window.__draw.ws.close());
  await ctxB.setOffline(false);
  await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 20000 });
  ok(true, 'reconnect after offline/online');
  // XSS: title must be escaped
  const title = await A.textContent('#doctitle');
  ok(title.includes('<img') && (await A.evaluate(() => document.querySelectorAll('#doctitle img').length)) === 0, 'title escaped');
  // room page
  const ctxD = await browser.newContext(); const D = await ctxD.newPage(); await D.goto(BASE + "/" + ROOM);
  await sleep(1200);
  const items = await D.evaluate(() => document.querySelectorAll('.docitem').length);
  ok(items === 1 && (await D.evaluate(() => document.querySelectorAll('#doclist img').length)) === 0, 'room page lists doc, no injected markup');
  ok(await D.isHidden('#del-room').catch(() => true) || (await D.$('#del-room')) === null, 'non-owner sees no delete room');
  console.log('ROOM=' + ROOM + ' URL=' + url);
  await browser.close();
  console.log(fails ? `${fails} FAILED` : 'ALL OK');
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
