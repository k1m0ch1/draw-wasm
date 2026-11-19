// Room page (/<roomID>) must be really clickable (topmost element at each control), not just visible.
//   BASE=https://example.com SHOTS=dir node tests/roompage.js
const L = require('./lib');
const { ok, sleep, state, BASE } = L;
const ROOM = 'zz-test-rp' + L.rnd();
const top = (p, sel) => p.evaluate((s) => { const e = document.querySelector(s); if (!e) return 'missing'; const r = e.getBoundingClientRect(); const t = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return t && (e === t || e.contains(t)) ? 'self' : (t ? (t.id || t.tagName) : 'none'); }, sel);
(async () => {
  const browser = await L.launch();
  const [ctxO, O, url] = await L.createCloudRoom(browser, 'Owner', ROOM);
  for (const vp of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
    const ctx = await browser.newContext({ viewport: vp });
    const p = await ctx.newPage();
    await p.goto(BASE + '/' + ROOM);
    await p.waitForSelector('#new-doc');
    const tag = `${vp.width}x${vp.height}`;
    ok((await top(p, '#new-doc')) === 'self', `12: ${tag} New document is topmost`);
    ok((await top(p, '.docitem')) === 'self', `12: ${tag} document link is topmost`);
    for (const s of ['#stage', '#toolbar', '#zoombar']) ok(await p.evaluate((q) => getComputedStyle(document.querySelector(q)).display === 'none', s), `12: ${tag} ${s} hidden`);
    await p.screenshot({ path: `${L.SHOTS}/roompage-${vp.width}.png` });
    await p.click('.docitem', { timeout: 5000 });
    await p.waitForURL(new RegExp('/' + ROOM + '/[A-Za-z0-9_-]{10}$'), { timeout: 10000 });
    ok(true, `12: ${tag} real click on document link navigates`);
    await p.goto(BASE + '/' + ROOM);
    await p.click('#new-doc', { timeout: 5000 });
    await p.waitForURL(new RegExp('/' + ROOM + '/[A-Za-z0-9_-]{10}$'), { timeout: 10000 });
    ok(true, `12: ${tag} real click on New document navigates`);
    await ctx.close();
  }
  // owner controls
  await O.goto(BASE + '/' + ROOM);
  await O.waitForSelector('#new-doc');
  await sleep(500);
  for (const s of ['#room-pw', '#del-room']) ok((await top(O, s)) === 'self', `20/46: owner ${s} is topmost`);
  await O.click('#room-pw', { timeout: 5000 });
  ok(await O.waitForSelector('#modal:not(.hidden)', { timeout: 5000 }).then(() => true, () => false), '46: Password dialog opens by real click');
  await O.keyboard.press('Escape');
  await browser.close();
  process.exit(state.fails ? 1 : 0);
})();
