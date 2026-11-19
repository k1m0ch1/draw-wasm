// PLAN item 42: every participant of a cloud room (owner + joiners, separate browser contexts) can use every
// feature; every change reaches every other client and survives a reload. Real mouse/keyboard/clipboard input.
//   BASE=https://example.com SHOTS=dir node tests/multiuser.js
const L = require('./lib');
const { ok, sleep, drag, tool, scr, els, summary, created, converge, makePng, setClip, sig, state, FACE } = L;
const ROOM = 'zz-test-mu' + L.rnd();

const resetView = (p) => p.evaluate(() => { const b = window.__draw.board; b.pan(-b.pan_x(), -b.pan_y()); b.set_zoom(0, 0, 1); });
const styleBtn = (p, text) => p.locator('#style .chip', { hasText: new RegExp('^' + text + '$') }).first();
const waitImages = (p, n) => p.waitForFunction((k) => { const d = window.__draw; const hs = JSON.parse(d.board.images_used()); return hs.length >= 1 && hs.every((h) => { const e = d.images.get(h); return e && e.state === 'ok' && !d.images.status(h); }) && d.board.live_count() >= k; }, n, { timeout: 20000 }).then(() => true).catch(() => false);

async function run(P, others, who, bx, targetId) {
  const L2 = (m) => `${who}: ${m}`;
  const peersAll = [P, ...others];
  await resetView(P);
  const mine = {};
  // ---- draw
  let c = await created(P, async () => { await tool(P, 'rect'); await drag(P, [bx, 450], [bx + 120, 520]); });
  mine.rect = c[0]; ok(c.length === 1 && c[0].k === 'rect', L2('draws a rectangle'));
  c = await created(P, async () => { await tool(P, 'ellipse'); await drag(P, [bx + 150, 450], [bx + 260, 520]); });
  mine.ell = c[0]; ok(c.length === 1 && c[0].k === 'ellipse', L2('draws an ellipse'));
  c = await created(P, async () => { await tool(P, 'diamond'); await drag(P, [bx, 560], [bx + 120, 640]); });
  mine.dia = c[0]; ok(c.length === 1 && c[0].k === 'diamond', L2('draws a diamond'));
  c = await created(P, async () => { await tool(P, 'arrow'); await drag(P, [bx + 60, 485], [bx + 205, 485]); });
  mine.arr = c[0];
  ok(c.length === 1 && c[0].k === 'arrow' && c[0].sb === mine.rect.id && c[0].eb === mine.ell.id, L2('draws an arrow bound to both shapes'));
  c = await created(P, async () => { await tool(P, 'pencil'); await P.mouse.move(bx + 150, 600); await P.mouse.down(); for (let i = 0; i < 30; i++) await P.mouse.move(bx + 150 + i * 4, 600 + Math.sin(i / 3) * 25); await P.mouse.up(); });
  mine.pen = c[0]; ok(c.length === 1 && c[0].k === 'pencil' && c[0].pts.length >= 20, L2('draws a pencil stroke'));
  ok(await converge(peersAll), L2('all clients converge after drawing'));
  for (const O of others) { const s = await summary(O); ok(['rect', 'ellipse', 'diamond', 'arrow', 'pencil'].every((k) => s.some((e) => e.k.toLowerCase() === k && [mine.rect, mine.ell, mine.dia, mine.arr, mine.pen].some((m) => m.id === e.id))), L2('shapes visible on another client')); }

  // ---- move, resize
  await tool(P, 'select');
  let r0 = (await scr(P, mine.rect.id));
  await drag(P, [r0[0] + 20, r0[1] + 50], [r0[0] + 20, r0[1] + 50 - 40]);
  let r1 = await scr(P, mine.rect.id);
  ok(Math.abs(r1[1] - (r0[1] - 40)) < 2, L2(`moves the rectangle (${r0[1]} -> ${r1[1]})`));
  const a1 = (await summary(P)).find((e) => e.id === mine.arr.id);
  ok(a1.sb === mine.rect.id && a1.eb === mine.ell.id, L2('arrow stays bound while the shape moves'));
  await P.mouse.click(bx + 205, 505); // select the ellipse
  const e0 = await scr(P, mine.ell.id);
  await drag(P, [e0[0] + e0[2], e0[1] + e0[3]], [e0[0] + e0[2] + 40, e0[1] + e0[3] + 30]);
  const e1 = await scr(P, mine.ell.id);
  ok(e1[2] > e0[2] + 30 && e1[3] > e0[3] + 20, L2(`resizes the ellipse (${Math.round(e0[2])}x${Math.round(e0[3])} -> ${Math.round(e1[2])}x${Math.round(e1[3])})`));

  // ---- recolour, restroke, reorder, duplicate, delete
  await P.mouse.click(bx + 30, 505 - 40);
  await P.locator('#style button[title="Stroke #e03131"]').click();
  await P.locator('#style button[title="Fill #ffe3e3"]').click();
  await P.locator('#style [data-sw="6"]').click();
  let me = (await els(P)).find((e) => e.id === mine.rect.id);
  ok(me.st === '#e03131' && me.fl === '#ffe3e3' && me.sw === 6, L2(`recolours and restrokes (${me.st} ${me.fl} ${me.sw})`));
  const zBefore = me.z;
  await styleBtn(P, 'Front').click();
  me = (await els(P)).find((e) => e.id === mine.rect.id);
  ok(me.z > zBefore && me.z >= Math.max(...(await els(P)).map((e) => e.z)), L2('brings the shape to the front'));
  await styleBtn(P, 'Back').click();
  me = (await els(P)).find((e) => e.id === mine.rect.id);
  ok(me.z <= Math.min(...(await els(P)).map((e) => e.z)), L2('sends the shape to the back'));
  await tool(P, 'select');
  const d0 = await scr(P, mine.dia.id);
  await P.mouse.click(d0[0] + d0[2] / 2, d0[1] + d0[3] / 2);
  c = await created(P, async () => { await styleBtn(P, 'Duplicate').click(); });
  ok(c.length === 1 && c[0].k === 'diamond', L2('duplicates the diamond'));
  const before = (await els(P)).length;
  await P.keyboard.press('Delete'); await sleep(150);
  ok((await els(P)).length === before - 1, L2('deletes the duplicate'));
  ok(await converge(peersAll), L2('all clients converge after edits'));
  for (const O of others) ok(!(await els(O)).some((e) => e.id === c[0].id), L2('the deletion reached another client'));
  for (const O of others) { const he = (await els(O)).find((e) => e.id === mine.rect.id); ok(he && he.st === '#e03131' && he.sw === 6, L2('recolour/restroke reached another client')); }

  // ---- edit a shape somebody else created (text + colour)
  const t0 = await scr(P, targetId);
  await P.mouse.dblclick(t0[0] + t0[2] / 2, t0[1] + t0[3] / 2);
  await P.waitForSelector('textarea.editor', { timeout: 3000 });
  await P.keyboard.type('Edited by ' + who);
  await P.keyboard.press('Enter');
  await sleep(200);
  let tgt = (await els(P)).find((e) => e.id === targetId);
  ok(tgt && tgt.tx === 'Edited by ' + who, L2('edits the text of a shape somebody else created'));
  await P.mouse.click(t0[0] + 8, t0[1] + 8);
  await P.locator('#style button[title="Stroke #2f9e44"]').click();
  tgt = (await els(P)).find((e) => e.id === targetId);
  ok(tgt.st === '#2f9e44', L2('recolours a shape somebody else created'));
  ok(await converge(peersAll), L2('all clients converge after editing a foreign shape'));

  // ---- free text, text inside a shape, edit it
  c = await created(P, async () => { await tool(P, 'text'); await P.mouse.click(bx, 675); await P.waitForSelector('textarea.editor'); await P.keyboard.type('free ' + who); await P.keyboard.press('Enter'); });
  ok(c.length === 1 && c[0].k === 'text' && c[0].tx === 'free ' + who, L2('adds free text'));
  mine.txt = c[0];
  await tool(P, 'select');
  const e2 = await scr(P, mine.ell.id);
  await P.mouse.dblclick(e2[0] + e2[2] / 2, e2[1] + e2[3] / 2);
  await P.waitForSelector('textarea.editor'); await P.keyboard.type('in ' + who); await P.keyboard.press('Enter'); await sleep(150);
  ok((await els(P)).find((e) => e.id === mine.ell.id).tx === 'in ' + who, L2('adds text inside a shape'));
  await P.mouse.dblclick(e2[0] + e2[2] / 2, e2[1] + e2[3] / 2);
  await P.waitForSelector('textarea.editor'); await P.keyboard.type('again ' + who); await P.keyboard.press('Enter'); await sleep(150);
  ok((await els(P)).find((e) => e.id === mine.ell.id).tx === 'again ' + who, L2('edits the text inside the shape'));
  await P.mouse.dblclick(bx + 6, 675 + 8); // edit the free text
  await P.waitForSelector('textarea.editor').catch(() => {});
  if (await P.$('textarea.editor')) { await P.keyboard.type('free2 ' + who); await P.keyboard.press('Enter'); await sleep(150); }
  ok((await els(P)).find((e) => e.id === mine.txt.id).tx === 'free2 ' + who, L2('edits the free text'));
  ok(await converge(peersAll), L2('all clients converge after text edits'));

  // ---- images: OS clipboard paste, element copy/paste
  const png = makePng(90 + (bx % 50), 70, bx);
  await setClip(P, png);
  await P.mouse.move(bx + 200, 700);
  c = await created(P, async () => { await P.keyboard.press('Control+v'); await sleep(500); });
  ok(c.length === 1 && c[0].k === 'image', L2('pastes an OS-clipboard image'));
  mine.img = c[0];
  c = await created(P, async () => { await P.keyboard.press('Control+c'); await sleep(250); await P.mouse.move(bx + 60, 700); await P.keyboard.press('Control+v'); await sleep(500); });
  ok(c.length === 1 && c[0].k === 'image' && c[0].img === mine.img.img, L2('copies and pastes an image element'));
  mine.img2 = c[0];
  ok(await converge(peersAll), L2('all clients converge after image paste'));
  for (const O of peersAll) { await O.evaluate(() => window.__draw.board.fit(window.__draw.w, window.__draw.h)); ok(await waitImages(O, 1), L2('image bytes are available on a client')); }

  // ---- undo / redo of own change
  const n0 = (await els(P)).length;
  await P.keyboard.press('Control+z'); await sleep(200);
  ok((await els(P)).length === n0 - 1, L2('undo removes the last pasted image'));
  ok(await converge(peersAll), L2('undo reaches the other clients'));
  await P.keyboard.press('Control+Shift+z'); await sleep(200);
  ok((await els(P)).length === n0, L2('redo brings it back'));
  ok(await converge(peersAll), L2('redo reaches the other clients'));
  return mine;
}

