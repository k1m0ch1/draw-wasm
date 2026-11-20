// PLAN items 49, 50, 51 against a deployed instance (API + WebSocket level; at most 17 sockets from one IP).
//   BASE=https://example.com node tests/capacity.js
const BASE = process.env.BASE;
let fails = 0;
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rnd = () => Math.random().toString(36).slice(2, 7);
const ROOM = 'zz-test-cap' + rnd();
const api = async (m, p, body, h = {}) => {
  for (let i = 0; i < 8; i++) {
    const r = await fetch(BASE + p, { method: m, headers: Buffer.isBuffer(body) ? h : { 'content-type': 'application/json', ...h }, body: body === undefined ? undefined : (Buffer.isBuffer(body) ? body : JSON.stringify(body)) });
    if (r.status !== 429) return r;
    await sleep(1500);
  }
  throw new Error('rate limited');
};
const rect = (id, by) => ({ id, v: 1, by, k: 'rect', x: 1, y: 1, w: 50, h: 50 });
function join(nano, cid, token = '') {
  return new Promise((res) => {
    const ws = new WebSocket(BASE.replace(/^http/, 'ws') + `/ws/${ROOM}/${nano}`);
    const c = { ws, msgs: [], role: null, edit: null, cid };
    ws.onmessage = (e) => { const m = JSON.parse(e.data); c.msgs.push(m); if (m.t === 'role') c.role = m; if (m.t === 'perm') c.edit = m.edit; };
    ws.onopen = () => ws.send(JSON.stringify({ t: 'hello', cid, name: 'T' + cid.slice(-3), token }));
    ws.onclose = () => { c.closed = true; };
    const t0 = Date.now();
    const iv = setInterval(() => { if (c.role || c.closed || Date.now() - t0 > 8000) { clearInterval(iv); res(c); } }, 50);
  });
}
const waitFor = async (f, ms = 12000) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { if (f()) return true; await sleep(100); } return false; };
const png = (n, size = 100) => Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(size, n)]);
const gif = (n, size) => Buffer.concat([Buffer.from('GIF89a'), Buffer.alloc(size - 6, n)]);
const writeOp = (c, id) => c.ws.send(JSON.stringify({ t: 'ops', ops: [rect(id, c.cid)] }));

