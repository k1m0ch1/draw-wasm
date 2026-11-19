// draw-wasm web shell: DOM events, toolbar, text overlay, clipboard, IndexedDB, WebSocket.
import init, { Board } from './engine.js';
import { Gfx, FONT } from './gfx.js';
import { idb } from './db.js';
import { Images, prepareImage } from './images.js';

const $ = (id) => document.getElementById(id);
const el = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') e.className = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v);
  }
  for (const k of kids) e.append(k);
  return e;
};
const rnd = (n) => {
  const a = new Uint8Array(n);
  crypto.getRandomValues(a);
  const al = 'abcdefghijklmnopqrstuvwxyz0123456789';
  return [...a].map((b) => al[b % 36]).join('');
};

const CLIP_PREFIX = 'draw-wasm:clip:v1:';

// ---------------------------------------------------------------- routing
const parts = location.pathname.split('/').filter(Boolean);
const route = parts.length === 0 ? { kind: 'local' }
  : parts.length === 1 ? { kind: 'room', room: parts[0] }
  : { kind: 'doc', room: parts[0], nano: parts[1] };

// ---------------------------------------------------------------- state
const S = {
  board: null, gfx: null, images: new Images(), cid: 'c' + rnd(11),
  canvas: $('cv'), w: 0, h: 0, dpr: 1,
  dirty: false, saving: false, idleMs: 60000, idleTimer: null, lastDocRev: 0, lastSelRev: -1,
  ws: null, wsOpen: false, wsTry: 0, you: null, peers: new Map(), cursors: new Map(),
  seq: 0, owner: false, token: '', name: '', needName: false, title: '', lastPing: 0, stopped: false,
  spaceDown: false, pointers: new Map(), pan: null, pinch: null, editing: null,
  lastCursorSent: 0, netWrites: 0, mouse: null,
  build: document.querySelector('meta[name=draw-build]')?.content || '', reloadNeeded: false,
  edit: true, pw: false, grant: '',
};
window.__draw = S; // inspection hook for tests; contains no secrets

function toast(msg, ms = 3500) {
  const t = $('toast');
  t.textContent = msg;
  t.classList.add('show');
  clearTimeout(toast.t);
  toast.t = setTimeout(() => t.classList.remove('show'), ms);
}
function setStatus(s) { $('status').textContent = s; }
/** One short popup at the top of the screen; a new one replaces the previous one. */
function popup(msg) {
  const t = $('undopop');
  t.textContent = msg;
  t.classList.add('show');
  S.popupCount = (S.popupCount || 0) + 1;
  clearTimeout(popup.t);
  popup.t = setTimeout(() => t.classList.remove('show'), 1400);
}
function doUndo() {
  const l = S.board.undo();
  popup(l ? 'Undo: ' + l : 'Nothing to undo');
}
function doRedo() {
  const l = S.board.redo();
  popup(l ? 'Redo: ' + l : 'Nothing to redo');
}

// ---------------------------------------------------------------- icons
const ICON = {
  select: '<path d="M5 3l14 8-6 2-3 6z"/>',
  rect: '<rect x="4" y="6" width="16" height="12" rx="3"/>',
  ellipse: '<ellipse cx="12" cy="12" rx="8" ry="6"/>',
  diamond: '<path d="M12 4l8 8-8 8-8-8z"/>',
  arrow: '<path d="M5 19L19 5M10 5h9v9"/>',
  pencil: '<path d="M4 20l1-4L16 5l3 3L8 19z"/>',
  text: '<path d="M6 6h12M12 6v13M9 19h6"/>',
  image: '<rect x="4" y="5" width="16" height="14" rx="2"/><circle cx="9" cy="10" r="1.5"/><path d="M5 18l5-5 4 4 2-2 3 3"/>',
  undo: '<path d="M9 8H5V4M5 8a8 8 0 1 1-1 7"/>',
  redo: '<path d="M15 8h4V4M19 8a8 8 0 1 0 1 7"/>',
  fit: '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
};
const svg = (n) => `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICON[n]}</svg>`;

const TOOLS = [
  ['select', 'Select (V)'], ['rect', 'Rectangle (R)'], ['ellipse', 'Ellipse (O)'], ['diamond', 'Diamond (D)'],
  ['arrow', 'Arrow (A)'], ['pencil', 'Pencil (P)'], ['text', 'Text (T)'], ['image', 'Image (I)'],
];

function buildToolbar() {
  const tb = $('toolbar');
  for (const [id, title] of TOOLS) {
    const b = el('button', { 'data-tool': id, title, 'aria-label': title.replace(/ \(.\)/, '') });
    b.innerHTML = svg(id);
    b.addEventListener('click', () => {
      if (id === 'image') { $('filepick').click(); return; }
      setTool(id);
    });
    tb.append(b);
  }
  $('btn-undo').innerHTML = svg('undo');
  $('btn-redo').innerHTML = svg('redo');
  $('btn-fit').innerHTML = svg('fit');
}
function setTool(t) {
  commitEdit();
  S.board.set_tool(t);
  for (const b of document.querySelectorAll('#toolbar button')) b.classList.toggle('active', b.dataset.tool === t);
  S.board.invalidate();
}

// ---------------------------------------------------------------- style panel
const STROKES = ['#1f2937', '#e03131', '#2f9e44', '#1c7ed6', '#f08c00', '#ae3ec9', '#868e96'];
const FILLS = ['none', '#ffffff', '#dbeafe', '#d3f9d8', '#fff3bf', '#ffe3e3', '#f3d9fa', '#e9ecef'];
function swatch(colour, onclick, label) {
  const b = el('button', { class: 'sw', title: label, 'aria-label': label, 'data-colour': colour });
  if (colour === 'none') b.classList.add('none'); else b.style.background = colour;
  b.addEventListener('click', onclick);
  return b;
}
function buildStyle() {
  const p = $('style');
  const sec = (title, ...kids) => { const d = el('div', { class: 'sec' }, el('div', { class: 'sechead' }, title), el('div', { class: 'row' }, ...kids)); p.append(d); return d; };
  sec('Stroke', ...STROKES.map((c) => swatch(c, () => S.board.set_style(c, '~', -1, -1, -1), 'Stroke ' + c)));
  sec('Fill', ...FILLS.map((c) => swatch(c, () => S.board.set_style('', c, -1, -1, -1), 'Fill ' + c)));
  const wb = [1, 2, 4, 6].map((w) => {
    const b = el('button', { class: 'chip', title: 'Stroke width ' + w, 'data-sw': w });
    b.innerHTML = `<svg width="26" height="14"><line x1="3" y1="7" x2="23" y2="7" stroke="currentColor" stroke-width="${w}" stroke-linecap="round"/></svg>`;
    b.addEventListener('click', () => S.board.set_style('', '~', w, -1, -1));
    return b;
  });
  sec('Width', ...wb);
  const heads = [['none', 0], ['end', 1], ['both', 3]].map(([n, v]) => {
    const b = el('button', { class: 'chip', title: 'Arrowheads: ' + n, 'data-ah': v }, n);
    b.addEventListener('click', () => S.board.set_style('', '~', -1, v, -1));
    return b;
  });
  sec('Arrow', ...heads);
  const sizes = [['S', 14], ['M', 18], ['L', 26], ['XL', 38]].map(([n, v]) => {
    const b = el('button', { class: 'chip', title: 'Text size ' + v, 'data-fs': v }, n);
    b.addEventListener('click', () => S.board.set_style('', '~', -1, -1, v));
    return b;
  });
  sec('Text', ...sizes);
  const act = (label, fn, cls = '') => { const b = el('button', { class: 'chip wide ' + cls }, label); b.addEventListener('click', fn); return b; };
  sec('Arrange',
    act('Front', () => S.board.z_front()), act('Forward', () => S.board.z_up()),
    act('Backward', () => S.board.z_down()), act('Back', () => S.board.z_back()),
    act('Duplicate', () => S.board.duplicate()), act('Delete', () => S.board.delete_selected(), 'danger'));
}
function syncStyle() {
  const info = JSON.parse(S.board.style_info());
  const p = $('style');
  p.classList.toggle('hasSel', info.n > 0);
  const mark = (sel, pred) => { for (const b of p.querySelectorAll(sel)) b.classList.toggle('on', pred(b)); };
  mark('[data-colour]', (b) => {
    const row = b.parentElement.previousSibling.textContent;
    return row === 'Stroke' ? b.dataset.colour === info.st : (info.fl || 'none') === b.dataset.colour;
  });
  mark('[data-sw]', (b) => Number(b.dataset.sw) === info.sw);
  mark('[data-ah]', (b) => Number(b.dataset.ah) === info.ah);
  mark('[data-fs]', (b) => Number(b.dataset.fs) === info.fs);
  const k = info.kinds || [];
  const only = (...ks) => k.length > 0 && k.every((x) => ks.includes(x));
  const rows = p.querySelectorAll('.sec');
  const vis = (i, v) => rows[i].classList.toggle('hidden', !v);
  const none = info.n === 0;
  vis(0, none || !only('image'));
  vis(1, none || k.some((x) => ['rect', 'ellipse', 'diamond'].includes(x)));
  vis(2, none || k.some((x) => !['text', 'image'].includes(x)));
  vis(3, none || k.includes('arrow'));
  vis(4, none || k.some((x) => ['rect', 'ellipse', 'diamond', 'text'].includes(x)));
  vis(5, info.n > 0);
}

