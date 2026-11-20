// PLAN items 53..57 (version, HTML objects, mermaid -> flow, AI pipeline, whimsical routing) in a real browser.
//   BASE=https://example.com SHOTS=dir node tests/round10.js
const L = require('./lib');
const { ok, sleep, drag, tool, els, converge, state, BASE } = L;
const ROOM = 'zz-test-r10' + L.rnd();
const SHOT = (n) => (process.env.SHOTS || '.') + '/r10-' + n + '.png';
const toastText = (p) => p.evaluate(() => document.getElementById('toast').textContent);
const addText = (p, text) => p.evaluate((t) => { const b = window.__draw.board; b.add_text(t, 200, 40, 200, 150); return JSON.parse(b.selection())[0]; }, text);
const click = (p, label) => p.evaluate((l) => document.querySelector(`#style [data-conv="${l}"]`).click(), label);
const route = (p, id) => p.evaluate((i) => Array.from(window.__draw.board.arrow_route(i)), id);
const live = (p) => els(p);
const rects = (all) => all.filter((e) => ['rect', 'ellipse', 'diamond'].includes(e.k));
const inside = (r, e, pad = 0) => {
  for (let i = 0; i + 3 < r.length; i += 2) {
    const [x0, x1] = [Math.min(r[i], r[i + 2]), Math.max(r[i], r[i + 2])];
    const [y0, y1] = [Math.min(r[i + 1], r[i + 3]), Math.max(r[i + 1], r[i + 3])];
    if (x0 < e.x + e.w - pad && x1 > e.x + pad && y0 < e.y + e.h - pad && y1 > e.y + pad) return true;
  }
  return false;
};
const ortho = (r) => { for (let i = 0; i + 3 < r.length; i += 2) if (Math.abs(r[i] - r[i + 2]) > 0.05 && Math.abs(r[i + 1] - r[i + 3]) > 0.05) return false; return true; };
const overlap = (a, b) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;

const FLOW = '```mermaid\nflowchart TD\n  A[Start] --> B{Valid input?}\n  B -->|yes| C[Process data]\n  B -->|no| D[Show error]\n  C --> E((Save))\n  D --> A\n  E -.-> F(Notify user)\n  C ==> G[Audit log]\n  A --> G\n  B --> F\n  G --> F\n```\n';