(async () => {
  let r = await api('POST', '/api/rooms', { room: ROOM }); const tok = (await r.json()).token;
  r = await api('POST', `/api/rooms/${ROOM}/docs`, { title: 'cap', els: [] }, { 'x-owner-token': tok }); const nano = (await r.json()).nano;
  const all = [];

  // ---- 49: owner + 20 editors + a viewer
  const owner = await join(nano, 'cown' + rnd(), tok); all.push(owner);
  ok(owner.role && owner.role.editor && owner.role.owner, 'owner joins as editor without a slot');
  const eds = [];
  for (let i = 0; i < 20; i++) { const c = await join(nano, 'ced' + i + rnd()); eds.push(c); all.push(c); }
  ok(eds.every((c) => c.role && c.role.editor && c.edit), '20 joiners all become editors');
  ok(eds[19].role.cap === 20, 'role message carries the cap (20)');
  const v1 = await join(nano, 'cv1' + rnd()); all.push(v1);
  ok(v1.role && !v1.role.editor && v1.role.queued && v1.role.pos === 1 && v1.edit === false, `21st joiner is a viewer, queue position ${v1.role && v1.role.pos}`);
  // viewer receives live ops but cannot write
  writeOp(eds[0], 'e-live1');
  ok(await waitFor(() => v1.msgs.some((m) => m.t === 'ops' && m.ops.some((o) => o.id === 'e-live1'))), 'viewer receives live ops from an editor');
  writeOp(v1, 'v-illegal');
  await sleep(800);
  ok(!owner.msgs.some((m) => m.t === 'ops' && m.ops.some((o) => o.id === 'v-illegal' && !o.del)), 'viewer write is not broadcast to anyone');
  ok(v1.msgs.some((m) => m.t === 'err'), 'viewer is told its write was refused');
  r = await api('GET', `/api/rooms/${ROOM}/docs/${nano}`);
  ok(!JSON.stringify(await r.json()).includes('v-illegal'), 'viewer write never reached the stored document');
  // owner keeps editing while full
  writeOp(owner, 'o-1');
  ok(await waitFor(() => eds[1].msgs.some((m) => m.t === 'ops' && m.ops.some((o) => o.id === 'o-1'))), 'owner still edits while all slots are taken');
  // promotion: an editor leaves, the first viewer is promoted within a few seconds
  const gone = eds[3]; gone.ws.close();
  const t0 = Date.now();
  ok(await waitFor(() => v1.role && v1.role.editor && v1.edit === true, 12000), `first viewer promoted automatically (${Date.now() - t0} ms)`);
  ok((Date.now() - t0) < 10000, 'promotion happened within a few seconds');
  writeOp(v1, 'v-legal');
  ok(await waitFor(() => owner.msgs.some((m) => m.t === 'ops' && m.ops.some((o) => o.id === 'v-legal'))), 'the promoted viewer can write now');
  ok(v1.role.editors === 20, 'editors count stays at 20');
  // FIFO + reconnect grace: queue v2 and v3 behind full house; an editor drops and comes back (<60 s) ahead of them
  const v2 = await join(nano, 'cv2' + rnd()); all.push(v2);
  ok(v2.role && v2.role.queued && v2.role.pos === 1, `next viewer queued at position ${v2.role && v2.role.pos}`);
  for (const c of all) if (c.closed === undefined && c !== owner) { /* keep */ }
  const dropper = eds[5]; dropper.ws.close(); await sleep(300);
  const back = await join(nano, dropper.cid); all.push(back);
  ok(await waitFor(() => back.role && back.role.editor && back.edit, 8000), 'an editor reconnecting within the grace period reclaims a slot ahead of the queue');
  await sleep(1500);
  ok(v2.role.queued && !v2.edit, 'the waiting viewer was not promoted past the returning editor');
  // idle viewer position updates in place
  eds[7].ws.close();
  ok(await waitFor(() => v2.role.editor && v2.edit, 12000), 'second viewer promoted when the next slot frees up (no reload needed)');
  for (const c of all) { try { c.ws.close(); } catch { } }
  await sleep(1000);

  // ---- 49: a password room keeps its rules on top
  r = await api('PUT', `/api/rooms/${ROOM}/password`, { password: 'correct horse battery' }, { 'x-owner-token': tok });
  ok(r.status === 200, 'owner sets a password');
  const pw = await join(nano, 'cpw' + rnd());
  ok(pw.role && !pw.role.editor && pw.edit === false && !pw.role.queued, 'password room: a client without the grant is a read-only viewer, not queued');
  try { pw.ws.close(); } catch { }
  await api('PUT', `/api/rooms/${ROOM}/password`, { password: '' }, { 'x-owner-token': tok });

  // ---- 50: image cap
  r = await api('POST', `/api/rooms/${ROOM}/docs/${nano}/images`, png(1, 2 * 1024 * 1024 - 100), { 'x-owner-token': tok });
  ok(r.status === 200, `PNG just under 2 MB accepted (${r.status})`);
  r = await api('POST', `/api/rooms/${ROOM}/docs/${nano}/images`, png(2, 2 * 1024 * 1024 + 1), { 'x-owner-token': tok });
  const j = await r.json().catch(() => ({}));
  ok(r.status === 413 && /2 MB/.test(j.error || ''), `PNG over 2 MB -> ${r.status} "${j.error}"`);
  r = await api('POST', `/api/rooms/${ROOM}/docs/${nano}/images`, Buffer.concat([Buffer.from([0xff, 0xd8, 0xff]), Buffer.alloc(3 * 1024 * 1024)]), { 'x-owner-token': tok });
  ok(r.status === 413, `JPEG of 3 MB is refused (${r.status}): only GIFs get the larger cap`);
  // ---- 51: GIF cap
  r = await api('POST', `/api/rooms/${ROOM}/docs/${nano}/images`, gif(3, 2.9 * 1024 * 1024 | 0), { 'x-owner-token': tok });
  ok(r.status === 200, `GIF of 2.9 MB accepted, bytes kept (${r.status})`);
  if (r.status === 200) {
    const sha = (await r.json()).sha;
    const g = await fetch(BASE + '/api/img/' + sha);
    const b = Buffer.from(await g.arrayBuffer());
    ok(g.headers.get('content-type') === 'image/gif' && b.equals(gif(3, 2.9 * 1024 * 1024 | 0)), 'GIF served byte for byte as image/gif');
  }
  r = await api('POST', `/api/rooms/${ROOM}/docs/${nano}/images`, gif(4, 3 * 1024 * 1024 + 1), { 'x-owner-token': tok });
  const j2 = await r.json().catch(() => ({}));
  ok(r.status === 413 && /GIF/.test(j2.error || '') && /3 MB/.test(j2.error || ''), `GIF over 3 MB -> ${r.status} "${j2.error}"`);

  await api('DELETE', `/api/rooms/${ROOM}`, undefined, { 'x-owner-token': tok });
  console.log(fails ? `${fails} FAILED` : 'ALL PASSED');
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