// ---------------------------------------------------------------- sizing + render loop
function resize() {
  const r = $('stage').getBoundingClientRect();
  S.w = Math.max(1, Math.floor(r.width));
  S.h = Math.max(1, Math.floor(r.height));
  S.dpr = Math.min(window.devicePixelRatio || 1, 3);
  S.canvas.width = Math.round(S.w * S.dpr);
  S.canvas.height = Math.round(S.h * S.dpr);
  S.canvas.style.width = S.w + 'px';
  S.canvas.style.height = S.h + 'px';
  S.board?.invalidate();
}
function frame() {
  const b = S.board;
  if (b) {
    const dr = b.doc_rev();
    if (dr !== S.lastDocRev) { S.lastDocRev = dr; onDocChanged(); }
    const sr = b.sel_rev();
    if (sr !== S.lastSelRev) { S.lastSelRev = sr; syncStyle(); }
    const er = b.take_edit_request();
    if (er) startEdit(er);
    if (b.take_render()) {
      S.renders = (S.renders || 0) + 1;
      b.render(S.gfx, S.w, S.h, S.dpr);
      positionCursors();
      $('zlabel').textContent = Math.round(b.zoom() * 100) + '%';
      $('btn-undo').classList.toggle('dim', !b.can_undo());
      $('btn-redo').classList.toggle('dim', !b.can_redo());
      for (const t of document.querySelectorAll('#toolbar button')) t.classList.toggle('active', t.dataset.tool === b.tool());
      if (S.editing) layoutEditor();
    } else if (S.gfx.animNext <= performance.now()) {
      b.invalidate(); // an animated GIF is on screen and its next frame is due
    }
    S.canvas.style.cursor = S.spaceDown ? (S.pan ? 'grabbing' : 'grab') : b.cursor();
    flushOps();
  }
  requestAnimationFrame(frame);
}
let lastSent = 0;
function flushOps() {
  if (route.kind !== 'doc' || !S.wsOpen || S.reloadNeeded || !S.board.has_ops()) return;
  if (!S.edit) { S.board.take_ops(); return; } // read-only: nothing to send (the server would refuse it anyway)
  const now = performance.now();
  if (now - lastSent < 40) return;
  lastSent = now;
  const ops = JSON.parse(S.board.take_ops());
  for (let i = 0; i < ops.length; i += 200) wsSend({ t: 'ops', ops: ops.slice(i, i + 200) });
}

// ---------------------------------------------------------------- local persistence
function onDocChanged() {
  if (route.kind === 'local') {
    S.dirty = true;
    setStatus('Unsaved changes');
    clearTimeout(S.idleTimer);
    S.idleTimer = setTimeout(() => saveLocal('auto'), S.idleMs);
  }
  scanImages();
}
async function saveLocal(why) {
  if (route.kind !== 'local' || S.saving) return;
  if (!S.dirty && why !== 'manual') return;
  S.saving = true;
  clearTimeout(S.idleTimer);
  try {
    const json = S.board.export_live();
    S.dirty = false;
    await idb.put('docs', 'local', { json, saved: Date.now(), v: 1 });
    S.lastSaveReason = why;
    S.saveCount = (S.saveCount || 0) + 1;
    setStatus('Saved in this browser' + (why === 'auto' ? ' (autosave)' : ''));
  } catch (e) {
    S.dirty = true;
    setStatus('Could not save locally');
    toast('Saving in this browser failed: ' + (e && e.message || e));
  } finally { S.saving = false; }
}
async function loadLocal() {
  try {
    const d = await idb.get('docs', 'local');
    if (d && d.json) {
      S.board.load(d.json);
      S.board.fit(S.w, S.h);
      setStatus('Loaded from this browser');
    }
  } catch { /* private mode etc: start empty */ }
  S.lastDocRev = S.board.doc_rev();
  S.dirty = false;
}

// ---------------------------------------------------------------- images
const pendingImages = new Set();
function scanImages() {
  // make sure every referenced image is loadable (the Gfx lazily loads on draw)
  for (const h of JSON.parse(S.board.images_used())) pendingImages.add(h);
}
const imgUrl = (h) => `/api/rooms/${route.room}/docs/${route.nano}/images`;
/** Request headers that carry this browser's rights (the server decides what they are worth). */
async function authHeaders(room) {
  const h = {};
  const t = await token(room);
  if (t) h['x-owner-token'] = t;
  const g = await idb.get('meta', 'grant:' + room).catch(() => null);
  if (g) h['x-edit-grant'] = g;
  return h;
}
/** Make sure the server holds these images for this document (background, retried, shown on the element). */
function uploadImages(hashes) {
  if (route.kind !== 'doc' || !S.edit) return;
  for (const h of hashes) S.images.upload(h, imgUrl(h));
}
function imageHashesIn(json) {
  try { return [...new Set(JSON.parse(json).filter((e) => e && e.k === 'image' && e.img).map((e) => e.img))]; } catch { return []; }
}
/** The element appears at once; the upload runs in the background and its state is drawn on the element. */
async function addImageFile(file, at) {
  try {
    if (!file.type.startsWith('image/')) { toast('That file is not an image'); return; }
    const img = await prepareImage(file);
    await idb.put('images', img.hash, img.blob).catch(() => {});
    S.images.set(img.hash, img.bmp, img.animated ? img.blob : null);
    if (img.note) toast(img.note, 6000);
    const p = at || S.mouse || [S.w / 2, S.h / 2];
    S.board.add_image(img.hash, img.w, img.h, p[0], p[1]);
    S.board.invalidate();
    uploadImages([img.hash]);
  } catch (e) { toast('Could not add the image: ' + (e && e.message || e), 8000); }
}