(async () => {
  const browser = await L.launch();
  const [ctxO, O, url] = await L.createCloudRoom(browser, 'owner', ROOM);
  const [ctxJ, J] = await L.open(browser, url, 'joiner');
  await J.waitForFunction(() => window.__draw.wsOpen);
  const errs = [];
  O.on('pageerror', (e) => errs.push(e.message));

  // ================= 53 version
  const ver = await O.evaluate(() => window.__draw.version);
  ok(/^\d+\.\d+\.\d+$/.test(ver), `53: window.__draw.version is semver (${ver})`);
  ok((await O.textContent('#ver')) === 'v' + ver, '53: the topbar shows "v<version>" matching window.__draw.version');
  const vb = await O.evaluate(() => { const v = document.getElementById('ver').getBoundingClientRect(), b = document.getElementById('brand').getBoundingClientRect(), t = document.getElementById('topbar').getBoundingClientRect(); return { dx: Math.round(v.left - b.right), fs: parseFloat(getComputedStyle(document.getElementById('ver')).fontSize), inside: v.top >= t.top && v.bottom <= t.bottom }; });
  ok(vb.dx >= 0 && vb.dx < 24 && vb.fs <= 12 && vb.inside, `53: the version sits next to the logo, small (${JSON.stringify(vb)})`);
  await O.screenshot({ path: SHOT('53-version') });

  // ================= 54 HTML objects
  await O.evaluate(() => window.__draw.board.deselect());
  await click(O, 'text2html');
  ok(/select a text element/i.test(await toastText(O)), '54: text2html with nothing selected is a no-op with a hint');
  const n0 = (await live(O)).length;
  await click(O, 'text2mermaid');
  ok((await live(O)).length === n0, '54/55: text2mermaid with nothing selected creates nothing');

  const HTML = '<!DOCTYPE html>\n<html><head><meta charset="utf-8"><title>t</title><style>body{font-family:sans-serif;background:#fffbe6}h1{color:#c2255c}</style></head>\n<body><h1 id="h">Hello from HTML</h1><p>Rendered, not executed.</p>\n<img src="https://example.com/x.png" alt="ext">\n<script>try{window.parent.__pwned="parent";}catch(e){} try{top.document.title="pwned";}catch(e){} document.body.setAttribute("data-ran","yes");</script>\n<img src=x onerror="try{window.parent.__pwned=\'onerror\'}catch(e){}"></body></html>';
  const tid = await addText(O, HTML);
  const nBefore = (await live(O)).length;
  await click(O, 'text2html');
  await sleep(600);
  const a1 = await live(O);
  const ho = a1.find((e) => e.k === 'html');
  ok(!!ho && ho.src.includes('Hello from HTML'), '54: text2html creates an html element that holds the source (full document accepted)');
  ok(a1.some((e) => e.id === tid && e.k === 'text'), '65: the source text element stays');
  ok(a1.length === nBefore + 1, '65: exactly one new element is added');
  const fr = await O.evaluate(() => { const f = document.querySelector('#htmllayer iframe'); return f ? { sb: f.getAttribute('sandbox'), has: f.hasAttribute('sandbox') } : null; });
  ok(fr && fr.has && fr.sb === '', `54: the iframe has an EMPTY sandbox attribute (no allow-scripts, no allow-same-origin) (${JSON.stringify(fr)})`);
  ok((await O.evaluate(() => window.__pwned)) === undefined && (await O.title()) !== 'pwned', '54: embedded JavaScript did not run and could not reach the parent');
  const reach = await O.evaluate(() => { try { return document.querySelector('#htmllayer iframe').contentDocument ? 'reachable' : 'null'; } catch (e) { return 'blocked'; } });
  ok(reach !== 'reachable', `54: the parent cannot read the frame document (${reach})`);
  const frame = O.frames().find((f) => f !== O.mainFrame());
  let ran = 'unknown', h1 = '';
  if (frame) { try { ran = await frame.evaluate(() => document.body.getAttribute('data-ran')); h1 = await frame.evaluate(() => document.getElementById('h')?.textContent); } catch (e) { ran = 'noexec'; } }
  ok(ran !== 'yes', `54: the script inside the frame never executed (data-ran=${ran})`);
  const shown = await O.evaluate(() => { const f = document.querySelector('#htmllayer iframe'); return f.srcdoc.includes('Hello from HTML'); });
  ok(shown, `54: the markup is the frame's srcdoc and renders there (${h1 || 'sandboxed origin, not readable'})`);
  const csp = await O.evaluate(async () => (await fetch('/')).headers.get('content-security-policy'));
  ok(/img-src[^;]*https:/.test(csp || '') && /script-src 'self'/.test(csp || '') && !/script-src[^;]*https:/.test(csp || ''), '54: CSP allows https: images, scripts stay self-only');
  const geo = (p) => p.evaluate(() => { const f = document.querySelector('#htmllayer iframe').getBoundingClientRect(); const e = window.__draw.board.element_screen(JSON.parse(window.__draw.board.html_list())[0][0]); return { f: [f.left, f.top, f.width, f.height], e: Array.from(e) }; });
  const close = (a, b, t = 1.5) => a.every((v, i) => Math.abs(v - b[i]) <= t);
  let g = await geo(O);
  ok(close(g.f, g.e), `54: frame matches the element on screen (${g.f.map(Math.round)} vs ${g.e.map(Math.round)})`);
  await O.evaluate(() => window.__draw.board.set_zoom(640, 400, 1.7)); await sleep(250);
  g = await geo(O);
  ok(close(g.f, g.e), `54: frame follows zoom (${g.f.map(Math.round)} vs ${g.e.map(Math.round)})`);
  await O.evaluate(() => window.__draw.board.pan(-90, 55)); await sleep(250);
  g = await geo(O);
  ok(close(g.f, g.e), `54: frame follows pan (${g.f.map(Math.round)} vs ${g.e.map(Math.round)})`);
  await O.evaluate(() => window.__draw.board.set_zoom(640, 400, 1)); await sleep(150);
  await O.screenshot({ path: SHOT('54-html') });
  g = await geo(O);
  const c0 = [g.f[0] + 40, g.f[1] + 40];
  await drag(O, c0, [c0[0] + 120, c0[1] + 60]);
  await sleep(300);
  const hm = (await live(O)).find((e) => e.k === 'html');
  ok(Math.abs(hm.x - ho.x - 120) < 3 && Math.abs(hm.y - ho.y - 60) < 3, `54: the object moves like a shape (dx ${Math.round(hm.x - ho.x)}, dy ${Math.round(hm.y - ho.y)})`);
  g = await geo(O);
  ok(close(g.f, g.e), '54: the frame followed the move');
  ok(await converge([O, J]), '54: the html object syncs to another client');
  const jf = await J.evaluate(() => { const f = document.querySelector('#htmllayer iframe'); return f ? [f.getAttribute('sandbox'), f.srcdoc.includes('Hello from HTML')] : null; });
  ok(jf && jf[0] === '' && jf[1], '54: the other client renders it in its own sandboxed frame');
  await O.evaluate(() => window.__draw.board.undo()); await sleep(150);
  await O.evaluate(() => window.__draw.board.undo()); await sleep(300);
  const a2 = await live(O);
  ok(a2.some((e) => e.id === tid && e.k === 'text') && !a2.some((e) => e.k === 'html'), '54/65: undo removes only the html object, the text stays');
  ok((await O.$$('#htmllayer iframe')).length === 0, '54: the frame is gone after undo');
  await O.evaluate(() => { window.__draw.board.redo(); window.__draw.board.redo(); }); await sleep(300);
  ok((await live(O)).some((e) => e.k === 'html'), '54: redo brings it back');
  await O.evaluate(() => window.__draw.board.select_ids(JSON.stringify([JSON.parse(window.__draw.board.html_list())[0][0]])));
  await O.evaluate(() => window.__draw.board.delete_selected()); await sleep(300);
  ok(!(await live(O)).some((e) => e.k === 'html') && (await O.$$('#htmllayer iframe')).length === 0, '54: delete removes the object and its frame');
  ok((await converge([O, J])) && (await J.$$('#htmllayer iframe')).length === 0, '54: the delete reached the other client');
  await addText(O, '<h2 style="color:teal">Fragment</h2><button>ok</button>');
  await click(O, 'text2html'); await sleep(300);
  ok((await live(O)).some((e) => e.k === 'html' && e.src.startsWith('<h2')), '54: a plain fragment works');
  await addText(O, '\ufeff```html\n<p>fenced</p>\n```\n   ');
  await click(O, 'text2html'); await sleep(300);
  ok((await live(O)).some((e) => e.k === 'html' && e.src === '<p>fenced</p>'), '56: a fenced block with BOM and trailing whitespace is accepted and unwrapped');
  const big = '<p>' + 'x'.repeat(410000) + '</p>';
  const bid = await addText(O, big);
  const nHtml = (await live(O)).filter((e) => e.k === 'html').length;
  await click(O, 'text2html');
  const tt = await toastText(O);
  ok(/too large/i.test(tt) && /400 KB/.test(tt), `62: >400 KB is refused with a clear toast (${tt.slice(0, 80)})`);
  ok((await live(O)).filter((e) => e.k === 'html').length === nHtml && (await live(O)).some((e) => e.id === bid && e.k === 'text'), '54/56: on refusal the source text is intact');
  ok(/draw v\d+\.\d+\.\d+/.test(tt), '56: the error toast quotes the build version');
  const wsOps = (page, ops) => page.evaluate(([u, ops]) => new Promise((res) => {
    const ws = new WebSocket(u); const got = [];
    ws.onopen = () => ws.send(JSON.stringify({ t: 'hello', cid: 'h' + Math.random().toString(36).slice(2, 9), name: 'raw', token: '', seq: 0, build: window.__draw.build }));
    ws.onmessage = (e) => { const m = JSON.parse(e.data); got.push(m.t + (m.msg ? ':' + m.msg : '')); if (m.t === 'snap') ws.send(JSON.stringify({ t: 'ops', ops })); };
    setTimeout(() => { ws.close(); res(got); }, 1800);
  }), [BASE.replace(/^http/, 'ws') + '/ws' + new URL(url).pathname, ops]);
  const mk = (o) => ({ id: 'zz' + L.rnd(), v: 1, by: 'rawclient', k: 'html', x: 0, y: 0, w: 100, h: 100, z: 1, ...o });
  const r1 = await wsOps(O, [mk({ src: 'x'.repeat(409601) })]);
  ok(r1.some((m) => /err:.*html too large/.test(m)), `62: the server refuses an html source over 400 KB (${r1.join(',')})`);
  const r2 = await wsOps(O, [mk({ k: 'rect', src: '<b>x</b>' })]);
  ok(r2.some((m) => /err:.*html payload/.test(m)), `54: the server refuses an html payload on a non-html element (${r2.join(',')})`);
  const r3 = await wsOps(O, [mk({ src: '' })]);
  ok(r3.some((m) => /err:.*without source/.test(m)), `54: the server refuses an empty html object (${r3.join(',')})`);
  const r4 = await wsOps(O, [mk({ src: '<p>ok</p>' })]);
  ok(!r4.some((m) => m.startsWith('err')), `54: the server accepts a valid html object (${r4.join(',')})`);

  // ================= 55 mermaid
  await O.evaluate(() => window.__draw.board.deselect());
  const mid = await addText(O, FLOW);
  const pre = await live(O);
  await click(O, 'text2mermaid');
  await sleep(700);
  const all = await live(O);
  const fresh = (e) => !pre.some((p) => p.id === e.id);
  const nodes = rects(all).filter(fresh);
  const arrows = all.filter((e) => e.k === 'arrow' && fresh(e));
  const labels = all.filter((e) => e.k === 'text' && fresh(e));
  ok(all.some((e) => e.id === mid && e.k === 'text'), '65: the mermaid text element stays');
  const kinds = (k) => nodes.filter((e) => e.k === k).length;
  ok(nodes.length === 7 && kinds('diamond') === 1 && kinds('ellipse') === 1 && kinds('rect') === 5, `55: 7 nodes with the right shapes (rect ${kinds('rect')}, diamond ${kinds('diamond')}, circle ${kinds('ellipse')})`);
  ok(arrows.length === 10, `55: 10 edges became arrows (${arrows.length})`);
  ok(labels.length === 2 && labels.some((t) => t.tx === 'yes') && labels.some((t) => t.tx === 'no'), `55: edge labels became small text elements (${labels.map((t) => t.tx)})`);
  ok(nodes.some((n) => n.tx === 'Valid input?') && nodes.some((n) => n.tx === 'Process data'), '55: node labels with spaces are kept');
  let ov = 0; for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) if (overlap(nodes[i], nodes[j])) ov++;
  ok(ov === 0, `55: node bounding boxes never overlap (${ov} overlaps)`);
  const byId = new Map(nodes.map((n) => [n.id, n]));
  ok(arrows.every((a) => byId.has(a.sb) && byId.has(a.eb)), '55: every edge is a real arrow bound to its two nodes');
  const ays = await Promise.all(arrows.map((a) => route(O, a.id)));
  let cross = 0, nonOrtho = 0;
  arrows.forEach((a, i) => { if (!ortho(ays[i])) nonOrtho++; for (const n of nodes) if (n.id !== a.sb && n.id !== a.eb && inside(ays[i], n, 1)) cross++; });
  ok(cross === 0, `57: no routed edge passes through an unrelated node (${cross})`);
  ok(nonOrtho === 0, `57: every route is orthogonal (${nonOrtho} diagonal)`);
  let lov = 0; for (const t of labels) for (const n of nodes) if (overlap(t, n)) lov++;
  ok(lov === 0, `55: edge labels never sit on a node (${lov})`);
  const A = nodes.find((n) => n.tx === 'Start'), Bn = nodes.find((n) => n.tx === 'Valid input?');
  ok(Bn.y > A.y + A.h, '55: flowchart TD lays out top to bottom');
  await O.evaluate(() => window.__draw.board.fit(1280, 800)); await sleep(400);
  await O.screenshot({ path: SHOT('55-mermaid') });
  ok(await converge([O, J]), '55: the flow syncs to another client');
  ok(arrows.filter((a) => (a.ah & 1) === 1).length === 10 && arrows.filter((a) => (a.ah & 16) === 16).length === 1, '55: every edge has a head, -.-> is dashed');
  await O.evaluate(() => window.__draw.board.undo()); await sleep(300);
  const u = await live(O);
  ok(u.some((e) => e.id === mid && e.k === 'text') && !u.some((e) => arrows.some((a) => a.id === e.id) || nodes.some((n) => n.id === e.id) || labels.some((t) => t.id === e.id)), '55/65: ONE undo removes everything created, the text stays');
  await O.evaluate(() => window.__draw.board.redo()); await sleep(300);
  ok((await live(O)).filter((e) => e.k === 'arrow').length >= 10, '55: redo recreates the flow');
  for (const [src, chk, name] of [
    ['graph LR\nA[Left] --> B[Mid] --> C[Right]', (n) => n.find((x) => x.tx === 'Right').x > n.find((x) => x.tx === 'Left').x + 100, 'graph LR'],
    ['flowchart BT\nA[Bottom] --> B[Top]', (n) => n.find((x) => x.tx === 'Top').y < n.find((x) => x.tx === 'Bottom').y, 'flowchart BT'],
    ['flowchart RL\nA["Start here"] --> B{Pick}', (n) => n.find((x) => x.tx === 'Pick').x < n.find((x) => x.tx === 'Start here').x, 'flowchart RL with a quoted label'],
  ]) {
    await O.evaluate(() => window.__draw.board.deselect());
    const ids = new Set((await live(O)).map((e) => e.id));
    await addText(O, src); await click(O, 'text2mermaid'); await sleep(300);
    const fr2 = (await live(O)).filter((e) => !ids.has(e.id));
    ok(chk(rects(fr2)), `55: ${name} lays out in the right direction`);
  }
  const bad = 'flowchart TD\nA[ok] --> B[ok]\nB ~~> C\nC --> D';
  await O.evaluate(() => window.__draw.board.deselect());
  const badId = await addText(O, bad);
  const cnt = (await live(O)).length;
  await click(O, 'text2mermaid');
  const bt = await toastText(O);
  ok(/Line 3/.test(bt), `55: invalid code names the line (${bt.slice(0, 90)})`);
  ok((await live(O)).length === cnt && (await live(O)).some((e) => e.id === badId && e.tx === bad), '55/56: on invalid code NOTHING is created and the source text is intact');
  ok(/draw v\d+\.\d+\.\d+/.test(bt), '56: the error toast quotes the build version');
  await O.evaluate(() => window.__draw.board.deselect());
  const b2 = await addText(O, 'hello there');
  await click(O, 'text2mermaid');
  ok(/expected 'flowchart/i.test(await toastText(O)), '55: plain text is not mistaken for a diagram');
  await O.evaluate(([a, b]) => window.__draw.board.select_ids(JSON.stringify([a, b])), [badId, b2]);
  await click(O, 'text2mermaid');
  ok(/select one text/i.test(await toastText(O)), '55: with several elements selected the button explains itself');

  // ================= 57 routing on manual arrows (local doc)
  const [Rctx, R] = await L.open(browser, BASE + '/');
  await tool(R, 'rect'); await drag(R, [200, 200], [320, 260]);
  await tool(R, 'rect'); await drag(R, [600, 200], [720, 260]);
  await tool(R, 'rect'); await drag(R, [400, 180], [520, 280]);
  await tool(R, 'arrow'); await drag(R, [260, 230], [660, 230]);
  const m = await R.evaluate(() => JSON.parse(window.__draw.board.summary()));
  const arr = m.find((e) => e.k === 'Arrow');
  ok(arr && arr.sb && arr.eb, '57: a manual arrow still binds to both shapes');
  const rects3 = m.filter((e) => e.k === 'Rect');
  const mr = await route(R, arr.id);
  const obst = rects3.find((e) => e.id !== arr.sb && e.id !== arr.eb);
  ok(ortho(mr) && !inside(mr, obst, 1), `57: the manual arrow goes around the node in the way, orthogonally (${mr.map(Math.round)})`);
  const sn = rects3.find((e) => e.id === arr.sb);
  ok(Math.abs(mr[0] - (sn.x + sn.w)) < 6 || Math.abs(mr[0] - sn.x) < 6 || Math.abs(mr[1] - sn.y) < 6 || Math.abs(mr[1] - (sn.y + sn.h)) < 6, '57: the arrow leaves from a side of its node');
  const en = await R.evaluate((i) => Array.from(window.__draw.board.element_screen(i)), arr.eb);
  const node = await R.evaluate((i) => Array.from(window.__draw.board.element_screen(i)), arr.eb);
  await drag(R, [node[0] + 10, node[1] + 10], [node[0] + 40, node[1] + 210], 12);
  await sleep(200);
  const mr2 = await route(R, arr.id);
  ok(JSON.stringify(mr2) !== JSON.stringify(mr) && ortho(mr2), `57: moving a node reroutes the arrow (${mr2.map(Math.round)})`);
  const m2 = await R.evaluate(() => JSON.parse(window.__draw.board.summary()));
  ok(!inside(mr2, m2.find((e) => e.id === obst.id), 1), '57: and it still avoids the node in the way');
  await R.screenshot({ path: SHOT('57-route') });
  const dt = await R.evaluate(() => { const b = window.__draw.board; const t = performance.now(); for (let i = 0; i < 30; i++) { b.pan(1, 1); b.render(window.__draw.gfx, window.__draw.w, window.__draw.h, 1); } return (performance.now() - t) / 30; });
  ok(dt < 25, `57: a render with routed arrows is fast (${dt.toFixed(1)} ms)`);
  await R.evaluate(() => window.__draw.board.deselect());
  await tool(R, 'arrow'); await drag(R, [300, 600], [500, 700]);
  const free = (await R.evaluate(() => JSON.parse(window.__draw.board.summary()))).filter((e) => e.k === 'Arrow' && !e.sb && !e.eb).pop();
  ok((await route(R, free.id)).length === 4, '57: an arrow not attached to shapes stays a straight line');
  const lines = ['flowchart LR']; for (let i = 0; i < 40; i++) lines.push(`N${i}[Node ${i}] --> N${(i * 7 + 3) % 40}`, `N${i} --> N${(i + 1) % 40}`);
  await R.evaluate(() => window.__draw.board.deselect());
  await addText(R, lines.join('\n'));
  const t1 = Date.now();
  const res = await R.evaluate(() => window.__draw.board.text_to_mermaid());
  const dtBig = Date.now() - t1;
  ok(res.startsWith('ok:40:'), `57: a 40-node / 80-edge graph converts (${res}) in ${dtBig} ms`);
  ok(dtBig < 8000, `57: the big conversion is quick enough (${dtBig} ms)`);
  const bigAll = await live(R);
  const bn = rects(bigAll).filter((e) => /^Node \d+$/.test(e.tx));
  let bov = 0; for (let i = 0; i < bn.length; i++) for (let j = i + 1; j < bn.length; j++) if (overlap(bn[i], bn[j])) bov++;
  ok(bov === 0, `55: the big graph has no overlapping nodes (${bov})`);
  const barr = bigAll.filter((e) => e.k === 'arrow' && bn.some((n) => n.id === e.sb));
  let bc = 0, tot = 0;
  for (const a of barr) { const r = await route(R, a.id); tot++; for (const n of bn) if (n.id !== a.sb && n.id !== a.eb && inside(r, n, 1)) { bc++; break; } }
  ok(bc / tot < 0.05, `57: in the big graph almost no edge crosses an unrelated node (${bc}/${tot})`);
  await R.evaluate(() => window.__draw.board.fit(1280, 800)); await sleep(500);
  await R.screenshot({ path: SHOT('57-big-graph') });
  await Rctx.close();

  // ================= survives reload
  await O.reload(); await O.waitForFunction(() => window.__draw.board && window.__draw.wsOpen, null, { timeout: 15000 });
  await sleep(1000);
  const afterReload = await live(O);
  ok(afterReload.filter((e) => e.k === 'arrow').length >= 10 && afterReload.some((e) => e.k === 'html'), '55/54: the flow and the html object survive a reload');
  ok((await O.$$('#htmllayer iframe')).length >= 1, '54: the html frames come back after a reload');
  ok(errs.length === 0, `no page errors (${errs.join('|')})`);

  await browser.close();
  console.log(state.fails ? `\n${state.fails} FAILED` : '\nALL PASSED');
  process.exit(state.fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
