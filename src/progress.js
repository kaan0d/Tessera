'use strict';
const T = window.__TAURI__;
const root = document.getElementById('jobs');
const jobs = new Map(); // id -> {el, bar, pct, msg, btn}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function getJob(id, name) {
  let j = jobs.get(id);
  if (j) return j;
  const card = el('div', 'job');
  const nm = el('div', 'name', name);
  nm.title = name;
  const bar = el('progress');
  bar.max = 100;
  bar.value = 0;
  bar.setAttribute('aria-label', name);
  const pct = el('span', 'muted', '%0');
  const btn = el('button', 'btn', 'İptal');
  btn.type = 'button';
  btn.onclick = () => {
    btn.disabled = true;
    T.core.invoke('cancel_job', { id }).catch(() => (btn.disabled = false));
  };
  const meta = el('div', 'meta');
  meta.append(pct, btn);
  const msg = el('div', 'msg');
  card.append(nm, bar, meta, msg);
  root.append(card);
  j = { card, bar, pct, msg, btn, meta };
  jobs.set(id, j);
  return j;
}

function progress({ id, name, percent }) {
  const j = getJob(id, name);
  const p = Math.max(0, Math.min(100, percent || 0));
  j.bar.value = p;
  j.pct.textContent = `%${Math.round(p)}`;
}

function done({ id, name, ok, message, output }) {
  const j = getJob(id, name);
  j.bar.value = ok ? 100 : j.bar.value;
  j.btn.remove();
  j.pct.textContent = ok ? '%100' : '';
  j.msg.className = 'msg ' + (ok ? 'ok' : 'err');
  j.msg.textContent = ok ? 'Tamamlandı.' : message || 'Dönüştürme başarısız oldu.';
  if (ok && output) {
    const open = el('button', 'btn', 'Klasörü aç');
    open.type = 'button';
    open.onclick = () => T.core.invoke('open_folder', { path: output.replace(/[\\/][^\\/]*$/, '') }).catch((e) => {
      j.msg.className = 'msg err';
      j.msg.textContent = 'Klasör açılamadı: ' + e;
    });
    j.meta.append(open);
  }
  const dismiss = el('button', 'btn', 'Kapat');
  dismiss.type = 'button';
  dismiss.onclick = () => { j.card.remove(); jobs.delete(id); };
  j.meta.append(dismiss);
}

if (T && T.event) {
  T.event.listen('job-progress', (e) => progress(e.payload));
  T.event.listen('job-done', (e) => done(e.payload));
}
