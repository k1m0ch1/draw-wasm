// Authorisation tests (PLAN item 48): IDOR, owner/grant isolation between rooms, read-only enforcement on
// REST and WebSocket, no client-supplied rights, admin not reachable, rate limits not bypassable by header spoofing.
// Each check FAILS on the vulnerable behaviour. BASE=https://example.com node tests/security.js
// Rooms are prefixed zz-test- (purge with `draw-server admin purge-prefix zz-test-`).
const BASE = process.env.BASE;
let fails = 0;
const ok = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rnd = () => Math.random().toString(36).slice(2, 7);
const A = 'zz-test-sa' + rnd(), B = 'zz-test-sb' + rnd();
let retry = true; // ordinary calls wait out the API rate limit; the throttling checks switch this off
const call = async (m, p, body, h = {}) => {
  for (let i = 0; ; i++) {
    const r = await raw(m, p, body, h);
    if (r.status !== 429 || !retry || i >= 6) return r;
    await sleep(1500);
  }
};
const raw = (m, p, body, h = {}) => fetch(BASE + p, { method: m, headers: { 'content-type': 'application/json', ...h }, body: body === undefined ? undefined : (Buffer.isBuffer(body) ? body : JSON.stringify(body)) });
const own = (t) => ({ 'x-owner-token': t });
const grantH = (g) => ({ 'x-edit-grant': g });
const png = (n) => Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(100, n)]);
const rect = (id, by, x = 1) => ({ id, v: 1, by, k: 'rect', x, y: 1, w: 50, h: 50 });

function wsOpen(room, nano, hello) {
  return new Promise((res) => {
    const ws = new WebSocket(BASE.replace(/^http/, 'ws') + `/ws/${room}/${nano}`);
    const msgs = [];
    ws.onmessage = (e) => msgs.push(JSON.parse(e.data));
    ws.onopen = () => { ws.send(JSON.stringify({ t: 'hello', ...hello })); setTimeout(() => res({ ws, msgs }), 700); };
    ws.onerror = () => res({ ws: null, msgs });
    ws.onclose = () => res({ ws: null, msgs });
  });
}
const last = (msgs, t) => [...msgs].reverse().find((m) => m.t === t);