(async () => {
  const browser = await L.launch();
  const [ctxO, O, url] = await L.createCloudRoom(browser, 'Olive', ROOM);
  const ownerRect = (await els(O))[0];
  ok(!!ownerRect && ownerRect.k === 'rect', 'owner created the room with a rectangle');
  const [ctxB, B] = await L.open(browser, url, 'Bea B');
  await B.waitForFunction(() => window.__draw.wsOpen && window.__draw.board.live_count() >= 1);
  const [ctxC, C] = await L.open(browser, url, ''); // presses Enter with an empty name: random face
  await C.waitForFunction(() => window.__draw.wsOpen && window.__draw.board.live_count() >= 1);
  const cName = await C.evaluate(() => window.__draw.you && window.__draw.you.name);
  ok(FACE.test(cName || ''), `the third participant got a face emoji as name (${cName})`);
  const pages = [O, B, C];

  const m1 = await run(O, [B, C], 'owner', 100, ownerRect.id);
  const m2 = await run(B, [O, C], 'joinerB', 450, ownerRect.id);
  const m3 = await run(C, [O, B], 'joinerC', 800, m2.rect.id); // C edits B's shape
  const m4 = await run(O, [B, C], 'owner2', 100, m3.rect.id); // the owner edits a joiner's shape

  // ---- everything survives a reload (state comes back from the server) and all clients agree
  await sleep(2500); // the server flushes within a few seconds
  const before = await sig(O);
  ok(await converge(pages), 'all three clients hold identical documents');
  const count = (await els(O)).length;
  for (const [name, p] of [['owner', O], ['joinerB', B], ['joinerC', C]]) {
    await p.reload();
    await p.waitForFunction(() => window.__draw && window.__draw.board && window.__draw.wsOpen, null, { timeout: 20000 });
    await sleep(500);
    ok((await sig(p)) === before, `${name}: after a reload the document is identical (${count} elements)`);
    const imgs = await p.evaluate(() => { const d = window.__draw; return JSON.parse(d.board.images_used()).every((h) => d.images.get(h) && d.images.get(h).state === 'ok'); }).catch(() => false);
    await p.waitForFunction(() => { const d = window.__draw; return JSON.parse(d.board.images_used()).every((h) => d.images.get(h) && d.images.get(h).state === 'ok'); }, null, { timeout: 20000 }).then(() => ok(true, `${name}: all images load after the reload`)).catch(() => ok(false, `${name}: images did not load after the reload`));
    void imgs;
  }
  await O.screenshot({ path: L.SHOTS + '/42-multiuser-owner.png' });
  await C.screenshot({ path: L.SHOTS + '/42-multiuser-joiner.png' });
  const fresh = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const F = await fresh.newPage();
  await F.goto(url); await F.waitForFunction(() => window.__draw && window.__draw.board);
  await F.waitForSelector('#name-input'); await F.click('#name-go'); // empty name
  await F.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 15000 });
  await sleep(600);
  ok((await sig(F)) === before, 'a brand-new browser sees the identical document');
  await browser.close();
  console.log(state.fails ? `${state.fails} FAILED` : 'ALL OK');
  process.exit(state.fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
