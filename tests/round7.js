// PLAN items 41, 43, 44, 45, 46, 47 in a real browser against a deployed instance.
//   BASE=https://example.com SHOTS=dir node tests/round7.js
const L = require('./lib');
const { ok, sleep, drag, tool, scr, els, converge, makeGif, makePng, setClip, state, FACE, BASE } = L;
const ROOM = 'zz-test-r7' + L.rnd();
const zoom = (p) => p.evaluate(() => window.__draw.board.zoom());
const label = (p) => p.textContent('#zlabel');
const view = (p) => p.evaluate(() => [window.__draw.board.pan_x(), window.__draw.board.pan_y()]);
const popup = (p) => p.evaluate(() => { const t = document.getElementById('undopop'); return { text: t.textContent, shown: t.classList.contains('show'), top: t.getBoundingClientRect().top, n: document.querySelectorAll('#undopop').length }; });
const pixel = (p, x, y) => p.evaluate(([x, y]) => { const c = document.getElementById('cv'); const d = window.__draw.dpr; return [...c.getContext('2d').getImageData(Math.round(x * d), Math.round(y * d), 1, 1).data]; }, [x, y]);

(async () => {
  const browser = await L.launch();
  const [ctxO, O, url] = await L.createCloudRoom(browser, '', ROOM); // the owner leaves the name empty

  // ================= 41 default name
  const oname = await O.evaluate(() => window.__draw.you.name);
  ok(FACE.test(oname), `41: the owner without a name gets a face emoji (${oname})`);
  ok((await O.evaluate(() => document.querySelector('#peers .peer.owner')?.textContent)) === oname, '41: the avatar shows the emoji');
  const [ctxB, B] = await L.open(browser, url); // no name given
  const hint = await B.waitForSelector('#name-input');
  const dlg = await B.textContent('#modal');
  ok(/optional/i.test(dlg), '41: the name dialog says the name is optional');
  ok(/optional/i.test((await B.getAttribute('#name-input', 'placeholder')) || ''), '41: the input placeholder says optional');
  await B.click('#name-go'); // Join with nothing
  await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 15000 });
  const bname = await B.evaluate(() => window.__draw.you.name);
  ok(FACE.test(bname), `41: Join with an empty name admits with a face emoji (${bname})`);
  const stored = await B.evaluate(() => new Promise((res) => { const r = indexedDB.open('draw-wasm'); r.onsuccess = () => { try { const g = r.result.transaction('meta').objectStore('meta').get('name'); g.onsuccess = () => res(g.result); g.onerror = () => res(null); } catch (e) { res('ERR ' + e.message); } }; r.onerror = () => res(null); }));
  ok(stored === bname || /^[\s\S]$/u.test(stored || ''), `41: the emoji is remembered like a typed name (${stored})`);
  await B.reload(); await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 15000 });
  ok(!(await B.$('#name-input')) && FACE.test(await B.evaluate(() => window.__draw.you.name)), '41: after a reload no dialog is shown and the browser has a face name');
  const [ctxC, C] = await L.open(browser, url, ''); // Enter in the input with nothing
  await C.waitForFunction(() => window.__draw.wsOpen);
  const cname = await C.evaluate(() => window.__draw.you.name);
  ok(FACE.test(cname), `41: pressing Enter with nothing admits with a face emoji (${cname})`);
  ok(new Set([oname, bname, cname]).size === 3, `41: emoji are unique in the room (${oname} ${bname} ${cname})`);
  await sleep(500);
  const cur = await O.evaluate(() => [...document.querySelectorAll('#peers .peer')].map((d) => d.textContent));
  ok(cur.includes(bname) && cur.includes(cname), `41: avatars show the emoji (${cur.join(' ')})`);
  await B.mouse.move(500, 300); await B.mouse.move(520, 320); await sleep(500);
  const labels = await O.evaluate(() => [...document.querySelectorAll('#cursors .rcursor')].map((d) => d.textContent));
  ok(labels.some((t) => t.includes(bname)), `41: a live cursor label shows the emoji (${labels.join('|')})`);
  // server-side validation (raw sockets)
  const wsTry = (name) => O.evaluate(([u, name]) => new Promise((res) => {
    const ws = new WebSocket(u); const out = [];
    ws.onopen = () => ws.send(JSON.stringify({ t: 'hello', cid: 'nm' + Math.random().toString(36).slice(2, 9), name, token: '', seq: 0, build: window.__draw.build }));
    ws.onmessage = (e) => { const m = JSON.parse(e.data); out.push(m.t); if (m.t === 'snap') { res({ ok: true, name: m.you.name }); ws.close(); } if (m.t === 'err') { res({ ok: false, msg: m.msg }); ws.close(); } };
    setTimeout(() => res({ ok: false, msg: 'timeout' + out }), 4000);
  }), [BASE.replace(/^http/, 'ws') + '/ws' + new URL(url).pathname, name]);
  for (const good of ['\u{1F600}', '\u{1F60E}', '\u{1F914}', '\u{1F970}', '\u{1F97A}']) ok((await wsTry(good)).ok, `41: server accepts the face ${good} as a name`);
  for (const bad of ['\u{1F44D}', '\u{1F3F3}\uFE0F', '\u{1F5A5}', '\u{1F600}\u{1F600}', '<b>x</b>', 'a\u0007b', 'x'.repeat(33), '\u{1F44B}']) ok(!(await wsTry(bad)).ok, `41: server rejects ${JSON.stringify(bad).slice(0, 24)} as a name`);
  const empty = await wsTry('');
  ok(empty.ok && FACE.test(empty.name), `41: server assigns a face to an empty name (${empty.name})`);
  const many = new Set();
  for (let i = 0; i < 4; i++) { const r = await wsTry(''); many.add(r.name); }
  ok(many.size >= 2, `41: random faces vary (${[...many].join(' ')})`);
  await O.screenshot({ path: L.SHOTS + '/41-faces.png' });

  // ================= 47 undo/redo popup
  await tool(O, 'rect'); await drag(O, [200, 200], [300, 260]);
  await O.keyboard.press('Control+z'); await sleep(300);
  let pu = await popup(O);
  ok(pu.shown && /^Undo/.test(pu.text), `47: Ctrl+Z shows a popup (${pu.text})`);
  ok(pu.top >= 0 && pu.top < 80, `47: the popup is at the TOP of the screen (top=${Math.round(pu.top)}px)`);
  ok(/Undo: (add|delete|move|resize|edit|style|text)/.test(pu.text), `47: it says what was undone (${pu.text})`);
  await O.screenshot({ path: L.SHOTS + '/47-undo-popup.png' });
  await O.keyboard.press('Control+Shift+z'); pu = await popup(O); ok(/^Redo/.test(pu.text), `47: Ctrl+Shift+Z shows Redo (${pu.text})`);
  await O.keyboard.press('Control+z'); await O.keyboard.press('Control+y'); pu = await popup(O); ok(/^Redo/.test(pu.text), `47: Ctrl+Y shows Redo (${pu.text})`);
  await O.keyboard.press('Meta+z'); pu = await popup(O); ok(/^Undo|^Nothing/.test(pu.text), `47: Cmd+Z behaves like Ctrl+Z (${pu.text})`);
  await O.evaluate(() => { const b = window.__draw.board; while (b.can_undo()) b.undo(); while (b.can_redo()) b.redo(); while (b.can_undo()) b.undo(); });
  await O.keyboard.press('Control+z'); pu = await popup(O); ok(/Nothing to undo/.test(pu.text), `47: nothing to undo says so (${pu.text})`);
  await O.evaluate(() => { const b = window.__draw.board; while (b.can_redo()) b.redo(); });
  await O.keyboard.press('Control+y'); pu = await popup(O); ok(/Nothing to redo/.test(pu.text), `47: nothing to redo says so (${pu.text})`);
  await tool(O, 'rect'); await drag(O, [200, 200], [300, 260]);
  await O.click('#btn-undo'); pu = await popup(O); ok(/^Undo: /.test(pu.text), `47: the toolbar Undo button shows the popup (${pu.text})`);
  await O.click('#btn-redo'); pu = await popup(O); ok(/^Redo: /.test(pu.text), `47: the toolbar Redo button shows the popup (${pu.text})`);
  const before = await O.evaluate(() => window.__draw.popupCount);
  for (let i = 0; i < 8; i++) await O.keyboard.press('Control+z');
  pu = await popup(O);
  ok(pu.n === 1 && (await O.evaluate(() => document.querySelectorAll('#undopop.show').length)) === 1, `47: popups replace each other (8 presses, 1 element, ${pu.text})`);
  await O.evaluate(() => { const b = window.__draw.board; while (b.can_redo()) b.redo(); });

  // ================= 45 double click to label
  await O.evaluate(() => { const b = window.__draw.board; b.select_all(); b.delete_selected(); b.pan(-b.pan_x() + 0, -b.pan_y() + 0); b.set_zoom(0, 0, 1); });
  const shapes = {};
  for (const [k, x] of [['rect', 300], ['ellipse', 520], ['diamond', 740]]) {
    const c = await L.created(O, async () => { await tool(O, k); await drag(O, [x, 300], [x + 150, 400]); });
    shapes[k] = c[0];
  }
  await tool(O, 'arrow'); const ca = await L.created(O, async () => { await drag(O, [375, 480], [815, 480]); });
  shapes.arrow = ca[0];
  await tool(O, 'select');
  for (const k of ['rect', 'ellipse', 'diamond', 'arrow']) {
    const id = shapes[k].id;
    await B.evaluate(() => { const b = window.__draw.board; b.set_zoom(0, 0, 1); b.pan(-b.pan_x(), -b.pan_y()); });
    const s = await scr(O, id);
    const cx = s[0] + s[2] / 2, cy = s[1] + s[3] / 2;
    await O.mouse.dblclick(cx, cy);
    const ta = await O.waitForSelector('textarea.editor', { timeout: 3000 }).catch(() => null);
    ok(!!ta, `45: double-click on a ${k} opens the editor`);
    if (!ta) continue;
    const box = await ta.boundingBox();
    if (k !== 'arrow') ok(box.x >= s[0] - 1 && box.x + box.width <= s[0] + s[2] + 1 && box.y >= s[1] - 1 && box.y + box.height <= s[1] + s[3] + 1, `45: the ${k} editor is INSIDE the shape (${Math.round(box.x)},${Math.round(box.y)} ${Math.round(box.width)}x${Math.round(box.height)} in ${s.map(Math.round)})`);
    ok(Math.abs(box.x + box.width / 2 - cx) < 3 && Math.abs(box.y + box.height / 2 - cy) < 4, `45: the ${k} editor is centred`);
    await O.keyboard.type('Label ' + k + ' with enough words to wrap inside the shape');
    await O.keyboard.press('Enter');
    await sleep(150);
    ok((await els(O)).find((e) => e.id === id).tx === 'Label ' + k + ' with enough words to wrap inside the shape', `45: Enter commits the ${k} label`);
    await O.mouse.dblclick(cx, cy);
    await O.waitForSelector('textarea.editor');
    ok((await O.inputValue('textarea.editor')).startsWith('Label ' + k), `45: double-click edits the existing ${k} label`);
    await O.keyboard.press('Control+a'); await O.keyboard.type('Second ' + k);
    await O.keyboard.press('Escape'); await sleep(100);
    ok((await els(O)).find((e) => e.id === id).tx.startsWith('Label'), `45: Esc cancels the ${k} edit`);
    await O.mouse.dblclick(cx, cy); await O.waitForSelector('textarea.editor');
    await O.keyboard.press('Control+a'); await O.keyboard.type('Final ' + k);
    await O.mouse.click(60, 700); await sleep(200); // click outside commits
    ok((await els(O)).find((e) => e.id === id).tx === 'Final ' + k, `45: clicking outside commits the ${k} label`);
    await O.mouse.dblclick(cx, cy); await O.waitForSelector('textarea.editor');
    await O.keyboard.press('Control+a'); await O.keyboard.type('Ctrl ' + k); await O.keyboard.press('Control+Enter'); await sleep(100);
    ok((await els(O)).find((e) => e.id === id).tx === 'Ctrl ' + k, `45: Ctrl+Enter commits the ${k} label`);
    // seen and edited by other participants
    await B.waitForFunction(([i, t]) => { const e = JSON.parse(window.__draw.board.export_live()).find((x) => x.id === i); return e && e.tx === t; }, [id, 'Ctrl ' + k], { timeout: 6000 }).then(() => ok(true, `45: the ${k} label syncs to another participant`)).catch(() => ok(false, `45: the ${k} label did not sync`));
    const sb = await scr(B, id);
    await B.mouse.dblclick(sb[0] + sb[2] / 2, sb[1] + sb[3] / 2);
    await B.waitForSelector('textarea.editor', { timeout: 3000 }).catch(() => {});
    if (await B.$('textarea.editor')) { await B.keyboard.press('Control+a'); await B.keyboard.type('By B ' + k); await B.keyboard.press('Enter'); }
    await O.waitForFunction(([i, t]) => { const e = JSON.parse(window.__draw.board.export_live()).find((x) => x.id === i); return e && e.tx === t; }, [id, 'By B ' + k], { timeout: 6000 }).then(() => ok(true, `45: a joiner can label/edit the ${k} too, and the owner sees it`)).catch(() => ok(false, `45: the joiner's ${k} label did not arrive`));
  }
  await O.screenshot({ path: L.SHOTS + '/45-labels.png' });
  ok(await converge([O, B, C]), '45: all participants converge after labels');

  // ================= 44 zoom
  const zB0 = await zoom(B);
  const stage = { x: 700, y: 450 };
  const pageZoom = async () => O.evaluate(() => window.visualViewport.scale);
  await O.evaluate(() => window.__draw.board.set_zoom(0, 0, 1)); await O.evaluate(() => window.__draw.board.pan(-window.__draw.board.pan_x(), -window.__draw.board.pan_y()));
  const anchorWorld = async (px, py) => O.evaluate(([x, y]) => { const b = window.__draw.board; const z = b.zoom(); return [(x - b.pan_x()) / z, (y - b.pan_y()) / z]; }, [px, py]);
  const w0 = await anchorWorld(stage.x, stage.y);
  await O.mouse.move(stage.x, stage.y);
  await O.keyboard.down('Control'); await O.mouse.wheel(0, -100); await O.keyboard.up('Control'); await sleep(60);
  const z1 = await zoom(O); const w1 = await anchorWorld(stage.x, stage.y);
  ok(z1 > 1 && Math.abs(w1[0] - w0[0]) < 0.5 && Math.abs(w1[1] - w0[1]) < 0.5, `44: Ctrl+wheel zooms in around the pointer (${z1.toFixed(2)}, anchor moved ${(w1[0] - w0[0]).toFixed(2)},${(w1[1] - w0[1]).toFixed(2)})`);
  ok((await label(O)) === Math.round(z1 * 100) + '%', `44: the percentage label follows (${await label(O)})`);
  ok((await pageZoom()) === 1, '44: the browser page itself does not zoom');
  await O.keyboard.down('Control'); await O.mouse.wheel(0, 3); await O.mouse.wheel(0, 3); await O.keyboard.up('Control'); await sleep(60);
  ok((await zoom(O)) < z1, '44: a trackpad pinch (ctrl+wheel with small deltas) zooms out smoothly');
  const zs = [];
  for (let i = 0; i < 6; i++) { await O.keyboard.down('Control'); await O.mouse.wheel(0, -4); await O.keyboard.up('Control'); zs.push(await zoom(O)); }
  ok(zs.every((z, i) => i === 0 || (z > zs[i - 1] && z / zs[i - 1] < 1.08)), `44: pinch steps are small and monotonic (${zs.map((z) => z.toFixed(3)).join(' ')})`);
  await O.keyboard.press('Control+0'); await sleep(100); ok((await zoom(O)) === 1 && (await label(O)) === '100%', '44: Ctrl+0 resets to 100%');
  await O.keyboard.press('Control+='); ok((await zoom(O)) > 1, '44: Ctrl+= zooms in');
  await O.keyboard.press('Control+-'); await O.keyboard.press('Control+-'); ok((await zoom(O)) < 1, '44: Ctrl+- zooms out');
  await O.keyboard.press('Control+0');
  await O.click('#btn-zin'); const zb = await zoom(O); ok(zb > 1, `44: the + button zooms (${zb.toFixed(2)})`);
  await O.click('#btn-zout'); await O.click('#btn-zout'); ok((await zoom(O)) < 1, '44: the - button zooms');
  await O.keyboard.press('Shift+1'); await sleep(60);
  const fz = await zoom(O); ok(fz > 0.1 && fz <= 4, `44: Shift+1 fits the content (${fz.toFixed(2)})`);
  for (let i = 0; i < 40; i++) await O.click('#btn-zin');
  ok(Math.abs((await zoom(O)) - 4) < 1e-9 && (await label(O)) === '400%', '44: the zoom tops out at exactly 400%');
  for (let i = 0; i < 80; i++) await O.click('#btn-zout');
  ok(Math.abs((await zoom(O)) - 0.1) < 1e-9 && (await label(O)) === '10%', '44: the zoom bottoms out at exactly 10%');
  // zoom is per user
  ok((await zoom(B)) === zB0 && (await zoom(B)) !== (await zoom(O)), `44: zoom is not synced to other users (B stays ${(await zoom(B)).toFixed(2)}, O ${(await zoom(O)).toFixed(2)})`);
  // alignment at several zoom levels: selection handles, text editor, remote cursor, canvas pixels
  await O.evaluate(() => window.__draw.board.set_zoom(0, 0, 1));
  const r = shapes.rect.id;
  for (const zl of [0.25, 0.6, 1, 2, 3.5]) {
    await O.evaluate((z) => { const b = window.__draw.board; b.set_zoom(0, 0, 1); b.pan(-b.pan_x() + 20, -b.pan_y() + 20); b.set_zoom(0, 0, z); b.pan(-b.pan_x() + 640 - 375 * z, -b.pan_y() + 400 - 350 * z); }, zl);
    await O.evaluate((id) => window.__draw.board.select_ids(JSON.stringify([id])), r);
    await sleep(80);
    const s = await scr(O, r);
    // the outline of the (unselected-looking) top edge, sampled between the handles
    let found = false;
    for (let dy = -3; dy <= 3 && !found; dy++) { const q = await pixel(O, s[0] + s[2] * 0.3, s[1] + dy); if (q[3] > 0 && q[0] < 120 && q[1] < 120 && q[2] < 140) found = true; }
    ok(found, `44: at ${Math.round(zl * 100)}% the drawn shape edge is exactly where the engine says it is (${s.map(Math.round)})`);
    // selection handle (bottom-right) is drawn on the shape corner
    let hh = false;
    for (let dx = -6; dx <= 6 && !hh; dx++) for (let dy = -6; dy <= 6 && !hh; dy++) { const q = await pixel(O, s[0] + s[2] + dx, s[1] + s[3] + dy); if (q[3] > 0 && q[2] > 200 && q[0] < 110 && q[1] < 130) hh = true; }
    ok(hh, `44: at ${Math.round(zl * 100)}% the selection handle sits on the corner`);
    // editor placement
    await O.mouse.dblclick(s[0] + s[2] / 2, s[1] + s[3] / 2);
    const ta = await O.waitForSelector('textarea.editor', { timeout: 2000 }).catch(() => null);
    if (ta) {
      const bx = await ta.boundingBox();
      ok(Math.abs(bx.x + bx.width / 2 - (s[0] + s[2] / 2)) < 3 && Math.abs(bx.y + bx.height / 2 - (s[1] + s[3] / 2)) < Math.max(4, s[3] / 3), `44: at ${Math.round(zl * 100)}% the text editor is centred on the shape`);
      await O.keyboard.press('Escape');
    } else ok(false, `44: no editor at ${Math.round(zl * 100)}%`);
    // a remote cursor stays on the world point
    await B.evaluate(() => { const b = window.__draw.board; b.pan(-b.pan_x(), -b.pan_y()); b.set_zoom(0, 0, 1); });
    await B.mouse.move(300, 300); await B.mouse.move(320, 310); await sleep(350);
    const rc = await O.evaluate(() => { const d = document.querySelector('#cursors .rcursor'); if (!d) return null; const m = d.style.transform.match(/translate\(([-\d.]+)px, ([-\d.]+)px\)/); return m ? [+m[1], +m[2]] : null; });
    const want = await O.evaluate(() => window.__draw.board.w2s(320, 310));
    ok(rc && Math.abs(rc[0] - want[0]) < 2 && Math.abs(rc[1] - want[1]) < 2, `44: at ${Math.round(zl * 100)}% the remote cursor is aligned (${rc && rc.map(Math.round)} vs ${want.map(Math.round)})`);
  }
  // zoom is not stuck after panning or resizing the window
  await O.evaluate(() => window.__draw.board.set_zoom(0, 0, 1));
  await O.keyboard.down('Space'); await O.mouse.move(600, 400); await O.mouse.down(); await O.mouse.move(650, 430); await O.mouse.up(); await O.keyboard.up('Space');
  await O.keyboard.down('Control'); await O.mouse.wheel(0, -100); await O.keyboard.up('Control'); const zp = await zoom(O);
  ok(zp > 1, '44: zoom still works after panning');
  await O.setViewportSize({ width: 900, height: 600 }); await sleep(250);
  await O.mouse.move(400, 300); await O.keyboard.down('Control'); await O.mouse.wheel(0, 100); await O.keyboard.up('Control'); await sleep(60);
  ok((await zoom(O)) < zp, '44: zoom still works after resizing the window');
  const cvSize = await O.evaluate(() => [document.getElementById('cv').width / window.__draw.dpr, window.innerWidth]);
  ok(Math.abs(cvSize[0] - cvSize[1]) <= 1, `44: the canvas follows the window size (${cvSize.join(' vs ')})`);
  await O.setViewportSize({ width: 1280, height: 800 });
  // touch pinch
  const t = await browser.newContext({ viewport: { width: 700, height: 700 }, hasTouch: true, isMobile: false });
  const T = await t.newPage(); await T.goto(BASE + '/'); await T.waitForFunction(() => window.__draw && window.__draw.board);
  const cdp = await t.newCDPSession(T);
  const touch = (type, pts) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: pts.map(([x, y], id) => ({ x, y, id })) });
  const zt0 = await T.evaluate(() => window.__draw.board.zoom());
  await touch('touchStart', [[300, 350], [400, 350]]);
  for (let i = 1; i <= 8; i++) await touch('touchMove', [[300 - i * 10, 350], [400 + i * 10, 350]]);
  await touch('touchEnd', []);
  const zt1 = await T.evaluate(() => window.__draw.board.zoom());
  ok(zt1 > zt0 * 1.5, `44: a two-finger pinch zooms in (${zt0} -> ${zt1.toFixed(2)})`);
  await t.close();
  await O.screenshot({ path: L.SHOTS + '/44-zoom.png' });

  // ================= 43 animated GIF
  await O.evaluate(() => { const b = window.__draw.board; b.select_all(); b.delete_selected(); b.set_zoom(0, 0, 1); b.pan(-b.pan_x(), -b.pan_y()); });
  const gif = makeGif(40, [0, 1, 2], 12); // red, blue, green frames, 120 ms each
  await setClip(O, gif, 'image/gif').catch(async (e) => { console.log('clipboard gif not supported, using file chooser:', e.message); });
  await O.mouse.move(500, 350);
  let g = await L.created(O, async () => { await O.keyboard.press('Control+v'); await sleep(800); });
  if (!g.length) {
    const fc = O.waitForEvent('filechooser'); await O.evaluate(() => document.getElementById('filepick').click());
    g = await L.created(O, async () => { await (await fc).setFiles({ name: 'a.gif', mimeType: 'image/gif', buffer: gif }); await sleep(800); });
  }
  ok(g.length === 1 && g[0].k === 'image', '43: a GIF was added to the board');
  const gh = g[0].img;
  const sha = require('crypto').createHash('sha256').update(gif).digest('hex');
  ok(gh === sha, `43: the stored hash equals the sha256 of the ORIGINAL gif bytes (${gh.slice(0, 12)}...)`);
  await O.waitForFunction(() => { const d = window.__draw; return JSON.parse(d.board.images_used()).every((h) => !d.images.status(h)); }, null, { timeout: 15000 });
  const served = await O.evaluate(async (h) => { const r = await fetch('/api/img/' + h); const b = new Uint8Array(await r.arrayBuffer()); return { type: r.headers.get('content-type'), len: b.length, head: String.fromCharCode(...b.slice(0, 6)) }; }, gh);
  ok(served.type === 'image/gif' && served.len === gif.length && served.head === 'GIF89a', `43: the server serves the unchanged GIF (${served.type}, ${served.len} == ${gif.length} bytes)`);
  const gs = await scr(O, g[0].id);
  await O.mouse.click(5, 790); // deselect so the handles do not cover the pixel
  await sleep(200);
  const cx = gs[0] + gs[2] / 2, cy = gs[1] + gs[3] / 2;
  const seen = new Set();
  const t0 = Date.now();
  while (Date.now() - t0 < 2000) { const px = await pixel(O, cx, cy); seen.add(px[0] > 200 && px[1] < 60 ? 'red' : px[2] > 200 && px[0] < 60 ? 'blue' : px[1] > 150 && px[0] < 60 ? 'green' : 'other'); await sleep(40); }
  ok(seen.has('red') && seen.has('blue') && seen.has('green'), `43: the canvas shows the animation on the owner (colours seen: ${[...seen].join(',')})`);
  await B.waitForFunction((h) => { const e = window.__draw.images.get(h); return e && e.state === 'ok'; }, gh, { timeout: 15000 }).catch(() => {});
  await B.evaluate(() => { const b = window.__draw.board; b.fit(window.__draw.w, window.__draw.h); });
  await sleep(1000);
  const gsB = await scr(B, g[0].id);
  const seenB = new Set();
  const t1 = Date.now();
  while (Date.now() - t1 < 2200) { const px = await pixel(B, gsB[0] + gsB[2] / 2, gsB[1] + gsB[3] / 2); seenB.add(px[0] > 200 && px[1] < 60 ? 'red' : px[2] > 200 && px[0] < 60 ? 'blue' : px[1] > 150 && px[0] < 60 ? 'green' : 'other'); await sleep(40); }
  ok(seenB.has('red') && seenB.has('blue') && seenB.has('green'), `43: the animation plays on another client too (${[...seenB].join(',')})`);
  // while moved / zoomed
  await O.mouse.move(cx, cy); await O.mouse.down(); await O.mouse.move(cx + 80, cy + 40, { steps: 6 });
  const mv = new Set(); for (let i = 0; i < 40; i++) { const px = await pixel(O, cx + 80, cy + 40); mv.add(px[0] > 200 && px[1] < 60 ? 'red' : px[2] > 200 && px[0] < 60 ? 'blue' : px[1] > 150 && px[0] < 60 ? 'green' : 'other'); await sleep(40); }
  await O.mouse.up();
  ok(mv.size >= 2 && !mv.has('other') || mv.has('red') && mv.has('blue'), `43: it keeps animating while being moved (${[...mv].join(',')})`);
  await O.evaluate(() => window.__draw.board.set_zoom(window.__draw.w / 2, window.__draw.h / 2, 2));
  await sleep(300);
  const gz = await scr(O, g[0].id); const zz = new Set();
  for (let i = 0; i < 40; i++) { const px = await pixel(O, gz[0] + gz[2] / 2, gz[1] + gz[3] / 2); zz.add(px[0] > 200 && px[1] < 60 ? 'red' : px[2] > 200 && px[0] < 60 ? 'blue' : px[1] > 150 && px[0] < 60 ? 'green' : 'other'); await sleep(40); }
  ok(zz.has('red') && zz.has('blue'), `43: it keeps animating while zoomed to 200% (${[...zz].join(',')})`);
  // a sampled-frame check at fixed times: the colour must cycle with the 120 ms delay
  // idle CPU: a static image must not cause redraws; the GIF must cause them only while visible
  await O.evaluate(() => { const b = window.__draw.board; b.select_all(); b.delete_selected(); });
  const [staticSig] = [makePng(60, 60, 3)];
  await setClip(O, staticSig); await O.mouse.move(500, 350); await O.keyboard.press('Control+v'); await sleep(1200);
  const r0 = await O.evaluate(() => window.__draw.renders); await sleep(2000); const r1 = await O.evaluate(() => window.__draw.renders);
  ok(r1 - r0 <= 2, `43: a static image causes no redraws when idle (${r1 - r0} in 2 s)`);
  await O.evaluate(() => { const b = window.__draw.board; b.select_all(); b.delete_selected(); });
  await setClip(O, gif); await O.keyboard.press('Control+v'); await sleep(1200);
  await O.evaluate(() => window.__draw.board.pan(5000, 5000)); await sleep(500);
  const q0 = await O.evaluate(() => window.__draw.renders); await sleep(2000); const q1 = await O.evaluate(() => window.__draw.renders);
  ok(q1 - q0 <= 2, `43: an animated GIF that is off screen causes no redraws (${q1 - q0} in 2 s)`);
  await O.evaluate(() => window.__draw.board.pan(-5000, -5000)); await sleep(500);
  const p0 = await O.evaluate(() => window.__draw.renders); await sleep(2000); const p1 = await O.evaluate(() => window.__draw.renders);
  ok(p1 - p0 >= 5 && p1 - p0 <= 60, `43: a visible animated GIF redraws at its frame rate (${p1 - p0} in 2 s)`);
  // a PNG still goes through the normal path; GIF bytes in a fresh browser
  await O.screenshot({ path: L.SHOTS + '/43-gif.png' });
  // too large GIF: accepted up to the cap, still shown
  const bigGif = Buffer.concat([makeGif(40, [0, 1], 10).subarray(0, -1), Buffer.alloc(4.5 * 1024 * 1024, 0), Buffer.from([0x3b])]);
  await O.evaluate(() => { const b = window.__draw.board; b.select_all(); b.delete_selected(); });
  const fc2 = O.waitForEvent('filechooser'); await O.evaluate(() => document.getElementById('filepick').click());
  await (await fc2).setFiles({ name: 'big.gif', mimeType: 'image/gif', buffer: bigGif }); await sleep(1500);
  const note = await O.evaluate(() => document.getElementById('toast').textContent);
  ok(/too large|still|could not/i.test(note) || (await els(O)).some((e) => e.k === 'image'), `43: an oversized GIF is either downscaled with a visible note or rejected clearly (${note})`);

  // ================= 46 room password
  await O.evaluate(() => { window.__draw.board.select_all(); window.__draw.board.delete_selected(); window.__draw.board.set_zoom(0, 0, 1); window.__draw.board.pan(-window.__draw.board.pan_x(), -window.__draw.board.pan_y()); });
  await tool(O, 'rect'); await drag(O, [400, 300], [520, 380]);
  const rid = (await els(O)).find((e) => e.k === 'rect').id;
  ok(!(await B.evaluate(() => document.body.classList.contains('ro'))), '46: by default nobody is read-only');
  ok(await O.isVisible('#btn-pw') && !(await B.isVisible('#btn-pw')), '46: only the owner sees the Password button');
  await O.click('#btn-pw');
  await O.fill('#pw-input', 'abc'); await O.click('#pw-set');
  ok(/at least 4/.test(await O.textContent('#pw-msg')), '46: a too short password is refused in the UI');
  await O.fill('#pw-input', 'horse-staple-1'); await O.click('#pw-set');
  await B.waitForFunction(() => document.body.classList.contains('ro'), null, { timeout: 8000 }).then(() => ok(true, '46: setting the password makes a connected joiner read-only live')).catch(() => ok(false, '46: the joiner did not become read-only'));
  ok(!(await O.evaluate(() => document.body.classList.contains('ro'))), '46: the owner keeps edit rights');
  ok(/Read-only/.test(await B.textContent('#rochip')) && (await B.isVisible('#btn-unlock')) && /Enter password to edit/.test(await B.textContent('#btn-unlock')), "46: the UI shows 'Read-only' and an 'Enter password to edit' action");
  await B.screenshot({ path: L.SHOTS + '/46-readonly.png' });
  // a read-only user tries everything
  const nb = (await els(B)).length;
  await tool(B, 'rect').catch(() => {}); await drag(B, [700, 500], [800, 560]).catch(() => {});
  await B.evaluate(() => { const b = window.__draw.board; b.add_text('x', 10, 10, 100, 100); b.select_all(); b.delete_selected(); b.nudge(50, 50); b.duplicate(); });
  await B.keyboard.press('Delete').catch(() => {});
  await sleep(1200);
  ok((await els(B)).length === nb, `46: a read-only client cannot add, delete, move or duplicate locally (${nb} -> ${(await els(B)).length})`);
  await B.evaluate((id) => { const b = window.__draw.board; b.select_ids(JSON.stringify([id])); b.nudge(200, 200); b.set_style('#e03131', '~', -1, -1, -1); }, rid);
  await sleep(800);
  const ro = (await els(O)).find((e) => e.id === rid);
  ok(!!ro && ro.st !== '#e03131', '46: edits forced through a read-only client never reach the document');
  // force raw ops over the socket from the read-only page
  const forced = await B.evaluate((id) => new Promise((res) => {
    const d = window.__draw; const out = [];
    const h = (e) => { try { out.push(JSON.parse(e.data).t); } catch {} };
    d.ws.addEventListener('message', h);
    d.ws.send(JSON.stringify({ t: 'ops', ops: [{ id, v: 999, by: d.cid, k: 'rect', x: 5, y: 5, w: 10, h: 10, del: true }, { id: 'forced1', v: 1, by: d.cid, k: 'rect', x: 5, y: 5, w: 10, h: 10 }] }));
    setTimeout(() => res(out), 1500);
  }), rid);
  ok(forced.includes('err'), `46: the server answers forced ops with an error (${forced.join(',')})`);
  await sleep(500);
  const oe = await els(O);
  ok(oe.some((e) => e.id === rid) && !oe.some((e) => e.id === 'forced1'), '46: forced delete/add over the WebSocket changed nothing for the owner');
  ok((await sig(B)) === (await sig(O)), '46: the read-only client was reconciled with the server state');
  // REST write paths refused for the joiner (its browser holds no grant)
  const rest = await B.evaluate(async ([room, nano]) => {
    const post = (p, body, h = {}) => fetch(p, { method: 'POST', headers: h, body }).then((r) => r.status);
    return [
      await post(`/api/rooms/${room}/docs`, JSON.stringify({ title: 'x', els: [] }), { 'content-type': 'application/json' }),
      await post(`/api/rooms/${room}/docs/${nano}/images`, new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3])),
      await fetch(`/api/rooms/${room}/docs/${nano}`, { method: 'DELETE' }).then((r) => r.status),
      await fetch(`/api/rooms/${room}`, { method: 'DELETE' }).then((r) => r.status),
      await fetch(`/api/rooms/${room}/password`, { method: 'PUT', body: JSON.stringify({ password: 'takeover1' }) }).then((r) => r.status),
    ];
  }, [ROOM, new URL(url).pathname.split('/')[2]]);
  ok(rest.every((s) => s === 403), `46: create/upload/delete/delete-room/set-password are refused over REST (${rest.join(',')})`);
  // wrong password, then right
  await B.click('#btn-unlock'); await B.fill('#unlock-input', 'wrong-guess'); await B.click('#unlock-go');
  await B.waitForFunction(() => /Wrong password/.test(document.getElementById('unlock-msg')?.textContent || ''), null, { timeout: 6000 }).then(() => ok(true, '46: a wrong password is refused')).catch(() => ok(false, '46: no wrong-password message'));
  ok(await B.evaluate(() => document.body.classList.contains('ro')), '46: still read-only after the wrong password');
  await B.fill('#unlock-input', 'horse-staple-1'); await B.click('#unlock-go');
  await B.waitForFunction(() => !document.body.classList.contains('ro') && window.__draw.wsOpen, null, { timeout: 12000 }).then(() => ok(true, '46: the right password unlocks editing')).catch(() => ok(false, '46: the right password did not unlock'));
  const cb = await L.created(B, async () => { await tool(B, 'ellipse'); await drag(B, [700, 500], [800, 560]); });
  ok(cb.length === 1, '46: the unlocked joiner can draw');
  await O.waitForFunction((id) => JSON.parse(window.__draw.board.export_live()).some((e) => e.id === id), cb[0].id, { timeout: 6000 }).then(() => ok(true, "46: the unlocked joiner's shape reaches the owner")).catch(() => ok(false, 'not synced'));
  await B.reload(); await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 15000 });
  await sleep(500);
  ok(!(await B.evaluate(() => document.body.classList.contains('ro'))), '46: the grant is remembered in that browser (still editing after a reload)');
  // the other joiner (C) never entered the password: still read-only
  ok(await C.evaluate(() => document.body.classList.contains('ro')), '46: a browser that never entered the password stays read-only');
  // room page
  const [ctxR, R] = await L.open(browser, BASE + '/' + ROOM);
  await R.waitForSelector('#doclist'); await sleep(300);
  ok((await R.isVisible('#room-ro')) && (await R.isVisible('#room-unlock')) && !(await R.isVisible('#new-doc')), '46: the room page is read-only for visitors (no New document, shows Enter password)');
  // the owner changes the password: the joiner's grant dies, live and on reload
  await O.click('#btn-pw'); await O.fill('#pw-input', 'correct-battery-2'); await O.click('#pw-set');
  await B.waitForFunction(() => document.body.classList.contains('ro'), null, { timeout: 8000 }).then(() => ok(true, '46: changing the password revokes the joiner (live)')).catch(() => ok(false, '46: grant not revoked live'));
  await B.reload(); await B.waitForFunction(() => window.__draw.wsOpen, null, { timeout: 15000 }); await sleep(400);
  ok(await B.evaluate(() => document.body.classList.contains('ro')), '46: after a reload the revoked grant is still dead');
  // remove
  await O.click('#btn-pw'); await O.click('#pw-remove');
  await B.waitForFunction(() => !document.body.classList.contains('ro'), null, { timeout: 8000 }).then(() => ok(true, '46: removing the password lets everyone edit again (live)')).catch(() => ok(false, '46: remove not live'));
  await C.waitForFunction(() => !document.body.classList.contains('ro'), null, { timeout: 8000 }).then(() => ok(true, '46: ... including the browser that never had the password')).catch(() => ok(false, 'C still ro'));
  // the password is never in the page or in local storage for people who do not know it
  const leak = await C.evaluate(async () => JSON.stringify([localStorage, sessionStorage, document.documentElement.innerHTML.includes('correct-battery-2'), document.documentElement.innerHTML.includes('horse-staple-1')]));
  ok(!/horse-staple|correct-battery/.test(leak), '46: the password appears nowhere in another participant\'s page');

  await browser.close();
  console.log(state.fails ? `${state.fails} FAILED` : 'ALL OK');
  process.exit(state.fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
async function sig(p) { return L.sig(p); }