(async () => {
  // ---- setup: two rooms with a document each
  let r = await call('POST', '/api/rooms', { room: A }); const tokA = (await r.json()).token;
  await sleep(300);
  r = await call('POST', '/api/rooms', { room: B }); const tokB = (await r.json()).token;
  ok(tokA && tokB && tokA !== tokB, 'two rooms claimed, distinct tokens');
  r = await call('POST', `/api/rooms/${A}/docs`, { title: 'A doc', els: [rect('a1', 'zz')] }, own(tokA)); const dA = (await r.json()).nano;
  r = await call('POST', `/api/rooms/${B}/docs`, { title: 'B doc', els: [rect('b1', 'zz')] }, own(tokB)); const dB = (await r.json()).nano;
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(1), own(tokA)); const shaA = (await r.json()).sha;
  ok(/^[a-f0-9]{64}$/.test(shaA), 'image uploaded to room A doc');

  // ---- (a) IDOR
  r = await call('GET', `/api/rooms/${B}/docs/${dA}`); ok(r.status === 404, `GET doc of A through room B -> ${r.status} (must be 404)`);
  r = await call('POST', `/api/rooms/${B}/docs/${dA}/images`, png(2), own(tokB)); ok(r.status === 404, `upload image into A's doc through room B -> ${r.status} (must be 404)`);
  r = await call('DELETE', `/api/rooms/${B}/docs/${dA}`, undefined, own(tokB)); ok(r.status === 404, `delete A's doc through room B with B's owner token -> ${r.status} (must be 404)`);
  r = await call('GET', `/api/rooms/${A}/docs/${dA}`); ok(r.status === 200, "A's doc survived the cross-room delete");
  const ws0 = await wsOpen(B, dA, { cid: 'sec-idor', name: 'Idor' });
  ok(!last(ws0.msgs, 'snap') && !!last(ws0.msgs, 'err'), 'WebSocket: joining doc of A while claiming room B is refused');
  try { ws0.ws?.close(); } catch {}
  r = await call('GET', `/api/rooms/${A}`); let txt = await r.text();
  ok(r.status === 200 && !/[a-f0-9]{64}/.test(txt) && !/owner_hash|pw_hash|token/.test(txt), 'room listing exposes no image hashes, hashes or tokens');
  r = await call('GET', `/api/rooms/${A}/docs/${dA}`); txt = await r.text();
  ok(!/pw_hash|owner_hash|argon2/.test(txt), 'doc response carries no secrets');
  r = await call('GET', '/api/img/' + '0'.repeat(64)); ok(r.status === 404, 'unknown image hash -> 404 (no oracle beyond existence)');
  r = await call('GET', '/api/img'); ok(r.status >= 400 && r.status < 500, `no image index/listing endpoint (${r.status})`);

  // ---- (b) owner token of A has no power in B
  r = await call('DELETE', `/api/rooms/${B}`, undefined, own(tokA)); ok(r.status === 403, `A's owner token cannot delete room B -> ${r.status}`);
  r = await call('PUT', `/api/rooms/${B}/password`, { password: 'hijack-pw' }, own(tokA)); ok(r.status === 403, `A's owner token cannot set a password on B -> ${r.status}`);
  r = await call('GET', `/api/rooms/${B}`, undefined, own(tokA)); ok((await r.json()).owner === false, 'A owner token is not reported as owner of B');
  r = await call('PUT', `/api/rooms/${B}/password`, { password: 'nopenope' }); ok(r.status === 403, `password change without any token -> ${r.status}`);
  r = await call('PUT', `/api/rooms/${B}/password`, { password: 'nopenope', owner: true }, { 'x-owner-token': 'z'.repeat(48) }); ok(r.status === 403, 'client-asserted owner flag + forged token ignored');
  const wsO = await wsOpen(B, dB, { cid: 'sec-own1', name: 'Mallory', token: tokA, owner: true, edit: true });
  ok(last(wsO.msgs, 'snap') && last(wsO.msgs, 'snap').you.owner === false, "WebSocket: A's owner token + owner:true in hello does not make an owner of B");
  try { wsO.ws?.close(); } catch {}

  // ---- 46 basics + (c): password, grants
  await sleep(1500);
  r = await call('PUT', `/api/rooms/${A}/password`, { password: 'pw-alpha-1' }, own(tokA)); ok(r.status === 200, 'owner of A sets the password');
  r = await call('PUT', `/api/rooms/${B}/password`, { password: 'pw-beta-22' }, own(tokB)); ok(r.status === 200, 'owner of B sets the password');
  r = await call('PUT', `/api/rooms/${A}/password`, { password: 'abc' }, own(tokA)); ok(r.status === 422, 'too short a password rejected');
  r = await call('GET', `/api/rooms/${A}`); let j = await r.json(); ok(j.pw === true && j.edit === false && j.owner === false, 'room reports pw=true, edit=false for anonymous');
  r = await call('GET', `/api/rooms/${A}`, undefined, own(tokA)); j = await r.json(); ok(j.edit === true && j.owner === true, 'owner keeps edit rights');
  r = await call('POST', `/api/rooms/${A}/unlock`, { password: 'wrong-pw-1' }); ok(r.status === 401, 'wrong password -> 401');
  r = await call('POST', `/api/rooms/${A}/unlock`, { password: 'pw-alpha-1' }); j = await r.json();
  const gA = j.grant; ok(r.status === 200 && /^[a-f0-9]{48}$/.test(gA), 'correct password issues a 48-hex grant');
  ok(!JSON.stringify(j).includes('pw-alpha-1'), 'the password is never echoed');
  // every write path refuses without the grant
  r = await call('POST', `/api/rooms/${A}/docs`, { title: 'x', els: [] }); ok(r.status === 403, `create doc read-only -> ${r.status}`);
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(3)); ok(r.status === 403, `upload read-only -> ${r.status}`);
  r = await call('DELETE', `/api/rooms/${A}/docs/${dA}`); ok(r.status === 403, `delete doc read-only -> ${r.status}`);
  r = await call('DELETE', `/api/rooms/${A}`); ok(r.status === 403, `delete room read-only -> ${r.status}`);
  r = await call('GET', `/api/rooms/${A}/docs/${dA}`); ok(r.status === 200, 'reading stays open when read-only');
  // grant of A in room B / on B's doc
  r = await call('POST', `/api/rooms/${B}/docs`, { title: 'x', els: [] }, grantH(gA)); ok(r.status === 403, `grant of A cannot create docs in B -> ${r.status}`);
  r = await call('POST', `/api/rooms/${B}/docs/${dB}/images`, png(4), grantH(gA)); ok(r.status === 403, `grant of A cannot upload into B -> ${r.status}`);
  r = await call('DELETE', `/api/rooms/${B}/docs/${dB}`, undefined, grantH(gA)); ok(r.status === 403, `grant of A cannot delete B's doc -> ${r.status}`);
  r = await call('GET', `/api/rooms/${B}`, undefined, grantH(gA)); ok((await r.json()).edit === false, 'grant of A gives edit=false in B');
  r = await call('POST', `/api/rooms/${B}/docs/${dA}/images`, png(5), grantH(gA)); ok(r.status === 404 || r.status === 403, `grant of A + wrong room/doc pair -> ${r.status}`);
  // grant works in A
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(6), grantH(gA)); ok(r.status === 200, 'grant of A can upload into A');
  r = await call('DELETE', `/api/rooms/${A}`, undefined, grantH(gA)); ok(r.status === 403, 'a grant cannot delete the room (owner only)');
  r = await call('PUT', `/api/rooms/${A}/password`, { password: 'grant-takeover' }, grantH(gA)); ok(r.status === 403, 'a grant cannot change the password (owner only)');
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(7), grantH('f'.repeat(48))); ok(r.status === 403, 'a forged grant is refused');
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(7), grantH("x' OR '1'='1")); ok(r.status === 403, 'an injection-looking grant is refused');

  // ---- (d) WebSocket rights
  const ro = await wsOpen(A, dA, { cid: 'sec-ro1', name: 'Reader' });
  ok(last(ro.msgs, 'perm') && last(ro.msgs, 'perm').edit === false && last(ro.msgs, 'perm').pw === true, 'WebSocket: no grant -> perm edit=false');
  const peerId = last(ro.msgs, 'snap').you.id;
  ok(peerId !== 'sec-ro1' && /^p[a-z0-9]{11}$/.test(peerId), `participant id is server-assigned (${peerId}), not the client's cid`);
  ro.ws.send(JSON.stringify({ t: 'ops', ops: [rect('evil1', 'sec-ro1', 5), { id: 'a1', v: 9, by: 'sec-ro1', k: 'rect', x: 999, y: 1, w: 5, h: 5, del: true }] })); await sleep(800);
  ok(!!last(ro.msgs, 'err') && /read-only/.test(last(ro.msgs, 'err').msg), 'WebSocket: read-only ops are rejected with a message');
  r = await call('GET', `/api/rooms/${A}/docs/${dA}`); j = await r.json();
  ok(!j.els.some((e) => e.id === 'evil1' && !e.del) && j.els.some((e) => e.id === 'a1' && !e.del && e.x === 1), 'document unchanged by the read-only client (add refused, delete/edit of a1 refused)');
  const edr = await wsOpen(A, dA, { cid: 'sec-ed1', name: 'Editor', grant: gA });
  ok(last(edr.msgs, 'perm') && last(edr.msgs, 'perm').edit === true, 'WebSocket: valid grant -> perm edit=true');
  edr.ws.send(JSON.stringify({ t: 'ops', ops: [rect('good1', 'sec-ed1', 7)] })); await sleep(800);
  r = await call('GET', `/api/rooms/${A}/docs/${dA}`); j = await r.json(); ok(j.els.some((e) => e.id === 'good1' && !e.del), 'WebSocket: editor with grant can write');
  ok(ro.msgs.some((m) => m.t === 'ops' && m.ops.some((e) => e.id === 'good1')), 'the read-only client still receives live updates');
  const spoof = await wsOpen(A, dA, { cid: 'sec-sp1', name: 'Spoofer', grant: gA, owner: true });
  ok(last(spoof.msgs, 'snap').you.owner === false, 'WebSocket: hello cannot claim to be the owner');
  const own1 = await wsOpen(A, dA, { cid: 'sec-ow1', name: '', token: tokA });
  const youO = last(own1.msgs, 'snap').you;
  ok(youO.owner === true && /^\p{Extended_Pictographic}/u.test(youO.name), `owner without a name gets a face emoji (${youO.name})`);
  // escalation by reconnecting with another doc's room: A grant on B's websocket
  const cross = await wsOpen(B, dB, { cid: 'sec-cr1', name: 'Cross', grant: gA });
  ok(last(cross.msgs, 'perm') && last(cross.msgs, 'perm').edit === false, "WebSocket: A's grant gives no edit rights in room B");
  // password change revokes grants, live sockets lose edit rights, old grant dead on REST
  r = await call('PUT', `/api/rooms/${A}/password`, { password: 'pw-alpha-2' }, own(tokA)); ok(r.status === 200, 'owner changes the password');
  await sleep(3500);
  ok(last(edr.msgs, 'perm').edit === false, 'WebSocket: a live editor is demoted to read-only when the password changes');
  edr.ws.send(JSON.stringify({ t: 'ops', ops: [rect('late1', 'sec-ed1', 9)] })); await sleep(700);
  r = await call('GET', `/api/rooms/${A}/docs/${dA}`); j = await r.json(); ok(!j.els.some((e) => e.id === 'late1' && !e.del), 'demoted client cannot write any more');
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(8), grantH(gA)); ok(r.status === 403, 'revoked grant refused on REST');
  const again = await wsOpen(A, dA, { cid: 'sec-ed2', name: 'Editor', grant: gA });
  ok(last(again.msgs, 'perm').edit === false, 'reconnecting with the revoked grant stays read-only');
  r = await call('PUT', `/api/rooms/${A}/password`, { password: '' }, own(tokA)); ok(r.status === 200, 'owner removes the password');
  await sleep(3000);
  ok(last(ro.msgs, 'perm').edit === true, 'removing the password lets the read-only socket edit again');
  r = await call('POST', `/api/rooms/${A}/docs/${dA}/images`, png(9)); ok(r.status === 200, 'no password: anyone may upload again');
  for (const w of [ro, edr, spoof, own1, cross, again]) { try { w.ws?.close(); } catch {} }

  // ---- (e) admin is not reachable
  for (const [m, p] of [['GET', '/admin'], ['POST', '/admin/purge-prefix'], ['POST', '/api/admin/purge'], ['GET', '/api/admin/stats'], ['POST', '/api/rooms/purge'], ['DELETE', '/api/rooms']]) {
    r = await call(m, p, m === 'GET' ? undefined : { prefix: 'zz-test-' }); ok(r.status >= 400 && r.status < 500 && r.status !== 401, `${m} ${p} -> ${r.status} (no admin surface)`);
  }
  r = await call('GET', `/api/rooms/${A}`); ok(r.status === 200, 'rooms still exist after admin probes');

  // ---- (f) spoofing proxy headers does not bypass the limits
  await sleep(5000);
  const spoofed = await Promise.all(Array.from({ length: 260 }, (_, i) => fetch(BASE + `/api/rooms/${A}`, { headers: { 'x-real-ip': `10.${i % 250}.${(i * 7) % 250}.9`, 'x-forwarded-for': `203.0.113.${i % 250}`, 'x-client-ip': `198.51.100.${i % 250}`, forwarded: `for=192.0.2.${i % 250}` } })));
  const n429 = spoofed.filter((x) => x.status === 429).length;
  ok(n429 > 0, `rate limit still triggers while rotating X-Real-IP / X-Forwarded-For (${n429} x 429 of 260)`);
  await sleep(8000);
  // password guessing from "many addresses": the per-room bucket still stops it
  retry = false;
  let locked = 0;
  for (let i = 0; i < 16; i++) {
    r = await call('POST', `/api/rooms/${B}/unlock`, { password: 'guess-' + i }, { 'x-real-ip': `10.9.${i}.1`, 'x-forwarded-for': `203.0.113.${i}` });
    if (r.status === 429) locked++;
    if (r.status === 200) ok(false, 'a wrong guess must never unlock');
  }
  ok(locked > 0, `password guessing is throttled with rotating headers (${locked} of 16 guesses got 429)`);
  r = await call('POST', `/api/rooms/${B}/unlock`, { password: 'pw-beta-22' }); ok(r.status === 429 || r.status === 200, 'right password is also throttled during a lockout (no oracle)');

  // ---- cleanup
  await sleep(2000);
  await call('DELETE', `/api/rooms/${A}`, undefined, own(tokA));
  await call('DELETE', `/api/rooms/${B}`, undefined, own(tokB));
  console.log(fails ? `${fails} FAILED` : 'ALL OK');
  process.exit(fails ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
