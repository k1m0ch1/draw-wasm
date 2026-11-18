// API / WebSocket abuse and ownership checks. BASE=https://example.com node tests/abuse.js
const BASE = process.env.BASE;
const WebSocketImpl = globalThis.WebSocket || require('ws');
let fails = 0;
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const R = 'zz-test-' + Math.random().toString(36).slice(2, 7);
const j = (m, p, body, h = {}) => fetch(BASE + p, { method: m, headers: { 'content-type': 'application/json', ...h }, body: body === undefined ? undefined : (Buffer.isBuffer(body) ? body : JSON.stringify(body)) });
const png = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(100, 1)]);
(async () => {
  let r = await j('POST', '/api/rooms', { room: R });
  const tok = (await r.json()).token;
  ok(r.status === 200 && tok, 'claim room');
  r = await j('POST', '/api/rooms', { room: R }); ok(r.status === 409, 'second claim rejected 409');
  for (const bad of ['api', 'ws', 'admin', 'healthz', '../x', 'ab', 'A-B-C', '-abc', 'abc-', 'caf\u00e9x', 'a'.repeat(33), 'a b c', '%2e%2e']) {
    r = await j('POST', '/api/rooms', { room: bad }); ok(r.status === 422, `invalid/reserved room "${bad.slice(0, 12)}" -> ${r.status}`);
  }
  await sleep(200);
  r = await j('POST', `/api/rooms/${R}/docs`, { title: 'x', els: [] }, { 'x-owner-token': tok }); const d = await r.json();
  ok(r.status === 200 && /^[A-Za-z0-9_-]{10}$/.test(d.nano), 'create doc, nano ' + d.nano);
  const base = `/api/rooms/${R}/docs/${d.nano}`;
  // ownership
  r = await j('DELETE', base); ok(r.status === 403, 'delete without token -> 403');
  r = await j('DELETE', base, undefined, { 'x-owner-token': 'x'.repeat(48) }); ok(r.status === 403, 'delete with wrong token -> 403');
  r = await j('DELETE', `/api/rooms/${R}`); ok(r.status === 403, 'delete room without token -> 403');
  r = await fetch(BASE + base); ok(r.status === 200, 'doc still exists');
  // odd ids
  for (const p of ['/api/rooms/..%2F..%2Fetc/docs/abcdefghij', '/api/rooms/' + 'a'.repeat(300), '/api/rooms/%E2%98%83%E2%98%83%E2%98%83', `/api/rooms/${R}/docs/short`, '/api/img/zzz', '/assets/..%2f..%2fetc%2fpasswd', '/%2e%2e/%2e%2e/etc/passwd']) {
    r = await fetch(BASE + p); ok(r.status >= 400 && r.status < 500, `odd path ${p.slice(0, 40)} -> ${r.status}`);
  }
  // images
  r = await j('POST', base + '/images', Buffer.from('<script>alert(1)</script>')); ok(r.status === 415, 'non-image bytes rejected 415');
  r = await fetch(BASE + base + '/images', { method: 'POST', headers: { 'content-type': 'image/png' }, body: Buffer.from('GIF not really, html <b>') }); ok(r.status === 415, 'image mime with non-image bytes rejected');
  r = await fetch(BASE + base + '/images', { method: 'POST', body: Buffer.concat([png, Buffer.alloc(5 * 1024 * 1024)]) }); ok(r.status === 413, 'oversized image -> ' + r.status);
  r = await fetch(BASE + base + '/images', { method: 'POST', body: png }); const im = await r.json(); ok(r.status === 200 && im.sha, 'valid image accepted');
  r = await fetch(BASE + '/api/img/' + im.sha); console.log(r.status, r.headers.get('content-type'), r.headers.get('x-content-type-options')); ok(r.headers.get('content-type') === 'image/png' && (r.headers.get('x-content-type-options') || '').includes('nosniff'), 'image served with sniffed type + nosniff');
  // malformed / oversized docs
  r = await j('POST', `/api/rooms/${R}/docs`, Buffer.from('{not json')); ok(r.status === 400, 'malformed json -> 400');
  r = await j('POST', `/api/rooms/${R}/docs`, { els: [{ id: 'a', v: 1, by: 'b', k: 'rect', x: 'no' }] }); ok(r.status === 400 || r.status === 422, 'bad element -> ' + r.status);
  const big = Array.from({ length: 6000 }, (_, i) => ({ id: 'e' + i, v: 1, by: 'zz', k: 'rect', x: 1, y: 1, w: 1, h: 1 }));
  r = await j('POST', `/api/rooms/${R}/docs`, { els: big }); ok(r.status === 413, 'too many elements -> ' + r.status);
  r = await j('POST', `/api/rooms/${R}/docs`, { els: [{ id: 'a', v: 1, by: 'b', k: 'text', tx: 'x'.repeat(19000) }, ...Array.from({ length: 300 }, (_, i) => ({ id: 't' + i, v: 1, by: 'b', k: 'text', tx: 'y'.repeat(19000) }))] });
  ok(r.status === 413, 'oversized document -> ' + r.status);
  r = await j('POST', `/api/rooms/${R}/docs`, Buffer.alloc(6 * 1024 * 1024, 32)); ok(r.status === 413, 'huge body -> ' + r.status);
  // error body leaks
  r = await fetch(BASE + '/api/nope'); const t = await r.text(); ok(r.status === 404 && !/panic|rusqlite|src\/|\.rs/.test(t), 'error body clean: ' + t.slice(0, 60));
  // websocket abuse
  const origin = BASE;
  const open = (path) => new Promise((res) => { const ws = new WebSocketImpl(BASE.replace('http', 'ws') + path, { headers: { origin } }); ws.onopen = () => res(ws); ws.onerror = () => res(null); ws.onclose = () => res(null); });
  const ws = await open(`/ws/${R}/${d.nano}`);
  ok(!!ws, 'ws connects');
  const msgs = []; ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
  ws.send(JSON.stringify({ t: 'hello', cid: 'abuse1' })); await sleep(500);
  ok(msgs[0] && msgs[0].t === 'snap', 'snapshot on hello');
  ws.send('{broken'); await sleep(200); ok(msgs.some((m) => m.t === 'err'), 'malformed ws json -> err, still open');
  ws.send(JSON.stringify({ t: 'ops', ops: [{ id: 'x1', v: 1, by: 'abuse1', k: 'rect', x: 1e30 }] })); await sleep(200);
  ok(msgs.filter((m) => m.t === 'err').length >= 2, 'invalid op rejected');
  ws.send(JSON.stringify({ t: 'ops', ops: [{ id: 'x1', v: 1, by: 'abuse1', k: 'rect', x: 1, y: 1, w: 10, h: 10, st: '<script>' }] })); await sleep(200);
  ok(msgs.filter((m) => m.t === 'err').length >= 3, 'bad colour string rejected');
  let closed = false; ws.onclose = () => { closed = true; };
  ws.send('x'.repeat(2 * 1024 * 1024)); await sleep(800);
  ok(closed, 'oversized frame closes the socket');
  const ws2 = await open(`/ws/${R}/${d.nano}`); ws2.onmessage = (e) => msgs.push(JSON.parse(e.data));
  ws2.send(JSON.stringify({ t: 'hello', cid: 'abuse2' })); await sleep(300);
  const before = msgs.filter((m) => m.t === 'err').length;
  for (let i = 0; i < 300; i++) ws2.send(JSON.stringify({ t: 'ops', ops: [{ id: 'f' + i, v: 1, by: 'abuse2', k: 'rect', x: 1, y: 1, w: 5, h: 5 }] }));
  await sleep(1500);
  ok(msgs.filter((m) => m.t === 'err' && /rate/.test(m.msg)).length > 0 || ws2.readyState > 1, 'ws message rate limit triggers');
  ws2.close();
  r = await fetch(BASE + '/ws/' + R + '/short'); ok(r.status >= 400 && r.status < 500, 'ws bad doc id 4xx ' + r.status);
  // non-existent doc ws
  const ws3 = await open(`/ws/${R}/AAAAAAAAAA`); let em = []; if (ws3) { ws3.onmessage = (e) => em.push(JSON.parse(e.data)); ws3.send(JSON.stringify({ t: 'hello', cid: 'q1' })); await sleep(500); }
  ok(!ws3 || em.some((m) => m.t === 'err'), 'ws to unknown doc refused');
  // API rate limit
  const rs = await Promise.all(Array.from({ length: 200 }, () => fetch(BASE + `/api/rooms/${R}`))); const n429 = rs.filter((x) => x.status === 429).length;
  ok(n429 > 0, `API rate limit triggers (${n429} x 429 of 200)`);
  // health after abuse
  await sleep(1500);
  r = await fetch(BASE + '/healthz'); ok(r.status === 200, 'server healthy after abuse');
  // owner delete works
  await sleep(6000);
  r = await j('DELETE', base, undefined, { 'x-owner-token': tok }); ok(r.status === 200, 'owner can delete doc');
  r = await fetch(BASE + base); ok(r.status === 404, 'doc gone');
  r = await j('DELETE', `/api/rooms/${R}`, undefined, { 'x-owner-token': tok }); ok(r.status === 200, 'owner can delete room');
  console.log(fails ? `${fails} FAILED` : 'ALL OK');
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
