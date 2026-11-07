'use strict';
/* Area picker: one of these covers each screen. Drag a box, Enter records it, Esc cancels,
   right click takes the whole screen. The app turns the box into desktop pixels. */
const T = window.__TAURI__;
const invoke = (cmd, args) => (T && T.core ? T.core.invoke(cmd, args) : Promise.reject(t('err.backend')));
const box = document.getElementById('box');
const hint = document.getElementById('hint');
// window labels are capture-sel-<screen index>
const mon = Number(((T && T.window && T.window.getCurrentWindow().label) || '').split('-').pop()) || 0;
const MIN = 8; // css px: a click is not a box
let from = null; // drag start
let sel = null; // {x, y, w, h} once a box exists

setLang('en');
hint.textContent = t('cap.hint');
if (T && T.core) {
  T.core.invoke('get_settings').then((s) => {
    setLang(s.language);
    hint.textContent = t('cap.hint');
  }).catch(() => {});
}

function draw(r) {
  box.hidden = !r;
  document.body.classList.toggle('has-box', !!r);
  if (!r) return;
  box.style.left = r.x + 'px';
  box.style.top = r.y + 'px';
  box.style.width = r.w + 'px';
  box.style.height = r.h + 'px';
}

function rect(a, b) {
  const x = Math.max(0, Math.min(a.x, b.x)), y = Math.max(0, Math.min(a.y, b.y));
  const x2 = Math.min(innerWidth, Math.max(a.x, b.x)), y2 = Math.min(innerHeight, Math.max(a.y, b.y));
  return { x, y, w: x2 - x, h: y2 - y };
}

async function pick(r) {
  try {
    await invoke('capture_pick', { monitor: mon, x: r.x, y: r.y, w: r.w, h: r.h, innerW: innerWidth });
  } catch (e) {
    sel = null;
    draw(null);
    hint.textContent = String(e);
  }
}

addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  from = { x: e.clientX, y: e.clientY };
  sel = null;
  draw(null);
  document.body.setPointerCapture(e.pointerId);
});
addEventListener('pointermove', (e) => {
  if (!from) return;
  draw(rect(from, { x: e.clientX, y: e.clientY }));
});
addEventListener('pointerup', (e) => {
  if (!from) return;
  const r = rect(from, { x: e.clientX, y: e.clientY });
  from = null;
  if (r.w < MIN || r.h < MIN) return draw(null);
  sel = r;
  draw(r);
  hint.textContent = t('cap.confirm');
});
addEventListener('contextmenu', (e) => {
  e.preventDefault();
  pick({ x: 0, y: 0, w: innerWidth, h: innerHeight });
});
addEventListener('keydown', (e) => {
  if (e.key === 'Escape') invoke('capture_abort_select').catch(() => {});
  else if (e.key === 'Enter' && sel) pick(sel);
});
