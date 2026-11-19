// End-to-end test with Playwright (Chrome). Usage: BASE=http://127.0.0.1:8190 node tests/e2e.js
// Rooms created here start with "zz-test-" so they can be purged with `draw-server admin purge-prefix zz-test-`.
const { chromium } = require(process.env.PLAYWRIGHT || 'playwright');
const BASE = process.env.BASE || 'http://127.0.0.1:8190';
const SHOTS = process.env.SHOTS || '.';
let fails = 0;
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const summary = (p) => p.evaluate(() => JSON.parse(window.__draw.board.summary()));

async function drag(page, a, b, steps = 8) {
  await page.mouse.move(a[0], a[1]);
  await page.mouse.down();
  for (let i = 1; i <= steps; i++) await page.mouse.move(a[0] + ((b[0] - a[0]) * i) / steps, a[1] + ((b[1] - a[1]) * i) / steps);
  await page.mouse.up();
}
const tool = (p, t) => p.click(`#toolbar [data-tool="${t}"]`);

(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'chrome' });
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await ctx.newPage();
  const errs = [];
  page.on('pageerror', (e) => errs.push(String(e)));
  await page.goto(BASE + '/');
  await page.waitForFunction(() => window.__draw && window.__draw.board);

  // shapes
  await tool(page, 'rect'); await drag(page, [300, 200], [440, 280]);
  await tool(page, 'ellipse'); await drag(page, [600, 200], [740, 280]);
  await tool(page, 'diamond'); await drag(page, [900, 200], [1040, 300]);
  let s = await summary(page);
  ok(s.length === 3 && ['Rect', 'Ellipse', 'Diamond'].every((k) => s.some((e) => e.k === k)), 'draw rect/ellipse/diamond');
  // arrow connecting rect -> ellipse
  await tool(page, 'arrow'); await drag(page, [370, 240], [670, 240]);
  s = await summary(page);
  const arrow = s.find((e) => e.k === 'Arrow');
  const rect = s.find((e) => e.k === 'Rect'), ell = s.find((e) => e.k === 'Ellipse');
  ok(arrow && arrow.sb === rect.id && arrow.eb === ell.id, 'arrow binds both shapes');
  // move the ellipse; arrow end follows
  await tool(page, 'select');
  const before = arrow.ax;
  await drag(page, [670, 240], [670, 420]);
  s = await summary(page);
  const a2 = s.find((e) => e.k === 'Arrow');
  ok(a2.ax[3] > before[3] + 50, `arrow follows moved shape (${before[3]} -> ${a2.ax[3]})`);
  // resize via handle
  const r0 = s.find((e) => e.k === 'Rect');
  await page.mouse.click(5, 790); // deselect
  await page.mouse.click(370, 240);
  await drag(page, [r0.x + r0.w, r0.y + r0.h], [r0.x + r0.w + 60, r0.y + r0.h + 40]);
  s = await summary(page);
  ok(s.find((e) => e.k === 'Rect').w > r0.w + 40, 'resize rect with handle');
  // pencil
  await tool(page, 'pencil');
  await page.mouse.move(300, 500); await page.mouse.down();
  for (let i = 0; i < 40; i++) await page.mouse.move(300 + i * 8, 500 + Math.sin(i / 4) * 40);
  await page.mouse.up();
  s = await summary(page);
  const pen = s.find((e) => e.k === 'Pencil');
  ok(pen && pen.np >= 4 && pen.np < 40, `pencil stroke simplified (${pen && pen.np} pts)`);
  // text outside shapes
  await tool(page, 'text'); await page.mouse.click(700, 560); await sleep(150);
  await page.keyboard.type('hello text'); await page.mouse.click(1000, 700);
  s = await summary(page);
  ok(s.some((e) => e.k === 'Text' && e.tx === 'hello text'), 'free text');
  // text in shape by dblclick
  await tool(page, 'select');
  await page.mouse.dblclick(960, 250); await sleep(150);
  await page.keyboard.type('inside');
  await page.keyboard.press('Enter'); // Enter commits, Esc cancels (item 45)
  s = await summary(page);
  ok(s.some((e) => e.k === 'Diamond' && e.tx === 'inside'), 'text inside shape');
  // edit text
  await page.mouse.dblclick(960, 250); await sleep(150);
  await page.keyboard.press('Control+a'); await page.keyboard.type('changed');
  await page.keyboard.press('Enter');
  s = await summary(page);
  ok(s.some((e) => e.k === 'Diamond' && e.tx === 'changed'), 'edit text');
  await page.screenshot({ path: SHOTS + '/local-shapes.png' });
  // undo / redo
  const n0 = (await summary(page)).length;
  await page.mouse.click(5, 790);
  await page.mouse.click(300, 500); // hits the pencil stroke start
  await page.keyboard.press('Delete');
  const n1 = (await summary(page)).length;
  await page.keyboard.press('Control+z');
  const n2 = (await summary(page)).length;
  await page.keyboard.press('Control+Shift+z');
  const n3 = (await summary(page)).length;
  ok(n1 === n0 - 1 && n2 === n0 && n3 === n1, `undo/redo delete (${n0},${n1},${n2},${n3})`);
  // zoom
  await page.mouse.move(640, 400);
  for (let i = 0; i < 40; i++) await page.mouse.wheel(0, -200);
  await sleep(200);
  const zmax = await page.evaluate(() => window.__draw.board.zoom());
  for (let i = 0; i < 80; i++) await page.mouse.wheel(0, 400);
  await sleep(200);
  const zmin = await page.evaluate(() => window.__draw.board.zoom());
  ok(Math.abs(zmax - 4) < 0.01 && Math.abs(zmin - 0.1) < 0.01, `zoom clamps 10%..400% (${zmin}, ${zmax})`);
  await page.click('#btn-zin');
  ok((await page.evaluate(() => window.__draw.board.zoom())) > zmin, 'zoom button');
  // reload keeps document (local), no network write
  await page.click('#btn-save');
  await sleep(300);
  const writes = await page.evaluate(() => window.__draw.netWrites);
  await page.reload();
  await page.waitForFunction(() => window.__draw && window.__draw.board);
  await sleep(500);
  const n4 = (await summary(page)).length;
  ok(n4 === n3 && writes === 0, `local reload keeps document (${n4} elements, ${writes} network writes)`);
  ok(errs.length === 0, 'no page errors ' + errs.join('|'));
  await browser.close();
  console.log(fails ? `${fails} FAILED` : 'ALL OK');
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
