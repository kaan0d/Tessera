'use strict';
const T = window.__TAURI__;
const root = document.getElementById('jobs');
const jobs = new Map(); // id -> {li, sheet, msg, actions, percent, state}
const SVG = 'http://www.w3.org/2000/svg';

const SHEET = 'M4 1h11.5L23 8.5V27a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V3a2 2 0 0 1 2-2z';
const PICT = {
  image: 'M6.5 23.5l4.5-5 3.5 3.5 2-2 2.5 3.5M9.5 14.6v.1',
  video: 'M10 15v8l6.5-4z',
  audio: 'M11 23v-8.5l6-1.5v8M11 23a1.7 1.7 0 1 1-1.7-1.7A1.7 1.7 0 0 1 11 23zM17 21a1.7 1.7 0 1 1-1.7-1.7A1.7 1.7 0 0 1 17 21z',
  doc: 'M7 15h11M7 19h11M7 23h7',
};
const KIND = {
  image: /\.(jpe?g|png|webp|gif|bmp|tiff?|heic|heif|avif|ico)$/i,
  video: /\.(mp4|mkv|mov|avi|webm|wmv|flv|m4v|mpe?g)$/i,
  audio: /\.(mp3|wav|flac|ogg|oga|aac|m4a|opus|wma)$/i,
};
const kindOf = (name) => Object.keys(KIND).find((k) => KIND[k].test(name)) || 'doc';
let uid = 0;

/* ---------- toast-like entrance and exit ---------- */
const card = document.getElementById('card');
const timer = document.getElementById('p-timer');
let shown = false, seconds = 7, leaveTimer = null;

// corner decides the slide direction; refreshed while hidden so the next show is right
async function loadPanel() {
  try {
    const p = (await T.core.invoke('get_settings')).panel;
    if (p) {
      document.body.dataset.corner = p.corner;
      seconds = p.seconds;
    }
  } catch { /* keep the last known corner */ }
}

function restart(cls) {
  card.classList.remove('entering', 'leaving');
  void card.offsetWidth;
  card.classList.add(cls);
}

function enter() {
  cancelLeave();
  if (shown) return;
  shown = true;
  restart('entering');
}

function cancelLeave() {
  clearTimeout(leaveTimer);
  leaveTimer = null;
  timer.classList.remove('run');
  timer.style.transitionDuration = '';
}

// the backend hides the window panel.seconds after the last job; slide out just before it does
function scheduleLeave() {
  if (leaveTimer) return;
  timer.style.transitionDuration = seconds + 's';
  void timer.offsetWidth;
  timer.classList.add('run');
  leaveTimer = setTimeout(() => {
    shown = false;
    restart('leaving');
  }, Math.max(0, seconds * 1000 - 360));
}

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible') return enter();
  cancelLeave();
  shown = false;
  card.classList.remove('entering', 'leaving');
  loadPanel();
});

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
function svg(tag, attrs) {
  const e = document.createElementNS(SVG, tag);
  for (const k in attrs) e.setAttribute(k, attrs[k]);
  return e;
}

// the file as a sheet that fills while it is rewritten
function sheetFor(name) {
  const id = 'clip' + ++uid;
  const s = svg('svg', { class: 'sheet working', viewBox: '0 0 25 30', 'aria-hidden': 'true' });
  const clip = svg('clipPath', { id });
  clip.append(svg('path', { d: SHEET }));
  const defs = svg('defs', {});
  defs.append(clip);
  const g = svg('g', { 'clip-path': `url(#${id})` });
  const level = svg('g', { class: 'level' });
  level.append(svg('path', { class: 'ink', d: 'M-24 0q3-2 6 0t6 0t6 0t6 0t6 0t6 0t6 0t6 0V40h-48z' }));
  g.append(level);
  s.append(
    defs, g,
    svg('path', { class: 'outline', d: SHEET }),
    svg('path', { class: 'fold', d: 'M15.5 1v5.5a2 2 0 0 0 2 2H23' }),
    svg('path', { class: 'pict', d: PICT[kindOf(name)] }),
    svg('path', { class: 'mark-ok', pathLength: '1', d: 'M7.5 18.5l3.2 3.2 6.8-7' }),
    svg('path', { class: 'mark-err', pathLength: '1', d: 'M9 15.5l7 7' }),
    svg('path', { class: 'mark-err', pathLength: '1', d: 'M16 15.5l-7 7' }),
  );
  return s;
}

function iconBtn(icon, label, cls) {
  const b = el('button', 'iconbtn' + (cls ? ' ' + cls : ''));
  b.type = 'button';
  b.setAttribute('aria-label', label);
  b.title = label;
  const i = svg('svg', { class: 'ic', 'aria-hidden': 'true' });
  i.append(svg('use', { href: '#i-' + icon }));
  b.append(i);
  return b;
}

