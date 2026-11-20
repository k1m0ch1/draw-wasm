// PLAN items 58..65: version, adjustable elbows, subgraphs, the real mermaid sample, convert-beside, 400 KB html.
//   BASE=https://example.com SHOTS=dir node tests/round12.js
const fs = require('fs');
const path = require('path');
const L = require('./lib');
const { ok, sleep, drag, tool, els, converge, state, BASE } = L;
const ROOM = 'zz-test-r12' + L.rnd();
const SHOT = (n) => (process.env.SHOTS || '.') + '/r12-' + n + '.png';
const SAMPLE = fs.readFileSync(path.join(__dirname, 'fixtures', 'mermaid-r12.txt'), 'utf8');
const toastText = (p) => p.evaluate(() => document.getElementById('toast').textContent);
const addText = (p, text, sx = 120, sy = 150) => p.evaluate(([t, x, y]) => { const b = window.__draw.board; b.add_text(t, 200, 40, x, y); return JSON.parse(b.selection())[0]; }, [text, sx, sy]);
const click = (p, label) => p.evaluate((l) => document.querySelector(`#style [data-conv="${l}"]`).click(), label);
const route = (p, id) => p.evaluate((i) => Array.from(window.__draw.board.arrow_route(i)), id);
const overlap = (a, b) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
const within = (a, c) => a.x >= c.x && a.y >= c.y && a.x + a.w <= c.x + c.w && a.y + a.h <= c.y + c.h;
const segHits = (r, e, pad = 1) => {
  for (let i = 0; i + 3 < r.length; i += 2) {
    const [x0, x1] = [Math.min(r[i], r[i + 2]), Math.max(r[i], r[i + 2])];
    const [y0, y1] = [Math.min(r[i + 1], r[i + 3]), Math.max(r[i + 1], r[i + 3])];
    if (x0 < e.x + e.w - pad && x1 > e.x + pad && y0 < e.y + e.h - pad && y1 > e.y + pad) return true;
  }
  return false;
};
const ortho = (r) => { for (let i = 0; i + 3 < r.length; i += 2) if (Math.abs(r[i] - r[i + 2]) > 0.05 && Math.abs(r[i + 1] - r[i + 3]) > 0.05) return false; return true; };
const handles = (p) => p.evaluate(() => JSON.parse(window.__draw.board.seg_handles_json()));