// ---------------------------------------------------------------- text editing overlay
function startEdit(id) {
  commitEdit();
  const info = S.board.edit_info(id);
  if (!info) return;
  const e = JSON.parse(info);
  const ta = el('textarea', { class: 'editor', spellcheck: 'false', rows: '1', 'aria-label': 'Edit text' });
  ta.value = e.tx || '';
  $('editlayer').append(ta);
  S.editing = { id, ta, k: e.k, fs: e.fs || 18, st: e.st };
  S.board.set_editing(id === 'pending' ? '' : id);
  ta.addEventListener('input', () => layoutEditor());
  ta.addEventListener('keydown', (ev) => {
    ev.stopPropagation();
    if (ev.key === 'Escape') { ev.preventDefault(); cancelEdit(); }
    else if (ev.key === 'Enter' && !ev.shiftKey) { ev.preventDefault(); commitEdit(); } // Shift+Enter: new line
  });
  ta.addEventListener('blur', () => setTimeout(() => { if (S.editing && S.editing.ta === ta) commitEdit(); }, 0));
  layoutEditor();
  ta.focus();
  ta.select();
}
function textSize(text, fs) {
  const lines = text.split('\n');
  let w = 0;
  for (const l of lines) w = Math.max(w, S.gfx.measure(l, fs));
  return [w, lines.length * fs * 1.3];
}
function layoutEditor() {
  const ed = S.editing;
  if (!ed) return;
  const e = JSON.parse(S.board.edit_info(ed.id) || '{}');
  const z = S.board.zoom();
  const fs = (e.fs || ed.fs) * z;
  const s = S.board.w2s(e.x, e.y);
  const ta = ed.ta;
  ta.style.font = `${fs}px ${FONT}`;
  ta.style.color = e.st || '#1f2937';
  if (ed.k === 'text') {
    const [tw, th] = textSize(ta.value || ' ', e.fs || ed.fs);
    ta.style.textAlign = 'left';
    ta.style.left = s[0] + 'px';
    ta.style.top = s[1] + 'px';
    ta.style.width = Math.max(40, tw * z + 24) + 'px';
    ta.style.height = Math.max(th * z, fs * 1.3) + 'px';
  } else {
    // inside the shape: centred and wrapped to the same width the engine uses when it draws the label
    ta.style.textAlign = 'center';
    ta.style.whiteSpace = 'pre-wrap';
    ta.style.overflowWrap = 'anywhere';
    const w = e.w * z, h = e.h * z;
    const inner = ed.k === 'arrow' ? 180 * z : (e.k === 'rect' ? e.w - 20 : e.w * 0.62) * z;
    const bw = Math.max(56, inner);
    ta.style.width = bw + 'px';
    ta.style.height = '0px';
    const th = Math.max(ta.scrollHeight, fs * 1.3);
    ta.style.height = th + 'px';
    ta.style.left = s[0] + (w - bw) / 2 + 'px';
    ta.style.top = s[1] + (h - th) / 2 + 'px';
  }
}
function cancelEdit() {
  const ed = S.editing;
  if (!ed) return;
  S.editing = null;
  ed.ta.remove();
  S.board.set_editing('');
  S.board.cancel_text();
  S.board.invalidate();
  S.canvas.focus({ preventScroll: true });
}
function commitEdit() {
  const ed = S.editing;
  if (!ed) return;
  S.editing = null;
  const text = ed.ta.value;
  const [w, h] = textSize(text, ed.fs);
  ed.ta.remove();
  S.board.set_editing('');
  S.board.commit_text(ed.id, text, w, h);
  S.board.invalidate();
  S.canvas.focus({ preventScroll: true });
}