function removeJob(id, animate) {
  const j = jobs.get(id);
  if (!j) return;
  jobs.delete(id);
  clearTimeout(j.timer);
  if (!animate) return j.li.remove(), summary();
  j.li.classList.add('leaving');
  setTimeout(() => j.li.remove(), 450);
  summary();
}

function getJob(id, name) {
  let j = jobs.get(id);
  if (j) return j;
  // the window was hidden since the last batch: start clean
  if (![...jobs.values()].some((x) => x.state === 'run')) [...jobs.keys()].forEach((k) => removeJob(k, false));
  enter();
  const li = el('li', 'job entering');
  const inner = el('div', 'job-in');
  const sheet = sheetFor(name);
  const wrap = el('div', 'grow');
  const nm = el('div', 'name', name);
  nm.title = name;
  const msg = el('div', 'msg', '%0');
  wrap.append(nm, msg);
  const actions = el('div', 'job-actions');
  const cancel = iconBtn('x', 'İptal et: ' + name, 'cancel');
  cancel.onclick = () => {
    cancel.disabled = true;
    msg.textContent = 'İptal ediliyor…';
    T.core.invoke('cancel_job', { id }).catch(() => (cancel.disabled = false));
  };
  actions.append(cancel);
  inner.append(sheet, wrap, actions);
  const clip = el('div');
  clip.append(inner);
  li.append(clip);
  li.setAttribute('aria-label', name);
  root.append(li);
  li.addEventListener('animationend', () => li.classList.remove('entering'), { once: true });
  j = { li, sheet, msg, actions, name, percent: 0, state: 'run' };
  jobs.set(id, j);
  return j;
}

function setLevel(j, p) {
  j.percent = p;
  j.sheet.style.setProperty('--p', p / 100);
}

function summary() {
  const all = [...jobs.values()];
  const run = all.filter((j) => j.state === 'run').length;
  const ok = all.filter((j) => j.state === 'ok').length;
  const err = all.filter((j) => j.state === 'err').length;
  const pct = all.length ? all.reduce((a, j) => a + (j.state === 'run' ? j.percent : 100), 0) / all.length : 0;
  const head = document.getElementById('head');
  head.classList.toggle('done', !run && all.length > 0);
  document.getElementById('p-title').textContent = run ? 'Dönüştürülüyor' : err ? 'Bazı dosyalar dönüştürülemedi' : 'Tamamlandı';
  document.getElementById('p-sub').textContent = run
    ? all.length === 1 ? '1 dosya' : `${all.length - run}/${all.length} dosya bitti`
    : err ? `${ok} başarılı, ${err} başarısız` : `${ok} dosya hazır`;
  document.getElementById('p-pct').textContent = Math.round(pct);
  document.getElementById('p-bar').style.setProperty('--p', pct / 100);
  if (run) cancelLeave();
  else if (all.length) scheduleLeave();
}

function progress({ id, name, percent }) {
  const j = getJob(id, name);
  if (j.state !== 'run') return;
  const p = Math.max(0, Math.min(100, percent || 0));
  setLevel(j, p);
  if (!j.msg.textContent.startsWith('İptal')) j.msg.textContent = `%${Math.round(p)}`;
  summary();
}

function done({ id, name, ok, message, output }) {
  const j = getJob(id, name);
  j.state = ok ? 'ok' : 'err';
  j.li.classList.add(ok ? 'is-ok' : 'is-err');
  j.sheet.classList.remove('working');
  j.sheet.classList.add(ok ? 'ok' : 'err');
  if (ok) setLevel(j, 100);
  j.msg.textContent = ok ? 'Tamamlandı' : message || 'Dönüştürme başarısız oldu.';
  j.msg.title = j.msg.textContent;
  j.actions.replaceChildren();
  if (ok && output) {
    const open = iconBtn('folder', 'Klasörü aç');
    open.onclick = () => T.core.invoke('open_folder', { path: output.replace(/[\\/][^\\/]*$/, '') }).catch((e) => {
      j.li.classList.add('is-err');
      j.msg.textContent = 'Klasör açılamadı: ' + e;
      clearTimeout(j.timer);
    });
    j.actions.append(open);
  }
  const close = iconBtn('x', 'Kapat');
  close.onclick = () => removeJob(id, true);
  j.actions.append(close);
  // successes fold away on their own; failures wait to be read
  if (ok) j.timer = setTimeout(() => removeJob(id, true), 6000);
  document.getElementById('announce').textContent = ok ? `${name} dönüştürüldü.` : `${name} dönüştürülemedi.`;
  summary();
}

if (T && T.core) loadPanel();
if (T && T.event) {
  T.event.listen('job-progress', (e) => progress(e.payload));
  T.event.listen('job-done', (e) => done(e.payload));
}