(async () => {
  const browser = await L.launch();
  const [ctxO, O, url] = await L.createCloudRoom(browser, 'owner', ROOM);
  const [ctxJ, J] = await L.open(browser, url, 'joiner');
  await J.waitForFunction(() => window.__draw.wsOpen);
  const errs = [];
  O.on('pageerror', (e) => errs.push(e.message));

  // ---- 53/58: the build is the new one
  const ver = await O.evaluate(() => window.__draw.version);
  ok(ver === '0.11.0', `version is 0.11.0 (${ver})`);

  // ================= 63/64/65 the real sample
  await O.evaluate(() => window.__draw.board.deselect());
  const tid = await addText(O, '```mermaid\n' + SAMPLE.trimEnd() + '\n```\n  ', 60, 120);
  const t0 = (await els(O)).find((e) => e.id === tid);
  const pre = new Set((await els(O)).map((e) => e.id));
  await click(O, 'text2mermaid');
  await sleep(800);
  const all = await els(O);
  const fresh = all.filter((e) => !pre.has(e.id));
  const cont = fresh.filter((e) => e.k === 'rect' && e.sh === 1);
  const nodes = fresh.filter((e) => ['rect', 'ellipse', 'diamond'].includes(e.k) && e.sh !== 1);
  const arrows = fresh.filter((e) => e.k === 'arrow');
  const labels = fresh.filter((e) => e.k === 'text');
  const t1 = all.find((e) => e.id === tid);
  ok(t1 && t1.tx === t0.tx && t1.x === t0.x && t1.y === t0.y, '65: the source text stays exactly as it was');
  ok(fresh.every((e) => e.x >= t0.x + t0.w || (e.k === 'arrow')), `65: generated objects sit to the right of the text`);
  ok(cont.length === 3 && ['PLANNER', 'BUILDER', 'EVALUATOR'].every((n) => cont.some((c) => c.tx === n)), `63/64: 3 labelled containers (${cont.map((c) => c.tx)})`);
  ok(nodes.length === 13, `64: 13 nodes (${nodes.length})`);
  ok(nodes.filter((n) => n.sh === 2).length === 1 && nodes.filter((n) => n.k === 'diamond').length === 1, '64: one stadium (EXIT) and one decision (E5)');
  ok(!all.some((e) => /<br/i.test(e.tx || '') && e.id !== tid), '64: no literal <br/> in any generated label');
  ok(nodes.filter((n) => n.tx.includes('\n')).length >= 8, '64: <br/> became real line breaks');
  ok(arrows.length === 16 && arrows.filter((a) => cont.some((c) => c.id === a.sb) && cont.some((c) => c.id === a.eb)).length === 2, `64: 2 container-to-container edges, ${arrows.length} edges in total`);
  ok(labels.length === 5 && labels.filter((l) => l.tx === 'gagal').length === 3 && labels.some((l) => l.tx === 'Sudah') && labels.some((l) => /Belum/.test(l.tx)), `64: dotted and solid quoted edge labels (${labels.map((l) => l.tx)})`);
  const sorted = [...cont].sort((a, b) => a.x - b.x);
  ok(sorted.map((c) => c.tx).join() === 'PLANNER,BUILDER,EVALUATOR' && sorted[0].x + sorted[0].w <= sorted[1].x && sorted[1].x + sorted[1].w <= sorted[2].x, '63: containers are placed left to right without overlapping');
  const owner = (n) => cont.find((c) => within(n, c));
  const mine = (pre) => nodes.filter((n) => n.tx.startsWith(pre));
  const inside = { PLANNER: ['Kumpulin', 'Reproduksi', 'Bikin issue'], BUILDER: ['Ambil', 'Tulis', 'Implementasi', 'Commit'], EVALUATOR: ['Pipeline', 'Server', 'Fix', 'Screenshot', 'Semua'] };
  let bad = 0;
  for (const [c, starts] of Object.entries(inside)) for (const s of starts) { const n = mine(s)[0]; if (!n || !owner(n) || owner(n).tx !== c) bad++; }
  ok(bad === 0, `63: every node sits inside its own container (${bad} wrong)`);
  const planner = nodes.filter((n) => owner(n) && owner(n).tx === 'PLANNER').sort((a, b) => a.y - b.y);
  ok(planner.length === 3 && planner[0].y + planner[0].h < planner[1].y && planner[1].y + planner[1].h < planner[2].y && Math.abs(planner[0].x - planner[2].x) < 120, '63: PLANNER is laid out top to bottom');
  let ov = 0; for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) if (overlap(nodes[i], nodes[j])) ov++;
  let cov = 0; for (let i = 0; i < cont.length; i++) for (let j = i + 1; j < cont.length; j++) if (overlap(cont[i], cont[j])) cov++;
  let stray = 0; for (const c of cont) for (const n of nodes) if (overlap(c, n) && !within(n, c)) stray++;
  ok(ov === 0 && cov === 0 && stray === 0, `63: no node overlaps (${ov}), containers do not overlap (${cov}), nothing sticks out (${stray})`);
  const rs = await Promise.all(arrows.map((a) => route(O, a.id)));
  let cross = 0, diag = 0, endBad = 0;
  const byId = new Map([...nodes, ...cont].map((e) => [e.id, e]));
  arrows.forEach((a, i) => {
    if (!ortho(rs[i])) diag++;
    for (const n of nodes) if (n.id !== a.sb && n.id !== a.eb && segHits(rs[i], n)) { cross++; break; }
    const s = byId.get(a.sb), e = byId.get(a.eb), r = rs[i];
    const near = (x, y, b) => x >= b.x - 6 && x <= b.x + b.w + 6 && y >= b.y - 6 && y <= b.y + b.h + 6;
    if (!s || !e || !near(r[0], r[1], s) || !near(r[r.length - 2], r[r.length - 1], e)) endBad++;
  });
  ok(cross === 0, `64: no route crosses an unrelated node (${cross} of ${arrows.length})`);
  ok(diag === 0 && endBad === 0, `64: routes are orthogonal (${diag}) and land on their endpoints (${endBad} bad)`);
  let lov = 0; for (const l of labels) for (const n of nodes) if (overlap(l, n)) lov++;
  ok(lov === 0, `64: edge labels never sit on a node (${lov})`);
  await O.evaluate(() => window.__draw.board.fit(1280, 800)); await sleep(500);
  await O.screenshot({ path: SHOT('sample') });
  ok(await converge([O, J]), '64: the flow syncs to the other client');
  // undo removes only the generated objects (one step)
  await O.evaluate(() => window.__draw.board.undo()); await sleep(300);
  const u = await els(O);
  ok(u.length === all.length - fresh.length && u.some((e) => e.id === tid), '65: one undo removes exactly the generated objects, the text stays');
  await O.evaluate(() => window.__draw.board.redo()); await sleep(300);
  ok((await els(O)).length === all.length, '65: redo brings them back');
  // converting again makes another copy, not on top of the first
  await O.evaluate(([i]) => window.__draw.board.select_ids(JSON.stringify([i])), [tid]);
  await click(O, 'text2mermaid'); await sleep(500);
  const all2 = await els(O);
  const second = all2.filter((e) => !all.some((a) => a.id === e.id));
  ok(second.length === fresh.length && second.filter((e) => e.k === 'rect' && e.sh === 1).length === 3, '65: converting again creates another copy');
  const minX2 = Math.min(...second.filter((e) => e.k !== 'arrow').map((e) => e.x));
  const maxX1 = Math.max(...fresh.filter((e) => e.k !== 'arrow').map((e) => e.x + e.w));
  ok(minX2 >= maxX1, `65: the second copy does not overlap the first (${Math.round(minX2)} >= ${Math.round(maxX1)})`);
  const sel = await O.evaluate(() => JSON.parse(window.__draw.board.selection()));
  ok(sel.length === second.length && sel.every((i) => second.some((e) => e.id === i)), '65: the new objects are selected after the conversion');
  // moving a container carries its nodes
  await O.evaluate(() => { const b = window.__draw.board; b.undo(); b.deselect(); });
  await sleep(200);

  // ================= 65 html beside
  await O.evaluate(() => window.__draw.board.deselect());
  const htid = await addText(O, '<h1>side</h1>', 60, 600);
  const hb = (await els(O)).find((e) => e.id === htid);
  await click(O, 'text2html'); await sleep(400);
  const h1 = (await els(O)).find((e) => e.k === 'html' && e.src === '<h1>side</h1>');
  ok(h1 && h1.x >= hb.x + hb.w && (await els(O)).some((e) => e.id === htid && e.tx === '<h1>side</h1>'), '65: text2html leaves the text and puts the object to its right');

  // ================= 62 400 KB html
  const big = '<!DOCTYPE html><html><body><p>' + 'x'.repeat(399 * 1000) + '</p></body></html>';
  await O.evaluate(() => window.__draw.board.deselect());
  const bigId = await addText(O, big, 60, 700);
  await click(O, 'text2html'); await sleep(1500);
  const hb2 = (await els(O)).find((e) => e.k === 'html' && e.src.length > 390000);
  ok(!!hb2, '62: a 399 KB html source converts');
  ok(await converge([O, J], 15000), '62: and syncs to the other client');
  await O.reload(); await O.waitForFunction(() => window.__draw.board && window.__draw.wsOpen, null, { timeout: 20000 });
  await sleep(1500);
  ok((await els(O)).some((e) => e.k === 'html' && e.src.length > 390000), '62: the document with the 399 KB object reloads');
  const toobig = '<p>' + 'y'.repeat(401 * 1024) + '</p>';
  await O.evaluate(() => window.__draw.board.deselect());
  await addText(O, toobig, 60, 760);
  const nb = (await els(O)).filter((e) => e.k === 'html').length;
  await click(O, 'text2html');
  const tt = await toastText(O);
  ok(/too large/i.test(tt) && /400 KB/.test(tt) && (await els(O)).filter((e) => e.k === 'html').length === nb, `62: 401 KB is refused with a clear toast (${tt.slice(0, 70)})`);

  // ================= 59 adjustable elbows (real mouse)
  const [Rctx, R, rurl] = await L.createCloudRoom(browser, 'elbow', 'zz-test-r12e' + L.rnd());
  await R.evaluate(() => { const b = window.__draw.board; b.select_all(); b.delete_selected(); });
  await tool(R, 'rect'); await drag(R, [200, 200], [320, 260]);
  await tool(R, 'rect'); await drag(R, [620, 420], [740, 480]);
  await tool(R, 'arrow'); await drag(R, [260, 230], [680, 450]);
  await sleep(300);
  let a = (await els(R)).find((e) => e.k === 'arrow');
  ok(a && a.sb && a.eb, '59: an arrow bound to two shapes');
  const r0 = await route(R, a.id);
  ok(r0.length >= 8 && ortho(r0), `59: it has bends (${r0.length / 2} points)`);
  let hs = await handles(R);
  ok(hs.length >= 1, `59: a selected routed arrow shows ${hs.length} middle-segment handle(s)`);
  await R.screenshot({ path: SHOT('elbow-before') });
  const h = hs[0];
  const [j, hx, hy, horiz] = h;
  const rect = await R.evaluate(() => { const r = document.getElementById('cv').getBoundingClientRect(); return [r.left, r.top]; });
  const d = horiz ? [0, -50] : [-50, 0];
  await drag(R, [hx + rect[0], hy + rect[1]], [hx + rect[0] + d[0], hy + rect[1] + d[1]], 10);
  await sleep(300);
  a = (await els(R)).find((e) => e.k === 'arrow');
  ok(a.rt && a.rt.length >= 2, `59: the adjusted route is stored on the element (rt=${JSON.stringify(a.rt)})`);
  const r1 = await route(R, a.id);
  const k = j; // the dragged segment is points j..j+1
  const moved = horiz ? Math.abs(r1[k * 2 + 1] - r0[k * 2 + 1]) : Math.abs(r1[k * 2] - r0[k * 2]);
  ok(moved > 30 && ortho(r1), `59: the dragged segment moved ${Math.round(moved)} px and the route stays orthogonal`);
  ok(Math.abs(r1[0] - r0[0]) < 0.5 && Math.abs(r1[1] - r0[1]) < 0.5 && Math.abs(r1[r1.length - 2] - r0[r0.length - 2]) < 0.5, '59: still attached to the bound shapes');
  await R.screenshot({ path: SHOT('elbow-after') });
  await R.evaluate(() => window.__draw.board.undo()); await sleep(200);
  ok(!((await els(R)).find((e) => e.k === 'arrow').rt || []).length, '59: undo reverts the adjustment');
  await R.evaluate(() => window.__draw.board.redo()); await sleep(200);
  // second browser sees it, and it survives a reload
  const [Sctx, S] = await L.open(browser, rurl, 'viewer');
  await S.waitForFunction(() => window.__draw.wsOpen); await sleep(800);
  const aS = (await els(S)).find((e) => e.k === 'arrow');
  ok(aS && JSON.stringify(aS.rt) === JSON.stringify(a.rt), '59: the manual route syncs to another client');
  ok(JSON.stringify(await route(S, aS.id)) === JSON.stringify(r1), '59: and it renders the same route there');
  await R.reload(); await R.waitForFunction(() => window.__draw.board && window.__draw.wsOpen, null, { timeout: 20000 }); await sleep(1000);
  const aR = (await els(R)).find((e) => e.k === 'arrow');
  ok(aR && JSON.stringify(aR.rt) === JSON.stringify(a.rt), '59: the manual route survives a reload');
  // moving a bound shape: the route stays attached and orthogonal
  const sb = (await els(R)).find((e) => e.id === aR.sb);
  const sc = await R.evaluate((i) => Array.from(window.__draw.board.element_screen(i)), sb.id);
  await drag(R, [sc[0] + sc[2] / 2, sc[1] + sc[3] / 2], [sc[0] + sc[2] / 2, sc[1] + sc[3] / 2 - 60], 8); await sleep(300);
  const r2 = await route(R, aR.id);
  ok(ortho(r2) && r2.length >= 4, '59: after moving a bound shape the arrow is still attached and orthogonal');
  // reset via double-click on a handle
  await R.evaluate((i) => window.__draw.board.select_ids(JSON.stringify([i])), aR.id);
  await R.evaluate(() => window.__draw.board.invalidate());
  let aN = (await els(R)).find((e) => e.k === 'arrow');
  if (aN.rt && aN.rt.length) {
    hs = await handles(R);
    if (hs.length) {
      const [, x2, y2] = hs[0];
      await R.mouse.dblclick(x2 + rect[0], y2 + rect[1]); await sleep(300);
    }
    aN = (await els(R)).find((e) => e.k === 'arrow');
  } else {
    ok(true, '59: (the moved shape already made the route fall back to automatic)');
  }
  // make a fresh manual route and reset it with the double-click
  await R.evaluate((i) => window.__draw.board.select_ids(JSON.stringify([i])), aN.id);
  hs = await handles(R);
  if (hs.length) {
    const [, x3, y3, hz] = hs[0];
    await drag(R, [x3 + rect[0], y3 + rect[1]], [x3 + rect[0] + (hz ? 0 : -40), y3 + rect[1] + (hz ? -40 : 0)], 8); await sleep(300);
    ok(((await els(R)).find((e) => e.k === 'arrow').rt || []).length > 0, '59: a second adjustment is stored');
    hs = await handles(R);
    await R.mouse.dblclick(hs[0][1] + rect[0], hs[0][2] + rect[1]); await sleep(300);
    ok(!(((await els(R)).find((e) => e.k === 'arrow').rt) || []).length, '59: double-clicking a handle returns to automatic routing');
  }
  // touch: the same drag with a touch-emulating context
  const T = await browser.newContext({ viewport: { width: 1280, height: 800 }, hasTouch: true });
  const tp = await T.newPage();
  await tp.goto(rurl); await tp.waitForFunction(() => window.__draw && window.__draw.board, null, { timeout: 20000 });
  await tp.waitForSelector('#name-input', { timeout: 10000 });
  await tp.fill('#name-input', 'touch'); await tp.click('#name-go');
  await tp.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 20000 });
  await sleep(1200);
  const ta = (await els(tp)).find((e) => e.k === 'arrow');
  await tp.evaluate((i) => window.__draw.board.select_ids(JSON.stringify([i])), ta.id);
  hs = await handles(tp);
  if (hs.length) {
    const [, x4, y4, hz] = hs[0];
    const box = await tp.evaluate(() => { const r = document.getElementById('cv').getBoundingClientRect(); return [r.left, r.top]; });
    const cdp = await T.newCDPSession(tp);
    const pt = (x, y) => [{ x: x + box[0], y: y + box[1] }];
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: pt(x4, y4) });
    for (let i = 1; i <= 8; i++) await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: pt(x4 + (hz ? 0 : -5 * i), y4 + (hz ? -5 * i : 0)) });
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await sleep(500);
    ok((((await els(tp)).find((e) => e.k === 'arrow').rt) || []).length > 0, '59: dragging a segment handle works with touch');
  } else ok(false, '59: touch: no handles');
  await T.close();
  await Sctx.close(); await Rctx.close();

  ok(errs.length === 0, `no page errors (${errs.join('|')})`);
  await browser.close();
  console.log(state.fails ? `\n${state.fails} FAILED` : '\nALL PASSED');
  process.exit(state.fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