// ---------------------------------------------------------------- pointer input
function localPos(ev) {
  const r = S.canvas.getBoundingClientRect();
  return [ev.clientX - r.left, ev.clientY - r.top];
}
function bindInput() {
  const cv = S.canvas;
  cv.addEventListener('contextmenu', (e) => e.preventDefault());
  cv.addEventListener('mousedown', (e) => { if (e.button === 1) e.preventDefault(); }); // no autoscroll cursor
  cv.addEventListener('auxclick', (e) => e.preventDefault());
  cv.addEventListener('pointerleave', (ev) => { if (ev.pointerType === 'mouse') S.mouse = null; });
  cv.addEventListener('pointerdown', (ev) => {
    cv.focus({ preventScroll: true });
    commitEdit();
    cv.setPointerCapture(ev.pointerId);
    const p = localPos(ev);
    S.pointers.set(ev.pointerId, p);
    if (S.pointers.size === 2) { // pinch / two-finger pan
      S.board.cancel();
      const [a, b] = [...S.pointers.values()];
      S.pinch = { d: Math.hypot(a[0] - b[0], a[1] - b[1]), c: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2] };
      S.pan = null;
      return;
    }
    if (S.pointers.size > 2) return;
    if (ev.button === 1 || (ev.button === 0 && S.spaceDown)) {
      S.board.cancel();
      S.pan = { last: p };
      ev.preventDefault();
      return;
    }
    if (ev.button !== 0 && ev.pointerType === 'mouse') return;
    // double-click detection (pointer events do not give dblclick reliably on touch)
    const now = performance.now();
    const dbl = S.lastDown && now - S.lastDown.t < 350 && Math.hypot(p[0] - S.lastDown.p[0], p[1] - S.lastDown.p[1]) < 6;
    S.lastDown = { t: now, p };
    if (dbl && S.board.tool() === 'select') {
      S.board.double_click(p[0], p[1]);
      S.lastDown = null;
      return;
    }
    S.board.pointer_down(p[0], p[1], ev.shiftKey);
    syncToolButtons();
  });
  cv.addEventListener('pointermove', (ev) => {
    const p = localPos(ev);
    if (ev.pointerType === 'mouse') S.mouse = p;
    if (S.pointers.has(ev.pointerId)) S.pointers.set(ev.pointerId, p);
    if (S.pinch && S.pointers.size >= 2) {
      const [a, b] = [...S.pointers.values()];
      const d = Math.hypot(a[0] - b[0], a[1] - b[1]);
      const c = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      S.board.pan(c[0] - S.pinch.c[0], c[1] - S.pinch.c[1]);
      if (S.pinch.d > 0 && d > 0) S.board.zoom_at(c[0], c[1], d / S.pinch.d);
      S.pinch = { d, c };
      return;
    }
    if (S.pan) {
      S.board.pan(p[0] - S.pan.last[0], p[1] - S.pan.last[1]);
      S.pan.last = p;
      return;
    }
    S.board.pointer_move(p[0], p[1]);
    sendCursor(p);
  });
  const up = (ev) => {
    const p = localPos(ev);
    const had = S.pointers.delete(ev.pointerId);
    if (S.pinch) { if (S.pointers.size < 2) S.pinch = null; return; }
    if (S.pan) { S.pan = null; return; }
    if (!had) return;
    S.board.pointer_up(p[0], p[1]);
    syncToolButtons();
  };
  cv.addEventListener('pointerup', up);
  cv.addEventListener('pointercancel', (ev) => { S.pointers.delete(ev.pointerId); S.pinch = null; S.pan = null; S.board.cancel(); });
  cv.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    commitEdit();
    const p = localPos(ev);
    let dy = ev.deltaY, dx = ev.deltaX;
    if (ev.deltaMode === 1) { dy *= 16; dx *= 16; }
    if (ev.ctrlKey) { S.board.zoom_at(p[0], p[1], Math.exp(-dy * (Math.abs(dy) >= 40 ? 0.002 : 0.01))); return; } // pinch on a touchpad / ctrl+wheel
    if (ev.shiftKey && dx === 0) { S.board.pan(-dy, 0); return; }
    // A touchpad two-finger scroll sends small / fractional / horizontal deltas: pan. A mouse wheel
    // sends big whole steps: zoom to the cursor.
    const now = performance.now();
    const padLike = ev.deltaMode === 0 && (dx !== 0 || !Number.isInteger(dy) || Math.abs(dy) < 40);
    if (padLike) S.padAt = now;
    if (padLike || (S.padAt && now - S.padAt < 120)) { S.board.pan(-dx, -dy); return; }
    S.board.zoom_at(p[0], p[1], Math.exp(-dy * 0.0015));
  }, { passive: false });

  window.addEventListener('wheel', (ev) => { if (ev.ctrlKey) ev.preventDefault(); }, { passive: false });
  for (const g of ['gesturestart', 'gesturechange', 'gestureend']) document.addEventListener(g, (ev) => ev.preventDefault());
  $('stage').addEventListener('dragover', (e) => { e.preventDefault(); $('dropmask').classList.remove('hidden'); });
  $('stage').addEventListener('dragleave', () => $('dropmask').classList.add('hidden'));
  $('stage').addEventListener('drop', async (e) => {
    e.preventDefault();
    $('dropmask').classList.add('hidden');
    const files = [...(e.dataTransfer?.files || [])].filter((f) => f.type.startsWith('image/'));
    if (!files.length) { toast('Drop an image file'); return; }
    const p = localPos(e);
    for (const [i, f] of files.entries()) await addImageFile(f, [p[0] + i * 24, p[1] + i * 24]);
  });
  // Copy/cut put the selected elements on the SYSTEM clipboard (replacing whatever was there).
  const typing = () => S.editing || isTextField(document.activeElement) || !$('modal').classList.contains('hidden');
  const copyEvt = (cut) => (e) => {
    if (typing() || !S.board) return;
    const j = S.board.copy_json();
    if (!j) return;
    e.preventDefault();
    e.clipboardData.setData('text/plain', CLIP_PREFIX + j);
    if (cut) S.board.delete_selected();
    toast(cut ? 'Cut' : 'Copied', 900);
  };
  document.addEventListener('copy', copyEvt(false));
  document.addEventListener('cut', copyEvt(true));
  document.addEventListener('paste', async (e) => {
    if (typing()) return;
    const cd = e.clipboardData;
    const text = cd?.getData('text/plain') || '';
    if (text.startsWith(CLIP_PREFIX)) { // elements copied from this app (any document or tab)
      e.preventDefault();
      const j = text.slice(CLIP_PREFIX.length);
      const at = S.mouse || [NaN, NaN];
      S.board.paste_json(j, at[0], at[1]);
      uploadImages(imageHashesIn(j)); // the bytes may only be in another document or on the server
      return;
    }
    const files = [...(cd?.items || [])].filter((i) => i.kind === 'file' && i.type.startsWith('image/'));
    if (files.length) {
      e.preventDefault();
      let n = 0;
      for (const it of files) { const f = it.getAsFile(); if (f) await addImageFile(f, S.mouse ? [S.mouse[0] + 24 * n, S.mouse[1] + 24 * n] : null); n++; }
    } else if (text.trim()) { // plain text from elsewhere becomes a text label
      e.preventDefault();
      const t = text.slice(0, 2000);
      const [w, h] = textSize(t, S.board.default_fs());
      const at = S.mouse || [S.w / 2, S.h / 2];
      S.board.add_text(t, w, h, at[0], at[1]);
    }
  });
  $('filepick').addEventListener('change', async (e) => {
    for (const f of e.target.files) await addImageFile(f);
    e.target.value = '';
  });
  // Space is handled in the capture phase so no focused control can swallow it or be activated by it.
  window.addEventListener('keydown', onSpaceDown, true);
  window.addEventListener('keyup', onSpaceUp, true);
  window.addEventListener('keydown', onKey);
  window.addEventListener('blur', () => { S.spaceDown = false; S.pan = null; });
  // buttons never keep keyboard focus after a pointer click (keyboard users keep normal focus)
  document.addEventListener('click', (e) => {
    const b = e.target.closest && e.target.closest('button');
    if (b && e.detail > 0 && !$('modal').contains(b)) b.blur();
  }, true);
}
function isTextField(t) {
  if (!t || !t.tagName) return false;
  if (t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable) return true;
  return t.tagName === 'INPUT' && !/^(button|checkbox|radio|file|range|submit|reset|color|image)$/i.test(t.type || 'text');
}
function spaceBlocked() { return !!S.editing || isTextField(document.activeElement) || !$('modal').classList.contains('hidden'); }
function onSpaceDown(e) {
  if (e.code !== 'Space' || !S.board || isTextField(e.target) || spaceBlocked()) return;
  e.preventDefault();
  e.stopPropagation();
  const ae = document.activeElement;
  if (ae && ae !== document.body && ae !== S.canvas && ae.blur) ae.blur(); // never activate a focused button
  S.spaceDown = true;
  // Space pressed while the mouse is already down: switch to panning from the current position
  if (!S.pan && !S.pinch && S.pointers.size === 1) {
    S.board.cancel();
    S.pan = { last: [...S.pointers.values()][0] };
  }
}
function onSpaceUp(e) {
  if (e.code !== 'Space') return;
  if (S.spaceDown) { e.preventDefault(); e.stopPropagation(); }
  S.spaceDown = false;
}
function syncToolButtons() { /* the frame loop mirrors board.tool() into the toolbar */ }

function onKey(e) {
  if (S.editing || isTextField(e.target) || !$('modal').classList.contains('hidden')) return;
  const b = S.board;
  const mod = e.ctrlKey || e.metaKey;
  const k = e.key.toLowerCase();
  if (e.code === 'Space') return;
  if (mod) {
    if (k === 'z') { e.preventDefault(); e.shiftKey ? doRedo() : doUndo(); }
    else if (k === 'y') { e.preventDefault(); doRedo(); }
    else if (k === 'a') { e.preventDefault(); b.select_all(); }
    else if (k === 'd') { e.preventDefault(); b.duplicate(); }
    // c / x / v are handled by the copy, cut and paste events (system clipboard)
    else if (k === 's') { e.preventDefault(); doSave(); }
    else if (k === '0') { e.preventDefault(); b.set_zoom(S.w / 2, S.h / 2, 1); }
    else if (k === '=' || k === '+') { e.preventDefault(); b.zoom_at(S.w / 2, S.h / 2, 1.25); }
    else if (k === '-' || k === '_') { e.preventDefault(); b.zoom_at(S.w / 2, S.h / 2, 0.8); }
    return;
  }
  const tools = { v: 'select', r: 'rect', o: 'ellipse', d: 'diamond', a: 'arrow', p: 'pencil', t: 'text' };
  if (tools[k] && !e.altKey) { e.preventDefault(); setTool(tools[k]); return; }
  if (k === 'i') { $('filepick').click(); return; }
  if (k === 'delete' || k === 'backspace') { e.preventDefault(); b.delete_selected(); return; }
  if (k === 'escape') { b.cancel(); b.deselect(); setTool('select'); return; }
  if (k === ']') { e.shiftKey ? b.z_front() : b.z_up(); return; }
  if (k === '[') { e.shiftKey ? b.z_back() : b.z_down(); return; }
  if (e.key === '+' || e.key === '=') { b.zoom_at(S.w / 2, S.h / 2, 1.2); return; }
  if (e.key === '-') { b.zoom_at(S.w / 2, S.h / 2, 1 / 1.2); return; }
  if (e.key === '1' && e.shiftKey || e.key === '!') { b.fit(S.w, S.h); return; }
  const step = e.shiftKey ? 10 : 1;
  const arrows = { arrowleft: [-step, 0], arrowright: [step, 0], arrowup: [0, -step], arrowdown: [0, step] };
  if (arrows[k]) { e.preventDefault(); b.nudge(...arrows[k]); }
}

// ---------------------------------------------------------------- zoom bar / buttons
function bindButtons() {
  $('btn-zin').onclick = () => S.board.zoom_at(S.w / 2, S.h / 2, 1.25);
  $('btn-zout').onclick = () => S.board.zoom_at(S.w / 2, S.h / 2, 0.8);
  $('zlabel').onclick = () => S.board.set_zoom(S.w / 2, S.h / 2, 1);
  $('btn-fit').onclick = () => S.board.fit(S.w, S.h);
  $('btn-undo').onclick = () => doUndo();
  $('btn-redo').onclick = () => doRedo();
  $('btn-save').onclick = () => doSave();
  $('btn-cloud').onclick = () => cloudDialog();
  $('btn-share').onclick = () => shareDialog();
  $('btn-name').onclick = () => nameDialog(false);
  $('btn-pw').onclick = () => passwordDialog();
  $('btn-unlock').onclick = () => unlockDialog();
  $('btn-del').onclick = () => deleteDocDialog();
  $('btn-style').onclick = () => $('style').classList.toggle('open');
  window.addEventListener('resize', resize);
  window.addEventListener('pagehide', () => { if (route.kind === 'local' && S.dirty) saveLocal('hide'); });
  document.addEventListener('visibilitychange', () => { if (document.hidden && route.kind === 'local' && S.dirty) saveLocal('hide'); });
}
async function doSave() {
  if (route.kind === 'local') { await saveLocal('manual'); toast('Saved in this browser (private, no share link)'); }
  else if (route.kind === 'doc') { wsSend({ t: 'save' }); setStatus('Saving...'); }
}

// ---------------------------------------------------------------- modal helpers
function modal(content) {
  const m = $('modal');
  m.replaceChildren(el('div', { class: 'card', role: 'dialog', 'aria-modal': 'true' }, content));
  m.classList.remove('hidden');
  m.onclick = (e) => { if (e.target === m) closeModal(); };
  const first = m.querySelector('input,button');
  first?.focus();
}
function closeModal() { $('modal').classList.add('hidden'); $('modal').replaceChildren(); S.canvas?.focus({ preventScroll: true }); }

const token = async (room) => idb.get('meta', 'tok:' + room).catch(() => null);
const randomRoom = () => {
  const a = ['sunny', 'brave', 'calm', 'swift', 'lucky', 'merry', 'quiet', 'witty'];
  const n = ['otter', 'falcon', 'panda', 'lynx', 'heron', 'koala', 'tiger', 'gecko'];
  const r = new Uint8Array(3); crypto.getRandomValues(r);
  return `${a[r[0] % a.length]}-${n[r[1] % n.length]}-${rnd(3)}`;
};
async function api(method, path, body, room) {
  const headers = {};
  if (room) Object.assign(headers, await authHeaders(room));
  let payload;
  if (body instanceof Blob) payload = body;
  else if (body !== undefined) { headers['content-type'] = 'application/json'; payload = JSON.stringify(body); }
  if (method !== 'GET') S.netWrites++;
  const r = await fetch(path, { method, headers, body: payload });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) { const e = new Error(j.error || `Request failed (${r.status})`); e.status = r.status; throw e; }
  return j;
}

// ---------------------------------------------------------------- cloud upload
function cloudDialog() {
  const room = el('input', { id: 'cloud-room', type: 'text', value: randomRoom(), maxlength: '32', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'Room ID' });
  const title = el('input', { id: 'cloud-title', type: 'text', value: 'Untitled', maxlength: '80', 'aria-label': 'Document title' });
  const yname = el('input', { id: 'cloud-name', type: 'text', value: S.name || '', placeholder: 'optional', maxlength: '32', autocomplete: 'off', 'aria-label': 'Your name' });
  const msg = el('div', { class: 'msg', id: 'cloud-msg' });
  const go = el('button', { class: 'pill primary', id: 'cloud-go' }, 'Make public and store');
  const cancel = el('button', { class: 'pill', id: 'cloud-cancel' }, 'Cancel');
  const dice = el('button', { class: 'chip', title: 'Suggest another', type: 'button' }, 'New suggestion');
  dice.onclick = () => { room.value = randomRoom(); };
  cancel.onclick = closeModal;
  go.onclick = async () => {
    go.disabled = true;
    msg.textContent = '';
    const nm = yname.value.trim().replace(/\s+/g, ' ');
    if (nm && !validName(nm)) { msg.textContent = 'Your name must be up to 32 letters, digits, spaces, - _ . or an apostrophe (or one smiley)'; go.disabled = false; return; }
    if (nm) await idb.put('meta', 'name', nm).catch(() => {});
    try { await cloudStore(room.value.trim(), title.value.trim(), msg); }
    catch (e) { msg.textContent = e.message || 'Something went wrong'; go.disabled = false; }
  };
  modal(el('div', {},
    el('h2', {}, 'Store on cloud'),
    el('p', { class: 'warn', id: 'cloud-warning' }, 'Cloud documents are PUBLIC. Anyone who has the link can view and edit this document and its images, and anyone can open the room page. Do not store anything private.'),
    el('label', {}, 'Room ID (first come, first served)'), room, dice,
    el('label', {}, 'Title'), title,
    el('label', {}, 'Your name (optional, shown to others as the room owner; empty = a random smiley)'), yname, msg,
    el('div', { class: 'actions' }, cancel, go)));
}
async function cloudStore(roomId, title, msg) {
  let have = await token(roomId);
  if (!have) {
    const r = await api('POST', '/api/rooms', { room: roomId });
    await idb.put('meta', 'tok:' + roomId, r.token);
  }
  msg.textContent = 'Uploading...';
  const els = JSON.parse(S.board.export_all());
  const used = JSON.parse(S.board.images_used());
  const d = await api('POST', `/api/rooms/${roomId}/docs`, { title: title || 'Untitled', els }, roomId);
  for (const h of used) {
    const blob = await S.images.blob(h);
    await api('POST', `/api/rooms/${roomId}/docs/${d.nano}/images`, blob, roomId);
  }
  closeModal();
  S.dirty = false;
  location.assign(`/${roomId}/${d.nano}`);
}
function shareDialog() {
  const url = `${location.origin}/${route.room}/${route.nano}`;
  const inp = el('input', { type: 'text', value: url, readonly: 'true', id: 'share-url', 'aria-label': 'Share URL' });
  const copy = el('button', { class: 'pill primary' }, 'Copy link');
  copy.onclick = async () => { try { await navigator.clipboard.writeText(url); toast('Link copied'); } catch { inp.select(); } };
  const close = el('button', { class: 'pill' }, 'Close');
  close.onclick = closeModal;
  modal(el('div', {}, el('h2', {}, 'Share'),
    el('p', { class: 'warn' }, 'This document is public: anyone with the link can view and edit it.'),
    inp, el('div', { class: 'actions' }, close, copy)));
  inp.select();
}
async function deleteDocDialog() {
  const yes = el('button', { class: 'pill danger', id: 'del-confirm' }, 'Delete document');
  const no = el('button', { class: 'pill' }, 'Cancel');
  no.onclick = closeModal;
  yes.onclick = async () => {
    try { await api('DELETE', `/api/rooms/${route.room}/docs/${route.nano}`, undefined, route.room); S.stopped = true; location.assign('/' + route.room); }
    catch (e) { toast(e.message); closeModal(); }
  };
  modal(el('div', {}, el('h2', {}, 'Delete this document?'), el('p', {}, 'It will be removed for everyone. This cannot be undone.'), el('div', { class: 'actions' }, no, yes)));
}

// ---------------------------------------------------------------- websocket (cloud docs)
function wsSend(m) { if (S.ws && S.ws.readyState === 1) S.ws.send(JSON.stringify(m)); }
function connect() {
  if (S.stopped) return;
  clearTimeout(S.rcT);
  if (S.ws && S.ws.readyState <= 1 && S.ws.__live) return;
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${proto}://${location.host}/ws/${route.room}/${route.nano}`);
  S.ws = ws;
  ws.__live = true;
  setStatus(S.wsTry ? 'Reconnecting...' : 'Connecting...');
  ws.onopen = () => {
    ws.send(JSON.stringify({ t: 'hello', cid: S.cid, name: S.name, token: S.token, grant: S.grant, seq: S.seq, build: S.build }));
    clearInterval(S.pingT);
    S.pingT = setInterval(() => wsSend({ t: 'ping' }), 25000);
  };
  ws.onmessage = (ev) => {
    let m;
    try { m = JSON.parse(ev.data); } catch { return; }
    S.chain = (S.chain || Promise.resolve()).then(() => onServer(m)).catch((e) => console.error(e));
  };
  ws.onclose = () => {
    ws.__live = false;
    if (S.ws !== ws) return;
    S.wsOpen = false;
    clearInterval(S.pingT);
    S.peers.clear(); S.cursors.clear(); renderPeers();
    if (S.stopped) return;
    // exponential backoff with jitter, up to ~30 s; a socket that opens but is rejected counts as a failure
    const wait = Math.min(30000, 500 * 2 ** Math.min(S.wsTry++, 7)) * (0.75 + Math.random() * 0.5);
    S.nextTryAt = Date.now() + wait;
    offlineNotice();
    clearTimeout(S.rcT);
    S.rcT = setTimeout(connect, wait);
  };
  ws.onerror = () => {};
}
function showBanner(text, cls, actionLabel, action) {
  const bn = $('banner');
  bn.className = cls || '';
  bn.replaceChildren(el('span', {}, text));
  if (actionLabel) bn.append(el('button', { type: 'button', id: 'banner-action', onclick: action }, actionLabel));
}
function hideBanner() { if (!S.reloadNeeded) $('banner').className = 'hidden'; }
function offlineNotice() {
  if (S.reloadNeeded || S.needName) return;
  const tries = S.wsTry;
  const secs = Math.max(1, Math.round(((S.nextTryAt || Date.now()) - Date.now()) / 1000));
  setStatus(`Offline: retrying in ${secs}s`);
  if (tries >= 2 || !navigator.onLine) {
    showBanner('You are offline: your edits are not being shared. They are kept and will be sent when the connection returns.', 'err', 'Retry now', () => { S.wsTry = 0; connect(); });
  }
}
const PEND = () => 'pend:' + route.room + '/' + route.nano;
async function reloadPage() {
  // keep edits that were not sent yet, they are merged back after the reload
  try { if (S.board.has_ops()) await idb.put('meta', PEND(), S.board.export_all()); } catch { /* best effort */ }
  S.stopped = true;
  location.reload();
}
function onReload(m) {
  if (S.reloadNeeded) return;
  S.reloadNeeded = true;
  S.stopped = true; // stop reconnecting with an outdated client
  try { S.ws.close(); } catch { /* already closed */ }
  const last = Number(sessionStorage.getItem('draw-reloaded') || 0);
  const looping = Date.now() - last < 20000; // we already reloaded a moment ago: do not loop
  const go = () => { sessionStorage.setItem('draw-reloaded', String(Date.now())); reloadPage(); };
  setStatus('A new version is available');
  if (!S.board.has_ops() && !looping) {
    showBanner('A new version is available. Reloading...', '', 'Reload now', go);
    setTimeout(() => { if (!S.board.has_ops()) go(); }, 1200);
  } else {
    showBanner('A new version is available - reload. Your unsent edits are kept and restored after the reload.', '', 'Reload', go);
  }
}
async function restorePending() {
  try {
    const j = await idb.get('meta', PEND());
    if (j) { S.board.apply_remote(j); await idb.put('meta', PEND(), ''); }
  } catch { /* ignore */ }
}
async function onServer(m) {
  const b = S.board;
  switch (m.t) {
    case 'reload': onReload(m); break;
    case 'perm': setPerm(m.edit, m.pw); break;
    case 'snap': {
      if (S.reloadNeeded) break; // outdated build: keep local state untouched until the reload
      S.wsTry = 0;
      hideBanner();
      if (!S.restored) { S.restored = true; await restorePending(); }
      S.you = m.you; S.seq = m.seq; S.title = m.title;
      if (!S.name && m.you.name) { S.name = m.you.name; idb.put('meta', 'name', S.name).catch(() => {}); } // a smiley is remembered like a typed name
      $('doctitle').textContent = m.title;
      document.title = `${m.title} - draw-wasm`;
      S.peers = new Map(m.peers.map((p) => [p.id, p]));
      b.merge_snapshot(JSON.stringify(m.els));
      S.wsOpen = true;
      renderPeers();
      setStatus(S.edit ? 'Live' : 'Live (read-only)');
      if (!S.fitted) { S.fitted = true; b.fit(S.w, S.h); }
      lastSent = 0;
      uploadImages(JSON.parse(b.images_used())); // bytes held only by this browser reach the server
      break;
    }
    case 'ops': S.seq = m.seq; b.apply_remote(JSON.stringify(m.ops)); break;
    case 'peer': S.peers.set(m.peer.id, m.peer); renderPeers(); break;
    case 'gone': S.peers.delete(m.id); S.cursors.delete(m.id); renderPeers(); positionCursors(); break;
    case 'cur': S.cursors.set(m.id, { x: m.x, y: m.y, t: performance.now() }); positionCursors(); break;
    case 'saved': setStatus('Saved to the cloud'); setTimeout(() => S.wsOpen && setStatus('Live'), 2000); break;
    case 'err':
      if (/display name/.test(m.msg)) { S.needName = true; S.stopped = true; S.name = ''; nameDialog(true); } else toast(m.msg);
      break;
    case 'deleted': S.stopped = true; toast('This document was deleted by the room owner'); setTimeout(() => location.assign('/' + route.room), 1500); break;
    default: break;
  }
}
function sendCursor(p) {
  if (route.kind !== 'doc' || !S.wsOpen) return;
  const send = () => {
    S.lastCursorSent = performance.now();
    const q = S.curLast;
    const w = S.board.w2s(0, 0), z = S.board.zoom();
    wsSend({ t: 'cur', x: (q[0] - w[0]) / z, y: (q[1] - w[1]) / z });
  };
  S.curLast = p;
  const wait = 50 - (performance.now() - S.lastCursorSent);
  if (wait <= 0) { clearTimeout(S.curT); S.curT = null; send(); }
  else if (!S.curT) S.curT = setTimeout(() => { S.curT = null; send(); }, wait); // the last position is never dropped
}
const NAME_RE = /^[\p{L}\p{N} _.'-]+$/u;
// the only emoji a name may be: faces (same ranges as the server)
const FACES = [[0x1F600, 0x1F637], [0x1F641, 0x1F644], [0x1F910, 0x1F917], [0x1F920, 0x1F925], [0x1F927, 0x1F92F], [0x1F970, 0x1F976], [0x1F978, 0x1F97A]];
const isFace = (c) => FACES.some(([a, b]) => c >= a && c <= b);
/** A typed or stored name: up to 32 plain characters, or exactly one face emoji. Empty is handled by the caller. */
function validName(n) {
  const cps = [...n];
  if (cps.length === 1 && isFace(cps[0].codePointAt(0))) return true;
  return n.length >= 1 && n.length <= 32 && NAME_RE.test(n);
}
/** Ask for a display name. `required` = cannot be dismissed (first join). */
function nameDialog(required) {
  const inp = el('input', { id: 'name-input', type: 'text', value: S.name || '', maxlength: '32', autocomplete: 'off', 'aria-label': 'Your name', placeholder: 'Name (optional)' });
  const msg = el('div', { class: 'msg', id: 'name-msg' });
  const go = el('button', { class: 'pill primary', id: 'name-go' }, required ? 'Join' : 'Save');
  const submit = async () => {
    const n = inp.value.trim().replace(/\s+/g, ' ');
    if (n && !validName(n)) { msg.textContent = 'Use up to 32 letters, digits, spaces, - _ . or an apostrophe, or leave it empty'; return; }
    S.name = n; // empty = the server hands out a random smiley, remembered after the join
    if (n) await idb.put('meta', 'name', n).catch(() => {});
    else await idb.put('meta', 'name', '').catch(() => {});
    const wasStopped = S.needName;
    S.needName = false;
    closeModal();
    if (wasStopped) { S.stopped = false; S.wsTry = 0; connect(); }
    else if (S.ws) { try { S.ws.close(); } catch {} S.wsTry = 0; } // reconnect under the new name
  };
  go.onclick = submit;
  inp.addEventListener('keydown', (ev) => { ev.stopPropagation(); if (ev.key === 'Enter') submit(); });
  const actions = el('div', { class: 'actions' });
  if (!required) { const c = el('button', { class: 'pill', id: 'name-cancel' }, 'Cancel'); c.onclick = closeModal; actions.append(c); }
  actions.append(go);
  modal(el('div', {}, el('h2', {}, required ? 'Pick a name to join (optional)' : 'Change your name'),
    el('p', {}, 'Other people in this document see this name next to your cursor. Leave it empty to join as a random smiley.'), inp, msg, actions));
  const m = $('modal');
  if (required) m.onclick = null;
}
function renderPeers() {
  const box = $('peers');
  box.replaceChildren();
  const list = [...S.peers.values()];
  if (S.you) list.unshift({ ...S.you, me: true });
  for (const p of list.slice(0, 8)) {
    const d = el('span', { class: 'peer' + (p.owner ? ' owner' : ''), title: p.name + (p.owner ? ' (owner)' : '') + (p.me ? ' (you)' : '') });
    d.style.background = /^#[0-9a-f]{6}$/i.test(p.color) ? p.color : '#888';
    d.textContent = ([...(p.name || '?')][0] || '?').toUpperCase();
    d.dataset.peer = p.id;
    box.append(d);
  }
  if (list.length > 8) box.append(el('span', { class: 'peer more' }, '+' + (list.length - 8)));
  box.title = list.length + ' here';
  box.dataset.count = list.length;
}
function positionCursors() {
  const layer = $('cursors');
  const seen = new Set();
  for (const [id, c] of S.cursors) {
    const peer = S.peers.get(id);
    if (!peer) continue;
    seen.add(id);
    let d = layer.querySelector(`[data-cid="${CSS.escape(id)}"]`);
    if (!d) {
      d = el('div', { class: 'rcursor', 'data-cid': id });
      d.innerHTML = '<svg width="16" height="16" viewBox="0 0 16 16"><path d="M1 1l12 5-5 2-2 5z" fill="currentColor" stroke="#fff" stroke-width="1"/></svg><span></span>';
      layer.append(d);
    }
    const pc = /^#[0-9a-f]{6}$/i.test(peer.color) ? peer.color : '#888';
    d.style.color = pc;
    d.lastChild.style.background = pc;
    d.lastChild.textContent = (peer.owner ? '★ ' : '') + peer.name;
    const s = S.board.w2s(c.x, c.y);
    d.style.transform = `translate(${s[0]}px, ${s[1]}px)`;
  }
  for (const d of [...layer.children]) if (!seen.has(d.dataset.cid)) d.remove();
}

// ---------------------------------------------------------------- edit password (items 46)
/** Apply what the server says this connection may do. The server enforces it; this only shapes the UI. */
function setPerm(edit, pw) {
  S.edit = !!edit;
  S.pw = !!pw;
  S.board?.set_readonly(!S.edit);
  document.body.classList.toggle('ro', !S.edit);
  $('rochip').classList.toggle('hidden', S.edit || route.kind !== 'doc');
  $('btn-del').classList.toggle('hidden', !S.edit || route.kind !== 'doc');
  $('btn-save').classList.toggle('hidden', route.kind === 'doc' && !S.edit);
  if (!S.edit) { commitEdit(); setStatus('Read-only: enter the password to edit'); }
  else if (S.wsOpen) { setStatus('Live'); uploadImages(JSON.parse(S.board.images_used())); }
}
/** Ask for the room password; on success the server's grant is remembered in this browser. */
function unlockDialog(after) {
  const inp = el('input', { id: 'unlock-input', type: 'password', maxlength: '128', autocomplete: 'off', 'aria-label': 'Room edit password' });
  const msg = el('div', { class: 'msg', id: 'unlock-msg' });
  const go = el('button', { class: 'pill primary', id: 'unlock-go' }, 'Unlock editing');
  const cancel = el('button', { class: 'pill', id: 'unlock-cancel' }, 'Cancel');
  cancel.onclick = closeModal;
  const submit = async () => {
    go.disabled = true;
    msg.textContent = '';
    try {
      const r = await api('POST', `/api/rooms/${route.room}/unlock`, { password: inp.value }, route.room);
      if (r.grant) { await idb.put('meta', 'grant:' + route.room, r.grant); S.grant = r.grant; }
      closeModal();
      toast('Editing unlocked');
      if (after) { after(); return; }
      if (S.ws) { S.wsTry = 0; try { S.ws.close(); } catch { /* reconnect below */ } } // reconnect with the grant
    } catch (e) { msg.textContent = e.status === 401 ? 'Wrong password' : e.message; go.disabled = false; }
  };
  go.onclick = submit;
  inp.addEventListener('keydown', (ev) => { ev.stopPropagation(); if (ev.key === 'Enter') submit(); });
  modal(el('div', {}, el('h2', {}, 'This room is read-only'),
    el('p', {}, 'Enter the room password to edit. You can keep viewing without it.'), inp, msg, el('div', { class: 'actions' }, cancel, go)));
}
/** Owner: set, change or remove the room's edit password. */
function passwordDialog(after) {
  const inp = el('input', { id: 'pw-input', type: 'password', maxlength: '128', autocomplete: 'new-password', placeholder: S.pw ? 'New password' : 'Password (4+ characters)', 'aria-label': 'New room edit password' });
  const msg = el('div', { class: 'msg', id: 'pw-msg' });
  const send = async (pw) => {
    try {
      const r = await api('PUT', `/api/rooms/${route.room}/password`, { password: pw }, route.room);
      S.pw = r.pw;
      closeModal();
      toast(r.pw ? 'Edit password set: everyone without it is now read-only' : 'Edit password removed: everyone can edit');
      if (after) after();
    } catch (e) { msg.textContent = e.message; }
  };
  const go = el('button', { class: 'pill primary', id: 'pw-set' }, S.pw ? 'Change password' : 'Set password');
  go.onclick = () => { if (inp.value.length < 4) { msg.textContent = 'Use at least 4 characters'; return; } send(inp.value); };
  inp.addEventListener('keydown', (ev) => { ev.stopPropagation(); if (ev.key === 'Enter') go.click(); });
  const rm = el('button', { class: 'pill danger', id: 'pw-remove' }, 'Remove password');
  rm.onclick = () => send('');
  if (!S.pw) rm.classList.add('hidden');
  const cancel = el('button', { class: 'pill', id: 'pw-cancel' }, 'Cancel');
  cancel.onclick = closeModal;
  modal(el('div', {}, el('h2', {}, 'Room edit password'),
    el('p', {}, S.pw ? 'A password is set: only you and people who know it can edit. Changing or removing it signs everyone else out of editing.'
      : 'No password: everyone with the link can edit and delete. Set one to make the room read-only for everyone who does not know it.'),
    inp, msg, el('div', { class: 'actions' }, cancel, rm, go)));
}

// ---------------------------------------------------------------- pages
async function roomPage() {
  document.body.classList.add('roommode');
  const box = $('roompage');
  box.classList.remove('hidden');
  $('topbar').classList.add('hidden');
  const head = el('div', { class: 'rhead' }, el('a', { href: '/', class: 'pill' }, 'Home'));
  const h1 = el('h1', { id: 'room-title' }, 'Room ' + route.room);
  const list = el('div', { id: 'doclist', class: 'doclist' });
  const note = el('p', { class: 'warn' }, 'Rooms and their documents are public. Anyone with the link can view and edit.');
  box.replaceChildren(el('div', { class: 'rwrap' }, head, h1, note, list));
  let data;
  try { data = await api('GET', `/api/rooms/${route.room}`, undefined, route.room); }
  catch (e) { list.append(el('p', { class: 'msg' }, e.status === 404 ? 'This room does not exist.' : e.message)); return; }
  const mk = el('button', { class: 'pill primary', id: 'new-doc' }, 'New document');
  if (!data.edit) {
    mk.classList.add('hidden');
    head.append(el('span', { class: 'rotext', id: 'room-ro' }, 'Read-only'));
    const un = el('button', { class: 'pill primary', id: 'room-unlock' }, 'Enter password to edit');
    un.onclick = () => unlockDialog(() => location.reload());
    head.append(un);
  }
  if (data.owner) {
    const pb = el('button', { class: 'pill', id: 'room-pw' }, 'Password');
    pb.onclick = () => passwordDialog(() => location.reload());
    head.append(pb);
  }
  mk.onclick = async () => {
    mk.disabled = true;
    try { const d = await api('POST', `/api/rooms/${route.room}/docs`, { title: 'Untitled', els: [] }, route.room); location.assign(`/${route.room}/${d.nano}`); }
    catch (e) { toast(e.message); mk.disabled = false; }
  };
  head.append(mk);
  if (data.owner) {
    const dr = el('button', { class: 'pill danger', id: 'del-room' }, 'Delete room');
    dr.onclick = async () => {
      if (!confirm('Delete this room and all its documents for everyone?')) return;
      try { await api('DELETE', `/api/rooms/${route.room}`, undefined, route.room); location.assign('/'); } catch (e) { toast(e.message); }
    };
    head.append(dr);
  }
  if (!data.docs.length) list.append(el('p', { class: 'msg' }, 'No documents yet.'));
  for (const d of data.docs) {
    const a = el('a', { class: 'docitem', href: `/${route.room}/${d.nano}` }, el('span', { class: 'dt' }, d.title || 'Untitled'), el('span', { class: 'dm' }, new Date(d.updated * 1000).toLocaleString()));
    list.append(a);
  }
}

// ---------------------------------------------------------------- boot
async function main() {
  buildToolbar();
  buildStyle();
  bindButtons();
  if (route.kind === 'room') { await roomPage(); return; }
  await init();
  S.board = new Board(S.cid);
  S.gfx = new Gfx(S.canvas, S.images);
  S.gfx.onImageReady = () => S.board.invalidate();
  resize();
  bindInput();
  setTool('select');
  syncStyle();
  if (route.kind === 'local') {
    await loadLocal();
    $('btn-share').classList.add('hidden');
    setStatus(S.dirty ? '' : $('status').textContent || 'Private: stored only in this browser');
  } else {
    $('btn-cloud').classList.add('hidden');
    $('btn-share').classList.remove('hidden');
    try {
      const room = await api('GET', `/api/rooms/${route.room}`, undefined, route.room);
      S.owner = !!room.owner;
      if (S.owner) S.token = (await token(route.room)) || '';
      S.grant = (await idb.get('meta', 'grant:' + route.room).catch(() => '')) || '';
      $('btn-pw').classList.toggle('hidden', !S.owner);
      setPerm(room.edit !== false, !!room.pw);
      const crumb = el('a', { href: '/' + route.room, id: 'roomlink' }, route.room);
      $('doctitle').before(crumb, el('span', { class: 'slash' }, '/'));
    } catch { /* room page link is optional */ }
    S.name = (await idb.get('meta', 'name').catch(() => '')) || '';
    $('btn-name').classList.remove('hidden');
    S.images.auth = () => authHeaders(route.room);
    S.images.onChange = () => S.board?.invalidate();
    if (!S.owner && !S.name) { S.needName = true; S.stopped = true; nameDialog(true); }
    else if (S.name && !validName(S.name)) { S.name = ''; S.needName = true; S.stopped = true; nameDialog(true); }
    else connect();
    window.addEventListener('online', () => { if (!S.wsOpen && !S.reloadNeeded && !S.needName) { S.wsTry = 0; connect(); } });
    window.addEventListener('offline', () => { try { S.ws?.close(); } catch {} });
  }
  S.lastDocRev = S.board.doc_rev();
  requestAnimationFrame(frame);
}
main().catch((e) => { console.error(e); toast('Failed to start: ' + (e && e.message || e), 10000); });
